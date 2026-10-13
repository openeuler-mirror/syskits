#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[test]
fn posix_collating_constructs_follow_gnu_matching_and_diagnostics() {
    use std::os::unix::ffi::OsStrExt;
    use std::process::Command;

    for (input, locale, pattern, expected_splits) in [
        (
            b"a\nb\nz\n".as_slice(),
            "C",
            "/[[.z.]]/",
            vec![("xx00", b"a\nb\n".as_slice()), ("xx01", b"z\n".as_slice())],
        ),
        (
            b"a\nb\nz\n".as_slice(),
            "C",
            "/[[=z=]]/",
            vec![("xx00", b"a\nb\n".as_slice()), ("xx01", b"z\n".as_slice())],
        ),
        (
            b"a\nb\nz\n".as_slice(),
            "C",
            "/[a[.b.]]/",
            vec![
                ("xx00", b"".as_slice()),
                ("xx01", b"a\n".as_slice()),
                ("xx02", b"b\nz\n".as_slice()),
            ],
        ),
        (
            b"a\nb\nz\n".as_slice(),
            "C",
            "/[a[=b=]]/",
            vec![
                ("xx00", b"".as_slice()),
                ("xx01", b"a\n".as_slice()),
                ("xx02", b"b\nz\n".as_slice()),
            ],
        ),
        (
            b"a\n1\nb\n".as_slice(),
            "C",
            "/[^[:digit:][.a.]]/",
            vec![("xx00", b"a\n1\n".as_slice()), ("xx01", b"b\n".as_slice())],
        ),
        (
            b"a\0z\nz\n".as_slice(),
            "C",
            "/[[.z.]]/",
            vec![
                ("xx00", b"".as_slice()),
                ("xx01", b"a\0z\n".as_slice()),
                ("xx02", b"z\n".as_slice()),
            ],
        ),
        (
            b"a\nA\nb\n".as_slice(),
            "en_US.utf8",
            "/[[=a=]]/",
            vec![
                ("xx00", b"".as_slice()),
                ("xx01", b"a\n".as_slice()),
                ("xx02", b"A\nb\n".as_slice()),
            ],
        ),
    ] {
        if locale != "C" && !locale_is_available(locale) {
            continue;
        }

        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        std::fs::write(temp_dir.path().join("input"), input).expect("write input");

        let output = Command::new("bash")
            .args(["-c", "exec -a csplit \"$0\" \"$@\""])
            .arg(env!("CARGO_BIN_EXE_csplit"))
            .args(["-q", "input", pattern, "{*}"])
            .current_dir(temp_dir.path())
            .env("LC_ALL", locale)
            .output()
            .expect("run csplit");

        assert!(output.status.success(), "{pattern}: {output:?}");
        assert!(output.stdout.is_empty(), "{pattern}: {output:?}");
        assert!(output.stderr.is_empty(), "{pattern}: {output:?}");
        assert_output_files(temp_dir.path(), &expected_splits);
    }

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    std::fs::write(temp_dir.path().join("input"), b"a\n").expect("write input");
    let output = Command::new("bash")
        .args(["-c", "exec -a csplit \"$0\" \"$@\""])
        .arg(env!("CARGO_BIN_EXE_csplit"))
        .args(["-q", "input", "/[[.ch.]]/"])
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .output()
        .expect("run csplit");

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert_eq!(
        output.stderr,
        b"csplit: '/[[.ch.]]/': invalid regular expression: Invalid collation character\n"
    );
    assert_output_files(temp_dir.path(), &[]);

    fn assert_output_files(temp_dir: &std::path::Path, expected_splits: &[(&str, &[u8])]) {
        for (name, expected_contents) in expected_splits {
            assert_eq!(
                std::fs::read(temp_dir.join(name)).expect("read split"),
                *expected_contents
            );
        }
        assert_eq!(
            std::fs::read_dir(temp_dir)
                .expect("read output directory")
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().as_os_str().as_bytes().starts_with(b"xx"))
                .count(),
            expected_splits.len()
        );
    }

    fn locale_is_available(locale: &str) -> bool {
        Command::new("locale")
            .arg("-a")
            .output()
            .expect("list locales")
            .stdout
            .split(|byte| *byte == b'\n')
            .any(|name| name == locale.as_bytes())
    }
}
