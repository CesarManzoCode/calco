//! Per-thread guest state. Translated code reaches it through the host GS base, so the
//! layout is `#[repr(C)]` and every offset used by emitted code is a constant here.

use core::ffi::c_void;
use core::mem::offset_of;

/// General register indexes in encoding order.
pub const RAX: usize = 0;
pub const RCX: usize = 1;
pub const RDX: usize = 2;
pub const RBX: usize = 3;
pub const RSP: usize = 4;
pub const RBP: usize = 5;
pub const RSI: usize = 6;
pub const RDI: usize = 7;
pub const R8: usize = 8;
pub const R9: usize = 9;
pub const R10: usize = 10;
pub const R11: usize = 11;

/// Default x87 control word and MXCSR, the values Rust code runs with.
pub const FCW_DEFAULT: u16 = 0x037f;
pub const MXCSR_DEFAULT: u32 = 0x1f80;
/// Byte offsets inside the FXSAVE64 area.
pub const FX_FCW: usize = 0;
pub const FX_MXCSR: usize = 24;
/// RFLAGS at process start: reserved bit 1 and IF.
pub const RFLAGS_INIT: u64 = 0x202;

/// 512-byte FXSAVE64 area; `fxsave64`/`fxrstor64` need 16-byte alignment.
#[repr(C, align(16))]
pub struct FxArea(pub [u8; 512]);

#[repr(C)]
pub struct Cpu {
    pub gpr: [u64; 16],
    pub rip: u64,
    pub rflags: u64,
    pub fs_base: u64,
    pub gs_base: u64,
    pub fxsave: FxArea,
    pub scratch: [u64; 4],
    pub host_rsp: u64,
    pub host_fs: u64,
    pub next_rip: u64,
    pub exit_reason: u64,
    /// Bit `n - 1` set means guest signal `n` is pending.
    pub pending_signals: u64,
    /// Signal state of the thread, owned by `signal/` (K2).
    pub signal_state: *mut c_void,
    pub tid: u64,
    pub clear_child_tid: u64,
    /// Guest address of a write to code that faulted while the cache lock was busy; the
    /// dispatcher serves it (contract 3.13). 0 if none.
    pub smc_write: u64,
    /// Rip of a guest write whose fault invalidated its own block: the dispatcher runs
    /// that one instruction in a block of its own. 0 if none.
    pub smc_step: u64,
}

pub const OFF_GPR: usize = offset_of!(Cpu, gpr);
pub const OFF_RIP: usize = offset_of!(Cpu, rip);
pub const OFF_RFLAGS: usize = offset_of!(Cpu, rflags);
pub const OFF_FS_BASE: usize = offset_of!(Cpu, fs_base);
pub const OFF_GS_BASE: usize = offset_of!(Cpu, gs_base);
pub const OFF_FXSAVE: usize = offset_of!(Cpu, fxsave);
pub const OFF_SCRATCH: usize = offset_of!(Cpu, scratch);
pub const OFF_HOST_RSP: usize = offset_of!(Cpu, host_rsp);
pub const OFF_HOST_FS: usize = offset_of!(Cpu, host_fs);
pub const OFF_NEXT_RIP: usize = offset_of!(Cpu, next_rip);
pub const OFF_EXIT_REASON: usize = offset_of!(Cpu, exit_reason);
pub const OFF_PENDING_SIGNALS: usize = offset_of!(Cpu, pending_signals);
pub const OFF_SIGNAL_STATE: usize = offset_of!(Cpu, signal_state);
pub const OFF_TID: usize = offset_of!(Cpu, tid);
pub const OFF_CLEAR_CHILD_TID: usize = offset_of!(Cpu, clear_child_tid);

/// Offset of general register `reg` (encoding order).
pub const fn off_gpr(reg: usize) -> usize {
    OFF_GPR + 8 * reg
}

/// Offset of scratch slot `slot`.
pub const fn off_scratch(slot: usize) -> usize {
    OFF_SCRATCH + 8 * slot
}

const _: () = assert!(OFF_FXSAVE % 16 == 0);
const _: () = assert!(core::mem::align_of::<Cpu>() >= 16);

impl Cpu {
    /// A thread state that starts at `rip` with stack `rsp`; boxed so its address is stable
    /// for the GS base.
    pub fn new(rip: u64, rsp: u64) -> Box<Cpu> {
        let mut fxsave = FxArea([0; 512]);
        fxsave.0[FX_FCW..FX_FCW + 2].copy_from_slice(&FCW_DEFAULT.to_le_bytes());
        fxsave.0[FX_MXCSR..FX_MXCSR + 4].copy_from_slice(&MXCSR_DEFAULT.to_le_bytes());
        let mut gpr = [0; 16];
        gpr[RSP] = rsp;
        Box::new(Cpu {
            gpr,
            rip,
            rflags: RFLAGS_INIT,
            fs_base: 0,
            gs_base: 0,
            fxsave,
            scratch: [0; 4],
            host_rsp: 0,
            host_fs: 0,
            next_rip: 0,
            exit_reason: 0,
            pending_signals: 0,
            signal_state: core::ptr::null_mut(),
            tid: 0,
            clear_child_tid: 0,
            smc_write: 0,
            smc_step: 0,
        })
    }
}

#[cfg(test)]
mod tests;
