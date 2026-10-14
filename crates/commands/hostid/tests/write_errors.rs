#![cfg(target_os = "linux")]

use std::fs::OpenOptions;
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};

fn run_with_full_stdout(binary: &str, program_name: &str, args: &[&str]) -> Output {
    let stdout = OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .expect("open /dev/full");
    let mut command = Command::new(binary);
    command
        .arg0(program_name)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .env("LC_ALL", "C")
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG");
    command.output().expect("run hostid with /dev/full")
}

fn run_with_closed_stdout(binary: &str, program_name: &str, args: &[&str]) -> Output {
    const SCRIPT: &str = r#"
exec 3>&1
exec 1>&-
binary=$1
program_name=$2
shift 2
bash -c 'program_name=$1; shift; exec -a "$program_name" "$0" "$@"' "$binary" "$program_name" "$@"
printf '%s' "$?" >&3
"#;

    Command::new("bash")
        .args(["-c", SCRIPT, "bash", binary, program_name])
        .args(args)
        .stdin(Stdio::null())
        .env("LC_ALL", "C")
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG")
        .output()
        .expect("run hostid with closed stdout")
}

#[test]
fn full_stdout_errors_match_gnu() {
    for program_name in ["hostid", "custom", "/tmp/custom"] {
        for args in [
            [].as_slice(),
            ["--help"].as_slice(),
            ["--version"].as_slice(),
        ] {
            let gnu = run_with_full_stdout("/usr/bin/hostid", program_name, args);
            let syskits = run_with_full_stdout(env!("CARGO_BIN_EXE_hostid"), program_name, args);

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

#[test]
fn closed_stdout_errors_match_gnu() {
    for program_name in ["hostid", "custom", "/tmp/custom"] {
        for args in [
            [].as_slice(),
            ["--help"].as_slice(),
            ["--version"].as_slice(),
        ] {
            let gnu = run_with_closed_stdout("/usr/bin/hostid", program_name, args);
            let syskits = run_with_closed_stdout(env!("CARGO_BIN_EXE_hostid"), program_name, args);

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
