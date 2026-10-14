#![cfg(target_os = "linux")]

use std::os::unix::process::CommandExt;
use std::process::{Command, Output};

fn run_echo(binary: &str, locale: &str, language: Option<&str>, argument: &str) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0("echo")
        .arg(argument)
        .env("LC_ALL", locale)
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG");
    match language {
        Some(language) => command.env("LANGUAGE", language),
        None => command.env_remove("LANGUAGE"),
    };
    command.output().expect("run echo")
}

#[test]
fn zh_cn_legacy_encodings_match_gnu_coreutils() {
    for locale in ["zh_CN.gbk", "zh_CN.gb18030", "zh_CN.gb2312"] {
        for argument in ["--help", "--version"] {
            let gnu = run_echo("/usr/bin/echo", locale, None, argument);
            let syskits = run_echo(env!("CARGO_BIN_EXE_echo"), locale, None, argument);

            assert_eq!(
                syskits.status.code(),
                gnu.status.code(),
                "{locale} {argument}"
            );
            assert_eq!(syskits.stderr, gnu.stderr, "{locale} {argument}");
            assert_eq!(syskits.stdout, gnu.stdout, "{locale} {argument}");
        }
    }
}

#[test]
fn language_is_ignored_in_the_c_locale() {
    for argument in ["--help", "--version"] {
        let gnu = run_echo("/usr/bin/echo", "C", Some("zh_CN"), argument);
        let syskits = run_echo(env!("CARGO_BIN_EXE_echo"), "C", Some("zh_CN"), argument);

        assert_eq!(syskits.status.code(), gnu.status.code(), "{argument}");
        assert_eq!(syskits.stderr, gnu.stderr, "{argument}");
        assert_eq!(syskits.stdout, gnu.stdout, "{argument}");
    }
}

#[test]
fn invalid_locale_falls_back_to_c_locale() {
    for argument in ["--help", "--version"] {
        let gnu = run_echo("/usr/bin/echo", "zh_CN.invalid", Some("zh_CN"), argument);
        let syskits = run_echo(
            env!("CARGO_BIN_EXE_echo"),
            "zh_CN.invalid",
            Some("zh_CN"),
            argument,
        );

        assert_eq!(syskits.status.code(), gnu.status.code(), "{argument}");
        assert_eq!(syskits.stderr, gnu.stderr, "{argument}");
        assert_eq!(syskits.stdout, gnu.stdout, "{argument}");
    }
}
