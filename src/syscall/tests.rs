use super::*;
use crate::mem::PAGE_SIZE;
use crate::mem::tests::host_perms;
use crate::sys::{EACCES, ENOENT, ENOEXEC, MAP_PRIVATE};

fn state() -> State {
    State::new(b"/opt/guest/hello".to_vec(), b"/opt/calco".to_vec(), None)
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
    assert_eq!(host_perms(addr).as_deref(), Some("r--p"));

    let rwx = (PROT_READ | PROT_WRITE | PROT_EXEC) as u64;
    let args = [addr, len, rwx, 0, 0, 0];
    assert_eq!(st.dispatch(&mut cpu, &mut mem, MPROTECT, args), 0);
    assert_eq!(host_perms(addr).as_deref(), Some("rw-p"));
    let kept = invalidated(MPROTECT, args);
    assert!(kept.is_empty(), "adding write keeps translations");
    let want = Some((addr..addr + len, rwx as u32));
    assert_eq!(reprotected(MPROTECT, args), want);

    let args = [addr, len, 0, 0, 0, 0];
    assert_eq!(st.dispatch(&mut cpu, &mut mem, MUNMAP, args), 0);
    let dropped = invalidated(MUNMAP, args);
    assert_eq!(
        dropped,
        vec![addr..addr + len],
        "unmapping code drops its translations"
    );
    assert_eq!(mem.intervals.find(addr), None);
}

#[test]
fn classification_of_numbers() {
    for (nr, kind) in [
        (0, Kind::Pass),
        (MMAP, Kind::Emulated),
        (68, Kind::Unlisted),
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
    for nr in [RSEQ, CLONE3, 68, 1000] {
        let r = st.dispatch(&mut cpu, &mut mem, nr, [0; 6]);
        assert_eq!(r, -(ENOSYS as i64), "syscall {nr}");
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
    let r = st.dispatch(&mut cpu, &mut mem, ARCH_PRCTL, high);
    assert_eq!(r, -(EPERM as i64));
    // ARCH_SHSTK_ENABLE
    let shstk = [0x5001, 1, 0, 0, 0, 0];
    let r = st.dispatch(&mut cpu, &mut mem, ARCH_PRCTL, shstk);
    assert_eq!(r, -(EINVAL as i64));
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
fn a_symlink_to_calco_is_calco() {
    let exe = std::env::current_exe().unwrap();
    let link = exe.with_file_name("calco-k3-fix-link");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&exe, &link).unwrap();
    let (exe, link) = (exe.as_os_str().as_bytes(), link.as_os_str().as_bytes());
    assert!(same_file(link, exe));
    assert!(!same_file(b"/etc/passwd", exe));
    assert!(!same_file(b"/nonexistent/calco", b"/nonexistent/calco"));
    std::fs::remove_file(OsStr::from_bytes(link)).unwrap();
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
    let args = [0, PAGE_SIZE, rw, flags, u64::MAX, 0];
    let page = st.dispatch(&mut cpu, &mut mem, MMAP, args);
    assert!(page > 0, "mmap failed: {page}");
    let page = page as u64;

    let exe = std::env::current_exe().unwrap();
    let text = exe.with_file_name("calco-k2-01-text");
    let script = exe.with_file_name("calco-k2-01-script");
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
        let r = st.dispatch(&mut cpu, &mut mem, EXECVE, [page, argv, 0, 0, 0, 0]);
        assert_eq!(r, -(errno as i64), "{}", String::from_utf8_lossy(path));
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
    let fixed = (MAP_PRIVATE | MAP_ANONYMOUS | sys::MAP_FIXED) as u64;
    let placed = (MAP_PRIVATE | MAP_ANONYMOUS) as u64;
    let target = page + 0x10_0000;
    let moved = (sys::MREMAP_MAYMOVE | sys::MREMAP_FIXED) as u64;
    let (one, two) = (page..page + PAGE_SIZE, page..page + 2 * PAGE_SIZE);
    // Lengths round up to whole pages, as the kernel does.
    #[rustfmt::skip]
    let cases = [
        (MPROTECT, [page, 1, rw, 0, 0, 0], vec![one.clone()]),
        (MPROTECT, [page, 1, rx, 0, 0, 0], vec![]),
        (MADVISE, [page, 2 * PAGE_SIZE, MADV_DONTNEED, 0, 0, 0], vec![two]),
        // MADV_WILLNEED (3) keeps the contents.
        (MADVISE, [page, PAGE_SIZE, 3, 0, 0, 0], vec![]),
        (MMAP, [page, PAGE_SIZE, rx, fixed, u64::MAX, 0], vec![one.clone()]),
        (MMAP, [0, PAGE_SIZE, rx, placed, u64::MAX, 0], vec![]),
        (MREMAP, [page, PAGE_SIZE, 2 * PAGE_SIZE, moved, target, 0],
         vec![one.clone(), target..target + 2 * PAGE_SIZE]),
        (MUNMAP, [page, 0, 0, 0, 0, 0], vec![]),
        (BRK, [page, 0, 0, 0, 0, 0], vec![]),
    ];
    for (nr, args, ranges) in cases {
        assert_eq!(invalidated(nr, args), ranges, "syscall {nr} {args:x?}");
    }
    let rx_args = [page, 1, rx, 0, 0, 0];
    assert_eq!(reprotected(MPROTECT, rx_args), Some((one, rx as u32)));
    assert_eq!(reprotected(MPROTECT, [page, 1, rw, 0, 0, 0]), None);
}
