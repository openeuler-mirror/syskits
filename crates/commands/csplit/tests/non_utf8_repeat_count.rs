#[cfg(target_os = "linux")]
#[test]
fn non_utf8_repeat_count_diagnostics_use_gnu_locale_quoting() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    std::fs::write(temp_dir.path().join("input"), b"a\n").expect("write input");

    for (repeat_count, expected) in [
        (
            OsString::from_vec(b"{\xff".to_vec()),
            b"csplit: '{\\377': '}' is required in repeat count\n".as_slice(),
        ),
        (
            OsString::from_vec(b"{\xff}".to_vec()),
            b"csplit: '{\\377'}: integer required between '{' and '}'\n".as_slice(),
        ),
    ] {
        let output = Command::new("bash")
            .args(["-c", "exec -a csplit \"$0\" \"$@\""])
            .arg(env!("CARGO_BIN_EXE_csplit"))
            .args(["input", "/a/"])
            .arg(repeat_count)
            .env("LC_ALL", "C")
            .current_dir(temp_dir.path())
            .output()
            .expect("run csplit");

        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        assert_eq!(output.stderr, expected, "{output:?}");
    }
}
