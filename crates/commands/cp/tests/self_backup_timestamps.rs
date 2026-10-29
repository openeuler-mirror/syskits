#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::process::Command;

use filetime::{FileTime, set_file_times};

const OLD_ATIME: FileTime = FileTime::from_unix_time(946_684_800, 0);
const OLD_MTIME: FileTime = FileTime::from_unix_time(1_577_836_800, 0);

struct CopyResult {
    status: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    contents: Vec<u8>,
    mode: u32,
    atime: FileTime,
    mtime: FileTime,
}

fn run_self_backup(binary: &str, arguments: &[&str]) -> CopyResult {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("src");
    fs::write(&source, b"source-data\n").unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o640)).unwrap();
    set_file_times(&source, OLD_ATIME, OLD_MTIME).unwrap();

    let output = Command::new(binary)
        .arg0("cp")
        .current_dir(directory.path())
        .env("LC_ALL", "C")
        .args(arguments)
        .args(["src", "src"])
        .output()
        .expect("run cp self-backup");

    let backup = directory.path().join("src.~1~");
    let metadata = fs::metadata(&backup).unwrap();
    CopyResult {
        status: output.status.code(),
        stdout: output.stdout,
        stderr: output.stderr,
        contents: fs::read(backup).unwrap(),
        mode: metadata.permissions().mode() & 0o777,
        atime: FileTime::from_last_access_time(&metadata),
        mtime: FileTime::from_last_modification_time(&metadata),
    }
}

#[test]
fn self_backup_timestamps_match_gnu() {
    for (arguments, preserves_timestamps) in [
        (&["-f", "--backup=numbered"][..], false),
        (&["-i", "-f", "--backup=numbered"][..], false),
        (&["-f", "-i", "--backup=numbered"][..], false),
        (
            &["-f", "--preserve=timestamps", "--backup=numbered"][..],
            true,
        ),
    ] {
        let gnu = run_self_backup("/usr/bin/cp", arguments);
        let syskits = run_self_backup(env!("CARGO_BIN_EXE_cp"), arguments);

        assert_eq!(gnu.status, Some(0), "GNU cp: {arguments:?}");
        assert_eq!(syskits.status, gnu.status, "{arguments:?}");
        assert_eq!(syskits.stdout, gnu.stdout, "{arguments:?}");
        assert_eq!(syskits.stderr, gnu.stderr, "{arguments:?}");
        assert_eq!(syskits.contents, gnu.contents, "{arguments:?}");
        assert_eq!(syskits.mode, gnu.mode, "{arguments:?}");

        assert_eq!(gnu.atime == OLD_ATIME, preserves_timestamps);
        assert_eq!(gnu.mtime == OLD_MTIME, preserves_timestamps);
        assert_eq!(syskits.atime == OLD_ATIME, gnu.atime == OLD_ATIME);
        assert_eq!(syskits.mtime == OLD_MTIME, gnu.mtime == OLD_MTIME);
    }
}
