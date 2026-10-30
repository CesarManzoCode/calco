//! Opcode tables of the decoder (contract 3.4): one-byte map, `0F`, `0F 38` and `0F 3A`.
//! A slot that is not listed is `Invalid`. Prefix bytes, REX and the `0F` escape never
//! reach a lookup, so their one-byte slots stay `Invalid` too.
//!
//! An `Op` that the host re-encodes (`Plain`) may cover encodings the CPU itself rejects
//! or aliases (reserved x87 register forms, hint NOPs): the host then does exactly what
//! the native run does. What must not be `Plain` is anything outside the scope that the
//! host would execute, since CPUID does not announce it.

use self::Ext::*;
use self::Signal::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Map {
    OneByte,
    Map0F,
    Map0F38,
    Map0F3A,
}

/// Signal the kernel delivers for a trapping instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    Sigill,
    Sigtrap,
    Sigsegv,
}

impl Signal {
    pub const fn number(self) -> i32 {
        match self {
            Sigill => 4,
            Sigtrap => 5,
            Sigsegv => 11,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// Re-encoded as is.
    Plain,
    /// Direct jump: `jmp`, `jcc`, `loop*`, `jrcxz`. `Insn::target` holds the destination.
    Branch,
    /// `call rel32` (with `target`) or `call r/m` (`FF /2`, without).
    Call,
    /// `ret`, `ret imm16`; also the far forms `CA`/`CB`, which the emitter tells apart by opcode.
    Ret,
    /// `jmp r/m` (`FF /4`).
    IndirectBranch,
    Syscall,
    Cpuid,
    /// Instruction that ends in the signal the kernel would deliver.
    Trap(Signal),
    /// Table marker only: `decode` turns it into `DecodeError::Invalid`.
    Invalid,
}

/// ISA extension, as in the `ext` column of `isa/scope.tsv`, plus the CPUID features that
/// have no column there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ext {
    Base,
    X87,
    Mmx,
    Sse,
    Sse2,
    Sse3,
    Ssse3,
    Sse41,
    Sse42,
    Popcnt,
    Pclmul,
    Rdrand,
}

/// Whether the instruction has a ModRM byte and which forms of it are valid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modrm {
    No,
    Any,
    /// Only `mod != 3`.
    Mem,
    /// Only `mod == 3`.
    Reg,
    /// `mov` to or from CR/DR: `mod` is ignored and taken as 3, so no SIB or displacement.
    Control,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Imm {
    No,
    Ib,
    Iw,
    /// 2 bytes with `66`, 4 otherwise (also with REX.W, sign-extended).
    Iz,
    /// `B8+r`: 8 bytes with REX.W, 2 with `66`, 4 otherwise.
    Iv,
    /// `enter`: iw then ib.
    IwIb,
    /// `A0–A3`: absolute address of 8 bytes, 4 with `67`. Stored as displacement.
    Moffs,
    Rel8,
    Rel32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Op {
    pub modrm: Modrm,
    pub imm: Imm,
    pub class: Class,
    pub ext: Ext,
}

#[derive(Clone, Copy, Debug)]
pub enum Entry {
    Op(Op),
    /// By mandatory prefix: none, `66`, `F3`, `F2`.
    Prefixed([Op; 4]),
    /// By `ModRM.reg`.
    Group(&'static [Entry; 8]),
    /// By `ModRM.mod`: memory form, register form.
    Split(&'static Entry, &'static Entry),
    /// Register form by `ModRM.rm`.
    ByRm(&'static [Op; 8]),
}

const fn op(modrm: Modrm, imm: Imm, class: Class, ext: Ext) -> Op {
    Op {
        modrm,
        imm,
        class,
        ext,
    }
}

const INV_OP: Op = op(Modrm::No, Imm::No, Class::Invalid, Base);
const INV: Entry = Entry::Op(INV_OP);

const fn one(o: Op) -> Entry {
    Entry::Op(o)
}

const fn pfx(none: Op, p66: Op, f3: Op, f2: Op) -> Entry {
    Entry::Prefixed([none, p66, f3, f2])
}

const fn bare(ext: Ext) -> Op {
    op(Modrm::No, Imm::No, Class::Plain, ext)
}

const fn imm(i: Imm) -> Op {
    op(Modrm::No, i, Class::Plain, Base)
}

const fn rm(ext: Ext) -> Op {
    op(Modrm::Any, Imm::No, Class::Plain, ext)
}

const fn rm_ib(ext: Ext) -> Op {
    op(Modrm::Any, Imm::Ib, Class::Plain, ext)
}

const fn rm_iz() -> Op {
    op(Modrm::Any, Imm::Iz, Class::Plain, Base)
}

const fn mem(ext: Ext) -> Op {
    op(Modrm::Mem, Imm::No, Class::Plain, ext)
}

const fn reg(ext: Ext) -> Op {
    op(Modrm::Reg, Imm::No, Class::Plain, ext)
}

const fn reg_ib(ext: Ext) -> Op {
    op(Modrm::Reg, Imm::Ib, Class::Plain, ext)
}

const fn branch(i: Imm) -> Op {
    op(Modrm::No, i, Class::Branch, Base)
}

const fn trap(sig: Signal) -> Op {
    op(Modrm::No, Imm::No, Class::Trap(sig), Base)
}

const fn trap_rm(modrm: Modrm, sig: Signal) -> Op {
    op(modrm, Imm::No, Class::Trap(sig), Base)
}

/// `ps`, `pd`, `ss`, `sd` forms.
const fn sse_all(ps: Ext, pd: Ext) -> Entry {
    pfx(rm(ps), rm(pd), rm(ps), rm(pd))
}

/// `ps`, `pd` forms.
const fn ps_pd() -> Entry {
    pfx(rm(Sse), rm(Sse2), INV_OP, INV_OP)
}

/// MMX form without prefix, SSE2 form with `66`.
const fn mmx_sse2(mmx: Ext) -> Entry {
    pfx(rm(mmx), rm(Sse2), INV_OP, INV_OP)
}

const fn only66(o: Op) -> Entry {
    pfx(INV_OP, o, INV_OP, INV_OP)
}

pub static ONE_BYTE: [Entry; 256] = one_byte();
pub static MAP_0F: [Entry; 256] = map_0f();
pub static MAP_0F38: [Entry; 256] = map_0f38();
pub static MAP_0F3A: [Entry; 256] = map_0f3a();

const fn one_byte() -> [Entry; 256] {
    let mut t = [INV; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = one_byte_slot(i as u8);
        i += 1;
    }
    t
}

const fn map_0f() -> [Entry; 256] {
    let mut t = [INV; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = map_0f_slot(i as u8);
        i += 1;
    }
    t
}

const fn map_0f38() -> [Entry; 256] {
    let mut t = [INV; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = map_0f38_slot(i as u8);
        i += 1;
    }
    t
}

const fn map_0f3a() -> [Entry; 256] {
    let mut t = [INV; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = map_0f3a_slot(i as u8);
        i += 1;
    }
    t
}

// Invalid in 64-bit mode: 06 07 0E 16 17 1E 1F 27 2F 37 3F 60 61 82 9A CE D4 D5 D6 EA.
// Out of the scope: C4/C5 (VEX), 62 (EVEX), 8E (mov Sreg), CF (iret), FF /3 /5 (far).
const fn one_byte_slot(b: u8) -> Entry {
    match b {
        0x00..=0x3f => match b & 7 {
            0..=3 => one(rm(Base)),
            4 => one(imm(Imm::Ib)),
            5 => one(imm(Imm::Iz)),
            _ => INV,
        },
        0x50..=0x5f => one(bare(Base)),
        0x63 => one(rm(Base)),
        0x68 => one(imm(Imm::Iz)),
        0x69 => one(rm_iz()),
        0x6a => one(imm(Imm::Ib)),
        0x6b => one(rm_ib(Base)),
        // ins, outs: #GP without I/O permission.
        0x6c..=0x6f => one(trap(Sigsegv)),
        0x70..=0x7f => one(branch(Imm::Rel8)),
        0x80 | 0x83 => one(rm_ib(Base)),
        0x81 => one(rm_iz()),
        0x84..=0x8c => one(rm(Base)),
        0x8d => one(mem(Base)),
        0x8f => Entry::Group(&GRP1A),
        0x90..=0x99 => one(bare(Base)),
        0x9b => one(bare(X87)),
        0x9c..=0x9f => one(bare(Base)),
        0xa0..=0xa3 => one(imm(Imm::Moffs)),
        0xa4..=0xa7 | 0xaa..=0xaf => one(bare(Base)),
        0xa8 => one(imm(Imm::Ib)),
        0xa9 => one(imm(Imm::Iz)),
        0xb0..=0xb7 => one(imm(Imm::Ib)),
        0xb8..=0xbf => one(imm(Imm::Iv)),
        0xc0 | 0xc1 => one(rm_ib(Base)),
        0xc2 | 0xca => one(op(Modrm::No, Imm::Iw, Class::Ret, Base)),
        0xc3 | 0xcb => one(op(Modrm::No, Imm::No, Class::Ret, Base)),
        0xc6 => Entry::Group(&GRP11_C6),
        0xc7 => Entry::Group(&GRP11_C7),
        0xc8 => one(imm(Imm::IwIb)),
        0xc9 => one(bare(Base)),
        0xcc => one(trap(Sigtrap)),
        // int n: #GP through a DPL 0 gate; `decode` turns `CD 03` into SIGTRAP.
        0xcd => one(op(Modrm::No, Imm::Ib, Class::Trap(Sigsegv), Base)),
        0xd0..=0xd3 => one(rm(Base)),
        0xd7 => one(bare(Base)),
        0xd8..=0xdf => one(rm(X87)),
        0xe0..=0xe3 | 0xeb => one(branch(Imm::Rel8)),
        0xe4..=0xe7 => one(op(Modrm::No, Imm::Ib, Class::Trap(Sigsegv), Base)),
        0xe8 => one(op(Modrm::No, Imm::Rel32, Class::Call, Base)),
        0xe9 => one(branch(Imm::Rel32)),
        // in, out, hlt, cli, sti.
        0xec..=0xef | 0xf4 | 0xfa | 0xfb => one(trap(Sigsegv)),
        0xf1 => one(trap(Sigtrap)),
        0xf5 | 0xf8 | 0xf9 | 0xfc | 0xfd => one(bare(Base)),
        0xf6 => Entry::Group(&GRP3_F6),
        0xf7 => Entry::Group(&GRP3_F7),
        0xfe => Entry::Group(&GRP4),
        0xff => Entry::Group(&GRP5),
        _ => INV,
    }
}

/// `8F`: pop r/m; the other `reg` values are XOP.
static GRP1A: [Entry; 8] = [one(rm(Base)), INV, INV, INV, INV, INV, INV, INV];
/// `C6`/`C7`: mov r/m, imm; `/7` is xabort/xbegin (RTM).
static GRP11_C6: [Entry; 8] = [one(rm_ib(Base)), INV, INV, INV, INV, INV, INV, INV];
static GRP11_C7: [Entry; 8] = [one(rm_iz()), INV, INV, INV, INV, INV, INV, INV];
/// `F6`/`F7`: only test (`/0`, and its alias `/1`) takes an immediate.
static GRP3_F6: [Entry; 8] = [
    one(rm_ib(Base)),
    one(rm_ib(Base)),
    one(rm(Base)),
    one(rm(Base)),
    one(rm(Base)),
    one(rm(Base)),
    one(rm(Base)),
    one(rm(Base)),
];
static GRP3_F7: [Entry; 8] = [
    one(rm_iz()),
    one(rm_iz()),
    one(rm(Base)),
    one(rm(Base)),
    one(rm(Base)),
    one(rm(Base)),
    one(rm(Base)),
    one(rm(Base)),
];
static GRP4: [Entry; 8] = [one(rm(Base)), one(rm(Base)), INV, INV, INV, INV, INV, INV];
static GRP5: [Entry; 8] = [
    one(rm(Base)),
    one(rm(Base)),
    one(op(Modrm::Any, Imm::No, Class::Call, Base)),
    INV,
    one(op(Modrm::Any, Imm::No, Class::IndirectBranch, Base)),
    INV,
    one(rm(Base)),
    INV,
];

// Out of the scope: 0F 02/03 (lar, lsl), 0D (prefetchw), 0E/0F (3DNow!), 34/35 (#UD in
// long mode on AMD), 78/79 (VMX, SSE4A), A1/A9 (pop fs/gs), AA (rsm), B2/B4/B5 (lss, lfs, lgs).
const fn map_0f_slot(b: u8) -> Entry {
    match b {
        0x00 => Entry::Group(&GRP6),
        0x01 => Entry::Group(&GRP7),
        0x05 => one(op(Modrm::No, Imm::No, Class::Syscall, Base)),
        // clts, sysret, invd, wbinvd, wrmsr, rdmsr, rdpmc.
        0x06..=0x09 | 0x30 | 0x32 | 0x33 => one(trap(Sigsegv)),
        // ud2; ud0 has no ModRM on AMD.
        0x0b | 0xff => one(trap(Sigill)),
        0x10 | 0x11 => sse_all(Sse, Sse2),
        0x12 => pfx(rm(Sse), mem(Sse2), rm(Sse3), rm(Sse3)),
        0x13 | 0x17 => pfx(mem(Sse), mem(Sse2), INV_OP, INV_OP),
        0x14 | 0x15 => ps_pd(),
        0x16 => pfx(rm(Sse), mem(Sse2), rm(Sse3), INV_OP),
        // prefetch and the hint NOP space, endbr64 included.
        0x18..=0x1f => one(rm(Base)),
        0x20..=0x23 => one(trap_rm(Modrm::Control, Sigsegv)),
        0x28 | 0x29 | 0x2e | 0x2f => ps_pd(),
        0x2a | 0x2c | 0x2d => sse_all(Sse, Sse2),
        0x2b => pfx(mem(Sse), mem(Sse2), INV_OP, INV_OP),
        0x31 => one(bare(Base)),
        0x40..=0x4f => one(rm(Base)),
        0x50 => pfx(reg(Sse), reg(Sse2), INV_OP, INV_OP),
        0x51 | 0x58 | 0x59 | 0x5c..=0x5f => sse_all(Sse, Sse2),
        0x52 | 0x53 => pfx(rm(Sse), INV_OP, rm(Sse), INV_OP),
        0x54..=0x57 => ps_pd(),
        0x5a => sse_all(Sse2, Sse2),
        0x5b => pfx(rm(Sse2), rm(Sse2), rm(Sse2), INV_OP),
        0x60..=0x6b | 0x6e | 0x74..=0x76 => mmx_sse2(Mmx),
        0x6c | 0x6d => only66(rm(Sse2)),
        0x6f | 0x7e | 0x7f => pfx(rm(Mmx), rm(Sse2), rm(Sse2), INV_OP),
        0x70 => pfx(rm_ib(Sse), rm_ib(Sse2), rm_ib(Sse2), rm_ib(Sse2)),
        0x71 => Entry::Group(&GRP12),
        0x72 => Entry::Group(&GRP13),
        0x73 => Entry::Group(&GRP14),
        0x77 => pfx(bare(Mmx), INV_OP, INV_OP, INV_OP),
        0x7c | 0x7d | 0xd0 => pfx(INV_OP, rm(Sse3), INV_OP, rm(Sse3)),
        0x80..=0x8f => one(branch(Imm::Rel32)),
        0x90..=0x9f => one(rm(Base)),
        0xa0 | 0xa8 => one(bare(Base)),
        0xa2 => one(op(Modrm::No, Imm::No, Class::Cpuid, Base)),
        0xa3 | 0xa5 | 0xab | 0xad | 0xaf => one(rm(Base)),
        0xa4 | 0xac => one(rm_ib(Base)),
        0xae => Entry::Group(&GRP15),
        0xb0 | 0xb1 | 0xb3 | 0xb6 | 0xb7 => one(rm(Base)),
        // popcnt; without F3 it is jmpe.
        0xb8 => pfx(INV_OP, INV_OP, rm(Popcnt), INV_OP),
        0xb9 => one(trap_rm(Modrm::Any, Sigill)),
        0xba => Entry::Group(&GRP8),
        // bsf/bsr also with F3: the host runs them as tzcnt/lzcnt, as natively.
        0xbb..=0xbf => one(rm(Base)),
        0xc0 | 0xc1 => one(rm(Base)),
        0xc2 => pfx(rm_ib(Sse), rm_ib(Sse2), rm_ib(Sse), rm_ib(Sse2)),
        0xc3 => pfx(mem(Sse2), INV_OP, INV_OP, INV_OP),
        0xc4 | 0xc6 => pfx(rm_ib(Sse), rm_ib(Sse2), INV_OP, INV_OP),
        0xc5 => pfx(reg_ib(Sse), reg_ib(Sse2), INV_OP, INV_OP),
        0xc7 => Entry::Group(&GRP9),
        0xc8..=0xcf => one(bare(Base)),
        0xd1..=0xd3 | 0xd5 | 0xd8 | 0xd9 | 0xdb..=0xdd | 0xdf => mmx_sse2(Mmx),
        0xe1 | 0xe2 | 0xe5 | 0xe8 | 0xe9 | 0xeb..=0xed | 0xef => mmx_sse2(Mmx),
        0xf1..=0xf3 | 0xf5 | 0xf8..=0xfa | 0xfc..=0xfe => mmx_sse2(Mmx),
        // The MMX forms of paddq, pmuludq and psubq came with SSE2.
        0xd4 | 0xf4 | 0xfb => mmx_sse2(Sse2),
        // The MMX forms of pminub, pmaxub, pavgb, pavgw, pmulhuw, pminsw, pmaxsw and psadbw came with SSE.
        0xda | 0xde | 0xe0 | 0xe3 | 0xe4 | 0xea | 0xee | 0xf6 => mmx_sse2(Sse),
        0xd6 => pfx(INV_OP, rm(Sse2), reg(Sse2), reg(Sse2)),
        0xd7 => pfx(reg(Sse), reg(Sse2), INV_OP, INV_OP),
        0xe6 => pfx(INV_OP, rm(Sse2), rm(Sse2), rm(Sse2)),
        0xe7 => pfx(mem(Sse), mem(Sse2), INV_OP, INV_OP),
        0xf0 => pfx(INV_OP, INV_OP, INV_OP, mem(Sse3)),
        0xf7 => pfx(reg(Sse), reg(Sse2), INV_OP, INV_OP),
        _ => INV,
    }
}

/// `0F 00`: sldt, str (UMIP, no emulation for 64-bit processes), lldt, ltr. verr/verw are out of the scope.
static GRP6: [Entry; 8] = [
    one(trap_rm(Modrm::Any, Sigsegv)),
    one(trap_rm(Modrm::Any, Sigsegv)),
    one(trap_rm(Modrm::Any, Sigsegv)),
    one(trap_rm(Modrm::Any, Sigsegv)),
    INV,
    INV,
    INV,
    INV,
];
/// `0F 01`: the descriptor-table forms are privileged or blocked by UMIP; of the register
/// forms only xgetbv and rdtscp are in the scope.
static GRP7: [Entry; 8] = [
    one(trap_rm(Modrm::Mem, Sigsegv)),
    one(trap_rm(Modrm::Mem, Sigsegv)),
    Entry::Split(&TRAP_MEM_SEGV, &GRP7_REG2),
    one(trap_rm(Modrm::Mem, Sigsegv)),
    one(trap_rm(Modrm::Any, Sigsegv)),
    INV,
    one(trap_rm(Modrm::Any, Sigsegv)),
    Entry::Split(&TRAP_MEM_SEGV, &GRP7_REG7),
];
static TRAP_MEM_SEGV: Entry = one(trap_rm(Modrm::Mem, Sigsegv));
static GRP7_REG2: Entry = Entry::ByRm(&GRP7_RM2);
static GRP7_REG7: Entry = Entry::ByRm(&GRP7_RM7);
/// `0F 01 D0` xgetbv, `D1` xsetbv.
static GRP7_RM2: [Op; 8] = [
    reg(Base),
    trap_rm(Modrm::Reg, Sigsegv),
    INV_OP,
    INV_OP,
    INV_OP,
    INV_OP,
    INV_OP,
    INV_OP,
];
/// `0F 01 F8` swapgs, `F9` rdtscp.
static GRP7_RM7: [Op; 8] = [
    trap_rm(Modrm::Reg, Sigsegv),
    reg(Base),
    INV_OP,
    INV_OP,
    INV_OP,
    INV_OP,
    INV_OP,
    INV_OP,
];
/// `0F BA`: bt, bts, btr, btc with imm8.
static GRP8: [Entry; 8] = [
    INV,
    INV,
    INV,
    INV,
    one(rm_ib(Base)),
    one(rm_ib(Base)),
    one(rm_ib(Base)),
    one(rm_ib(Base)),
];
/// `0F C7`: cmpxchg8b/16b, rdrand. rdseed, rdpid and the VMX forms are out of the scope.
static GRP9: [Entry; 8] = [
    INV,
    one(mem(Base)),
    INV,
    INV,
    INV,
    INV,
    pfx(reg(Rdrand), reg(Rdrand), INV_OP, INV_OP),
    INV,
];
/// `0F 71`/`0F 72`: psrl, psra, psll by imm8.
static GRP12: [Entry; 8] = [
    INV,
    INV,
    pfx(reg_ib(Mmx), reg_ib(Sse2), INV_OP, INV_OP),
    INV,
    pfx(reg_ib(Mmx), reg_ib(Sse2), INV_OP, INV_OP),
    INV,
    pfx(reg_ib(Mmx), reg_ib(Sse2), INV_OP, INV_OP),
    INV,
];
static GRP13: [Entry; 8] = GRP12;
/// `0F 73`: psrlq, psrldq, psllq, pslldq by imm8; the dq forms only with `66`.
static GRP14: [Entry; 8] = [
    INV,
    INV,
    pfx(reg_ib(Mmx), reg_ib(Sse2), INV_OP, INV_OP),
    only66(reg_ib(Sse2)),
    INV,
    INV,
    pfx(reg_ib(Mmx), reg_ib(Sse2), INV_OP, INV_OP),
    only66(reg_ib(Sse2)),
];
/// `0F AE`: fxsave, fxrstor, ldmxcsr, stmxcsr, lfence, mfence, sfence, clflush. XSAVE,
/// the FSGSBASE forms (`F3`) and clflushopt (`66`) are out of the scope.
static GRP15: [Entry; 8] = [
    pfx(mem(Base), INV_OP, INV_OP, INV_OP),
    pfx(mem(Base), INV_OP, INV_OP, INV_OP),
    pfx(mem(Sse), INV_OP, INV_OP, INV_OP),
    pfx(mem(Sse), INV_OP, INV_OP, INV_OP),
    INV,
    pfx(reg(Sse2), INV_OP, INV_OP, INV_OP),
    pfx(reg(Sse2), INV_OP, INV_OP, INV_OP),
    Entry::Split(&CLFLUSH, &SFENCE),
];
static CLFLUSH: Entry = pfx(mem(Sse2), INV_OP, INV_OP, INV_OP);
static SFENCE: Entry = pfx(reg(Sse), INV_OP, INV_OP, INV_OP);

// Out of the scope: movbe (no prefix or 66 F0/F1), SHA, AES, ADX and the VMX/SVM forms.
const fn map_0f38_slot(b: u8) -> Entry {
    match b {
        0x00..=0x0b | 0x1c..=0x1e => pfx(rm(Ssse3), rm(Ssse3), INV_OP, INV_OP),
        0x10 | 0x14 | 0x15 | 0x17 | 0x20..=0x25 | 0x28 | 0x29 | 0x2b => only66(rm(Sse41)),
        0x30..=0x35 | 0x38..=0x41 => only66(rm(Sse41)),
        0x2a => only66(mem(Sse41)),
        0x37 => only66(rm(Sse42)),
        // crc32.
        0xf0 | 0xf1 => pfx(INV_OP, INV_OP, INV_OP, rm(Sse42)),
        _ => INV,
    }
}

const fn map_0f3a_slot(b: u8) -> Entry {
    match b {
        0x0f => pfx(rm_ib(Ssse3), rm_ib(Ssse3), INV_OP, INV_OP),
        0x08..=0x0e | 0x14..=0x17 | 0x20..=0x22 | 0x40..=0x42 => only66(rm_ib(Sse41)),
        0x44 => only66(rm_ib(Pclmul)),
        // pcmpestrm, pcmpestri, pcmpistrm, pcmpistri.
        0x60..=0x63 => only66(rm_ib(Sse42)),
        _ => INV,
    }
}
