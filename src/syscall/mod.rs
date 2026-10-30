//! Guest system calls (contract 3.10). The `syscall` exits to the dispatcher and the
//! number decides: calls of the pass list go straight to the kernel with the same
//! arguments (identity mapping makes guest pointers valid), memory calls go through
//! `mem.rs` without `PROT_EXEC`, and anything off the list returns `-ENOSYS`.
//!
//! For now `rt_sigaction` and `rt_sigprocmask` only keep the guest state; nothing is
//! installed in the host (k2-02). The emulated calls that belong to K2 (threads, signal
//! delivery, `execve`) also return `-ENOSYS` until then.

mod table;

pub use self::table::{Kind, classify};

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use self::table::*;
use crate::cpu::{Cpu, R8, R9, R10, R11, RAX, RCX, RDI, RDX, RSI};
use crate::mem::{Mem, USER_END, host_prot};
use crate::sys::{
    self, EFAULT, EINVAL, ENOSYS, EPERM, MAP_ANONYMOUS, PROT_EXEC, PROT_READ, PROT_WRITE,
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
// /usr/include/asm-generic/signal-defs.h
const SIG_BLOCK: u64 = 0;
const SIG_UNBLOCK: u64 = 1;
const SIG_SETMASK: u64 = 2;
const SIGKILL: u64 = 9;
const SIGSTOP: u64 = 19;
/// Guest signals with an action: 62–64 are reserved (contract 3.11).
const GUEST_SIGNALS: u64 = 61;
const UNBLOCKABLE: u64 = (1 << (SIGKILL - 1)) | (1 << (SIGSTOP - 1));
/// `sizeof(struct robust_list_head)` on x86-64.
const ROBUST_LIST_HEAD_SIZE: u64 = 24;
/// Longest path read from the guest (`PATH_MAX`).
const PATH_MAX: u64 = 4096;

/// Guest `struct sigaction` as `rt_sigaction` takes it on x86-64.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SigAction {
    pub handler: u64,
    pub flags: u64,
    pub restorer: u64,
    pub mask: u64,
}

/// Process state the emulated calls keep.
pub struct State {
    /// Absolute path of the guest program, what `/proc/self/exe` reads as.
    exe: Vec<u8>,
    /// File named by `CALCO_LOG`, where unsupported calls are noted.
    log: Option<PathBuf>,
    pub actions: [SigAction; GUEST_SIGNALS as usize],
    pub sigmask: u64,
    pub robust_list: (u64, u64),
    /// Set when guest code may have been unmapped, replaced or lost execute permission.
    code_changed: bool,
}

fn err(errno: i32) -> i64 {
    -(errno as i64)
}

fn result(r: Result<u64, i32>) -> i64 {
    r.map_or_else(err, |v| v as i64)
}

fn pass(nr: u64, a: [u64; 6]) -> i64 {
    let [a1, a2, a3, a4, a5, a6] = a.map(|v| v as usize);
    // Identity mapping: guest pointers are valid as they are; the kernel checks them.
    unsafe { sys::syscall6(nr as usize, a1, a2, a3, a4, a5, a6) as i64 }
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

fn read_u64(mem: &Mem, addr: u64) -> Result<u64, i32> {
    if !accessible(mem, addr, 8, false) {
        return Err(EFAULT);
    }
    Ok(unsafe { (addr as *const u64).read_unaligned() })
}

fn write_bytes(mem: &Mem, addr: u64, bytes: &[u8]) -> Result<(), i32> {
    if !accessible(mem, addr, bytes.len() as u64, true) {
        return Err(EFAULT);
    }
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), addr as *mut u8, bytes.len()) };
    Ok(())
}

fn write_u64(mem: &Mem, addr: u64, v: u64) -> Result<(), i32> {
    write_bytes(mem, addr, &v.to_le_bytes())
}

/// The NUL-terminated string at `addr`, if it is readable guest memory.
fn read_cstr(mem: &Mem, addr: u64) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut at = addr;
    while out.len() < PATH_MAX as usize {
        let r = mem.intervals.find(at)?;
        if host_prot(r.prot) & PROT_READ == 0 {
            return None;
        }
        while at < r.end && out.len() < PATH_MAX as usize {
            let b = unsafe { (at as *const u8).read() };
            if b == 0 {
                return Some(out);
            }
            out.push(b);
            at += 1;
        }
    }
    None
}

/// Host number of guest signal `sig` (contract 3.11): 32 and 33 move to 64 and 63; 0 is
/// the existence check; 62–64 are not available.
pub fn host_signal(sig: u64) -> Option<u64> {
    match sig {
        0..=31 | 34..=61 => Some(sig),
        32 => Some(64),
        33 => Some(63),
        _ => None,
    }
}

fn gettid() -> u64 {
    unsafe { sys::syscall0(sys::SYS_GETTID) as u64 }
}

impl State {
    pub fn new(exe: Vec<u8>, log: Option<PathBuf>) -> State {
        State {
            exe,
            log,
            actions: [SigAction::default(); GUEST_SIGNALS as usize],
            sigmask: 0,
            robust_list: (0, 0),
            code_changed: false,
        }
    }

    /// Whether guest code changed since the last call; the dispatcher then empties the
    /// code cache.
    pub fn take_code_changed(&mut self) -> bool {
        std::mem::take(&mut self.code_changed)
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
            MUNMAP => {
                self.note_code_change(mem, a[0], a[1]);
                result(mem.unmap(a[0], a[1]).map(|()| 0))
            }
            MPROTECT => {
                if a[2] & PROT_EXEC as u64 == 0 {
                    self.note_code_change(mem, a[0], a[1]);
                }
                let prot = u32::try_from(a[2]).map_err(|_| EINVAL);
                result(prot.and_then(|p| mem.protect(a[0], a[1], p)).map(|()| 0))
            }
            MREMAP => {
                self.note_code_change(mem, a[0], a[1]);
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
            EXIT | EXIT_GROUP => pass(nr, a),
            RT_SIGACTION => self.sigaction(mem, a[0], a[1], a[2], a[3]),
            RT_SIGPROCMASK => self.sigprocmask(mem, a[0], a[1], a[2], a[3]),
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
        // A fixed mapping may replace guest code.
        if flags & sys::MAP_FIXED != 0 {
            self.note_code_change(mem, addr, len);
        }
        let r = if flags & MAP_ANONYMOUS != 0 {
            mem.map_anon(addr, len, prot, flags)
        } else {
            mem.map_file(addr, len, prot, flags, fd as i32, offset)
        };
        result(r)
    }

    /// Marks the code cache stale if `[addr, addr + len)` touches guest code.
    fn note_code_change(&mut self, mem: &Mem, addr: u64, len: u64) {
        let end = addr.saturating_add(len).min(USER_END);
        if addr < end
            && mem
                .intervals
                .overlapping(addr, end)
                .iter()
                .any(|r| r.prot & PROT_EXEC != 0)
        {
            self.code_changed = true;
        }
    }

    fn sigaction(&mut self, mem: &Mem, sig: u64, act: u64, oact: u64, size: u64) -> i64 {
        if size != 8 || !(1..=GUEST_SIGNALS).contains(&sig) {
            return err(EINVAL);
        }
        let new = if act != 0 {
            let field = |i: u64| read_u64(mem, act + 8 * i);
            let action: Result<SigAction, i32> = (|| {
                Ok(SigAction {
                    handler: field(0)?,
                    flags: field(1)?,
                    restorer: field(2)?,
                    mask: field(3)? & !UNBLOCKABLE,
                })
            })();
            match action {
                Ok(_) if sig == SIGKILL || sig == SIGSTOP => return err(EINVAL),
                Ok(action) => Some(action),
                Err(e) => return err(e),
            }
        } else {
            None
        };
        let slot = &mut self.actions[sig as usize - 1];
        let old = *slot;
        if let Some(action) = new {
            *slot = action;
        }
        if oact != 0 {
            let words = [old.handler, old.flags, old.restorer, old.mask];
            let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
            if let Err(e) = write_bytes(mem, oact, &bytes) {
                return err(e);
            }
        }
        0
    }

    fn sigprocmask(&mut self, mem: &Mem, how: u64, set: u64, oset: u64, size: u64) -> i64 {
        if size != 8 {
            return err(EINVAL);
        }
        let old = self.sigmask;
        if set != 0 {
            let s = match read_u64(mem, set) {
                Ok(s) => s,
                Err(e) => return err(e),
            };
            let new = match how {
                SIG_BLOCK => old | s,
                SIG_UNBLOCK => old & !s,
                SIG_SETMASK => s,
                _ => return err(EINVAL),
            };
            self.sigmask = new & !UNBLOCKABLE;
        }
        if oset != 0 {
            return result(write_u64(mem, oset, old).map(|()| 0));
        }
        0
    }

    /// `readlink`/`readlinkat`: `/proc/self/exe` reads as the guest program, not calco;
    /// any other path goes to the kernel.
    fn readlink(&self, mem: &Mem, nr: u64, a: [u64; 6], path: u64, buf: u64, size: u64) -> i64 {
        let is_exe = read_cstr(mem, path)
            .is_some_and(|p| p == b"/proc/self/exe" || p == b"/proc/thread-self/exe");
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

/// Serves the `syscall` the guest just executed; `cpu.rip` is already past it.
pub fn handle(cpu: &mut Cpu, mem: &mut Mem, state: &mut State) {
    let nr = cpu.gpr[RAX];
    let args = [RDI, RSI, RDX, R10, R8, R9].map(|r| cpu.gpr[r]);
    let ret = state.dispatch(cpu, mem, nr, args);
    cpu.gpr[RAX] = ret as u64;
    // What the `syscall` instruction leaves: the return address in RCX, RFLAGS in R11.
    cpu.gpr[RCX] = cpu.rip;
    cpu.gpr[R11] = cpu.rflags;
}

#[cfg(test)]
mod tests;
