#[cfg(target_os = "linux")]
#[test]
fn empty_bre_interval_lower_bounds_match_gnu() {
    use std::os::unix::ffi::OsStrExt;
    use std::process::Command;

    for (pattern, expected_splits) in [
        (
            "/^a\\{,2\\}$/",
            vec![
                ("xx00", b"".as_slice()),
                ("xx01", b"\n".as_slice()),
                ("xx02", b"a\n".as_slice()),
                ("xx03", b"aa\naaa\nb\n".as_slice()),
            ],
        ),
        (
            "/^a\\{,\\}$/",
            vec![
                ("xx00", b"".as_slice()),
                ("xx01", b"\n".as_slice()),
                ("xx02", b"a\n".as_slice()),
                ("xx03", b"aa\n".as_slice()),
                ("xx04", b"aaa\nb\n".as_slice()),
            ],
        ),
    ] {
        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        std::fs::write(temp_dir.path().join("input"), b"\na\naa\naaa\nb\n").expect("write input");

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
        for (name, expected_contents) in &expected_splits {
            assert_eq!(
                std::fs::read(temp_dir.path().join(name)).expect("read split"),
                *expected_contents,
                "{pattern}"
            );
        }
        assert_eq!(
            std::fs::read_dir(temp_dir.path())
                .expect("read output directory")
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().as_os_str().as_bytes().starts_with(b"xx"))
                .count(),
            expected_splits.len(),
            "{pattern}"
        );
    }

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    std::fs::write(temp_dir.path().join("input"), b"{,2}\nother\n").expect("write input");
    let output = Command::new("bash")
        .args(["-c", "exec -a csplit \"$0\" \"$@\""])
        .arg(env!("CARGO_BIN_EXE_csplit"))
        .args(["-q", "input", r"/\{,2\}/", "{*}"])
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .output()
        .expect("run csplit");

    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        std::fs::read(temp_dir.path().join("xx00")).expect("read first split"),
        b""
    );
    assert_eq!(
        std::fs::read(temp_dir.path().join("xx01")).expect("read second split"),
        b"{,2}\nother\n"
    );
}
