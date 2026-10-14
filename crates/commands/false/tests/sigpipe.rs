#![cfg(target_os = "linux")]

use std::fs::File;
use std::os::fd::FromRawFd;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, Output, Stdio};

fn run_with_closed_pipe(binary: &str, argument: &str, ignore_sigpipe: bool) -> Output {
    let mut pipe_fds = [0; 2];
    assert_eq!(unsafe { ctcore::libc::pipe(pipe_fds.as_mut_ptr()) }, 0);
    assert_eq!(unsafe { ctcore::libc::close(pipe_fds[0]) }, 0);
    let pipe_writer = unsafe { File::from_raw_fd(pipe_fds[1]) };

    let mut command = Command::new(binary);
    command
        .arg0("false")
        .arg(argument)
        .stdin(Stdio::null())
        .stdout(Stdio::from(pipe_writer))
        .env("LC_ALL", "C");
    unsafe {
        command.pre_exec(move || {
            let disposition = if ignore_sigpipe {
                ctcore::libc::SIG_IGN
            } else {
                ctcore::libc::SIG_DFL
            };
            if ctcore::libc::signal(ctcore::libc::SIGPIPE, disposition)
                == ctcore::libc::SIG_ERR
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command.output().expect("run false with a closed pipe")
}

#[test]
fn closed_pipe_behavior_matches_gnu() {
    for ignore_sigpipe in [false, true] {
        for argument in ["--help", "--version"] {
            let gnu = run_with_closed_pipe("/usr/bin/false", argument, ignore_sigpipe);
            let syskits =
                run_with_closed_pipe(env!("CARGO_BIN_EXE_false"), argument, ignore_sigpipe);

            assert_eq!(syskits.status.code(), gnu.status.code());
            assert_eq!(syskits.status.signal(), gnu.status.signal());
            assert_eq!(syskits.stdout, gnu.stdout);
            assert_eq!(syskits.stderr, gnu.stderr);
        }
    }
}
