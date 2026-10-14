#![cfg(target_os = "linux")]

use std::fs::OpenOptions;
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};

fn run_hostid(binary: &str, locale: &str, args: &[&str]) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0("hostid")
        .args(args)
        .stdin(Stdio::null())
        .env("LC_ALL", locale)
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG");
    command.output().expect("run hostid")
}

fn run_with_full_stdout(binary: &str, locale: &str) -> Output {
    let stdout = OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .expect("open /dev/full");
    let mut command = Command::new(binary);
    command
        .arg0("hostid")
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .env("LC_ALL", locale)
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG");
    command.output().expect("run hostid with /dev/full")
}

#[test]
fn chinese_diagnostics_match_gnu() {
    for locale in ["zh_CN.utf8", "zh_CN.gbk", "zh_TW.utf8", "zh_TW.big5"] {
        for args in [
            ["value"].as_slice(),
            ["-h"].as_slice(),
            ["--invalid"].as_slice(),
            ["--help=value"].as_slice(),
            ["--=value"].as_slice(),
        ] {
            let gnu = run_hostid("/usr/bin/hostid", locale, args);
            let syskits = run_hostid(env!("CARGO_BIN_EXE_hostid"), locale, args);

            assert_eq!(
                syskits.status.code(),
                gnu.status.code(),
                "{locale}: {args:?}"
            );
            assert_eq!(syskits.stdout, gnu.stdout, "{locale}: {args:?}");
            assert_eq!(syskits.stderr, gnu.stderr, "{locale}: {args:?}");
        }
    }
}

#[test]
fn chinese_write_error_diagnostics_match_gnu() {
    for locale in ["zh_CN.utf8", "zh_CN.gbk", "zh_TW.utf8", "zh_TW.big5"] {
        let gnu = run_with_full_stdout("/usr/bin/hostid", locale);
        let syskits = run_with_full_stdout(env!("CARGO_BIN_EXE_hostid"), locale);

        assert_eq!(syskits.status.code(), gnu.status.code(), "{locale}");
        assert_eq!(syskits.stdout, gnu.stdout, "{locale}");
        assert_eq!(syskits.stderr, gnu.stderr, "{locale}");
    }
}
