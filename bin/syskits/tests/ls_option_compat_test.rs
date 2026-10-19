use std::fs;
use std::process::{Command, Output};

use tempfile::TempDir;

fn run_gnu(directory: &TempDir, arguments: &[&str]) -> Output {
    Command::new("/usr/bin/ls")
        .current_dir(directory.path())
        .env("LC_ALL", "C")
        .args(arguments)
        .output()
        .expect("run GNU ls")
}

fn run_syskits(directory: &TempDir, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(directory.path())
        .env("LC_ALL", "C")
        .arg("ls")
        .args(arguments)
        .output()
        .expect("run syskits ls")
}

#[test]
fn ls_accepts_separated_required_long_option_values() {
    let directory = TempDir::new().expect("tempdir");
    fs::write(directory.path().join("a"), b"").expect("create fixture");

    for arguments in [
        ["--format", "long", "."],
        ["--sort", "size", "."],
        ["--time", "atime", "."],
        ["--block-size", "1", "."],
    ] {
        let gnu = run_gnu(&directory, &arguments);
        let syskits = run_syskits(&directory, &arguments);

        assert_eq!(syskits.status.code(), gnu.status.code(), "{arguments:?}");
        assert_eq!(syskits.stderr, gnu.stderr, "{arguments:?}");
    }
}

#[test]
fn ls_extensions_do_not_collide_with_gnu_value_abbreviations() {
    let directory = TempDir::new().expect("tempdir");
    fs::write(directory.path().join("a"), b"").expect("create fixture a");
    fs::write(directory.path().join("b"), b"").expect("create fixture b");

    for arguments in [["--format=c", "."], ["--format=co", "."], ["--sort=n", "."]] {
        let gnu = run_gnu(&directory, &arguments);
        let syskits = run_syskits(&directory, &arguments);

        assert_eq!(syskits.status.code(), gnu.status.code(), "{arguments:?}");
        assert_eq!(syskits.stderr, gnu.stderr, "{arguments:?}");
        assert_eq!(syskits.stdout, gnu.stdout, "{arguments:?}");
    }
}

#[test]
fn ls_invalid_enum_values_use_gnu_failure_status() {
    let directory = TempDir::new().expect("tempdir");
    fs::write(directory.path().join("a"), b"").expect("create fixture");

    for argument in [
        "--hyperlink=bad",
        "--indicator-style=bad",
        "--quoting-style=bad",
    ] {
        let gnu = run_gnu(&directory, &[argument, "a"]);
        let syskits = run_syskits(&directory, &[argument, "a"]);

        assert_eq!(syskits.status.code(), gnu.status.code(), "{argument}");
        assert_eq!(syskits.status.code(), Some(1), "{argument}");
    }
}
