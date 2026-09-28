use std::ffi::OsString;
use std::fs::{self, File};
use std::io;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::symlink;
use std::os::unix::process::CommandExt;
use std::process::Command;

use tempfile::TempDir;

#[test]
fn parents_failure_names_directory_in_error() {
    let tempdir = TempDir::new().unwrap();
    fs::create_dir_all(tempdir.path().join("a/b")).unwrap();
    File::create(tempdir.path().join("a/keep")).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rmdir"))
        .arg0("rmdir")
        .args(["-p", "a/b"])
        .current_dir(tempdir.path())
        .env("LC_ALL", "C")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"rmdir: failed to remove directory 'a': Directory not empty\n"
    );
    assert!(!tempdir.path().join("a/b").exists());
    assert!(tempdir.path().join("a").exists());
}

#[test]
fn posixly_correct_stops_option_parsing_at_first_directory() {
    let tempdir = TempDir::new().unwrap();
    fs::create_dir(tempdir.path().join("a")).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rmdir"))
        .arg0("rmdir")
        .args(["a", "-v"])
        .current_dir(tempdir.path())
        .env("LC_ALL", "C")
        .env("POSIXLY_CORRECT", "1")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"rmdir: failed to remove '-v': No such file or directory\n"
    );
    assert!(!tempdir.path().join("a").exists());
}

#[test]
fn non_utf8_directory_error_uses_gnu_shell_quoting() {
    let tempdir = TempDir::new().unwrap();
    let name = OsString::from_vec(b"bad\xff".to_vec());
    File::create(tempdir.path().join(&name)).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rmdir"))
        .arg0("rmdir")
        .arg(&name)
        .current_dir(tempdir.path())
        .env("LC_ALL", "C")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"rmdir: failed to remove 'bad'$'\\377': Not a directory\n"
    );
}

#[test]
fn directory_symlink_with_multiple_trailing_slashes_is_not_followed() {
    let tempdir = TempDir::new().unwrap();
    fs::create_dir(tempdir.path().join("target")).unwrap();
    symlink("target", tempdir.path().join("link")).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rmdir"))
        .arg0("rmdir")
        .arg("link//")
        .current_dir(tempdir.path())
        .env("LC_ALL", "C")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"rmdir: failed to remove 'link//': Symbolic link not followed\n"
    );
    assert!(tempdir.path().join("link").is_symlink());
    assert!(tempdir.path().join("target").is_dir());
}

#[test]
fn verbose_removes_directory_before_reporting_closed_stdout() {
    let tempdir = TempDir::new().unwrap();
    fs::create_dir(tempdir.path().join("directory")).unwrap();

    let mut command = Command::new(env!("CARGO_BIN_EXE_rmdir"));
    command
        .arg0("rmdir")
        .args(["-v", "directory"])
        .current_dir(tempdir.path())
        .env("LC_ALL", "C");
    unsafe {
        command.pre_exec(|| {
            if libc::close(libc::STDOUT_FILENO) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let output = command.output().unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"rmdir: write error: Bad file descriptor\n");
    assert!(!tempdir.path().join("directory").exists());
}

#[test]
fn parents_preserve_dot_component_in_parent_path() {
    let tempdir = TempDir::new().unwrap();
    fs::create_dir_all(tempdir.path().join("a/b")).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rmdir"))
        .arg0("rmdir")
        .args(["-p", "a/./b"])
        .current_dir(tempdir.path())
        .env("LC_ALL", "C")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"rmdir: failed to remove directory 'a/.': Invalid argument\n"
    );
    assert!(!tempdir.path().join("a/b").exists());
    assert!(tempdir.path().join("a").exists());
}
