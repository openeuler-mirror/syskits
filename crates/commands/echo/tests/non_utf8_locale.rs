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

fn run_echo_with_split_locale(
    binary: &str,
    messages_locale: &str,
    ctype_locale: &str,
    argument: &str,
) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0("echo")
        .arg(argument)
        .env_remove("LC_ALL")
        .env("LC_MESSAGES", messages_locale)
        .env("LC_CTYPE", ctype_locale)
        .env_remove("LANG")
        .env_remove("LANGUAGE");
    command.output().expect("run echo")
}

fn assert_syskits_output(output: &Output, argument: &str, context: &str) {
    assert_eq!(output.status.code(), Some(0), "{context} {argument}");
    assert!(output.stderr.is_empty(), "{context} {argument}");
    assert!(
        output
            .stdout
            .windows(b"syskits".len())
            .any(|window| window == b"syskits"),
        "{context} {argument}"
    );
    assert!(
        !output.stdout.windows(3).any(|window| window == b"GNU"),
        "{context} {argument}"
    );
    if argument == "--version" {
        assert!(
            output
                .stdout
                .windows(b"Mulan PSL v2".len())
                .any(|window| window == b"Mulan PSL v2"),
            "{context} {argument}"
        );
    }
}

#[test]
fn zh_cn_legacy_encodings_preserve_syskits_information() {
    for locale in ["zh_CN.gbk", "zh_CN.gb18030", "zh_CN.gb2312"] {
        for argument in ["--help", "--version"] {
            let syskits = run_echo(env!("CARGO_BIN_EXE_echo"), locale, None, argument);
            assert_syskits_output(&syskits, argument, locale);
        }
    }
}

#[test]
fn language_is_ignored_in_the_c_locale() {
    for argument in ["--help", "--version"] {
        let baseline = run_echo(env!("CARGO_BIN_EXE_echo"), "C", None, argument);
        let syskits = run_echo(env!("CARGO_BIN_EXE_echo"), "C", Some("zh_CN"), argument);

        assert_eq!(syskits.status.code(), baseline.status.code(), "{argument}");
        assert_eq!(syskits.stderr, baseline.stderr, "{argument}");
        assert_eq!(syskits.stdout, baseline.stdout, "{argument}");
        assert_syskits_output(&syskits, argument, "C with LANGUAGE=zh_CN");
    }
}

#[test]
fn invalid_locale_falls_back_to_c_locale() {
    for argument in ["--help", "--version"] {
        let baseline = run_echo(env!("CARGO_BIN_EXE_echo"), "C", None, argument);
        let syskits = run_echo(
            env!("CARGO_BIN_EXE_echo"),
            "zh_CN.invalid",
            Some("zh_CN"),
            argument,
        );

        assert_eq!(syskits.status.code(), baseline.status.code(), "{argument}");
        assert_eq!(syskits.stderr, baseline.stderr, "{argument}");
        assert_eq!(syskits.stdout, baseline.stdout, "{argument}");
        assert_syskits_output(&syskits, argument, "invalid locale");
    }
}

#[test]
fn c_ctype_transliterates_localized_output() {
    for ctype_locale in ["C", "POSIX"] {
        for argument in ["--help", "--version"] {
            let syskits = run_echo_with_split_locale(
                env!("CARGO_BIN_EXE_echo"),
                "zh_CN.UTF-8",
                ctype_locale,
                argument,
            );

            assert_syskits_output(&syskits, argument, ctype_locale);
            assert!(syskits.stdout.is_ascii(), "{ctype_locale} {argument}");
        }
    }
}
