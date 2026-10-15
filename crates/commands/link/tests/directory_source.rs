#[cfg(unix)]
#[test]
fn directory_source_uses_gnu_eperm_diagnostic() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    let source = temp_dir.path().join("source");
    let target = temp_dir.path().join("target");
    std::fs::create_dir(&source).expect("create source directory");

    let output = Command::new(env!("CARGO_BIN_EXE_link"))
        .arg0("link")
        .env("LC_ALL", "C")
        .args([source.as_os_str(), target.as_os_str()])
        .output()
        .expect("run link");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        format!(
            "link: cannot create link '{}' to '{}': Operation not permitted\n",
            target.display(),
            source.display()
        )
        .as_bytes()
    );
}
