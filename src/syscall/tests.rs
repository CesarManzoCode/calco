use super::*;
use crate::mem::{BRK_LIMIT, PAGE_SIZE};
use crate::sys::{EACCES, ENOENT, ENOEXEC, MAP_PRIVATE};

fn state() -> State {
    State::new(b"/opt/guest/hello".to_vec(), b"/opt/calco".to_vec(), None)
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
    let args = [addr, len, rwx, 0, 0, 0];
    assert!(
        invalidated(MPROTECT, args).is_empty(),
        "adding write keeps translations"
    );
    assert_eq!(
        reprotected(MPROTECT, args),
        Some((addr..addr + len, rwx as u32))
    );

    assert_eq!(
        st.dispatch(&mut cpu, &mut mem, MUNMAP, [addr, len, 0, 0, 0, 0]),
        0
    );
    assert_eq!(
        invalidated(MUNMAP, [addr, len, 0, 0, 0, 0]),
        vec![addr..addr + len],
        "unmapping code drops its translations"
    );
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
        (MADVISE, Kind::Emulated),
        (WAIT4, Kind::Emulated),
        (EXECVE, Kind::Emulated),
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

#[test]
fn reexec_runs_calco_on_the_program_with_the_guest_argv() {
    let argv = [b"name".to_vec(), b"child".to_vec(), b"".to_vec()];
    let want: [&[u8]; 7] = [
        b"/opt/calco",
        b"--argv0",
        b"name",
        b"--",
        b"./exec",
        b"child",
        b"",
    ];
    assert_eq!(reexec_argv(b"/opt/calco", b"./exec", &argv), want);
    let want: [&[u8]; 5] = [b"/opt/calco", b"--argv0", b"", b"--", b"-x"];
    assert_eq!(reexec_argv(b"/opt/calco", b"-x", &[]), want);
}

#[test]
fn wait_status_signals_63_and_64_become_33_and_32() {
    // Killed (with and without core), stopped, exited, killed by a signal kept as is.
    assert_eq!(guest_wait_status(63), 33);
    assert_eq!(guest_wait_status(0x80 | 64), 0x80 | 32);
    assert_eq!(guest_wait_status((64 << 8) | 0x7f), (32 << 8) | 0x7f);
    assert_eq!(guest_wait_status(7 << 8), 7 << 8);
    assert_eq!(guest_wait_status(9), 9);
    assert_eq!(guest_waitid_status(CLD_KILLED, 64), 32);
    assert_eq!(guest_waitid_status(CLD_STOPPED, 63), 33);
    assert_eq!(
        guest_waitid_status(1, 63),
        63,
        "an exit code is not a signal"
    );
}

#[test]
fn execve_that_the_kernel_refuses_returns_its_errno() {
    let mut st = state();
    let mut cpu = Cpu::new(0, 0);
    let mut mem = Mem::new();
    mem.set_brk_base(0x0000_2c00_0000_0000);
    let rw = (PROT_READ | PROT_WRITE) as u64;
    let flags = (MAP_PRIVATE | MAP_ANONYMOUS) as u64;
    let page = st.dispatch(
        &mut cpu,
        &mut mem,
        MMAP,
        [0, PAGE_SIZE, rw, flags, u64::MAX, 0],
    );
    assert!(page > 0, "mmap failed: {page}");
    let page = page as u64;

    let dir = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let text = dir.join("calco-k2-01-text");
    let script = dir.join("calco-k2-01-script");
    std::fs::write(&text, "not a program\n").unwrap();
    std::fs::write(&script, "#! /nonexistent/calco-k2-01 -x\n").unwrap();
    for f in [&text, &script] {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(f, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let cases: [(&[u8], i32); 5] = [
        (b"/nonexistent/calco-k2-01", ENOENT),
        (b"/", EACCES),
        (b"/etc/passwd", EACCES),
        (text.as_os_str().as_encoded_bytes(), ENOEXEC),
        (script.as_os_str().as_encoded_bytes(), ENOENT),
    ];
    for (path, errno) in cases {
        // Path at the page start, argv = { path, NULL } after it.
        let argv = page + 2048;
        unsafe {
            std::ptr::write_bytes(page as *mut u8, 0, PAGE_SIZE as usize);
            std::ptr::copy_nonoverlapping(path.as_ptr(), page as *mut u8, path.len());
            (argv as *mut u64).write(page);
        }
        let args = [page, argv, 0, 0, 0, 0];
        assert_eq!(
            st.dispatch(&mut cpu, &mut mem, EXECVE, args),
            -(errno as i64),
            "{}",
            String::from_utf8_lossy(path)
        );
    }
    let bad = [page + PAGE_SIZE, 0, 0, 0, 0, 0];
    assert_eq!(
        st.dispatch(&mut cpu, &mut mem, EXECVE, bad),
        -(EFAULT as i64)
    );
}

#[test]
fn memory_calls_name_the_ranges_they_invalidate() {
    let page = 0x0000_2d00_0000_0000u64;
    let rw = (PROT_READ | PROT_WRITE) as u64;
    let rx = (PROT_READ | PROT_EXEC) as u64;
    // Lengths round up to whole pages, as the kernel does.
    assert_eq!(
        invalidated(MPROTECT, [page, 1, rw, 0, 0, 0]),
        vec![page..page + PAGE_SIZE]
    );
    assert!(invalidated(MPROTECT, [page, 1, rx, 0, 0, 0]).is_empty());
    assert_eq!(
        reprotected(MPROTECT, [page, 1, rx, 0, 0, 0]),
        Some((page..page + PAGE_SIZE, rx as u32))
    );
    assert_eq!(reprotected(MPROTECT, [page, 1, rw, 0, 0, 0]), None);
    assert_eq!(
        invalidated(MADVISE, [page, 2 * PAGE_SIZE, MADV_DONTNEED, 0, 0, 0]),
        vec![page..page + 2 * PAGE_SIZE]
    );
    // MADV_WILLNEED (3) keeps the contents.
    assert!(invalidated(MADVISE, [page, PAGE_SIZE, 3, 0, 0, 0]).is_empty());
    let fixed = (MAP_PRIVATE | MAP_ANONYMOUS | sys::MAP_FIXED) as u64;
    assert_eq!(
        invalidated(MMAP, [page, PAGE_SIZE, rx, fixed, u64::MAX, 0]),
        vec![page..page + PAGE_SIZE]
    );
    let placed = (MAP_PRIVATE | MAP_ANONYMOUS) as u64;
    assert!(invalidated(MMAP, [0, PAGE_SIZE, rx, placed, u64::MAX, 0]).is_empty());
    let target = page + 0x10_0000;
    let moved = (sys::MREMAP_MAYMOVE | sys::MREMAP_FIXED) as u64;
    assert_eq!(
        invalidated(MREMAP, [page, PAGE_SIZE, 2 * PAGE_SIZE, moved, target, 0]),
        vec![page..page + PAGE_SIZE, target..target + 2 * PAGE_SIZE]
    );
    assert!(invalidated(MUNMAP, [page, 0, 0, 0, 0, 0]).is_empty());
    assert!(invalidated(BRK, [page, 0, 0, 0, 0, 0]).is_empty());
}
