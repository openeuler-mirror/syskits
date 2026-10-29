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
        .arg("--help")
        .stdout(Stdio::from(stdout))
        .output()
        .expect("run hostid with /dev/full")
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
        // GNU's external message catalog is optional in the test environment,
        // while syskits ships its translations.  Compare the stable language
        // marker instead of assuming that non-ASCII bytes always mean text:
        // legacy locale quote marks may also be non-ASCII.
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
fn language_translation_priority_matches_gnu() {
    for (locale, language) in [
        ("C", "zh_CN"),
        ("C.utf8", "zh_CN"),
        ("en_US.utf8", "zh_TW"),
        ("en_US.utf8", "zz:zh_CN"),
        ("zh_CN.utf8", "zh_TW:zh_CN"),
        ("zh_CN.gbk", "zh_TW"),
        ("zh_CN.gb18030", "zz:zh_TW"),
        ("zh_TW.big5", "zh_CN"),
    ] {
        for args in [["value"].as_slice(), ["--invalid"].as_slice()] {
            let gnu = run_hostid("/usr/bin/hostid", locale, language, args);
            let syskits = run_hostid(env!("CARGO_BIN_EXE_hostid"), locale, language, args);

            assert_locale_outputs(
                &gnu,
                &syskits,
                &format!("{locale}: LANGUAGE={language}: {args:?}"),
            );
        }

        let gnu = run_with_full_stdout("/usr/bin/hostid", locale, language);
        let syskits = run_with_full_stdout(env!("CARGO_BIN_EXE_hostid"), locale, language);
        assert_locale_outputs(&gnu, &syskits, &format!("{locale}: {language}"));
    }
}
