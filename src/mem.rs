//! Guest memory map: guest-owned intervals with logical permissions, placement window and `brk`.
//!
//! Guest addresses are host addresses (identity mapping). Every guest page is mapped in the
//! host without `PROT_EXEC`; the permissions the guest asked for are kept in `Intervals`.
//! Anything mapped in the host that is not in `Intervals` belongs to calco.

use std::collections::BTreeMap;

use crate::sys::{
    self, EEXIST, EFAULT, EINVAL, ENOMEM, MAP_ANONYMOUS, MAP_FIXED, MAP_FIXED_NOREPLACE,
    MAP_NORESERVE, MAP_POPULATE, MAP_PRIVATE, MAP_STACK, MAP_TYPE, MREMAP_FIXED, MREMAP_MAYMOVE,
    PROT_EXEC, PROT_NONE, PROT_READ, PROT_WRITE,
};

pub const PAGE_SIZE: u64 = 4096;
/// Load base of `ET_DYN` executables.
pub const DYN_BASE: u64 = 0x0000_1000_0000_0000;
/// End of the guest placement window.
pub const WINDOW_END: u64 = 0x0000_3fff_0000_0000;
/// Top of the main guest stack.
pub const STACK_TOP: u64 = 0x0000_4000_0000_0000;
pub const STACK_SIZE: u64 = 8 << 20;
pub const BRK_LIMIT: u64 = 1 << 30;
pub const CODE_CACHE_SIZE: u64 = 256 << 20;
const CODE_CACHE_ALIGN: u64 = 2 << 20;
/// Gap between the end of the code cache and the start of the placement window.
const WINDOW_GAP: u64 = 16 << 20;
/// First address past the user half of the address space.
pub const USER_END: u64 = 0x0000_8000_0000_0000;

const PROT_ALL: u32 = PROT_READ | PROT_WRITE | PROT_EXEC;
/// Guest `mmap` flags passed to the host; placement flags are decided here.
const PASS_FLAGS: u32 = MAP_TYPE | MAP_ANONYMOUS | MAP_NORESERVE | MAP_POPULATE | MAP_STACK;

pub fn page_down(addr: u64) -> u64 {
    addr & !(PAGE_SIZE - 1)
}

/// Rounds up to a page; callers keep `addr` below `USER_END`, so it cannot overflow.
pub fn page_up(addr: u64) -> u64 {
    (addr + PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
}

fn align_up(addr: u64, align: u64) -> u64 {
    (addr + align - 1) & !(align - 1)
}

/// Host protection for a logical guest protection: never executable. On x86 `PROT_EXEC`
/// implies read, and the translator reads code from the page, so it maps to `PROT_READ`.
pub fn host_prot(prot: u32) -> u32 {
    let mut host = prot & (PROT_READ | PROT_WRITE);
    if prot & PROT_EXEC != 0 {
        host |= PROT_READ;
    }
    host
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub start: u64,
    pub end: u64,
    pub prot: u32,
}

/// Sorted, non-overlapping guest intervals `[start, end)`; adjacent ones with the same
/// protection are always merged.
#[derive(Default)]
pub struct Intervals {
    map: BTreeMap<u64, (u64, u32)>,
}

impl Intervals {
    pub fn new() -> Intervals {
        Intervals::default()
    }

    pub fn regions(&self) -> Vec<Region> {
        self.map
            .iter()
            .map(|(&start, &(end, prot))| Region { start, end, prot })
            .collect()
    }

    pub fn find(&self, addr: u64) -> Option<Region> {
        let (&start, &(end, prot)) = self.map.range(..=addr).next_back()?;
        (end > addr).then_some(Region { start, end, prot })
    }

    pub fn overlapping(&self, start: u64, end: u64) -> Vec<Region> {
        let mut out: Vec<Region> = self
            .map
            .range(..end)
            .rev()
            .take_while(|(_, (e, _))| *e > start)
            .map(|(&s, &(e, p))| Region {
                start: s,
                end: e,
                prot: p,
            })
            .collect();
        out.reverse();
        out
    }

    pub fn overlaps(&self, start: u64, end: u64) -> bool {
        self.map
            .range(..end)
            .next_back()
            .is_some_and(|(_, &(e, _))| e > start)
    }

    /// Sub-ranges of `[start, end)` that no guest interval covers.
    pub fn gaps(&self, start: u64, end: u64) -> Vec<(u64, u64)> {
        let mut out = Vec::new();
        let mut cursor = start;
        for r in self.overlapping(start, end) {
            if r.start > cursor {
                out.push((cursor, r.start));
            }
            cursor = cursor.max(r.end);
        }
        if cursor < end {
            out.push((cursor, end));
        }
        out
    }

    pub fn covers(&self, start: u64, end: u64) -> bool {
        self.gaps(start, end).is_empty()
    }

    /// Removes `[start, end)`, splitting the intervals that stick out of it.
    pub fn remove(&mut self, start: u64, end: u64) {
        for r in self.overlapping(start, end) {
            self.map.remove(&r.start);
            if r.start < start {
                self.map.insert(r.start, (start, r.prot));
            }
            if r.end > end {
                self.map.insert(end, (r.end, r.prot));
            }
        }
    }

    /// Sets `[start, end)` to `prot`, replacing what was there and merging with neighbors.
    pub fn insert(&mut self, start: u64, end: u64, prot: u32) {
        self.remove(start, end);
        let mut start = start;
        let mut end = end;
        if let Some((&ps, &(pe, pp))) = self.map.range(..start).next_back()
            && pe == start
            && pp == prot
        {
            self.map.remove(&ps);
            start = ps;
        }
        if let Some(&(ne, np)) = self.map.get(&end)
            && np == prot
        {
            self.map.remove(&end);
            end = ne;
        }
        self.map.insert(start, (end, prot));
    }

    /// Lowest address in `[lo, hi)` with `len` free bytes, ignoring host mappings.
    pub fn first_fit(&self, lo: u64, hi: u64, len: u64) -> Option<u64> {
        let mut cursor = lo;
        for (&s, &(e, _)) in &self.map {
            if e <= cursor {
                continue;
            }
            if s >= cursor && s - cursor >= len {
                break;
            }
            cursor = e;
        }
        let end = cursor.checked_add(len)?;
        (end <= hi).then_some(cursor)
    }
}

/// The guest address space of the process.
#[derive(Default)]
pub struct Mem {
    pub intervals: Intervals,
    brk_base: u64,
    brk_cur: u64,
    code_cache: u64,
    window_start: u64,
}

impl Mem {
    pub fn new() -> Mem {
        Mem::default()
    }

    /// Fixes the layout that depends on the executable: `brk`, code cache and window.
    pub fn set_brk_base(&mut self, base: u64) {
        self.brk_base = base;
        self.brk_cur = base;
        self.code_cache = align_up(base + BRK_LIMIT, CODE_CACHE_ALIGN);
        self.window_start = self.code_cache + CODE_CACHE_SIZE + WINDOW_GAP;
    }

    pub fn brk_base(&self) -> u64 {
        self.brk_base
    }

    pub fn window_start(&self) -> u64 {
        self.window_start
    }

    /// Code cache range `[start, start + CODE_CACHE_SIZE)`, valid after `set_brk_base`.
    pub fn code_cache(&self) -> u64 {
        self.code_cache
    }

    /// Reserves the code cache range as calco memory (`PROT_NONE`); the JIT opens it later.
    pub fn reserve_code_cache(&mut self) -> Result<u64, i32> {
        if self.code_cache == 0 {
            return Err(EINVAL);
        }
        let flags = MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE | MAP_FIXED_NOREPLACE;
        unsafe { sys::mmap(self.code_cache, CODE_CACHE_SIZE, PROT_NONE, flags, -1, 0) }
    }

    /// Maps the main guest stack below `STACK_TOP` with a guard page under it.
    pub fn map_stack(&mut self) -> Result<u64, i32> {
        let low = STACK_TOP - STACK_SIZE;
        let flags = MAP_PRIVATE | MAP_FIXED_NOREPLACE | MAP_STACK;
        self.map_anon(low - PAGE_SIZE, PAGE_SIZE, PROT_NONE, flags)?;
        self.map_anon(low, STACK_SIZE, PROT_READ | PROT_WRITE, flags)?;
        Ok(STACK_TOP)
    }

    pub fn map_anon(&mut self, addr: u64, len: u64, prot: u32, flags: u32) -> Result<u64, i32> {
        self.map(addr, len, prot, flags | MAP_ANONYMOUS, -1, 0)
    }

    pub fn map_file(
        &mut self,
        addr: u64,
        len: u64,
        prot: u32,
        flags: u32,
        fd: i32,
        offset: u64,
    ) -> Result<u64, i32> {
        if offset % PAGE_SIZE != 0 {
            return Err(EINVAL);
        }
        self.map(addr, len, prot, flags & !MAP_ANONYMOUS, fd, offset)
    }

    fn map(
        &mut self,
        addr: u64,
        len: u64,
        prot: u32,
        flags: u32,
        fd: i32,
        offset: u64,
    ) -> Result<u64, i32> {
        if len == 0 || prot & !PROT_ALL != 0 {
            return Err(EINVAL);
        }
        if len > USER_END {
            return Err(ENOMEM);
        }
        let len = page_up(len);
        let hprot = host_prot(prot);
        let hflags = flags & PASS_FLAGS;

        if flags & (MAP_FIXED | MAP_FIXED_NOREPLACE) != 0 {
            if addr % PAGE_SIZE != 0 {
                return Err(EINVAL);
            }
            let end = addr
                .checked_add(len)
                .filter(|&e| e <= USER_END)
                .ok_or(ENOMEM)?;
            if flags & MAP_FIXED_NOREPLACE != 0 {
                if self.intervals.overlaps(addr, end) {
                    return Err(EEXIST);
                }
                let flags = hflags | MAP_FIXED_NOREPLACE;
                unsafe { sys::mmap(addr, len, hprot, flags, fd, offset) }
                    .map_err(|e| if e == EEXIST { ENOMEM } else { e })?;
            } else {
                // Guest-owned parts are replaced; the rest must be free in the host.
                let claimed = self.claim(addr, end)?;
                let res = unsafe { sys::mmap(addr, len, hprot, hflags | MAP_FIXED, fd, offset) };
                if let Err(e) = res {
                    release(&claimed);
                    return Err(e);
                }
            }
            self.intervals.insert(addr, end, prot);
            return Ok(addr);
        }

        let hint = page_down(addr);
        if hint != 0
            && hint.checked_add(len).is_some_and(|e| e <= WINDOW_END)
            && !self.intervals.overlaps(hint, hint + len)
        {
            let flags = hflags | MAP_FIXED_NOREPLACE;
            if unsafe { sys::mmap(hint, len, hprot, flags, fd, offset) }.is_ok() {
                self.intervals.insert(hint, hint + len, prot);
                return Ok(hint);
            }
        }
        let start = self.place(len, hprot, hflags, fd, offset)?;
        self.intervals.insert(start, start + len, prot);
        Ok(start)
    }

    /// First fit in the placement window, skipping ranges the host already uses.
    fn place(&self, len: u64, hprot: u32, hflags: u32, fd: i32, offset: u64) -> Result<u64, i32> {
        if self.window_start == 0 {
            return Err(ENOMEM);
        }
        let mut lo = self.window_start;
        loop {
            let at = self
                .intervals
                .first_fit(lo, WINDOW_END, len)
                .ok_or(ENOMEM)?;
            let flags = hflags | MAP_FIXED_NOREPLACE;
            match unsafe { sys::mmap(at, len, hprot, flags, fd, offset) } {
                Ok(_) => return Ok(at),
                Err(EEXIST) => lo = at + len,
                Err(e) => return Err(e),
            }
        }
    }

    /// Reserves in the host every gap of `[start, end)` not owned by the guest. Fails with
    /// `ENOMEM` (and reserves nothing) if a gap is calco memory.
    fn claim(&self, start: u64, end: u64) -> Result<Vec<(u64, u64)>, i32> {
        let mut claimed = Vec::new();
        for (s, e) in self.intervals.gaps(start, end) {
            let flags = MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE | MAP_FIXED_NOREPLACE;
            match unsafe { sys::mmap(s, e - s, PROT_NONE, flags, -1, 0) } {
                Ok(_) => claimed.push((s, e)),
                Err(err) => {
                    release(&claimed);
                    return Err(if err == EEXIST { ENOMEM } else { err });
                }
            }
        }
        Ok(claimed)
    }

    /// Changes the logical protection of a fully guest-owned range.
    pub fn protect(&mut self, addr: u64, len: u64, prot: u32) -> Result<(), i32> {
        if addr % PAGE_SIZE != 0 || prot & !PROT_ALL != 0 {
            return Err(EINVAL);
        }
        if len == 0 {
            return Ok(());
        }
        let end = addr
            .checked_add(page_up(len.min(USER_END)))
            .filter(|&e| e <= USER_END)
            .ok_or(ENOMEM)?;
        if !self.intervals.covers(addr, end) {
            return Err(ENOMEM);
        }
        unsafe { sys::mprotect(addr, end - addr, host_prot(prot)) }?;
        self.intervals.insert(addr, end, prot);
        Ok(())
    }

    /// Unmaps the guest-owned parts of a range; calco memory inside it is left alone.
    pub fn unmap(&mut self, addr: u64, len: u64) -> Result<(), i32> {
        if addr % PAGE_SIZE != 0 || len == 0 {
            return Err(EINVAL);
        }
        let end = addr
            .checked_add(page_up(len.min(USER_END)))
            .filter(|&e| e <= USER_END)
            .ok_or(EINVAL)?;
        for r in self.intervals.overlapping(addr, end) {
            let s = r.start.max(addr);
            let e = r.end.min(end);
            unsafe { sys::munmap(s, e - s) }?;
        }
        self.intervals.remove(addr, end);
        Ok(())
    }

    /// Emulated `mremap` of a guest range: shrinks or grows in place, or moves to the
    /// placement window (`MREMAP_MAYMOVE`) or to a fixed address (`MREMAP_FIXED`), never
    /// over calco memory. The range must lie in one guest interval; `MREMAP_DONTUNMAP` is
    /// not supported.
    pub fn remap(
        &mut self,
        old: u64,
        old_len: u64,
        new_len: u64,
        flags: u32,
        new_addr: u64,
    ) -> Result<u64, i32> {
        if old % PAGE_SIZE != 0 || flags & !(MREMAP_MAYMOVE | MREMAP_FIXED) != 0 {
            return Err(EINVAL);
        }
        if flags & MREMAP_FIXED != 0 && flags & MREMAP_MAYMOVE == 0 {
            return Err(EINVAL);
        }
        if old_len == 0 || new_len == 0 || old_len > USER_END || new_len > USER_END {
            return Err(EINVAL);
        }
        let old_len = page_up(old_len);
        let new_len = page_up(new_len);
        let old_end = old
            .checked_add(old_len)
            .filter(|&e| e <= USER_END)
            .ok_or(EFAULT)?;
        let prot = self
            .intervals
            .find(old)
            .filter(|r| r.end >= old_end)
            .ok_or(EFAULT)?
            .prot;

        if flags & MREMAP_FIXED != 0 {
            if new_addr % PAGE_SIZE != 0 {
                return Err(EINVAL);
            }
            let new_end = new_addr
                .checked_add(new_len)
                .filter(|&e| e <= USER_END)
                .ok_or(EINVAL)?;
            if new_addr < old_end && old < new_end {
                return Err(EINVAL);
            }
            // Claims the target like a guest MAP_FIXED: ENOMEM over calco memory.
            self.map_anon(new_addr, new_len, PROT_NONE, MAP_PRIVATE | MAP_FIXED)?;
            return self.move_range(old, old_len, new_len, new_addr, prot);
        }
        if new_len <= old_len {
            if new_len < old_len {
                self.unmap(old + new_len, old_len - new_len)?;
            }
            return Ok(old);
        }
        // In place when the pages after the range are free in the host and stay below the
        // window end, away from calco's own mappings.
        if let Some(end) = old.checked_add(new_len).filter(|&e| e <= WINDOW_END)
            && !self.intervals.overlaps(old_end, end)
            && unsafe { sys::mremap(old, old_len, new_len, 0, 0) }.is_ok()
        {
            self.intervals.insert(old_end, end, prot);
            return Ok(old);
        }
        if flags & MREMAP_MAYMOVE == 0 {
            return Err(ENOMEM);
        }
        let target = self.map_anon(0, new_len, PROT_NONE, MAP_PRIVATE)?;
        self.move_range(old, old_len, new_len, target, prot)
    }

    /// Moves `[old, old + old_len)` onto `target`, a guest range of `new_len` bytes just
    /// claimed by the caller.
    fn move_range(
        &mut self,
        old: u64,
        old_len: u64,
        new_len: u64,
        target: u64,
        prot: u32,
    ) -> Result<u64, i32> {
        let flags = MREMAP_MAYMOVE | MREMAP_FIXED;
        if let Err(e) = unsafe { sys::mremap(old, old_len, new_len, flags, target) } {
            let _ = self.unmap(target, new_len);
            return Err(e);
        }
        self.intervals.remove(old, old + old_len);
        self.intervals.insert(target, target + new_len, prot);
        Ok(target)
    }

    /// Emulated `brk`: returns the new break, or the current one if the request is refused.
    pub fn brk(&mut self, new: u64) -> u64 {
        if self.brk_base == 0 || new < self.brk_base || new > self.brk_base + BRK_LIMIT {
            return self.brk_cur;
        }
        let old_top = page_up(self.brk_cur);
        let new_top = page_up(new);
        if new_top > old_top {
            let flags = MAP_PRIVATE | MAP_FIXED_NOREPLACE;
            let prot = PROT_READ | PROT_WRITE;
            if self
                .map_anon(old_top, new_top - old_top, prot, flags)
                .is_err()
            {
                return self.brk_cur;
            }
        } else if new_top < old_top && self.unmap(new_top, old_top - new_top).is_err() {
            return self.brk_cur;
        }
        self.brk_cur = new;
        new
    }
}

fn release(ranges: &[(u64, u64)]) {
    for &(s, e) in ranges {
        let _ = unsafe { sys::munmap(s, e - s) };
    }
}

#[cfg(test)]
mod tests;
