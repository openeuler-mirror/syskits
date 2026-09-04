#[cfg(all(unix, feature = "nohup"))]
use std::fs::File;
#[cfg(all(unix, feature = "nohup"))]
use std::io::Read;
#[cfg(all(unix, feature = "nohup"))]
use std::process::{Command, Stdio};

#[cfg(all(unix, feature = "nohup"))]
#[test]
fn nohup_reports_tty_stderr_redirection() {
    let pty = nix::pty::openpty(None, None).expect("open pty");
    let mut master = File::from(pty.master);

    let status = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["nohup", "true"])
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(pty.slave))
        .status()
        .expect("run syskits nohup");

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

    assert!(status.success());
    assert_eq!(
        String::from_utf8(terminal_output)
            .expect("utf8 terminal output")
            .replace("\r\n", "\n"),
        "nohup: redirecting stderr to stdout\n"
    );
}
