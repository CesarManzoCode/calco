//! Compiles `guests/exec.c` static (`-O2 -static`, contract D21) and checks that under
//! calco its output is `guests/exec.expected`: the child that `execve`s the program again
//! also runs translated, so it sees the filtered CPUID too (k2-01).

mod common;

#[test]
fn exec_child_runs_translated() {
    let exe = common::cc("exec", "exec.c", "exec", &["-O2", "-static"]);
    let expected = std::fs::read_to_string(common::root().join("guests/exec.expected")).unwrap();
    common::assert_run(&common::calco(&exe, &[], &[]), &expected, Some(0), "");
}
