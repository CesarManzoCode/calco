//! Assembles `guests/asm/basic.S` without libc and runs it under calco (k1-03): the first
//! complete path through the translator, the code cache, the dispatcher and `syscall`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn run(cmd: &mut Command) {
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("cannot run {cmd:?}: {e}"));
    assert!(status.success(), "{cmd:?} failed: {status}");
}

#[test]
fn basic_guest_runs_translated() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("jit_basic");
    std::fs::create_dir_all(&dir).unwrap();
    let obj = dir.join("basic.o");
    let exe = dir.join("basic");
    run(Command::new("as")
        .arg("-o")
        .arg(&obj)
        .arg(root.join("guests/asm/basic.S")));
    run(Command::new("ld")
        .args(["-static", "-nostdlib", "-o"])
        .arg(&exe)
        .arg(&obj));

    let out = Command::new(env!("CARGO_BIN_EXE_calco"))
        .arg(&exe)
        .output()
        .expect("cannot run calco");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "calco jit basic ok\n",
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(42), "status: {}", out.status);
}
