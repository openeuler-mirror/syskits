#[cfg(target_os = "linux")]
#[test]
fn suppress_matched_removes_lines_after_skip_patterns_without_affecting_line_patterns() {
    use std::process::Command;

    for (input, arguments, expected_stdout, splits) in [
        (
            b"a\nb\na\nc\n".as_slice(),
            vec!["--suppress-matched", "input", "%a%", "{1}"],
            b"2\n".as_slice(),
            vec![("xx00", b"c\n".as_slice())],
        ),
        (
            b"a\nb\nc\n".as_slice(),
            vec!["--suppress-matched", "input", "2"],
            b"2\n2\n".as_slice(),
            vec![("xx00", b"a\n".as_slice()), ("xx01", b"c\n".as_slice())],
        ),
        (
            b"a\nb\na\nc\nd\n".as_slice(),
            vec!["--suppress-matched", "input", "%a%+1"],
            b"6\n".as_slice(),
            vec![("xx00", b"a\nc\nd\n".as_slice())],
        ),
    ] {
        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        std::fs::write(temp_dir.path().join("input"), input).expect("write input");

        let output = Command::new("bash")
            .args(["-c", "exec -a csplit \"$0\" \"$@\""])
            .arg(env!("CARGO_BIN_EXE_csplit"))
            .args(arguments)
            .current_dir(temp_dir.path())
            .output()
            .expect("run csplit");

        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, expected_stdout, "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        for (name, expected_contents) in splits {
            assert_eq!(
                std::fs::read(temp_dir.path().join(name)).expect("read split"),
                expected_contents
            );
        }
    }
}
