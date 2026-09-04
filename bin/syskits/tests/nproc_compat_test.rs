#[cfg(target_os = "linux")]
use std::fs::OpenOptions;
#[cfg(target_os = "linux")]
use std::process::Stdio;
use std::process::{Command, Output};

fn run_nproc(args: &[&str], openmp_limit: bool) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_syskits"));
    command
        .arg("nproc")
        .args(args)
        .env("LC_ALL", "C")
        .env("LANG", "C");

    if openmp_limit {
        command
            .env("OMP_NUM_THREADS", "1")
            .env("OMP_THREAD_LIMIT", "1");
    } else {
        command
            .env_remove("OMP_NUM_THREADS")
            .env_remove("OMP_THREAD_LIMIT");
    }

    command.output().expect("run syskits nproc")
}

#[cfg(target_os = "linux")]
#[test]
fn nproc_reports_stdout_write_errors() {
    let full = OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .expect("open /dev/full");
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .arg("nproc")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdout(Stdio::from(full))
        .output()
        .expect("run syskits nproc");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "nproc: write error: No space left on device\n"
    );
}

#[test]
fn nproc_does_not_duplicate_errors_or_display_output() {
    let help = run_nproc(&["--help"], false);
    assert!(help.status.success());
    assert!(help.stderr.is_empty());
    let help_text = String::from_utf8(help.stdout).expect("UTF-8 help");
    assert_eq!(help_text.matches("Usage: nproc [OPTIONS]...").count(), 1);

    let version = run_nproc(&["--version"], false);
    assert!(version.status.success());
    assert!(version.stderr.is_empty());
    assert_eq!(
        String::from_utf8(version.stdout)
            .expect("UTF-8 version")
            .lines()
            .count(),
        1
    );

    let invalid = run_nproc(&["--ignore=x"], false);
    assert_eq!(invalid.status.code(), Some(1));
    assert!(invalid.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&invalid.stderr),
        "nproc: invalid number: 'x'\n"
    );

    let missing = run_nproc(&["--ignore"], false);
    assert_eq!(missing.status.code(), Some(1));
    assert!(missing.stdout.is_empty());
    let missing_stderr = String::from_utf8_lossy(&missing.stderr);
    assert_eq!(missing_stderr.matches("a value is required").count(), 1);
}
