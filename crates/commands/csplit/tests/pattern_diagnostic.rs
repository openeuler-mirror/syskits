#[cfg(unix)]
#[test]
fn regexp_errors_quote_the_original_pattern() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp_dir.path().join("input"), b"a\n").expect("write input");

    let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
        .arg0("csplit")
        .current_dir(temp_dir.path())
        .arg("input")
        .arg("/z/1")
        .output()
        .expect("run csplit");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stdout, b"2\n");
    assert_eq!(output.stderr, b"csplit: '/z/1': match not found\n");
}
