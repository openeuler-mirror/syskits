#[cfg(unix)]
#[test]
fn posixly_correct_stops_option_parsing_at_first_operand() {
    use std::os::unix::{fs::MetadataExt, process::CommandExt};
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    let source = temp_dir.path().join("source");
    let target = temp_dir.path().join("target");
    std::fs::write(&source, b"source").expect("create source");

    let short_option = Command::new(env!("CARGO_BIN_EXE_link"))
        .arg0("link")
        .env("LC_ALL", "C")
        .env("POSIXLY_CORRECT", "1")
        .args([
            source.as_os_str(),
            std::ffi::OsStr::new("-h"),
            target.as_os_str(),
        ])
        .output()
        .expect("run link with a short option after an operand");
    assert_eq!(short_option.status.code(), Some(1));
    assert!(short_option.stdout.is_empty());
    assert_eq!(
        short_option.stderr,
        format!(
            "link: extra operand '{}'\nTry 'link --help' for more information.\n",
            target.display()
        )
        .as_bytes()
    );

    let help_operand = Command::new(env!("CARGO_BIN_EXE_link"))
        .arg0("link")
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .env("POSIXLY_CORRECT", "1")
        .arg(&source)
        .arg("--help")
        .output()
        .expect("run link with --help as an operand");
    assert_eq!(help_operand.status.code(), Some(0));
    assert!(help_operand.stdout.is_empty());
    assert!(help_operand.stderr.is_empty());
    assert_eq!(
        std::fs::metadata(&source)
            .expect("read source metadata")
            .ino(),
        std::fs::metadata(temp_dir.path().join("--help"))
            .expect("read target metadata")
            .ino()
    );
}
