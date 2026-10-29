#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::process::{Command, Output};

fn run_cp(current_dir: &std::path::Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cp"))
        .arg0("cp")
        .env("LC_ALL", "C")
        .current_dir(current_dir)
        .args(arguments)
        .output()
        .expect("run cp")
}

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

#[test]
fn copying_directory_into_itself_reports_effective_destination() {
    for arguments in [
        &["-R", "dir", "dir"][..],
        &["-rl", "dir", "dir"][..],
        &["-rl", "a", "dir", "dir"][..],
    ] {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("a")).unwrap();
        fs::create_dir(directory.path().join("dir")).unwrap();

        let output = run_cp(directory.path(), arguments);

        assert_eq!(output.status.code(), Some(1), "{arguments:?}");
        assert!(output.stdout.is_empty(), "{arguments:?}");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            "cp: cannot copy a directory, 'dir', into itself, 'dir/dir'\n",
            "{arguments:?}"
        );
        assert!(directory.path().join("dir/dir").is_dir());
    }
}

#[test]
fn copies_directory_with_parents_to_missing_destination_path() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("a/b/c");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir(directory.path().join("d")).unwrap();
    fs::set_permissions(
        directory.path().join("a"),
        fs::Permissions::from_mode(0o721),
    )
    .unwrap();
    fs::set_permissions(
        directory.path().join("a/b"),
        fs::Permissions::from_mode(0o732),
    )
    .unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o743)).unwrap();

    let output = run_cp(directory.path(), &["-a", "--parents", "a/b/c", "d"]);

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    for (path, mode) in [("d/a", 0o721), ("d/a/b", 0o732), ("d/a/b/c", 0o743)] {
        assert_eq!(
            fs::metadata(directory.path().join(path))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            mode,
            "{path}"
        );
    }
}
