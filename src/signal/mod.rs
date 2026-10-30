//! Guest signals (contract 3.11): actions, per-thread masks and pending signals, guest
//! signal frames, `rt_sigreturn` and the decisions about interrupted system calls. The
//! host side (handler, safe system calls, host actions and mask) is in `host.rs`.
//!
//! - Numbers: guest 1–31 and 34–61 are the host ones; guest 32 and 33 are host 64 and 63;
//!   host 62 belongs to calco. Masks, `kill`-like calls, `signalfd4` and `timer_create`
//!   are translated.
//! - The host handles every signal the guest catches plus SIGSEGV, SIGBUS, SIGILL, SIGFPE
//!   and SIGTRAP. Ignored and default signals keep that action in the host, so the kernel
//!   ignores, stops or kills natively. The host mask of the thread mirrors the guest one,
//!   except that the synchronous signals are never blocked in the host.
//! - The host handler never builds a guest frame: it brings the thread back to the
//!   dispatcher at a guest instruction boundary with the exact guest state in `Cpu`, and
//!   the dispatcher calls `deliver`.

pub mod frame;
mod host;

use core::cell::{Cell, UnsafeCell};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub use self::host::{die, init, init_thread, install, safe_syscall, tf_works};

use self::frame::{AltStack, FRAME_INFO, FRAME_SIZE, FRAME_UC, SIGINFO_SIZE, UC_SIZE};
use crate::cpu::{Cpu, RAX, RDI, RDX, RSI, RSP};
use crate::jit::translate::{GuestPc, PcEntry, SCRATCH_SLOTS};
use crate::mem::Mem;
use crate::sys::{self, EINTR, EINVAL, ENOMEM, EPERM};
use crate::syscall::{accessible, guest_waitid_status, read_bytes, read_u64, write_bytes};

pub const SIGILL: u32 = 4;
pub const SIGTRAP: u32 = 5;
pub const SIGBUS: u32 = 7;
pub const SIGFPE: u32 = 8;
pub const SIGKILL: u32 = 9;
pub const SIGSEGV: u32 = 11;
pub const SIGPIPE: u32 = 13;
pub const SIGCHLD: u32 = 17;
pub const SIGSTOP: u32 = 19;
pub const SIGSYS: u32 = 31;
/// Guest signals with an action: 62–64 are not available (contract 3.11).
pub const GUEST_SIGNALS: usize = 61;
/// Host signal calco keeps to kick its own threads.
pub const HOST_KICK: u32 = 62;

// /usr/include/asm-generic/signal-defs.h, /usr/include/asm/signal.h
pub const SIG_DFL: u64 = 0;
pub const SIG_IGN: u64 = 1;
pub const SIG_BLOCK: u64 = 0;
pub const SIG_UNBLOCK: u64 = 1;
pub const SIG_SETMASK: u64 = 2;
pub const SA_NOCLDSTOP: u64 = 0x1;
pub const SA_NOCLDWAIT: u64 = 0x2;
pub const SA_SIGINFO: u64 = 0x4;
pub const SA_EXPOSE_TAGBITS: u64 = 0x800;
pub const SA_RESTORER: u64 = 0x0400_0000;
pub const SA_ONSTACK: u64 = 0x0800_0000;
pub const SA_RESTART: u64 = 0x1000_0000;
pub const SA_NODEFER: u64 = 0x4000_0000;
pub const SA_RESETHAND: u64 = 0x8000_0000;
/// `UAPI_SA_FLAGS` of x86: the flags `rt_sigaction` keeps.
const SA_KNOWN: u64 = SA_NOCLDSTOP
    | SA_NOCLDWAIT
    | SA_SIGINFO
    | SA_ONSTACK
    | SA_RESTART
    | SA_NODEFER
    | SA_RESETHAND
    | SA_EXPOSE_TAGBITS
    | SA_RESTORER;

// /usr/include/asm-generic/siginfo.h
pub const SI_KERNEL: i32 = 0x80;
pub const ILL_ILLOPN: i32 = 2;
pub const SEGV_MAPERR: i32 = 1;
pub const SEGV_ACCERR: i32 = 2;
// /usr/include/asm-generic/siginfo.h: `struct sigevent`
const SIGEV_NONE: i32 = 1;
const SIGEVENT_SIZE: usize = 64;
const SIGEV_SIGNO: usize = 8;
const SIGEV_NOTIFY: usize = 12;

// RFLAGS bits (Intel SDM vol. 1, 3.4.3)
pub const TF: u64 = 1 << 8;
pub const DF: u64 = 1 << 10;
pub const RF: u64 = 1 << 16;
pub const AC: u64 = 1 << 18;
/// `FIX_EFLAGS` of the kernel: the RFLAGS bits `rt_sigreturn` takes from the frame.
pub const FIX_EFLAGS: u64 =
    AC | (1 << 11) | DF | TF | (1 << 7) | (1 << 6) | (1 << 4) | (1 << 2) | 1 | RF;

/// What the safe system call returns when a signal came before the `syscall` ran
/// (`ERESTARTSYS`, never seen by the guest).
pub const ERESTART_INTERNAL: i64 = 512;

pub const fn bit(sig: u32) -> u64 {
    1 << (sig - 1)
}

pub const UNBLOCKABLE: u64 = bit(SIGKILL) | bit(SIGSTOP);
/// Signals the CPU raises. Never blocked in the host, so a fault always reaches calco.
pub const SYNC_SIGNALS: u64 = bit(SIGILL) | bit(SIGTRAP) | bit(SIGBUS) | bit(SIGFPE) | bit(SIGSEGV);
/// `SYNCHRONOUS_MASK` of the kernel: dequeued before the rest.
const SYNCHRONOUS_MASK: u64 = SYNC_SIGNALS | bit(SIGSYS);

/// Host number of guest signal `sig` (contract 3.11): 32 and 33 move to 64 and 63; 0 is
/// the existence check; 62–64 are not available.
pub fn host_signal(sig: u64) -> Option<u64> {
    match sig {
        0..=31 | 34..=61 => Some(sig),
        32 => Some(64),
        33 => Some(63),
        _ => None,
    }
}

/// Guest number of host signal `sig`: 64 and 63 go back to 32 and 33.
pub fn guest_signal(sig: u32) -> u32 {
    match sig {
        64 => 32,
        63 => 33,
        s => s,
    }
}

/// Host set of the guest set `guest`; guest 62–64 have no host signal.
pub fn host_set(guest: u64) -> u64 {
    (1..=64u32)
        .filter(|&g| guest & bit(g) != 0)
        .filter_map(|g| host_signal(g as u64))
        .fold(0, |set, h| set | bit(h as u32))
}

/// Guest set of the host set `host`. Host 32 and 33 (no guest number reaches them) and
/// calco's 62 are left out.
pub fn guest_set(host: u64) -> u64 {
    (1..=64u32)
        .filter(|&h| host & bit(h) != 0 && !matches!(h, 32 | 33 | HOST_KICK))
        .fold(0, |set, h| set | bit(guest_signal(h)))
}

/// Host mask of a thread whose guest mask is `mask`.
pub fn host_mask(mask: u64) -> u64 {
    host_set(mask) & !SYNC_SIGNALS & !bit(HOST_KICK)
}

/// Default action of a guest signal (signal(7)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefaultAction {
    Term,
    Core,
    Ignore,
    Stop,
}

pub fn default_action(sig: u32) -> DefaultAction {
    match sig {
        SIGCHLD | 18 | 23 | 28 => DefaultAction::Ignore,
        SIGSTOP | 20 | 21 | 22 => DefaultAction::Stop,
        3 | SIGILL | SIGTRAP | 6 | SIGBUS | SIGFPE | SIGSEGV | 24 | 25 | SIGSYS => {
            DefaultAction::Core
        }
        _ => DefaultAction::Term,
    }
}

/// Guest `struct sigaction` as `rt_sigaction` takes it on x86-64.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SigAction {
    pub handler: u64,
    pub flags: u64,
    pub restorer: u64,
    pub mask: u64,
}

/// Raw `siginfo_t` with the guest signal number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SigInfo(pub [u8; SIGINFO_SIZE]);

impl SigInfo {
    /// What the kernel fills for a signal it raises itself.
    pub fn kernel(sig: u32, code: i32, addr: u64) -> SigInfo {
        let mut b = [0u8; SIGINFO_SIZE];
        frame::put32(&mut b, frame::SI_SIGNO, sig);
        frame::put32(&mut b, frame::SI_CODE, code as u32);
        frame::put64(&mut b, frame::SI_ADDR, addr);
        SigInfo(b)
    }

    pub fn code(&self) -> i32 {
        frame::get32(&self.0, frame::SI_CODE) as i32
    }

    pub fn addr(&self) -> u64 {
        frame::get64(&self.0, frame::SI_ADDR)
    }

    /// The host `siginfo_t` of host signal `sig` as the guest sees it: its number and, for
    /// SIGCHLD, the signal in `si_status` translated.
    pub fn from_host(raw: &[u8; SIGINFO_SIZE], sig: u32) -> SigInfo {
        let mut info = SigInfo(*raw);
        let guest = guest_signal(sig);
        frame::put32(&mut info.0, frame::SI_SIGNO, guest);
        if guest == SIGCHLD {
            let status = frame::get32(raw, frame::SI_STATUS) as i32;
            let status = guest_waitid_status(info.code(), status);
            frame::put32(&mut info.0, frame::SI_STATUS, status as u32);
        }
        info
    }
}

/// A synchronous signal ready to be delivered, with the trap data of the sigcontext.
#[derive(Clone, Copy, Debug)]
pub struct SyncSignal {
    pub sig: u32,
    pub info: SigInfo,
    pub trapno: u64,
    pub err: u64,
    pub cr2: u64,
}

/// Signal state of one guest thread, pointed to by `Cpu.signal_state`. The host handler
/// of this thread touches only the atomics, `infos[sig]` of a signal it then blocks in the
/// host, and `fault` while the thread is in translated code.
pub struct Thread {
    mask: AtomicU64,
    /// Signals the host handler took and the guest has not been given yet.
    queued: AtomicU64,
    infos: UnsafeCell<[SigInfo; 64]>,
    fault: UnsafeCell<Option<SyncSignal>>,
    /// Single-stepping translated code with TF up to the next guest boundary.
    stepping: AtomicBool,
    altstack: Cell<AltStack>,
    /// Mask to restore after `rt_sigsuspend`.
    saved_mask: Cell<Option<u64>>,
    /// A system call that ended with `EINTR` or did not run: number and result.
    interrupted: Cell<Option<(u64, i64)>>,
}

/// The word `Cpu.pending_signals`: guest signals queued and not blocked.
fn pending_word(cpu: *mut Cpu) -> &'static AtomicU64 {
    // The Cpu lives as long as its thread; the handler and the thread share the word.
    unsafe { AtomicU64::from_ptr(&raw mut (*cpu).pending_signals) }
}

impl Thread {
    fn new(mask: u64) -> Thread {
        Thread {
            mask: AtomicU64::new(mask & !UNBLOCKABLE),
            queued: AtomicU64::new(0),
            infos: UnsafeCell::new([SigInfo([0; SIGINFO_SIZE]); 64]),
            fault: UnsafeCell::new(None),
            stepping: AtomicBool::new(false),
            altstack: Cell::new(AltStack::DISABLED),
            saved_mask: Cell::new(None),
            interrupted: Cell::new(None),
        }
    }

    pub fn mask(&self) -> u64 {
        self.mask.load(Ordering::Relaxed)
    }

    pub fn queued(&self) -> u64 {
        self.queued.load(Ordering::Relaxed)
    }

    /// Sets the guest mask and mirrors it to the host mask of the thread.
    pub fn set_mask(&self, cpu: *mut Cpu, mask: u64) {
        self.set_mask_quiet(cpu, mask);
        host::set_host_mask(host_mask(self.mask()));
    }

    /// Sets the guest mask and the deliverable summary, leaving the host mask as it is.
    fn set_mask_quiet(&self, cpu: *mut Cpu, mask: u64) {
        self.mask.store(mask & !UNBLOCKABLE, Ordering::Relaxed);
        self.refresh(cpu);
    }

    /// Recomputes `Cpu.pending_signals`; repeats if the handler queued a signal meanwhile.
    fn refresh(&self, cpu: *mut Cpu) {
        let pending = pending_word(cpu);
        loop {
            let queued = self.queued.load(Ordering::Acquire);
            pending.store(queued & !self.mask(), Ordering::Release);
            if self.queued.load(Ordering::Acquire) == queued {
                break;
            }
        }
    }

    /// Queues guest signal `sig` (host handler, every host signal blocked). Returns
    /// whether the guest can take it now.
    fn queue(&self, cpu: *mut Cpu, sig: u32, info: SigInfo) -> bool {
        // The host blocks `sig` until it is taken, so nothing reads this slot now.
        unsafe { (*self.infos.get())[sig as usize - 1] = info };
        self.queued.fetch_or(bit(sig), Ordering::AcqRel);
        if self.mask() & bit(sig) != 0 {
            return false;
        }
        pending_word(cpu).fetch_or(bit(sig), Ordering::AcqRel);
        true
    }

    /// Takes the next queued signal of `set` that the mask lets through: synchronous ones
    /// first, then the lowest number, as the kernel dequeues.
    fn take(&self, cpu: *mut Cpu, set: u64) -> Option<(u32, SigInfo)> {
        let ready = self.queued() & set;
        if ready == 0 {
            return None;
        }
        let first = if ready & SYNCHRONOUS_MASK != 0 {
            ready & SYNCHRONOUS_MASK
        } else {
            ready
        };
        let sig = first.trailing_zeros() + 1;
        let info = unsafe { (*self.infos.get())[sig as usize - 1] };
        self.queued.fetch_and(!bit(sig), Ordering::AcqRel);
        self.refresh(cpu);
        Some((sig, info))
    }

    /// Drops a queued `sig` (its action became ignore).
    fn discard(&self, cpu: *mut Cpu, sig: u32) {
        self.queued.fetch_and(!bit(sig), Ordering::AcqRel);
        self.refresh(cpu);
    }

    fn set_fault(&self, fault: SyncSignal) {
        unsafe { self.fault.get().write_volatile(Some(fault)) };
    }

    fn take_fault(&self) -> Option<SyncSignal> {
        unsafe { self.fault.get().replace(None) }
    }

    fn has_fault(&self) -> bool {
        unsafe { self.fault.get().read_volatile().is_some() }
    }

    pub fn altstack(&self) -> AltStack {
        self.altstack.get()
    }
}

/// The signal state of the thread of `cpu`, created on first use.
pub fn thread(cpu: &mut Cpu) -> &'static Thread {
    if cpu.signal_state.is_null() {
        cpu.signal_state = Box::into_raw(Box::new(Thread::new(0))).cast();
    }
    // Never freed: a thread's state lives until the process ends.
    unsafe { &*(cpu.signal_state as *const Thread) }
}

/// Signal state of a new thread of `clone`: the guest mask of its parent, nothing pending
/// and no alternate stack (clone(2)); then calco's alternate stack and the host mask that
/// mirrors the guest one. The thread comes with every host signal blocked.
pub fn spawned(cpu: &mut Cpu, mask: u64) {
    cpu.signal_state = Box::into_raw(Box::new(Thread::new(mask))).cast();
    init_thread(cpu);
    host::set_host_mask(host_mask(thread(cpu).mask()));
}

/// The child of a `fork`: no pending signals, no interrupted call (signal(7)).
pub fn forked(cpu: &mut Cpu) {
    let th = thread(cpu);
    th.queued.store(0, Ordering::Relaxed);
    th.interrupted.set(None);
    th.refresh(cpu);
}

/// Whether the dispatcher has to call `deliver` before the next block.
pub fn wants_delivery(cpu: &mut Cpu) -> bool {
    if pending_word(cpu).load(Ordering::Acquire) != 0 {
        return true;
    }
    let th = thread(cpu);
    th.has_fault() || th.interrupted.get().is_some() || th.saved_mask.get().is_some()
}

fn err(errno: i32) -> i64 {
    -(errno as i64)
}

// --- State reconstruction from the pc table (contract 3.5, 3.11) ---

/// The pc table entry that covers host offset `off` of a block.
pub fn entry_at(pcs: &[PcEntry], off: u32) -> Option<&PcEntry> {
    let n = pcs.partition_point(|e| e.host_off <= off);
    n.checked_sub(1).map(|i| &pcs[i])
}

/// Whether host offset `off` starts a guest instruction boundary with no temporaries in
/// scratch: every register holds its guest value and nothing of the next guest
/// instruction has run.
pub fn at_boundary(pcs: &[PcEntry], off: u32) -> bool {
    pcs.binary_search_by_key(&off, |e| e.host_off)
        .is_ok_and(|i| pcs[i].temps.iter().all(Option::is_none))
}

/// Puts back the guest values of the registers that `entry` says live in scratch, and
/// returns the guest rip of the boundary.
pub fn reconstruct(
    entry: &PcEntry,
    gpr: &mut [u64; 16],
    scratch: &[u64; SCRATCH_SLOTS],
    next_rip: u64,
) -> u64 {
    for (slot, reg) in entry.temps.iter().enumerate() {
        if let Some(reg) = reg {
            gpr[*reg as usize] = scratch[slot];
        }
    }
    match entry.pc {
        GuestPc::At(rip) => rip,
        GuestPc::NextRip => next_rip,
    }
}

// --- Synchronous signals from the dispatcher ---

/// A translated trap exit (`ud2`, invalid instruction, `int3`, `int n`, `hlt`…): the
/// guest gets `sig` as the kernel raises it, with `cpu.rip` already where the kernel
/// leaves it.
pub fn trap(cpu: &mut Cpu, sig: u32) {
    let (code, addr, trapno) = match sig {
        // #UD: `do_error_trap` with the address of the instruction.
        SIGILL => (ILL_ILLOPN, cpu.rip, 6),
        // #BP: `do_int3_user` raises it with `force_sig`.
        SIGTRAP => (SI_KERNEL, 0, 3),
        // #GP: `gp_user_force_sig_segv`.
        _ => (SI_KERNEL, 0, 13),
    };
    thread(cpu).set_fault(SyncSignal {
        sig,
        info: SigInfo::kernel(sig, code, addr),
        trapno,
        err: 0,
        cr2: 0,
    });
}

/// The block at `cpu.rip` cannot be fetched: SIGSEGV on `addr`, the first byte without
/// execute permission, as an instruction fetch page fault gives it.
pub fn exec_fault(cpu: &mut Cpu, mem: &Mem, sig: u32, addr: u64) {
    let mapped = mem.intervals.find(addr).is_some();
    let code = if mapped { SEGV_ACCERR } else { SEGV_MAPERR };
    // Page fault error code: user (4) | instruction fetch (0x10) | present (1).
    let err = 0x14 | mapped as u64;
    thread(cpu).set_fault(SyncSignal {
        sig,
        info: SigInfo::kernel(sig, code, addr),
        trapno: 14,
        err,
        cr2: addr,
    });
}

fn force_segv(cpu: &mut Cpu) {
    thread(cpu).set_fault(SyncSignal {
        sig: SIGSEGV,
        info: SigInfo::kernel(SIGSEGV, SI_KERNEL, 0),
        trapno: 0,
        err: 0,
        cr2: 0,
    });
}

// --- Interrupted system calls (contract 3.11, signal(7)) ---

/// Calls the kernel restarts after a handler with `SA_RESTART`.
const RESTART_WITH_SA: &[u64] = &[
    0, 1, 19, 20, 17, 18, 295, 296, // read write readv writev pread64 pwrite64 preadv pwritev
    16, 2, 257, // ioctl open openat
    61, 247, // wait4 waitid
    43, 288, 42, 45, 47, 44, 46, // accept accept4 connect recvfrom recvmsg sendto sendmsg
    73, 72, 202, 318, // flock fcntl futex getrandom
    40, 275, 326, // sendfile splice copy_file_range
];

/// Whether system call `nr`, which returned `ret` (`-EINTR`, or `-ERESTART_INTERNAL` if
/// it never ran), runs again after the guest gets the signal: always if it never ran (the
/// signal came before the `syscall`, as natively) or if no handler runs; after a handler,
/// by `SA_RESTART` for the restartable calls and never for the rest (`poll`, `select`,
/// `epoll_wait`, `nanosleep`, `pause`, `rt_sigsuspend`, `rt_sigtimedwait`…).
pub fn restarts(nr: u64, ret: i64, handler_flags: Option<u64>) -> bool {
    match handler_flags {
        _ if ret == -ERESTART_INTERNAL => true,
        None => true,
        Some(flags) if RESTART_WITH_SA.contains(&nr) => flags & SA_RESTART != 0,
        Some(_) => false,
    }
}

/// Notes the result of guest system call `nr`; returns what the guest sees in RAX until
/// `deliver` decides.
pub fn note_result(cpu: &mut Cpu, nr: u64, ret: i64) -> i64 {
    if ret == -(EINTR as i64) || ret == -ERESTART_INTERNAL {
        thread(cpu).interrupted.set(Some((nr, ret)));
        return -(EINTR as i64);
    }
    ret
}

/// Restarts the interrupted call (rip back on the 2-byte `syscall`, RAX the number) or
/// leaves `-EINTR`.
fn finish_interrupted(cpu: &mut Cpu, nr: u64, ret: i64, handler_flags: Option<u64>) {
    if restarts(nr, ret, handler_flags) {
        cpu.rip -= 2;
        cpu.gpr[RAX] = nr;
    } else {
        cpu.gpr[RAX] = -(EINTR as i64) as u64;
    }
}

// --- Actions and emulated calls ---

/// Process-wide signal state: the guest actions.
pub struct Process {
    pub actions: [SigAction; GUEST_SIGNALS],
}

impl Default for Process {
    fn default() -> Process {
        Process::new()
    }
}

impl Process {
    pub fn new() -> Process {
        Process {
            actions: [SigAction::default(); GUEST_SIGNALS],
        }
    }

    pub fn action(&self, sig: u32) -> SigAction {
        self.actions
            .get(sig as usize - 1)
            .copied()
            .unwrap_or_default()
    }

    /// `rt_sigaction`.
    pub fn sigaction(&mut self, cpu: &mut Cpu, mem: &Mem, a: [u64; 6]) -> i64 {
        let [sig, act, oact, size, ..] = a;
        if size != 8 || !(1..=GUEST_SIGNALS as u64).contains(&sig) {
            return err(EINVAL);
        }
        let sig = sig as u32;
        let new = if act != 0 {
            let bytes = match read_bytes(mem, act, 32) {
                Ok(b) => b,
                Err(e) => return err(e),
            };
            if sig == SIGKILL || sig == SIGSTOP {
                return err(EINVAL);
            }
            Some(SigAction {
                handler: frame::get64(&bytes, 0),
                flags: frame::get64(&bytes, 8) & SA_KNOWN,
                restorer: frame::get64(&bytes, 16),
                mask: frame::get64(&bytes, 24) & !UNBLOCKABLE,
            })
        } else {
            None
        };
        let old = self.actions[sig as usize - 1];
        if let Some(action) = new {
            self.actions[sig as usize - 1] = action;
            install(sig, &action);
            let ignored = action.handler == SIG_IGN
                || (action.handler == SIG_DFL && default_action(sig) == DefaultAction::Ignore);
            if ignored {
                thread(cpu).discard(cpu, sig);
            }
        }
        if oact != 0 {
            let mut out = [0u8; 32];
            frame::put64(&mut out, 0, old.handler);
            frame::put64(&mut out, 8, old.flags);
            frame::put64(&mut out, 16, old.restorer);
            frame::put64(&mut out, 24, old.mask);
            if let Err(e) = write_bytes(mem, oact, &out) {
                return err(e);
            }
        }
        0
    }

    /// Takes the next signal and builds its frame if the guest handles it; the default
    /// and ignore actions are applied on the way. Called by the dispatcher before entering
    /// the next block when `wants_delivery` says so.
    pub fn deliver(&mut self, cpu: &mut Cpu, mem: &Mem) {
        let th = thread(cpu);
        let mut interrupted = th.interrupted.take();
        loop {
            let (sig, info, trap) = if let Some(f) = th.take_fault() {
                let act = self.action(f.sig);
                // A fault the guest blocks or ignores kills, as `force_sig_fault` does.
                if th.mask() & bit(f.sig) != 0 || act.handler == SIG_IGN {
                    die(f.sig);
                }
                (f.sig, f.info, (f.trapno, f.err, f.cr2))
            } else if let Some((sig, info)) = th.take(cpu, !th.mask()) {
                (sig, info, (0, 0, 0))
            } else {
                break;
            };
            let act = self.action(sig);
            match act.handler {
                SIG_IGN => {}
                SIG_DFL => match default_action(sig) {
                    DefaultAction::Ignore => {}
                    DefaultAction::Stop => host::stop(sig),
                    DefaultAction::Term | DefaultAction::Core => die(sig),
                },
                _ => {
                    if let Some((nr, ret)) = interrupted.take() {
                        finish_interrupted(cpu, nr, ret, Some(act.flags));
                    }
                    let old = th.saved_mask.take().unwrap_or(th.mask());
                    if setup_frame(cpu, mem, th, sig, &info, trap, &act, old).is_none() {
                        die(SIGSEGV);
                    }
                    let mut mask = old | act.mask;
                    if act.flags & SA_NODEFER == 0 {
                        mask |= bit(sig);
                    }
                    if act.flags & SA_RESETHAND != 0 {
                        self.actions[sig as usize - 1].handler = SIG_DFL;
                        install(sig, &self.actions[sig as usize - 1]);
                    }
                    th.set_mask(cpu, mask);
                    return;
                }
            }
        }
        if let Some((nr, ret)) = interrupted {
            finish_interrupted(cpu, nr, ret, None);
        }
        let mask = th.saved_mask.take().unwrap_or(th.mask());
        // Also unblocks in the host the signals the handler queued and blocked.
        th.set_mask(cpu, mask);
    }

    /// `rt_sigreturn`: the guest state, mask and alternate stack of the frame below RSP.
    /// RAX ends as the restored one; RCX and R11 are not touched.
    pub fn sigreturn(&mut self, cpu: &mut Cpu, mem: &Mem) {
        let th = thread(cpu);
        let frame = cpu.gpr[RSP].wrapping_sub(8);
        let uc = match read_bytes(mem, frame + FRAME_UC as u64, UC_SIZE as u64) {
            Ok(uc) if accessible(mem, frame, FRAME_SIZE as u64, false) => uc,
            _ => return force_segv(cpu),
        };
        let r = frame::restore(&uc);
        th.set_mask(cpu, r.mask);
        let fx = if r.fpstate == 0 {
            frame::fx_init()
        } else {
            match read_bytes(mem, r.fpstate, frame::FP_SIZE as u64) {
                Ok(b) => {
                    let mut fx: [u8; frame::FP_SIZE] = b.try_into().unwrap();
                    // MXCSR bits above the ones the CPU has (`mxcsr_feature_mask`).
                    let mxcsr = frame::get32(&fx, frame::FP_MXCSR) & 0xffff;
                    frame::put32(&mut fx, frame::FP_MXCSR, mxcsr);
                    fx
                }
                Err(_) => return force_segv(cpu),
            }
        };
        cpu.gpr = r.gpr;
        cpu.rip = r.rip;
        cpu.rflags = (cpu.rflags & !FIX_EFLAGS) | (r.eflags & FIX_EFLAGS);
        cpu.fxsave.0 = fx;
        // Like `restore_altstack`: errors other than EFAULT are dropped.
        let (sp, flags, size) = r.stack;
        let _ = set_altstack(th, sp, flags, size, cpu.gpr[RSP]);
    }
}

/// Builds the frame of `sig` for handler `act` and points the guest at the handler, as
/// `__setup_rt_frame` and `handle_signal` do. `None` if the frame cannot be written.
#[allow(clippy::too_many_arguments)]
fn setup_frame(
    cpu: &mut Cpu,
    mem: &Mem,
    th: &Thread,
    sig: u32,
    info: &SigInfo,
    (trapno, err, cr2): (u64, u64, u64),
    act: &SigAction,
    mask: u64,
) -> Option<()> {
    // x86-64 frames always return through `sa_restorer`.
    if act.flags & SA_RESTORER == 0 {
        return None;
    }
    let alt = th.altstack();
    let (frame, fpstate) = frame::place(cpu.gpr[RSP], act.flags & SA_ONSTACK != 0, &alt)?;
    let saved = frame::Saved {
        gpr: &cpu.gpr,
        rip: cpu.rip,
        rflags: cpu.rflags,
        err,
        trapno,
        cr2,
    };
    let bytes = frame::build(&saved, mask, &alt, act.restorer, fpstate, &info.0);
    // Without SA_SIGINFO the kernel leaves the siginfo part alone.
    let len = if act.flags & SA_SIGINFO != 0 {
        FRAME_SIZE
    } else {
        FRAME_INFO
    };
    write_bytes(mem, frame, &bytes[..len]).ok()?;
    write_bytes(mem, fpstate, &frame::fpstate(&cpu.fxsave.0)).ok()?;
    if alt.flags & frame::SS_AUTODISARM != 0 {
        th.altstack.set(AltStack::DISABLED);
    }
    cpu.gpr[RDI] = sig as u64;
    cpu.gpr[RAX] = 0;
    cpu.gpr[RSI] = frame + FRAME_INFO as u64;
    cpu.gpr[RDX] = frame + FRAME_UC as u64;
    cpu.gpr[RSP] = frame;
    cpu.rip = act.handler;
    cpu.rflags &= !(DF | RF | TF);
    cpu.fxsave.0 = frame::fx_init();
    Some(())
}

/// `do_sigaltstack` with a new stack, for a thread whose stack pointer is `sp`.
fn set_altstack(th: &Thread, ss_sp: u64, ss_flags: u32, ss_size: u64, sp: u64) -> Result<(), i32> {
    let old = th.altstack();
    if old.on(sp) {
        return Err(EPERM);
    }
    let mode = ss_flags & !frame::SS_AUTODISARM;
    if mode != frame::SS_DISABLE && mode != frame::SS_ONSTACK && mode != 0 {
        return Err(EINVAL);
    }
    let new = if mode == frame::SS_DISABLE {
        AltStack {
            sp: 0,
            size: 0,
            flags: ss_flags,
        }
    } else if ss_size < frame::MINSIGSTKSZ {
        return Err(ENOMEM);
    } else {
        AltStack {
            sp: ss_sp,
            size: ss_size,
            flags: ss_flags,
        }
    };
    th.altstack.set(new);
    Ok(())
}

/// `sigaltstack`.
pub fn sigaltstack(cpu: &mut Cpu, mem: &Mem, ss: u64, oss: u64) -> i64 {
    let th = thread(cpu);
    let sp = cpu.gpr[RSP];
    let new = if ss != 0 {
        match read_bytes(mem, ss, frame::STACK_T_SIZE as u64) {
            Ok(b) => Some((
                frame::get64(&b, frame::SS_SP),
                frame::get32(&b, frame::SS_FLAGS),
                frame::get64(&b, frame::SS_SIZE),
            )),
            Err(e) => return err(e),
        }
    } else {
        None
    };
    let old = th.altstack();
    if let Some((ss_sp, flags, size)) = new
        && let Err(e) = set_altstack(th, ss_sp, flags, size, sp)
    {
        return err(e);
    }
    if oss != 0 {
        let mut out = [0u8; frame::STACK_T_SIZE];
        frame::put64(&mut out, frame::SS_SP, old.sp);
        let flags = old.mode(sp) | (old.flags & frame::SS_AUTODISARM);
        frame::put32(&mut out, frame::SS_FLAGS, flags);
        frame::put64(&mut out, frame::SS_SIZE, old.size);
        if let Err(e) = write_bytes(mem, oss, &out) {
            return err(e);
        }
    }
    0
}

/// `rt_sigprocmask`.
pub fn sigprocmask(cpu: &mut Cpu, mem: &Mem, how: u64, set: u64, oset: u64, size: u64) -> i64 {
    if size != 8 {
        return err(EINVAL);
    }
    let th = thread(cpu);
    let old = th.mask();
    if set != 0 {
        let s = match read_u64(mem, set) {
            Ok(s) => s,
            Err(e) => return err(e),
        };
        let new = match how {
            SIG_BLOCK => old | s,
            SIG_UNBLOCK => old & !s,
            SIG_SETMASK => s,
            _ => return err(EINVAL),
        };
        th.set_mask(cpu, new);
    }
    if oset != 0
        && let Err(e) = write_bytes(mem, oset, &old.to_le_bytes())
    {
        return err(e);
    }
    0
}

/// `rt_sigpending`: blocked signals pending in the host or queued by calco.
pub fn sigpending(cpu: &mut Cpu, mem: &Mem, set: u64, size: u64) -> i64 {
    if size > 8 {
        return err(EINVAL);
    }
    let th = thread(cpu);
    let host = host::host_pending();
    let pending = (guest_set(host) | th.queued()) & th.mask();
    match write_bytes(mem, set, &pending.to_le_bytes()[..size as usize]) {
        Ok(()) => 0,
        Err(e) => err(e),
    }
}

/// The guest `sigset_t` at `set` of a call that takes it with its size (8).
pub fn read_set(mem: &Mem, set: u64, size: u64) -> Result<u64, i64> {
    if size != 8 {
        return Err(err(EINVAL));
    }
    read_u64(mem, set).map_err(err)
}

/// `rt_sigsuspend`.
pub fn sigsuspend(cpu: &mut Cpu, mem: &Mem, set: u64, size: u64) -> i64 {
    read_set(mem, set, size).map_or_else(|e| e, |s| suspend(cpu, s))
}

/// `rt_sigsuspend` with the set already read: waits with the guest mask `s` in the host
/// `rt_sigsuspend` (safe, so a signal already queued ends it at once). The old mask goes in
/// the frame of the signal that ends it, and comes back afterwards. Always `-EINTR`: a
/// signal that the new mask lets through is what ends the wait, also when it came before
/// the host call. Takes no lock: other threads go on meanwhile.
pub fn suspend(cpu: &mut Cpu, s: u64) -> i64 {
    let th = thread(cpu);
    th.saved_mask.set(Some(th.mask()));
    th.set_mask_quiet(cpu, s);
    let host = host_mask(th.mask());
    safe_syscall(
        sys::SYS_RT_SIGSUSPEND as u64,
        [&host as *const u64 as u64, 8, 0, 0, 0, 0],
    );
    -(EINTR as i64)
}

/// The first step of `rt_sigtimedwait`: finished at once, or the host set to wait for.
pub enum TimedWait {
    Done(i64),
    Host(u64),
}

/// `rt_sigtimedwait`: a signal of `set` queued by calco first, then the host call.
pub fn sigtimedwait(cpu: &mut Cpu, mem: &Mem, a: [u64; 6]) -> i64 {
    match timedwait_queued(cpu, mem, a) {
        TimedWait::Done(ret) => ret,
        TimedWait::Host(host) => {
            let ret = timedwait_host(host, a);
            timedwait_info(mem, a, ret)
        }
    }
}

/// Reads the guest set and takes a signal of it that calco already queued.
pub fn timedwait_queued(cpu: &mut Cpu, mem: &Mem, a: [u64; 6]) -> TimedWait {
    let [set, info, _, size, ..] = a;
    let s = match read_set(mem, set, size) {
        Ok(s) => s & !UNBLOCKABLE,
        Err(e) => return TimedWait::Done(e),
    };
    let th = thread(cpu);
    if let Some((sig, sinfo)) = th.take(cpu, s) {
        if info != 0
            && let Err(e) = write_bytes(mem, info, &sinfo.0)
        {
            return TimedWait::Done(err(e));
        }
        return TimedWait::Done(sig as i64);
    }
    TimedWait::Host(host_set(s))
}

/// The host `rt_sigtimedwait` on the host set `host`; takes no lock, so other threads go
/// on meanwhile.
pub fn timedwait_host(host: u64, a: [u64; 6]) -> i64 {
    let [_, info, timeout, ..] = a;
    let args = [&host as *const u64 as u64, info, timeout, 8, 0, 0];
    safe_syscall(sys::SYS_RT_SIGTIMEDWAIT as u64, args)
}

/// The result of the host call `ret` as the guest sees it: its signal number, in the
/// return value and in the `siginfo_t`.
pub fn timedwait_info(mem: &Mem, a: [u64; 6], ret: i64) -> i64 {
    if ret <= 0 {
        return ret;
    }
    let info = a[1];
    let sig = ret as u32;
    if info != 0
        && let Ok(raw) = read_bytes(mem, info, SIGINFO_SIZE as u64)
    {
        let raw: [u8; SIGINFO_SIZE] = raw.try_into().unwrap();
        let _ = write_bytes(mem, info, &SigInfo::from_host(&raw, sig).0);
    }
    guest_signal(sig) as i64
}

/// `rt_sigqueueinfo` (`tgid, sig, info`) and `rt_tgsigqueueinfo` (`tgid, tid, sig, info`):
/// the number translated, in the arguments and in `si_signo`.
pub fn sigqueueinfo(mem: &Mem, nr: u64, a: [u64; 6]) -> i64 {
    let (sig_arg, info_arg) = if nr == sys::SYS_RT_SIGQUEUEINFO as u64 {
        (1, 2)
    } else {
        (2, 3)
    };
    let Some(host) = host_signal(a[sig_arg]) else {
        return err(EINVAL);
    };
    let mut info = match read_bytes(mem, a[info_arg], SIGINFO_SIZE as u64) {
        Ok(b) => b,
        Err(e) => return err(e),
    };
    frame::put32(&mut info, frame::SI_SIGNO, host as u32);
    let mut args = a;
    args[sig_arg] = host;
    args[info_arg] = info.as_ptr() as u64;
    safe_syscall(nr, args)
}

/// `signalfd4` with the mask translated.
pub fn signalfd4(mem: &Mem, nr: u64, a: [u64; 6]) -> i64 {
    if a[2] != 8 {
        return err(EINVAL);
    }
    let host = match read_u64(mem, a[1]) {
        Ok(s) => host_set(s) & !bit(HOST_KICK),
        Err(e) => return err(e),
    };
    safe_syscall(nr, [a[0], &host as *const u64 as u64, 8, a[3], 0, 0])
}

/// `timer_create` with the signal of its `sigevent` translated.
pub fn timer_create(mem: &Mem, nr: u64, a: [u64; 6]) -> i64 {
    if a[1] == 0 {
        return safe_syscall(nr, a);
    }
    let mut event = match read_bytes(mem, a[1], SIGEVENT_SIZE as u64) {
        Ok(b) => b,
        Err(e) => return err(e),
    };
    if frame::get32(&event, SIGEV_NOTIFY) as i32 != SIGEV_NONE {
        let signo = frame::get32(&event, SIGEV_SIGNO) as i32;
        let host = match u64::try_from(signo).ok().and_then(host_signal) {
            Some(h) => h,
            None => return err(EINVAL),
        };
        frame::put32(&mut event, SIGEV_SIGNO, host as u32);
    }
    let mut args = a;
    args[1] = event.as_ptr() as u64;
    safe_syscall(nr, args)
}

#[cfg(test)]
mod tests;
