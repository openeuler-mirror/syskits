#[cfg(target_os = "linux")]
#[test]
fn suppress_matched_allows_a_boundary_line_past_eof() {
    use std::process::Command;

    for (input, arguments, expected_error, expected_splits) in [
        (
            b"a\n".as_slice(),
            vec!["--suppress-matched", "-q", "input", "2"],
            b"".as_slice(),
            vec![("xx00", b"a\n".as_slice()), ("xx01", b"".as_slice())],
        ),
        (
            b"a\nb\nc\n".as_slice(),
            vec!["--suppress-matched", "-q", "input", "2", "{1}"],
            b"".as_slice(),
            vec![
                ("xx00", b"a\n".as_slice()),
                ("xx01", b"c\n".as_slice()),
                ("xx02", b"".as_slice()),
            ],
        ),
        (
            b"a\n".as_slice(),
            vec!["--suppress-matched", "-q", "input", "2", "{1}"],
            b"csplit: '2': line number out of range on repetition 1\n".as_slice(),
            vec![],
        ),
    ] {
        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        std::fs::write(temp_dir.path().join("input"), input).expect("write input");

        let output = Command::new("bash")
            .args(["-c", "exec -a csplit \"$0\" \"$@\""])
            .arg(env!("CARGO_BIN_EXE_csplit"))
            .args(arguments)
            .current_dir(temp_dir.path())
            .env("LC_ALL", "C")
            .output()
            .expect("run csplit");

        assert_eq!(output.stdout, b"", "{output:?}");
        assert_eq!(output.stderr, expected_error, "{output:?}");
        assert_eq!(
            output.status.success(),
            expected_error.is_empty(),
            "{output:?}"
        );
        for (name, expected_contents) in &expected_splits {
            assert_eq!(
                std::fs::read(temp_dir.path().join(*name)).expect("read split"),
                *expected_contents
            );
        }
        assert_eq!(
            std::fs::read_dir(temp_dir.path())
                .expect("read output directory")
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().as_encoded_bytes().starts_with(b"xx"))
                .count(),
            expected_splits.len()
        );
    }
}
