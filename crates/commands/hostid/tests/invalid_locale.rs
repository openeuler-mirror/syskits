#![cfg(target_os = "linux")]

use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};

fn run_hostid(binary: &str, locale: &str, language: Option<&str>, args: &[&str]) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0("hostid")
        .args(args)
        .stdin(Stdio::null())
        .env("LC_ALL", locale)
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG");
    match language {
        Some(language) => {
            command.env("LANGUAGE", language);
        }
        None => {
            command.env_remove("LANGUAGE");
        }
    }
    command.output().expect("run hostid")
}

#[test]
fn invalid_locale_falls_back_to_c_diagnostics_like_gnu() {
    for locale in ["zh_BAD", "zh_CN.invalid", "invalid", "C.invalid"] {
        for language in [None, Some("zh_CN")] {
            for args in [["value"].as_slice(), ["--invalid"].as_slice()] {
                let gnu = run_hostid("/usr/bin/hostid", locale, language, args);
                let syskits = run_hostid(env!("CARGO_BIN_EXE_hostid"), locale, language, args);

                assert_eq!(
                    syskits.status.code(),
                    gnu.status.code(),
                    "{locale}: LANGUAGE={language:?}: {args:?}"
                );
                assert_eq!(
                    syskits.stdout, gnu.stdout,
                    "{locale}: LANGUAGE={language:?}: {args:?}"
                );
                assert_eq!(
                    syskits.stderr, gnu.stderr,
                    "{locale}: LANGUAGE={language:?}: {args:?}"
                );
            }
        }
    }
}
