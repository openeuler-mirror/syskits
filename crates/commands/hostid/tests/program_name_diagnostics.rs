#![cfg(target_os = "linux")]

use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};

fn run_hostid(binary: &str, program_name: &str, args: &[&str]) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0(program_name)
        .args(args)
        .stdin(Stdio::null())
        .env("LC_ALL", "C")
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG");
    command.output().expect("run hostid")
}

#[test]
fn diagnostic_program_names_match_gnu() {
    for program_name in ["custom", "/tmp/custom"] {
        for args in [
            ["value"].as_slice(),
            ["-h"].as_slice(),
            ["--invalid"].as_slice(),
        ] {
            let gnu = run_hostid("/usr/bin/hostid", program_name, args);
            let syskits = run_hostid(env!("CARGO_BIN_EXE_hostid"), program_name, args);

            assert_eq!(
                syskits.status.code(),
                gnu.status.code(),
                "{program_name}: {args:?}"
            );
            assert_eq!(syskits.stdout, gnu.stdout, "{program_name}: {args:?}");
            assert_eq!(syskits.stderr, gnu.stderr, "{program_name}: {args:?}");
        }
    }
}
