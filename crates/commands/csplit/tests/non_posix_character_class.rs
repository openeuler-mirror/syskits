#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[test]
fn oniguruma_only_character_classes_are_rejected() {
    use std::process::Command;

    for pattern in ["/[[:word:]]/", "/[[:ascii:]]/"] {
        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        std::fs::write(temp_dir.path().join("input"), b"a\n").expect("write input");

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
            format!(
                "csplit: '{pattern}': invalid regular expression: Invalid character class name\n"
            )
            .as_bytes(),
            "{pattern}"
        );
        assert!(!temp_dir.path().join("xx00").exists(), "{pattern}");
    }
}
