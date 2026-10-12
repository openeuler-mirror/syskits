#[cfg(unix)]
#[test]
fn reversed_bre_intervals_use_gnu_error_semantics() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp_dir.path().join("input"), b"a\naa\naaa\n").expect("write input");

    let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
        .arg0("csplit")
        .current_dir(temp_dir.path())
        .args(["input", "/a\\{2,1\\}/"])
        .output()
        .expect("run csplit with invalid BRE interval");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    assert!(stderr.starts_with("csplit: "));
    assert!(stderr.ends_with(": invalid regular expression: Invalid content of \\{\\}\n"));
}
