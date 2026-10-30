//! Entry and exit trampolines between Rust and translated code (contract 3.3, 3.6, 3.8).
//!
//! While translated code runs, the host registers are the guest ones, RSP is the guest
//! stack, FS base is the guest `fs_base` and GS base points to the thread's `Cpu`. Rust
//! runs on the host stack with the host FS base, DF clear, FCW `0x037F`, MXCSR `0x1F80`
//! and an empty x87 stack.

use core::arch::naked_asm;

use crate::cpu::{
    Cpu, FCW_DEFAULT, MXCSR_DEFAULT, OFF_FS_BASE, OFF_FXSAVE, OFF_HOST_FS, OFF_HOST_RSP,
    OFF_RFLAGS, off_gpr, off_scratch,
};

// `fninit` is what leaves FCW at the value Rust expects.
const _: () = assert!(FCW_DEFAULT == 0x037f);

/// Runs translated code from the host address `code` with the guest state of `cpu` and
/// returns when a block exits, with the guest registers, RFLAGS, x87/SSE state,
/// `next_rip` and `exit_reason` written back to `cpu`.
///
/// Saves the callee-saved registers on the host stack and that stack in `host_rsp`, keeps
/// the host FS base in `host_fs` and installs the guest one, restores x87/SSE with
/// `fxrstor64` and RFLAGS with `popfq` (on the host stack), and loads the 16 registers
/// with `mov`, which leaves the flags alone.
///
/// # Safety
/// `cpu` must be the `Cpu` the GS base of this thread points to, and `code` the start of a
/// block in the code cache.
#[unsafe(naked)]
pub unsafe extern "sysv64" fn enter(cpu: *mut Cpu, code: u64) {
    naked_asm!(
        "push rbx",
        "push rbp",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "mov qword ptr [rdi + {host_rsp}], rsp",
        "rdfsbase rax",
        "mov qword ptr [rdi + {host_fs}], rax",
        "mov rax, qword ptr [rdi + {fs_base}]",
        "wrfsbase rax",
        "fxrstor64 [rdi + {fxsave}]",
        // No temporary is live at a block entry, so scratch[0] can hold the target.
        "mov qword ptr [rdi + {target}], rsi",
        "push qword ptr [rdi + {rflags}]",
        "popfq",
        "mov rax, qword ptr [rdi + {g0}]",
        "mov rcx, qword ptr [rdi + {g1}]",
        "mov rdx, qword ptr [rdi + {g2}]",
        "mov rbx, qword ptr [rdi + {g3}]",
        "mov rbp, qword ptr [rdi + {g5}]",
        "mov rsi, qword ptr [rdi + {g6}]",
        "mov r8, qword ptr [rdi + {g8}]",
        "mov r9, qword ptr [rdi + {g9}]",
        "mov r10, qword ptr [rdi + {g10}]",
        "mov r11, qword ptr [rdi + {g11}]",
        "mov r12, qword ptr [rdi + {g12}]",
        "mov r13, qword ptr [rdi + {g13}]",
        "mov r14, qword ptr [rdi + {g14}]",
        "mov r15, qword ptr [rdi + {g15}]",
        "mov rsp, qword ptr [rdi + {g4}]",
        "mov rdi, qword ptr [rdi + {g7}]",
        "jmp qword ptr gs:[{target}]",
        host_rsp = const OFF_HOST_RSP,
        host_fs = const OFF_HOST_FS,
        fs_base = const OFF_FS_BASE,
        fxsave = const OFF_FXSAVE,
        target = const off_scratch(0),
        rflags = const OFF_RFLAGS,
        g0 = const off_gpr(0),
        g1 = const off_gpr(1),
        g2 = const off_gpr(2),
        g3 = const off_gpr(3),
        g4 = const off_gpr(4),
        g5 = const off_gpr(5),
        g6 = const off_gpr(6),
        g7 = const off_gpr(7),
        g8 = const off_gpr(8),
        g9 = const off_gpr(9),
        g10 = const off_gpr(10),
        g11 = const off_gpr(11),
        g12 = const off_gpr(12),
        g13 = const off_gpr(13),
        g14 = const off_gpr(14),
        g15 = const off_gpr(15),
    )
}

/// Where every block exit ends (through the thunk at the start of the code cache): saves
/// the 16 registers with `mov`, switches to the host stack, saves RFLAGS with `pushfq`,
/// clears DF, saves x87/SSE with `fxsave64`, gives Rust FCW `0x037F` (`fninit`) and MXCSR
/// `0x1F80`, restores the host FS base and returns from `enter`.
///
/// # Safety
/// Only translated code jumps here, with GS pointing to the `Cpu` passed to `enter`.
#[unsafe(naked)]
pub unsafe extern "sysv64" fn exit_trampoline() {
    naked_asm!(
        "mov qword ptr gs:[{g0}], rax",
        "mov qword ptr gs:[{g1}], rcx",
        "mov qword ptr gs:[{g2}], rdx",
        "mov qword ptr gs:[{g3}], rbx",
        "mov qword ptr gs:[{g4}], rsp",
        "mov qword ptr gs:[{g5}], rbp",
        "mov qword ptr gs:[{g6}], rsi",
        "mov qword ptr gs:[{g7}], rdi",
        "mov qword ptr gs:[{g8}], r8",
        "mov qword ptr gs:[{g9}], r9",
        "mov qword ptr gs:[{g10}], r10",
        "mov qword ptr gs:[{g11}], r11",
        "mov qword ptr gs:[{g12}], r12",
        "mov qword ptr gs:[{g13}], r13",
        "mov qword ptr gs:[{g14}], r14",
        "mov qword ptr gs:[{g15}], r15",
        "mov rsp, qword ptr gs:[{host_rsp}]",
        "pushfq",
        "pop qword ptr gs:[{rflags}]",
        "cld",
        "rdgsbase rdi",
        "fxsave64 [rdi + {fxsave}]",
        "fninit",
        "push {mxcsr}",
        "ldmxcsr dword ptr [rsp]",
        "pop rax",
        "mov rax, qword ptr [rdi + {host_fs}]",
        "wrfsbase rax",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbp",
        "pop rbx",
        "ret",
        host_rsp = const OFF_HOST_RSP,
        host_fs = const OFF_HOST_FS,
        fxsave = const OFF_FXSAVE,
        rflags = const OFF_RFLAGS,
        mxcsr = const MXCSR_DEFAULT,
        g0 = const off_gpr(0),
        g1 = const off_gpr(1),
        g2 = const off_gpr(2),
        g3 = const off_gpr(3),
        g4 = const off_gpr(4),
        g5 = const off_gpr(5),
        g6 = const off_gpr(6),
        g7 = const off_gpr(7),
        g8 = const off_gpr(8),
        g9 = const off_gpr(9),
        g10 = const off_gpr(10),
        g11 = const off_gpr(11),
        g12 = const off_gpr(12),
        g13 = const off_gpr(13),
        g14 = const off_gpr(14),
        g15 = const off_gpr(15),
    )
}
