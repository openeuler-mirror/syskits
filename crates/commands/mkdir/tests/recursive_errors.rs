#[cfg(unix)]
#[test]
fn recursive_creation_reports_not_a_directory_for_file_parent() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    std::fs::write(temp_dir.path().join("prefix"), b"file").expect("create prefix file");

    let output = Command::new(env!("CARGO_BIN_EXE_mkdir"))
        .arg0("mkdir")
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args(["-p", "prefix/child", "second"])
        .output()
        .expect("run mkdir with a non-directory parent");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"mkdir: cannot create directory 'prefix': Not a directory\n"
    );
    assert!(temp_dir.path().join("second").is_dir());
}
