#![cfg(target_os = "linux")]

use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};

fn run_hostid(binary: &str, args: &[&str]) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0("hostid")
        .args(args)
        .stdin(Stdio::null())
        .env("LC_ALL", "C")
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG");
    command.output().expect("run hostid")
}

#[test]
fn long_option_diagnostics_match_gnu() {
    for args in [
        ["--help=value"].as_slice(),
        ["--he=value"].as_slice(),
        ["--h=value"].as_slice(),
        ["--version=value"].as_slice(),
        ["--ver=value"].as_slice(),
        ["--v=value"].as_slice(),
        ["--invalid"].as_slice(),
        ["--invalid=value"].as_slice(),
        ["--=value"].as_slice(),
        ["---"].as_slice(),
        ["--invalid", "-h"].as_slice(),
        ["--invalid", "--help"].as_slice(),
        ["--invalid", "--version"].as_slice(),
        ["value", "--invalid"].as_slice(),
    ] {
        let gnu = run_hostid("/usr/bin/hostid", args);
        let syskits = run_hostid(env!("CARGO_BIN_EXE_hostid"), args);

        assert_eq!(syskits.status.code(), gnu.status.code(), "{args:?}");
        assert_eq!(syskits.stdout, gnu.stdout, "{args:?}");
        assert_eq!(syskits.stderr, gnu.stderr, "{args:?}");
    }
}
