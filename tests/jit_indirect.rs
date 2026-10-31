//! Assembles `guests/asm/indirect.S` without libc and runs it under calco (k3-03): a jump
//! table, recursion with `ret` and an indirect call, all through the inline lookup of
//! indirect transfers, with flags live across `ret` and `jmp`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn run(cmd: &mut Command) {
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("cannot run {cmd:?}: {e}"));
    assert!(status.success(), "{cmd:?} failed: {status}");
}

#[test]
fn indirect_guest_runs_translated() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("jit_indirect");
    std::fs::create_dir_all(&dir).unwrap();
    let obj = dir.join("indirect.o");
    let exe = dir.join("indirect");
    run(Command::new("as")
        .arg("-o")
        .arg(&obj)
        .arg(root.join("guests/asm/indirect.S")));
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
        "table 2454267021 fib 6765 call 55\n",
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(0), "status: {}", out.status);
}
