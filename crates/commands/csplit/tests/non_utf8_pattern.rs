#[cfg(target_os = "linux")]
#[test]
fn non_utf8_pattern_matches_the_original_input_bytes() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    let pattern = OsString::from_vec(b"/\xff/".to_vec());
    std::fs::write(temp_dir.path().join("input"), b"\xff\n").expect("write input");
    let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
        .arg("-q")
        .arg("input")
        .arg(pattern)
        .env("LC_ALL", "C")
        .current_dir(temp_dir.path())
        .output()
        .expect("run csplit");

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(std::fs::read(temp_dir.path().join("xx00")).unwrap(), b"");
    assert_eq!(
        std::fs::read(temp_dir.path().join("xx01")).unwrap(),
        b"\xff\n"
    );
}
