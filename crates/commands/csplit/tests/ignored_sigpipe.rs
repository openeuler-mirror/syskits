#[cfg(target_os = "linux")]
#[test]
fn inherited_ignored_sigpipe_reports_a_write_error_and_keeps_splits() {
    use std::process::Command;

    const SCRIPT: &str = r#"
trap '' PIPE
set -o pipefail
{ seq 10000 | bash -c 'exec -a csplit "$0" - "/^/" "{*}"' "$1" 2>stderr; printf '%s' "${PIPESTATUS[1]}" > status; } | head -n 1 >/dev/null
"#;

    let temp_dir = tempfile::tempdir().expect("tempdir");
    let output = Command::new("bash")
        .args(["-c", SCRIPT, "bash", env!("CARGO_BIN_EXE_csplit")])
        .env("LC_ALL", "C")
        .current_dir(temp_dir.path())
        .output()
        .expect("run csplit with ignored SIGPIPE");

    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert_eq!(std::fs::read(temp_dir.path().join("status")).unwrap(), b"1");
    assert_eq!(
        std::fs::read(temp_dir.path().join("stderr")).unwrap(),
        b"csplit: write error: Broken pipe\n"
    );
    assert!(
        std::fs::read_dir(temp_dir.path())
            .expect("read temporary directory")
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().starts_with("xx"))
    );
}
