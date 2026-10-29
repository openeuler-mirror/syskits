#![cfg(target_os = "linux")]

use std::fs::OpenOptions;
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};

fn run_false(binary: &str, locale: &str, language: Option<&str>, argument: &str) -> Output {
    let stdout = OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .expect("open /dev/full");
    let mut command = Command::new(binary);
    command
        .arg0("false")
        .arg(argument)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .env("LC_ALL", locale)
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG");
    match language {
        Some(language) => command.env("LANGUAGE", language),
        None => command.env_remove("LANGUAGE"),
    };
    command.output().expect("run false with localized output")
}

fn normalize_write_error(stderr: &[u8]) -> Vec<u8> {
    let prefix = b"false: ";
    let Some(message) = stderr.strip_prefix(prefix) else {
        return stderr.to_vec();
    };
    let Some(separator) = message.windows(2).position(|window| window == b": ") else {
        return stderr.to_vec();
    };

    let mut normalized = prefix.to_vec();
    normalized.extend_from_slice(b"<write-error>");
    normalized.extend_from_slice(&message[separator..]);
    normalized
}

fn assert_matches_gnu(locale: &str, language: Option<&str>, argument: &str) {
    let gnu = run_false("/usr/bin/false", locale, language, argument);
    let syskits = run_false(env!("CARGO_BIN_EXE_false"), locale, language, argument);

    assert_eq!(syskits.status.code(), gnu.status.code());
    assert_eq!(syskits.stdout, gnu.stdout);
    assert_eq!(
        normalize_write_error(&syskits.stderr),
        normalize_write_error(&gnu.stderr)
    );
}

#[test]
fn locale_write_error_labels_match_gnu() {
    for locale in [
        "C",
        "C.utf8",
        "en_US.utf8",
        "zh_CN.utf8",
        "zh_CN.gbk",
        "zh_CN.gb18030",
        "zh_CN.gb2312",
        "zh_TW.utf8",
        "zh_TW.big5",
        "zh_HK.utf8",
        "zh_SG.utf8",
    ] {
        for argument in ["--help", "--version"] {
            assert_matches_gnu(locale, None, argument);
        }
    }
}

#[test]
fn language_priority_and_invalid_locale_match_gnu() {
    for (locale, language) in [
        ("C", "zh_CN"),
        ("C.utf8", "zh_CN"),
        ("en_US.utf8", "zh_CN"),
        ("en_US.utf8", "xx:zh_CN"),
        ("zh_CN.gbk", "zh_TW"),
        ("zh_TW.big5", "zh_CN"),
        ("invalid", "zh_CN"),
    ] {
        for argument in ["--help", "--version"] {
            assert_matches_gnu(locale, Some(language), argument);
        }
    }
}
