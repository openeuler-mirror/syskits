#[cfg(target_os = "linux")]
#[test]
fn non_utf8_output_create_error_preserves_the_target_filename() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    std::fs::write(temp_dir.path().join("input"), b"a\n").expect("write input");
    let prefix = OsString::from_vec(b"missing/\xff".to_vec());

    let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
        .arg0("csplit")
        .args(["-f"])
        .arg(prefix)
        .args(["input", "1"])
        .env("LC_ALL", "C")
        .current_dir(temp_dir.path())
        .output()
        .expect("run csplit");

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert_eq!(
        output.stderr, b"csplit: 'missing/'$'\\377''00': No such file or directory\n",
        "{output:?}"
    );
}
