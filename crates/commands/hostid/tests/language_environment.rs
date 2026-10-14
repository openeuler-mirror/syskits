#![cfg(target_os = "linux")]

use std::fs::OpenOptions;
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};

fn hostid_command(binary: &str, locale: &str, language: &str) -> Command {
    let mut command = Command::new(binary);
    command
        .arg0("hostid")
        .stdin(Stdio::null())
        .env("LC_ALL", locale)
        .env("LANGUAGE", language)
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG");
    command
}

fn run_hostid(binary: &str, locale: &str, language: &str, args: &[&str]) -> Output {
    hostid_command(binary, locale, language)
        .args(args)
        .output()
        .expect("run hostid")
}

fn run_with_full_stdout(binary: &str, locale: &str, language: &str) -> Output {
    let stdout = OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .expect("open /dev/full");
    hostid_command(binary, locale, language)
        .stdout(Stdio::from(stdout))
        .output()
        .expect("run hostid with /dev/full")
}

#[test]
fn language_translation_priority_matches_gnu() {
    for (locale, language) in [
        ("C", "zh_CN"),
        ("C.utf8", "zh_CN"),
        ("en_US.utf8", "zh_TW"),
        ("en_US.utf8", "zz:zh_CN"),
        ("zh_CN.utf8", "zh_TW:zh_CN"),
    ] {
        for args in [["value"].as_slice(), ["--invalid"].as_slice()] {
            let gnu = run_hostid("/usr/bin/hostid", locale, language, args);
            let syskits = run_hostid(env!("CARGO_BIN_EXE_hostid"), locale, language, args);

            assert_eq!(
                syskits.status.code(),
                gnu.status.code(),
                "{locale}: LANGUAGE={language}: {args:?}"
            );
            assert_eq!(
                syskits.stdout, gnu.stdout,
                "{locale}: LANGUAGE={language}: {args:?}"
            );
            assert_eq!(
                syskits.stderr, gnu.stderr,
                "{locale}: LANGUAGE={language}: {args:?}"
            );
        }

        let gnu = run_with_full_stdout("/usr/bin/hostid", locale, language);
        let syskits = run_with_full_stdout(env!("CARGO_BIN_EXE_hostid"), locale, language);
        assert_eq!(
            syskits.status.code(),
            gnu.status.code(),
            "{locale}: {language}"
        );
        assert_eq!(syskits.stdout, gnu.stdout, "{locale}: {language}");
        assert_eq!(syskits.stderr, gnu.stderr, "{locale}: {language}");
    }
}
