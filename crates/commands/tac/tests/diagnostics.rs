#![cfg(unix)]

use std::fs::{self, File};
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};

fn run_tac(locale: &str, configure: impl FnOnce(&mut Command)) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tac"));
    command.arg0("tac").env("LC_ALL", locale);
    configure(&mut command);
    command.output().expect("run tac")
}

#[test]
fn localized_unknown_option_diagnostic_matches_gnu() {
    let output = run_tac("zh_CN.UTF-8", |command| {
        command.arg("--bad_flag");
    });

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "tac：未识别的选项 ‘--bad_flag’\n请尝试执行 \"tac --help\" 来获取更多信息。\n"
    );
}

#[test]
fn stdin_directory_preserves_read_errno() {
    let directory = tempfile::tempdir().unwrap();
    let stdin_directory = directory.path().join("stdin-directory");
    fs::create_dir(&stdin_directory).unwrap();
    let stdin = File::open(stdin_directory).unwrap();

    let output = run_tac("C", |command| {
        command.stdin(Stdio::from(stdin));
    });

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "tac: 'standard input': read error: Is a directory\n"
    );
}
