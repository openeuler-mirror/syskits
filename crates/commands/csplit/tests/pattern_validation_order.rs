#[cfg(target_os = "linux")]
#[test]
fn pattern_validation_precedes_the_following_repeat_count() {
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    std::fs::write(temp_dir.path().join("input"), b"a\nb\n").expect("write input");

    for (arguments, expected) in [
        (
            ["0", "{bad}"].as_slice(),
            b"csplit: 0: line number must be greater than zero\n".as_slice(),
        ),
        (
            ["bad", "{bad}"].as_slice(),
            b"csplit: 'bad': invalid pattern\n".as_slice(),
        ),
        (
            ["2", "1", "{bad}"].as_slice(),
            b"csplit: line number '1' is smaller than preceding line number, 2\n".as_slice(),
        ),
    ] {
        let output = Command::new("bash")
            .args(["-c", "exec -a csplit \"$0\" \"$@\""])
            .arg(env!("CARGO_BIN_EXE_csplit"))
            .args(["input"])
            .args(arguments)
            .env("LC_ALL", "C")
            .current_dir(temp_dir.path())
            .output()
            .expect("run csplit");

        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        assert_eq!(output.stderr, expected, "{output:?}");
    }
}
