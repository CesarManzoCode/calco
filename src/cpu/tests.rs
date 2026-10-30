use super::*;

#[test]
fn offsets_are_stable() {
    assert_eq!(OFF_GPR, 0);
    assert_eq!(off_gpr(RSP), 32);
    assert_eq!(OFF_RIP, 128);
    assert_eq!(OFF_RFLAGS, 136);
    assert_eq!(OFF_FS_BASE, 144);
    assert_eq!(OFF_GS_BASE, 152);
    assert_eq!(OFF_FXSAVE, 160);
    assert_eq!(OFF_SCRATCH, 672);
    assert_eq!(off_scratch(3), 696);
    assert_eq!(OFF_HOST_RSP, 704);
    assert_eq!(OFF_HOST_FS, 712);
    assert_eq!(OFF_NEXT_RIP, 720);
    assert_eq!(OFF_EXIT_REASON, 728);
    assert_eq!(OFF_PENDING_SIGNALS, 736);
    assert_eq!(OFF_SIGNAL_STATE, 744);
    assert_eq!(OFF_TID, 752);
    assert_eq!(OFF_CLEAR_CHILD_TID, 760);
    assert_eq!(offset_of!(Cpu, smc_write), 768);
    assert_eq!(offset_of!(Cpu, smc_step), 776);
    assert_eq!(core::mem::size_of::<Cpu>(), 784);
}

#[test]
fn fxsave_is_aligned() {
    assert_eq!(core::mem::align_of::<FxArea>(), 16);
    assert_eq!(core::mem::size_of::<FxArea>(), 512);
    let cpu = Cpu::new(0x40_1000, 0x3fff_ffff_e000);
    assert_eq!(&cpu.fxsave as *const FxArea as usize % 16, 0);
}

#[test]
fn initial_state() {
    let cpu = Cpu::new(0x40_1000, 0x3fff_ffff_e000);
    assert_eq!(cpu.rip, 0x40_1000);
    assert_eq!(cpu.gpr[RSP], 0x3fff_ffff_e000);
    assert_eq!(cpu.rflags, RFLAGS_INIT);
    assert_eq!(
        u16::from_le_bytes([cpu.fxsave.0[0], cpu.fxsave.0[1]]),
        0x037f
    );
    let mxcsr: [u8; 4] = cpu.fxsave.0[24..28].try_into().unwrap();
    assert_eq!(u32::from_le_bytes(mxcsr), 0x1f80);
    assert!(cpu.fxsave.0[28..].iter().all(|&b| b == 0));
}
