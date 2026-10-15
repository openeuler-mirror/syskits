#[cfg(unix)]
#[test]
fn existing_target_uses_gnu_eexist_diagnostic() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    let source = temp_dir.path().join("source");
    let target = temp_dir.path().join("target");
    std::fs::write(&source, b"source").expect("create source");
    std::fs::write(&target, b"target").expect("create target");

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
            "link: cannot create link '{}' to '{}': File exists\n",
            target.display(),
            source.display()
        )
        .as_bytes()
    );
}
