use std::fs::File;
use std::io;
use std::process::{Command, Output, Stdio};

#[cfg(unix)]
use std::os::unix::process::{CommandExt, ExitStatusExt};

#[cfg(target_os = "linux")]
fn run_with_closed_stdout_pipe(sigpipe_handler: ctcore::libc::sighandler_t) -> Output {
    let mut pipe_fds = [0; 2];
    assert_eq!(unsafe { ctcore::libc::pipe(pipe_fds.as_mut_ptr()) }, 0);
    let read_end = pipe_fds[0];
    let write_end = pipe_fds[1];
    assert_eq!(unsafe { ctcore::libc::close(read_end) }, 0);

    let mut command = Command::new(env!("CARGO_BIN_EXE_pwd"));
    command
        .arg0("pwd")
        .env("LC_ALL", "C")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        command.pre_exec(move || {
            if ctcore::libc::signal(ctcore::libc::SIGPIPE, sigpipe_handler) == ctcore::libc::SIG_ERR
            {
                return Err(io::Error::last_os_error());
            }
            if ctcore::libc::dup2(write_end, ctcore::libc::STDOUT_FILENO) == -1 {
                return Err(io::Error::last_os_error());
            }
            if write_end != ctcore::libc::STDOUT_FILENO {
                ctcore::libc::close(write_end);
            }
            Ok(())
        });
    }

    let child = command.spawn().expect("spawn pwd");
    assert_eq!(unsafe { ctcore::libc::close(write_end) }, 0);
    child.wait_with_output().expect("wait for pwd")
}

#[test]
fn non_option_arguments_emit_gnu_warning() {
    let output = Command::new(env!("CARGO_BIN_EXE_pwd"))
        .arg0("pwd")
        .arg("ignored")
        .env("LC_ALL", "C")
        .output()
        .expect("run pwd");

    assert_eq!(output.status.code(), Some(0));
    assert!(!output.stdout.is_empty());
    assert_eq!(output.stderr, b"pwd: ignoring non-option arguments\n");
}

#[cfg(target_os = "linux")]
#[test]
fn closed_stdout_pipe_with_default_sigpipe_terminates_process() {
    let output = run_with_closed_stdout_pipe(ctcore::libc::SIG_DFL);

    assert_eq!(output.status.signal(), Some(ctcore::libc::SIGPIPE));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[cfg(target_os = "linux")]
#[test]
fn closed_stdout_pipe_with_ignored_sigpipe_reports_write_error() {
    let output = run_with_closed_stdout_pipe(ctcore::libc::SIG_IGN);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"pwd: write error: Broken pipe\n");
}

#[cfg(target_os = "linux")]
#[test]
fn closed_stdout_reports_bad_file_descriptor() {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pwd"));
    command
        .arg0("pwd")
        .env("LC_ALL", "C")
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    unsafe {
        command.pre_exec(|| {
            if ctcore::libc::close(ctcore::libc::STDOUT_FILENO) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let output = command.output().expect("run pwd");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"pwd: write error: Bad file descriptor\n");
}

#[test]
fn full_stdout_reports_write_error() {
    let full = File::options()
        .write(true)
        .open("/dev/full")
        .expect("open /dev/full");
    let output = Command::new(env!("CARGO_BIN_EXE_pwd"))
        .arg0("pwd")
        .env("LC_ALL", "C")
        .stdout(Stdio::from(full))
        .stderr(Stdio::piped())
        .output()
        .expect("run pwd");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"pwd: write error: No space left on device\n"
    );
}
