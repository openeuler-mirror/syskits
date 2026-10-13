#[cfg(target_os = "linux")]
#[test]
fn positive_offsets_skip_the_break_line_during_repeated_searches() {
    use std::process::Command;

    for (input, arguments, expected_success, expected_error, expected_splits) in [
        (
            b"a\na\na\n".as_slice(),
            vec!["-q", "input", "/a/1", "{1}"],
            true,
            b"".as_slice(),
            vec![
                ("xx00", b"a\n".as_slice()),
                ("xx01", b"a\na\n".as_slice()),
                ("xx02", b"".as_slice()),
            ],
        ),
        (
            b"a\na\na\n".as_slice(),
            vec!["-q", "input", "%a%1", "{1}"],
            true,
            b"".as_slice(),
            vec![("xx00", b"".as_slice())],
        ),
        (
            b"a\na\n".as_slice(),
            vec!["-q", "input", "/a/1", "{1}"],
            false,
            b"csplit: '/a/1': match not found on repetition 1\n".as_slice(),
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

        assert_eq!(output.status.success(), expected_success, "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        assert_eq!(output.stderr, expected_error, "{output:?}");
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
