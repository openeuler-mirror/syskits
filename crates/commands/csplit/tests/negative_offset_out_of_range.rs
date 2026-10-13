#[cfg(unix)]
#[test]
fn negative_offset_out_of_range_closes_the_empty_split_before_failing() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp_dir.path().join("input"), b"a\nb\n").expect("write input");

    let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
        .arg0("csplit")
        .env("LC_ALL", "C")
        .current_dir(temp_dir.path())
        .args(["input", "/a/-1"])
        .output()
        .expect("run csplit with an out-of-range negative offset");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stdout, b"0\n");
    assert_eq!(
        output.stderr,
        b"csplit: '/a/-1': line number out of range\n"
    );
    assert!(!temp_dir.path().join("xx00").exists());
}
