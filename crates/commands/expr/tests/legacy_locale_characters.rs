#![cfg(target_os = "linux")]

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::process::CommandExt;
use std::process::{Command, Output};

fn run_expr(binary: &str, locale: &str, arguments: &[OsString]) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0("expr")
        .args(arguments)
        .env("LC_ALL", locale)
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG");
    command.output().expect("run expr")
}

#[test]
fn legacy_chinese_locale_character_operations_match_gnu_coreutils() {
    let string = OsString::from_vec(vec![b'a', 0xd6, 0xd0, b'b']);
    for locale in ["zh_CN.gbk", "zh_CN.gb18030", "zh_CN.gb2312"] {
        for arguments in [
            vec![OsString::from("length"), string.clone()],
            vec![OsString::from("index"), string.clone(), OsString::from("b")],
            vec![
                OsString::from("substr"),
                string.clone(),
                OsString::from("2"),
                OsString::from("1"),
            ],
        ] {
            let gnu = run_expr("/usr/bin/expr", locale, &arguments);
            let syskits = run_expr(env!("CARGO_BIN_EXE_expr"), locale, &arguments);

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
