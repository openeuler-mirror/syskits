#![cfg(target_os = "linux")]

use std::fs::OpenOptions;
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};

fn run_echo(binary: &str, locale: &str, arguments: &[&str]) -> Output {
    let full = OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .expect("open /dev/full");
    let mut command = Command::new(binary);
    command
        .arg0("echo")
        .args(arguments)
        .env("LC_ALL", locale)
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG")
        .env_remove("LANGUAGE")
        .stdout(Stdio::from(full));
    command.output().expect("run echo with /dev/full stdout")
}

#[test]
fn localized_stdout_write_errors_match_gnu_coreutils() {
    for locale in ["zh_CN.UTF-8", "zh_CN.gbk", "zh_CN.gb18030", "zh_CN.gb2312"] {
        for arguments in [
            ["value"].as_slice(),
            ["--help"].as_slice(),
            ["--version"].as_slice(),
        ] {
            let gnu = run_echo("/usr/bin/echo", locale, arguments);
            let syskits = run_echo(env!("CARGO_BIN_EXE_echo"), locale, arguments);

            assert_eq!(
                syskits.status.code(),
                gnu.status.code(),
                "{locale} {arguments:?}"
            );
            assert_eq!(syskits.stdout, gnu.stdout, "{locale} {arguments:?}");
            assert_eq!(syskits.stderr, gnu.stderr, "{locale} {arguments:?}");
        }
    }
}
