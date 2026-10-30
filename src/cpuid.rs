//! CPUID for the guest (contract 3.9). For now the host values pass through unchanged;
//! k1-04 adds the filter.

use crate::cpu::{Cpu, RAX, RBX, RCX, RDX};

/// EAX, EBX, ECX, EDX of the host CPUID for `leaf` and `subleaf`.
pub fn host(leaf: u32, subleaf: u32) -> [u32; 4] {
    #[allow(unused_unsafe)]
    let r = unsafe { core::arch::x86_64::__cpuid_count(leaf, subleaf) };
    [r.eax, r.ebx, r.ecx, r.edx]
}

/// Serves the `cpuid` the guest just executed: leaf in EAX, subleaf in ECX, and the four
/// results zero-extended like the CPU does.
pub fn handle(cpu: &mut Cpu) {
    let [a, b, c, d] = host(cpu.gpr[RAX] as u32, cpu.gpr[RCX] as u32);
    cpu.gpr[RAX] = a as u64;
    cpu.gpr[RBX] = b as u64;
    cpu.gpr[RCX] = c as u64;
    cpu.gpr[RDX] = d as u64;
}
