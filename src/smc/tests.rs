use super::*;
use crate::sys::{PROT_READ, PROT_WRITE};

const PAGE: u64 = 0x0000_2d00_0000_0000;
const RWX: u32 = PROT_READ | PROT_WRITE | PROT_EXEC;
const RX: u32 = PROT_READ | PROT_EXEC;

fn all(prot: u32) -> impl Fn(u64) -> u32 {
    move |_| prot
}

#[test]
fn a_block_touches_the_pages_of_its_guest_bytes() {
    let one: Vec<u64> = block_pages(PAGE + 0x10, PAGE + 0x40).collect();
    assert_eq!(one, vec![PAGE]);
    // The last instruction crosses into the next page.
    let two: Vec<u64> = block_pages(PAGE + 0xff0, PAGE + 0x1003).collect();
    assert_eq!(two, vec![PAGE, PAGE + PAGE_SIZE]);
    // Ends exactly at the page end: one page.
    let edge: Vec<u64> = block_pages(PAGE + 0xff0, PAGE + 0x1000).collect();
    assert_eq!(edge, vec![PAGE]);
    // A block with no bytes read (cannot happen, but) still holds its first byte.
    let empty: Vec<u64> = block_pages(PAGE + 8, PAGE + 8).collect();
    assert_eq!(empty, vec![PAGE]);
    assert_eq!(pages(PAGE, PAGE).count(), 0);
}

#[test]
fn spans_round_like_the_kernel() {
    assert_eq!(span(PAGE, 1), Some(PAGE..PAGE + PAGE_SIZE));
    assert_eq!(span(PAGE + 5, PAGE_SIZE), Some(PAGE..PAGE + 2 * PAGE_SIZE));
    assert_eq!(span(PAGE, 0), None);
    assert_eq!(span(USER_END, PAGE_SIZE), None);
    assert_eq!(
        span(USER_END - PAGE_SIZE, u64::MAX),
        Some(USER_END - PAGE_SIZE..USER_END)
    );
}

#[test]
fn writable_code_pages_are_protected_once() {
    let mut smc = Smc::new();
    let first = smc.track(0, PAGE + 0x10, PAGE + 0x20, all(RWX));
    assert_eq!(first, vec![(PAGE, PROT_READ)]);
    // A second block of the same page: already protected.
    assert!(smc.track(1, PAGE + 0x20, PAGE + 0x30, all(RWX)).is_empty());
    let p = smc.page(PAGE + 0x123).unwrap();
    assert!(p.protected);
    assert_eq!(p.blocks, vec![0, 1]);
    // Code the guest cannot write is noted but left alone.
    assert!(
        smc.track(2, PAGE + 0x2000, PAGE + 0x2010, all(RX))
            .is_empty()
    );
    assert!(!smc.page(PAGE + 0x2000).unwrap().protected);
}

#[test]
fn a_write_fault_drops_the_page_with_its_blocks() {
    let mut smc = Smc::new();
    smc.track(0, PAGE + 0x10, PAGE + 0x20, all(RWX));
    // Crosses into the next page: in both.
    smc.track(1, PAGE + 0xff8, PAGE + 0x1004, all(RWX));
    smc.track(2, PAGE + 0x1100, PAGE + 0x1110, all(RWX));

    let inv = smc.write_fault(PAGE + 0x800).unwrap();
    assert_eq!(inv.blocks, vec![0, 1]);
    assert_eq!(inv.restore, vec![(PAGE, PROT_READ | PROT_WRITE)]);
    // After invalidating, the page has no state: a new translation protects it again.
    assert_eq!(smc.page(PAGE), None);
    assert_eq!(
        smc.track(3, PAGE, PAGE + 4, all(RWX)),
        vec![(PAGE, PROT_READ)]
    );
    // The next page is still protected and keeps block 1 noted.
    let next = smc.page(PAGE + PAGE_SIZE).unwrap();
    assert!(next.protected);
    assert_eq!(next.blocks, vec![1, 2]);
}

#[test]
fn faults_that_are_not_on_protected_code_belong_to_the_guest() {
    let mut smc = Smc::new();
    assert_eq!(smc.write_fault(PAGE), None);
    smc.track(0, PAGE, PAGE + 8, all(RX));
    assert_eq!(smc.write_fault(PAGE + 4), None);
    assert!(
        smc.page(PAGE).is_some(),
        "a guest fault keeps the translations"
    );
}

#[test]
fn invalidating_a_range_takes_every_page_in_it() {
    let mut smc = Smc::new();
    smc.track(0, PAGE, PAGE + 8, all(RWX));
    smc.track(1, PAGE + PAGE_SIZE, PAGE + PAGE_SIZE + 8, all(RX));
    smc.track(2, PAGE + 3 * PAGE_SIZE, PAGE + 3 * PAGE_SIZE + 8, all(RWX));
    let inv = smc.invalidate(PAGE..PAGE + 2 * PAGE_SIZE);
    assert_eq!(inv.blocks, vec![0, 1]);
    // Only the protected page needs its protection back.
    assert_eq!(inv.restore, vec![(PAGE, RWX & !PROT_EXEC)]);
    assert_eq!(smc.len(), 1);
    assert!(smc.invalidate(PAGE..PAGE + 2 * PAGE_SIZE).is_empty());
}

#[test]
fn mprotect_round_trip_protects_the_page_again() {
    let mut smc = Smc::new();
    smc.track(0, PAGE, PAGE + 8, all(RWX));
    // Read-only and executable: the host page is already without write.
    assert!(smc.reprotect(PAGE..PAGE + PAGE_SIZE, RX).is_empty());
    let p = smc.page(PAGE).unwrap();
    assert_eq!((p.prot, p.protected), (RX, false));
    assert_eq!(
        smc.write_fault(PAGE),
        None,
        "a write to read-only code is the guest's"
    );
    // Back to RWX: the host gave write back, calco takes it away again.
    assert_eq!(
        smc.reprotect(PAGE..PAGE + PAGE_SIZE, RWX),
        vec![(PAGE, PROT_READ)]
    );
    let inv = smc.write_fault(PAGE).unwrap();
    assert_eq!(inv.blocks, vec![0]);
    assert_eq!(inv.restore, vec![(PAGE, PROT_READ | PROT_WRITE)]);
}

#[test]
fn clearing_restores_every_protected_page() {
    let mut smc = Smc::new();
    smc.track(0, PAGE, PAGE + 8, all(RWX));
    smc.track(1, PAGE + PAGE_SIZE, PAGE + PAGE_SIZE + 8, all(RX));
    assert_eq!(smc.clear(), vec![(PAGE, PROT_READ | PROT_WRITE)]);
    assert_eq!(smc.len(), 0);
}
