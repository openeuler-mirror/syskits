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
        .arg("--help")
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .env("LC_ALL", locale)
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG");
    command.output().expect("run hostid with /dev/full")
}

fn contains_untranslated_diagnostic(stderr: &[u8]) -> bool {
    [
        b"extra operand".as_slice(),
        b"invalid option".as_slice(),
        b"unrecognized option".as_slice(),
        b"doesn't allow an argument".as_slice(),
        b"is ambiguous".as_slice(),
        b"Try ".as_slice(),
        b"write error: ".as_slice(),
    ]
    .iter()
    .any(|marker| stderr.windows(marker.len()).any(|window| window == *marker))
}

fn assert_locale_outputs(gnu: &Output, syskits: &Output, context: &str) {
    assert_eq!(syskits.status.code(), gnu.status.code(), "{context}");
    assert_eq!(syskits.stdout, gnu.stdout, "{context}");

    if syskits.stderr != gnu.stderr {
        let translation_only = !syskits.stderr.is_empty()
            && !gnu.stderr.is_empty()
            && (contains_untranslated_diagnostic(&syskits.stderr)
                != contains_untranslated_diagnostic(&gnu.stderr));
        assert!(
            translation_only,
            "{context}: syskits stderr={:?}, GNU stderr={:?}",
            syskits.stderr, gnu.stderr
        );
    }
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

            assert_locale_outputs(&gnu, &syskits, &format!("{locale}: {args:?}"));
        }
    }
}

#[test]
fn chinese_write_error_diagnostics_match_gnu() {
    for locale in ["zh_CN.utf8", "zh_CN.gbk", "zh_TW.utf8", "zh_TW.big5"] {
        let gnu = run_with_full_stdout("/usr/bin/hostid", locale);
        let syskits = run_with_full_stdout(env!("CARGO_BIN_EXE_hostid"), locale);

        assert_locale_outputs(&gnu, &syskits, locale);
    }
}
