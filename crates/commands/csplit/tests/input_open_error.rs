#[cfg(unix)]
#[test]
fn missing_input_uses_the_gnu_open_diagnostic() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("tempdir");
    let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
        .arg0("csplit")
        .current_dir(temp_dir.path())
        .args(["missing", "1"])
        .output()
        .expect("run csplit");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"csplit: cannot open 'missing' for reading: No such file or directory\n"
    );
}
