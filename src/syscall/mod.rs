//! Guest system calls (contract 3.10). The `syscall` exits to the dispatcher and the
//! number decides: calls of the pass list go straight to the kernel with the same
//! arguments (identity mapping makes guest pointers valid), memory calls go through
//! `mem.rs` without `PROT_EXEC`, and anything off the list returns `-ENOSYS`.
//!
//! `execve` runs calco again on the new program; `fork`, `vfork` and `clone` without
//! `CLONE_VM` fork the host process, and the child goes on in the dispatcher. `clone` with
//! `CLONE_VM|CLONE_THREAD` starts a guest thread (`thread.rs`); `exit` of a thread is served
//! by the dispatcher.
//!
//! Every call that reaches the kernel goes through the safe system call of `signal/`, so
//! a guest signal that arrives before the `syscall` skips it and the call runs again after
//! the signal, or ends with `EINTR`, as `SA_RESTART` says (contract 3.11). The signal
//! calls themselves are in `signal/`.
//!
//! With threads (contract 3.12), `handle` serves the emulated calls with the process lock
//! held, except the ones that wait (`wait4`, `waitid`, `rt_sigsuspend`,
//! `rt_sigtimedwait`): they take it only to read or write guest memory, since the thread
//! they wait for may need it. Pass-through calls take no lock.

mod table;

pub use self::table::{EXIT, Kind, classify};

use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use self::table::*;
use crate::cpu::{Cpu, R8, R9, R10, R11, RAX, RCX, RDI, RDX, RSI, RSP};
use crate::loader;
use crate::mem::{Mem, USER_END, host_prot};
use crate::signal::{self, TimedWait, guest_signal, host_signal};
use crate::smc;
use crate::sys::{
    self, AT_EMPTY_PATH, AT_FDCWD, AT_SYMLINK_NOFOLLOW, E2BIG, EFAULT, EINVAL, ELOOP, ENAMETOOLONG,
    ENOSYS, EPERM, MAP_ANONYMOUS, PROT_EXEC, PROT_READ, PROT_WRITE,
};
use crate::thread::{
    self, CLONE_CHILD_CLEARTID, CLONE_SETTLS, CLONE_VM, CSIGNAL, CloneKind, Member,
};

// /usr/include/asm/prctl.h
const ARCH_SET_GS: u64 = 0x1001;
const ARCH_SET_FS: u64 = 0x1002;
const ARCH_GET_FS: u64 = 0x1003;
const ARCH_GET_GS: u64 = 0x1004;
// /usr/include/linux/prctl.h
const PR_SET_PDEATHSIG: u64 = 1;
const PR_SET_NAME: u64 = 15;
const PR_GET_NAME: u64 = 16;
/// `sizeof(struct robust_list_head)` on x86-64.
const ROBUST_LIST_HEAD_SIZE: u64 = 24;
/// Longest path read from the guest (`PATH_MAX`).
const PATH_MAX: u64 = 4096;
/// Longest argument or environment string `execve` takes (`MAX_ARG_STRLEN`).
const MAX_ARG_STRLEN: u64 = 32 * 4096;
const SIGCHLD: u64 = 17;
// /usr/include/asm-generic/siginfo.h: `si_code` of SIGCHLD whose `si_status` is a signal.
const CLD_KILLED: i32 = 2;
const CLD_DUMPED: i32 = 3;
const CLD_TRAPPED: i32 = 4;
const CLD_STOPPED: i32 = 5;
/// Offsets of `si_code` and `si_status` in the x86-64 `siginfo_t` of SIGCHLD.
const SI_CODE: u64 = 8;
const SI_STATUS: u64 = 24;
// /usr/include/asm-generic/mman-common.h
const MADV_DONTNEED: u64 = 4;
// /usr/include/asm-generic/fcntl.h
const F_GETFD: u64 = 1;
const FCNTL: u64 = 72;

/// Process state the emulated calls keep, shared by the guest threads.
pub struct State {
    /// Absolute path of the guest program, what `/proc/self/exe` reads as.
    exe: Vec<u8>,
    /// Host path of calco itself, read from `/proc/self/exe` at start; `execve` runs it.
    calco: Vec<u8>,
    /// File named by `CALCO_LOG`, where unsupported calls are noted.
    log: Option<PathBuf>,
    /// Guest signal actions.
    pub signals: signal::Process,
    /// The last `set_robust_list` of any thread.
    pub robust_list: (u64, u64),
}

fn err(errno: i32) -> i64 {
    -(errno as i64)
}

fn result(r: Result<u64, i32>) -> i64 {
    r.map_or_else(err, |v| v as i64)
}

/// Straight to the kernel through the safe system call (contract 3.11).
fn pass(nr: u64, a: [u64; 6]) -> i64 {
    signal::safe_syscall(nr, a)
}

/// Whether the guest owns `[addr, addr + len)` with read (or write) permission, so calco
/// can access it without faulting.
pub fn accessible(mem: &Mem, addr: u64, len: u64, write: bool) -> bool {
    let Some(end) = addr.checked_add(len) else {
        return false;
    };
    let mut at = addr;
    while at < end {
        let Some(r) = mem.intervals.find(at) else {
            return false;
        };
        let need = if write { PROT_WRITE } else { PROT_READ };
        if host_prot(r.prot) & need == 0 {
            return false;
        }
        at = r.end;
    }
    true
}

pub fn read_u64(mem: &Mem, addr: u64) -> Result<u64, i32> {
    if !accessible(mem, addr, 8, false) {
        return Err(EFAULT);
    }
    Ok(unsafe { (addr as *const u64).read_unaligned() })
}

pub fn read_bytes(mem: &Mem, addr: u64, len: u64) -> Result<Vec<u8>, i32> {
    if !accessible(mem, addr, len, false) {
        return Err(EFAULT);
    }
    let mut out = vec![0u8; len as usize];
    unsafe { core::ptr::copy_nonoverlapping(addr as *const u8, out.as_mut_ptr(), out.len()) };
    Ok(out)
}

pub fn write_bytes(mem: &Mem, addr: u64, bytes: &[u8]) -> Result<(), i32> {
    if !accessible(mem, addr, bytes.len() as u64, true) {
        return Err(EFAULT);
    }
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), addr as *mut u8, bytes.len()) };
    Ok(())
}

fn write_u64(mem: &Mem, addr: u64, v: u64) -> Result<(), i32> {
    write_bytes(mem, addr, &v.to_le_bytes())
}

fn read_u32(mem: &Mem, addr: u64) -> Result<u32, i32> {
    if !accessible(mem, addr, 4, false) {
        return Err(EFAULT);
    }
    Ok(unsafe { (addr as *const u32).read_unaligned() })
}

fn write_u32(mem: &Mem, addr: u64, v: u32) -> Result<(), i32> {
    write_bytes(mem, addr, &v.to_le_bytes())
}

/// The NUL-terminated string at `addr`, NUL included in `max`: `EFAULT` if it leaves
/// readable guest memory, `too_long` if it does not end in time.
fn read_string(mem: &Mem, addr: u64, max: u64, too_long: i32) -> Result<Vec<u8>, i32> {
    let mut out = Vec::new();
    let mut at = addr;
    while out.len() < max as usize {
        let r = mem.intervals.find(at).ok_or(EFAULT)?;
        if host_prot(r.prot) & PROT_READ == 0 {
            return Err(EFAULT);
        }
        while at < r.end && out.len() < max as usize {
            let b = unsafe { (at as *const u8).read() };
            if b == 0 {
                return Ok(out);
            }
            out.push(b);
            at += 1;
        }
    }
    Err(too_long)
}

/// The path at `addr`, if it is readable guest memory.
fn read_cstr(mem: &Mem, addr: u64) -> Option<Vec<u8>> {
    read_string(mem, addr, PATH_MAX, ENAMETOOLONG).ok()
}

/// The NULL-terminated string array at `addr` (`argv`, `envp`); a NULL array is empty.
fn read_strings(mem: &Mem, addr: u64) -> Result<Vec<Vec<u8>>, i32> {
    let mut out = Vec::new();
    if addr == 0 {
        return Ok(out);
    }
    loop {
        let at = addr.checked_add(8 * out.len() as u64).ok_or(EFAULT)?;
        match read_u64(mem, at)? {
            0 => return Ok(out),
            p => out.push(read_string(mem, p, MAX_ARG_STRLEN, E2BIG)?),
        }
    }
}

fn is_proc_self_exe(path: &[u8]) -> bool {
    path == b"/proc/self/exe" || path == b"/proc/thread-self/exe"
}

/// Host argv that runs calco again on `path` with the guest `argv` (contract 3.10). An
/// empty guest argv gets an empty argv[0], as the kernel gives it.
pub fn reexec_argv(calco: &[u8], path: &[u8], argv: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let mut out = vec![
        calco.to_vec(),
        b"--argv0".to_vec(),
        argv.first().cloned().unwrap_or_default(),
        b"--".to_vec(),
        path.to_vec(),
    ];
    out.extend(argv.iter().skip(1).cloned());
    out
}

/// `wait4` status as the guest sees it: the terminating or stopping signal translated,
/// the rest (exit code, core flag, ptrace event) as it is.
pub fn guest_wait_status(status: u32) -> u32 {
    if status & 0xff == 0x7f {
        let sig = (status >> 8) & 0xff;
        (status & !0xff00) | (guest_signal(sig) << 8)
    } else if status & 0x7f != 0 {
        (status & !0x7f) | guest_signal(status & 0x7f)
    } else {
        status
    }
}

/// `si_status` of a `waitid` result as the guest sees it, given its `si_code`.
pub fn guest_waitid_status(code: i32, status: i32) -> i32 {
    match code {
        CLD_KILLED | CLD_DUMPED | CLD_TRAPPED | CLD_STOPPED => guest_signal(status as u32) as i32,
        _ => status,
    }
}

fn gettid() -> u64 {
    unsafe { sys::syscall0(sys::SYS_GETTID) as u64 }
}

impl State {
    pub fn new(exe: Vec<u8>, calco: Vec<u8>, log: Option<PathBuf>) -> State {
        State {
            exe,
            calco,
            log,
            signals: signal::Process::new(),
            robust_list: (0, 0),
        }
    }

    /// Serves call `nr` with arguments `a` and returns what the guest gets in RAX.
    pub fn dispatch(&mut self, cpu: &mut Cpu, mem: &mut Mem, nr: u64, a: [u64; 6]) -> i64 {
        match classify(nr) {
            Kind::Pass => pass(nr, a),
            Kind::Emulated => self.emulate(cpu, mem, nr, a),
            Kind::Unlisted => self.unsupported(cpu, nr),
        }
    }

    fn emulate(&mut self, cpu: &mut Cpu, mem: &mut Mem, nr: u64, a: [u64; 6]) -> i64 {
        match nr {
            BRK => mem.brk(a[0]) as i64,
            MMAP => self.mmap(mem, a),
            MUNMAP => result(mem.unmap(a[0], a[1]).map(|()| 0)),
            MPROTECT => {
                let prot = u32::try_from(a[2]).map_err(|_| EINVAL);
                result(prot.and_then(|p| mem.protect(a[0], a[1], p)).map(|()| 0))
            }
            MREMAP => {
                let flags = u32::try_from(a[3]).map_err(|_| EINVAL);
                result(flags.and_then(|f| mem.remap(a[0], a[1], a[2], f, a[4])))
            }
            ARCH_PRCTL => arch_prctl(cpu, mem, a[0], a[1]),
            SET_TID_ADDRESS => {
                cpu.clear_child_tid = a[0];
                cpu.tid = gettid();
                cpu.tid as i64
            }
            SET_ROBUST_LIST => {
                if a[1] != ROBUST_LIST_HEAD_SIZE {
                    return err(EINVAL);
                }
                self.robust_list = (a[0], a[1]);
                0
            }
            GET_ROBUST_LIST => {
                if a[0] != 0 && a[0] != gettid() {
                    return err(EPERM);
                }
                let (head, len) = self.robust_list;
                result(
                    write_u64(mem, a[1], head)
                        .and_then(|()| write_u64(mem, a[2], len))
                        .map(|()| 0),
                )
            }
            // glibc turns rseq off and falls back from clone3 to clone.
            RSEQ | CLONE3 => err(ENOSYS),
            // The dispatcher serves the `exit` of a guest thread before it gets here.
            EXIT | EXIT_GROUP => pass(nr, a),
            RT_SIGACTION => self.signals.sigaction(cpu, mem, a),
            RT_SIGPROCMASK => signal::sigprocmask(cpu, mem, a[0], a[1], a[2], a[3]),
            RT_SIGPENDING => signal::sigpending(cpu, mem, a[0], a[1]),
            RT_SIGSUSPEND => signal::sigsuspend(cpu, mem, a[0], a[1]),
            RT_SIGTIMEDWAIT => signal::sigtimedwait(cpu, mem, a),
            RT_SIGQUEUEINFO | RT_TGSIGQUEUEINFO => signal::sigqueueinfo(mem, nr, a),
            SIGALTSTACK => signal::sigaltstack(cpu, mem, a[0], a[1]),
            SIGNALFD4 => signal::signalfd4(mem, nr, a),
            TIMER_CREATE => signal::timer_create(mem, nr, a),
            KILL | TKILL => match host_signal(a[1]) {
                Some(sig) => pass(nr, [a[0], sig, 0, 0, 0, 0]),
                None => err(EINVAL),
            },
            TGKILL => match host_signal(a[2]) {
                Some(sig) => pass(nr, [a[0], a[1], sig, 0, 0, 0]),
                None => err(EINVAL),
            },
            READLINK => self.readlink(mem, nr, a, a[0], a[1], a[2]),
            READLINKAT => self.readlink(mem, nr, a, a[1], a[2], a[3]),
            CLONE => match thread::clone_kind(a[0]) {
                // vfork as fork (contract 3.10): the child gets a copy of the memory.
                Ok(CloneKind::Fork) => fork(cpu, a[0] & !CLONE_VM, a[1], a[2], a[3], a[4]),
                Ok(CloneKind::Thread) => thread::clone(cpu, mem, a),
                Err(e) => err(e),
            },
            FORK | VFORK => fork(cpu, SIGCHLD, 0, 0, 0, 0),
            EXECVE => match read_string(mem, a[0], PATH_MAX, ENAMETOOLONG) {
                Ok(path) => self.execve(mem, path, a[1], a[2]),
                Err(e) => err(e),
            },
            EXECVEAT => self.execveat(mem, a),
            WAIT4 | WAITID => {
                let ret = pass(nr, a);
                wait_result(mem, nr, a, ret)
            }
            // Translations of pages dropped by MADV_DONTNEED went before (`invalidated`).
            MADVISE => pass(nr, a),
            PRCTL => match a[0] {
                PR_SET_PDEATHSIG | PR_SET_NAME | PR_GET_NAME => pass(nr, a),
                _ => err(EINVAL),
            },
            _ => self.unsupported(cpu, nr),
        }
    }

    fn mmap(&mut self, mem: &mut Mem, a: [u64; 6]) -> i64 {
        let [addr, len, prot, flags, fd, offset] = a;
        let (Ok(prot), Ok(flags)) = (u32::try_from(prot), u32::try_from(flags)) else {
            return err(EINVAL);
        };
        let r = if flags & MAP_ANONYMOUS != 0 {
            mem.map_anon(addr, len, prot, flags)
        } else {
            mem.map_file(addr, len, prot, flags, fd as i32, offset)
        };
        result(r)
    }

    /// `execve` of `path`: if the kernel would run it, calco runs itself again on it with
    /// the guest environment (descriptors stay as they are); otherwise the guest gets the
    /// errno the kernel gives and goes on.
    fn execve(&self, mem: &Mem, path: Vec<u8>, argv: u64, envp: u64) -> i64 {
        // In the new process /proc/self/exe is calco: the guest meant its own program.
        let path = if is_proc_self_exe(&path) {
            self.exe.clone()
        } else {
            path
        };
        // Same order as the kernel: the file first, then the argument and environment.
        if let Err(e) = loader::resolve(Path::new(OsStr::from_bytes(&path)), Vec::new()) {
            return err(e);
        }
        let (argv, envp) = match (read_strings(mem, argv), read_strings(mem, envp)) {
            (Ok(argv), Ok(envp)) => (argv, envp),
            (Err(e), _) | (_, Err(e)) => return err(e),
        };
        let host_argv = reexec_argv(&self.calco, &path, &argv);
        err(sys::execve(&self.calco, &host_argv, &envp))
    }

    /// `execveat`: the path relative to `dirfd` becomes one under `/proc/self/fd`, which
    /// the new process still has open.
    fn execveat(&self, mem: &Mem, a: [u64; 6]) -> i64 {
        let [dirfd, path, argv, envp, flags, _] = a;
        if flags & !(AT_SYMLINK_NOFOLLOW | AT_EMPTY_PATH) != 0 {
            return err(EINVAL);
        }
        let path = match read_string(mem, path, PATH_MAX, ENAMETOOLONG) {
            Ok(p) => p,
            Err(e) => return err(e),
        };
        let dirfd = dirfd as i32;
        let relative = !path.starts_with(b"/") && dirfd != AT_FDCWD;
        // calco's own check, not the guest's call: a pending signal must not make it fail.
        if relative
            && unsafe { sys::syscall2(FCNTL as usize, dirfd as isize as usize, F_GETFD as usize) }
                < 0
        {
            return err(sys::EBADF);
        }
        let full = if !relative {
            path
        } else if path.is_empty() && flags & AT_EMPTY_PATH != 0 {
            format!("/proc/self/fd/{dirfd}").into_bytes()
        } else if path.is_empty() {
            path
        } else {
            let mut full = format!("/proc/self/fd/{dirfd}/").into_bytes();
            full.extend_from_slice(&path);
            full
        };
        if flags & AT_SYMLINK_NOFOLLOW != 0
            && std::fs::symlink_metadata(OsStr::from_bytes(&full))
                .is_ok_and(|m| m.file_type().is_symlink())
        {
            return err(ELOOP);
        }
        self.execve(mem, full, argv, envp)
    }

    /// `readlink`/`readlinkat`: `/proc/self/exe` reads as the guest program, not calco;
    /// any other path goes to the kernel.
    fn readlink(&self, mem: &Mem, nr: u64, a: [u64; 6], path: u64, buf: u64, size: u64) -> i64 {
        let is_exe = read_cstr(mem, path).is_some_and(|p| is_proc_self_exe(&p));
        if !is_exe {
            return pass(nr, a);
        }
        let size = size as i32;
        if size <= 0 {
            return err(EINVAL);
        }
        let n = self.exe.len().min(size as usize);
        result(write_bytes(mem, buf, &self.exe[..n]).map(|()| n as u64))
    }

    fn unsupported(&self, cpu: &Cpu, nr: u64) -> i64 {
        if let Some(path) = &self.log
            && let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path)
        {
            // Opened per call: the guest may close any descriptor calco keeps.
            let _ = writeln!(f, "calco: syscall {nr} at rip {:#x}: ENOSYS", cpu.rip);
        }
        err(ENOSYS)
    }
}

/// `wait4` and `waitid` once the kernel answered `ret`: the signal of the status as the
/// guest numbers it.
fn wait_result(mem: &Mem, nr: u64, a: [u64; 6], ret: i64) -> i64 {
    if nr == WAIT4 {
        if ret > 0
            && a[1] != 0
            && let Ok(status) = read_u32(mem, a[1])
        {
            let _ = write_u32(mem, a[1], guest_wait_status(status));
        }
        return ret;
    }
    let (code, status) = (a[2].wrapping_add(SI_CODE), a[2].wrapping_add(SI_STATUS));
    if ret == 0
        && a[2] != 0
        && let (Ok(c), Ok(st)) = (read_u32(mem, code), read_u32(mem, status))
    {
        let st = guest_waitid_status(c as i32, st as i32);
        let _ = write_u32(mem, status, st as u32);
    }
    ret
}

/// `fork`, `vfork` and `clone` without `CLONE_VM`: the host process forks and the child
/// goes on in the dispatcher with `rax = 0`. The child's stack and TLS are guest state, so
/// they go to its `Cpu` instead of to the host; the exit signal is translated.
fn fork(cpu: &mut Cpu, flags: u64, stack: u64, ptid: u64, ctid: u64, tls: u64) -> i64 {
    let Some(exit_signal) = host_signal(flags & CSIGNAL) else {
        return err(EINVAL);
    };
    let host_flags = (flags & !(CSIGNAL | CLONE_SETTLS)) | exit_signal;
    // No new stack: the host child returns here on its copy of calco's stack.
    let args = [host_flags, 0, ptid, ctid, 0, 0];
    let ret = pass(sys::SYS_CLONE as u64, args);
    if ret == 0 {
        // As in the kernel, the child starts with no pending signals.
        signal::forked(cpu);
        cpu.tid = gettid();
        if stack != 0 {
            cpu.gpr[RSP] = stack;
        }
        if flags & CLONE_SETTLS != 0 {
            cpu.fs_base = tls;
        }
        if flags & CLONE_CHILD_CLEARTID != 0 {
            cpu.clear_child_tid = ctid;
        }
    }
    ret
}

fn arch_prctl(cpu: &mut Cpu, mem: &Mem, code: u64, addr: u64) -> i64 {
    match code {
        ARCH_SET_FS | ARCH_SET_GS if addr >= USER_END => err(EPERM),
        ARCH_SET_FS => {
            cpu.fs_base = addr;
            0
        }
        ARCH_SET_GS => {
            cpu.gs_base = addr;
            0
        }
        ARCH_GET_FS => result(write_u64(mem, addr, cpu.fs_base).map(|()| 0)),
        ARCH_GET_GS => result(write_u64(mem, addr, cpu.gs_base).map(|()| 0)),
        // ARCH_SHSTK_* included: CPUID does not announce CET.
        _ => err(EINVAL),
    }
}

/// Serves the `syscall` the guest thread of `cpu` (`me` in the thread list) just executed;
/// `cpu.rip` is already past it.
pub fn handle(cpu: &mut Cpu, me: &Arc<Member>) {
    let shared = thread::shared();
    let nr = cpu.gpr[RAX];
    if nr == RT_SIGRETURN {
        // The whole state comes from the frame, RCX and R11 included.
        let mut world = shared.world();
        let world = &mut *world;
        world.state.signals.sigreturn(cpu, &world.mem);
        return;
    }
    let args = [RDI, RSI, RDX, R10, R8, R9].map(|r| cpu.gpr[r]);
    // The guard of a `let` goes at the end of its statement: the waits hold no lock.
    let ret = match (classify(nr), nr) {
        (Kind::Pass, _) => pass(nr, args),
        (_, WAIT4 | WAITID) => {
            let ret = pass(nr, args);
            wait_result(&shared.world().mem, nr, args, ret)
        }
        (_, RT_SIGSUSPEND) => {
            let set = signal::read_set(&shared.world().mem, args[0], args[1]);
            set.map_or_else(|e| e, |s| signal::suspend(cpu, s))
        }
        (_, RT_SIGTIMEDWAIT) => {
            let first = signal::timedwait_queued(cpu, &shared.world().mem, args);
            match first {
                TimedWait::Done(ret) => ret,
                TimedWait::Host(host) => {
                    let ret = signal::timedwait_host(host, args);
                    signal::timedwait_info(&shared.world().mem, args, ret)
                }
            }
        }
        _ => locked(cpu, me, nr, args),
    };
    // An interrupted call waits for `signal::deliver` to restart it or leave EINTR.
    let ret = signal::note_result(cpu, nr, ret);
    cpu.gpr[RAX] = ret as u64;
    // What the `syscall` instruction leaves: the return address in RCX, RFLAGS in R11.
    cpu.gpr[RCX] = cpu.rip;
    cpu.gpr[R11] = cpu.rflags;
}

/// The guest ranges whose translations go before memory call `nr` runs (contract 3.7,
/// 3.10): what `munmap`, `mremap` (its old range, and the target of `MREMAP_FIXED`), a
/// fixed `mmap`, `mprotect` without execute and `madvise(MADV_DONTNEED)` touch. Going
/// first, write-protected pages get their host protection back before they move or change.
pub fn invalidated(nr: u64, a: [u64; 6]) -> Vec<core::ops::Range<u64>> {
    let drops = match nr {
        MUNMAP | MREMAP => true,
        MMAP => a[3] & sys::MAP_FIXED as u64 != 0,
        MPROTECT => a[2] & PROT_EXEC as u64 == 0,
        MADVISE => a[2] == MADV_DONTNEED,
        _ => false,
    };
    let mut out = Vec::new();
    if drops {
        out.extend(smc::span(a[0], a[1]));
    }
    if nr == MREMAP && a[3] & sys::MREMAP_FIXED as u64 != 0 {
        out.extend(smc::span(a[4], a[2]));
    }
    out
}

/// The range of an `mprotect` that keeps execute, whose pages with translations the
/// guest may now write: they lose write in the host again after the call.
pub fn reprotected(nr: u64, a: [u64; 6]) -> Option<(core::ops::Range<u64>, u32)> {
    if nr != MPROTECT || a[2] & PROT_EXEC as u64 == 0 {
        return None;
    }
    Some((smc::span(a[0], a[1])?, u32::try_from(a[2]).ok()?))
}

/// An emulated call served with the process lock held. A call that may change guest code
/// takes the cache lock first and invalidates the translations of the range it touches
/// before it runs, so no thread enters a translation of code that is gone; `fork` takes
/// the cache lock and the thread list too, so the child finds every lock of calco free.
fn locked(cpu: &mut Cpu, me: &Arc<Member>, nr: u64, a: [u64; 6]) -> i64 {
    let shared = thread::shared();
    let forks = matches!(nr, FORK | VFORK)
        || (nr == CLONE && thread::clone_kind(a[0]) == Ok(CloneKind::Fork));
    let drops = invalidated(nr, a);
    let keeps = reprotected(nr, a);
    let mut jit = (forks || !drops.is_empty() || keeps.is_some()).then(|| shared.jit());
    if let Some(jit) = jit.as_mut() {
        for range in drops {
            jit.invalidate_range(range);
        }
    }
    let mut world = shared.world();
    let mut threads = forks.then(|| shared.threads());
    let w = &mut *world;
    let ret = w.state.dispatch(cpu, &mut w.mem, nr, a);
    if ret == 0
        && let Some(threads) = threads.as_mut()
    {
        thread::forked(threads, me);
    }
    if ret == 0
        && let (Some(jit), Some((range, prot))) = (jit.as_mut(), keeps)
    {
        jit.reprotect(range, prot);
    }
    ret
}

#[cfg(test)]
mod tests;
