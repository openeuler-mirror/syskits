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

#[test]
fn ls_conditional_shell_special_characters_match_gnu() {
    let directory = TempDir::new().expect("tempdir");
    let names = ["x=x", "x]x", "x^x", "x{x", "x}x", "{", "}"];
    for name in names {
        fs::write(directory.path().join(name), b"").expect("create shell quoting fixture");
    }
    let names = names.map(OsStr::new);

    let gnu = run_gnu("ls", &directory, &names);
    let syskits = run_syskits("ls", &directory, &names);

    assert_eq!(syskits.status.code(), gnu.status.code());
    assert_eq!(syskits.stderr, gnu.stderr);
    assert_eq!(syskits.stdout, gnu.stdout);
}

#[cfg(target_os = "linux")]
#[test]
fn ls_non_utf8_names_match_gnu_for_literal_and_shell_styles() {
    let directory = TempDir::new().expect("tempdir");
    let name = OsStr::from_bytes(b"bad-\xff");
    fs::write(directory.path().join(name), b"").expect("create non-UTF-8 fixture");

    let cases: [&[&OsStr]; 3] = [
        &[OsStr::new("-1"), OsStr::new("--quoting-style=literal")],
        &[OsStr::new("-q1"), OsStr::new("--quoting-style=literal")],
        &[OsStr::new("-1"), OsStr::new("--quoting-style=shell-escape")],
    ];

    for arguments in cases {
        let gnu = Command::new("/usr/bin/ls")
            .current_dir(directory.path())
            .env("LC_ALL", "C")
            .args(arguments)
            .output()
            .expect("run GNU ls");
        let syskits = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .current_dir(directory.path())
            .env("LC_ALL", "C")
            .arg("ls")
            .args(arguments)
            .output()
            .expect("run syskits ls");

        assert_eq!(syskits.status.code(), gnu.status.code(), "{arguments:?}");
        assert_eq!(syskits.stderr, gnu.stderr, "{arguments:?}");
        assert_eq!(syskits.stdout, gnu.stdout, "{arguments:?}");
    }
}
