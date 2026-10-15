#[cfg(unix)]
#[test]
fn creates_non_utf8_fifo_name_without_lossy_conversion() {
    use std::ffi::OsString;
    use std::os::unix::{ffi::OsStringExt, fs::FileTypeExt, process::CommandExt};
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    let fifo_name = OsString::from_vec(vec![b'f', b'i', b'f', b'o', 0xff]);
    let output = Command::new(env!("CARGO_BIN_EXE_mkfifo"))
        .arg0("mkfifo")
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .arg(&fifo_name)
        .output()
        .expect("run mkfifo with a non-UTF-8 name");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert!(
        std::fs::metadata(temp_dir.path().join(fifo_name))
            .expect("read FIFO metadata")
            .file_type()
            .is_fifo()
    );
}
