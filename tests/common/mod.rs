//! What the integration tests share: building a guest of `guests/` into the test's own
//! directory and running it natively and under calco.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Runs `cmd` and fails the test if it does not succeed.
fn must(cmd: &mut Command) {
    let status = cmd.status();
    let status = status.unwrap_or_else(|e| panic!("cannot run {cmd:?}: {e}"));
    assert!(status.success(), "{cmd:?} failed: {status}");
}

fn dir(test: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(test);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// Compiles `guests/<src>` with `cc <flags>` into the directory of `test` as `exe`.
pub fn cc(test: &str, src: &str, exe: &str, flags: &[&str]) -> PathBuf {
    let exe = dir(test).join(exe);
    let src = root().join("guests").join(src);
    must(Command::new("cc").args(flags).arg("-o").arg(&exe).arg(src));
    exe
}

/// Assembles `guests/asm/<name>.S` and links it without libc into the directory of `test`.
pub fn asm(test: &str, name: &str) -> PathBuf {
    let (obj, exe) = (dir(test).join(format!("{name}.o")), dir(test).join(name));
    let src = root().join(format!("guests/asm/{name}.S"));
    must(Command::new("as").arg("-o").arg(&obj).arg(src));
    let ld = ["-static", "-nostdlib", "-o"];
    must(Command::new("ld").args(ld).arg(&exe).arg(&obj));
    exe
}

pub fn output(cmd: &mut Command) -> Output {
    cmd.output()
        .unwrap_or_else(|e| panic!("cannot run {cmd:?}: {e}"))
}

/// `exe` under calco, with `args` and `env`.
pub fn calco(exe: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_calco"));
    output(cmd.arg(exe).args(args).envs(env.iter().copied()))
}

/// `exe` natively, with `args` and `env`.
pub fn native(exe: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    output(Command::new(exe).args(args).envs(env.iter().copied()))
}

/// Fails unless the run under calco printed `stdout` and ended with `code`.
pub fn assert_run(calco: &Output, stdout: &str, code: Option<i32>, what: &str) {
    let stderr = String::from_utf8_lossy(&calco.stderr);
    let got = String::from_utf8_lossy(&calco.stdout);
    assert_eq!(got, stdout, "{what}calco stderr: {stderr}");
    assert_eq!(calco.status.code(), code, "{what}calco: {}", calco.status);
}

/// Fails unless the run under calco ended like `native` with the same stdout.
pub fn assert_same(native: &Output, calco: &Output, what: &str) {
    let stdout = String::from_utf8_lossy(&native.stdout);
    assert_run(calco, &stdout, native.status.code(), what);
}
