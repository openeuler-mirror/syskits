#[cfg(target_os = "linux")]
#[test]
fn a_count_write_failure_keeps_completed_splits() {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    std::fs::write(temp_dir.path().join("input"), b"a\nb\n").expect("write input");
    let dev_full = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .expect("open /dev/full");

    let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
        .arg0("csplit")
        .args(["input", "2"])
        .env("LC_ALL", "C")
        .current_dir(temp_dir.path())
        .stdout(Stdio::from(dev_full))
        .output()
        .expect("run csplit with count write error");

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(
        output.stderr, b"csplit: write error: No space left on device\n",
        "{output:?}"
    );
    assert_eq!(
        std::fs::read(temp_dir.path().join("xx00")).expect("read first split"),
        b"a\n"
    );
    assert_eq!(
        std::fs::read(temp_dir.path().join("xx01")).expect("read second split"),
        b"b\n"
    );

    let error_dir = tempfile::tempdir().expect("create error temporary directory");
    std::fs::write(error_dir.path().join("input"), b"a\n").expect("write error input");
    let dev_full = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .expect("open /dev/full");
    let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
        .arg0("csplit")
        .args(["input", "3"])
        .env("LC_ALL", "C")
        .current_dir(error_dir.path())
        .stdout(Stdio::from(dev_full))
        .output()
        .expect("run csplit with split and count errors");

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(
        output.stderr,
        b"csplit: '3': line number out of range\n\
csplit: write error: No space left on device\n",
        "{output:?}"
    );
    assert!(!error_dir.path().join("xx00").exists());
}
