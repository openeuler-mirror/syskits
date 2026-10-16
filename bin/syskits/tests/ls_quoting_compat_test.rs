use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::process::{Command, Output};

use tempfile::TempDir;

fn run_gnu(command: &str, directory: &TempDir, names: &[&OsStr]) -> Output {
    let mut process = Command::new(format!("/usr/bin/{command}"));
    process
        .current_dir(directory.path())
        .env("LC_ALL", "C")
        .arg("--quoting-style=shell")
        .args(names);
    process
        .output()
        .unwrap_or_else(|err| panic!("run GNU {command}: {err}"))
}

fn run_syskits(command: &str, directory: &TempDir, names: &[&OsStr]) -> Output {
    let mut process = Command::new(env!("CARGO_BIN_EXE_syskits"));
    process
        .current_dir(directory.path())
        .env("LC_ALL", "C")
        .arg(command)
        .arg("--quoting-style=shell")
        .args(names);
    process
        .output()
        .unwrap_or_else(|err| panic!("run syskits {command}: {err}"))
}

#[test]
fn shell_quoting_of_newline_matches_gnu_for_ls_dir_and_vdir() {
    let directory = TempDir::new().expect("tempdir");
    let newline_name = OsStr::from_bytes(b"line\nname");
    let space_name = OsStr::new("space name");
    fs::write(directory.path().join(newline_name), b"").expect("create newline fixture");
    fs::write(directory.path().join(space_name), b"").expect("create space fixture");

    for command in ["ls", "dir", "vdir"] {
        let names = [newline_name, space_name];
        let gnu = run_gnu(command, &directory, &names);
        let syskits = run_syskits(command, &directory, &names);

        assert_eq!(syskits.status.code(), gnu.status.code(), "{command} status");
        assert_eq!(syskits.stderr, gnu.stderr, "{command} stderr");
        assert_eq!(
            syskits.stdout,
            gnu.stdout,
            "{command} stdout: GNU={:?}, syskits={:?}",
            String::from_utf8_lossy(&gnu.stdout),
            String::from_utf8_lossy(&syskits.stdout)
        );
    }
}
