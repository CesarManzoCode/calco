//! Compiles `guests/smc.c` static (`-O2 -static`, contract D21) and checks that under calco
//! it behaves like the native run: same stdout and same exit status. The guest rewrites
//! code in a RWX page between calls, rewrites an instruction ahead of it in its own block,
//! and changes the page with `mprotect`, `munmap`, `madvise(MADV_DONTNEED)` and `mremap`
//! between calls (k2-04).

use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn smc_matches_native() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("smc");
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("smc");
    let status = Command::new("cc")
        .args(["-O2", "-static", "-o"])
        .arg(&exe)
        .arg(root.join("guests/smc.c"))
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
