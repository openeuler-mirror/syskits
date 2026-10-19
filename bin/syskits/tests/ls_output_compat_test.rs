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
fn ls_zero_comma_wraps_with_nul_records() {
    let directory = TempDir::new().expect("tempdir");
    fs::create_dir(directory.path().join("d")).expect("create fixture directory");
    for name in ["a", "bb", "ccc", "dddd", "eeeee"] {
        fs::write(directory.path().join("d").join(name), b"").expect("create fixture file");
    }
    let arguments = ["--zero", "-m", "-w5", "d"];

    let gnu = run_gnu(&directory, &arguments);
    let syskits = run_syskits(&directory, &arguments);

    assert_eq!(syskits.status.code(), gnu.status.code());
    assert_eq!(syskits.stderr, gnu.stderr);
    assert_eq!(syskits.stdout, gnu.stdout);
}
