//! Instruction decoder (contract 3.4): turns guest bytes into an `Insn` with every field
//! the emitter needs to re-encode it (contract 3.5). It never executes guest code.

pub mod table;

pub use self::table::{Class, Ext, Map, Signal};
use self::table::{Entry, Imm, Modrm, Op};

/// Longest instruction the CPU accepts.
pub const MAX_LEN: usize = 15;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// Not in the scope, or not an instruction in 64-bit mode: SIGILL in the guest.
    Invalid,
    /// The bytes end before the instruction does (and before 15 bytes).
    Truncated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Insn {
    /// Legacy prefixes (groups 1–4) as they appear, in order. A REX byte followed by a
    /// legacy prefix has no effect and is dropped.
    pub prefix_bytes: [u8; MAX_LEN],
    pub prefix_count: u8,
    /// REX byte right before the opcode (or escape), 0 if none.
    pub rex: u8,
    pub map: Map,
    pub opcode: u8,
    pub modrm: Option<u8>,
    pub sib: Option<u8>,
    /// Sign-extended displacement; for `A0–A3`, the absolute address.
    pub disp: i64,
    pub disp_size: u8,
    /// Immediate as read, zero-extended. `enter` stores iw in bits 0–15 and ib in 16–23.
    pub imm: u64,
    pub imm_size: u8,
    pub len: u8,
    pub class: Class,
    pub ext: Ext,
    /// Mandatory prefix that selected the table variant (`66`, `F3`, `F2`), 0 if none.
    pub mandatory: u8,
    /// Destination of a relative jump or call.
    pub target: Option<u64>,
    /// RIP of the instruction.
    pub rip: u64,
    pub rip_relative: bool,
    pub seg_fs: bool,
    pub seg_gs: bool,
}

impl Insn {
    pub fn prefixes(&self) -> &[u8] {
        &self.prefix_bytes[..self.prefix_count as usize]
    }

    pub fn has_prefix(&self, b: u8) -> bool {
        self.prefixes().contains(&b)
    }

    pub fn rex_w(&self) -> bool {
        self.rex & 8 != 0
    }

    /// Immediate sign-extended from its size.
    pub fn imm_sext(&self) -> i64 {
        match self.imm_size {
            1 => self.imm as u8 as i8 as i64,
            2 => self.imm as u16 as i16 as i64,
            4 => self.imm as u32 as i32 as i64,
            _ => self.imm as i64,
        }
    }

    /// Effective address of a RIP-relative operand.
    pub fn rip_target(&self) -> Option<u64> {
        self.rip_relative.then(|| {
            self.rip
                .wrapping_add(self.len as u64)
                .wrapping_add(self.disp as u64)
        })
    }
}

fn is_legacy_prefix(b: u8) -> bool {
    matches!(
        b,
        0xf0 | 0xf2 | 0xf3 | 0x2e | 0x36 | 0x3e | 0x26 | 0x64 | 0x65 | 0x66 | 0x67
    )
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Cursor<'_> {
    fn next(&mut self) -> Result<u8, DecodeError> {
        if self.pos >= MAX_LEN {
            return Err(DecodeError::Invalid);
        }
        let b = *self.bytes.get(self.pos).ok_or(DecodeError::Truncated)?;
        self.pos += 1;
        Ok(b)
    }

    fn le(&mut self, n: u8) -> Result<u64, DecodeError> {
        let mut v = 0u64;
        for k in 0..n {
            v |= (self.next()? as u64) << (8 * k);
        }
        Ok(v)
    }
}

fn sext(v: u64, size: u8) -> i64 {
    match size {
        1 => v as u8 as i8 as i64,
        2 => v as u16 as i16 as i64,
        4 => v as u32 as i32 as i64,
        _ => v as i64,
    }
}

/// Decodes the instruction at the start of `bytes`, which sits at guest address `rip`.
pub fn decode(bytes: &[u8], rip: u64) -> Result<Insn, DecodeError> {
    let mut c = Cursor { bytes, pos: 0 };
    let mut prefix_bytes = [0u8; MAX_LEN];
    let mut prefix_count = 0usize;
    let mut rex = 0u8;
    let mut segment = 0u8;
    let mut rep = 0u8;
    let mut op_size = false;
    let mut addr_size = false;

    let first = loop {
        let b = c.next()?;
        if is_legacy_prefix(b) {
            rex = 0;
            prefix_bytes[prefix_count] = b;
            prefix_count += 1;
            match b {
                0x2e | 0x36 | 0x3e | 0x26 | 0x64 | 0x65 => segment = b,
                0xf2 | 0xf3 => rep = b,
                0x66 => op_size = true,
                0x67 => addr_size = true,
                _ => {}
            }
        } else if b & 0xf0 == 0x40 {
            rex = b;
        } else {
            break b;
        }
    };

    let (map, opcode) = if first == 0x0f {
        match c.next()? {
            0x38 => (Map::Map0F38, c.next()?),
            0x3a => (Map::Map0F3A, c.next()?),
            b => (Map::Map0F, b),
        }
    } else {
        (Map::OneByte, first)
    };
    let tab = match map {
        Map::OneByte => &table::ONE_BYTE,
        Map::Map0F => &table::MAP_0F,
        Map::Map0F38 => &table::MAP_0F38,
        Map::Map0F3A => &table::MAP_0F3A,
    };

    // With 66 and F2/F3 together, F2/F3 selects the variant.
    let (pidx, pbyte) = match (rep, op_size) {
        (0xf3, _) => (2, 0xf3),
        (0xf2, _) => (3, 0xf2),
        (_, true) => (1, 0x66),
        _ => (0, 0),
    };

    let mut modrm: Option<u8> = None;
    let mut mandatory = 0u8;
    let mut entry: &Entry = &tab[opcode as usize];
    let op: Op = loop {
        match entry {
            Entry::Op(o) => break *o,
            Entry::Prefixed(ops) => {
                mandatory = pbyte;
                break ops[pidx];
            }
            Entry::Group(g) => {
                let m = fetch_modrm(&mut c, &mut modrm)?;
                entry = &g[((m >> 3) & 7) as usize];
            }
            Entry::Split(mem, reg) => {
                let m = fetch_modrm(&mut c, &mut modrm)?;
                entry = if m >> 6 == 3 { *reg } else { *mem };
            }
            Entry::ByRm(ops) => {
                let m = fetch_modrm(&mut c, &mut modrm)?;
                break ops[(m & 7) as usize];
            }
        }
    };
    if op.class == Class::Invalid {
        return Err(DecodeError::Invalid);
    }
    // Near jumps, calls and returns with 66 are out of the scope (contract 3.4).
    if op_size
        && matches!(
            op.class,
            Class::Branch | Class::Call | Class::Ret | Class::IndirectBranch
        )
    {
        return Err(DecodeError::Invalid);
    }

    let mut sib = None;
    let mut disp = 0i64;
    let mut disp_size = 0u8;
    let mut rip_relative = false;
    if op.modrm != Modrm::No {
        let m = fetch_modrm(&mut c, &mut modrm)?;
        let md = m >> 6;
        match op.modrm {
            Modrm::Mem if md == 3 => return Err(DecodeError::Invalid),
            Modrm::Reg if md != 3 => return Err(DecodeError::Invalid),
            _ => {}
        }
        if md != 3 && op.modrm != Modrm::Control {
            let mut base = m & 7;
            if base == 4 {
                let s = c.next()?;
                sib = Some(s);
                base = s & 7;
            }
            disp_size = match md {
                1 => 1,
                2 => 4,
                _ if base == 5 => {
                    rip_relative = sib.is_none();
                    4
                }
                _ => 0,
            };
            disp = sext(c.le(disp_size)?, disp_size);
        }
    }

    let w = rex & 8 != 0;
    let mut imm = 0u64;
    let mut imm_size = 0u8;
    match op.imm {
        Imm::No => {}
        Imm::Moffs => {
            disp_size = if addr_size { 4 } else { 8 };
            disp = c.le(disp_size)? as i64;
        }
        Imm::IwIb => {
            imm = c.le(2)? | c.le(1)? << 16;
            imm_size = 3;
        }
        _ => {
            imm_size = match op.imm {
                Imm::Ib | Imm::Rel8 => 1,
                Imm::Iw => 2,
                Imm::Iz if w => 4,
                Imm::Iz if op_size => 2,
                Imm::Iv if w => 8,
                Imm::Iv if op_size => 2,
                _ => 4,
            };
            imm = c.le(imm_size)?;
        }
    }

    let len = c.pos as u8;
    let mut class = op.class;
    if map == Map::OneByte && opcode == 0xcd && imm == 3 {
        class = Class::Trap(Signal::Sigtrap);
    }
    let target = matches!(op.imm, Imm::Rel8 | Imm::Rel32).then(|| {
        rip.wrapping_add(len as u64)
            .wrapping_add(sext(imm, imm_size) as u64)
    });

    Ok(Insn {
        prefix_bytes,
        prefix_count: prefix_count as u8,
        rex,
        map,
        opcode,
        modrm,
        sib,
        disp,
        disp_size,
        imm,
        imm_size,
        len,
        class,
        ext: op.ext,
        mandatory,
        target,
        rip,
        rip_relative,
        seg_fs: segment == 0x64,
        seg_gs: segment == 0x65,
    })
}

fn fetch_modrm(c: &mut Cursor<'_>, modrm: &mut Option<u8>) -> Result<u8, DecodeError> {
    match *modrm {
        Some(m) => Ok(m),
        None => {
            let m = c.next()?;
            *modrm = Some(m);
            Ok(m)
        }
    }
}

#[cfg(test)]
mod tests;
