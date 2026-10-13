#[test]
fn invalid_bre_backreferences_use_gnu_compile_semantics() {
    use std::process::Command;

    for pattern in [
        "/\\1/",
        "/\\9/",
        "/\\10/",
        "/\\(a\\)\\2/",
        "/\\1\\(a\\)/",
        "/\\2\\(a\\)\\(b\\)/",
        "/\\(\\1a\\)/",
        "/\\(a\\(b\\)\\)\\3/",
    ] {
        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        std::fs::write(temp_dir.path().join("input"), b"a\naa\na0\nab\n").expect("write input");

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
                "csplit: '{}': invalid regular expression: Invalid back reference\n",
                pattern.replace('\\', "\\\\")
            )
            .as_bytes(),
            "{pattern}"
        );
        assert!(!temp_dir.path().join("xx00").exists(), "{pattern}");
    }

    for pattern in ["/[\\(\\)\\1]/", "/[[:alpha:]\\1]/"] {
        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        std::fs::write(temp_dir.path().join("input"), b"a\n1\n").expect("write input");

        let output = Command::new("bash")
            .args(["-c", "exec -a csplit \"$0\" \"$@\""])
            .arg(env!("CARGO_BIN_EXE_csplit"))
            .args(["-q", "input", pattern])
            .current_dir(temp_dir.path())
            .env("LC_ALL", "C")
            .output()
            .expect("run csplit");

        assert!(output.status.success(), "{pattern}: {output:?}");
        assert!(output.stdout.is_empty(), "{pattern}: {output:?}");
        assert!(output.stderr.is_empty(), "{pattern}: {output:?}");
    }
}
