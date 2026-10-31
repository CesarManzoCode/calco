//! Compiles `guests/smc.c` static (`-O2 -static`, contract D21) and checks that under calco
//! it behaves like the native run: same stdout and same exit status. The guest rewrites
//! code in a RWX page between calls, rewrites an instruction ahead of it in its own block,
//! and changes the page with `mprotect`, `munmap`, `madvise(MADV_DONTNEED)` and `mremap`
//! between calls (k2-04).

mod common;

#[test]
fn smc_matches_native() {
    let exe = common::cc("smc", "smc.c", "smc", &["-O2", "-static"]);
    let native = common::native(&exe, &[], &[]);
    assert!(native.status.success(), "native: {}", native.status);
    common::assert_same(&native, &common::calco(&exe, &[], &[]), "");
}
