//! Assembles `guests/asm/indirect.S` without libc and runs it under calco (k3-03): a jump
//! table, recursion with `ret` and an indirect call, all through the inline lookup of
//! indirect transfers, with flags live across `ret` and `jmp`.

mod common;

#[test]
fn indirect_guest_runs_translated() {
    let exe = common::asm("jit_indirect", "indirect");
    let out = common::calco(&exe, &[], &[]);
    common::assert_run(&out, "table 2454267021 fib 6765 call 55\n", Some(0), "");
}
