use super::*;

const VADDR: u64 = 0x40_0000;

/// A minimal static x86-64 ELF: header, one `PT_LOAD` (R+X) and `mov eax, 60; syscall`.
fn minimal_elf() -> Vec<u8> {
    let code: &[u8] = &[0xb8, 0x3c, 0x00, 0x00, 0x00, 0x0f, 0x05];
    let code_off = (EHDR_SIZE + PHDR_SIZE) as u64;
    let mut b = Vec::new();
    b.extend_from_slice(&ELF_MAGIC);
    b.extend_from_slice(&[ELFCLASS64, ELFDATA2LSB, 1, 0]);
    b.extend_from_slice(&[0; 8]);
    b.extend_from_slice(&ET_EXEC.to_le_bytes());
    b.extend_from_slice(&EM_X86_64.to_le_bytes());
    b.extend_from_slice(&1u32.to_le_bytes()); // e_version
    b.extend_from_slice(&(VADDR + code_off).to_le_bytes()); // e_entry
    b.extend_from_slice(&(EHDR_SIZE as u64).to_le_bytes()); // e_phoff
    b.extend_from_slice(&0u64.to_le_bytes()); // e_shoff
    b.extend_from_slice(&0u32.to_le_bytes()); // e_flags
    b.extend_from_slice(&(EHDR_SIZE as u16).to_le_bytes());
    b.extend_from_slice(&(PHDR_SIZE as u16).to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // e_phnum
    b.extend_from_slice(&[0; 6]); // e_shentsize, e_shnum, e_shstrndx
    assert_eq!(b.len(), EHDR_SIZE);
    let size = code_off + code.len() as u64;
    b.extend_from_slice(&PT_LOAD.to_le_bytes());
    b.extend_from_slice(&(PF_R | PF_X).to_le_bytes());
    b.extend_from_slice(&0u64.to_le_bytes()); // p_offset
    b.extend_from_slice(&VADDR.to_le_bytes()); // p_vaddr
    b.extend_from_slice(&VADDR.to_le_bytes()); // p_paddr
    b.extend_from_slice(&size.to_le_bytes()); // p_filesz
    b.extend_from_slice(&(size + 0x100).to_le_bytes()); // p_memsz
    b.extend_from_slice(&PAGE_SIZE.to_le_bytes()); // p_align
    b.extend_from_slice(code);
    b
}

#[test]
fn parses_minimal_elf() {
    let elf = parse(&minimal_elf()).unwrap();
    assert_eq!(elf.e_type, ET_EXEC);
    assert_eq!(elf.entry, VADDR + 120);
    assert_eq!(elf.phoff, 64);
    assert_eq!(elf.phdrs.len(), 1);
    let ph = elf.phdrs[0];
    assert_eq!(ph.p_type, PT_LOAD);
    assert_eq!(ph.p_flags, PF_R | PF_X);
    assert_eq!(ph.p_vaddr, VADDR);
    assert_eq!(ph.p_filesz, 127);
    assert_eq!(ph.p_memsz, 127 + 0x100);
    assert!(!elf.has_interp());
    assert_eq!(elf_prot(ph.p_flags), PROT_READ | PROT_EXEC);
}

#[test]
fn rejects_what_it_cannot_load() {
    let mut b = minimal_elf();
    b[4] = 1;
    assert!(parse(&b).unwrap_err().contains("64-bit"));
    let mut b = minimal_elf();
    b[18..20].copy_from_slice(&183u16.to_le_bytes()); // EM_AARCH64
    assert!(parse(&b).unwrap_err().contains("x86-64"));
    assert!(parse(b"#!/bin/sh\n").is_err());
    assert!(
        parse(&minimal_elf()[..EHDR_SIZE + 10]).is_err(),
        "truncated"
    );
    // A PT_LOAD whose p_offset is not congruent with its p_vaddr.
    let mut b = minimal_elf();
    b[EHDR_SIZE + 8..EHDR_SIZE + 16].copy_from_slice(&0x10u64.to_le_bytes());
    assert!(parse(&b).is_err());
}

#[test]
fn detects_interp() {
    let mut b = minimal_elf();
    b[EHDR_SIZE..EHDR_SIZE + 4].copy_from_slice(&PT_INTERP.to_le_bytes());
    // With its only PT_LOAD turned into PT_INTERP there is nothing to load. A PT_INTERP
    // next to a PT_LOAD is `dynamic_elf`, in `auxv_of_an_executable_with_interpreter`.
    assert!(parse(&b).is_err());
}

fn word(img: &StackImage, addr: u64) -> u64 {
    let off = (addr - img.rsp) as usize;
    u64::from_le_bytes(img.bytes[off..off + 8].try_into().unwrap())
}

fn cstr(img: &StackImage, addr: u64) -> Vec<u8> {
    let off = (addr - img.rsp) as usize;
    let len = img.bytes[off..].iter().position(|&c| c == 0).unwrap();
    img.bytes[off..off + len].to_vec()
}

fn strings(v: &[&str]) -> Vec<Vec<u8>> {
    v.iter().map(|s| s.as_bytes().to_vec()).collect()
}

#[test]
fn stack_layout() {
    let top = 0x0000_4000_0000_0000;
    let random: [u8; 16] = core::array::from_fn(|i| i as u8 + 1);
    let aux = AuxInfo {
        phdr: 0x40_0040,
        phnum: 3,
        entry: 0x40_1000,
        hwcap: 0x178b_fbff,
        uid: 1000,
        ..AuxInfo::default()
    };
    for argc in 1..4 {
        let argv = strings(&["prog", "a", "bc"][..argc]);
        let envp = strings(&["X=1", "PATH=/bin"]);
        let img = build_stack(top, &argv, &envp, b"/tmp/prog", random, &aux);
        assert_eq!(img.rsp % 16, 0);
        assert_eq!(img.rsp + img.bytes.len() as u64, top);

        let mut p = img.rsp;
        assert_eq!(word(&img, p), argc as u64);
        p += 8;
        for a in &argv {
            assert_eq!(&cstr(&img, word(&img, p)), a);
            p += 8;
        }
        assert_eq!(word(&img, p), 0);
        p += 8;
        for e in &envp {
            assert_eq!(&cstr(&img, word(&img, p)), e);
            p += 8;
        }
        assert_eq!(word(&img, p), 0);
        p += 8;

        let mut keys = Vec::new();
        loop {
            let (k, v) = (word(&img, p), word(&img, p + 8));
            p += 16;
            keys.push(k);
            match k {
                AT_RANDOM => {
                    let off = (v - img.rsp) as usize;
                    assert!(v > p);
                    assert_eq!(img.bytes[off..off + 16], random);
                }
                AT_PLATFORM => assert_eq!(cstr(&img, v), b"x86_64"),
                AT_EXECFN => assert_eq!(cstr(&img, v), b"/tmp/prog"),
                AT_HWCAP => assert_eq!(v, aux.hwcap),
                AT_PHDR => assert_eq!(v, aux.phdr),
                AT_PHENT => assert_eq!(v, PHDR_SIZE as u64),
                AT_PAGESZ => assert_eq!(v, PAGE_SIZE),
                AT_UID => assert_eq!(v, 1000),
                AT_HWCAP2 | AT_SECURE => assert_eq!(v, 0),
                AT_CLKTCK => assert_eq!(v, 100),
                AT_NULL => break,
                _ => {}
            }
        }
        #[rustfmt::skip]
        let want = [
            AT_PHDR, AT_PHENT, AT_PHNUM, AT_PAGESZ, AT_BASE, AT_FLAGS, AT_ENTRY, AT_UID, AT_EUID,
            AT_GID, AT_EGID, AT_SECURE, AT_RANDOM, AT_HWCAP, AT_HWCAP2, AT_CLKTCK, AT_PLATFORM,
            AT_EXECFN, AT_MINSIGSTKSZ, AT_NULL,
        ];
        assert_eq!(keys, want);
        assert!(p <= top);
    }
}

#[test]
fn shebang_line_parses_like_the_kernel() {
    let sh = || b"/bin/sh".to_vec();
    assert_eq!(parse_shebang(b"#!/bin/sh\necho"), Ok((sh(), None)));
    assert_eq!(parse_shebang(b"#!/bin/sh"), Ok((sh(), None)));
    let env = parse_shebang(b"#! /usr/bin/env  python3 -u \t\nx");
    let want = (b"/usr/bin/env".to_vec(), Some(b"python3 -u".to_vec()));
    assert_eq!(env, Ok(want));
    assert_eq!(parse_shebang(b"#!  \t\n"), Err(ENOEXEC));
    let cut = [b"#!/".as_slice(), &[b'a'; 300]].concat();
    assert_eq!(
        parse_shebang(&cut),
        Err(ENOEXEC),
        "interpreter cut at 256 bytes"
    );
}

#[test]
fn resolve_rewrites_a_script_into_its_interpreter() {
    let script = std::env::current_exe().unwrap();
    let script = script.with_file_name("calco-k2-01-resolve");
    std::fs::write(&script, "#!/bin/sh -e\nexit 0\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let argv = vec![b"name".to_vec(), b"arg".to_vec()];
    let (path, argv) = resolve(&script, argv).unwrap();
    assert_eq!(path, Path::new("/bin/sh"));
    let script = script.as_os_str().as_bytes().to_vec();
    let want = [b"/bin/sh".to_vec(), b"-e".to_vec(), script, b"arg".to_vec()];
    assert_eq!(argv, want);
}

const LD_SO: &[u8] = b"/lib64/ld-linux-x86-64.so.2\0";

/// `minimal_elf` turned into a PIE with a `PT_INTERP` naming `LD_SO`, appended at the end.
fn dynamic_elf() -> Vec<u8> {
    let mut b = minimal_elf();
    b[16..18].copy_from_slice(&ET_DYN.to_le_bytes());
    b[56..58].copy_from_slice(&2u16.to_le_bytes());
    let at = EHDR_SIZE + PHDR_SIZE;
    let mut interp = b[EHDR_SIZE..at].to_vec();
    interp[..4].copy_from_slice(&PT_INTERP.to_le_bytes());
    interp[4..8].copy_from_slice(&PF_R.to_le_bytes());
    let off = (b.len() + PHDR_SIZE) as u64;
    interp[8..16].copy_from_slice(&off.to_le_bytes());
    interp[32..40].copy_from_slice(&(LD_SO.len() as u64).to_le_bytes());
    interp[40..48].copy_from_slice(&(LD_SO.len() as u64).to_le_bytes());
    b.splice(at..at, interp);
    b.extend_from_slice(LD_SO);
    b
}

#[test]
fn places_et_dyn_at_the_dyn_base() {
    let elf = parse(&dynamic_elf()).unwrap();
    assert_eq!(elf.e_type, ET_DYN);
    assert_eq!(elf.span().0, VADDR);
    let bias = load_bias(&elf, DYN_BASE);
    assert_eq!(elf.span().0 + bias, DYN_BASE);
    assert_eq!(load_bias(&parse(&minimal_elf()).unwrap(), DYN_BASE), 0);
}

#[test]
fn auxv_of_an_executable_with_interpreter() {
    let b = dynamic_elf();
    let elf = parse(&b).unwrap();
    assert!(elf.has_interp());
    let path = std::env::current_exe().unwrap();
    let path = path.with_file_name("calco-k2-05-interp");
    std::fs::write(&path, &b).unwrap();
    let interp = interp_path(&File::open(&path).unwrap(), &elf).unwrap();
    assert_eq!(interp, Some(PathBuf::from("/lib64/ld-linux-x86-64.so.2")));

    let base = 0x0000_1000_5000_0000;
    let bias = load_bias(&elf, DYN_BASE);
    let aux = aux_for(&elf, bias, base);
    assert_eq!(
        aux.phdr,
        DYN_BASE + EHDR_SIZE as u64,
        "AT_PHDR from e_phoff"
    );
    assert_eq!(aux.phnum, 2);
    assert_eq!(aux.entry, DYN_BASE + 120, "AT_ENTRY is the executable's");
    let argv = strings(&["p"]);
    let img = build_stack(crate::mem::STACK_TOP, &argv, &[], b"p", [0; 16], &aux);
    let mut p = img.rsp + 8 * 4;
    let mut seen = Vec::new();
    while word(&img, p) != AT_NULL {
        seen.push((word(&img, p), word(&img, p + 8)));
        p += 16;
    }
    assert!(seen.contains(&(AT_BASE, base)));
    assert!(seen.contains(&(AT_ENTRY, DYN_BASE + 120)));
}
