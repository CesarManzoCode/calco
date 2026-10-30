use super::*;
use crate::mem::{BRK_LIMIT, PAGE_SIZE};
use crate::sys::MAP_PRIVATE;

fn state() -> State {
    State::new(b"/opt/guest/hello".to_vec(), None)
}

/// Permission column of the host `/proc/self/maps` line that starts at `addr`.
fn host_perms(addr: u64) -> String {
    let maps = std::fs::read_to_string("/proc/self/maps").unwrap();
    let prefix = format!("{addr:x}-");
    let line = maps
        .lines()
        .find(|l| l.starts_with(&prefix))
        .unwrap_or_else(|| panic!("no mapping at {addr:#x}"));
    line.split_whitespace().nth(1).unwrap().to_string()
}

#[test]
fn mmap_drops_prot_exec_and_keeps_logical_prot() {
    let mut st = state();
    let mut cpu = Cpu::new(0, 0);
    let mut mem = Mem::new();
    mem.set_brk_base(0x0000_2d00_0000_0000);
    let rx = (PROT_READ | PROT_EXEC) as u64;
    let flags = (MAP_PRIVATE | MAP_ANONYMOUS) as u64;
    let len = 2 * PAGE_SIZE;
    let addr = st.dispatch(&mut cpu, &mut mem, MMAP, [0, len, rx, flags, u64::MAX, 0]);
    assert!(addr > 0, "mmap failed: {addr}");
    let addr = addr as u64;
    assert_eq!(addr, mem.window_start());
    assert_eq!(
        mem.intervals.find(addr).unwrap().prot,
        PROT_READ | PROT_EXEC
    );
    assert_eq!(host_perms(addr), "r--p");

    let rwx = (PROT_READ | PROT_WRITE | PROT_EXEC) as u64;
    assert_eq!(
        st.dispatch(&mut cpu, &mut mem, MPROTECT, [addr, len, rwx, 0, 0, 0]),
        0
    );
    assert_eq!(host_perms(addr), "rw-p");
    assert!(!st.take_code_changed(), "adding write keeps translations");

    assert_eq!(
        st.dispatch(&mut cpu, &mut mem, MUNMAP, [addr, len, 0, 0, 0, 0]),
        0
    );
    assert!(st.take_code_changed(), "unmapping code stales the cache");
    assert_eq!(mem.intervals.find(addr), None);
}

#[test]
fn brk_stops_at_its_limit() {
    let mut st = state();
    let mut cpu = Cpu::new(0, 0);
    let mut mem = Mem::new();
    let base = 0x0000_2e00_0000_0000;
    mem.set_brk_base(base);
    let mut brk = |mem: &mut Mem, v: u64| st.dispatch(&mut cpu, mem, BRK, [v, 0, 0, 0, 0, 0]);
    assert_eq!(brk(&mut mem, 0), base as i64);
    assert_eq!(brk(&mut mem, base + BRK_LIMIT), (base + BRK_LIMIT) as i64);
    assert_eq!(
        brk(&mut mem, base + BRK_LIMIT + PAGE_SIZE),
        (base + BRK_LIMIT) as i64,
        "past the limit the break stays"
    );
    assert_eq!(brk(&mut mem, base - PAGE_SIZE), (base + BRK_LIMIT) as i64);
    assert_eq!(brk(&mut mem, base), base as i64);
    assert_eq!(mem.intervals.find(base), None);
}

#[test]
fn classification_of_numbers() {
    for (nr, kind) in [
        (0, Kind::Pass),
        (1, Kind::Pass),
        (202, Kind::Pass),
        (318, Kind::Pass),
        (28, Kind::Pass),
        (MMAP, Kind::Emulated),
        (BRK, Kind::Emulated),
        (READLINK, Kind::Emulated),
        (PRCTL, Kind::Emulated),
        (RSEQ, Kind::Emulated),
        (CLONE3, Kind::Emulated),
        (EXIT_GROUP, Kind::Emulated),
        (29, Kind::Unlisted),
        (101, Kind::Unlisted),
        (425, Kind::Unlisted),
        (1000, Kind::Unlisted),
        (u64::MAX, Kind::Unlisted),
    ] {
        assert_eq!(classify(nr), kind, "syscall {nr}");
    }
}

#[test]
fn tables_match_the_kernel_header_and_do_not_overlap() {
    let header = std::fs::read_to_string("/usr/include/asm/unistd_64.h").unwrap();
    let numbers: std::collections::HashMap<&str, u64> = header
        .lines()
        .filter_map(|l| {
            let mut w = l.split_whitespace();
            (w.next()? == "#define").then_some(())?;
            let name = w.next()?.strip_prefix("__NR_")?;
            Some((name, w.next()?.parse().ok()?))
        })
        .collect();
    let mut seen = std::collections::HashSet::new();
    for &(nr, name) in PASS.iter().chain(EMULATED) {
        assert_eq!(numbers.get(name), Some(&nr), "{name}");
        assert!(seen.insert(nr), "{name} listed twice");
    }
    assert_eq!(PASS.len() + EMULATED.len(), seen.len());
}

#[test]
fn deferred_calls_return_enosys() {
    let mut st = state();
    let mut cpu = Cpu::new(0, 0);
    let mut mem = Mem::new();
    for nr in [RSEQ, CLONE3, 29, 1000] {
        assert_eq!(
            st.dispatch(&mut cpu, &mut mem, nr, [0; 6]),
            -(ENOSYS as i64),
            "syscall {nr}"
        );
    }
}

#[test]
fn arch_prctl_sets_fs_and_refuses_shadow_stack() {
    let mut st = state();
    let mut cpu = Cpu::new(0, 0);
    let mut mem = Mem::new();
    let set = [ARCH_SET_FS, 0x1234_5000, 0, 0, 0, 0];
    assert_eq!(st.dispatch(&mut cpu, &mut mem, ARCH_PRCTL, set), 0);
    assert_eq!(cpu.fs_base, 0x1234_5000);
    let high = [ARCH_SET_FS, USER_END, 0, 0, 0, 0];
    assert_eq!(
        st.dispatch(&mut cpu, &mut mem, ARCH_PRCTL, high),
        -(EPERM as i64)
    );
    // ARCH_SHSTK_ENABLE
    let shstk = [0x5001, 1, 0, 0, 0, 0];
    assert_eq!(
        st.dispatch(&mut cpu, &mut mem, ARCH_PRCTL, shstk),
        -(EINVAL as i64)
    );
}

#[test]
fn signal_numbers_map_like_the_contract() {
    assert_eq!(host_signal(0), Some(0));
    assert_eq!(host_signal(11), Some(11));
    assert_eq!(host_signal(32), Some(64));
    assert_eq!(host_signal(33), Some(63));
    assert_eq!(host_signal(34), Some(34));
    assert_eq!(host_signal(61), Some(61));
    assert_eq!(host_signal(62), None);
    assert_eq!(host_signal(65), None);
}
