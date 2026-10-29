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

#[cfg(target_os = "linux")]
#[test]
fn copy_contents_updates_source_fifo_access_time() {
    use filetime::{FileTime, set_file_times};
    use std::ffi::CString;
    use std::io::Write;
    use std::os::unix::ffi::OsStrExt;
    use std::process::Stdio;

    let directory = tempfile::tempdir().unwrap();
    let source_directory = directory.path().join("fifodir");
    let source_fifo = source_directory.join("pipe");
    fs::create_dir(&source_directory).unwrap();

    let fifo_name = CString::new(source_fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo_name.as_ptr(), 0o600) }, 0);

    let old_time = FileTime::from_unix_time(1_577_836_800, 0);
    set_file_times(&source_fifo, old_time, old_time).unwrap();

    let writer_fifo = source_fifo.clone();
    let writer = std::thread::spawn(move || {
        let mut fifo = fs::OpenOptions::new()
            .write(true)
            .open(writer_fifo)
            .unwrap();
        fifo.write_all(b"fifo payload\n").unwrap();
    });

    let output = Command::new(env!("CARGO_BIN_EXE_cp"))
        .arg0("cp")
        .env("LC_ALL", "C")
        .current_dir(directory.path())
        .args(["-R", "--copy-contents", "fifodir", "outdir"])
        .stdin(Stdio::null())
        .output()
        .expect("run cp");
    writer.join().unwrap();

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert_eq!(
        fs::read(directory.path().join("outdir/pipe")).unwrap(),
        b"fifo payload\n"
    );

    let source_atime = FileTime::from_last_access_time(&fs::metadata(source_fifo).unwrap());
    assert!(
        source_atime > old_time,
        "source FIFO access time was not updated"
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
