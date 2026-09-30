use std::io;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, Output, Stdio};

#[cfg(target_os = "linux")]
fn run_with_closed_stdout_pipe(sigpipe_handler: ctcore::libc::sighandler_t) -> Output {
    let mut pipe_fds = [0; 2];
    assert_eq!(unsafe { ctcore::libc::pipe(pipe_fds.as_mut_ptr()) }, 0);
    let read_end = pipe_fds[0];
    let write_end = pipe_fds[1];
    assert_eq!(unsafe { ctcore::libc::close(read_end) }, 0);

    let mut command = Command::new(env!("CARGO_BIN_EXE_printenv"));
    command
        .arg0("printenv")
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

    let child = command.spawn().expect("spawn printenv");
    assert_eq!(unsafe { ctcore::libc::close(write_end) }, 0);
    child.wait_with_output().expect("wait for printenv")
}

#[cfg(target_os = "linux")]
#[test]
fn closed_stdout_pipe_uses_default_sigpipe() {
    let output = run_with_closed_stdout_pipe(ctcore::libc::SIG_DFL);

    assert_eq!(output.status.signal(), Some(ctcore::libc::SIGPIPE));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[cfg(target_os = "linux")]
#[test]
fn closed_stdout_pipe_with_ignored_sigpipe_reports_write_error() {
    let output = run_with_closed_stdout_pipe(ctcore::libc::SIG_IGN);

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"printenv: write error: Broken pipe\n");
}

#[cfg(target_os = "linux")]
#[test]
fn closed_stdout_reports_bad_file_descriptor_after_output() {
    let mut command = Command::new(env!("CARGO_BIN_EXE_printenv"));
    command
        .arg0("printenv")
        .arg("PRINTENV_CLOSED")
        .env("PRINTENV_CLOSED", "value")
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

    let output = command.output().expect("run printenv");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"printenv: write error: Bad file descriptor\n"
    );
}
