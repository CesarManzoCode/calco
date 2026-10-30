//! Compiles `guests/signals.c` static (`-O2 -static`, contract D21) and checks that under
//! calco it behaves like the native run: same stdout and same exit status. The guest
//! takes SIGSEGV, SIGILL and SIGFPE from the CPU and SIGALRM in the middle of a loop, and
//! prints what its handlers see (k2-02).

use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn signals_match_native() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("signals");
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("signals");
    let status = Command::new("cc")
        .args(["-O2", "-static", "-o"])
        .arg(&exe)
        .arg(root.join("guests/signals.c"))
        .status()
        .expect("cannot run cc");
    assert!(status.success(), "cc failed: {status}");

    let native = Command::new(&exe).output().expect("cannot run the guest");
    assert!(native.status.success(), "native: {}", native.status);
    let calco = Command::new(env!("CARGO_BIN_EXE_calco"))
        .arg(&exe)
        .output()
        .expect("cannot run calco");
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
