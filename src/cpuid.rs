//! CPUID for the guest (contract 3.9): the host values filtered down to the ISA of the
//! scope, x86-64-v2 without XSAVE, so glibc never picks AVX or `xsave` routines.

use crate::cpu::{Cpu, RAX, RBX, RCX, RDX};

// Leaf 1 EDX.
const FPU: u32 = 1 << 0;
const PSE: u32 = 1 << 3;
const TSC: u32 = 1 << 4;
const PAE: u32 = 1 << 6;
const CX8: u32 = 1 << 8;
const PGE: u32 = 1 << 13;
const CMOV: u32 = 1 << 15;
const CLFSH: u32 = 1 << 19;
const MMX: u32 = 1 << 23;
const FXSR: u32 = 1 << 24;
const SSE: u32 = 1 << 25;
const SSE2: u32 = 1 << 26;

// Leaf 1 ECX.
const SSE3: u32 = 1 << 0;
const PCLMULQDQ: u32 = 1 << 1;
const SSSE3: u32 = 1 << 9;
pub const FMA: u32 = 1 << 12;
const CX16: u32 = 1 << 13;
const SSE41: u32 = 1 << 19;
const SSE42: u32 = 1 << 20;
pub const MOVBE: u32 = 1 << 22;
const POPCNT: u32 = 1 << 23;
pub const AES: u32 = 1 << 25;
pub const XSAVE: u32 = 1 << 26;
pub const OSXSAVE: u32 = 1 << 27;
pub const AVX: u32 = 1 << 28;
pub const F16C: u32 = 1 << 29;
const RDRAND: u32 = 1 << 30;

// Leaf 0x80000001 ECX and EDX.
const LAHF_SAHF: u32 = 1 << 0;
pub const ABM: u32 = 1 << 5;
pub const SSE4A: u32 = 1 << 6;
const SYSCALL: u32 = 1 << 11;
const NX: u32 = 1 << 20;
const RDTSCP: u32 = 1 << 27;
const LM: u32 = 1 << 29;

/// Bits that survive the filter when the host has them.
pub const LEAF1_EDX: u32 =
    FPU | PSE | TSC | PAE | CX8 | PGE | CMOV | CLFSH | MMX | FXSR | SSE | SSE2;
pub const LEAF1_ECX: u32 = SSE3 | PCLMULQDQ | SSSE3 | CX16 | SSE41 | SSE42 | POPCNT | RDRAND;
pub const EXT1_ECX: u32 = LAHF_SAHF;
pub const EXT1_EDX: u32 = SYSCALL | NX | RDTSCP | LM;

/// Leaf 1 ECX bits the contract clears by name; the allow list above already drops them.
const LEAF1_ECX_CLEARED: u32 = FMA | MOVBE | AES | XSAVE | OSXSAVE | AVX | F16C;
const _: () = assert!(LEAF1_ECX & LEAF1_ECX_CLEARED == 0);
const _: () = assert!(EXT1_ECX & (ABM | SSE4A) == 0);

/// EAX, EBX, ECX, EDX of the host CPUID for `leaf` and `subleaf`.
pub fn host(leaf: u32, subleaf: u32) -> [u32; 4] {
    #[allow(unused_unsafe)]
    let r = unsafe { core::arch::x86_64::__cpuid_count(leaf, subleaf) };
    [r.eax, r.ebx, r.ecx, r.edx]
}

/// What the guest sees for `leaf` given the host values. Leaves with feature bits keep
/// only the allowed ones; informative leaves pass as they are; any other leaf, leaf 7
/// (AVX2, AVX-512, BMI, SHA, FSGSBASE, CET) and `0xD` (XSAVE) included, reads as zeros.
pub fn filter(leaf: u32, host: [u32; 4]) -> [u32; 4] {
    let [a, b, c, d] = host;
    match leaf {
        1 => [a, b, c & LEAF1_ECX, d & LEAF1_EDX],
        0x8000_0001 => [a, b, c & EXT1_ECX, d & EXT1_EDX],
        0
        | 4
        | 0xb
        | 0x8000_0000
        | 0x8000_0002..=0x8000_0006
        | 0x8000_0008
        | 0x8000_001d
        | 0x8000_001e => host,
        _ => [0; 4],
    }
}

/// `AT_HWCAP` for the loader: CPUID.1:EDX already filtered.
pub fn hwcap() -> u64 {
    filter(1, host(1, 0))[3] as u64
}

/// Serves the `cpuid` the guest just executed: leaf in EAX, subleaf in ECX, and the four
/// results zero-extended like the CPU does.
pub fn handle(cpu: &mut Cpu) {
    let leaf = cpu.gpr[RAX] as u32;
    let [a, b, c, d] = filter(leaf, host(leaf, cpu.gpr[RCX] as u32));
    cpu.gpr[RAX] = a as u64;
    cpu.gpr[RBX] = b as u64;
    cpu.gpr[RCX] = c as u64;
    cpu.gpr[RDX] = d as u64;
}

#[cfg(test)]
mod tests;
