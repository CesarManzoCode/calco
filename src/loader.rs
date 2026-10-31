//! ELF loader: `ET_EXEC` and `ET_DYN` executables, their `PT_INTERP` interpreter, `#!`
//! scripts, `brk` start and the initial stack.

use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};

use crate::mem::{DYN_BASE, Mem, PAGE_SIZE, STACK_SIZE, USER_END, page_down, page_up};
use crate::sys::{
    self, EACCES, ELOOP, ENOEXEC, MAP_FIXED, MAP_NORESERVE, MAP_PRIVATE, PROT_EXEC, PROT_NONE,
    PROT_READ, PROT_WRITE,
};

// /usr/include/elf.h
const ELF_MAGIC: [u8; 4] = *b"\x7fELF";
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;
pub const ET_EXEC: u16 = 2;
pub const ET_DYN: u16 = 3;
pub const EM_X86_64: u16 = 62;
pub const PT_LOAD: u32 = 1;
pub const PT_INTERP: u32 = 3;
pub const PT_PHDR: u32 = 6;
pub const PT_TLS: u32 = 7;
pub const PT_GNU_STACK: u32 = 0x6474_e551;
pub const PF_X: u32 = 1;
pub const PF_W: u32 = 2;
pub const PF_R: u32 = 4;
pub const EHDR_SIZE: usize = 64;
pub const PHDR_SIZE: usize = 56;
/// The kernel refuses program header tables larger than 64 KiB.
const MAX_PHDR_TABLE: usize = 64 << 10;

pub const AT_NULL: u64 = 0;
pub const AT_PHDR: u64 = 3;
pub const AT_PHENT: u64 = 4;
pub const AT_PHNUM: u64 = 5;
pub const AT_PAGESZ: u64 = 6;
pub const AT_BASE: u64 = 7;
pub const AT_FLAGS: u64 = 8;
pub const AT_ENTRY: u64 = 9;
pub const AT_UID: u64 = 11;
pub const AT_EUID: u64 = 12;
pub const AT_GID: u64 = 13;
pub const AT_EGID: u64 = 14;
pub const AT_PLATFORM: u64 = 15;
pub const AT_HWCAP: u64 = 16;
pub const AT_CLKTCK: u64 = 17;
pub const AT_SECURE: u64 = 23;
pub const AT_RANDOM: u64 = 25;
pub const AT_HWCAP2: u64 = 26;
pub const AT_EXECFN: u64 = 31;
pub const AT_MINSIGSTKSZ: u64 = 51;
/// `MINSIGSTKSZ` of /usr/include/asm/signal.h; the guest frame is FXSAVE-only, no XSAVE.
const MINSIGSTKSZ: u64 = 2048;
const PLATFORM: &[u8] = b"x86_64\0";
/// Bytes of the file start the kernel reads to find the format and the `#!` line.
pub const BINPRM_BUF_SIZE: usize = 256;
/// Format rewrites the kernel follows before `ELOOP` (`exec_binprm` in fs/exec.c).
const MAX_EXEC_DEPTH: usize = 5;
/// `PATH_MAX`: the kernel refuses a longer `PT_INTERP` (`load_elf_binary`).
const MAX_INTERP: u64 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Phdr {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

#[derive(Debug)]
pub struct Elf {
    pub e_type: u16,
    pub entry: u64,
    pub phoff: u64,
    pub phdrs: Vec<Phdr>,
}

impl Elf {
    pub fn has_interp(&self) -> bool {
        self.phdrs.iter().any(|p| p.p_type == PT_INTERP)
    }

    fn loads(&self) -> impl Iterator<Item = &Phdr> {
        self.phdrs.iter().filter(|p| p.p_type == PT_LOAD)
    }

    /// Page-aligned `[low, high)` of the `PT_LOAD` segments, before relocation.
    pub fn span(&self) -> (u64, u64) {
        let low = self.loads().map(|p| p.p_vaddr).min().unwrap_or(0);
        let high = self
            .loads()
            .map(|p| p.p_vaddr + p.p_memsz)
            .max()
            .unwrap_or(0);
        (page_down(low), page_up(high))
    }

    /// Largest `p_align` of the `PT_LOAD` segments that is a power of two, at least a page.
    fn align(&self) -> u64 {
        self.loads()
            .map(|p| p.p_align)
            .filter(|a| a.is_power_of_two())
            .fold(PAGE_SIZE, u64::max)
    }

    /// Address of the program headers once loaded with `bias`: `PT_PHDR`, or the
    /// `PT_LOAD` that holds `e_phoff` in the file.
    pub fn phdr_addr(&self, bias: u64) -> u64 {
        match self.phdrs.iter().find(|p| p.p_type == PT_PHDR) {
            Some(p) => p.p_vaddr + bias,
            None => self
                .loads()
                .find(|p| p.p_offset <= self.phoff && self.phoff < p.p_offset + p.p_filesz)
                .map(|p| p.p_vaddr + (self.phoff - p.p_offset) + bias)
                .unwrap_or(0),
        }
    }
}

/// Bias that places an `ET_DYN` image so its lowest page lands on `base`; `ET_EXEC`
/// images stay at their `p_vaddr`.
pub fn load_bias(elf: &Elf, base: u64) -> u64 {
    if elf.e_type == ET_DYN {
        base.wrapping_sub(elf.span().0)
    } else {
        0
    }
}

/// Values of the auxiliary vector that depend on what was loaded: program headers and
/// entry of the executable, load address of the interpreter (0 without one).
pub fn aux_for(exe: &Elf, exe_bias: u64, interp_base: u64) -> AuxInfo {
    AuxInfo {
        phdr: exe.phdr_addr(exe_bias),
        phnum: exe.phdrs.len() as u64,
        entry: exe.entry + exe_bias,
        base: interp_base,
        ..AuxInfo::default()
    }
}

fn u16_at(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(b[off..off + 2].try_into().unwrap())
}

fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}

fn u64_at(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().unwrap())
}

/// End offset of the program header table, from a buffer holding at least the ELF header.
fn phdr_table_end(image: &[u8]) -> Option<usize> {
    if image.len() < EHDR_SIZE {
        return None;
    }
    let phoff = usize::try_from(u64_at(image, 32)).ok()?;
    let phnum = u16_at(image, 56) as usize;
    phoff.checked_add(phnum * PHDR_SIZE)
}

/// Parses the ELF header and program headers of `image`, the start of the file.
pub fn parse(image: &[u8]) -> Result<Elf, String> {
    if image.len() < EHDR_SIZE || image[..4] != ELF_MAGIC {
        return Err("not an ELF file".into());
    }
    if image[4] != ELFCLASS64 {
        return Err("not a 64-bit ELF (32-bit programs are not supported)".into());
    }
    if image[5] != ELFDATA2LSB {
        return Err("not a little-endian ELF".into());
    }
    let e_type = u16_at(image, 16);
    let machine = u16_at(image, 18);
    if machine != EM_X86_64 {
        return Err(format!("ELF machine {machine} is not x86-64"));
    }
    if e_type != ET_EXEC && e_type != ET_DYN {
        return Err(format!("ELF type {e_type} is not an executable"));
    }
    let entry = u64_at(image, 24);
    let phoff = u64_at(image, 32);
    let phentsize = u16_at(image, 54) as usize;
    let phnum = u16_at(image, 56) as usize;
    if phentsize != PHDR_SIZE {
        return Err(format!("unexpected program header size {phentsize}"));
    }
    if phnum == 0 || phnum * PHDR_SIZE > MAX_PHDR_TABLE {
        return Err(format!("bad program header count {phnum}"));
    }
    let end = phdr_table_end(image).filter(|&e| e <= image.len());
    if end.is_none() {
        return Err("program headers out of the file".into());
    }

    let mut phdrs = Vec::with_capacity(phnum);
    for i in 0..phnum {
        let b = &image[phoff as usize + i * PHDR_SIZE..][..PHDR_SIZE];
        let ph = Phdr {
            p_type: u32_at(b, 0),
            p_flags: u32_at(b, 4),
            p_offset: u64_at(b, 8),
            p_vaddr: u64_at(b, 16),
            p_filesz: u64_at(b, 32),
            p_memsz: u64_at(b, 40),
            p_align: u64_at(b, 48),
        };
        if ph.p_type == PT_LOAD {
            if ph.p_filesz > ph.p_memsz {
                return Err("PT_LOAD with file size larger than memory size".into());
            }
            if ph.p_offset % PAGE_SIZE != ph.p_vaddr % PAGE_SIZE {
                return Err("PT_LOAD offset and address disagree modulo the page size".into());
            }
            let end = ph.p_vaddr.checked_add(ph.p_memsz);
            if end.is_none_or(|e| e > USER_END) {
                return Err("PT_LOAD outside the user address space".into());
            }
        }
        phdrs.push(ph);
    }
    let elf = Elf {
        e_type,
        entry,
        phoff,
        phdrs,
    };
    if elf.loads().next().is_none() {
        return Err("no PT_LOAD segment".into());
    }
    Ok(elf)
}

/// Values of the auxiliary vector that do not point into the stack.
#[derive(Clone, Copy, Debug, Default)]
pub struct AuxInfo {
    pub phdr: u64,
    pub phnum: u64,
    pub entry: u64,
    pub base: u64,
    pub uid: u64,
    pub euid: u64,
    pub gid: u64,
    pub egid: u64,
    pub hwcap: u64,
}

/// Bytes of the initial stack, `[rsp, top)`.
pub struct StackImage {
    pub rsp: u64,
    pub bytes: Vec<u8>,
}

impl StackImage {
    fn put(&mut self, addr: u64, data: &[u8]) {
        let off = (addr - self.rsp) as usize;
        self.bytes[off..off + data.len()].copy_from_slice(data);
    }

    fn put_u64(&mut self, addr: u64, v: u64) {
        self.put(addr, &v.to_le_bytes());
    }

    /// Writes a NUL-terminated string at `*cursor` (the NUL is already there) and advances it.
    fn put_str(&mut self, cursor: &mut u64, s: &[u8]) -> u64 {
        let at = *cursor;
        self.put(at, s);
        *cursor += s.len() as u64 + 1;
        at
    }
}

/// Builds the System V initial stack below `top`: `argc`, `argv`, `envp`, auxv, then
/// `AT_RANDOM` bytes, platform and the strings. `rsp` comes out 16-byte aligned.
pub fn build_stack(
    top: u64,
    argv: &[Vec<u8>],
    envp: &[Vec<u8>],
    execfn: &[u8],
    random: [u8; 16],
    aux: &AuxInfo,
) -> StackImage {
    let strlen = |v: &[Vec<u8>]| v.iter().map(|s| s.len() as u64 + 1).sum::<u64>();
    let strings =
        16 + PLATFORM.len() as u64 + strlen(argv) + strlen(envp) + execfn.len() as u64 + 1;
    // Eight zero bytes at the very top, like the kernel's end marker.
    let strings_low = top - 8 - strings;

    const NAUX: u64 = 20;
    let words = 1 + (argv.len() as u64 + 1) + (envp.len() as u64 + 1) + 2 * NAUX;
    let rsp = ((strings_low & !15) - 8 * words) & !15;
    let mut img = StackImage {
        rsp,
        bytes: vec![0; (top - rsp) as usize],
    };

    let random_addr = strings_low;
    img.put(random_addr, &random);
    let platform_addr = random_addr + 16;
    img.put(platform_addr, PLATFORM);
    let mut cursor = platform_addr + PLATFORM.len() as u64;
    let argv_addrs: Vec<u64> = argv.iter().map(|s| img.put_str(&mut cursor, s)).collect();
    let envp_addrs: Vec<u64> = envp.iter().map(|s| img.put_str(&mut cursor, s)).collect();
    let execfn_addr = img.put_str(&mut cursor, execfn);

    let auxv: [(u64, u64); NAUX as usize] = [
        (AT_PHDR, aux.phdr),
        (AT_PHENT, PHDR_SIZE as u64),
        (AT_PHNUM, aux.phnum),
        (AT_PAGESZ, PAGE_SIZE),
        (AT_BASE, aux.base),
        (AT_FLAGS, 0),
        (AT_ENTRY, aux.entry),
        (AT_UID, aux.uid),
        (AT_EUID, aux.euid),
        (AT_GID, aux.gid),
        (AT_EGID, aux.egid),
        (AT_SECURE, 0),
        (AT_RANDOM, random_addr),
        (AT_HWCAP, aux.hwcap),
        (AT_HWCAP2, 0),
        (AT_CLKTCK, 100),
        (AT_PLATFORM, platform_addr),
        (AT_EXECFN, execfn_addr),
        (AT_MINSIGSTKSZ, MINSIGSTKSZ),
        (AT_NULL, 0),
    ];

    let mut vector = Vec::with_capacity(words as usize);
    vector.push(argv.len() as u64);
    vector.extend(&argv_addrs);
    vector.push(0);
    vector.extend(&envp_addrs);
    vector.push(0);
    for (key, value) in auxv {
        vector.push(key);
        vector.push(value);
    }
    for (i, word) in vector.into_iter().enumerate() {
        img.put_u64(rsp + 8 * i as u64, word);
    }
    img
}

/// Result of loading a program: where the guest starts.
#[derive(Debug)]
pub struct Loaded {
    pub entry: u64,
    pub rsp: u64,
    pub brk: u64,
    pub phdr: u64,
}

fn os_err(errno: i32) -> io::Error {
    io::Error::from_raw_os_error(errno)
}

fn read_prefix(file: &File, len: usize) -> io::Result<Vec<u8>> {
    let mut buf = vec![0; len];
    let mut done = 0;
    while done < len {
        match file.read_at(&mut buf[done..], done as u64) {
            Ok(0) => break,
            Ok(n) => done += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    buf.truncate(done);
    Ok(buf)
}

fn elf_prot(flags: u32) -> u32 {
    let mut prot = 0;
    if flags & PF_R != 0 {
        prot |= PROT_READ;
    }
    if flags & PF_W != 0 {
        prot |= PROT_WRITE;
    }
    if flags & PF_X != 0 {
        prot |= PROT_EXEC;
    }
    prot
}

/// Maps one `PT_LOAD`: file pages, zeroed tail of the last file page and anonymous bss.
fn map_segment(mem: &mut Mem, fd: i32, ph: &Phdr, bias: u64) -> Result<(), i32> {
    let prot = elf_prot(ph.p_flags);
    let vaddr = ph.p_vaddr + bias;
    let start = page_down(vaddr);
    let file_end = vaddr + ph.p_filesz;
    let mem_end = vaddr + ph.p_memsz;
    let flags = MAP_PRIVATE | MAP_FIXED;
    let mut anon_start = start;
    if ph.p_filesz > 0 {
        let file_page_end = page_up(file_end);
        let offset = ph.p_offset - (vaddr - start);
        let zero_tail = mem_end > file_end && file_end % PAGE_SIZE != 0;
        let map_prot = if zero_tail { prot | PROT_WRITE } else { prot };
        mem.map_file(start, file_page_end - start, map_prot, flags, fd, offset)?;
        if zero_tail {
            let len = (file_page_end - file_end) as usize;
            unsafe { std::ptr::write_bytes(file_end as *mut u8, 0, len) };
            if map_prot != prot {
                mem.protect(start, file_page_end - start, prot)?;
            }
        }
        anon_start = file_page_end;
    }
    let mem_page_end = page_up(mem_end);
    if mem_page_end > anon_start {
        mem.map_anon(anon_start, mem_page_end - anon_start, prot, flags)?;
    }
    Ok(())
}

/// Maps every `PT_LOAD` of `elf` with `bias` and returns the end of the highest one.
fn map_image(mem: &mut Mem, file: &File, elf: &Elf, bias: u64, name: &str) -> Result<u64, String> {
    let mut high = 0;
    for ph in elf.loads() {
        let end = ph.p_vaddr.wrapping_add(bias).checked_add(ph.p_memsz);
        let end = end.filter(|&e| e <= USER_END);
        let end = end.ok_or_else(|| format!("{name}: segment outside the user address space"))?;
        map_segment(mem, file.as_raw_fd(), ph, bias).map_err(|e| {
            format!(
                "{name}: cannot map segment at {:#x}: {}",
                ph.p_vaddr.wrapping_add(bias),
                os_err(e)
            )
        })?;
        high = high.max(end);
    }
    Ok(high)
}

/// Path named by `PT_INTERP`, without its NUL.
fn interp_path(file: &File, elf: &Elf) -> Result<Option<PathBuf>, String> {
    let Some(ph) = elf.phdrs.iter().find(|p| p.p_type == PT_INTERP) else {
        return Ok(None);
    };
    if ph.p_filesz < 2 || ph.p_filesz > MAX_INTERP {
        return Err("bad PT_INTERP size".into());
    }
    let mut buf = vec![0; ph.p_filesz as usize];
    file.read_exact_at(&mut buf, ph.p_offset)
        .map_err(|e| format!("PT_INTERP: {e}"))?;
    match buf.pop() {
        Some(0) if !buf.contains(&0) => Ok(Some(PathBuf::from(OsString::from_vec(buf)))),
        _ => Err("PT_INTERP is not a NUL-terminated path".into()),
    }
}

/// Loads the interpreter in the placement window, aligned to its largest `p_align`, with
/// the holes between its segments kept reserved for it. Returns its ELF and bias.
fn load_interp(mem: &mut Mem, path: &Path) -> Result<(Elf, u64), String> {
    let name = path.display().to_string();
    let file = File::open(path).map_err(|e| format!("{name}: {e}"))?;
    let image = read_head(&file).map_err(|e| format!("{name}: {e}"))?;
    let elf = parse(&image).map_err(|e| format!("{name}: {e}"))?;
    if elf.has_interp() {
        return Err(format!("{name}: the interpreter asks for an interpreter"));
    }
    let (low, high) = elf.span();
    let bias = if elf.e_type == ET_DYN {
        let (span, align) = (high - low, elf.align());
        let flags = MAP_PRIVATE | MAP_NORESERVE;
        let placed = mem
            .map_anon(0, span + align - PAGE_SIZE, PROT_NONE, flags)
            .map_err(|e| format!("{name}: no room in the placement window: {}", os_err(e)))?;
        let start = placed.next_multiple_of(align);
        let trims = [
            (placed, start),
            (start + span, placed + span + align - PAGE_SIZE),
        ];
        for (s, e) in trims.into_iter().filter(|(s, e)| e > s) {
            mem.unmap(s, e - s)
                .map_err(|e| format!("{name}: {}", os_err(e)))?;
        }
        start - low
    } else {
        0
    };
    map_image(mem, &file, &elf, bias, &name)?;
    Ok((elf, bias))
}

/// The start of `file`: what the kernel reads to tell the format, extended to the whole
/// program header table of an ELF.
fn read_head(file: &File) -> io::Result<Vec<u8>> {
    let image = read_prefix(file, BINPRM_BUF_SIZE)?;
    match phdr_table_end(&image).filter(|&e| e > image.len() && image[..4] == ELF_MAGIC) {
        Some(end) => read_prefix(
            file,
            end.min(EHDR_SIZE + MAX_PHDR_TABLE + PAGE_SIZE as usize),
        ),
        None => Ok(image),
    }
}

/// Interpreter and optional argument of a `#!` line, parsed as `load_script` does
/// (fs/binfmt_script.c): only the first `BINPRM_BUF_SIZE` bytes count, the interpreter
/// ends at a space, tab or NUL, and the rest of the line, without the spaces or tabs
/// around it, is one argument.
pub fn parse_shebang(head: &[u8]) -> Result<(Vec<u8>, Option<Vec<u8>>), i32> {
    let spacetab = |c: u8| c == b' ' || c == b'\t';
    let mut buf = [0u8; BINPRM_BUF_SIZE];
    let n = head.len().min(BINPRM_BUF_SIZE);
    buf[..n].copy_from_slice(&head[..n]);
    if !buf.starts_with(b"#!") {
        return Err(ENOEXEC);
    }
    let last = BINPRM_BUF_SIZE - 1;
    let mut end = match buf.iter().position(|&c| c == b'\n') {
        Some(nl) => nl,
        None => {
            // Without a newline the line was cut; an interpreter cut too is refused.
            let name = (2..=last).find(|&i| !spacetab(buf[i])).ok_or(ENOEXEC)?;
            if !buf[name..=last].iter().any(|&c| spacetab(c) || c == 0) {
                return Err(ENOEXEC);
            }
            last
        }
    };
    while end > 2 && spacetab(buf[end - 1]) {
        end -= 1;
    }
    let line = &buf[2..end];
    let start = line.iter().position(|&c| !spacetab(c)).ok_or(ENOEXEC)?;
    let line = &line[start..];
    let cstr = |s: &[u8]| s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())].to_vec();
    match line.iter().position(|&c| spacetab(c) || c == 0) {
        Some(sep) if line[sep] != 0 => {
            let rest = &line[sep..];
            let arg = rest
                .iter()
                .position(|&c| !spacetab(c))
                .map(|i| cstr(&rest[i..]));
            Ok((line[..sep].to_vec(), arg))
        }
        _ => Ok((cstr(line), None)),
    }
}

/// Follows `path` like `execve` does before loading: checks that it may be executed and
/// rewrites `#!` scripts into their interpreter (argv[0] goes away; interpreter, optional
/// argument and script path come first). Returns the ELF to load and its argv, or the
/// errno the kernel gives.
pub fn resolve(path: &Path, mut argv: Vec<Vec<u8>>) -> Result<(PathBuf, Vec<Vec<u8>>), i32> {
    let mut path = path.to_path_buf();
    for _ in 0..=MAX_EXEC_DEPTH {
        let bytes = path.as_os_str().as_bytes();
        sys::access_exec(bytes)?;
        let file = File::open(&path).map_err(|e| e.raw_os_error().unwrap_or(EACCES))?;
        let meta = file
            .metadata()
            .map_err(|e| e.raw_os_error().unwrap_or(EACCES))?;
        if !meta.is_file() {
            return Err(EACCES);
        }
        let head = read_head(&file).map_err(|e| e.raw_os_error().unwrap_or(EACCES))?;
        if !head.starts_with(b"#!") {
            parse(&head).map_err(|_| ENOEXEC)?;
            return Ok((path, argv));
        }
        let (interp, arg) = parse_shebang(&head)?;
        let mut next = vec![interp.clone()];
        next.extend(arg);
        next.push(bytes.to_vec());
        next.extend(argv.into_iter().skip(1));
        argv = next;
        path = PathBuf::from(OsString::from_vec(interp));
    }
    Err(ELOOP)
}

/// Loads an executable into `mem` (and its `PT_INTERP` interpreter, which then is where the
/// guest starts), reserves the code cache, maps the stack and writes the initial stack.
/// `execfn` is the path `AT_EXECFN` names (the script for `#!`); `hwcap` is the already
/// filtered CPUID.1:EDX.
pub fn load(
    path: &Path,
    execfn: &[u8],
    mem: &mut Mem,
    argv: &[Vec<u8>],
    envp: &[Vec<u8>],
    hwcap: u64,
) -> Result<Loaded, String> {
    let name = path.display().to_string();
    let file = File::open(path).map_err(|e| format!("{name}: {e}"))?;
    let image = read_head(&file).map_err(|e| format!("{name}: {e}"))?;
    let elf = parse(&image).map_err(|e| format!("{name}: {e}"))?;
    let interp = interp_path(&file, &elf).map_err(|e| format!("{name}: {e}"))?;

    let bias = load_bias(&elf, DYN_BASE);
    let high = map_image(mem, &file, &elf, bias, &name)?;
    drop(file);

    let brk = page_up(high);
    mem.set_brk_base(brk);
    mem.reserve_code_cache()
        .map_err(|e| format!("cannot reserve the code cache: {}", os_err(e)))?;

    // The interpreter goes in the placement window, which exists once `brk` is fixed.
    let (start, interp_base) = match interp {
        Some(ipath) => {
            let (ielf, ibias) = load_interp(mem, &ipath)?;
            (ielf.entry + ibias, ielf.span().0 + ibias)
        }
        None => (elf.entry + bias, 0),
    };
    let aux = AuxInfo {
        uid: sys::getuid(),
        euid: sys::geteuid(),
        gid: sys::getgid(),
        egid: sys::getegid(),
        hwcap,
        ..aux_for(&elf, bias, interp_base)
    };
    let phdr = aux.phdr;
    let mut random = [0u8; 16];
    sys::getrandom(&mut random).map_err(|e| format!("getrandom: {}", os_err(e)))?;

    let top = mem
        .map_stack()
        .map_err(|e| format!("cannot map the guest stack: {}", os_err(e)))?;
    let stack = build_stack(top, argv, envp, execfn, random, &aux);
    if stack.bytes.len() as u64 > STACK_SIZE {
        return Err("arguments and environment do not fit in the guest stack".into());
    }
    // The stack range was just mapped read-write for the guest.
    unsafe {
        std::ptr::copy_nonoverlapping(
            stack.bytes.as_ptr(),
            stack.rsp as *mut u8,
            stack.bytes.len(),
        )
    };
    Ok(Loaded {
        entry: start,
        rsp: stack.rsp,
        brk,
        phdr,
    })
}

#[cfg(test)]
mod tests;
