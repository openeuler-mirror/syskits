#[test]
fn repeated_bre_range_operators_use_the_gnu_diagnostic() {
    use std::process::Command;

    for pattern in ["/[a-b-c]/", "/[0-9-a]/", "/[a-b-c-d]/"] {
        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        std::fs::write(temp_dir.path().join("input"), b"a\n-\n5\n").expect("write input");

        let output = Command::new("bash")
            .args(["-c", "exec -a csplit \"$0\" \"$@\""])
            .arg(env!("CARGO_BIN_EXE_csplit"))
            .args(["-q", "input", pattern])
            .current_dir(temp_dir.path())
            .env("LC_ALL", "C")
            .output()
            .expect("run csplit");

        assert_eq!(output.status.code(), Some(1), "{pattern}: {output:?}");
        assert!(output.stdout.is_empty(), "{pattern}: {output:?}");
        assert_eq!(
            output.stderr,
            format!("csplit: '{pattern}': invalid regular expression: Invalid range end\n")
                .as_bytes(),
            "{pattern}"
        );
        assert!(!temp_dir.path().join("xx00").exists(), "{pattern}");
    }
}
