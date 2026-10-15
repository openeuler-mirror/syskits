#[cfg(all(target_os = "linux", feature = "stty"))]
mod linux_stty_compat {
    use nix::pty::openpty;
    use nix::sys::termios::{SetArg, SpecialCharacterIndices, tcgetattr, tcsetattr};
    use std::fs::File;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::process::{Command, Stdio};

    fn dup_as_file<Fd: AsRawFd>(fd: &Fd) -> File {
        let raw = unsafe { nix::libc::dup(fd.as_raw_fd()) };
        assert!(raw >= 0, "dup failed");
        unsafe { File::from_raw_fd(raw) }
    }

    #[test]
    fn stty_erase_accepts_octal_control_character() {
        let pty = openpty(None, None).expect("open pty");
        let original = tcgetattr(&pty.slave).expect("read original termios");

        let mut initial = original.clone();
        initial.control_chars[SpecialCharacterIndices::VERASE as usize] = 3;
        tcsetattr(&pty.slave, SetArg::TCSANOW, &initial).expect("set initial erase");

        let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .env("TZ", "UTC0")
            .args(["stty", "erase", "0177"])
            .stdin(Stdio::from(dup_as_file(&pty.slave)))
            .output()
            .expect("run syskits stty");

        let updated = tcgetattr(&pty.slave).expect("read updated termios");
        tcsetattr(&pty.slave, SetArg::TCSANOW, &original).expect("restore original termios");

        assert_eq!(
            output.status.code(),
            Some(0),
            "stdout: {}, stderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"");
        assert_eq!(output.stderr, b"");
        assert_eq!(
            updated.control_chars[SpecialCharacterIndices::VERASE as usize],
            127
        );
    }
}
