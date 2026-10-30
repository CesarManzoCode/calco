//! calco: dynamic binary translator from x86-64 to x86-64 for Linux user-mode programs.

// Modules expose what later sprints consume (signals, threads, SMC); much is still unused.
#![allow(dead_code)]

mod cpu;
mod cpuid;
mod decode;
mod emit;
mod jit;
mod loader;
mod mem;
mod signal;
mod sys;
mod syscall;

use std::env;
use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::process::exit;

/// Exit status for errors of calco itself, not of the guest.
const EXIT_CALCO: i32 = 125;
const USAGE: &str = "usage: calco [--argv0 NAME] [--] PROGRAM [ARGS...]";

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
    let (image, argv) = loader::resolve(&program, argv).unwrap_or_else(|e| {
        fail(&format!(
            "{}: {}",
            program.display(),
            std::io::Error::from_raw_os_error(e)
        ))
    });
    let mut mem = mem::Mem::new();
    let execfn = program.as_os_str().as_bytes();
    let loaded = loader::load(&image, execfn, &mut mem, &argv, &envp, cpuid::hwcap())
        .unwrap_or_else(|e| fail(&e));
    let mut cpu = cpu::Cpu::new(loaded.entry, loaded.rsp);
    let mut jit = jit::Jit::new(&mem).unwrap_or_else(|e| {
        fail(&format!(
            "cannot open the code cache: {}",
            std::io::Error::from_raw_os_error(e)
        ))
    });
    // What the guest reads from /proc/self/exe; calco's own path is the kernel's answer,
    // and the one guest `execve` runs again.
    let exe = std::fs::canonicalize(&image).unwrap_or(image);
    let calco = std::fs::read_link("/proc/self/exe")
        .unwrap_or_else(|e| fail(&format!("/proc/self/exe: {e}")));
    let log = env::var_os("CALCO_LOG")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let mut state = syscall::State::new(
        exe.into_os_string().into_vec(),
        calco.into_os_string().into_vec(),
        log,
    );
    jit::run(&mut cpu, &mut mem, &mut jit, &mut state)
}
