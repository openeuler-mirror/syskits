#[cfg(unix)]
#[test]
fn positive_offset_intmax_overflow_preserves_only_pre_match_lines() {
    use std::process::Command;

    for (offset, expected_stdout, expected_split) in [
        (
            "9223372036854775805",
            b"6\n".as_slice(),
            b"a\nb\nc\n".as_slice(),
        ),
        ("9223372036854775806", b"2\n".as_slice(), b"a\n".as_slice()),
        ("9223372036854775807", b"2\n".as_slice(), b"a\n".as_slice()),
    ] {
        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        std::fs::write(temp_dir.path().join("input"), b"a\nb\nc\n").expect("write input");
        let pattern = format!("/b/+{offset}");

        let output = Command::new("bash")
            .args(["-c", "exec -a csplit \"$0\" \"$@\""])
            .arg(env!("CARGO_BIN_EXE_csplit"))
            .args(["-k", "input", &pattern])
            .current_dir(temp_dir.path())
            .env("LC_ALL", "C")
            .output()
            .expect("run csplit");

        assert_eq!(output.status.code(), Some(1), "{pattern}: {output:?}");
        assert_eq!(output.stdout, expected_stdout, "{pattern}: {output:?}");
        assert_eq!(
            output.stderr,
            format!("csplit: '{pattern}': line number out of range\n").as_bytes(),
            "{pattern}"
        );
        assert_eq!(
            std::fs::read(temp_dir.path().join("xx00")).expect("read split"),
            expected_split,
            "{pattern}"
        );
        assert!(!temp_dir.path().join("xx01").exists(), "{pattern}");
    }
}
