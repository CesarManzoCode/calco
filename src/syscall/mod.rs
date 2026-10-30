//! Guest system calls (contract 3.10). For now only what a complete path needs: `write`,
//! `exit` and `exit_group` go straight to the kernel and everything else returns
//! `-ENOSYS`; k1-04 adds the rest of the list.

use crate::cpu::{Cpu, R8, R9, R10, R11, RAX, RCX, RDI, RDX, RSI};
use crate::mem::Mem;
use crate::sys::{self, ENOSYS, SYS_EXIT, SYS_EXIT_GROUP, SYS_WRITE};

/// Serves the `syscall` the guest just executed; `cpu.rip` is already past it.
pub fn handle(cpu: &mut Cpu, _mem: &mut Mem) {
    let nr = cpu.gpr[RAX] as usize;
    let [a1, a2, a3, a4, a5, a6] = [RDI, RSI, RDX, R10, R8, R9].map(|r| cpu.gpr[r] as usize);
    let ret = match nr {
        // Identity mapping: guest pointers are valid as they are.
        SYS_WRITE | SYS_EXIT | SYS_EXIT_GROUP => unsafe {
            sys::syscall6(nr, a1, a2, a3, a4, a5, a6)
        },
        _ => -(ENOSYS as isize),
    };
    cpu.gpr[RAX] = ret as u64;
    // What the `syscall` instruction leaves: the return address in RCX, RFLAGS in R11.
    cpu.gpr[RCX] = cpu.rip;
    cpu.gpr[R11] = cpu.rflags;
}
