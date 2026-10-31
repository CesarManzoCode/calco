//! The x86-64 `rt_sigframe` as the kernel builds it (contract 3.11). Offsets come from
//! /usr/include/asm/ucontext.h (`struct ucontext`), /usr/include/asm/sigcontext.h
//! (`struct sigcontext`, `struct _fpstate_64`) and /usr/include/asm/siginfo.h (`siginfo_t`);
//! the kernel frame is `pretcode`, then the `ucontext`, then the `siginfo_t`, with the
//! FXSAVE area above it, 64-byte aligned. `src/signal/tests.rs` checks every offset against
//! the headers with the C compiler.

// struct ucontext
pub const UC_FLAGS: usize = 0;
pub const UC_LINK: usize = 8;
pub const UC_STACK: usize = 16;
pub const UC_MCONTEXT: usize = 40;
pub const UC_SIGMASK: usize = 296;
pub const UC_SIZE: usize = 304;

// stack_t
pub const SS_SP: usize = 0;
pub const SS_FLAGS: usize = 8;
pub const SS_SIZE: usize = 16;
pub const STACK_T_SIZE: usize = 24;

// struct sigcontext (64-bit): r8..r15, rdi, rsi, rbp, rbx, rdx, rax, rcx, rsp, 8 bytes each.
pub const SC_RIP: usize = 128;
pub const SC_EFLAGS: usize = 136;
pub const SC_CS: usize = 144;
pub const SC_GS: usize = 146;
pub const SC_FS: usize = 148;
pub const SC_SS: usize = 150;
pub const SC_ERR: usize = 152;
pub const SC_TRAPNO: usize = 160;
pub const SC_OLDMASK: usize = 168;
pub const SC_CR2: usize = 176;
pub const SC_FPSTATE: usize = 184;
pub const SC_SIZE: usize = 256;
/// Slot of each general register, in encoding order (RAX, RCX, …, R15), in the sigcontext.
pub const SC_SLOT: [usize; 16] = [13, 14, 12, 11, 15, 10, 9, 8, 0, 1, 2, 3, 4, 5, 6, 7];

/// Offset of general register `reg` (encoding order) from the start of a `ucontext`.
pub const fn uc_gpr(reg: usize) -> usize {
    UC_MCONTEXT + 8 * SC_SLOT[reg]
}
pub const UC_RIP: usize = UC_MCONTEXT + SC_RIP;
pub const UC_EFLAGS: usize = UC_MCONTEXT + SC_EFLAGS;
pub const UC_ERR: usize = UC_MCONTEXT + SC_ERR;
pub const UC_TRAPNO: usize = UC_MCONTEXT + SC_TRAPNO;
pub const UC_CR2: usize = UC_MCONTEXT + SC_CR2;
pub const UC_FPSTATE: usize = UC_MCONTEXT + SC_FPSTATE;

// siginfo_t
pub const SI_SIGNO: usize = 0;
pub const SI_CODE: usize = 8;
pub const SI_ADDR: usize = 16;
/// `si_status` of SIGCHLD.
pub const SI_STATUS: usize = 24;
pub const SIGINFO_SIZE: usize = 128;

// struct rt_sigframe (arch/x86/include/asm/sigframe.h, built from the uapi structs)
pub const FRAME_PRETCODE: usize = 0;
pub const FRAME_UC: usize = 8;
pub const FRAME_INFO: usize = FRAME_UC + UC_SIZE;
pub const FRAME_SIZE: usize = FRAME_INFO + SIGINFO_SIZE;

// struct _fpstate_64 (FXSAVE format)
pub const FP_CWD: usize = 0;
pub const FP_SWD: usize = 2;
pub const FP_TWD: usize = 4;
pub const FP_MXCSR: usize = 24;
pub const FP_ST: usize = 32;
pub const FP_XMM: usize = 160;
pub const FP_SW_RESERVED: usize = 464;
pub const FP_SIZE: usize = 512;
/// `xfeatures` of `struct _header`, right after the FXSAVE part of an XSAVE frame.
pub const FP_XFEATURES: usize = 512;
pub const FP_XSTATE_MAGIC1: u32 = 0x4650_5853;

// /usr/include/asm/ucontext.h
pub const UC_SIGCONTEXT_SS: u64 = 0x2;
pub const UC_STRICT_RESTORE_SS: u64 = 0x4;
/// Selectors of 64-bit user code and data (`__USER_CS`, `__USER_DS`).
pub const USER_CS: u16 = 0x33;
pub const USER_DS: u16 = 0x2b;

// /usr/include/asm-generic/signal-defs.h and /usr/include/linux/signal.h
pub const SS_ONSTACK: u32 = 1;
pub const SS_DISABLE: u32 = 2;
pub const SS_AUTODISARM: u32 = 1 << 31;
/// `MINSIGSTKSZ` of /usr/include/asm/signal.h.
pub const MINSIGSTKSZ: u64 = 2048;

/// The x86-64 ABI red zone below RSP, which a frame never overwrites.
const RED_ZONE: u64 = 128;
/// x87, SSE and MXCSR as `fpu__clear_user_states` leaves them for a handler.
pub const FCW_INIT: u16 = 0x037f;
pub const MXCSR_INIT: u32 = 0x1f80;

pub fn get16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(b[off..off + 2].try_into().unwrap())
}
pub fn get32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}
pub fn get64(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().unwrap())
}
pub fn put16(b: &mut [u8], off: usize, v: u16) {
    b[off..off + 2].copy_from_slice(&v.to_le_bytes());
}
pub fn put32(b: &mut [u8], off: usize, v: u32) {
    b[off..off + 4].copy_from_slice(&v.to_le_bytes());
}
pub fn put64(b: &mut [u8], off: usize, v: u64) {
    b[off..off + 8].copy_from_slice(&v.to_le_bytes());
}

/// The guest `sigaltstack` of a thread, with the fields the kernel keeps in the task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AltStack {
    pub sp: u64,
    pub size: u64,
    /// `sas_ss_flags`: the flags of the last `sigaltstack` that set it.
    pub flags: u32,
}

impl AltStack {
    pub const DISABLED: AltStack = AltStack {
        sp: 0,
        size: 0,
        flags: SS_DISABLE,
    };

    /// `__on_sig_stack`: `sp` is inside the stack (which grows down from `sp + size`).
    pub fn contains(&self, sp: u64) -> bool {
        sp > self.sp && sp - self.sp <= self.size
    }

    /// `on_sig_stack`: with `SS_AUTODISARM` a handler is never nested on it.
    pub fn on(&self, sp: u64) -> bool {
        self.flags & SS_AUTODISARM == 0 && self.contains(sp)
    }

    /// `sas_ss_flags`.
    pub fn mode(&self, sp: u64) -> u32 {
        if self.size == 0 {
            SS_DISABLE
        } else if self.on(sp) {
            SS_ONSTACK
        } else {
            0
        }
    }
}

/// Where the frame and the FXSAVE area of a handler go, as `get_sigframe` does: below the
/// red zone, or at the top of the alternate stack with `SA_ONSTACK` when not on it yet.
/// `None` when the frame would overflow the alternate stack (the kernel then forces
/// SIGSEGV).
pub fn place(rsp: u64, onstack: bool, alt: &AltStack) -> Option<(u64, u64)> {
    let nested = alt.on(rsp);
    let mut sp = rsp.wrapping_sub(RED_ZONE);
    let mut entering = false;
    if onstack && alt.mode(sp) == 0 {
        sp = alt.sp.wrapping_add(alt.size);
        entering = true;
    }
    let fpstate = sp.wrapping_sub(FP_SIZE as u64) & !63;
    let frame = (fpstate.wrapping_sub(FRAME_SIZE as u64) & !15).wrapping_sub(8);
    if (nested || entering) && !alt.contains(frame) {
        return None;
    }
    Some((frame, fpstate))
}

/// Guest state saved in a frame.
pub struct Saved<'a> {
    pub gpr: &'a [u64; 16],
    pub rip: u64,
    pub rflags: u64,
    pub err: u64,
    pub trapno: u64,
    pub cr2: u64,
}

/// The bytes of an `rt_sigframe` (`FRAME_SIZE`) whose FXSAVE area is at `fpstate`, for a
/// thread with alternate stack `alt` (before `SS_AUTODISARM` resets it) and guest mask
/// `mask` to restore on return.
pub fn build(
    saved: &Saved,
    mask: u64,
    alt: &AltStack,
    restorer: u64,
    fpstate: u64,
    info: &[u8; SIGINFO_SIZE],
) -> [u8; FRAME_SIZE] {
    let mut f = [0u8; FRAME_SIZE];
    put64(&mut f, FRAME_PRETCODE, restorer);
    let uc = FRAME_UC;
    put64(
        &mut f,
        uc + UC_FLAGS,
        UC_SIGCONTEXT_SS | UC_STRICT_RESTORE_SS,
    );
    put64(&mut f, uc + UC_LINK, 0);
    put64(&mut f, uc + UC_STACK + SS_SP, alt.sp);
    put32(&mut f, uc + UC_STACK + SS_FLAGS, alt.flags);
    put64(&mut f, uc + UC_STACK + SS_SIZE, alt.size);
    for (reg, &v) in saved.gpr.iter().enumerate() {
        put64(&mut f, uc + uc_gpr(reg), v);
    }
    let sc = uc + UC_MCONTEXT;
    put64(&mut f, sc + SC_RIP, saved.rip);
    put64(&mut f, sc + SC_EFLAGS, saved.rflags);
    put16(&mut f, sc + SC_CS, USER_CS);
    put16(&mut f, sc + SC_GS, 0);
    put16(&mut f, sc + SC_FS, 0);
    put16(&mut f, sc + SC_SS, USER_DS);
    put64(&mut f, sc + SC_ERR, saved.err);
    put64(&mut f, sc + SC_TRAPNO, saved.trapno);
    put64(&mut f, sc + SC_OLDMASK, mask);
    put64(&mut f, sc + SC_CR2, saved.cr2);
    put64(&mut f, sc + SC_FPSTATE, fpstate);
    put64(&mut f, uc + UC_SIGMASK, mask);
    f[FRAME_INFO..].copy_from_slice(info);
    f
}

/// The FXSAVE area of a guest frame: the guest state with `sw_reserved` zero, so
/// `magic1 = 0` says there is no XSAVE state (CPUID announces none).
pub fn fpstate(fx: &[u8; FP_SIZE]) -> [u8; FP_SIZE] {
    let mut out = *fx;
    out[FP_SW_RESERVED..].fill(0);
    out
}

/// The FXSAVE area of the init state a handler starts with.
pub fn fx_init() -> [u8; FP_SIZE] {
    let mut fx = [0u8; FP_SIZE];
    put16(&mut fx, FP_CWD, FCW_INIT);
    put32(&mut fx, FP_MXCSR, MXCSR_INIT);
    fx
}

/// The guest state that `rt_sigreturn` reads from a frame `uc` (`UC_SIZE` bytes).
pub struct Restored {
    pub gpr: [u64; 16],
    pub rip: u64,
    pub eflags: u64,
    pub fpstate: u64,
    pub mask: u64,
    pub stack: (u64, u32, u64),
}

pub fn restore(uc: &[u8]) -> Restored {
    let mut gpr = [0; 16];
    for (reg, v) in gpr.iter_mut().enumerate() {
        *v = get64(uc, uc_gpr(reg));
    }
    Restored {
        gpr,
        rip: get64(uc, UC_RIP),
        eflags: get64(uc, UC_EFLAGS),
        fpstate: get64(uc, UC_FPSTATE),
        mask: get64(uc, UC_SIGMASK),
        stack: (
            get64(uc, UC_STACK + SS_SP),
            get32(uc, UC_STACK + SS_FLAGS),
            get64(uc, UC_STACK + SS_SIZE),
        ),
    }
}
