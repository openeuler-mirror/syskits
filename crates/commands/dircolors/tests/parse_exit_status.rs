use std::os::unix::process::CommandExt;
use std::process::{Command, Output};

fn run_dircolors(input: &str) -> (Output, String) {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), input).unwrap();
    let file = file.path().to_str().unwrap().to_string();

    let output = Command::new(env!("CARGO_BIN_EXE_dircolors"))
        .arg0("dircolors")
        .args(["-b", file.as_str()])
        .env("LC_ALL", "C")
        .env_remove("TERM")
        .env_remove("COLORTERM")
        .output()
        .unwrap();

    (output, file)
}

#[test]
fn valid_input_exits_successfully() {
    let (output, _) = run_dircolors("DIR 01;34\n");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "LS_COLORS='di=01;34:';\nexport LS_COLORS\n"
    );
}

#[test]
fn parse_warnings_suppress_output_and_fail() {
    let (output, file) = run_dircolors("DIR 01;34\nINVALID\n");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        format!("dircolors: {file}:2: invalid line;  missing second token\n")
    );
}

#[test]
fn unrecognized_keywords_suppress_output_and_fail() {
    let (output, file) = run_dircolors("TERM none\nDIR 01;34\nUNKNOWN 01;32\n");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        format!("dircolors: {file}:3: unrecognized keyword UNKNOWN\n")
    );
}

#[test]
fn missing_file_reports_the_gnu_error_message() {
    let directory = tempfile::tempdir().unwrap();
    let missing_file = directory.path().join("missing");
    let missing_file = missing_file.to_str().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_dircolors"))
        .arg0("dircolors")
        .args(["-b", missing_file])
        .env("LC_ALL", "C")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        format!("dircolors: {missing_file}: No such file or directory\n")
    );
}
