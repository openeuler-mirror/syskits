#[cfg(unix)]
#[test]
fn verbose_output_write_error_uses_gnu_diagnostic() {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    let output = Command::new(env!("CARGO_BIN_EXE_mkdir"))
        .arg0("mkdir")
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args(["-v", "created"])
        .stdout(Stdio::from(
            std::fs::File::options()
                .write(true)
                .open("/dev/full")
                .expect("open /dev/full"),
        ))
        .output()
        .expect("run mkdir with a full stdout device");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"mkdir: write error: No space left on device\n"
    );
    assert!(temp_dir.path().join("created").is_dir());
}
