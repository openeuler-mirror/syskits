#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[test]
fn escaped_intervals_in_posix_bracket_constructs_are_not_validated_as_repeats() {
    use std::os::unix::ffi::OsStrExt;
    use std::process::Command;

    for pattern in [
        "/[[:alpha:]\\{2,1\\}]/",
        "/[a[:digit:]\\{4,1\\}]/",
        "/[[.a.]\\{2,1\\}]/",
    ] {
        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        std::fs::write(temp_dir.path().join("input"), b"a\n2\n{\n").expect("write input");

        let output = Command::new("bash")
            .args(["-c", "exec -a csplit \"$0\" \"$@\""])
            .arg(env!("CARGO_BIN_EXE_csplit"))
            .args(["-q", "input", pattern, "{*}"])
            .current_dir(temp_dir.path())
            .env("LC_ALL", "C")
            .output()
            .expect("run csplit");

        assert!(output.status.success(), "{pattern}: {output:?}");
        assert!(output.stdout.is_empty(), "{pattern}: {output:?}");
        assert!(output.stderr.is_empty(), "{pattern}: {output:?}");
        for (name, expected_contents) in [
            ("xx00", b"".as_slice()),
            ("xx01", b"a\n".as_slice()),
            ("xx02", b"2\n".as_slice()),
            ("xx03", b"{\n".as_slice()),
        ] {
            assert_eq!(
                std::fs::read(temp_dir.path().join(name)).expect("read split"),
                expected_contents,
                "{pattern}"
            );
        }
        assert_eq!(
            std::fs::read_dir(temp_dir.path())
                .expect("read output directory")
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().as_os_str().as_bytes().starts_with(b"xx"))
                .count(),
            4,
            "{pattern}"
        );
    }
}
