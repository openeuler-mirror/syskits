use std::fs::File;
use std::io;
use std::process::{Command, Output, Stdio};

#[cfg(unix)]
use std::os::unix::process::{CommandExt, ExitStatusExt};

#[cfg(target_os = "linux")]
unsafe fn force_getcwd_failure_with_low_fd_limit() -> io::Result<()> {
    let mut filter = [
        ctcore::libc::sock_filter {
            code: (ctcore::libc::BPF_LD | ctcore::libc::BPF_W | ctcore::libc::BPF_ABS) as u16,
            jt: 0,
            jf: 0,
            k: 0,
        },
        ctcore::libc::sock_filter {
            code: (ctcore::libc::BPF_JMP | ctcore::libc::BPF_JEQ | ctcore::libc::BPF_K) as u16,
            jt: 0,
            jf: 1,
            k: ctcore::libc::SYS_getcwd as u32,
        },
        ctcore::libc::sock_filter {
            code: (ctcore::libc::BPF_RET | ctcore::libc::BPF_K) as u16,
            jt: 0,
            jf: 0,
            k: ctcore::libc::SECCOMP_RET_ERRNO | ctcore::libc::ENOENT as u32,
        },
        ctcore::libc::sock_filter {
            code: (ctcore::libc::BPF_RET | ctcore::libc::BPF_K) as u16,
            jt: 0,
            jf: 0,
            k: ctcore::libc::SECCOMP_RET_ALLOW,
        },
    ];
    let filter_program = ctcore::libc::sock_fprog {
        len: filter.len() as u16,
        filter: filter.as_mut_ptr(),
    };
    let limit = ctcore::libc::rlimit {
        rlim_cur: 4,
        rlim_max: 4,
    };

    if unsafe { ctcore::libc::prctl(ctcore::libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe {
        ctcore::libc::prctl(
            ctcore::libc::PR_SET_SECCOMP,
            ctcore::libc::SECCOMP_MODE_FILTER,
            &filter_program,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    if unsafe { ctcore::libc::setrlimit(ctcore::libc::RLIMIT_NOFILE, &limit) } != 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(())
}

#[cfg(unix)]
fn run_in_deleted_working_directory() -> Output {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let tempdir = tempfile::tempdir().expect("create temporary directory");
    let path = CString::new(tempdir.path().as_os_str().as_bytes()).expect("path has no NUL byte");

    let mut command = Command::new(env!("CARGO_BIN_EXE_pwd"));
    command
        .arg0("pwd")
        .env("LC_ALL", "C")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        command.pre_exec(move || {
            if ctcore::libc::chdir(path.as_ptr()) != 0 {
                return Err(io::Error::last_os_error());
            }
            if ctcore::libc::rmdir(path.as_ptr()) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }

    command.output().expect("run pwd")
}

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

#[test]
fn option_errors_use_gnu_diagnostics() {
    let cases = [
        (
            vec!["-Z"],
            b"pwd: invalid option -- 'Z'\nTry 'pwd --help' for more information.\n".as_slice(),
        ),
        (
            vec!["-LPx"],
            b"pwd: invalid option -- 'x'\nTry 'pwd --help' for more information.\n".as_slice(),
        ),
        (
            vec!["--invalid"],
            b"pwd: unrecognized option '--invalid'\nTry 'pwd --help' for more information.\n"
                .as_slice(),
        ),
        (
            vec!["--lo=x"],
            b"pwd: option '--logical' doesn't allow an argument\nTry 'pwd --help' for more information.\n"
                .as_slice(),
        ),
    ];

    for (args, expected_stderr) in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_pwd"))
            .arg0("pwd")
            .args(args)
            .env("LC_ALL", "C")
            .output()
            .expect("run pwd");

        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr, expected_stderr);
    }
}

#[cfg(unix)]
#[test]
fn invalid_non_utf8_short_option_is_preserved_in_diagnostic() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let output = Command::new(env!("CARGO_BIN_EXE_pwd"))
        .arg0("pwd")
        .arg(OsString::from_vec(vec![b'-', 0xff]))
        .env("LC_ALL", "C")
        .output()
        .expect("run pwd");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"pwd: invalid option -- '\xff'\nTry 'pwd --help' for more information.\n"
    );
}

#[cfg(unix)]
#[test]
fn deleted_working_directory_reports_gnu_inode_lookup_error() {
    let output = run_in_deleted_working_directory();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"pwd: couldn't find directory entry in '..' with matching i-node\n"
    );
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

#[cfg(target_os = "linux")]
#[test]
fn getcwd_fallback_succeeds_with_four_file_descriptors() {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pwd"));
    command
        .arg0("pwd")
        .env("LC_ALL", "C")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        command.pre_exec(|| force_getcwd_failure_with_low_fd_limit());
    }

    let output = command.output().expect("run pwd");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.ends_with(b"\n"));
    assert!(output.stderr.is_empty());
}
