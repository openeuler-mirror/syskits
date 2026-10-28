#![cfg(target_os = "linux")]

use std::fs::write;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Output};

fn run_dd(binary: &str, working_dir: &Path, input_name: &str) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0("dd")
        .current_dir(working_dir)
        .env("LC_ALL", "C")
        .args([
            format!("if={input_name}"),
            String::from("of=test_dd_output"),
            String::from("bs=1"),
            String::from("skip=1000"),
            String::from("status=noxfer"),
        ]);
    command.output().expect("run dd")
}

#[test]
fn skip_past_eof_quotes_input_name_like_gnu() {
    for input_name in ["test_dd_input", "test dd input"] {
        let working_dir = tempfile::tempdir().expect("create working directory");
        write(working_dir.path().join(input_name), b"hello\n").expect("write input file");

        let gnu = run_dd("/usr/bin/dd", working_dir.path(), input_name);
        let syskits = run_dd(env!("CARGO_BIN_EXE_dd"), working_dir.path(), input_name);

        assert_eq!(syskits.status.code(), gnu.status.code(), "{input_name}");
        assert_eq!(syskits.stdout, gnu.stdout, "{input_name}");
        assert_eq!(syskits.stderr, gnu.stderr, "{input_name}");
    }
}
