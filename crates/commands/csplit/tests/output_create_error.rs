#[cfg(unix)]
#[test]
fn output_creation_errors_name_the_target_file() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let directory_case = tempfile::tempdir().expect("tempdir");
    std::fs::write(directory_case.path().join("input"), b"a\n").expect("write input");
    std::fs::create_dir(directory_case.path().join("xx00")).expect("create xx00 directory");

    let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
        .arg0("csplit")
        .current_dir(directory_case.path())
        .args(["input", "1"])
        .output()
        .expect("run csplit");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"csplit: xx00: Is a directory\n");

    let missing_parent_case = tempfile::tempdir().expect("tempdir");
    std::fs::write(missing_parent_case.path().join("input"), b"a\n").expect("write input");
    let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
        .arg0("csplit")
        .current_dir(missing_parent_case.path())
        .args(["-f", "missing/x", "input", "1"])
        .output()
        .expect("run csplit");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"csplit: missing/x00: No such file or directory\n"
    );
}
