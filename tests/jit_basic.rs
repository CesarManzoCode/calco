//! Assembles `guests/asm/basic.S` without libc and runs it under calco (k1-03): the first
//! complete path through the translator, the code cache, the dispatcher and `syscall`.

mod common;

#[test]
fn basic_guest_runs_translated() {
    let exe = common::asm("jit_basic", "basic");
    let out = common::calco(&exe, &[], &[]);
    common::assert_run(&out, "calco jit basic ok\n", Some(42), "");
}
