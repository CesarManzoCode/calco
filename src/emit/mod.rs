//! Host x86-64 encoder (contract 3.5).
//!
//! `Asm::recode` re-encodes a decoded guest instruction from its fields (never copying the
//! guest bytes): same prefixes, REX, opcode, operands and immediate, optionally with a new
//! memory operand. The other methods emit the forms the translator and the trampolines
//! use; all of them except `pushfq` and `popfq` leave RFLAGS untouched.

use crate::decode::{Insn, MAX_LEN, Map};

/// Host general register, in encoding order.
pub type Reg = u8;

pub const RAX: Reg = 0;
pub const RCX: Reg = 1;
pub const RDX: Reg = 2;
pub const RBX: Reg = 3;
pub const RSP: Reg = 4;
pub const RBP: Reg = 5;
pub const RSI: Reg = 6;
pub const RDI: Reg = 7;
pub const R8: Reg = 8;
pub const R9: Reg = 9;
pub const R10: Reg = 10;
pub const R11: Reg = 11;
pub const R12: Reg = 12;
pub const R13: Reg = 13;
pub const R14: Reg = 14;
pub const R15: Reg = 15;

pub const REX_W: u8 = 8;
pub const REX_R: u8 = 4;
pub const REX_X: u8 = 2;
pub const REX_B: u8 = 1;

pub const PREFIX_FS: u8 = 0x64;
pub const PREFIX_GS: u8 = 0x65;
pub const PREFIX_ADDR32: u8 = 0x67;

const NOP: u8 = 0x90;
const INT3: u8 = 0xcc;

/// Memory operand `[base + index * 2^scale + disp]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mem {
    pub base: Option<Reg>,
    pub index: Option<Reg>,
    /// log2 of the index scale.
    pub scale: u8,
    pub disp: i32,
}

impl Mem {
    pub const fn base(base: Reg, disp: i32) -> Mem {
        Mem {
            base: Some(base),
            index: None,
            scale: 0,
            disp,
        }
    }

    pub const fn base_index(base: Reg, index: Reg) -> Mem {
        Mem {
            base: Some(base),
            index: Some(index),
            scale: 0,
            disp: 0,
        }
    }

    /// `[disp32]` in the SIB form without base or index, never RIP-relative. With `gs:` it
    /// addresses a field of the `Cpu`.
    pub const fn abs(disp: i32) -> Mem {
        Mem {
            base: None,
            index: None,
            scale: 0,
            disp,
        }
    }

    fn rex_xb(&self) -> u8 {
        let x = self.index.map_or(0, |i| (i >> 3) & 1);
        let b = self.base.map_or(0, |b| (b >> 3) & 1);
        x * REX_X | b * REX_B
    }
}

/// How `recode` writes the memory operand of an instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemRewrite {
    /// ModRM, SIB and displacement (or the `moffs` address) as decoded.
    Keep,
    /// RIP-relative to this absolute address, with the displacement computed for the place
    /// where the instruction is emitted.
    RipTarget(u64),
    /// `[reg]`, with `reg` already holding the linear address. The `gs` and address-size
    /// prefixes go away, since their effect is already in `reg`; `mov` with `moffs`
    /// (`A0`–`A3`) becomes its ModRM form (`8A 8B 88 89`).
    Base(Reg),
}

/// A RIP-relative operand whose target is more than 2 GiB away from the host code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutOfRange;

/// Displacement from `end` to `target`, if it fits in 32 bits.
pub fn rel32(end: u64, target: u64) -> Option<i32> {
    i32::try_from(target.wrapping_sub(end) as i64).ok()
}

/// `mov` between the accumulator and an absolute address (`A0`–`A3`).
pub fn is_moffs(insn: &Insn) -> bool {
    insn.map == Map::OneByte && matches!(insn.opcode, 0xa0..=0xa3) && insn.modrm.is_none()
}

fn escape(map: Map) -> &'static [u8] {
    match map {
        Map::OneByte => &[],
        Map::Map0F => &[0x0f],
        Map::Map0F38 => &[0x0f, 0x38],
        Map::Map0F3A => &[0x0f, 0x3a],
    }
}

/// Code being emitted for the host address `base`.
pub struct Asm {
    base: u64,
    buf: Vec<u8>,
}

impl Asm {
    pub fn new(base: u64) -> Asm {
        Asm {
            base,
            buf: Vec::with_capacity(512),
        }
    }

    /// Host address of the next byte.
    pub fn pc(&self) -> u64 {
        self.base + self.buf.len() as u64
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn bytes(&self) -> &[u8] {
        &self.buf
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    pub fn byte(&mut self, b: u8) {
        self.buf.push(b);
    }

    /// Raw little-endian data (the address after the exit thunk's `jmp [rip]`).
    pub fn data_u64(&mut self, v: u64) {
        self.le(v, 8);
    }

    fn le(&mut self, v: u64, n: usize) {
        for k in 0..n {
            self.buf.push((v >> (8 * k)) as u8);
        }
    }

    fn rex(&mut self, bits: u8) {
        if bits != 0 {
            self.byte(0x40 | bits);
        }
    }

    /// ModRM, SIB and displacement for `reg` and the memory operand `m`.
    fn modrm_mem(&mut self, reg: u8, m: &Mem) {
        let reg = (reg & 7) << 3;
        let index = match m.index {
            Some(i) => {
                assert!(i != RSP, "rsp cannot be an index");
                i & 7
            }
            None => 4,
        };
        let Some(base) = m.base else {
            self.byte(reg | 4);
            self.byte((m.scale << 6) | (index << 3) | 5);
            self.le(m.disp as u32 as u64, 4);
            return;
        };
        // [rbp] and [r13] have no form without displacement.
        let (md, size) = if m.disp == 0 && base & 7 != RBP {
            (0, 0)
        } else if i8::try_from(m.disp).is_ok() {
            (1, 1)
        } else {
            (2, 4)
        };
        // [rsp] and [r12] always need a SIB.
        if m.index.is_none() && base & 7 != RSP {
            self.byte((md << 6) | reg | (base & 7));
        } else {
            self.byte((md << 6) | reg | 4);
            self.byte((m.scale << 6) | (index << 3) | (base & 7));
        }
        self.le(m.disp as u32 as u64, size);
    }

    fn op_mem(&mut self, w: bool, esc: &[u8], opcode: u8, reg: u8, m: &Mem) {
        let w = if w { REX_W } else { 0 };
        self.rex(w | ((reg >> 3) & 1) * REX_R | m.rex_xb());
        self.buf.extend_from_slice(esc);
        self.byte(opcode);
        self.modrm_mem(reg, m);
    }

    fn op_reg(&mut self, w: bool, esc: &[u8], opcode: u8, reg: u8, rm: Reg) {
        let w = if w { REX_W } else { 0 };
        self.rex(w | ((reg >> 3) & 1) * REX_R | ((rm >> 3) & 1) * REX_B);
        self.buf.extend_from_slice(esc);
        self.byte(opcode);
        self.byte(0xc0 | ((reg & 7) << 3) | (rm & 7));
    }

    /// `mov qword [m], src`.
    pub fn mov_store(&mut self, m: &Mem, src: Reg) {
        self.op_mem(true, &[], 0x89, src, m);
    }

    /// `mov dst, qword [m]`.
    pub fn mov_load(&mut self, dst: Reg, m: &Mem) {
        self.op_mem(true, &[], 0x8b, dst, m);
    }

    /// `mov qword [m], imm32` (sign-extended).
    pub fn mov_store_imm(&mut self, m: &Mem, imm: i32) {
        self.op_mem(true, &[], 0xc7, 0, m);
        self.le(imm as u32 as u64, 4);
    }

    /// `mov dword [m], imm32`.
    pub fn mov_store_dword(&mut self, m: &Mem, imm: u32) {
        self.op_mem(false, &[], 0xc7, 0, m);
        self.le(imm as u64, 4);
    }

    /// `mov dst, src`, 64 bits.
    pub fn mov_rr(&mut self, dst: Reg, src: Reg) {
        self.op_reg(true, &[], 0x89, src, dst);
    }

    /// `lea dst, [m]`, 64 bits.
    pub fn lea(&mut self, dst: Reg, m: &Mem) {
        self.op_mem(true, &[], 0x8d, dst, m);
    }

    /// `mov qword gs:[off], src`.
    pub fn mov_gs_store(&mut self, off: usize, src: Reg) {
        self.byte(PREFIX_GS);
        self.mov_store(&Mem::abs(off as i32), src);
    }

    /// `mov dst, qword gs:[off]`.
    pub fn mov_gs_load(&mut self, dst: Reg, off: usize) {
        self.byte(PREFIX_GS);
        self.mov_load(dst, &Mem::abs(off as i32));
    }

    /// `lea dst32, [m]`: the low 32 bits of the address, zero-extended.
    pub fn lea32(&mut self, dst: Reg, m: &Mem) {
        self.op_mem(false, &[], 0x8d, dst, m);
    }

    /// `mov dst32, dword gs:[off]`, zero-extended.
    pub fn mov_gs_load32(&mut self, dst: Reg, off: usize) {
        self.byte(PREFIX_GS);
        self.op_mem(false, &[], 0x8b, dst, &Mem::abs(off as i32));
    }

    /// `movzx dst32, word gs:[off]`.
    pub fn movzx_gs_word(&mut self, dst: Reg, off: usize) {
        self.byte(PREFIX_GS);
        self.op_mem(false, &[0x0f], 0xb7, dst, &Mem::abs(off as i32));
    }

    /// `jmp qword gs:[off]`.
    pub fn jmp_gs(&mut self, off: usize) {
        self.byte(PREFIX_GS);
        self.op_mem(false, &[], 0xff, 4, &Mem::abs(off as i32));
    }

    /// `mov dword gs:[off], imm32`.
    pub fn mov_gs_dword(&mut self, off: usize, imm: u32) {
        self.byte(PREFIX_GS);
        self.mov_store_dword(&Mem::abs(off as i32), imm);
    }

    /// `pop qword gs:[off]`.
    pub fn pop_gs(&mut self, off: usize) {
        self.byte(PREFIX_GS);
        self.op_mem(false, &[], 0x8f, 0, &Mem::abs(off as i32));
    }

    /// `mov dst, imm64`.
    pub fn mov_imm64(&mut self, dst: Reg, imm: u64) {
        self.rex(REX_W | ((dst >> 3) & 1) * REX_B);
        self.byte(0xb8 | (dst & 7));
        self.le(imm, 8);
    }

    /// `xchg a, b`, 64 bits.
    pub fn xchg(&mut self, a: Reg, b: Reg) {
        self.op_reg(true, &[], 0x87, b, a);
    }

    pub fn pushfq(&mut self) {
        self.byte(0x9c);
    }

    pub fn popfq(&mut self) {
        self.byte(0x9d);
    }

    /// `fxsave64 [m]`.
    pub fn fxsave64(&mut self, m: &Mem) {
        self.op_mem(true, &[0x0f], 0xae, 0, m);
    }

    /// `fxrstor64 [m]`.
    pub fn fxrstor64(&mut self, m: &Mem) {
        self.op_mem(true, &[0x0f], 0xae, 1, m);
    }

    /// `rdfsbase r`, 64 bits.
    pub fn rdfsbase(&mut self, r: Reg) {
        self.byte(0xf3);
        self.op_reg(true, &[0x0f], 0xae, 0, r);
    }

    /// `wrfsbase r`, 64 bits.
    pub fn wrfsbase(&mut self, r: Reg) {
        self.byte(0xf3);
        self.op_reg(true, &[0x0f], 0xae, 2, r);
    }

    /// `jmp rel32` to `target`; returns the offset of the rel32 field.
    pub fn jmp_rel32(&mut self, target: u64) -> usize {
        let at = self.jmp_rel32_fwd();
        self.set_rel32(at, target);
        at
    }

    /// `jmp rel32` whose target is set later with `set_rel32`.
    pub fn jmp_rel32_fwd(&mut self) -> usize {
        self.byte(0xe9);
        let at = self.len();
        self.le(0, 4);
        at
    }

    /// `jcc rel32` (`0F 80+cc`) to `target`; returns the offset of the rel32 field.
    pub fn jcc_rel32(&mut self, cc: u8, target: u64) -> usize {
        let at = self.jcc_rel32_fwd(cc);
        self.set_rel32(at, target);
        at
    }

    /// `jcc rel32` whose target is set later with `set_rel32`.
    pub fn jcc_rel32_fwd(&mut self, cc: u8) -> usize {
        self.byte(0x0f);
        self.byte(0x80 | (cc & 15));
        let at = self.len();
        self.le(0, 4);
        at
    }

    /// Points the rel32 field at offset `at` to the host address `target`.
    pub fn set_rel32(&mut self, at: usize, target: u64) {
        let end = self.base + at as u64 + 4;
        let rel = rel32(end, target).expect("rel32 target out of range");
        self.buf[at..at + 4].copy_from_slice(&rel.to_le_bytes());
    }

    /// `jrcxz rel8`.
    pub fn jrcxz(&mut self, rel: i8) {
        self.byte(0xe3);
        self.byte(rel as u8);
    }

    /// `jmp rel8`.
    pub fn jmp_rel8(&mut self, rel: i8) {
        self.byte(0xeb);
        self.byte(rel as u8);
    }

    /// `jmp qword [rip + disp]`.
    pub fn jmp_rip_mem(&mut self, disp: i32) {
        self.byte(0xff);
        self.byte(0x25);
        self.le(disp as u32 as u64, 4);
    }

    /// Re-encodes a `loop*` or `jrcxz` (prefixes, REX and opcode) with a rel8 to be set
    /// with `set_rel8`; returns the offset of the rel8 field.
    pub fn recode_rel8(&mut self, insn: &Insn) -> usize {
        self.buf.extend_from_slice(insn.prefixes());
        if insn.rex != 0 {
            self.byte(insn.rex);
        }
        self.byte(insn.opcode);
        let at = self.len();
        self.byte(0);
        at
    }

    /// Points the rel8 field at offset `at` to offset `target` of this code.
    pub fn set_rel8(&mut self, at: usize, target: usize) {
        let rel = target as i64 - (at as i64 + 1);
        self.buf[at] = i8::try_from(rel).expect("rel8 target out of range") as u8;
    }

    /// Pads with `nop` until a field `field` bytes after the current position is aligned
    /// to `align` in the host, so it can be patched with one atomic store.
    pub fn pad_for_field(&mut self, field: usize, align: u64) {
        while (self.pc() + field as u64) % align != 0 {
            self.byte(NOP);
        }
    }

    /// Pads with `int3` (never executed) to an `align` boundary.
    pub fn align(&mut self, align: u64) {
        while self.pc() % align != 0 {
            self.byte(INT3);
        }
    }

    /// Re-encodes `insn` from its fields, with the memory operand written as `rw` says.
    /// Fails, emitting nothing, only for `RipTarget` out of rel32 range.
    pub fn recode(&mut self, insn: &Insn, rw: MemRewrite) -> Result<(), OutOfRange> {
        // Injected faults of the harness (contract 4.6), off by default.
        #[cfg(feature = "mutant-fadd-fsub")]
        let insn = &Insn {
            modrm: insn.modrm.map(|m| {
                m | 0x20
                    * (insn.map == Map::OneByte && insn.opcode & 0xfb == 0xd8 && m & 0x38 == 0)
                        as u8
            }),
            ..*insn
        };
        let moffs = is_moffs(insn);
        let base_rw = matches!(rw, MemRewrite::Base(_));
        let mut prefixes = [0u8; MAX_LEN];
        let mut np = 0;
        for &p in insn.prefixes() {
            if base_rw && (p == PREFIX_GS || p == PREFIX_ADDR32) {
                continue;
            }
            prefixes[np] = p;
            np += 1;
        }
        let esc = escape(insn.map);
        let (rex, opcode) = match rw {
            MemRewrite::Base(r) => {
                // REX.X and REX.B described the old operand; REX.R is meaningless for moffs.
                let keep = if moffs { REX_W } else { REX_W | REX_R };
                let bits = (insn.rex & keep) | ((r >> 3) & 1) * REX_B;
                // A REX the guest had stays, even empty: it selects SPL..DIL over AH..BH.
                let rex = if insn.rex != 0 || bits != 0 {
                    0x40 | bits
                } else {
                    0
                };
                let opcode = if moffs {
                    [0x8a, 0x8b, 0x88, 0x89][(insn.opcode - 0xa0) as usize]
                } else {
                    insn.opcode
                };
                (rex, opcode)
            }
            _ => (insn.rex, insn.opcode),
        };
        #[cfg(feature = "mutant-adc-carry")]
        let opcode = opcode - 0x10 * (insn.map == Map::OneByte && opcode & 0xf8 == 0x10) as u8;
        #[cfg(feature = "mutant-movsxd")]
        let (rex, opcode) = if insn.map == Map::OneByte && opcode == 0x63 {
            (rex & !REX_W, 0x8b)
        } else {
            (rex, opcode)
        };
        let rip_disp = match rw {
            MemRewrite::RipTarget(target) => {
                let len = np + (rex != 0) as usize + esc.len() + 2 + 4 + insn.imm_size as usize;
                Some(rel32(self.pc() + len as u64, target).ok_or(OutOfRange)?)
            }
            _ => None,
        };

        self.buf.extend_from_slice(&prefixes[..np]);
        if rex != 0 {
            self.byte(rex);
        }
        self.buf.extend_from_slice(esc);
        self.byte(opcode);
        match rw {
            MemRewrite::Keep => {
                if let Some(m) = insn.modrm {
                    self.byte(m);
                }
                if let Some(s) = insn.sib {
                    self.byte(s);
                }
                self.le(insn.disp as u64, insn.disp_size as usize);
            }
            MemRewrite::RipTarget(_) => {
                let m = insn.modrm.expect("RIP-relative operand without ModRM");
                self.byte((m & 0x38) | 0x05);
                self.le(rip_disp.unwrap_or(0) as u32 as u64, 4);
            }
            MemRewrite::Base(r) => {
                let reg = match insn.modrm {
                    Some(m) if !moffs => (m >> 3) & 7,
                    _ => 0,
                };
                self.modrm_mem(reg, &Mem::base(r, 0));
            }
        }
        self.le(insn.imm, insn.imm_size as usize);
        #[cfg(feature = "mutant-pshufd-imm")]
        if insn.map == Map::Map0F && insn.opcode == 0x70 && insn.mandatory == 0x66 {
            *self.buf.last_mut().unwrap() ^= 1;
        }
        #[cfg(feature = "mutant-shift-flags")]
        if insn.map == Map::OneByte
            && insn.opcode | 0x13 == 0xd3
            && insn.modrm.is_some_and(|m| m & 0x38 >= 0x20)
        {
            self.byte(0xf5);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
