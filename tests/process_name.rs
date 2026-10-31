//! The guest process goes by the last component of its argv[0], and a guest that runs
//! calco itself (the busybox suite wrapper, contract 6) gets it without a calco under
//! calco. The guests are the system `cat` and `sh`, dynamic with glibc.

use std::process::Command;

fn comm(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_calco"))
        .args(args)
        .output()
        .expect("cannot run calco");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{args:?}: {} {stderr}", out.status);
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn the_guest_is_named_after_its_argv0() {
    let cat = ["/usr/bin/cat", "/proc/self/comm"];
    assert_eq!(comm(&["--argv0", "/x/pidof", cat[0], cat[1]]), "pidof\n");
    let calco = env!("CARGO_BIN_EXE_calco");
    let nested = format!("exec {calco} --argv0 inner {} {}", cat[0], cat[1]);
    assert_eq!(comm(&["/bin/sh", "-c", &nested]), "inner\n");
}
