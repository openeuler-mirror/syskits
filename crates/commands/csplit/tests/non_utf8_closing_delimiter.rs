#[cfg(target_os = "linux")]
#[test]
fn non_utf8_missing_delimiter_diagnostic_preserves_the_original_bytes() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    std::fs::write(temp_dir.path().join("input"), b"a\n").expect("write input");
    let output = Command::new("bash")
        .args(["-c", "exec -a csplit \"$0\" \"$@\""])
        .arg(env!("CARGO_BIN_EXE_csplit"))
        .args(["input"])
        .arg(OsString::from_vec(b"/\xff".to_vec()))
        .env("LC_ALL", "C")
        .current_dir(temp_dir.path())
        .output()
        .expect("run csplit");

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert_eq!(
        output.stderr,
        b"csplit: /\xff: closing delimiter '/' missing\n"
    );
}
