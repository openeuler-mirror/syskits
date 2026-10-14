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
fn short_options_are_rejected_like_gnu() {
    for args in [
        ["-h"].as_slice(),
        ["-V"].as_slice(),
        ["-hV"].as_slice(),
        ["-Vh"].as_slice(),
        ["-?"].as_slice(),
        ["--", "-h"].as_slice(),
    ] {
        let gnu = run_hostid("/usr/bin/hostid", args);
        let syskits = run_hostid(env!("CARGO_BIN_EXE_hostid"), args);

        assert_eq!(syskits.status.code(), gnu.status.code(), "{args:?}");
        assert_eq!(syskits.stdout, gnu.stdout, "{args:?}");
        assert_eq!(syskits.stderr, gnu.stderr, "{args:?}");
    }
}
