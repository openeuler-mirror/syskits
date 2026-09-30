use std::io;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, Stdio};

#[cfg(target_os = "linux")]
#[test]
fn closed_stdout_pipe_uses_default_sigpipe() {
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
            if ctcore::libc::signal(ctcore::libc::SIGPIPE, ctcore::libc::SIG_DFL)
                == ctcore::libc::SIG_ERR
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
    let output = child.wait_with_output().expect("wait for printenv");

    assert_eq!(output.status.signal(), Some(ctcore::libc::SIGPIPE));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}
