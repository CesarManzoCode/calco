//! Compiles `guests/signals.c` static (`-O2 -static`, contract D21) and checks that under
//! calco it behaves like the native run: same stdout and same exit status. The guest
//! takes SIGSEGV, SIGILL and SIGFPE from the CPU and SIGALRM in the middle of a loop, and
//! prints what its handlers see (k2-02).

mod common;

#[test]
fn signals_match_native() {
    let exe = common::cc("signals", "signals.c", "signals", &["-O2", "-static"]);
    let native = common::native(&exe, &[], &[]);
    assert!(native.status.success(), "native: {}", native.status);
    common::assert_same(&native, &common::calco(&exe, &[], &[]), "");
}
