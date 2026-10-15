#[cfg(unix)]
#[test]
fn creation_error_uses_errno_text_without_rust_suffix() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    let output = Command::new(env!("CARGO_BIN_EXE_mkdir"))
        .arg0("mkdir")
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .arg("missing/child")
        .output()
        .expect("run mkdir with a missing parent");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"mkdir: cannot create directory 'missing/child': No such file or directory\n"
    );
}
