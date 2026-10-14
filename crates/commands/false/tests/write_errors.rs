#![cfg(target_os = "linux")]

use std::ffi::{OsStr, OsString};
use std::fs::OpenOptions;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};

fn run_with_full_stdout(binary: &str, program_name: &OsStr, argument: &str) -> Output {
    let stdout = OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .expect("open /dev/full");
    let mut command = Command::new(binary);
    command
        .arg0(program_name)
        .arg(argument)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .env("LC_ALL", "C");
    command.output().expect("run false with /dev/full")
}

fn run_with_closed_stdout(binary: &str, program_name: &str, argument: &str) -> Output {
    const SCRIPT: &str = r#"
exec 3>&1
exec 1>&-
binary=$1
program_name=$2
argument=$3
bash -c 'exec -a "$1" "$0" "$2"' "$binary" "$program_name" "$argument"
printf '%s' "$?" >&3
"#;

    Command::new("bash")
        .args([
            "-c",
            SCRIPT,
            "bash",
            binary,
            program_name,
            argument,
        ])
        .stdin(Stdio::null())
        .env("LC_ALL", "C")
        .output()
        .expect("run false with closed stdout")
}

#[test]
fn full_stdout_write_errors_match_gnu() {
    let program_names = [
        OsString::from("false"),
        OsString::from("/tmp/custom"),
        OsString::from_vec(b"fa\xfflse".to_vec()),
    ];

    for program_name in &program_names {
        for argument in ["--help", "--version"] {
            let gnu = run_with_full_stdout("/usr/bin/false", program_name, argument);
            let syskits =
                run_with_full_stdout(env!("CARGO_BIN_EXE_false"), program_name, argument);

            assert_eq!(syskits.status.code(), gnu.status.code());
            assert_eq!(syskits.stdout, gnu.stdout);
            assert_eq!(syskits.stderr, gnu.stderr);
        }
    }
}

#[test]
fn closed_stdout_write_errors_match_gnu() {
    for program_name in ["false", "custom", "/tmp/custom"] {
        for argument in ["--help", "--version"] {
            let gnu = run_with_closed_stdout("/usr/bin/false", program_name, argument);
            let syskits =
                run_with_closed_stdout(env!("CARGO_BIN_EXE_false"), program_name, argument);

            assert!(gnu.status.success());
            assert!(syskits.status.success());
            assert_eq!(syskits.stdout, gnu.stdout);
            assert_eq!(syskits.stderr, gnu.stderr);
        }
    }
}
