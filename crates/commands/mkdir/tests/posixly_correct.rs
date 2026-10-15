#[cfg(unix)]
#[test]
fn posixly_correct_stops_option_parsing_at_first_operand() {
    use std::os::unix::{fs::PermissionsExt, process::CommandExt};
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    let output = Command::new(env!("CARGO_BIN_EXE_mkdir"))
        .arg0("mkdir")
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .env("POSIXLY_CORRECT", "1")
        .args(["-m", "700", "directory", "-p"])
        .output()
        .expect("run mkdir with POSIXLY_CORRECT");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert!(temp_dir.path().join("directory").is_dir());
    assert_eq!(
        std::fs::metadata(temp_dir.path().join("directory"))
            .expect("read directory metadata")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert!(temp_dir.path().join("-p").is_dir());
}
