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
fn rejects_32_bit() {
    let mut b = minimal_elf();
    b[4] = 1;
    assert!(parse(&b).unwrap_err().contains("64-bit"));
}

#[test]
fn rejects_other_machine() {
    let mut b = minimal_elf();
    b[18..20].copy_from_slice(&183u16.to_le_bytes()); // EM_AARCH64
    assert!(parse(&b).unwrap_err().contains("x86-64"));
}

#[test]
fn rejects_non_elf_and_truncated() {
    assert!(parse(b"#!/bin/sh\n").is_err());
    let b = minimal_elf();
    assert!(parse(&b[..EHDR_SIZE + 10]).is_err());
}

#[test]
fn rejects_misaligned_load() {
    let mut b = minimal_elf();
    let off = EHDR_SIZE + 8;
    b[off..off + 8].copy_from_slice(&0x10u64.to_le_bytes());
    assert!(parse(&b).is_err());
}

#[test]
fn detects_interp() {
    let mut b = minimal_elf();
    b[EHDR_SIZE..EHDR_SIZE + 4].copy_from_slice(&PT_INTERP.to_le_bytes());
    // With its only PT_LOAD turned into PT_INTERP there is nothing to load.
    assert!(parse(&b).is_err());

    let mut two = minimal_elf();
    two[56..58].copy_from_slice(&2u16.to_le_bytes());
    let mut interp = two[EHDR_SIZE..EHDR_SIZE + PHDR_SIZE].to_vec();
    interp[..4].copy_from_slice(&PT_INTERP.to_le_bytes());
    let at = EHDR_SIZE + PHDR_SIZE;
    two.splice(at..at, interp);
    let elf = parse(&two).unwrap();
    assert!(elf.has_interp());
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
        assert_eq!(
            keys,
            vec![
                AT_PHDR,
                AT_PHENT,
                AT_PHNUM,
                AT_PAGESZ,
                AT_BASE,
                AT_FLAGS,
                AT_ENTRY,
                AT_UID,
                AT_EUID,
                AT_GID,
                AT_EGID,
                AT_SECURE,
                AT_RANDOM,
                AT_HWCAP,
                AT_HWCAP2,
                AT_CLKTCK,
                AT_PLATFORM,
                AT_EXECFN,
                AT_MINSIGSTKSZ,
                AT_NULL,
            ]
        );
        assert!(p <= top);
    }
}
