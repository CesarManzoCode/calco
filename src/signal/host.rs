//! The host side of guest signals (contract 3.11): host actions and mask, the per-thread
//! alternate stack, the assembly entry of the host handler, the handler itself and the
//! safe system call.
//!
//! The handler runs on calco's alternate stack (the guest stack and its red zone stay
//! untouched) with every host signal blocked. Its entry stub restores calco's FS base
//! before Rust runs and installs on the way out the one the handler returns. What the
//! handler does depends on where the signal found the thread:
//!
//! - A fault (SIGSEGV, SIGBUS, SIGILL, SIGFPE, SIGTRAP raised by the CPU) in a translated
//!   block: the pc table gives the guest rip and the temporaries in scratch; since every
//!   recoded instruction faults before having any effect, that is the exact guest state.
//!   It goes to `Cpu` with the signal, and the thread resumes in the dispatcher.
//! - A fault anywhere else is a calco bug: the process dies by that signal.
//! - An asynchronous signal is queued and blocked in the host until `deliver` takes it. If
//!   the guest blocks it, nothing else happens. Otherwise: in the safe system call before
//!   the `syscall`, the call is skipped with `ERESTART_INTERNAL`; in `enter` after the
//!   point of no return, or in a block at a guest boundary, the thread resumes in the
//!   dispatcher; in the middle of an expansion, TF steps host instructions up to the next
//!   boundary (or, if the TF probe failed at start, the block is unlinked and leaves to
//!   the dispatcher at its end); in Rust, the dispatcher sees `Cpu.pending_signals`.

use core::arch::naked_asm;
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering};

use super::frame::{
    self, FCW_INIT, FP_CWD, FP_MXCSR, FP_SIZE, FP_SW_RESERVED, FP_SWD, FP_TWD, FP_XFEATURES,
    FP_XMM, FP_XSTATE_MAGIC1, MXCSR_INIT, SIGINFO_SIZE, UC_CR2, UC_EFLAGS, UC_ERR, UC_FPSTATE,
    UC_RIP, UC_SIGMASK, UC_TRAPNO, uc_gpr,
};
use super::{
    AC, DF, ERESTART_INTERNAL, GUEST_SIGNALS, HOST_KICK, Process, SA_NOCLDSTOP, SA_NOCLDWAIT,
    SA_ONSTACK, SA_RESTORER, SA_SIGINFO, SIG_DFL, SIG_IGN, SIGBUS, SIGFPE, SIGILL, SIGPIPE,
    SIGSEGV, SIGTRAP, SYNC_SIGNALS, SigAction, SigInfo, SyncSignal, TF, Thread, at_boundary, bit,
    entry_at, guest_set, guest_signal, host_mask, host_signal, reconstruct, thread,
};
use crate::cpu::{Cpu, OFF_HOST_FS, OFF_PENDING_SIGNALS, RAX, RSP};
use crate::jit::translate::EXIT_INTERRUPTED;
use crate::jit::{Block, Jit, trampoline};
use crate::sys::{self, KernelSigaction, MAP_ANONYMOUS, MAP_PRIVATE, PROT_READ, PROT_WRITE};

/// The code cache, for the handler to find the block of a pc.
static JIT: AtomicPtr<Jit> = AtomicPtr::new(ptr::null_mut());
/// Result of the TF probe: the kernel honours TF set in a `ucontext`.
static TF_WORKS: AtomicBool = AtomicBool::new(false);
static PROBE: AtomicU32 = AtomicU32::new(PROBE_IDLE);
const PROBE_IDLE: u32 = 0;
const PROBE_ARMED: u32 = 1;
const PROBE_STEPPING: u32 = 2;
/// Size of calco's alternate stack of each thread.
const HOST_STACK_SIZE: u64 = 64 << 10;

unsafe extern "C" {
    /// First instruction of the safe system call that checks for pending signals.
    static calco_safe_start: u8;
    /// Its `syscall` instruction.
    static calco_safe_syscall: u8;
    /// First instruction of `enter` after `host_rsp` and `host_fs` are saved.
    static calco_enter_committed: u8;
    /// The jump of `enter` into the code cache.
    static calco_enter_jump: u8;
}

/// Sets up signals for the main thread: inherited mask and ignored signals become the
/// guest's, the synchronous signals and calco's 62 get the handler, and TF is probed.
pub fn init(cpu: &mut Cpu, process: &mut Process, jit: &mut Jit) {
    JIT.store(jit, Ordering::Release);
    // What execve keeps from the program before: the mask and the ignored signals.
    let inherited = sys::rt_sigprocmask(super::SIG_BLOCK as usize, None).unwrap_or(0);
    for sig in 1..=GUEST_SIGNALS as u32 {
        let Some(host) = host_signal(sig as u64) else {
            continue;
        };
        if sig == SIGPIPE {
            // The Rust runtime ignores SIGPIPE before `main`: the guest starts with the
            // default, as almost every parent leaves it.
            let _ = sys::rt_sigaction(host as u32, Some(&KernelSigaction::default()));
            continue;
        }
        if let Ok(old) = sys::rt_sigaction(host as u32, None)
            && old.handler == SIG_IGN as usize
        {
            process.actions[sig as usize - 1].handler = SIG_IGN;
        }
    }
    init_thread(cpu);
    let th = thread(cpu);
    th.set_mask_quiet(cpu, guest_set(inherited));
    for sig in [SIGILL, SIGTRAP, SIGBUS, SIGFPE, SIGSEGV, HOST_KICK] {
        install_handler(sig, 0);
    }
    set_host_mask(host_mask(th.mask()));
    probe_tf();
}

/// Gives the thread of `cpu` its signal state and calco's alternate stack.
pub fn init_thread(cpu: &mut Cpu) {
    thread(cpu);
    let flags = MAP_PRIVATE | MAP_ANONYMOUS;
    let stack = unsafe { sys::mmap(0, HOST_STACK_SIZE, PROT_READ | PROT_WRITE, flags, -1, 0) }
        .expect("cannot map the signal stack");
    sys::sigaltstack(stack, HOST_STACK_SIZE).expect("cannot set the signal stack");
}

/// Sets the host action that corresponds to the guest action `act` of guest signal `sig`:
/// calco's handler if the guest catches it, otherwise the same ignore or default. The
/// synchronous signals always keep calco's handler.
pub fn install(sig: u32, act: &SigAction) {
    let Some(host) = host_signal(sig as u64) else {
        return;
    };
    let host = host as u32;
    if host == 0 || SYNC_SIGNALS & bit(host) != 0 {
        return;
    }
    let keep = act.flags & (SA_NOCLDSTOP | SA_NOCLDWAIT);
    match act.handler {
        SIG_DFL | SIG_IGN => {
            let action = KernelSigaction {
                handler: act.handler as usize,
                flags: keep,
                restorer: 0,
                mask: 0,
            };
            let _ = sys::rt_sigaction(host, Some(&action));
        }
        _ => install_handler(host, keep),
    }
}

/// calco's handler for host signal `sig`, on the alternate stack, with every signal
/// blocked and without `SA_RESTART` (a safe call must come back with `EINTR`).
fn install_handler(sig: u32, extra_flags: u64) {
    let action = KernelSigaction {
        handler: handler_entry as *const () as usize,
        flags: SA_SIGINFO | SA_ONSTACK | SA_RESTORER | extra_flags,
        restorer: restorer as *const () as usize,
        mask: !0,
    };
    let _ = sys::rt_sigaction(sig, Some(&action));
}

pub fn set_host_mask(mask: u64) {
    let _ = sys::rt_sigprocmask(super::SIG_SETMASK as usize, Some(mask));
}

/// Host signals pending for the thread or the process.
pub fn host_pending() -> u64 {
    sys::rt_sigpending().unwrap_or(0)
}

/// Dies by guest signal `sig` the way its default action does.
pub fn die(sig: u32) -> ! {
    let host = host_signal(sig as u64).unwrap_or(sig as u64);
    sys::die_by_signal(host as i32)
}

/// Default action "stop" of guest signal `sig`: the host stops the process by it (its host
/// action is the default) and the thread goes on when continued.
pub fn stop(sig: u32) {
    if let Some(host) = host_signal(sig as u64) {
        sys::tgkill_self(host as u32);
    }
}

/// Guest system call `nr` through the safe stub: returns `-ERESTART_INTERNAL` without
/// calling the kernel if a signal is pending, before or during the check.
pub fn safe_syscall(nr: u64, a: [u64; 6]) -> i64 {
    // Identity mapping: guest pointers are valid as they are; the kernel checks them.
    unsafe { safe_syscall6(nr, a[0], a[1], a[2], a[3], a[4], a[5]) }
}

/// `syscall` with the check of `gs:[pending_signals]` right before it (contract 3.11).
/// The handler knows the range from `calco_safe_start` to `calco_safe_syscall`: a signal
/// there skips the `syscall` and returns `-ERESTART_INTERNAL`. Without a `Cpu` (GS base 0)
/// it is a plain `syscall`.
#[unsafe(naked)]
#[allow(named_asm_labels)]
unsafe extern "sysv64" fn safe_syscall6(
    nr: u64,
    a1: u64,
    a2: u64,
    a3: u64,
    a4: u64,
    a5: u64,
    a6: u64,
) -> i64 {
    naked_asm!(
        "mov rax, rdi",
        "mov rdi, rsi",
        "mov rsi, rdx",
        "mov rdx, rcx",
        "mov r10, r8",
        "mov r8, r9",
        "mov r9, qword ptr [rsp + 8]",
        // R11 is the scratch register the `syscall` clobbers anyway.
        "rdgsbase r11",
        "test r11, r11",
        "jz 2f",
        ".globl calco_safe_start",
        ".hidden calco_safe_start",
        "calco_safe_start:",
        "cmp qword ptr [r11 + {pending}], 0",
        "jne 3f",
        ".globl calco_safe_syscall",
        ".hidden calco_safe_syscall",
        "calco_safe_syscall:",
        "2:",
        "syscall",
        "ret",
        "3:",
        "mov rax, {restart}",
        "ret",
        pending = const OFF_PENDING_SIGNALS,
        restart = const -ERESTART_INTERNAL,
    )
}

/// Entry of the host handler (`sa_handler`, called as `handler(sig, info, uc)` on the
/// alternate stack). Keeps the interrupted FS base, installs calco's (from `gs:[host_fs]`
/// when the thread has a `Cpu`), calls `on_signal` with the old one and installs the FS
/// base it returns.
#[unsafe(naked)]
unsafe extern "sysv64" fn handler_entry() {
    naked_asm!(
        "push rbx",
        "rdfsbase rbx",
        "rdgsbase rax",
        "test rax, rax",
        "jz 2f",
        "mov rax, qword ptr [rax + {host_fs}]",
        "test rax, rax",
        "jz 2f",
        "wrfsbase rax",
        "2:",
        "mov rcx, rbx",
        "call {handler}",
        "wrfsbase rax",
        "pop rbx",
        "ret",
        host_fs = const OFF_HOST_FS,
        handler = sym on_signal,
    )
}

/// `sa_restorer` of calco's handler.
#[unsafe(naked)]
unsafe extern "sysv64" fn restorer() {
    naked_asm!("mov eax, {nr}", "syscall", "ud2", nr = const sys::SYS_RT_SIGRETURN)
}

unsafe fn get(uc: *const u8, off: usize) -> u64 {
    unsafe { (uc.add(off) as *const u64).read_unaligned() }
}

unsafe fn set(uc: *mut u8, off: usize, v: u64) {
    unsafe { (uc.add(off) as *mut u64).write_unaligned(v) }
}

fn sync_fault(sig: u32, info: *const u8) -> bool {
    let code = unsafe { (info.add(frame::SI_CODE) as *const i32).read_unaligned() };
    SYNC_SIGNALS & bit(sig) != 0 && code > 0
}

/// The host handler. Returns the FS base to install before going back to the kernel.
unsafe extern "sysv64" fn on_signal(sig: i32, info: *const u8, uc: *mut u8, fs: u64) -> u64 {
    let sig = sig as u32;
    unsafe {
        if probe(sig, uc) {
            return fs;
        }
        let cpu = sys::gs_base() as *mut Cpu;
        if cpu.is_null() || (*cpu).signal_state.is_null() {
            if sync_fault(sig, info) {
                sys::die_by_signal(sig as i32);
            }
            return fs;
        }
        let th = &*((*cpu).signal_state as *const Thread);
        let pc = get(uc, UC_RIP);
        if sig == SIGTRAP && sync_fault(sig, info) && th.stepping.load(Ordering::Relaxed) {
            return step(cpu, th, uc, pc, fs);
        }
        if sig == HOST_KICK {
            return fs;
        }
        if sync_fault(sig, info) {
            return fault(cpu, th, sig, info, uc, pc);
        }
        async_signal(cpu, th, sig, info, uc, pc, fs)
    }
}

/// The block whose host code holds `pc`, its id and the offset of `pc` in it.
unsafe fn locate(pc: u64) -> Option<(u32, &'static Block, u32)> {
    let jit = JIT.load(Ordering::Acquire);
    if jit.is_null() {
        return None;
    }
    // The thread is in translated code, so the dispatcher is not changing the cache.
    let jit = unsafe { &*jit };
    let id = jit.block_at(pc)?;
    let block = jit.block(id);
    Some((id, block, (pc - block.host) as u32))
}

unsafe fn gprs(uc: *const u8) -> [u64; 16] {
    core::array::from_fn(|reg| unsafe { get(uc, uc_gpr(reg)) })
}

/// The guest state at `pc` inside `block` (at offset `off`): registers with the
/// temporaries put back, and the rip of the boundary.
unsafe fn guest_state(cpu: *mut Cpu, uc: *const u8, block: &Block, off: u32) -> ([u64; 16], u64) {
    let entry = entry_at(&block.pcs, off).expect("a pc table starts at offset 0");
    let mut gpr = unsafe { gprs(uc) };
    let (scratch, next_rip) = unsafe { ((*cpu).scratch, (*cpu).next_rip) };
    let rip = reconstruct(entry, &mut gpr, &scratch, next_rip);
    (gpr, rip)
}

unsafe fn fault(
    cpu: *mut Cpu,
    th: &Thread,
    sig: u32,
    info: *const u8,
    uc: *mut u8,
    pc: u64,
) -> u64 {
    let Some((_, block, off)) = (unsafe { locate(pc) }) else {
        sys::die_by_signal(sig as i32);
    };
    let (gpr, rip) = unsafe { guest_state(cpu, uc, block, off) };
    let raw = unsafe { &*(info as *const [u8; SIGINFO_SIZE]) };
    let mut ginfo = SigInfo::from_host(raw, sig);
    // The kernel gives the address of the instruction for these; the host one is in the
    // code cache.
    if matches!(sig, SIGILL | SIGFPE | SIGTRAP) {
        frame::put64(&mut ginfo.0, frame::SI_ADDR, rip);
    }
    th.set_fault(SyncSignal {
        sig: guest_signal(sig),
        info: ginfo,
        trapno: unsafe { get(uc, UC_TRAPNO) },
        err: unsafe { get(uc, UC_ERR) },
        cr2: unsafe { get(uc, UC_CR2) },
    });
    unsafe { redirect(cpu, th, uc, &gpr, rip) }
}

#[allow(clippy::too_many_arguments)]
unsafe fn async_signal(
    cpu: *mut Cpu,
    th: &Thread,
    sig: u32,
    info: *const u8,
    uc: *mut u8,
    pc: u64,
    fs: u64,
) -> u64 {
    let raw = unsafe { &*(info as *const [u8; SIGINFO_SIZE]) };
    unsafe { set(uc, UC_SIGMASK, get(uc, UC_SIGMASK) | bit(sig)) };
    if !th.queue(cpu, guest_signal(sig), SigInfo::from_host(raw, sig)) {
        return fs;
    }
    let safe = (
        &raw const calco_safe_start as u64,
        &raw const calco_safe_syscall as u64,
    );
    if (safe.0..=safe.1).contains(&pc) {
        // The call has not run: skip the `syscall` (2 bytes) and say why.
        unsafe {
            set(uc, UC_RIP, safe.1 + 2);
            set(uc, uc_gpr(RAX), (-ERESTART_INTERNAL) as u64);
        }
        return fs;
    }
    let enter = (
        &raw const calco_enter_committed as u64,
        &raw const calco_enter_jump as u64,
    );
    if (enter.0..=enter.1).contains(&pc) {
        // `enter` has not jumped yet: `Cpu` still holds the whole guest state.
        unsafe {
            (*cpu).next_rip = (*cpu).rip;
            return resume(cpu, th, uc);
        }
    }
    let Some((id, block, off)) = (unsafe { locate(pc) }) else {
        // Rust or the way out of the cache: the dispatcher checks the pending word.
        return fs;
    };
    if at_boundary(&block.pcs, off) {
        let (gpr, rip) = unsafe { guest_state(cpu, uc, block, off) };
        return unsafe { redirect(cpu, th, uc, &gpr, rip) };
    }
    if TF_WORKS.load(Ordering::Relaxed) {
        unsafe { set(uc, UC_EFLAGS, get(uc, UC_EFLAGS) | TF) };
        th.stepping.store(true, Ordering::Relaxed);
    } else {
        // Without TF the block runs to its end and leaves to the dispatcher.
        unsafe { (*JIT.load(Ordering::Acquire)).unlink_outgoing(id) };
    }
    fs
}

/// SIGTRAP of TF while stepping: at a guest boundary the thread resumes in the
/// dispatcher; out of the blocks (on the way to the dispatcher) stepping ends.
unsafe fn step(cpu: *mut Cpu, th: &Thread, uc: *mut u8, pc: u64, fs: u64) -> u64 {
    match unsafe { locate(pc) } {
        Some((_, block, off)) if at_boundary(&block.pcs, off) => unsafe {
            let (gpr, rip) = guest_state(cpu, uc, block, off);
            redirect(cpu, th, uc, &gpr, rip)
        },
        Some(_) => fs,
        None => {
            unsafe { set(uc, UC_EFLAGS, get(uc, UC_EFLAGS) & !TF) };
            th.stepping.store(false, Ordering::Relaxed);
            fs
        }
    }
}

/// Writes the guest state into `Cpu` (registers, rip, RFLAGS, x87/SSE from the host
/// frame) and resumes in the dispatcher.
unsafe fn redirect(cpu: *mut Cpu, th: &Thread, uc: *mut u8, gpr: &[u64; 16], rip: u64) -> u64 {
    unsafe {
        (*cpu).gpr = *gpr;
        (*cpu).rip = rip;
        (*cpu).next_rip = rip;
        (*cpu).rflags = get(uc, UC_EFLAGS) & !TF;
        let fp = get(uc, UC_FPSTATE) as *const u8;
        if !fp.is_null() {
            (*cpu).fxsave.0 = guest_fx(fp);
        }
        resume(cpu, th, uc)
    }
}

/// The FXSAVE part of the host frame at `fp`. An XSAVE frame may leave x87 or SSE in
/// their init state (bits 0 and 1 of `xfeatures` clear); then their legacy bytes do not
/// count and the init values are used.
unsafe fn guest_fx(fp: *const u8) -> [u8; FP_SIZE] {
    let mut fx = [0u8; FP_SIZE];
    unsafe { ptr::copy_nonoverlapping(fp, fx.as_mut_ptr(), FP_SIZE) };
    if frame::get32(&fx, FP_SW_RESERVED) == FP_XSTATE_MAGIC1 {
        let xfeatures = unsafe { get(fp, FP_XFEATURES) };
        if xfeatures & 1 == 0 {
            let mxcsr = frame::get32(&fx, FP_MXCSR);
            fx[..FP_XMM].fill(0);
            frame::put16(&mut fx, FP_CWD, FCW_INIT);
            frame::put32(&mut fx, FP_MXCSR, mxcsr);
        }
        if xfeatures & 2 == 0 {
            fx[FP_XMM..FP_XMM + 256].fill(0);
        }
    }
    fx
}

/// Makes the kernel return to the dispatcher as if `enter` returned with
/// `EXIT_INTERRUPTED`: host stack, DF and TF clear, x87 empty with FCW `0x037F`, MXCSR
/// `0x1F80` and calco's FS base.
unsafe fn resume(cpu: *mut Cpu, th: &Thread, uc: *mut u8) -> u64 {
    th.stepping.store(false, Ordering::Relaxed);
    unsafe {
        (*cpu).exit_reason = EXIT_INTERRUPTED as u64;
        set(uc, UC_RIP, trampoline::resume as *const () as u64);
        set(uc, uc_gpr(RSP), (*cpu).host_rsp);
        set(uc, UC_EFLAGS, get(uc, UC_EFLAGS) & !(TF | DF | AC));
        let fp = get(uc, UC_FPSTATE) as *mut u8;
        if !fp.is_null() {
            let area = core::slice::from_raw_parts_mut(fp, FP_SIZE);
            frame::put16(area, FP_CWD, FCW_INIT);
            frame::put16(area, FP_SWD, 0);
            area[FP_TWD] = 0;
            frame::put32(area, FP_MXCSR, MXCSR_INIT);
        }
        (*cpu).host_fs
    }
}

/// The TF probe (hypothesis of contract 3.11): host signal 62 sets TF in its `ucontext`;
/// if the kernel honours it, the next instruction raises SIGTRAP.
unsafe fn probe(sig: u32, uc: *mut u8) -> bool {
    match PROBE.load(Ordering::Acquire) {
        PROBE_ARMED if sig == HOST_KICK => {
            unsafe { set(uc, UC_EFLAGS, get(uc, UC_EFLAGS) | TF) };
            PROBE.store(PROBE_STEPPING, Ordering::Release);
            true
        }
        PROBE_STEPPING if sig == SIGTRAP => {
            unsafe { set(uc, UC_EFLAGS, get(uc, UC_EFLAGS) & !TF) };
            TF_WORKS.store(true, Ordering::Release);
            PROBE.store(PROBE_IDLE, Ordering::Release);
            true
        }
        _ => false,
    }
}

/// Measures at start whether TF set from a handler works; if not, asynchronous signals
/// in the middle of an expansion unlink the block instead.
fn probe_tf() {
    PROBE.store(PROBE_ARMED, Ordering::SeqCst);
    // 62 arrives on the way back from `tgkill`; one instruction later, SIGTRAP.
    sys::tgkill_self(HOST_KICK);
    core::hint::black_box(0u64);
    PROBE.store(PROBE_IDLE, Ordering::SeqCst);
}

/// Whether the TF probe succeeded.
pub fn tf_works() -> bool {
    TF_WORKS.load(Ordering::Acquire)
}
