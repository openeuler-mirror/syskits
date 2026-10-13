#[cfg(target_os = "linux")]
#[test]
fn input_read_errors_use_gnu_diagnostics_and_finish_the_current_split() {
    use std::process::Command;

    for (options, expected_stdout) in [
        (Vec::<&str>::new(), b"0\n".as_slice()),
        (vec!["--quiet"], b"".as_slice()),
    ] {
        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        std::fs::create_dir(temp_dir.path().join("input")).expect("create input directory");

        let output = Command::new("bash")
            .args(["-c", "exec -a csplit \"$0\" \"$@\""])
            .arg(env!("CARGO_BIN_EXE_csplit"))
            .args(options)
            .args(["input", "1"])
            .current_dir(temp_dir.path())
            .env("LC_ALL", "C")
            .output()
            .expect("run csplit");

        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert_eq!(output.stdout, expected_stdout, "{output:?}");
        assert_eq!(output.stderr, b"csplit: read error: Is a directory\n");
        assert!(!temp_dir.path().join("xx00").exists());
    }
}
