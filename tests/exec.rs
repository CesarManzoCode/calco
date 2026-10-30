//! Compiles `guests/exec.c` static (`-O2 -static`, contract D21) and checks that under
//! calco its output is `guests/exec.expected`: the child that `execve`s the program again
//! also runs translated, so it sees the filtered CPUID too (k2-01).

use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn exec_child_runs_translated() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("exec");
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("exec");
    let status = Command::new("cc")
        .args(["-O2", "-static", "-o"])
        .arg(&exe)
        .arg(root.join("guests/exec.c"))
        .status()
        .expect("cannot run cc");
    assert!(status.success(), "cc failed: {status}");

    let out = Command::new(env!("CARGO_BIN_EXE_calco"))
        .arg(&exe)
        .output()
        .expect("cannot run calco");
    let expected = std::fs::read_to_string(root.join("guests/exec.expected")).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        expected,
        "calco stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(0), "calco: {}", out.status);
}
