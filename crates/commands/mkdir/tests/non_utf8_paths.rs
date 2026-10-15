#[cfg(unix)]
#[test]
fn creates_non_utf8_directory_name_without_lossy_conversion() {
    use std::ffi::OsString;
    use std::os::unix::{ffi::OsStringExt, process::CommandExt};
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    let directory_name = OsString::from_vec(vec![b'd', b'i', b'r', 0xff]);

    let output = Command::new(env!("CARGO_BIN_EXE_mkdir"))
        .arg0("mkdir")
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .arg(&directory_name)
        .output()
        .expect("run mkdir with a non-UTF-8 directory name");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert!(temp_dir.path().join(&directory_name).is_dir());
}
