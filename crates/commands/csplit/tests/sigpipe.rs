#[cfg(target_os = "linux")]
#[test]
fn closed_stdout_pipe_terminates_with_sigpipe() {
    use std::process::Command;

    for keep_files in [false, true] {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let csplit_args = if keep_files {
            "-k - '/^/' '{*}'"
        } else {
            "- '/^/' '{*}'"
        };
        let output = Command::new("bash")
            .arg("-c")
            .arg(format!(
                "set -o pipefail; seq 10000 | \"$1\" {csplit_args} | head -n 1 >/dev/null; printf '%s' \"${{PIPESTATUS[1]}}\""
            ))
            .arg("bash")
            .arg(env!("CARGO_BIN_EXE_csplit"))
            .env("LC_ALL", "C")
            .current_dir(temp_dir.path())
            .output()
            .expect("run csplit with a closed stdout pipe");

        assert!(output.status.success());
        assert_eq!(output.stdout, b"141");
        assert!(output.stderr.is_empty());

        let created_splits = std::fs::read_dir(temp_dir.path())
            .expect("read temporary directory")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("xx"))
            .count();
        assert_eq!(created_splits > 0, keep_files, "keep_files={keep_files}");
    }
}
