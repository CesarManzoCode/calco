//! Block translation (contract 3.5–3.7): decodes guest instructions from a rip and emits
//! host code for them, meant to run at a known host address. It only reads guest code
//! through `GuestCode` and returns bytes plus metadata; `Jit` installs them.
//!
//! Invariants every expansion keeps (contract 3.3, 3.6):
//! - Inserted code only uses `mov`, `lea`, `jmp`, `jcc`/`loop`/`jrcxz` of the guest, `nop`
//!   and `pop gs:[next_rip]` (the `ret` itself): RFLAGS stay exactly what the guest left.
//! - Nothing is pushed or called on the guest stack; the only store below the guest RSP is
//!   the return address of a guest `call`, which the guest instruction writes anyway.
//! - A temporary register is saved in `gs:[scratch]` before use and restored after, and
//!   the pc table says, for every host byte, at which guest instruction boundary the guest
//!   state is and which registers live in scratch meanwhile.

use crate::cpu::{OFF_EXIT_REASON, OFF_GS_BASE, OFF_NEXT_RIP, off_scratch};
use crate::decode::{self, Class, DecodeError, Insn, MAX_LEN, Map, Signal};
use crate::emit::{
    self, Asm, Mem, MemRewrite, PREFIX_ADDR32, PREFIX_FS, R8, R9, R10, R11, R12, R13, R14, R15,
    RAX, RBP, RBX, RCX, RDI, REX_B, REX_R, REX_W, RSI, RSP, Reg,
};
use crate::mem::PAGE_SIZE;

/// Most guest instructions in a block.
pub const MAX_INSNS: usize = 64;
/// Scratch slots of `Cpu`.
pub const SCRATCH_SLOTS: usize = 4;

/// Kinds of exit, in the low dword of `Cpu.exit_reason`. The high dword holds the exit id of
/// an `EXIT_BRANCH`.
pub const EXIT_BRANCH: u32 = 1;
/// Indirect jump, indirect call or `ret`: the target is in `next_rip`.
pub const EXIT_INDIRECT: u32 = 2;
pub const EXIT_SYSCALL: u32 = 3;
pub const EXIT_CPUID: u32 = 4;
/// The guest gets a signal; bits 8–15 hold its number.
pub const EXIT_SIGNAL: u32 = 5;
/// The signal handler brought the thread back with the guest state already in `Cpu`, or
/// `enter` saw a pending signal (contract 3.11).
pub const EXIT_INTERRUPTED: u32 = 6;

const SIGILL: i32 = Signal::Sigill.number();
const SIGSEGV: i32 = Signal::Sigsegv.number();

pub const fn exit_signal(sig: i32) -> u32 {
    EXIT_SIGNAL | (sig as u32) << 8
}

/// Guest code as the translator sees it.
pub trait GuestCode {
    /// Copies to `buf` the bytes at `rip` that are in logically executable guest memory, up
    /// to `buf.len()`, and returns how many.
    fn fetch(&self, rip: u64, buf: &mut [u8]) -> usize;
}

/// Where the translation will live and what it jumps to.
pub struct Env {
    /// Host address of the first byte of the block.
    pub host: u64,
    /// Thunk to the exit trampoline.
    pub exit_thunk: u64,
    /// Shared exit for indirect transfers: sets `EXIT_INDIRECT` and goes to the thunk.
    pub indirect_exit: u64,
    /// Id of the first chainable exit of this block; the rest follow in order.
    pub first_exit_id: u32,
}

/// Guest instruction boundary of a pc table entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuestPc {
    At(u64),
    /// The guest rip is the one in `gs:[next_rip]` (after `ret` or an indirect transfer).
    NextRip,
}

/// Scratch slot `i` holds the guest value of register `temps[i]`, if any.
pub type Temps = [Option<Reg>; SCRATCH_SLOTS];

/// From `host_off` up to the next entry, the guest state is the one at the boundary `pc`:
/// the registers hold it except the ones in `temps`, whose value is in scratch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PcEntry {
    pub host_off: u32,
    pub pc: GuestPc,
    pub temps: Temps,
}

/// A chainable exit: a `jmp rel32` or `jcc rel32` whose rel32 (4-byte aligned in the host)
/// points at its stub until it is patched to point at the translation of `target`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitSite {
    /// Offset of the rel32 field.
    pub jump: u32,
    /// Offset of the stub that leaves to the dispatcher.
    pub stub: u32,
    pub target: u64,
}

#[derive(Debug)]
pub struct Translation {
    pub rip: u64,
    /// End of the last guest byte read.
    pub guest_end: u64,
    pub insns: usize,
    pub code: Vec<u8>,
    pub pcs: Vec<PcEntry>,
    pub exits: Vec<ExitSite>,
}

impl Translation {
    /// Pc table entry for the host byte at `host_off`.
    pub fn lookup(&self, host_off: u32) -> Option<&PcEntry> {
        let n = self.pcs.partition_point(|e| e.host_off <= host_off);
        n.checked_sub(1).map(|i| &self.pcs[i])
    }
}

/// The first instruction of a block cannot be fetched: the guest gets `signal` with
/// `addr` as the faulting address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fault {
    pub signal: i32,
    pub addr: u64,
}

/// Translates the block that starts at `rip`.
pub fn translate(code: &dyn GuestCode, rip: u64, env: &Env) -> Result<Translation, Fault> {
    translate_max(code, rip, env, MAX_INSNS)
}

/// Translates the block that starts at `rip` with at most `max` guest instructions
/// (1 for the step after a write to code, contract 3.13).
pub fn translate_max(
    code: &dyn GuestCode,
    rip: u64,
    env: &Env,
    max: usize,
) -> Result<Translation, Fault> {
    let mut tr = Tr {
        env,
        asm: Asm::new(env.host),
        temps: [None; SCRATCH_SLOTS],
        pcs: Vec::new(),
        pending: Vec::new(),
    };
    let page = rip & !(PAGE_SIZE - 1);
    let mut pc = rip;
    let mut insns = 0;
    let mut guest_end = rip;
    loop {
        if insns == max.min(MAX_INSNS) || (pc & !(PAGE_SIZE - 1)) != page {
            tr.mark(GuestPc::At(pc));
            tr.chain_jmp(pc);
            break;
        }
        let mut buf = [0u8; MAX_LEN];
        let avail = code.fetch(pc, &mut buf);
        let insn = match decode::decode(&buf[..avail], pc) {
            Ok(insn) => insn,
            // The instruction runs into memory without execute permission.
            Err(DecodeError::Truncated) if insns == 0 => {
                return Err(Fault {
                    signal: SIGSEGV,
                    addr: pc + avail as u64,
                });
            }
            Err(DecodeError::Truncated) => {
                tr.mark(GuestPc::At(pc));
                tr.chain_jmp(pc);
                break;
            }
            Err(DecodeError::Invalid) => {
                insns += 1;
                guest_end = pc + 1;
                tr.mark(GuestPc::At(pc));
                tr.exit_inline(pc, exit_signal(SIGILL));
                break;
            }
        };
        insns += 1;
        guest_end = pc + insn.len as u64;
        tr.mark(GuestPc::At(pc));
        if tr.insn(&insn) {
            break;
        }
        pc = guest_end;
    }

    let pending = std::mem::take(&mut tr.pending);
    let mut exits = Vec::with_capacity(pending.len());
    for (i, (jump, target)) in pending.into_iter().enumerate() {
        let stub = tr.asm.len();
        let stub_pc = tr.asm.pc();
        tr.asm.set_rel32(jump, stub_pc);
        tr.mark(GuestPc::At(target));
        tr.set_next_rip(target);
        tr.set_exit_reason(EXIT_BRANCH, env.first_exit_id + i as u32);
        tr.asm.jmp_rel32(env.exit_thunk);
        exits.push(ExitSite {
            jump: jump as u32,
            stub: stub as u32,
            target,
        });
    }
    Ok(Translation {
        rip,
        guest_end,
        insns,
        code: tr.asm.into_bytes(),
        pcs: tr.pcs,
        exits,
    })
}

struct Tr<'a> {
    env: &'a Env,
    asm: Asm,
    temps: Temps,
    pcs: Vec<PcEntry>,
    /// Chainable jumps still pointing nowhere: offset of the rel32 and guest target.
    pending: Vec<(usize, u64)>,
}

impl Tr<'_> {
    /// From here on the guest state is the boundary `pc` with the current temporaries.
    fn mark(&mut self, pc: GuestPc) {
        let entry = PcEntry {
            host_off: self.asm.len() as u32,
            pc,
            temps: self.temps,
        };
        match self.pcs.last_mut() {
            Some(last) if last.host_off == entry.host_off => *last = entry,
            _ => self.pcs.push(entry),
        }
    }

    fn save(&mut self, slot: usize, reg: Reg, pc: GuestPc) {
        self.asm.mov_gs_store(off_scratch(slot), reg);
        self.temps[slot] = Some(reg);
        self.mark(pc);
    }

    fn restore(&mut self, slot: usize, pc: GuestPc) {
        let reg = self.temps[slot]
            .take()
            .expect("restoring an empty scratch slot");
        self.asm.mov_gs_load(reg, off_scratch(slot));
        self.mark(pc);
    }

    /// `jmp rel32` to the exit stub of `target`, patchable later to chain.
    fn chain_jmp(&mut self, target: u64) {
        self.asm.pad_for_field(1, 4);
        let at = self.asm.jmp_rel32_fwd();
        self.pending.push((at, target));
    }

    /// `jcc rel32` to the exit stub of `target`, patchable later to chain.
    fn chain_jcc(&mut self, cc: u8, target: u64) {
        self.asm.pad_for_field(2, 4);
        let at = self.asm.jcc_rel32_fwd(cc);
        self.pending.push((at, target));
    }

    fn set_next_rip(&mut self, rip: u64) {
        self.asm.mov_gs_dword(OFF_NEXT_RIP, rip as u32);
        self.asm.mov_gs_dword(OFF_NEXT_RIP + 4, (rip >> 32) as u32);
    }

    fn set_exit_reason(&mut self, kind: u32, id: u32) {
        self.asm.mov_gs_dword(OFF_EXIT_REASON, kind);
        self.asm.mov_gs_dword(OFF_EXIT_REASON + 4, id);
    }

    /// Leaves to the dispatcher with `reason`, to continue at `next`.
    fn exit_inline(&mut self, next: u64, reason: u32) {
        self.set_next_rip(next);
        self.set_exit_reason(reason, 0);
        self.asm.jmp_rel32(self.env.exit_thunk);
    }

    /// Emits one guest instruction; returns whether it ends the block.
    fn insn(&mut self, insn: &Insn) -> bool {
        let next = insn.rip + insn.len as u64;
        match insn.class {
            Class::Plain if insn.seg_gs && implicit_segment_memory(insn) => {
                // The `gs` would apply to an implicit operand that has no ModRM to rewrite.
                self.exit_inline(insn.rip, exit_signal(SIGILL));
                true
            }
            Class::Plain => {
                self.plain(insn);
                false
            }
            Class::Branch => {
                self.branch(insn);
                true
            }
            Class::Call => {
                self.call(insn);
                true
            }
            Class::Ret => {
                self.ret(insn);
                true
            }
            Class::IndirectBranch => {
                self.load_target(insn);
                self.mark(GuestPc::NextRip);
                self.asm.jmp_rel32(self.env.indirect_exit);
                true
            }
            Class::Syscall => {
                self.exit_inline(next, EXIT_SYSCALL);
                true
            }
            Class::Cpuid => {
                self.exit_inline(next, EXIT_CPUID);
                true
            }
            Class::Trap(sig) => {
                // Like the kernel: after int3/int1 the rip is past the instruction.
                let at = if sig == Signal::Sigtrap {
                    next
                } else {
                    insn.rip
                };
                self.exit_inline(at, exit_signal(sig.number()));
                true
            }
            Class::Invalid => {
                self.exit_inline(insn.rip, exit_signal(SIGILL));
                true
            }
        }
    }

    fn plain(&mut self, insn: &Insn) {
        let at = GuestPc::At(insn.rip);
        let next = GuestPc::At(insn.rip + insn.len as u64);
        if insn.seg_gs && gs_memory(insn) {
            let t = pick_temp(insn);
            self.save(0, t, at);
            self.address_into(insn, t);
            self.recode(insn, MemRewrite::Base(t));
            self.mark(next);
            self.restore(0, next);
        } else if insn.rip_relative {
            let target = rip_target(insn);
            if self
                .asm
                .recode(insn, MemRewrite::RipTarget(target))
                .is_err()
            {
                let t = pick_temp(insn);
                self.save(0, t, at);
                self.asm.mov_imm64(t, target);
                self.recode(insn, MemRewrite::Base(t));
                self.mark(next);
                self.restore(0, next);
            }
        } else {
            self.recode(insn, MemRewrite::Keep);
        }
    }

    fn recode(&mut self, insn: &Insn, rw: MemRewrite) {
        self.asm
            .recode(insn, rw)
            .expect("only RIP-relative recoding can be out of range");
    }

    /// Leaves in `t` (already saved) the linear address of the memory operand of `insn`,
    /// with the guest GS base added if it has the `gs` prefix.
    fn address_into(&mut self, insn: &Insn, t: Reg) {
        let at = GuestPc::At(insn.rip);
        if insn.rip_relative {
            self.asm.mov_imm64(t, rip_target(insn));
        } else if emit::is_moffs(insn) {
            self.asm.mov_imm64(t, insn.disp as u64);
        } else {
            self.recode(&with_op(insn, 0x8d, t, &[PREFIX_ADDR32]), MemRewrite::Keep);
            if pops_rsp_based(insn) {
                // `pop` computes an RSP-based address after incrementing RSP.
                self.asm.lea(t, &Mem::base(t, 8));
            }
        }
        if insn.seg_gs {
            let t2 = if t == RAX { RCX } else { RAX };
            self.save(1, t2, at);
            self.asm.mov_gs_load(t2, OFF_GS_BASE);
            self.asm.lea(t, &Mem::base_index(t, t2));
            self.restore(1, at);
        }
    }

    /// Stores in `gs:[next_rip]` the target of an indirect `call` or `jmp`.
    fn load_target(&mut self, insn: &Insn) {
        let m = insn.modrm.expect("indirect transfer without ModRM");
        if m >> 6 == 3 {
            let r = (m & 7) | (insn.rex & REX_B) << 3;
            self.asm.mov_gs_store(OFF_NEXT_RIP, r);
            return;
        }
        let at = GuestPc::At(insn.rip);
        let t = pick_temp(insn);
        self.save(0, t, at);
        let load = with_op(insn, 0x8b, t, &[PREFIX_FS, PREFIX_ADDR32]);
        if insn.seg_gs {
            self.address_into(insn, t);
            self.recode(&load, MemRewrite::Base(t));
        } else if insn.rip_relative {
            let target = rip_target(insn);
            if self
                .asm
                .recode(&load, MemRewrite::RipTarget(target))
                .is_err()
            {
                self.asm.mov_imm64(t, target);
                self.recode(&load, MemRewrite::Base(t));
            }
        } else {
            self.recode(&load, MemRewrite::Keep);
        }
        self.asm.mov_gs_store(OFF_NEXT_RIP, t);
        self.restore(0, at);
    }

    /// Writes the return address where the guest `call` writes it, then moves RSP: a fault
    /// on the store leaves the guest state untouched.
    fn push_return(&mut self, rip: u64, ret: u64) {
        let slot = Mem::base(RSP, -8);
        match i32::try_from(ret) {
            Ok(imm) => self.asm.mov_store_imm(&slot, imm),
            Err(_) => {
                let at = GuestPc::At(rip);
                self.save(0, RAX, at);
                self.asm.mov_imm64(RAX, ret);
                self.asm.mov_store(&slot, RAX);
                self.restore(0, at);
            }
        }
        self.asm.lea(RSP, &Mem::base(RSP, -8));
    }

    fn call(&mut self, insn: &Insn) {
        let ret = insn.rip + insn.len as u64;
        match insn.target {
            Some(target) => {
                self.push_return(insn.rip, ret);
                self.mark(GuestPc::At(target));
                self.chain_jmp(target);
            }
            None => {
                // The target is read before RSP moves, as the CPU does.
                self.load_target(insn);
                self.push_return(insn.rip, ret);
                self.mark(GuestPc::NextRip);
                self.asm.jmp_rel32(self.env.indirect_exit);
            }
        }
    }

    fn ret(&mut self, insn: &Insn) {
        if matches!(insn.opcode, 0xca | 0xcb) {
            // Far return: out of the scope (see LIMITATIONS.md).
            self.exit_inline(insn.rip, exit_signal(SIGSEGV));
            return;
        }
        if insn.opcode == 0xc2 && insn.imm != 0 {
            let at = GuestPc::At(insn.rip);
            self.save(0, RAX, at);
            self.asm.mov_load(RAX, &Mem::base(RSP, 0));
            self.asm.mov_gs_store(OFF_NEXT_RIP, RAX);
            self.restore(0, at);
            self.asm.lea(RSP, &Mem::base(RSP, 8 + insn.imm as i32));
        } else {
            self.asm.pop_gs(OFF_NEXT_RIP);
        }
        self.mark(GuestPc::NextRip);
        self.asm.jmp_rel32(self.env.indirect_exit);
    }

    fn branch(&mut self, insn: &Insn) {
        let target = insn.target.expect("direct branch without target");
        let next = insn.rip + insn.len as u64;
        match (insn.map, insn.opcode) {
            (Map::OneByte, 0xe9 | 0xeb) => self.chain_jmp(target),
            (Map::OneByte, 0x70..=0x7f) | (Map::Map0F, 0x80..=0x8f) => {
                self.chain_jcc(insn.opcode & 15, target);
                self.mark(GuestPc::At(next));
                self.chain_jmp(next);
            }
            // loop, loope, loopne and jrcxz only have rel8: taken, they jump over the
            // fallthrough exit to the exit of the target.
            (Map::OneByte, 0xe0..=0xe3) => {
                let rel8 = self.asm.recode_rel8(insn);
                self.mark(GuestPc::At(next));
                self.chain_jmp(next);
                let taken = self.asm.len();
                self.asm.set_rel8(rel8, taken);
                self.mark(GuestPc::At(target));
                self.chain_jmp(target);
            }
            _ => unreachable!("direct branch with opcode {:#x}", insn.opcode),
        }
    }
}

/// Address of a RIP-relative operand; with `67` the address is 32 bits.
fn rip_target(insn: &Insn) -> u64 {
    let target = insn.rip_target().expect("not RIP-relative");
    #[cfg(feature = "mutant-riprel-disp")]
    let target = target.wrapping_add(1);
    if insn.has_prefix(PREFIX_ADDR32) {
        target & 0xffff_ffff
    } else {
        target
    }
}

/// Whether the `gs` prefix changes a memory access of the ModRM or `moffs` operand. `lea`
/// and the hint NOPs compute or ignore the address without accessing it.
fn gs_memory(insn: &Insn) -> bool {
    let no_access = matches!(
        (insn.map, insn.opcode),
        (Map::OneByte, 0x8d) | (Map::Map0F, 0x18..=0x1f)
    );
    let memory = emit::is_moffs(insn) || insn.modrm.is_some_and(|m| m >> 6 != 3);
    memory && !no_access
}

/// Instructions whose segment prefix applies to an implicit memory operand: `movs`, `cmps`,
/// `lods`, `outs`, `xlat`, `maskmovq`/`maskmovdqu`.
fn implicit_segment_memory(insn: &Insn) -> bool {
    matches!(
        (insn.map, insn.opcode),
        (Map::OneByte, 0xa4..=0xa7 | 0xac | 0xad | 0x6e | 0x6f | 0xd7) | (Map::Map0F, 0xf7)
    )
}

/// `pop r/m` with an RSP base.
fn pops_rsp_based(insn: &Insn) -> bool {
    insn.map == Map::OneByte
        && insn.opcode == 0x8f
        && insn.modrm.is_some_and(|m| m >> 6 != 3 && m & 7 == 4)
        && insn.sib.is_some_and(|s| s & 7 == 4)
        && insn.rex & REX_B == 0
}

/// `insn` with its memory operand and the prefixes in `keep`, as `opcode /r` (`8B` mov or
/// `8D` lea) into the 64-bit register `dst`.
fn with_op(insn: &Insn, opcode: u8, dst: Reg, keep: &[u8]) -> Insn {
    let mut op = *insn;
    op.prefix_count = 0;
    for &p in insn.prefixes() {
        if keep.contains(&p) {
            op.prefix_bytes[op.prefix_count as usize] = p;
            op.prefix_count += 1;
        }
    }
    op.rex = 0x40 | REX_W | ((dst >> 3) & 1) * REX_R | (insn.rex & 3);
    op.map = Map::OneByte;
    op.opcode = opcode;
    op.modrm = insn.modrm.map(|m| (m & 0xc7) | ((dst & 7) << 3));
    op.imm = 0;
    op.imm_size = 0;
    op.mandatory = 0;
    op.class = Class::Plain;
    op
}

/// A temporary register `insn` does not name. Without REX, one that needs no REX either,
/// so the recoded instruction keeps meaning AH..BH in ModRM.reg.
fn pick_temp(insn: &Insn) -> Reg {
    let rex = insn.rex;
    let mut used = 1u16 << RSP;
    if let Some(m) = insn.modrm {
        used |= 1 << (((m >> 3) & 7) | (rex & 4) << 1);
        if m >> 6 != 3 {
            match insn.sib {
                Some(s) => {
                    used |= 1 << ((s & 7) | (rex & 1) << 3);
                    used |= 1 << (((s >> 3) & 7) | (rex & 2) << 2);
                }
                None => used |= 1 << ((m & 7) | (rex & 1) << 3),
            }
        }
    }
    // cmpxchg8b/16b use RBX implicitly.
    if insn.map == Map::Map0F && insn.opcode == 0xc7 {
        used |= 1 << RBX;
    }
    let candidates: &[Reg] = if rex == 0 {
        &[RSI, RDI, RBP, RBX]
    } else {
        &[RSI, RDI, RBP, RBX, R8, R9, R10, R11, R12, R13, R14, R15]
    };
    *candidates
        .iter()
        .find(|&&r| used & (1 << r) == 0)
        .expect("an instruction names at most three registers")
}
