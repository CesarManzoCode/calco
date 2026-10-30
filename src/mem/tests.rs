use super::*;

const R: u32 = PROT_READ;
const RW: u32 = PROT_READ | PROT_WRITE;
const RX: u32 = PROT_READ | PROT_EXEC;

fn region(start: u64, end: u64, prot: u32) -> Region {
    Region { start, end, prot }
}

#[test]
fn insert_merges_adjacent_same_prot() {
    let mut iv = Intervals::new();
    iv.insert(0x1000, 0x3000, RW);
    iv.insert(0x3000, 0x5000, RW);
    assert_eq!(iv.regions(), vec![region(0x1000, 0x5000, RW)]);
    iv.insert(0x5000, 0x6000, R);
    assert_eq!(
        iv.regions(),
        vec![region(0x1000, 0x5000, RW), region(0x5000, 0x6000, R)]
    );
}

#[test]
fn protect_middle_splits_and_restore_merges() {
    let mut iv = Intervals::new();
    iv.insert(0x10000, 0x20000, RW);
    iv.insert(0x14000, 0x15000, RX);
    assert_eq!(
        iv.regions(),
        vec![
            region(0x10000, 0x14000, RW),
            region(0x14000, 0x15000, RX),
            region(0x15000, 0x20000, RW),
        ]
    );
    iv.insert(0x14000, 0x15000, RW);
    assert_eq!(iv.regions(), vec![region(0x10000, 0x20000, RW)]);
}

#[test]
fn remove_splits() {
    let mut iv = Intervals::new();
    iv.insert(0x10000, 0x20000, RW);
    iv.remove(0x12000, 0x13000);
    assert_eq!(
        iv.regions(),
        vec![region(0x10000, 0x12000, RW), region(0x13000, 0x20000, RW)]
    );
    iv.remove(0x0, 0x11000);
    iv.remove(0x1f000, 0x30000);
    assert_eq!(
        iv.regions(),
        vec![region(0x11000, 0x12000, RW), region(0x13000, 0x1f000, RW)]
    );
}

#[test]
fn gaps_covers_and_find() {
    let mut iv = Intervals::new();
    iv.insert(0x2000, 0x4000, R);
    iv.insert(0x6000, 0x7000, RW);
    assert_eq!(
        iv.gaps(0x1000, 0x8000),
        vec![(0x1000, 0x2000), (0x4000, 0x6000), (0x7000, 0x8000)]
    );
    assert!(iv.covers(0x2000, 0x4000));
    assert!(!iv.covers(0x2000, 0x5000));
    assert!(iv.overlaps(0x3000, 0x6001));
    assert!(!iv.overlaps(0x4000, 0x6000));
    assert_eq!(iv.find(0x6fff), Some(region(0x6000, 0x7000, RW)));
    assert_eq!(iv.find(0x7000), None);
}

#[test]
fn first_fit_skips_used_ranges() {
    let mut iv = Intervals::new();
    iv.insert(0x10000, 0x12000, R);
    iv.insert(0x13000, 0x14000, R);
    assert_eq!(iv.first_fit(0x10000, 0x20000, 0x1000), Some(0x12000));
    assert_eq!(iv.first_fit(0x10000, 0x20000, 0x2000), Some(0x14000));
    assert_eq!(iv.first_fit(0x10000, 0x15000, 0x2000), None);
}

#[test]
fn host_prot_never_exec() {
    assert_eq!(host_prot(PROT_EXEC), PROT_READ);
    assert_eq!(host_prot(RX | PROT_WRITE), RW);
    assert_eq!(host_prot(PROT_NONE), PROT_NONE);
}

#[test]
fn layout_follows_brk() {
    let mut mem = Mem::new();
    mem.set_brk_base(0x40_1000);
    assert_eq!(mem.code_cache(), 0x4060_0000);
    assert_eq!(mem.code_cache() % (2 << 20), 0);
    assert_eq!(
        mem.window_start(),
        mem.code_cache() + CODE_CACHE_SIZE + (16 << 20)
    );
}

// The real-mapping tests use fixed addresses far from where the kernel places anything
// (top-down near 0x7f..), one distinct block per test.

#[test]
fn map_fixed_over_foreign_range_is_enomem() {
    let host = unsafe { sys::mmap(0, PAGE_SIZE, RW, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0) }.unwrap();
    unsafe { (host as *mut u8).write(0xa5) };
    let mut mem = Mem::new();
    let flags = MAP_PRIVATE | MAP_FIXED;
    assert_eq!(mem.map_anon(host, PAGE_SIZE, RW, flags), Err(ENOMEM));
    assert_eq!(
        mem.map_anon(host, PAGE_SIZE, RW, MAP_PRIVATE | MAP_FIXED_NOREPLACE),
        Err(ENOMEM)
    );
    assert_eq!(unsafe { (host as *const u8).read() }, 0xa5);
    assert!(mem.intervals.regions().is_empty());
    unsafe { sys::munmap(host, PAGE_SIZE) }.unwrap();
}

#[test]
fn map_fixed_over_guest_range_replaces() {
    let base = 0x0000_2a00_0000_0000;
    let mut mem = Mem::new();
    let flags = MAP_PRIVATE | MAP_FIXED;
    assert_eq!(mem.map_anon(base, 3 * PAGE_SIZE, RW, flags), Ok(base));
    let mid = base + PAGE_SIZE;
    unsafe { (mid as *mut u8).write(7) };
    assert_eq!(mem.map_anon(mid, PAGE_SIZE, R, flags), Ok(mid));
    assert_eq!(unsafe { (mid as *const u8).read() }, 0);
    assert_eq!(
        mem.intervals.regions(),
        vec![
            region(base, mid, RW),
            region(mid, mid + PAGE_SIZE, R),
            region(mid + PAGE_SIZE, base + 3 * PAGE_SIZE, RW),
        ]
    );
    assert_eq!(
        mem.map_anon(base, PAGE_SIZE, RW, MAP_PRIVATE | MAP_FIXED_NOREPLACE),
        Err(EEXIST)
    );
    mem.protect(mid, PAGE_SIZE, RW).unwrap();
    assert_eq!(
        mem.intervals.regions(),
        vec![region(base, base + 3 * PAGE_SIZE, RW)]
    );
    mem.unmap(mid, PAGE_SIZE).unwrap();
    assert_eq!(mem.intervals.regions().len(), 2);
    assert_eq!(mem.protect(base, 3 * PAGE_SIZE, R), Err(ENOMEM));
    mem.unmap(base, 3 * PAGE_SIZE).unwrap();
    assert!(mem.intervals.regions().is_empty());
}

#[test]
fn placement_uses_window() {
    let mut mem = Mem::new();
    mem.set_brk_base(0x0000_2b00_0000_0000);
    let flags = MAP_PRIVATE;
    let a = mem.map_anon(0, 2 * PAGE_SIZE, RW, flags).unwrap();
    assert_eq!(a, mem.window_start());
    let b = mem.map_anon(0, PAGE_SIZE, R, flags).unwrap();
    assert_eq!(b, a + 2 * PAGE_SIZE);
    mem.unmap(a, 3 * PAGE_SIZE).unwrap();
}

#[test]
fn brk_grows_shrinks_and_respects_limit() {
    let base = 0x0000_2c00_0000_0100;
    let mut mem = Mem::new();
    mem.set_brk_base(base);
    assert_eq!(mem.brk(0), base);
    assert_eq!(mem.brk(base + BRK_LIMIT + 1), base);
    let top = base + 3 * PAGE_SIZE;
    assert_eq!(mem.brk(top), top);
    unsafe { ((top - 1) as *mut u8).write(1) };
    assert_eq!(
        mem.intervals.find(top - 1).map(|r| r.prot),
        Some(PROT_READ | PROT_WRITE)
    );
    assert_eq!(mem.brk(base), base);
    assert_eq!(mem.intervals.find(top - 1), None);
}
