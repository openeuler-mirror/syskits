#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::process::Command;

#[test]
fn copies_slash_dot_contents_to_missing_destination() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("src/EFI/BOOT");
    let destination = directory.path().join("dst");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("mmx64.efi"), b"efi-test\n").unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o750)).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_cp"))
        .arg0("cp")
        .env("LC_ALL", "C")
        .args(["-R", "-L"])
        .arg(source.join("."))
        .arg(&destination)
        .output()
        .expect("run cp");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert_eq!(
        fs::read(destination.join("mmx64.efi")).unwrap(),
        b"efi-test\n"
    );
    assert_eq!(
        fs::metadata(destination).unwrap().permissions().mode() & 0o777,
        0o750
    );
}
