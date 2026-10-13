#[cfg(target_os = "linux")]
#[test]
fn non_utf8_pattern_diagnostics_use_gnu_locale_quoting() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    std::fs::write(temp_dir.path().join("input"), b"a\n").expect("write input");

    for (pattern, expected) in [
        (
            OsString::from_vec(b"/\xff/".to_vec()),
            b"csplit: '/\\377/': match not found\n".as_slice(),
        ),
        (
            OsString::from_vec(b"/\xff/x".to_vec()),
            b"csplit: '/\\377/x': integer expected after delimiter\n".as_slice(),
        ),
    ] {
        let output = Command::new("bash")
            .args(["-c", "exec -a csplit \"$0\" \"$@\""])
            .arg(env!("CARGO_BIN_EXE_csplit"))
            .args(["-q", "input"])
            .arg(pattern)
            .env("LC_ALL", "C")
            .current_dir(temp_dir.path())
            .output()
            .expect("run csplit");

        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        assert_eq!(output.stderr, expected, "{output:?}");
    }
}
