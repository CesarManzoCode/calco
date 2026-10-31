//! Guest threads (contract 3.12): `clone` with `CLONE_VM|CLONE_THREAD`, the end of a
//! thread, and what the threads share.
//!
//! - A guest thread is a `std::thread` of the host with its own `Cpu`, GS base and
//!   alternate signal stack; its tid is the host one. The child starts with the registers
//!   of the parent at the `syscall`, `rax = 0`, the stack it asked for and, with
//!   `CLONE_SETTLS`, the `fs_base` it asked for. Its guest mask is the parent's.
//! - `CLONE_PARENT_SETTID` and `CLONE_CHILD_SETTID` are written by the parent with the tid
//!   the child sends, before the parent goes on and before the child runs guest code.
//!   `CLONE_CHILD_CLEARTID` stays in the child's `Cpu`: its `exit` writes 0 there and
//!   wakes a waiter with `futex(FUTEX_WAKE)`, as `mm_release` does, without touching the
//!   `set_tid_address` of the host glibc. `exit_group` goes to the kernel, which ends
//!   every thread with that code.
//! - Shared state: the code cache behind `Mutex<Jit>` (translate, patch, invalidate) with
//!   its `Index` read without the lock, and the memory map and process state behind
//!   `Mutex<World>`. Lock order: cache, world, thread list.
//! - Safe point: a thread is in the cache from before it looks a block up until it is
//!   back in Rust from `enter`. Emptying the cache raises `FLUSHING`, kicks with host
//!   signal 62 every thread still in the cache (its handler sends it to the dispatcher at
//!   a guest boundary), waits until none is, empties it, and wakes the threads that met
//!   the flush on their way in and wait at the barrier.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use crate::cpu::{Cpu, R11, RAX, RCX, RSP};
use crate::jit::{self, Index, Jit};
use crate::mem::{Mem, USER_END};
use crate::signal::{self, HOST_KICK, SIG_SETMASK};
use crate::sys::{self, EAGAIN, EINVAL, EPERM};
use crate::syscall::{State, write_bytes};

// /usr/include/linux/sched.h
pub const CSIGNAL: u64 = 0xff;
pub const CLONE_VM: u64 = 0x100;
pub const CLONE_FS: u64 = 0x200;
pub const CLONE_FILES: u64 = 0x400;
pub const CLONE_SIGHAND: u64 = 0x800;
pub const CLONE_VFORK: u64 = 0x4000;
pub const CLONE_THREAD: u64 = 0x10000;
pub const CLONE_NEWNS: u64 = 0x20000;
pub const CLONE_SYSVSEM: u64 = 0x40000;
pub const CLONE_SETTLS: u64 = 0x80000;
pub const CLONE_PARENT_SETTID: u64 = 0x100000;
pub const CLONE_CHILD_CLEARTID: u64 = 0x200000;
pub const CLONE_DETACHED: u64 = 0x400000;
pub const CLONE_CHILD_SETTID: u64 = 0x1000000;

/// What a host thread always shares with the others: a guest thread has to ask for it.
const THREAD_SHARES: u64 = CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD;
/// What a guest thread may ask for besides; anything else is refused.
const THREAD_OPTIONS: u64 = CSIGNAL
    | CLONE_SYSVSEM
    | CLONE_SETTLS
    | CLONE_PARENT_SETTID
    | CLONE_CHILD_CLEARTID
    | CLONE_DETACHED
    | CLONE_CHILD_SETTID;
/// How long a flushing thread waits before kicking again the threads still in the cache.
const KICK_AGAIN: Duration = Duration::from_micros(100);

/// How calco serves a `clone`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloneKind {
    /// `CLONE_VM|CLONE_THREAD`: a new guest thread.
    Thread,
    /// No `CLONE_VM`, or vfork (`CLONE_VM|CLONE_VFORK`, served as fork): the host forks.
    Fork,
}

/// The kind of `clone` that `flags` ask for. First the kernel's own checks
/// (`CLONE_THREAD` needs `CLONE_SIGHAND`, which needs `CLONE_VM`); then `EINVAL` for
/// `CLONE_VM` without `CLONE_THREAD` other than vfork (contract 3.12), and for a thread
/// that does not share what host threads always share or asks for anything else.
pub fn clone_kind(flags: u64) -> Result<CloneKind, i32> {
    if flags & CLONE_THREAD != 0 && flags & CLONE_SIGHAND == 0 {
        return Err(EINVAL);
    }
    if flags & CLONE_SIGHAND != 0 && flags & CLONE_VM == 0 {
        return Err(EINVAL);
    }
    match (flags & CLONE_VM != 0, flags & CLONE_THREAD != 0) {
        (false, _) => Ok(CloneKind::Fork),
        (true, false) if flags & CLONE_VFORK != 0 => Ok(CloneKind::Fork),
        (true, false) => Err(EINVAL),
        (true, true)
            if flags & THREAD_SHARES == THREAD_SHARES
                && flags & !(THREAD_SHARES | THREAD_OPTIONS) == 0 =>
        {
            Ok(CloneKind::Thread)
        }
        (true, true) => Err(EINVAL),
    }
}

/// The `Cpu` of a new thread of `parent`, which is at the `syscall` of `clone` with
/// `parent.rip` past it: the parent's registers, RFLAGS, segment bases and x87/SSE state,
/// with `rax = 0`, RCX and R11 as the `syscall` leaves them, `stack` as RSP if not 0,
/// `tls` as `fs_base` with `CLONE_SETTLS` and `ctid` as `clear_child_tid` with
/// `CLONE_CHILD_CLEARTID`. No signal state, no tid: the child sets them.
pub fn child_cpu(parent: &Cpu, flags: u64, stack: u64, ctid: u64, tls: u64) -> Box<Cpu> {
    let mut cpu = Cpu::new(parent.rip, parent.gpr[RSP]);
    cpu.gpr = parent.gpr;
    cpu.gpr[RAX] = 0;
    cpu.gpr[RCX] = parent.rip;
    cpu.gpr[R11] = parent.rflags;
    if stack != 0 {
        cpu.gpr[RSP] = stack;
    }
    cpu.rflags = parent.rflags;
    cpu.fs_base = if flags & CLONE_SETTLS != 0 {
        tls
    } else {
        parent.fs_base
    };
    cpu.gs_base = parent.gs_base;
    cpu.fxsave.0 = parent.fxsave.0;
    if flags & CLONE_CHILD_CLEARTID != 0 {
        cpu.clear_child_tid = ctid;
    }
    cpu
}

/// The process state behind one lock: memory map, signal actions, emulated call state.
pub struct World {
    pub mem: Mem,
    pub state: State,
}

/// What every guest thread uses.
pub struct Shared {
    /// The part of the code cache read without its lock.
    pub index: &'static Index,
    jit: Mutex<Jit>,
    world: Mutex<World>,
    threads: Mutex<Vec<Arc<Member>>>,
    /// Threads that met a flush on their way into the cache wait here, with `threads`.
    parked: Condvar,
}

/// A live guest thread, as a flushing thread sees it.
pub struct Member {
    /// Host tid, where the kick goes.
    tid: AtomicU64,
    /// From `enter_cache` to `leave_cache`.
    in_cache: AtomicBool,
}

static SHARED: OnceLock<Shared> = OnceLock::new();
/// A flush waits for the threads or empties the cache.
static FLUSHING: AtomicBool = AtomicBool::new(false);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    pub fn jit(&self) -> MutexGuard<'_, Jit> {
        lock(&self.jit)
    }

    pub fn world(&self) -> MutexGuard<'_, World> {
        lock(&self.world)
    }

    pub fn threads(&self) -> MutexGuard<'_, Vec<Arc<Member>>> {
        lock(&self.threads)
    }
}

/// The shared state, set by `run_main` before any guest code runs.
pub fn shared() -> &'static Shared {
    SHARED.get().expect("the guest has not started")
}

/// The lock-free part of the cache, for the signal handler; `None` before the start.
pub fn index() -> Option<&'static Index> {
    SHARED.get().map(|s| s.index)
}

/// The cache, if no thread holds its lock (for the signal handler, which cannot wait).
pub fn try_jit() -> Option<MutexGuard<'static, Jit>> {
    SHARED.get()?.jit.try_lock().ok()
}

/// Whether a flush is waiting for the threads to leave the cache.
pub fn flushing() -> bool {
    FLUSHING.load(Ordering::SeqCst)
}

/// Marks the thread as in the cache, before it looks up a block; if a flush runs, waits
/// for it to end first. With `safe_point`, each side stores its flag and loads the other's
/// in sequential consistency: either the flush sees the thread in the cache, or the thread
/// sees the flush.
pub fn enter_cache(me: &Member) {
    loop {
        me.in_cache.store(true, Ordering::SeqCst);
        if !FLUSHING.load(Ordering::SeqCst) {
            return;
        }
        me.in_cache.store(false, Ordering::SeqCst);
        let shared = shared();
        let threads = shared.threads();
        let _threads = shared
            .parked
            .wait_while(threads, |_| FLUSHING.load(Ordering::SeqCst))
            .unwrap_or_else(PoisonError::into_inner);
    }
}

/// The thread holds no address of the cache and runs none of its code.
pub fn leave_cache(me: &Member) {
    me.in_cache.store(false, Ordering::SeqCst);
}

/// Runs `f`, which empties the cache, once no other thread is in it (contract 3.7, 3.12).
/// The caller holds the cache lock, so no other flush runs, and a thread on its way to
/// that lock is out of the cache. Threads still in it get host signal 62 until they leave;
/// the ones that come in meanwhile park in `enter_cache`.
pub fn safe_point(f: impl FnOnce()) {
    let Some(shared) = SHARED.get() else {
        return f();
    };
    FLUSHING.store(true, Ordering::SeqCst);
    let me = sys::gettid();
    loop {
        let busy: Vec<u64> = shared
            .threads()
            .iter()
            .filter(|m| m.in_cache.load(Ordering::SeqCst))
            .map(|m| m.tid.load(Ordering::Relaxed))
            .filter(|&tid| tid != me)
            .collect();
        if busy.is_empty() {
            break;
        }
        for tid in busy {
            // A thread that ended meanwhile gives ESRCH; it is out of the list next time.
            let _ = sys::tgkill(tid, HOST_KICK);
        }
        std::thread::sleep(KICK_AGAIN);
    }
    f();
    let _threads = shared.threads();
    FLUSHING.store(false, Ordering::SeqCst);
    shared.parked.notify_all();
}

fn register(tid: u64) -> Arc<Member> {
    let me = Arc::new(Member {
        tid: AtomicU64::new(tid),
        in_cache: AtomicBool::new(false),
    });
    shared().threads().push(me.clone());
    me
}

/// In the child of a `fork` made by `me`, which is the only thread left. The forking
/// thread held every lock of calco across the fork, so the child finds them consistent.
pub fn forked(threads: &mut Vec<Arc<Member>>, me: &Arc<Member>) {
    me.tid.store(sys::gettid(), Ordering::Relaxed);
    // Frees nothing: the threads that do not exist in the child still hold their entries.
    threads.retain(|m| Arc::ptr_eq(m, me));
}

/// Runs the guest on the main thread until it calls `exit`; never returns.
pub fn run_main(mut cpu: Box<Cpu>, mem: Mem, jit: Jit, state: State) -> ! {
    let shared = Shared {
        index: jit.index(),
        jit: Mutex::new(jit),
        world: Mutex::new(World { mem, state }),
        threads: Mutex::new(Vec::new()),
        parked: Condvar::new(),
    };
    assert!(SHARED.set(shared).is_ok(), "the guest starts once");
    cpu.tid = sys::gettid();
    // Translated code reaches the Cpu through GS; nothing else in calco uses GS.
    unsafe { sys::set_gs_base(&mut *cpu as *mut Cpu as u64) };
    signal::init(&mut cpu, &mut self::shared().world().state.signals);
    let me = register(cpu.tid);
    let code = jit::run(&mut cpu, &me);
    finish(&mut cpu, &me);
    // The leader ends like any other thread: the process lives on while others run, and
    // its exit status is this code.
    sys::exit_thread(code)
}

/// The `Cpu` of a new thread on its way to the host thread that runs it. Its signal state
/// is still null: nothing of the parent's thread goes along.
struct Handoff(Box<Cpu>);

unsafe impl Send for Handoff {}

/// `clone` of a guest thread, called with the world lock held (the child does not need it
/// before it runs guest code). Returns the tid of the child.
pub fn clone(parent: &mut Cpu, mem: &Mem, a: [u64; 6]) -> i64 {
    let [flags, stack, ptid, ctid, tls, _] = a;
    // As `do_arch_prctl_64(ARCH_SET_FS)` for `CLONE_SETTLS`.
    if flags & CLONE_SETTLS != 0 && tls >= USER_END {
        return -(EPERM as i64);
    }
    let child = Handoff(child_cpu(parent, flags, stack, ctid, tls));
    let mask = signal::thread(parent).mask();
    let (tid_tx, tid_rx) = mpsc::channel();
    let (go_tx, go_rx) = mpsc::channel();
    // Until its GS base points to its own Cpu, calco's handler must not run on the new
    // host thread: it starts with every signal blocked.
    let old = sys::rt_sigprocmask(SIG_SETMASK as usize, Some(!0)).unwrap_or(0);
    let spawned = std::thread::Builder::new().spawn(move || child_main(child, mask, tid_tx, go_rx));
    let _ = sys::rt_sigprocmask(SIG_SETMASK as usize, Some(old));
    let tid = match spawned.map(|_| tid_rx.recv()) {
        Ok(Ok(tid)) => tid,
        _ => return -(EAGAIN as i64),
    };
    // The kernel ignores a bad address here, and so does calco.
    let tid32 = (tid as u32).to_le_bytes();
    if flags & CLONE_PARENT_SETTID != 0 {
        let _ = write_bytes(mem, ptid, &tid32);
    }
    if flags & CLONE_CHILD_SETTID != 0 {
        let _ = write_bytes(mem, ctid, &tid32);
    }
    let _ = go_tx.send(());
    tid as i64
}

/// The host thread of a new guest thread: its GS base, signal state and alternate stack,
/// its entry in the thread list, the tid to the parent, and the guest code once the parent
/// says so.
fn child_main(handoff: Handoff, mask: u64, tid_tx: Sender<u64>, go: Receiver<()>) {
    let mut cpu = handoff.0;
    cpu.tid = sys::gettid();
    unsafe { sys::set_gs_base(&mut *cpu as *mut Cpu as u64) };
    signal::spawned(&mut cpu, mask);
    let me = register(cpu.tid);
    // Only the exit code of the leader is seen (as the status of the process).
    if tid_tx.send(cpu.tid).is_ok() && go.recv().is_ok() {
        jit::run(&mut cpu, &me);
    }
    finish(&mut cpu, &me);
}

/// The end of a guest thread: out of the thread list, `clear_child_tid` written and woken
/// as `mm_release` does, and GS base 0. Every host signal stays blocked from here on, so no
/// handler runs on a thread whose `Cpu` goes away; the kernel moves the pending process
/// signals to another thread.
fn finish(cpu: &mut Cpu, me: &Arc<Member>) {
    let _ = sys::rt_sigprocmask(SIG_SETMASK as usize, Some(!0));
    let shared = shared();
    shared.threads().retain(|m| !Arc::ptr_eq(m, me));
    if cpu.clear_child_tid != 0 {
        let _ = write_bytes(
            &shared.world().mem,
            cpu.clear_child_tid,
            &0u32.to_le_bytes(),
        );
        sys::futex_wake(cpu.clear_child_tid, 1);
    }
    unsafe { sys::set_gs_base(0) };
}

#[cfg(test)]
mod tests;
