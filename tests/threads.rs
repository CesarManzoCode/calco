//! Compiles `guests/threads.c` static with `-pthread` (`-O2 -static`, contract D21) and
//! checks that a few runs under calco give the same stdout and exit status as the native
//! run: 8 threads with a mutex, condition variables, `__thread`, an atomic counter,
//! `pthread_join`, and signals to one thread and to the process (k2-03). The 50 runs of
//! the criterion are `scripts/threads.py`.

use std::path::{Path, PathBuf};
use std::process::Command;

const RUNS: usize = 5;

#[test]
fn threads_match_native() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("threads");
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("threads");
    let status = Command::new("cc")
        .args(["-O2", "-static", "-pthread", "-o"])
        .arg(&exe)
        .arg(root.join("guests/threads.c"))
        .status()
        .expect("cannot run cc");
    assert!(status.success(), "cc failed: {status}");

    let native = Command::new(&exe).output().expect("cannot run the guest");
    assert!(native.status.success(), "native: {}", native.status);
    for run in 1..=RUNS {
        let calco = Command::new(env!("CARGO_BIN_EXE_calco"))
            .arg(&exe)
            .output()
            .expect("cannot run calco");
        assert_eq!(
            String::from_utf8_lossy(&calco.stdout),
            String::from_utf8_lossy(&native.stdout),
            "run {run}, calco stderr: {}",
            String::from_utf8_lossy(&calco.stderr)
        );
        assert_eq!(
            calco.status.code(),
            native.status.code(),
            "run {run}, calco: {}",
            calco.status
        );
    }
}
