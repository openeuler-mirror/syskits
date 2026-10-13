#[cfg(target_os = "linux")]
#[test]
fn suppress_matched_empty_input_uses_the_line_range_error() {
    use std::process::Command;

    for (arguments, keeps_split) in [
        (vec!["--suppress-matched", "-q", "-", "1"], false),
        (vec!["-k", "--suppress-matched", "-q", "-", "1"], true),
    ] {
        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        let output = Command::new("bash")
            .args(["-c", "printf '' | exec -a csplit \"$0\" \"$@\""])
            .arg(env!("CARGO_BIN_EXE_csplit"))
            .args(arguments)
            .current_dir(temp_dir.path())
            .env("LC_ALL", "C")
            .output()
            .expect("run csplit");

        assert!(!output.status.success(), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        assert_eq!(
            output.stderr, b"csplit: '1': line number out of range\n",
            "{output:?}"
        );
        assert_eq!(temp_dir.path().join("xx00").exists(), keeps_split);
    }
}
