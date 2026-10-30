//! Raw host system calls through the `syscall` instruction, without crates.
//!
//! The `syscallN` functions return the kernel value as is: a result in `-4095..=-1` is `-errno`.
//! The typed wrappers turn it into `Result<_, i32>` with the positive errno.

use core::arch::asm;

// /usr/include/asm/unistd_64.h
pub const SYS_WRITE: usize = 1;
pub const SYS_MMAP: usize = 9;
pub const SYS_MPROTECT: usize = 10;
pub const SYS_MUNMAP: usize = 11;
pub const SYS_RT_SIGACTION: usize = 13;
pub const SYS_RT_SIGPROCMASK: usize = 14;
pub const SYS_MREMAP: usize = 25;
pub const SYS_CLONE: usize = 56;
pub const SYS_EXECVE: usize = 59;
pub const SYS_GETPID: usize = 39;
pub const SYS_EXIT: usize = 60;
pub const SYS_GETUID: usize = 102;
pub const SYS_GETGID: usize = 104;
pub const SYS_GETEUID: usize = 107;
pub const SYS_GETEGID: usize = 108;
pub const SYS_GETTID: usize = 186;
pub const SYS_EXIT_GROUP: usize = 231;
pub const SYS_TGKILL: usize = 234;
pub const SYS_GETRANDOM: usize = 318;
pub const SYS_FACCESSAT2: usize = 439;

// /usr/include/asm-generic/mman-common.h
pub const PROT_NONE: u32 = 0x0;
pub const PROT_READ: u32 = 0x1;
pub const PROT_WRITE: u32 = 0x2;
pub const PROT_EXEC: u32 = 0x4;
pub const MAP_TYPE: u32 = 0x0f;
pub const MAP_FIXED: u32 = 0x10;
pub const MAP_ANONYMOUS: u32 = 0x20;
pub const MAP_POPULATE: u32 = 0x008000;
pub const MAP_STACK: u32 = 0x020000;
pub const MAP_FIXED_NOREPLACE: u32 = 0x100000;
// /usr/include/linux/mman.h
pub const MAP_SHARED: u32 = 0x01;
pub const MAP_PRIVATE: u32 = 0x02;
pub const MREMAP_MAYMOVE: u32 = 1;
pub const MREMAP_FIXED: u32 = 2;
pub const MREMAP_DONTUNMAP: u32 = 4;
// /usr/include/asm-generic/mman.h
pub const MAP_NORESERVE: u32 = 0x4000;

// /usr/include/asm-generic/errno-base.h
pub const EPERM: i32 = 1;
pub const ENOENT: i32 = 2;
pub const EINTR: i32 = 4;
pub const E2BIG: i32 = 7;
pub const ENOEXEC: i32 = 8;
pub const EBADF: i32 = 9;
pub const ENOMEM: i32 = 12;
pub const EACCES: i32 = 13;
pub const EFAULT: i32 = 14;
pub const EEXIST: i32 = 17;
pub const EINVAL: i32 = 22;
// /usr/include/asm-generic/errno.h
pub const ENAMETOOLONG: i32 = 36;
pub const ENOSYS: i32 = 38;
pub const ELOOP: i32 = 40;

// /usr/include/linux/fcntl.h
pub const AT_FDCWD: i32 = -100;
pub const AT_SYMLINK_NOFOLLOW: u64 = 0x100;
pub const AT_EACCESS: usize = 0x200;
pub const AT_EMPTY_PATH: u64 = 0x1000;
// /usr/include/unistd.h
pub const X_OK: usize = 1;

// /usr/include/asm-generic/signal-defs.h
const SIG_DFL: usize = 0;
const SIG_UNBLOCK: usize = 1;

/// # Safety
/// The call must be valid for the kernel with no arguments.
#[inline]
pub unsafe fn syscall0(n: usize) -> isize {
    let ret: isize;
    unsafe {
        asm!("syscall", inlateout("rax") n as isize => ret,
            lateout("rcx") _, lateout("r11") _, options(nostack));
    }
    ret
}

/// # Safety
/// The arguments must be valid for system call `n`.
#[inline]
pub unsafe fn syscall1(n: usize, a1: usize) -> isize {
    let ret: isize;
    unsafe {
        asm!("syscall", inlateout("rax") n as isize => ret, in("rdi") a1,
            lateout("rcx") _, lateout("r11") _, options(nostack));
    }
    ret
}

/// # Safety
/// The arguments must be valid for system call `n`.
#[inline]
pub unsafe fn syscall2(n: usize, a1: usize, a2: usize) -> isize {
    let ret: isize;
    unsafe {
        asm!("syscall", inlateout("rax") n as isize => ret, in("rdi") a1, in("rsi") a2,
            lateout("rcx") _, lateout("r11") _, options(nostack));
    }
    ret
}

/// # Safety
/// The arguments must be valid for system call `n`.
#[inline]
pub unsafe fn syscall3(n: usize, a1: usize, a2: usize, a3: usize) -> isize {
    let ret: isize;
    unsafe {
        asm!("syscall", inlateout("rax") n as isize => ret, in("rdi") a1, in("rsi") a2,
            in("rdx") a3, lateout("rcx") _, lateout("r11") _, options(nostack));
    }
    ret
}

/// # Safety
/// The arguments must be valid for system call `n`.
#[inline]
pub unsafe fn syscall4(n: usize, a1: usize, a2: usize, a3: usize, a4: usize) -> isize {
    let ret: isize;
    unsafe {
        asm!("syscall", inlateout("rax") n as isize => ret, in("rdi") a1, in("rsi") a2,
            in("rdx") a3, in("r10") a4, lateout("rcx") _, lateout("r11") _, options(nostack));
    }
    ret
}

/// # Safety
/// The arguments must be valid for system call `n`.
#[inline]
pub unsafe fn syscall5(n: usize, a1: usize, a2: usize, a3: usize, a4: usize, a5: usize) -> isize {
    let ret: isize;
    unsafe {
        asm!("syscall", inlateout("rax") n as isize => ret, in("rdi") a1, in("rsi") a2,
            in("rdx") a3, in("r10") a4, in("r8") a5,
            lateout("rcx") _, lateout("r11") _, options(nostack));
    }
    ret
}

/// # Safety
/// The arguments must be valid for system call `n`.
#[inline]
pub unsafe fn syscall6(
    n: usize,
    a1: usize,
    a2: usize,
    a3: usize,
    a4: usize,
    a5: usize,
    a6: usize,
) -> isize {
    let ret: isize;
    unsafe {
        asm!("syscall", inlateout("rax") n as isize => ret, in("rdi") a1, in("rsi") a2,
            in("rdx") a3, in("r10") a4, in("r8") a5, in("r9") a6,
            lateout("rcx") _, lateout("r11") _, options(nostack));
    }
    ret
}

/// Splits a raw kernel return value into a value or a positive errno.
pub fn check(ret: isize) -> Result<usize, i32> {
    if (-4095..0).contains(&ret) {
        Err(-ret as i32)
    } else {
        Ok(ret as usize)
    }
}

/// # Safety
/// With `MAP_FIXED` the range is replaced; the caller must own it.
pub unsafe fn mmap(
    addr: u64,
    len: u64,
    prot: u32,
    flags: u32,
    fd: i32,
    offset: u64,
) -> Result<u64, i32> {
    let ret = unsafe {
        syscall6(
            SYS_MMAP,
            addr as usize,
            len as usize,
            prot as usize,
            flags as usize,
            fd as isize as usize,
            offset as usize,
        )
    };
    check(ret).map(|v| v as u64)
}

/// # Safety
/// The caller must own the range and no live reference may point into it.
pub unsafe fn munmap(addr: u64, len: u64) -> Result<(), i32> {
    check(unsafe { syscall2(SYS_MUNMAP, addr as usize, len as usize) }).map(|_| ())
}

/// # Safety
/// The caller must own the range.
pub unsafe fn mprotect(addr: u64, len: u64, prot: u32) -> Result<(), i32> {
    check(unsafe { syscall3(SYS_MPROTECT, addr as usize, len as usize, prot as usize) }).map(|_| ())
}

/// # Safety
/// The caller must own both the old range and, with `MREMAP_FIXED`, the new one.
pub unsafe fn mremap(
    old: u64,
    old_len: u64,
    new_len: u64,
    flags: u32,
    new: u64,
) -> Result<u64, i32> {
    let ret = unsafe {
        syscall5(
            SYS_MREMAP,
            old as usize,
            old_len as usize,
            new_len as usize,
            flags as usize,
            new as usize,
        )
    };
    check(ret).map(|v| v as u64)
}

pub fn getuid() -> u64 {
    unsafe { syscall0(SYS_GETUID) as u64 }
}

pub fn geteuid() -> u64 {
    unsafe { syscall0(SYS_GETEUID) as u64 }
}

pub fn getgid() -> u64 {
    unsafe { syscall0(SYS_GETGID) as u64 }
}

pub fn getegid() -> u64 {
    unsafe { syscall0(SYS_GETEGID) as u64 }
}

/// Fills `buf` from the kernel random source, retrying short reads and `EINTR`.
pub fn getrandom(buf: &mut [u8]) -> Result<(), i32> {
    let mut done = 0;
    while done < buf.len() {
        let rest = &mut buf[done..];
        let ret = unsafe { syscall3(SYS_GETRANDOM, rest.as_mut_ptr() as usize, rest.len(), 0) };
        match check(ret) {
            Ok(n) => done += n,
            Err(EINTR) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn nul_terminated(s: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(s.len() + 1);
    v.extend_from_slice(s);
    v.push(0);
    v
}

/// Whether `path` may be executed by the effective ids: the lookup and permission errors
/// `execve` gives (`ENOENT`, `ENOTDIR`, `EACCES`, `ELOOP`…).
pub fn access_exec(path: &[u8]) -> Result<(), i32> {
    let path = nul_terminated(path);
    let ret = unsafe {
        syscall4(
            SYS_FACCESSAT2,
            AT_FDCWD as isize as usize,
            path.as_ptr() as usize,
            X_OK,
            AT_EACCESS,
        )
    };
    check(ret).map(|_| ())
}

/// Replaces this process by `path` with `argv` and `envp`. Returns only on failure,
/// with the errno.
pub fn execve(path: &[u8], argv: &[Vec<u8>], envp: &[Vec<u8>]) -> i32 {
    let path = nul_terminated(path);
    let argv: Vec<Vec<u8>> = argv.iter().map(|s| nul_terminated(s)).collect();
    let envp: Vec<Vec<u8>> = envp.iter().map(|s| nul_terminated(s)).collect();
    let table = |v: &[Vec<u8>]| -> Vec<usize> {
        v.iter().map(|s| s.as_ptr() as usize).chain([0]).collect()
    };
    let (argv_table, envp_table) = (table(&argv), table(&envp));
    let ret = unsafe {
        syscall3(
            SYS_EXECVE,
            path.as_ptr() as usize,
            argv_table.as_ptr() as usize,
            envp_table.as_ptr() as usize,
        )
    };
    check(ret).err().unwrap_or(EINVAL)
}

/// `struct sigaction` as `rt_sigaction` takes it on x86-64.
#[repr(C)]
struct KernelSigaction {
    handler: usize,
    flags: u64,
    restorer: usize,
    mask: u64,
}

/// Dies by `sig` as its default action does: restores `SIG_DFL`, unblocks it and sends it
/// to this thread, so the process ends by the same signal as natively.
pub fn die_by_signal(sig: i32) -> ! {
    let action = KernelSigaction {
        handler: SIG_DFL,
        flags: 0,
        restorer: 0,
        mask: 0,
    };
    let set: u64 = 1 << (sig - 1);
    unsafe {
        syscall4(
            SYS_RT_SIGACTION,
            sig as usize,
            &action as *const KernelSigaction as usize,
            0,
            8,
        );
        syscall4(
            SYS_RT_SIGPROCMASK,
            SIG_UNBLOCK,
            &set as *const u64 as usize,
            0,
            8,
        );
        let pid = syscall0(SYS_GETPID) as usize;
        let tid = syscall0(SYS_GETTID) as usize;
        syscall3(SYS_TGKILL, pid, tid, sig as usize);
        // Only if the signal did not kill: the exit status a shell shows for it.
        syscall1(SYS_EXIT_GROUP, 128 + sig as usize);
    }
    unreachable!("exit_group returned")
}

/// Points the GS base of this thread at `base` (`wrgsbase`, FSGSBASE is enabled).
///
/// # Safety
/// Nothing else in the thread may rely on the GS base.
pub unsafe fn set_gs_base(base: u64) {
    unsafe { asm!("wrgsbase {}", in(reg) base, options(nostack, preserves_flags)) };
}
