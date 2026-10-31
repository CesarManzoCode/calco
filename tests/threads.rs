//! Compiles `guests/threads.c` static with `-pthread` (`-O2 -static`, contract D21) and
//! checks that a few runs under calco give the same stdout and exit status as the native
//! run: 8 threads with a mutex, condition variables, `__thread`, an atomic counter,
//! `pthread_join`, and signals to one thread and to the process (k2-03). The 50 runs of
//! the criterion are `scripts/threads.py`.

mod common;

const RUNS: usize = 5;

#[test]
fn threads_match_native() {
    let flags = ["-O2", "-static", "-pthread"];
    let exe = common::cc("threads", "threads.c", "threads", &flags);
    let native = common::native(&exe, &[], &[]);
    assert!(native.status.success(), "native: {}", native.status);
    for run in 1..=RUNS {
        let calco = common::calco(&exe, &[], &[]);
        common::assert_same(&native, &calco, &format!("run {run}, "));
    }
}
