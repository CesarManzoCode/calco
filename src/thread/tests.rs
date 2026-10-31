use super::*;
use crate::cpu::{R8, RBX, RDI, RDX, RSI};
use crate::signal::SIGCHLD;

/// What glibc 2.44 passes to `clone` in `pthread_create` (`create_thread`).
#[rustfmt::skip]
const PTHREAD: u64 = CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SYSVSEM | CLONE_SIGHAND
    | CLONE_THREAD | CLONE_SETTLS | CLONE_PARENT_SETTID | CLONE_CHILD_CLEARTID;
const EXIT_SIGNAL: u64 = SIGCHLD as u64;

#[test]
fn clone_flags_are_read_as_the_kernel_and_the_contract_say() {
    use CloneKind::{Fork, Thread};
    for (flags, kind) in [
        (PTHREAD, Ok(Thread)),
        (PTHREAD | CLONE_CHILD_SETTID | CLONE_DETACHED, Ok(Thread)),
        // Without CLONE_SYSVSEM or the TLS and tid options, still a thread.
        (THREAD_SHARES, Ok(Thread)),
        // The exit signal does not count for a thread.
        (PTHREAD | EXIT_SIGNAL, Ok(Thread)),
        // fork of glibc, plain fork, and vfork of posix_spawn (served as fork).
        (
            CLONE_CHILD_SETTID | CLONE_CHILD_CLEARTID | EXIT_SIGNAL,
            Ok(Fork),
        ),
        (EXIT_SIGNAL, Ok(Fork)),
        (CLONE_VM | CLONE_VFORK | EXIT_SIGNAL, Ok(Fork)),
        // CLONE_VM without CLONE_THREAD, other than vfork (contract 3.12).
        (CLONE_VM | EXIT_SIGNAL, Err(EINVAL)),
        (CLONE_VM | CLONE_FS | CLONE_FILES, Err(EINVAL)),
        // The kernel's own rules: THREAD needs SIGHAND, SIGHAND needs VM.
        (CLONE_VM | CLONE_THREAD, Err(EINVAL)),
        (CLONE_SIGHAND | EXIT_SIGNAL, Err(EINVAL)),
        (CLONE_THREAD | CLONE_SIGHAND, Err(EINVAL)),
        // A thread that does not share what host threads share, or asks for more.
        (PTHREAD & !CLONE_FILES, Err(EINVAL)),
        (PTHREAD & !CLONE_FS, Err(EINVAL)),
        (PTHREAD | CLONE_VFORK, Err(EINVAL)),
        (PTHREAD | CLONE_NEWNS, Err(EINVAL)),
    ] {
        assert_eq!(clone_kind(flags), kind, "flags {flags:#x}");
    }
}

/// A parent at the `syscall` of `clone`: every register different, RAX the number.
fn parent() -> Box<Cpu> {
    let mut cpu = Cpu::new(0x40_1234, 0x3fff_ffff_e000);
    for (i, r) in cpu.gpr.iter_mut().enumerate() {
        *r = 0x1111_0000 + i as u64;
    }
    cpu.gpr[RAX] = 56;
    cpu.rflags = 0x247;
    cpu.fs_base = 0x7000_0000;
    cpu.gs_base = 0x7100_0000;
    cpu.fxsave.0[160..176].fill(0xa5);
    cpu.next_rip = 0x40_1234;
    cpu.exit_reason = 3;
    cpu.pending_signals = 1 << 9;
    cpu.tid = 4242;
    cpu.clear_child_tid = 0x5000;
    cpu.scratch = [1, 2, 3, 4];
    cpu
}

#[test]
fn child_cpu_starts_from_the_parent_with_what_clone_asks() {
    let p = parent();
    let stack = 0x2000_0000_f000;
    let (ctid, tls) = (0x2000_0001_0000, 0x2000_0002_0000);
    let c = child_cpu(&p, PTHREAD, stack, ctid, tls);

    assert_eq!(c.rip, p.rip, "the child goes on after the syscall");
    assert_eq!(c.gpr[RAX], 0, "clone returns 0 in the child");
    assert_eq!(c.gpr[RSP], stack);
    // What the `syscall` instruction leaves in the child as in the parent.
    assert_eq!(c.gpr[RCX], p.rip);
    assert_eq!(c.gpr[R11], p.rflags);
    for reg in [RBX, RDX, RSI, RDI, R8, 5, 9, 10, 12, 13, 14, 15] {
        assert_eq!(c.gpr[reg], p.gpr[reg], "register {reg}");
    }
    assert_eq!(c.rflags, p.rflags);
    assert_eq!(c.fs_base, tls, "CLONE_SETTLS");
    assert_eq!(c.gs_base, p.gs_base);
    assert_eq!(c.fxsave.0, p.fxsave.0, "x87/SSE state");
    assert_eq!(c.clear_child_tid, ctid, "CLONE_CHILD_CLEARTID");

    // Nothing of the parent's thread: no tid, no signals, no exit in flight.
    assert_eq!(c.tid, 0);
    assert_eq!(c.pending_signals, 0);
    assert!(c.signal_state.is_null());
    assert_eq!(c.exit_reason, 0);
    assert_eq!(c.scratch, [0; 4]);
}

#[test]
fn child_cpu_keeps_the_parent_stack_and_tls_unless_asked() {
    let p = parent();
    let flags = PTHREAD & !(CLONE_SETTLS | CLONE_CHILD_CLEARTID);
    let c = child_cpu(&p, flags, 0, 0x2000_0001_0000, 0x2000_0002_0000);
    assert_eq!(c.gpr[RSP], p.gpr[RSP], "stack 0 keeps the parent's");
    assert_eq!(c.fs_base, p.fs_base, "without CLONE_SETTLS");
    let ctid = c.clear_child_tid;
    assert_eq!(ctid, 0, "without CLONE_CHILD_CLEARTID nothing is inherited");
}

#[test]
fn a_flush_without_threads_runs_at_once() {
    // Before the guest starts (and in unit tests) there is nobody to wait for.
    let mut ran = false;
    safe_point(|| ran = true);
    assert!(ran);
    assert!(!flushing());
}
