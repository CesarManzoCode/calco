//! Self-modifying code (contract 3.13): which guest pages hold translated code, and which
//! of them calco keeps without write permission in the host.
//!
//! Every block is noted in the pages its guest bytes touch (one, or two when its last
//! instruction crosses a page). A page the guest may write is write-protected in the host
//! the first time a block is translated from it. A guest write there faults; the signal
//! handler (or the dispatcher, if the cache lock is busy) asks `write_fault`, which drops
//! the page with all its blocks and says which host protection gives the write back. The
//! same goes for a guest range that `munmap`, `mprotect` without execute, `mremap`, a fixed
//! `mmap` or `madvise(MADV_DONTNEED)` touches (`invalidate`).
//!
//! This is bookkeeping only: `jit` applies the protections it returns and unlinks the
//! blocks. Everything here runs under the cache lock.

use std::collections::BTreeMap;
use std::ops::Range;

use crate::mem::{PAGE_SIZE, USER_END, host_prot, page_down, page_up};
use crate::sys::{PROT_EXEC, PROT_WRITE};

/// A page with translations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page {
    /// Logical protection the guest asked for.
    pub prot: u32,
    /// Write is taken away in the host.
    pub protected: bool,
    /// Blocks with guest bytes in the page.
    pub blocks: Vec<u32>,
}

/// What an invalidation leaves to do: the blocks to unlink and take out of the lookup
/// table, and the host protection to give back to each page that was write-protected.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Invalidation {
    pub blocks: Vec<u32>,
    pub restore: Vec<(u64, u32)>,
}

impl Invalidation {
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty() && self.restore.is_empty()
    }
}

/// Pages that hold the bytes `[start, end)`; an empty range touches none.
pub fn pages(start: u64, end: u64) -> impl Iterator<Item = u64> {
    let first = page_down(start);
    let last = if end > start { page_up(end) } else { first };
    (first..last).step_by(PAGE_SIZE as usize)
}

/// Pages of a block whose guest bytes are `[rip, guest_end)`. A block always holds at
/// least the byte at `rip`.
pub fn block_pages(rip: u64, guest_end: u64) -> impl Iterator<Item = u64> {
    pages(rip, guest_end.max(rip + 1))
}

/// Page-aligned range that a memory call on `[addr, addr + len)` touches, as the kernel
/// rounds it; `None` if empty or out of the user half.
pub fn span(addr: u64, len: u64) -> Option<Range<u64>> {
    if len == 0 || addr >= USER_END {
        return None;
    }
    let start = page_down(addr);
    let end = page_up(addr.saturating_add(len).min(USER_END));
    (start < end).then_some(start..end)
}

/// Host protection of a write-protected page whose logical protection is `prot`.
pub fn protected_prot(prot: u32) -> u32 {
    host_prot(prot) & !PROT_WRITE
}

/// Whether a page with logical protection `prot` and translations needs write taken away.
fn needs_protection(prot: u32) -> bool {
    prot & PROT_EXEC != 0 && prot & PROT_WRITE != 0
}

#[derive(Default)]
pub struct Smc {
    pages: BTreeMap<u64, Page>,
}

impl Smc {
    pub fn new() -> Smc {
        Smc::default()
    }

    pub fn page(&self, page: u64) -> Option<&Page> {
        self.pages.get(&page_down(page))
    }

    pub fn len(&self) -> usize {
        self.pages.len()
    }

    /// Notes block `block`, with guest bytes `[rip, guest_end)`, in its pages; `prot_of`
    /// gives the logical protection of a page. Returns the pages to write-protect now and
    /// the host protection for each.
    pub fn track(
        &mut self,
        block: u32,
        rip: u64,
        guest_end: u64,
        prot_of: impl Fn(u64) -> u32,
    ) -> Vec<(u64, u32)> {
        let mut protect = Vec::new();
        for page in block_pages(rip, guest_end) {
            let entry = self.pages.entry(page).or_insert_with(|| Page {
                prot: prot_of(page),
                protected: false,
                blocks: Vec::new(),
            });
            entry.blocks.push(block);
            if !entry.protected && needs_protection(entry.prot) {
                entry.protected = true;
                protect.push((page, protected_prot(entry.prot)));
            }
        }
        protect
    }

    /// A guest write to `addr` faulted. If it is a page calco write-protected and the
    /// guest may write, the page goes with its blocks; `None` means the fault is the
    /// guest's own.
    pub fn write_fault(&mut self, addr: u64) -> Option<Invalidation> {
        let page = page_down(addr);
        let p = self.pages.get(&page)?;
        if !p.protected || p.prot & PROT_WRITE == 0 {
            return None;
        }
        Some(self.invalidate(page..page + PAGE_SIZE))
    }

    /// Drops every page in `range` with its blocks.
    pub fn invalidate(&mut self, range: Range<u64>) -> Invalidation {
        let gone: Vec<u64> = self.pages.range(range).map(|(&page, _)| page).collect();
        let mut out = Invalidation::default();
        for page in gone {
            let p = self.pages.remove(&page).expect("listed page");
            if p.protected {
                out.restore.push((page, host_prot(p.prot)));
            }
            for b in p.blocks {
                if !out.blocks.contains(&b) {
                    out.blocks.push(b);
                }
            }
        }
        out
    }

    /// The guest set `range` to `prot`, still executable, and the host now has
    /// `host_prot(prot)` there: the pages with translations that the guest may write are
    /// write-protected again. Returns those pages with their host protection.
    pub fn reprotect(&mut self, range: Range<u64>, prot: u32) -> Vec<(u64, u32)> {
        let mut protect = Vec::new();
        for (&page, p) in self.pages.range_mut(range) {
            p.prot = prot;
            p.protected = needs_protection(prot);
            if p.protected {
                protect.push((page, protected_prot(prot)));
            }
        }
        protect
    }

    /// Forgets every page (the cache was emptied); returns the host protection to give
    /// back to the write-protected ones.
    pub fn clear(&mut self) -> Vec<(u64, u32)> {
        let restore = self
            .pages
            .iter()
            .filter(|(_, p)| p.protected)
            .map(|(&page, p)| (page, host_prot(p.prot)))
            .collect();
        self.pages.clear();
        restore
    }
}

#[cfg(test)]
mod tests;
