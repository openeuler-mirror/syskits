#[cfg(target_os = "linux")]
#[test]
fn an_output_write_failure_uses_the_gnu_write_diagnostic() {
    use std::os::unix::fs::symlink;
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    let mut input = vec![b'a'; 100_000];
    input.push(b'\n');
    std::fs::write(temp_dir.path().join("input"), input).expect("write input");
    symlink("/dev/full", temp_dir.path().join("xx00")).expect("link output to /dev/full");

    let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
        .arg0("csplit")
        .args(["-q", "input", "2"])
        .env("LC_ALL", "C")
        .current_dir(temp_dir.path())
        .output()
        .expect("run csplit with output write error");

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert_eq!(
        output.stderr, b"csplit: write error for 'xx00': No space left on device\n",
        "{output:?}"
    );
    assert!(!temp_dir.path().join("xx00").exists());
}
