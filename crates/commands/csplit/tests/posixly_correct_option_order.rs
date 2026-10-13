#[cfg(unix)]
#[test]
fn posixly_correct_stops_option_parsing_at_the_first_operand() {
    use std::process::Command;

    for posixly_correct in ["", "1"] {
        for (arguments, invalid_pattern) in [
            (["input", "-q", "2"], "-q"),
            (["input", "2", "--quiet"], "--quiet"),
            (["input", "--", "2"], "--"),
        ] {
            let temp_dir = tempfile::tempdir().expect("create temporary directory");
            std::fs::write(temp_dir.path().join("input"), b"a\nb\n").expect("write input");

            let output = Command::new("bash")
                .args(["-c", "exec -a csplit \"$0\" \"$@\""])
                .arg(env!("CARGO_BIN_EXE_csplit"))
                .args(arguments)
                .current_dir(temp_dir.path())
                .env("LC_ALL", "C")
                .env("POSIXLY_CORRECT", posixly_correct)
                .output()
                .expect("run csplit");

            assert_eq!(output.status.code(), Some(1), "{arguments:?}: {output:?}");
            assert!(output.stdout.is_empty(), "{arguments:?}: {output:?}");
            assert_eq!(
                output.stderr,
                format!("csplit: '{invalid_pattern}': invalid pattern\n").as_bytes(),
                "{arguments:?}"
            );
            assert_eq!(output_file_count(temp_dir.path()), 0, "{arguments:?}");
        }
    }
}

#[cfg(unix)]
#[test]
fn default_mode_keeps_gnu_option_permutation() {
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    std::fs::write(temp_dir.path().join("input"), b"a\nb\n").expect("write input");

    let output = Command::new("bash")
        .args(["-c", "exec -a csplit \"$0\" \"$@\""])
        .arg(env!("CARGO_BIN_EXE_csplit"))
        .args(["input", "-q", "2"])
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .env_remove("POSIXLY_CORRECT")
        .output()
        .expect("run csplit");

    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        std::fs::read(temp_dir.path().join("xx00")).expect("read first split"),
        b"a\n"
    );
    assert_eq!(
        std::fs::read(temp_dir.path().join("xx01")).expect("read second split"),
        b"b\n"
    );
    assert_eq!(output_file_count(temp_dir.path()), 2);
}

#[cfg(unix)]
fn output_file_count(directory: &std::path::Path) -> usize {
    std::fs::read_dir(directory)
        .expect("read output directory")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("xx"))
        .count()
}
