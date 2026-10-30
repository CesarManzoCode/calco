//! calco: dynamic binary translator from x86-64 to x86-64 for Linux user-mode programs.

// Modules expose what the JIT (k1-03 onward) consumes; until then much of it is unused.
#![allow(dead_code)]

mod cpu;
mod loader;
mod mem;
mod sys;

use std::env;
use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::process::exit;

/// Exit status for errors of calco itself, not of the guest.
const EXIT_CALCO: i32 = 125;
const USAGE: &str = "usage: calco [--argv0 NAME] [--] PROGRAM [ARGS...]";
/// `AT_HWCAP` until k1-04 passes CPUID.1:EDX filtered by `cpuid.rs`.
const HWCAP_PENDING: u64 = 0;

fn fail(msg: &str) -> ! {
    eprintln!("calco: {msg}");
    exit(EXIT_CALCO);
}

#[derive(Debug, PartialEq)]
struct Args {
    argv0: Option<OsString>,
    program: OsString,
    args: Vec<OsString>,
}

fn parse_args(raw: Vec<OsString>) -> Result<Args, String> {
    let mut it = raw.into_iter();
    let mut argv0 = None;
    let program = loop {
        let Some(arg) = it.next() else {
            return Err(USAGE.into());
        };
        match arg.as_bytes() {
            b"--argv0" => match it.next() {
                Some(name) => argv0 = Some(name),
                None => return Err("--argv0 needs a NAME".into()),
            },
            b"--" => break it.next().ok_or_else(|| USAGE.to_string())?,
            [b'-', ..] => return Err(format!("unknown option {}\n{USAGE}", arg.display())),
            _ => break arg,
        }
    };
    Ok(Args {
        argv0,
        program,
        args: it.collect(),
    })
}

fn main() {
    let args = parse_args(env::args_os().skip(1).collect()).unwrap_or_else(|e| fail(&e));
    let mut argv = vec![
        args.argv0
            .unwrap_or_else(|| args.program.clone())
            .into_vec(),
    ];
    argv.extend(args.args.into_iter().map(OsStringExt::into_vec));
    let envp: Vec<Vec<u8>> = env::vars_os()
        .map(|(key, value)| {
            let mut entry = key.into_vec();
            entry.push(b'=');
            entry.extend_from_slice(value.as_bytes());
            entry
        })
        .collect();

    let program = PathBuf::from(args.program);
    let mut mem = mem::Mem::new();
    let loaded =
        loader::load(&program, &mut mem, &argv, &envp, HWCAP_PENDING).unwrap_or_else(|e| fail(&e));
    let _cpu = cpu::Cpu::new(loaded.entry, loaded.rsp);
    fail("jit not implemented");
}
