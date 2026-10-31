use super::frame::*;
use super::*;
use crate::cpu::RBX;
use crate::jit::translate::{Env, GuestCode, Translation, translate};

#[test]
fn guest_numbers_map_like_the_contract() {
    assert_eq!(host_signal(0), Some(0));
    assert_eq!(host_signal(SIGSEGV as u64), Some(11));
    assert_eq!(host_signal(31), Some(31));
    assert_eq!(host_signal(32), Some(64));
    assert_eq!(host_signal(33), Some(63));
    assert_eq!(host_signal(34), Some(34));
    assert_eq!(host_signal(61), Some(61));
    for sig in [62, 63, 64, 65] {
        assert_eq!(host_signal(sig), None, "guest {sig}");
    }
    assert_eq!(guest_signal(64), 32);
    assert_eq!(guest_signal(63), 33);
    assert_eq!(guest_signal(17), 17);
    for sig in (1..=61).filter(|&s| s != 32 && s != 33) {
        assert_eq!(guest_signal(host_signal(sig).unwrap() as u32), sig as u32);
    }
}

#[test]
fn masks_are_translated_both_ways() {
    let guest = bit(1) | bit(SIGSEGV) | bit(31) | bit(32) | bit(33) | bit(34) | bit(61);
    let host = bit(1) | bit(SIGSEGV) | bit(31) | bit(64) | bit(63) | bit(34) | bit(61);
    assert_eq!(host_set(guest), host);
    assert_eq!(guest_set(host), guest);
    // Guest 62–64 have no host signal; host 32, 33 and calco's 62 no guest one.
    assert_eq!(host_set(bit(62) | bit(63) | bit(64)), 0);
    assert_eq!(guest_set(bit(32) | bit(33) | bit(HOST_KICK)), 0);
    // The host never blocks the CPU's signals nor calco's own.
    let all = host_mask(!0);
    assert_eq!(all & SYNC_SIGNALS, 0);
    assert_eq!(all & bit(HOST_KICK), 0);
    assert_eq!(all & bit(10), bit(10));
    assert_eq!(host_mask(bit(32)), bit(64));
}

#[test]
fn thread_mask_queue_and_pending_word() {
    let mut cpu = Cpu::new(0, 0);
    let cpu: *mut Cpu = &mut *cpu;
    let th = Thread::new(0);
    th.set_mask_quiet(cpu, bit(10) | UNBLOCKABLE);
    assert_eq!(th.mask(), bit(10), "SIGKILL and SIGSTOP cannot be blocked");
    let info = |sig| SigInfo::kernel(sig, 0, 0);
    assert!(!th.queue(cpu, 10, info(10)), "blocked: not deliverable");
    assert_eq!(unsafe { (*cpu).pending_signals }, 0);
    assert!(th.queue(cpu, 12, info(12)));
    assert!(th.queue(cpu, 32, info(32)));
    assert_eq!(unsafe { (*cpu).pending_signals }, bit(12) | bit(32));
    assert!(th.queue(cpu, SIGSEGV, info(SIGSEGV)));
    // Synchronous first, then the lowest number, as the kernel dequeues.
    let order: Vec<u32> = std::iter::from_fn(|| th.take(cpu, !th.mask()).map(|(s, _)| s)).collect();
    assert_eq!(order, [SIGSEGV, 12, 32]);
    assert_eq!(th.queued(), bit(10));
    assert_eq!(unsafe { (*cpu).pending_signals }, 0);
    // Unblocking makes the queued signal deliverable, with its siginfo.
    th.set_mask_quiet(cpu, 0);
    assert_eq!(unsafe { (*cpu).pending_signals }, bit(10));
    let (sig, got) = th.take(cpu, !0).unwrap();
    assert_eq!((sig, got), (10, info(10)));
    assert_eq!(th.queued(), 0);
}

#[test]
fn host_siginfo_gets_guest_numbers() {
    let mut raw = [0u8; SIGINFO_SIZE];
    put32(&mut raw, SI_SIGNO, 64);
    put32(&mut raw, SI_CODE, -6i32 as u32); // SI_TKILL
    assert_eq!(get32(&SigInfo::from_host(&raw, 64).0, SI_SIGNO), 32);
    // SIGCHLD of a child killed by host 63 (CLD_KILLED = 2).
    let mut raw = [0u8; SIGINFO_SIZE];
    put32(&mut raw, SI_SIGNO, SIGCHLD);
    put32(&mut raw, SI_CODE, 2);
    put32(&mut raw, SI_STATUS, 63);
    assert_eq!(get32(&SigInfo::from_host(&raw, SIGCHLD).0, SI_STATUS), 33);
}

#[test]
fn interrupted_calls_restart_like_the_kernel() {
    let eintr = -(EINTR as i64);
    let skipped = -ERESTART_INTERNAL;
    // read: by SA_RESTART after a handler, always without one.
    assert!(restarts(0, eintr, Some(SA_RESTART)));
    assert!(!restarts(0, eintr, Some(0)));
    assert!(restarts(0, eintr, None));
    // nanosleep and rt_sigsuspend end with EINTR after any handler.
    assert!(!restarts(35, eintr, Some(SA_RESTART)));
    assert!(!restarts(130, eintr, Some(SA_RESTART)));
    // A call the signal came before never ran: it always runs after the handler.
    assert!(restarts(35, skipped, Some(0)));
    assert!(restarts(1, skipped, Some(0)));
}

#[test]
fn default_actions_follow_signal_7() {
    assert_eq!(default_action(SIGSEGV), DefaultAction::Core);
    assert_eq!(default_action(SIGCHLD), DefaultAction::Ignore);
    assert_eq!(default_action(SIGSTOP), DefaultAction::Stop);
    assert_eq!(default_action(14), DefaultAction::Term);
    assert_eq!(default_action(34), DefaultAction::Term);
}

/// The C expression of every layout constant of `frame.rs`, from the installed headers.
fn header_layout() -> Vec<(&'static str, String, u64)> {
    let mut v: Vec<(&'static str, String, u64)> = vec![
        (
            "uc_flags",
            "offsetof(struct ucontext, uc_flags)".into(),
            UC_FLAGS as u64,
        ),
        (
            "uc_link",
            "offsetof(struct ucontext, uc_link)".into(),
            UC_LINK as u64,
        ),
        (
            "uc_stack",
            "offsetof(struct ucontext, uc_stack)".into(),
            UC_STACK as u64,
        ),
        (
            "uc_mcontext",
            "offsetof(struct ucontext, uc_mcontext)".into(),
            UC_MCONTEXT as u64,
        ),
        (
            "uc_sigmask",
            "offsetof(struct ucontext, uc_sigmask)".into(),
            UC_SIGMASK as u64,
        ),
        ("ucontext", "sizeof(struct ucontext)".into(), UC_SIZE as u64),
        ("ss_sp", "offsetof(stack_t, ss_sp)".into(), SS_SP as u64),
        (
            "ss_flags",
            "offsetof(stack_t, ss_flags)".into(),
            SS_FLAGS as u64,
        ),
        (
            "ss_size",
            "offsetof(stack_t, ss_size)".into(),
            SS_SIZE as u64,
        ),
        ("stack_t", "sizeof(stack_t)".into(), STACK_T_SIZE as u64),
        (
            "rip",
            "offsetof(struct sigcontext, rip)".into(),
            SC_RIP as u64,
        ),
        (
            "eflags",
            "offsetof(struct sigcontext, eflags)".into(),
            SC_EFLAGS as u64,
        ),
        ("cs", "offsetof(struct sigcontext, cs)".into(), SC_CS as u64),
        ("gs", "offsetof(struct sigcontext, gs)".into(), SC_GS as u64),
        ("fs", "offsetof(struct sigcontext, fs)".into(), SC_FS as u64),
        ("ss", "offsetof(struct sigcontext, ss)".into(), SC_SS as u64),
        (
            "err",
            "offsetof(struct sigcontext, err)".into(),
            SC_ERR as u64,
        ),
        (
            "trapno",
            "offsetof(struct sigcontext, trapno)".into(),
            SC_TRAPNO as u64,
        ),
        (
            "oldmask",
            "offsetof(struct sigcontext, oldmask)".into(),
            SC_OLDMASK as u64,
        ),
        (
            "cr2",
            "offsetof(struct sigcontext, cr2)".into(),
            SC_CR2 as u64,
        ),
        (
            "fpstate",
            "offsetof(struct sigcontext, fpstate)".into(),
            SC_FPSTATE as u64,
        ),
        (
            "sigcontext",
            "sizeof(struct sigcontext)".into(),
            SC_SIZE as u64,
        ),
        (
            "si_signo",
            "offsetof(siginfo_t, si_signo)".into(),
            SI_SIGNO as u64,
        ),
        (
            "si_code",
            "offsetof(siginfo_t, si_code)".into(),
            SI_CODE as u64,
        ),
        (
            "si_addr",
            "offsetof(siginfo_t, si_addr)".into(),
            SI_ADDR as u64,
        ),
        (
            "si_status",
            "offsetof(siginfo_t, si_status)".into(),
            SI_STATUS as u64,
        ),
        ("siginfo_t", "sizeof(siginfo_t)".into(), SIGINFO_SIZE as u64),
        (
            "pretcode",
            "offsetof(struct rt_sigframe, pretcode)".into(),
            FRAME_PRETCODE as u64,
        ),
        (
            "frame_uc",
            "offsetof(struct rt_sigframe, uc)".into(),
            FRAME_UC as u64,
        ),
        (
            "frame_info",
            "offsetof(struct rt_sigframe, info)".into(),
            FRAME_INFO as u64,
        ),
        (
            "rt_sigframe",
            "sizeof(struct rt_sigframe)".into(),
            FRAME_SIZE as u64,
        ),
        (
            "cwd",
            "offsetof(struct _fpstate_64, cwd)".into(),
            FP_CWD as u64,
        ),
        (
            "swd",
            "offsetof(struct _fpstate_64, swd)".into(),
            FP_SWD as u64,
        ),
        (
            "twd",
            "offsetof(struct _fpstate_64, twd)".into(),
            FP_TWD as u64,
        ),
        (
            "mxcsr",
            "offsetof(struct _fpstate_64, mxcsr)".into(),
            FP_MXCSR as u64,
        ),
        (
            "st_space",
            "offsetof(struct _fpstate_64, st_space)".into(),
            FP_ST as u64,
        ),
        (
            "xmm_space",
            "offsetof(struct _fpstate_64, xmm_space)".into(),
            FP_XMM as u64,
        ),
        (
            "sw_reserved",
            "offsetof(struct _fpstate_64, sw_reserved)".into(),
            FP_SW_RESERVED as u64,
        ),
        (
            "_fpstate_64",
            "sizeof(struct _fpstate_64)".into(),
            FP_SIZE as u64,
        ),
        (
            "FP_XSTATE_MAGIC1",
            "FP_XSTATE_MAGIC1".into(),
            FP_XSTATE_MAGIC1 as u64,
        ),
        (
            "UC_SIGCONTEXT_SS",
            "UC_SIGCONTEXT_SS".into(),
            UC_SIGCONTEXT_SS,
        ),
        (
            "UC_STRICT_RESTORE_SS",
            "UC_STRICT_RESTORE_SS".into(),
            UC_STRICT_RESTORE_SS,
        ),
        ("SS_ONSTACK", "SS_ONSTACK".into(), SS_ONSTACK as u64),
        ("SS_DISABLE", "SS_DISABLE".into(), SS_DISABLE as u64),
        (
            "SS_AUTODISARM",
            "SS_AUTODISARM".into(),
            SS_AUTODISARM as u64,
        ),
        ("MINSIGSTKSZ", "MINSIGSTKSZ".into(), MINSIGSTKSZ),
        ("SA_SIGINFO", "SA_SIGINFO".into(), SA_SIGINFO),
        ("SA_ONSTACK", "SA_ONSTACK".into(), SA_ONSTACK),
        ("SA_RESTART", "SA_RESTART".into(), SA_RESTART),
        ("SA_NODEFER", "SA_NODEFER".into(), SA_NODEFER),
        ("SA_RESETHAND", "SA_RESETHAND".into(), SA_RESETHAND),
        ("SA_RESTORER", "SA_RESTORER".into(), SA_RESTORER),
    ];
    let names = [
        "rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12",
        "r13", "r14", "r15",
    ];
    for (reg, name) in names.iter().enumerate() {
        let expr = format!("offsetof(struct ucontext, uc_mcontext.{name})");
        v.push((name, expr, uc_gpr(reg) as u64));
    }
    v
}

#[test]
fn frame_layout_matches_the_kernel_headers() {
    let layout = header_layout();
    let mut c = String::from(
        "#include <stddef.h>\n\
         #include <asm/signal.h>\n\
         #include <asm/sigcontext.h>\n\
         #include <asm/ucontext.h>\n\
         #include <asm/siginfo.h>\n\
         #include <linux/signal.h>\n\
         int printf(const char *, ...);\n\
         /* arch/x86/include/asm/sigframe.h, built from the uapi structs. */\n\
         struct rt_sigframe { char *pretcode; struct ucontext uc; siginfo_t info; };\n\
         int main(void) {\n",
    );
    for (name, expr, _) in &layout {
        c.push_str(&format!(
            "    printf(\"%s %lu\\n\", \"{name}\", (unsigned long)({expr}));\n"
        ));
    }
    c.push_str("    return 0;\n}\n");

    let dir = std::env::temp_dir().join(format!("calco-sigframe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (src, exe) = (dir.join("layout.c"), dir.join("layout"));
    std::fs::write(&src, c).unwrap();
    let cc = std::process::Command::new("cc")
        .arg("-o")
        .arg(&exe)
        .arg(&src)
        .output()
        .expect("cannot run cc");
    assert!(
        cc.status.success(),
        "{}",
        String::from_utf8_lossy(&cc.stderr)
    );
    let out = std::process::Command::new(&exe).output().unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    let out = String::from_utf8(out.stdout).unwrap();

    let got: Vec<(&str, u64)> = out
        .lines()
        .map(|l| {
            let (name, v) = l.split_once(' ').unwrap();
            (name, v.parse().unwrap())
        })
        .collect();
    let want: Vec<(&str, u64)> = layout.iter().map(|(n, _, v)| (*n, *v)).collect();
    assert_eq!(got, want);
}

#[test]
fn built_frame_has_the_state_where_the_kernel_puts_it() {
    let gpr: [u64; 16] = core::array::from_fn(|r| 0x1000 + r as u64);
    let saved = Saved {
        gpr: &gpr,
        rip: 0x40_1234,
        rflags: 0x246,
        err: 7,
        trapno: 14,
        cr2: 0xdead_0000,
    };
    let alt = AltStack {
        sp: 0x7000_0000,
        size: 0x4000,
        flags: 0,
    };
    let info = SigInfo::kernel(SIGSEGV, SEGV_ACCERR, 0xdead_0000);
    let f = build(&saved, bit(10), &alt, 0x40_9999, 0x7fff_f000, &info.0);
    assert_eq!(get64(&f, FRAME_PRETCODE), 0x40_9999);
    let uc = &f[FRAME_UC..FRAME_INFO];
    assert_eq!(get64(uc, uc_gpr(RBX)), 0x1003);
    assert_eq!(get64(uc, uc_gpr(RSP)), 0x1004);
    assert_eq!(get64(uc, UC_RIP), 0x40_1234);
    assert_eq!(get64(uc, UC_EFLAGS), 0x246);
    assert_eq!(get64(uc, UC_TRAPNO), 14);
    assert_eq!(get64(uc, UC_ERR), 7);
    assert_eq!(get64(uc, UC_CR2), 0xdead_0000);
    assert_eq!(get64(uc, UC_FPSTATE), 0x7fff_f000);
    assert_eq!(get64(uc, UC_SIGMASK), bit(10));
    assert_eq!(get64(uc, UC_MCONTEXT + SC_OLDMASK), bit(10));
    assert_eq!(get16(uc, UC_MCONTEXT + SC_CS), USER_CS);
    assert_eq!(get16(uc, UC_MCONTEXT + SC_SS), USER_DS);
    assert_eq!(get64(uc, UC_STACK + SS_SP), 0x7000_0000);
    assert_eq!(get64(uc, UC_STACK + SS_SIZE), 0x4000);
    let si = &f[FRAME_INFO..];
    assert_eq!(get32(si, SI_SIGNO), SIGSEGV);
    assert_eq!(get32(si, SI_CODE), SEGV_ACCERR as u32);
    assert_eq!(get64(si, SI_ADDR), 0xdead_0000);

    // rt_sigreturn reads back what the frame holds.
    let r = restore(uc);
    assert_eq!(r.gpr, gpr);
    assert_eq!((r.rip, r.eflags, r.mask), (0x40_1234, 0x246, bit(10)));
    assert_eq!(r.fpstate, 0x7fff_f000);
    assert_eq!(r.stack, (0x7000_0000, 0, 0x4000));
}

#[test]
fn fpstate_says_there_is_no_xsave() {
    let mut fx = [0xffu8; FP_SIZE];
    put32(&mut fx, FP_SW_RESERVED, FP_XSTATE_MAGIC1);
    let out = fpstate(&fx);
    assert_eq!(get32(&out, FP_SW_RESERVED), 0, "magic1 = 0");
    assert_eq!(&out[..FP_SW_RESERVED], &fx[..FP_SW_RESERVED]);
    let init = fx_init();
    assert_eq!(get16(&init, FP_CWD), FCW_INIT);
    assert_eq!(get32(&init, FP_MXCSR), MXCSR_INIT);
}

#[test]
fn frames_go_below_the_red_zone_or_on_the_alternate_stack() {
    let rsp = 0x7fff_1238;
    let (frame, fp) = place(rsp, false, &AltStack::DISABLED).unwrap();
    assert_eq!(fp % 64, 0);
    assert!(
        fp + FP_SIZE as u64 <= rsp - 128,
        "the red zone stays intact"
    );
    assert!(frame + FRAME_SIZE as u64 <= fp);
    // The handler starts as after a `call`: (rsp + 8) % 16 == 0.
    assert_eq!((frame + 8) % 16, 0);

    let alt = AltStack {
        sp: 0x5000_0000,
        size: 0x2000,
        flags: 0,
    };
    let (frame, fp) = place(rsp, true, &alt).unwrap();
    assert!(alt.contains(frame) && fp + FP_SIZE as u64 <= alt.sp + alt.size);
    // Already on it: nested below the current RSP.
    let inner = alt.sp + 0x1800;
    let (frame, _) = place(inner, true, &alt).unwrap();
    assert!(frame < inner - 128 && alt.contains(frame));
    // Without room left the kernel forces SIGSEGV.
    assert_eq!(place(alt.sp + 0x200, true, &alt), None);
    // Without SA_ONSTACK the alternate stack is not used.
    let (frame, _) = place(rsp, false, &alt).unwrap();
    assert!(frame > alt.sp + alt.size);
}

#[test]
fn altstack_modes_follow_sigaltstack_2() {
    let alt = AltStack {
        sp: 0x5000_0000,
        size: 0x2000,
        flags: 0,
    };
    assert_eq!(AltStack::DISABLED.mode(0x1234), SS_DISABLE);
    assert_eq!(alt.mode(0x5000_1000), SS_ONSTACK);
    assert_eq!(alt.mode(0x6000_0000), 0);
    let auto = AltStack {
        flags: SS_AUTODISARM,
        ..alt
    };
    assert_eq!(auto.mode(0x5000_1000), 0, "never on it with SS_AUTODISARM");

    let th = Thread::new(0);
    assert_eq!(set_altstack(&th, alt.sp, 0, 1024, 0), Err(ENOMEM));
    assert_eq!(set_altstack(&th, alt.sp, 7, alt.size, 0), Err(EINVAL));
    assert_eq!(set_altstack(&th, alt.sp, 0, alt.size, 0), Ok(()));
    assert_eq!(th.altstack(), alt);
    // Changing it while running on it fails.
    assert_eq!(set_altstack(&th, 0, SS_DISABLE, 0, 0x5000_1000), Err(EPERM));
}

/// Guest code at `BASE`.
struct Guest(Vec<u8>);

const BASE: u64 = 0x40_1000;

impl GuestCode for Guest {
    fn fetch(&self, rip: u64, buf: &mut [u8]) -> usize {
        let Some(off) = rip.checked_sub(BASE).map(|o| o as usize) else {
            return 0;
        };
        let n = buf.len().min(self.0.len().saturating_sub(off));
        buf[..n].copy_from_slice(&self.0[off..off + n]);
        n
    }
}

fn translated(bytes: &[u8]) -> Translation {
    let env = Env {
        host: 0x2000_0000,
        exit_thunk: 0x2000_0000 - 0x100,
        indirect_exit: 0x2000_0000 - 0x80,
        lookup_table: 0x5000_0000,
        cache_base: 0x2000_0000 - 0x100,
        first_exit_id: 0,
    };
    translate(&Guest(bytes.to_vec()), BASE, &env).unwrap()
}

#[test]
fn state_is_rebuilt_from_the_pc_table_with_scratch_temporaries() {
    // mov gs:[rsp+8], rax; nop. The store goes through RSI (scratch 0) and RAX holds the
    // GS base for a moment (scratch 1).
    let t = translated(&[0x65, 0x48, 0x89, 0x44, 0x24, 0x08, 0x90]);
    // Host instructions of the expansion: save rsi, lea, save rax, load gs_base, lea,
    // restore rax, the store, restore rsi.
    let lens = [9u32, 5, 9, 9, 4, 9, 3, 9];
    let at: Vec<u32> = lens
        .iter()
        .scan(0, |off, len| {
            let start = *off;
            *off += len;
            Some(start)
        })
        .collect();
    let end = at[7] + lens[7];
    let table = &t.pcs;
    for off in 0..end + 1 {
        assert_eq!(entry_at(table, off), t.lookup(off), "offset {off}");
    }

    let guest: [u64; 16] = core::array::from_fn(|r| 0x100 * (r as u64 + 1));
    let scratch = [guest[RSI], guest[RAX], 0xbad, 0xbad];
    // What the host registers hold at `off`: the guest ones but for the temporaries.
    let host = |rsi: u64, rax: u64| {
        let mut g = guest;
        g[RSI] = rsi;
        g[RAX] = rax;
        g
    };
    let check = |off: u32, regs: [u64; 16], rip: u64, boundary: bool| {
        let mut gpr = regs;
        let entry = entry_at(table, off).unwrap();
        assert_eq!(
            reconstruct(entry, &mut gpr, &scratch, 0),
            rip,
            "offset {off}"
        );
        assert_eq!(gpr, guest, "registers at offset {off}");
        assert_eq!(
            at_boundary(table, off),
            boundary,
            "boundary at offset {off}"
        );
    };
    check(0, guest, BASE, true);
    // RSI holds the address; then RAX the GS base too.
    check(at[2], host(0x7008, guest[RAX]), BASE, false);
    check(at[4], host(0x7008, 0x9000), BASE, false);
    // The store faults with RSI still in scratch: the state is the one before it.
    check(at[6], host(0x9008, guest[RAX]), BASE, false);
    // After the store, before RSI comes back: already the next boundary.
    check(at[7], host(0x9008, guest[RAX]), BASE + 6, false);
    // The nop starts with every register in place.
    check(end, guest, BASE + 6, true);
}

#[test]
fn indirect_exits_take_the_rip_from_next_rip() {
    // ret
    let t = translated(&[0xc3]);
    let last = *t.pcs.last().unwrap();
    assert_eq!(last.pc, GuestPc::NextRip);
    let mut gpr = [0; 16];
    let rip = reconstruct(&last, &mut gpr, &[0; SCRATCH_SLOTS], 0x40_5555);
    assert_eq!(rip, 0x40_5555);
    assert!(at_boundary(&t.pcs, last.host_off));
}
