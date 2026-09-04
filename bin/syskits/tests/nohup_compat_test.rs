#[cfg(all(unix, feature = "nohup"))]
use std::fs::{self, File};
#[cfg(all(unix, feature = "nohup"))]
use std::io::Read;
#[cfg(all(unix, feature = "nohup"))]
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
#[cfg(all(unix, feature = "nohup"))]
use std::os::unix::fs::PermissionsExt;
#[cfg(all(unix, feature = "nohup"))]
use std::path::Path;
#[cfg(all(unix, feature = "nohup"))]
use std::process::{Command, ExitStatus, Stdio};

#[cfg(all(unix, feature = "nohup"))]
fn read_terminal_output(mut master: File) -> String {
    let mut terminal_output = Vec::new();
    loop {
        let mut buffer = [0; 256];
        match master.read(&mut buffer) {
            Ok(0) => break,
            Ok(size) => terminal_output.extend_from_slice(&buffer[..size]),
            Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
            Err(error) => panic!("read pty: {error}"),
        }
    }

    String::from_utf8(terminal_output)
        .expect("utf8 terminal output")
        .replace("\r\n", "\n")
}

#[cfg(all(unix, feature = "nohup"))]
fn duplicate_fd(fd: RawFd) -> File {
    let duplicated_fd = unsafe { libc::dup(fd) };
    assert!(
        duplicated_fd >= 0,
        "dup pty slave: {}",
        std::io::Error::last_os_error()
    );
    unsafe { File::from_raw_fd(duplicated_fd) }
}

#[cfg(all(unix, feature = "nohup"))]
fn run_with_tty_output(current_dir: &Path, home: &Path) -> (ExitStatus, String) {
    let pty = nix::pty::openpty(None, None).expect("open pty");
    let master = File::from(pty.master);
    let stdout = duplicate_fd(pty.slave.as_raw_fd());

    let status = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["nohup", "/bin/sh", "-c", "printf output"])
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("HOME", home)
        .current_dir(current_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(pty.slave))
        .status()
        .expect("run syskits nohup");

    (status, read_terminal_output(master))
}

#[cfg(all(unix, feature = "nohup"))]
fn run_with_tty_stdio(current_dir: &Path) -> (ExitStatus, String) {
    let pty = nix::pty::openpty(None, None).expect("open pty");
    let master = File::from(pty.master);
    let stdout = duplicate_fd(pty.slave.as_raw_fd());
    let stderr = duplicate_fd(pty.slave.as_raw_fd());

    let status = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["nohup", "/bin/sh", "-c", "read x; echo read_rc=$?"])
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .current_dir(current_dir)
        .stdin(Stdio::from(pty.slave))
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .status()
        .expect("run syskits nohup");

    (status, read_terminal_output(master))
}

#[cfg(all(unix, feature = "nohup"))]
#[test]
fn nohup_reports_tty_stderr_redirection() {
    let pty = nix::pty::openpty(None, None).expect("open pty");
    let master = File::from(pty.master);

    let status = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["nohup", "true"])
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(pty.slave))
        .status()
        .expect("run syskits nohup");

    assert!(status.success());
    assert_eq!(
        read_terminal_output(master),
        "nohup: redirecting stderr to stdout\n"
    );
}
