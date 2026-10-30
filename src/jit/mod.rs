//! Code cache, chaining and dispatcher (contract 3.5–3.8, 3.12).
//!
//! The cache is a 256 MiB arena at the address `mem.rs` reserved (contract 3.1). It starts
//! with the exit thunk and the shared indirect exit; blocks follow, 16-byte aligned, until
//! it fills up and is emptied whole. Chainable exits are patched in place to jump to the
//! translation of their target, and the cache keeps, for each block, the exits that jump
//! into it.
//!
//! Every guest thread runs `run` and shares the cache (contract 3.12): translating,
//! patching and invalidating happen under the `Mutex<Jit>` of `thread::Shared`. The lookup
//! table and the block list are published with atomic stores (`Index`) and read without
//! the lock, by the dispatcher and by the signal handler. A thread only uses what it read
//! there while `thread::enter_cache` marks it in the cache; emptying the cache waits for
//! every other thread to be out of it (`thread::safe_point`).

pub mod trampoline;
pub mod translate;

use core::ops::Range;
use core::ptr;
use core::sync::atomic::{AtomicPtr, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::alloc::{Layout, alloc_zeroed, handle_alloc_error};
use std::sync::Arc;

use crate::cpu::{Cpu, OFF_EXIT_REASON, RAX, RDI};
use crate::cpuid;
use crate::emit::{self, Asm};
use crate::mem::{CODE_CACHE_SIZE, Mem};
use crate::signal;
use crate::sys::{self, EINVAL, PROT_EXEC, PROT_READ, PROT_WRITE};
use crate::syscall;
use crate::thread::{self, Member};

use self::translate::{
    EXIT_BRANCH, EXIT_CPUID, EXIT_INDIRECT, EXIT_INTERRUPTED, EXIT_SIGNAL, EXIT_SYSCALL, Env,
    Fault, GuestCode, PcEntry, Translation,
};

/// Space kept free after the start area: the exit thunk and the indirect exit.
const START_AREA: u64 = 64;
const BLOCK_ALIGN: u64 = 16;
/// Buckets of the lookup table, a power of two. The table never grows, so readers never
/// see it move: the cache is emptied before half of the buckets are in use.
const MAP_SLOTS: usize = 1 << 20;
/// Most blocks of a generation. The block list is allocated once with this capacity and
/// never reallocates.
const MAX_BLOCKS: usize = MAP_SLOTS / 2;

/// A translated block in the cache. Never changed once published.
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

#[repr(C)]
struct Bucket {
    rip: AtomicU64,
    block: AtomicU32,
}

/// Open-addressing hash table `guest rip → block index`, of fixed size, read without the
/// cache lock. Rip 0 marks a free bucket: page 0 is never guest code (`mmap_min_addr`). The
/// writer (with the lock) stores the block before the rip, the rip with release; a reader
/// that sees the rip with acquire sees the block. Entries only go all at once, in `clear`.
pub struct BlockMap {
    buckets: Box<[Bucket]>,
    len: AtomicUsize,
}

impl BlockMap {
    pub fn new() -> BlockMap {
        let layout = Layout::array::<Bucket>(MAP_SLOTS).expect("lookup table size");
        // All-zero buckets are free ones; zeroed pages come lazily from the kernel.
        let buckets = unsafe {
            let p = alloc_zeroed(layout) as *mut Bucket;
            if p.is_null() {
                handle_alloc_error(layout);
            }
            Box::from_raw(ptr::slice_from_raw_parts_mut(p, MAP_SLOTS))
        };
        BlockMap {
            buckets,
            len: AtomicUsize::new(0),
        }
    }

    fn slot(&self, rip: u64) -> usize {
        let bits = MAP_SLOTS.trailing_zeros();
        (rip.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> (64 - bits)) as usize
    }

    pub fn len(&self) -> usize {
        self.len.load(Ordering::Relaxed)
    }

    pub fn get(&self, rip: u64) -> Option<u32> {
        let mut i = self.slot(rip);
        loop {
            let b = &self.buckets[i];
            match b.rip.load(Ordering::Acquire) {
                0 => return None,
                r if r == rip => return Some(b.block.load(Ordering::Relaxed)),
                _ => i = (i + 1) & (MAP_SLOTS - 1),
            }
        }
    }

    /// Only with the cache lock held and fewer than half of the buckets in use.
    pub fn insert(&self, rip: u64, block: u32) {
        let mut i = self.slot(rip);
        loop {
            let b = &self.buckets[i];
            match b.rip.load(Ordering::Relaxed) {
                0 => {
                    b.block.store(block, Ordering::Relaxed);
                    b.rip.store(rip, Ordering::Release);
                    self.len.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                r if r == rip => {
                    b.block.store(block, Ordering::Release);
                    return;
                }
                _ => i = (i + 1) & (MAP_SLOTS - 1),
            }
        }
    }

    /// Only at a safe point, when no thread reads the table. Buckets that are already free
    /// are not written, so pages never used stay untouched.
    pub fn clear(&self) {
        for b in self.buckets.iter() {
            if b.rip.load(Ordering::Relaxed) != 0 {
                b.rip.store(0, Ordering::Relaxed);
                b.block.store(0, Ordering::Relaxed);
            }
        }
        self.len.store(0, Ordering::Relaxed);
    }
}

/// The part of the cache that threads read without its lock: the lookup table, the blocks
/// of the current generation and that generation. What it returns is valid while the
/// reading thread is in the cache (`thread::enter_cache`): no flush ends meanwhile.
pub struct Index {
    start: u64,
    end: u64,
    map: BlockMap,
    /// Base of the block list of `Jit`, allocated once (capacity `MAX_BLOCKS`).
    blocks: AtomicPtr<Block>,
    /// Blocks `[0, published)` are complete.
    published: AtomicUsize,
    generation: AtomicU64,
}

impl Index {
    /// The block at `rip` and its host code, if translated in this generation.
    pub fn lookup(&self, rip: u64) -> Option<(u32, u64)> {
        let id = self.map.get(rip)?;
        Some((id, self.blocks()[id as usize].host))
    }

    /// Counts flushes; an exit id is only valid in the generation that made it.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    fn blocks(&self) -> &[Block] {
        let len = self.published.load(Ordering::Acquire);
        // `Jit` never reallocates the list and only writes past `published` until a flush.
        unsafe { core::slice::from_raw_parts(self.blocks.load(Ordering::Relaxed), len) }
    }

    fn publish(&self, blocks: &mut Vec<Block>) {
        self.blocks.store(blocks.as_mut_ptr(), Ordering::Relaxed);
        self.published.store(blocks.len(), Ordering::Release);
    }

    /// The block whose host code holds `pc`. Blocks of a generation are laid out in the
    /// order of their ids; the start area belongs to no block. A `pc` outside the arena
    /// returns before the block list is read: the signal handler calls this from any pc,
    /// and only a thread in translated code knows no flush is emptying the list.
    pub fn block_at(&self, pc: u64) -> Option<(u32, &Block)> {
        if pc < self.start || pc >= self.end {
            return None;
        }
        let blocks = self.blocks();
        let n = blocks.partition_point(|b| b.host <= pc);
        let id = n.checked_sub(1)?;
        let b = &blocks[id];
        (pc < b.host + b.host_len as u64).then_some((id as u32, b))
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
    /// Published through `index`; capacity `MAX_BLOCKS`, so it never reallocates.
    blocks: Vec<Block>,
    /// Ids of the exits of any block that are chained to each block, by block id.
    incoming: Vec<Vec<u32>>,
    exits: Vec<Exit>,
    /// Counts flushes; an exit id is only valid in the generation that made it.
    generation: u64,
    index: &'static Index,
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
        asm.data_u64(trampoline::exit_trampoline as *const () as u64);
        asm.align(BLOCK_ALIGN);
        let indirect_exit = asm.pc();
        asm.mov_gs_dword(OFF_EXIT_REASON, EXIT_INDIRECT);
        asm.mov_gs_dword(OFF_EXIT_REASON + 4, 0);
        asm.jmp_rel32(exit_thunk);
        assert!(asm.len() as u64 <= START_AREA);
        let code = asm.into_bytes();
        unsafe { core::ptr::copy_nonoverlapping(code.as_ptr(), base as *mut u8, code.len()) };

        let mut blocks = Vec::with_capacity(MAX_BLOCKS);
        let index = Box::leak(Box::new(Index {
            start: base + START_AREA,
            end: base + CODE_CACHE_SIZE,
            map: BlockMap::new(),
            blocks: AtomicPtr::new(blocks.as_mut_ptr()),
            published: AtomicUsize::new(0),
            generation: AtomicU64::new(0),
        }));
        Ok(Jit {
            start: base + START_AREA,
            end: base + CODE_CACHE_SIZE,
            top: base + START_AREA,
            exit_thunk,
            indirect_exit,
            blocks,
            incoming: Vec::new(),
            exits: Vec::new(),
            generation: 0,
            index,
        })
    }

    pub fn index(&self) -> &'static Index {
        self.index
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn block(&self, id: u32) -> &Block {
        &self.blocks[id as usize]
    }

    pub fn lookup(&self, rip: u64) -> Option<u32> {
        self.index.map.get(rip)
    }

    /// Translates the block at `rip` and installs it, emptying the cache first if it does
    /// not fit in the arena or in the block list.
    pub fn translate(&mut self, rip: u64, mem: &Mem) -> Result<u32, Fault> {
        let mut t = self.translate_at_top(rip, mem)?;
        if self.top + t.code.len() as u64 > self.end || self.blocks.len() >= MAX_BLOCKS {
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

    /// Writes the code, then publishes the block, then its rip in the table: a thread that
    /// finds the rip finds the block and its code complete.
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
        assert!(self.blocks.len() < MAX_BLOCKS, "block list full");
        self.blocks.push(Block {
            rip: t.rip,
            guest_end: t.guest_end,
            host,
            host_len: t.code.len() as u32,
            insns: t.insns as u32,
            pcs: t.pcs,
            exits: first..self.exits.len() as u32,
        });
        self.incoming.push(Vec::new());
        self.index.publish(&mut self.blocks);
        self.index.map.insert(t.rip, id);
        id
    }

    /// Empties the whole cache (contract 3.7) once no other thread is in it
    /// (`thread::safe_point`). The caller holds the cache lock.
    pub fn flush(&mut self) {
        let index = self.index;
        thread::safe_point(|| {
            self.blocks.clear();
            self.incoming.clear();
            self.exits.clear();
            index.map.clear();
            index.publish(&mut self.blocks);
            self.top = self.start;
            self.generation += 1;
            index.generation.store(self.generation, Ordering::Release);
        });
    }

    /// Chains exit `exit` to `block`, whose rip is the exit's target.
    pub fn link(&mut self, exit: u32, block: u32) {
        let e = &mut self.exits[exit as usize];
        let b = &self.blocks[block as usize];
        if e.linked || e.target != b.rip {
            return;
        }
        patch_rel32(e.jump, b.host);
        e.linked = true;
        self.incoming[block as usize].push(exit);
    }

    /// Points every exit chained to `block` back at its stub (for invalidation).
    pub fn unlink_incoming(&mut self, block: u32) {
        for exit in std::mem::take(&mut self.incoming[block as usize]) {
            let e = &mut self.exits[exit as usize];
            patch_rel32(e.jump, e.stub);
            e.linked = false;
        }
    }

    /// Points every chained exit of `block` back at its stub, so the block leaves to the
    /// dispatcher at its end (signal delivery without TF, contract 3.11).
    pub fn unlink_outgoing(&mut self, block: u32) {
        let exits = self.blocks[block as usize].exits.clone();
        for exit in exits {
            let e = &mut self.exits[exit as usize];
            if e.linked {
                patch_rel32(e.jump, e.stub);
                e.linked = false;
            }
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

/// Runs the guest thread of `cpu` from `cpu.rip`: looks up or translates the block, enters
/// it and serves the reason it exited for, until the thread calls `exit`. Returns the code
/// it passed. `me` is the thread's entry in the thread list.
pub fn run(cpu: &mut Cpu, me: &Arc<Member>) -> u64 {
    let index = thread::shared().index;
    // Exit to chain to the next block, with the generation it belongs to.
    let mut chain: Option<(u32, u64)> = None;
    loop {
        if signal::wants_delivery(cpu) {
            let mut world = thread::shared().world();
            let world = &mut *world;
            world.state.signals.deliver(cpu, &world.mem);
        }
        thread::enter_cache(me);
        let host = match index.lookup(cpu.rip) {
            Some((_, host)) if chain.is_none() => host,
            _ => match locked_block(cpu, me, chain.take()) {
                Some(host) => host,
                None => continue,
            },
        };
        unsafe { trampoline::enter(cpu, host) };
        // Read while still in the cache: no flush ends before the thread leaves it.
        let generation = index.generation();
        thread::leave_cache(me);
        cpu.rip = cpu.next_rip;
        let reason = cpu.exit_reason;
        let kind = reason as u32;
        match kind & 0xff {
            EXIT_BRANCH => chain = Some(((reason >> 32) as u32, generation)),
            EXIT_INDIRECT => {}
            // The end of a guest thread is `thread.rs`'s business.
            EXIT_SYSCALL if cpu.gpr[RAX] == syscall::EXIT => return cpu.gpr[RDI],
            EXIT_SYSCALL => syscall::handle(cpu, me),
            EXIT_CPUID => cpuid::handle(cpu),
            EXIT_SIGNAL => signal::trap(cpu, (kind >> 8) & 0xff),
            EXIT_INTERRUPTED => {}
            _ => panic!("unknown exit reason {reason:#x}"),
        }
    }
}

/// The host code of the block at `cpu.rip`, under the cache lock: translated if no thread
/// did it meanwhile, and with the exit `chain` linked to it if it is of this generation.
/// `None` if the code cannot be fetched; the guest then gets the fault. The thread is in
/// the cache when this returns `Some`.
fn locked_block(cpu: &mut Cpu, me: &Member, chain: Option<(u32, u64)>) -> Option<u64> {
    let shared = thread::shared();
    // A thread waiting for the lock is out of the cache, so a flush does not wait for it.
    thread::leave_cache(me);
    let mut jit = shared.jit();
    // No flush runs while this thread holds the lock: entering does not park.
    thread::enter_cache(me);
    let block = match jit.lookup(cpu.rip) {
        Some(block) => block,
        None => {
            let world = shared.world();
            match jit.translate(cpu.rip, &world.mem) {
                Ok(block) => block,
                Err(f) => {
                    signal::exec_fault(cpu, &world.mem, f.signal as u32, f.addr);
                    thread::leave_cache(me);
                    return None;
                }
            }
        }
    };
    if let Some((exit, generation)) = chain
        && generation == jit.generation()
    {
        jit.link(exit, block);
    }
    Some(jit.block(block).host)
}

#[cfg(test)]
mod tests;
