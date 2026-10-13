#[cfg(target_os = "linux")]
#[test]
fn odd_backslash_runs_before_bre_intervals_preserve_gnu_diagnostics() {
    use std::process::Command;

    for pattern in [r"/a\\\{2,1\}/", r"/a\\\\\{2,1\}/"] {
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
                "csplit: '{}': invalid regular expression: Invalid content of \\{{\\}}\n",
                pattern.replace('\\', "\\\\")
            )
            .as_bytes(),
            "{pattern}"
        );
        assert_eq!(
            std::fs::read_dir(temp_dir.path())
                .expect("read output directory")
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().starts_with("xx"))
                .count(),
            0,
            "{pattern}"
        );
    }
}
