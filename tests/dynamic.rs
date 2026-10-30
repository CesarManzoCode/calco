//! Compiles `guests/hello.c` dynamic with gcc's default PIE (contract D21) and checks that
//! under calco, entering through the system `ld.so`, it prints the same stdout and exits
//! with the same code as natively (k2-05).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(cmd: &mut Command) -> Output {
    cmd.env("CALCO_HELLO", "dynamic")
        .output()
        .unwrap_or_else(|e| panic!("cannot run {cmd:?}: {e}"))
}

#[test]
fn dynamic_hello_matches_native() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("dynamic");
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("hello-dynamic");
    let status = Command::new("cc")
        .args(["-O2", "-o"])
        .arg(&exe)
        .arg(root.join("guests/hello.c"))
        .status()
        .expect("cannot run cc");
    assert!(status.success(), "cc failed: {status}");

    let args = ["one", "two words"];
    let native = run(Command::new(&exe).args(args));
    let calco = run(Command::new(env!("CARGO_BIN_EXE_calco"))
        .arg(&exe)
        .args(args));
    assert_eq!(native.status.code(), Some(3), "native: {}", native.status);
    assert_eq!(
        String::from_utf8_lossy(&calco.stdout),
        String::from_utf8_lossy(&native.stdout),
        "calco stderr: {}",
        String::from_utf8_lossy(&calco.stderr)
    );
    assert_eq!(
        calco.status.code(),
        native.status.code(),
        "calco: {}",
        calco.status
    );
}
