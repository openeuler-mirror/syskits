#[cfg(unix)]
#[test]
fn numeric_splits_on_empty_input_preserve_the_initial_empty_file() {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    for options in [vec![], vec!["-q"], vec!["-z"], vec!["-k"]] {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
            .arg0("csplit")
            .env("LC_ALL", "C")
            .current_dir(temp_dir.path())
            .args(options)
            .args(["-", "1"])
            .stdin(Stdio::null())
            .output()
            .expect("run csplit with empty standard input");

        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr, b"csplit: input disappeared\n");
        assert_eq!(
            std::fs::metadata(temp_dir.path().join("xx00"))
                .unwrap()
                .len(),
            0
        );
    }
}
