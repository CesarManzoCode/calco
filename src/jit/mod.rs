//! Code cache, chaining and dispatcher (contract 3.5–3.8).
//!
//! The cache is a 256 MiB arena at the address `mem.rs` reserved (contract 3.1). It starts
//! with the exit thunk and the shared indirect exit; blocks follow, 16-byte aligned, until
//! it fills up and is emptied whole. Chainable exits are patched in place to jump to the
//! translation of their target, and each block keeps the exits that jump into it.

pub mod trampoline;
pub mod translate;

use core::ops::Range;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::cpu::{Cpu, OFF_EXIT_REASON};
use crate::cpuid;
use crate::emit::{self, Asm};
use crate::mem::{CODE_CACHE_SIZE, Mem};
use crate::sys::{self, EINVAL, PROT_EXEC, PROT_READ, PROT_WRITE};
use crate::syscall;

use self::translate::{
    EXIT_BRANCH, EXIT_CPUID, EXIT_INDIRECT, EXIT_SIGNAL, EXIT_SYSCALL, Env, Fault, GuestCode,
    PcEntry, Translation,
};

/// Space kept free after the start area: the exit thunk and the indirect exit.
const START_AREA: u64 = 64;
const BLOCK_ALIGN: u64 = 16;

/// A translated block in the cache.
pub struct Block {
    pub rip: u64,
    pub guest_end: u64,
    pub host: u64,
    pub host_len: u32,
    pub insns: u32,
    /// Pc table, offsets relative to `host`.
    pub pcs: Vec<PcEntry>,
    /// Ids of the block's own chainable exits.
    pub exits: Range<u32>,
    /// Ids of the exits of any block that are chained to this one.
    pub incoming: Vec<u32>,
}

/// A chainable exit of a block.
pub struct Exit {
    pub block: u32,
    /// Host address of the rel32 to patch (4-byte aligned).
    pub jump: u64,
    /// Host address of the stub the rel32 points to while unchained.
    pub stub: u64,
    pub target: u64,
    pub linked: bool,
}

/// Open-addressing hash table `guest rip → block index`. Rip 0 marks a free slot: page 0
/// is never guest code (`mmap_min_addr`).
struct BlockMap {
    slots: Vec<(u64, u32)>,
    len: usize,
}

impl BlockMap {
    const INITIAL: usize = 1 << 14;

    fn new() -> BlockMap {
        BlockMap {
            slots: vec![(0, 0); Self::INITIAL],
            len: 0,
        }
    }

    fn slot(&self, rip: u64) -> usize {
        let bits = self.slots.len().trailing_zeros();
        (rip.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> (64 - bits)) as usize
    }

    fn get(&self, rip: u64) -> Option<u32> {
        let mask = self.slots.len() - 1;
        let mut i = self.slot(rip);
        loop {
            match self.slots[i] {
                (0, _) => return None,
                (r, block) if r == rip => return Some(block),
                _ => i = (i + 1) & mask,
            }
        }
    }

    fn insert(&mut self, rip: u64, block: u32) {
        if 2 * (self.len + 1) > self.slots.len() {
            let old = std::mem::replace(&mut self.slots, vec![(0, 0); 2 * self.slots.len()]);
            self.len = 0;
            for (r, b) in old {
                if r != 0 {
                    self.insert(r, b);
                }
            }
        }
        let mask = self.slots.len() - 1;
        let mut i = self.slot(rip);
        while self.slots[i].0 != 0 && self.slots[i].0 != rip {
            i = (i + 1) & mask;
        }
        if self.slots[i].0 == 0 {
            self.len += 1;
        }
        self.slots[i] = (rip, block);
    }

    fn clear(&mut self) {
        self.slots.iter_mut().for_each(|s| *s = (0, 0));
        self.len = 0;
    }
}

impl GuestCode for Mem {
    fn fetch(&self, rip: u64, buf: &mut [u8]) -> usize {
        let mut n = 0;
        while n < buf.len() {
            let addr = rip + n as u64;
            let Some(region) = self.intervals.find(addr) else {
                break;
            };
            if region.prot & PROT_EXEC == 0 {
                break;
            }
            let take = (buf.len() - n).min((region.end - addr) as usize);
            // Guest pages with logical execute permission are readable in the host.
            unsafe {
                core::ptr::copy_nonoverlapping(addr as *const u8, buf[n..].as_mut_ptr(), take)
            };
            n += take;
        }
        n
    }
}

pub struct Jit {
    start: u64,
    end: u64,
    /// Next free byte of the arena.
    top: u64,
    exit_thunk: u64,
    indirect_exit: u64,
    blocks: Vec<Block>,
    exits: Vec<Exit>,
    map: BlockMap,
    /// Counts flushes; an exit id is only valid in the generation that made it.
    generation: u64,
}

fn align_up(addr: u64, align: u64) -> u64 {
    (addr + align - 1) & !(align - 1)
}

impl Jit {
    /// Opens the code cache reserved by `mem` and writes the start area.
    pub fn new(mem: &Mem) -> Result<Jit, i32> {
        let base = mem.code_cache();
        if base == 0 {
            return Err(EINVAL);
        }
        unsafe { sys::mprotect(base, CODE_CACHE_SIZE, PROT_READ | PROT_WRITE | PROT_EXEC) }?;

        let mut asm = Asm::new(base);
        // Exit thunk: the trampoline is in calco's text, too far for a rel32.
        let exit_thunk = asm.pc();
        asm.jmp_rip_mem(0);
        asm.data_u64(trampoline::exit_trampoline as usize as u64);
        asm.align(BLOCK_ALIGN);
        let indirect_exit = asm.pc();
        asm.mov_gs_dword(OFF_EXIT_REASON, EXIT_INDIRECT);
        asm.mov_gs_dword(OFF_EXIT_REASON + 4, 0);
        asm.jmp_rel32(exit_thunk);
        assert!(asm.len() as u64 <= START_AREA);
        let code = asm.into_bytes();
        unsafe { core::ptr::copy_nonoverlapping(code.as_ptr(), base as *mut u8, code.len()) };

        Ok(Jit {
            start: base + START_AREA,
            end: base + CODE_CACHE_SIZE,
            top: base + START_AREA,
            exit_thunk,
            indirect_exit,
            blocks: Vec::new(),
            exits: Vec::new(),
            map: BlockMap::new(),
            generation: 0,
        })
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn block(&self, id: u32) -> &Block {
        &self.blocks[id as usize]
    }

    pub fn lookup(&self, rip: u64) -> Option<u32> {
        self.map.get(rip)
    }

    /// Translates the block at `rip` and installs it, emptying the cache first if it does
    /// not fit.
    pub fn translate(&mut self, rip: u64, mem: &Mem) -> Result<u32, Fault> {
        let mut t = self.translate_at_top(rip, mem)?;
        if self.top + t.code.len() as u64 > self.end {
            self.flush();
            t = self.translate_at_top(rip, mem)?;
        }
        Ok(self.install(t))
    }

    fn translate_at_top(&self, rip: u64, mem: &Mem) -> Result<Translation, Fault> {
        let env = Env {
            host: self.top,
            exit_thunk: self.exit_thunk,
            indirect_exit: self.indirect_exit,
            first_exit_id: self.exits.len() as u32,
        };
        translate::translate(mem, rip, &env)
    }

    fn install(&mut self, t: Translation) -> u32 {
        let host = self.top;
        // The arena is calco's RWX memory and [host, host + len) is free.
        unsafe { core::ptr::copy_nonoverlapping(t.code.as_ptr(), host as *mut u8, t.code.len()) };
        self.top = align_up(host + t.code.len() as u64, BLOCK_ALIGN).min(self.end);
        let id = self.blocks.len() as u32;
        let first = self.exits.len() as u32;
        for e in &t.exits {
            self.exits.push(Exit {
                block: id,
                jump: host + e.jump as u64,
                stub: host + e.stub as u64,
                target: e.target,
                linked: false,
            });
        }
        self.blocks.push(Block {
            rip: t.rip,
            guest_end: t.guest_end,
            host,
            host_len: t.code.len() as u32,
            insns: t.insns as u32,
            pcs: t.pcs,
            exits: first..self.exits.len() as u32,
            incoming: Vec::new(),
        });
        self.map.insert(t.rip, id);
        id
    }

    /// Empties the whole cache (contract 3.7). Only valid with no thread inside it.
    pub fn flush(&mut self) {
        self.blocks.clear();
        self.exits.clear();
        self.map.clear();
        self.top = self.start;
        self.generation += 1;
    }

    /// Chains exit `exit` to `block`, whose rip is the exit's target.
    pub fn link(&mut self, exit: u32, block: u32) {
        let e = &mut self.exits[exit as usize];
        let b = &mut self.blocks[block as usize];
        if e.linked || e.target != b.rip {
            return;
        }
        patch_rel32(e.jump, b.host);
        e.linked = true;
        b.incoming.push(exit);
    }

    /// Points every exit chained to `block` back at its stub (for invalidation).
    pub fn unlink_incoming(&mut self, block: u32) {
        for exit in std::mem::take(&mut self.blocks[block as usize].incoming) {
            let e = &mut self.exits[exit as usize];
            patch_rel32(e.jump, e.stub);
            e.linked = false;
        }
    }
}

/// Points the rel32 at `site` to `target` with one aligned 4-byte store, so a thread
/// running the code sees either the old or the new jump.
fn patch_rel32(site: u64, target: u64) {
    assert!(site % 4 == 0, "unaligned rel32 at {site:#x}");
    let rel = emit::rel32(site + 4, target).expect("code cache jumps fit in rel32");
    // The site is inside the code cache, which calco owns and keeps mapped.
    let field = unsafe { AtomicU32::from_ptr(site as *mut u32) };
    field.store(rel as u32, Ordering::Release);
}

/// Runs the guest from `cpu.rip`: looks up or translates the block, enters it and serves
/// the reason it exited for, until the guest exits or dies.
pub fn run(cpu: &mut Cpu, mem: &mut Mem, jit: &mut Jit, state: &mut syscall::State) -> ! {
    // Translated code reaches the Cpu through GS; nothing else in calco uses GS.
    unsafe { sys::set_gs_base(cpu as *mut Cpu as u64) };
    // Exit to chain to the next block, with the generation it belongs to.
    let mut chain: Option<(u32, u64)> = None;
    loop {
        let block = match jit.lookup(cpu.rip) {
            Some(block) => block,
            None => jit
                .translate(cpu.rip, mem)
                .unwrap_or_else(|f| sys::die_by_signal(f.signal)),
        };
        if let Some((exit, generation)) = chain.take()
            && generation == jit.generation()
        {
            jit.link(exit, block);
        }
        let host = jit.block(block).host;
        unsafe { trampoline::enter(cpu, host) };
        cpu.rip = cpu.next_rip;
        let reason = cpu.exit_reason;
        let kind = reason as u32;
        match kind & 0xff {
            EXIT_BRANCH => chain = Some(((reason >> 32) as u32, jit.generation())),
            EXIT_INDIRECT => {}
            EXIT_SYSCALL => {
                syscall::handle(cpu, mem, state);
                // Guest code was unmapped or changed: no translation of it may survive.
                if state.take_code_changed() {
                    jit.flush();
                }
            }
            EXIT_CPUID => cpuid::handle(cpu),
            EXIT_SIGNAL => sys::die_by_signal((kind >> 8) as i32),
            _ => panic!("unknown exit reason {reason:#x}"),
        }
    }
}

#[cfg(test)]
mod tests;
