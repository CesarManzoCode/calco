use super::translate::*;
use super::{BlockMap, LookupTable};
use crate::cpu::{OFF_EXIT_REASON, OFF_NEXT_RIP, off_scratch};
use crate::decode::{Class, Insn, Map, decode};
use crate::emit::{RAX, RCX, RSI};

/// Guest code of a static executable.
const BASE: u64 = 0x40_1000;
/// Guest code in the placement window, beyond rel32 of anything near the executable.
const HIGH: u64 = 0x1000_0000_1000;
/// Host code within 2 GiB of `BASE`, as the code cache is.
const NEAR: u64 = 0x2000_0000;
/// Host code more than 2 GiB away from both.
const FAR: u64 = 0x7000_0000_0000;
/// Address of the lookup table of indirect transfers.
const LOOKUP_TABLE: u64 = 0x5555_0000_0000;

struct Guest {
    base: u64,
    bytes: Vec<u8>,
    /// First address without execute permission.
    exec_end: u64,
}

impl Guest {
    fn new(base: u64, bytes: &[u8]) -> Guest {
        Guest {
            base,
            bytes: bytes.to_vec(),
            exec_end: base + bytes.len() as u64,
        }
    }
}

impl GuestCode for Guest {
    fn fetch(&self, rip: u64, buf: &mut [u8]) -> usize {
        if rip < self.base || rip >= self.exec_end {
            return 0;
        }
        let off = (rip - self.base) as usize;
        let n = buf.len().min((self.exec_end - rip) as usize);
        buf[..n].copy_from_slice(&self.bytes[off..off + n]);
        n
    }
}

fn env(host: u64) -> Env {
    Env {
        host,
        exit_thunk: host - 0x100,
        indirect_exit: host - 0x80,
        lookup_table: LOOKUP_TABLE,
        cache_base: host - 0x100,
        first_exit_id: 7,
    }
}

fn tr(g: &Guest, host: u64) -> Translation {
    translate(g, g.base, &env(host)).unwrap()
}

fn targets(t: &Translation) -> Vec<u64> {
    t.exits.iter().map(|e| e.target).collect()
}

/// Every host instruction of the block (body and stubs) with its offset.
fn host_insns(t: &Translation, host: u64) -> Vec<(usize, Insn)> {
    let mut out = Vec::new();
    let mut off = 0;
    while off < t.code.len() {
        let insn = decode(&t.code[off..], host + off as u64)
            .unwrap_or_else(|e| panic!("host code at +{off:#x}: {e:?}"));
        out.push((off, insn));
        off += insn.len as usize;
    }
    out
}

/// `mov dword gs:[off], imm32`: offset and immediate.
fn gs_dword(i: &Insn) -> Option<(usize, u64)> {
    (i.seg_gs && i.map == Map::OneByte && i.opcode == 0xc7 && i.imm_size == 4)
        .then_some((i.disp as usize, i.imm))
}

/// `(next_rip, exit_reason)` of the four stores of an exit, if `insns` starts with them.
fn exit_values(insns: &[(usize, Insn)]) -> Option<(u64, u64)> {
    let v: Vec<(usize, u64)> = insns
        .iter()
        .take(4)
        .filter_map(|(_, i)| gs_dword(i))
        .collect();
    let offsets = [
        OFF_NEXT_RIP,
        OFF_NEXT_RIP + 4,
        OFF_EXIT_REASON,
        OFF_EXIT_REASON + 4,
    ];
    (v.len() == 4 && v.iter().map(|x| x.0).eq(offsets))
        .then(|| (v[0].1 | v[1].1 << 32, v[2].1 | v[3].1 << 32))
}

/// Every exit in the block, inline or stub.
fn exits_in(t: &Translation, host: u64) -> Vec<(u64, u64)> {
    let insns = host_insns(t, host);
    (0..insns.len())
        .filter_map(|k| exit_values(&insns[k..]))
        .collect()
}

/// The exit at host offset `off`.
fn exit_at(t: &Translation, host: u64, off: usize) -> (u64, u64) {
    let insns: Vec<(usize, Insn)> = host_insns(t, host)
        .into_iter()
        .skip_while(|(o, _)| *o < off)
        .collect();
    assert_eq!(insns[0].0, off);
    exit_values(&insns).expect("an exit")
}

/// Instructions the translator may insert: none writes RFLAGS, pushes or calls.
fn neutral(i: &Insn) -> bool {
    match (i.map, i.opcode) {
        (Map::OneByte, 0x87..=0x8b | 0x8d | 0x90..=0x97 | 0xa0..=0xa3 | 0xb8..=0xbf) => true,
        (Map::OneByte, 0xc7) => i.modrm.is_some_and(|m| (m >> 3) & 7 == 0),
        // `jmp qword [m]` (FF /4), the probe's jump to the block it found.
        (Map::OneByte, 0xff) => i.modrm.is_some_and(|m| (m >> 3) & 7 == 4),
        // `movzx r32, word [m]`.
        (Map::Map0F, 0xb7) => true,
        (Map::OneByte, 0xe0..=0xe3 | 0xe9 | 0xeb) => true,
        // `pop gs:[next_rip]` is the `ret` itself: it reads the guest stack, writes none.
        (Map::OneByte, 0x8f) => i.seg_gs,
        (Map::Map0F, 0x80..=0x8f | 0x1f) => true,
        _ => false,
    }
}

#[test]
fn block_ends_at_the_first_control_flow_instruction() {
    // mov eax, 1; jmp +0; nop
    let g = Guest::new(BASE, &[0xb8, 1, 0, 0, 0, 0xeb, 0x00, 0x90]);
    let t = tr(&g, NEAR);
    assert_eq!(t.insns, 2);
    assert_eq!(t.guest_end, BASE + 7);
    assert_eq!(targets(&t), [BASE + 7]);
}

#[test]
fn block_ends_after_64_instructions() {
    let g = Guest::new(BASE, &[0x90; 100]);
    let t = tr(&g, NEAR);
    assert_eq!(t.insns, MAX_INSNS);
    assert_eq!(t.guest_end, BASE + 64);
    assert_eq!(targets(&t), [BASE + 64]);
}

#[test]
fn block_ends_at_a_page_boundary() {
    let g = Guest::new(BASE - 3, &[0x90; 8]);
    let t = tr(&g, NEAR);
    assert_eq!(t.insns, 3);
    assert_eq!(targets(&t), [BASE]);
    // An instruction that starts before the boundary may cross it: nop; nop; mov eax, 1.
    let g = Guest::new(BASE - 3, &[0x90, 0x90, 0xb8, 1, 0, 0, 0, 0x90]);
    let t = tr(&g, NEAR);
    assert_eq!(t.insns, 3);
    assert_eq!(t.guest_end, BASE + 4);
    assert_eq!(targets(&t), [BASE + 4]);
}

#[test]
fn syscall_cpuid_and_traps_leave_inline() {
    let cases: &[(&[u8], u32, u64)] = &[
        (&[0x0f, 0x05], EXIT_SYSCALL, BASE + 2),
        (&[0x0f, 0xa2], EXIT_CPUID, BASE + 2),
        // int3 and int 3: the rip is past the instruction, as the kernel reports it.
        (&[0xcc], exit_signal(5), BASE + 1),
        (&[0xcd, 0x03], exit_signal(5), BASE + 2),
        // ud2, hlt, int 0x80.
        (&[0x0f, 0x0b], exit_signal(4), BASE),
        (&[0xf4], exit_signal(11), BASE),
        (&[0xcd, 0x80], exit_signal(11), BASE),
        // Far return is out of the scope.
        (&[0xcb], exit_signal(11), BASE),
        // A string instruction with gs.
        (&[0x65, 0xa4], exit_signal(4), BASE),
    ];
    for &(bytes, reason, next) in cases {
        let mut code = bytes.to_vec();
        code.push(0x90);
        let g = Guest::new(BASE, &code);
        let t = tr(&g, NEAR);
        assert_eq!(t.insns, 1, "{bytes:02x?}");
        assert!(t.exits.is_empty(), "{bytes:02x?}");
        assert_eq!(exits_in(&t, NEAR), [(next, reason as u64)], "{bytes:02x?}");
    }
}

#[test]
fn invalid_instruction_leaves_with_sigill() {
    // nop; vzeroupper (VEX is out of the scope)
    let g = Guest::new(BASE, &[0x90, 0xc5, 0xf8, 0x77]);
    let t = tr(&g, NEAR);
    assert_eq!(t.insns, 2);
    assert!(t.exits.is_empty());
    assert_eq!(exits_in(&t, NEAR), [(BASE + 1, exit_signal(4) as u64)]);
}

#[test]
fn code_without_execute_permission_faults() {
    let mut g = Guest::new(BASE, &[0x90, 0x48, 0xb8, 1, 2, 3, 4, 5, 6, 7, 8]);
    g.exec_end = BASE;
    assert_eq!(
        translate(&g, BASE, &env(NEAR)).unwrap_err(),
        Fault {
            signal: 11,
            addr: BASE
        }
    );
    // The movabs runs into memory without permission: the block stops before it, and the
    // block that starts at it faults at the first byte without permission.
    g.exec_end = BASE + 3;
    let t = tr(&g, NEAR);
    assert_eq!(t.insns, 1);
    assert_eq!(targets(&t), [BASE + 1]);
    assert_eq!(
        translate(&g, BASE + 1, &env(NEAR)).unwrap_err(),
        Fault {
            signal: 11,
            addr: BASE + 3
        }
    );
}

#[test]
fn chainable_exits_are_aligned_and_start_at_their_stubs() {
    // jne +2; nop; nop
    let g = Guest::new(BASE, &[0x75, 0x02, 0x90, 0x90]);
    for host in [NEAR, NEAR + 1, NEAR + 2, NEAR + 3] {
        let t = translate(&g, BASE, &env(host)).unwrap();
        assert_eq!(targets(&t), [BASE + 4, BASE + 2]);
        for (i, e) in t.exits.iter().enumerate() {
            assert_eq!((host + e.jump as u64) % 4, 0);
            let rel = i32::from_le_bytes(t.code[e.jump as usize..][..4].try_into().unwrap());
            assert_eq!(e.jump as i64 + 4 + rel as i64, e.stub as i64);
            let reason = EXIT_BRANCH as u64 | (7 + i as u64) << 32;
            assert_eq!(exit_at(&t, host, e.stub as usize), (e.target, reason));
            assert_eq!(
                t.lookup(e.stub).unwrap().pc,
                GuestPc::At(e.target),
                "a stub is at the boundary of its target"
            );
        }
    }
}

#[test]
fn loop_and_jrcxz_jump_over_the_fallthrough_exit() {
    for op in [0xe0, 0xe1, 0xe2, 0xe3] {
        // To itself.
        let g = Guest::new(BASE, &[op, 0xfe]);
        let t = tr(&g, NEAR);
        assert_eq!(targets(&t), [BASE + 2, BASE]);
        let (_, short) = host_insns(&t, NEAR)[0];
        assert_eq!(short.opcode, op);
        // Taken, it lands on the padding before the jmp of the target's exit.
        let landing = (short.target.unwrap() - NEAR) as usize;
        let jmp = t.exits[1].jump as usize - 1;
        assert!(t.code[landing..jmp].iter().all(|&b| b == 0x90));
        assert_eq!(t.code[jmp], 0xe9);
        assert_eq!(t.lookup(landing as u32).unwrap().pc, GuestPc::At(BASE));
    }
}

#[test]
fn direct_call_writes_the_return_address_and_chains() {
    let g = Guest::new(BASE, &[0xe8, 0x10, 0x00, 0x00, 0x00]);
    let t = tr(&g, NEAR);
    assert_eq!(targets(&t), [BASE + 0x15]);
    // mov qword [rsp-8], BASE+5; lea rsp, [rsp-8]
    assert_eq!(
        &t.code[..14],
        [
            0x48, 0xc7, 0x44, 0x24, 0xf8, 0x05, 0x10, 0x40, 0x00, 0x48, 0x8d, 0x64, 0x24, 0xf8
        ]
    );
    assert_eq!(t.lookup(9).unwrap().pc, GuestPc::At(BASE));
    assert_eq!(t.lookup(14).unwrap().pc, GuestPc::At(BASE + 0x15));
}

#[test]
fn pc_table_maps_host_code_to_guest_boundaries() {
    // mov rax, rbx; add rax, rcx; mov rax, [rip+0x10]; push rax; ret
    let code = [
        0x48, 0x89, 0xd8, 0x48, 0x01, 0xc8, 0x48, 0x8b, 0x05, 0x10, 0x00, 0x00, 0x00, 0x50, 0xc3,
    ];
    let g = Guest::new(BASE, &code);
    let t = tr(&g, NEAR);
    assert_eq!(t.insns, 5);
    assert_eq!(t.pcs[0].host_off, 0);
    assert!(t.pcs.windows(2).all(|w| w[0].host_off < w[1].host_off));
    let insns = host_insns(&t, NEAR);
    let rips = [BASE, BASE + 3, BASE + 6, BASE + 13, BASE + 14];
    for (k, &rip) in rips.iter().enumerate() {
        let e = t.lookup(insns[k].0 as u32).unwrap();
        assert_eq!(e.pc, GuestPc::At(rip), "host instruction {k}");
        assert_eq!(e.temps, [None; SCRATCH_SLOTS]);
    }
    // After `pop gs:[next_rip]` the guest is wherever `ret` goes: the probe starts there,
    // with no temporaries yet.
    let (off, save) = insns[5];
    assert_eq!((save.opcode, save.seg_gs), (0x89, true));
    let e = t.lookup(off as u32).unwrap();
    assert_eq!((e.pc, e.temps), (GuestPc::NextRip, [None; SCRATCH_SLOTS]));
}

#[test]
fn pc_table_tracks_temporaries() {
    // mov rax, [rip+0x10] more than 2 GiB from the host code; nop
    let g = Guest::new(BASE, &[0x48, 0x8b, 0x05, 0x10, 0x00, 0x00, 0x00, 0x90]);
    let t = tr(&g, FAR);
    let insns = host_insns(&t, FAR);
    let none = [None; SCRATCH_SLOTS];
    let mut saved = none;
    saved[0] = Some(RSI);
    let expect = [
        // mov gs:[scratch0], rsi
        (GuestPc::At(BASE), none),
        // mov rsi, BASE+0x17
        (GuestPc::At(BASE), saved),
        // mov rax, [rsi]
        (GuestPc::At(BASE), saved),
        // mov rsi, gs:[scratch0]
        (GuestPc::At(BASE + 7), saved),
        // nop
        (GuestPc::At(BASE + 7), none),
    ];
    for (k, (pc, temps)) in expect.into_iter().enumerate() {
        let e = t.lookup(insns[k].0 as u32).unwrap();
        assert_eq!((e.pc, e.temps), (pc, temps), "host instruction {k}");
    }
    assert_eq!(insns[1].1.imm, BASE + 0x17);
    assert_eq!(&t.code[insns[2].0..insns[3].0], [0x48, 0x8b, 0x06]);
}

#[test]
fn gs_operand_adds_the_guest_gs_base() {
    // mov gs:[rsp+8], rax
    let g = Guest::new(BASE, &[0x65, 0x48, 0x89, 0x44, 0x24, 0x08]);
    let t = tr(&g, NEAR);
    #[rustfmt::skip]
    let expect: &[u8] = &[
        0x65, 0x48, 0x89, 0x34, 0x25, 0xa0, 0x02, 0x00, 0x00, // mov gs:[scratch0], rsi
        0x48, 0x8d, 0x74, 0x24, 0x08,                         // lea rsi, [rsp+8]
        0x65, 0x48, 0x89, 0x04, 0x25, 0xa8, 0x02, 0x00, 0x00, // mov gs:[scratch1], rax
        0x65, 0x48, 0x8b, 0x04, 0x25, 0x98, 0x00, 0x00, 0x00, // mov rax, gs:[gs_base]
        0x48, 0x8d, 0x34, 0x06,                               // lea rsi, [rsi+rax]
        0x65, 0x48, 0x8b, 0x04, 0x25, 0xa8, 0x02, 0x00, 0x00, // mov rax, gs:[scratch1]
        0x48, 0x89, 0x06,                                     // mov [rsi], rax
        0x65, 0x48, 0x8b, 0x34, 0x25, 0xa0, 0x02, 0x00, 0x00, // mov rsi, gs:[scratch0]
    ];
    assert_eq!(&t.code[..expect.len()], expect);
}

#[test]
fn gs_pop_with_rsp_base_counts_the_pop() {
    // pop qword gs:[rsp+8]: the address uses RSP after the pop.
    let g = Guest::new(BASE, &[0x65, 0x8f, 0x44, 0x24, 0x08]);
    let t = tr(&g, NEAR);
    let insns = host_insns(&t, NEAR);
    // lea rsi, [rsp+8]; lea rsi, [rsi+8]
    assert_eq!(
        &t.code[insns[1].0..insns[3].0],
        [0x48, 0x8d, 0x74, 0x24, 0x08, 0x48, 0x8d, 0x76, 0x08]
    );
}

#[test]
fn inserted_code_never_writes_flags_nor_pushes() {
    // Guest instructions that write no flags and push nothing themselves, so every host
    // instruction must be neutral.
    let snippets: &[&[u8]] = &[
        // mov rax, [rip+0x10]; lea rax, [rip+0x10]
        &[0x48, 0x8b, 0x05, 0x10, 0x00, 0x00, 0x00],
        &[0x48, 0x8d, 0x05, 0x10, 0x00, 0x00, 0x00],
        // mov rax, gs:[0x28]; mov gs:[rsp+8], rax; mov eax, gs:[rip+0x10]
        &[0x65, 0x48, 0x8b, 0x04, 0x25, 0x28, 0x00, 0x00, 0x00],
        &[0x65, 0x48, 0x89, 0x44, 0x24, 0x08],
        &[0x65, 0x8b, 0x05, 0x10, 0x00, 0x00, 0x00],
        // mov rax, gs:[moffs64]; lea rax, gs:[0x10]
        &[0x65, 0x48, 0xa1, 0x10, 0, 0, 0, 0, 0, 0, 0],
        &[0x65, 0x48, 0x8d, 0x04, 0x25, 0x10, 0x00, 0x00, 0x00],
        // call rel32; call rax; call [rip+0x10]; call gs:[rbx]
        &[0xe8, 0x10, 0x00, 0x00, 0x00],
        &[0xff, 0xd0],
        &[0xff, 0x15, 0x10, 0x00, 0x00, 0x00],
        &[0x65, 0xff, 0x13],
        // jmp [r8+rax*8]; jmp [rsp+8]; jmp gs:[0x10]; notrack jmp rax
        &[0x41, 0xff, 0x24, 0xc0],
        &[0xff, 0x64, 0x24, 0x08],
        &[0x65, 0xff, 0x24, 0x25, 0x10, 0x00, 0x00, 0x00],
        &[0x3e, 0xff, 0xe0],
        // ret; ret 16; rep ret
        &[0xc3],
        &[0xc2, 0x10, 0x00],
        &[0xf3, 0xc3],
        // je, je rel32, jmp rel32, jrcxz, jecxz, loop
        &[0x74, 0x00],
        &[0x0f, 0x84, 0x00, 0x00, 0x00, 0x00],
        &[0xe9, 0x00, 0x00, 0x00, 0x00],
        &[0xe3, 0x00],
        &[0x67, 0xe3, 0x00],
        &[0xe2, 0x00],
        // syscall, cpuid, int3, ud2, an invalid instruction
        &[0x0f, 0x05],
        &[0x0f, 0xa2],
        &[0xcc],
        &[0x0f, 0x0b],
        &[0xc5, 0xf8, 0x77],
        // 64 instructions and a fallthrough exit
        &[0x90; 70],
    ];
    for &bytes in snippets {
        for base in [BASE, HIGH] {
            for host in [NEAR, FAR] {
                let g = Guest::new(base, bytes);
                let t = tr(&g, host);
                for (off, i) in host_insns(&t, host) {
                    assert!(
                        neutral(&i),
                        "{bytes:02x?} at {base:#x} on {host:#x}: host +{off:#x} {i:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn guest_flag_writers_are_the_only_ones() {
    // add rax, [rip+0x10] out of reach: the add, now `add rax, [rsi]`, is the only host
    // instruction that writes flags.
    let g = Guest::new(BASE, &[0x48, 0x03, 0x05, 0x10, 0x00, 0x00, 0x00]);
    let t = tr(&g, FAR);
    let writers: Vec<Insn> = host_insns(&t, FAR)
        .into_iter()
        .map(|(_, i)| i)
        .filter(|i| !neutral(i))
        .collect();
    assert_eq!(writers.len(), 1);
    assert_eq!((writers[0].opcode, writers[0].modrm), (0x03, Some(0x06)));
}

#[test]
fn block_map_finds_grows_and_clears() {
    let m = BlockMap::new();
    for i in 0..20_000u32 {
        m.insert(0x40_0000 + 16 * i as u64, i);
    }
    for i in 0..20_000u32 {
        assert_eq!(m.get(0x40_0000 + 16 * i as u64), Some(i));
    }
    assert_eq!(m.get(0x40_0008), None);
    m.insert(0x40_0000, 99);
    assert_eq!(m.get(0x40_0000), Some(99));
    m.clear();
    assert_eq!(m.get(0x40_0010), None);
}

/// Offset of the first host instruction of the probe: the save of RCX to scratch 0.
fn probe_start(insns: &[(usize, Insn)]) -> usize {
    insns
        .iter()
        .position(|(_, i)| {
            i.seg_gs
                && i.opcode == 0x89
                && i.disp as usize == off_scratch(0)
                && i.modrm.is_some_and(|m| (m >> 3) & 7 == RCX)
        })
        .expect("a probe")
}

/// `jmp rax; ret; call [rip+0x10]; ret 16`: every way into the probe.
const INDIRECT: &[&[u8]] = &[
    &[0xff, 0xe0],
    &[0xc3],
    &[0xff, 0x15, 0x10, 0x00, 0x00, 0x00],
    &[0xc2, 0x10, 0x00],
];

#[test]
fn probe_writes_no_flags_and_never_touches_the_guest_stack() {
    for &bytes in INDIRECT {
        for host in [NEAR, FAR] {
            let g = Guest::new(BASE, bytes);
            let t = tr(&g, host);
            let insns = host_insns(&t, host);
            let probe = &insns[probe_start(&insns)..];
            for (off, i) in probe {
                assert!(neutral(i), "{bytes:02x?}: host +{off:#x} {i:?}");
                // No RSP-based operand: the probe reads only `Cpu` and the table.
                let rsp_base = i.modrm.is_some_and(|m| m >> 6 != 3 && m & 7 == 4)
                    && i.sib.is_some_and(|s| s & 7 == 4);
                assert!(!rsp_base, "{bytes:02x?}: host +{off:#x} {i:?}");
            }
            let jrcxz = probe.iter().filter(|(_, i)| i.opcode == 0xe3).count();
            assert_eq!(jrcxz, 2, "{bytes:02x?}");
            // The two misses go to the indirect exit; the hit jumps through scratch 2.
            let exits: Vec<u64> = probe
                .iter()
                .filter(|(_, i)| i.opcode == 0xe9)
                .map(|(_, i)| i.target.unwrap())
                .collect();
            assert_eq!(exits, [host - 0x80, host - 0x80], "{bytes:02x?}");
            let (_, last) = probe.last().unwrap();
            assert_eq!((last.opcode, last.seg_gs), (0xff, true));
            assert_eq!(last.disp as usize, off_scratch(2));
            assert_eq!(last.class, Class::IndirectBranch);
        }
    }
}

#[test]
fn probe_indexes_the_table_by_the_low_bits_and_compares_with_lea() {
    let g = Guest::new(BASE, &[0xff, 0xe0]);
    let t = tr(&g, NEAR);
    let insns = host_insns(&t, NEAR);
    let probe: Vec<Insn> = insns[probe_start(&insns)..]
        .iter()
        .map(|(_, i)| *i)
        .collect();
    let movzx: Vec<usize> = probe
        .iter()
        .filter(|i| i.map == Map::Map0F && i.opcode == 0xb7)
        .map(|i| i.disp as usize)
        .collect();
    // Bits 48–63 first, then the slot (bits 0–15).
    assert_eq!(movzx, [OFF_NEXT_RIP + 6, OFF_NEXT_RIP]);
    let imm64: Vec<u64> = probe
        .iter()
        .filter(|i| i.opcode == 0xb8 && i.imm_size == 8)
        .map(|i| i.imm)
        .collect();
    assert_eq!(imm64, [LOOKUP_TABLE, NEAR - 0x100]);
    let code = |a: usize, b: usize| t.code[a..b].to_vec();
    let at = |k: usize| insns[probe_start(&insns) + k].0;
    // ... movzx ecx, word gs:[next_rip]; mov rax, table; mov rax, [rax+rcx*8];
    // mov ecx, dword gs:[next_rip+2]; lea ecx, [rcx+rax]; jrcxz
    let k = probe
        .iter()
        .position(|i| i.opcode == 0xb7 && i.disp as usize == OFF_NEXT_RIP)
        .unwrap();
    assert_eq!(code(at(k + 2), at(k + 3)), [0x48, 0x8b, 0x04, 0xc8]);
    let load = probe[k + 3];
    assert_eq!(
        (load.opcode, load.seg_gs, load.disp as usize),
        (0x8b, true, OFF_NEXT_RIP + 2)
    );
    assert_eq!(code(at(k + 4), at(k + 5)), [0x8d, 0x0c, 0x01]);
    assert_eq!(probe[k + 5].opcode, 0xe3);
}

#[test]
fn probe_pc_table_keeps_next_rip_and_its_temporaries() {
    let n = [None; SCRATCH_SLOTS];
    let c = [Some(RCX), None, None, None];
    let b = [Some(RCX), Some(RAX), None, None];
    // Temporaries in scratch at each host instruction of the probe.
    #[rustfmt::skip]
    let expect = [
        n, // mov gs:[scratch0], rcx
        c, // movzx ecx, word gs:[next_rip+6]
        c, // jrcxz
        c, // mov rcx, gs:[scratch0]
        n, // jmp indirect_exit
        c, // mov gs:[scratch1], rax
        b, // movzx ecx, word gs:[next_rip]
        b, // mov rax, table
        b, // mov rax, [rax+rcx*8]
        b, // mov ecx, dword gs:[next_rip+2]
        b, // lea ecx, [rcx+rax]
        b, // jrcxz
        b, // mov rax, gs:[scratch1]
        c, // mov rcx, gs:[scratch0]
        n, // jmp indirect_exit
        b, // mov gs:[scratch2], rax
        b, // mov ecx, dword gs:[scratch2+4]
        b, // mov rax, cache_base
        b, // lea rcx, [rcx+rax]
        b, // mov gs:[scratch2], rcx
        b, // mov rax, gs:[scratch1]
        c, // mov rcx, gs:[scratch0]
        n, // jmp gs:[scratch2]
    ];
    for &bytes in INDIRECT {
        let g = Guest::new(BASE, bytes);
        let t = tr(&g, NEAR);
        let insns = host_insns(&t, NEAR);
        let probe = &insns[probe_start(&insns)..];
        assert_eq!(probe.len(), expect.len(), "{bytes:02x?}");
        for (k, ((off, _), temps)) in probe.iter().zip(expect).enumerate() {
            let e = t.lookup(*off as u32).unwrap();
            assert_eq!(
                (e.pc, e.temps),
                (GuestPc::NextRip, temps),
                "{bytes:02x?}: {k}"
            );
        }
        // Each `jrcxz` lands on the save or spill that follows a miss path.
        for (k, (_, i)) in probe.iter().enumerate() {
            if i.opcode == 0xe3 {
                let landing = i.target.unwrap() - NEAR;
                let next_miss = probe[k..]
                    .iter()
                    .position(|(_, j)| j.opcode == 0xe9)
                    .unwrap();
                assert_eq!(landing as usize, probe[k + next_miss + 1].0, "{bytes:02x?}");
            }
        }
    }
}

/// What the probe does with `entry` for the guest `target`: the cache offset it jumps to,
/// or `None` if it leaves to the dispatcher. The same steps as the emitted code.
fn probe_hit(entry: u64, target: u64) -> Option<u32> {
    if target >> 48 != 0 {
        return None;
    }
    let ecx = ((target >> 16) as u32).wrapping_add(entry as u32);
    (ecx == 0).then_some((entry >> 32) as u32)
}

#[test]
fn lookup_entries_are_indexed_by_the_low_bits_and_published_whole() {
    assert!(LOOKUP_SLOTS.is_power_of_two());
    assert_eq!(
        LOOKUP_SLOTS,
        1 << 16,
        "the probe takes the slot with movzx of a word"
    );
    const EXIT: u32 = 0x10;
    let table = LookupTable::new(EXIT);
    let rip = 0x1000_0040_1234u64;
    assert_eq!(lookup_slot(rip), 0x1234);
    // Free slots send every target to the indirect exit, low rips included.
    for target in [rip, 0x1234, 0, 0x40_0000] {
        assert_eq!(probe_hit(table.get(target), target), Some(EXIT));
    }
    table.set(rip, 0x4560);
    let entry = table.get(rip);
    assert_eq!(entry, lookup_entry(rip, 0x4560));
    // The probe reads `[table + slot * 8]`.
    let raw = unsafe { *((table.addr() + 8 * 0x1234) as *const u64) };
    assert_eq!(raw, entry);
    assert_eq!(probe_hit(entry, rip), Some(0x4560));
    // Same slot, another rip; same bits 0–47 with bits 48–63 set.
    assert_eq!(probe_hit(entry, rip + 0x1_0000), None);
    assert_eq!(probe_hit(entry, rip + 0x10_0000_0000), None);
    assert_eq!(probe_hit(entry, rip | 1 << 48), None);
    assert_eq!(probe_hit(entry, rip | 0xffff << 48), None);
    // A rip that shares the slot replaces the entry.
    table.set(rip + 0x1_0000, 0x8000);
    assert_eq!(probe_hit(table.get(rip), rip), None);
    assert_eq!(probe_hit(table.get(rip), rip + 0x1_0000), Some(0x8000));
    // Other slots are untouched.
    assert_eq!(probe_hit(table.get(rip + 1), rip + 1), Some(EXIT));
}

#[test]
fn invalidating_and_flushing_empty_the_lookup_table() {
    const EXIT: u32 = 0x10;
    let table = LookupTable::new(EXIT);
    let (a, b) = (0x40_1000u64, 0x40_2008u64);
    table.set(a, 0x100);
    table.set(b, 0x200);
    // A block that no longer owns its slot does not empty it.
    table.remove(a, 0x180);
    assert_eq!(probe_hit(table.get(a), a), Some(0x100));
    table.remove(a, 0x100);
    assert_eq!(probe_hit(table.get(a), a), Some(EXIT));
    assert_eq!(probe_hit(table.get(b), b), Some(0x200));
    // Replaced by another rip, then that rip's old block is invalidated: the new one stays.
    table.set(b + 0x1_0000, 0x300);
    table.remove(b, 0x200);
    assert_eq!(probe_hit(table.get(b), b + 0x1_0000), Some(0x300));
    table.clear();
    for rip in [a, b, b + 0x1_0000] {
        assert_eq!(probe_hit(table.get(rip), rip), Some(EXIT));
    }
}
