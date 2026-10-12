#[cfg(unix)]
#[test]
fn chroot_rejects_non_gnu_short_options() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    for (argument, option) in [
        ("-h", "h"),
        ("-V", "V"),
        ("-u", "u"),
        ("-g", "g"),
        ("-G", "G"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_chroot"))
            .env("LC_ALL", "C")
            .arg0("chroot")
            .arg(argument)
            .output()
            .expect("run chroot");

        assert_eq!(output.status.code(), Some(125));
        assert_eq!(
            String::from_utf8(output.stderr).expect("stderr is UTF-8"),
            format!(
                "chroot: invalid option -- '{option}'\nTry 'chroot --help' for more information.\n"
            )
        );
    }
}

#[cfg(unix)]
#[test]
fn chroot_long_option_prefixes_use_gnu_targets() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    if unsafe { libc::geteuid() } != 0 {
        return;
    }

    let group_output = Command::new(env!("CARGO_BIN_EXE_chroot"))
        .env("LC_ALL", "C")
        .arg0("chroot")
        .args(["--group=65534", "/", "/usr/bin/id", "-g"])
        .output()
        .expect("run chroot with --group prefix");
    assert_eq!(group_output.status.code(), Some(0));
    assert_eq!(group_output.stdout, b"0\n");

    let user_output = Command::new(env!("CARGO_BIN_EXE_chroot"))
        .env("LC_ALL", "C")
        .arg0("chroot")
        .args(["--user=65534", "/", "/usr/bin/id", "-u"])
        .output()
        .expect("run chroot with --user prefix");
    assert_eq!(user_output.status.code(), Some(0));
    assert_eq!(user_output.stdout, b"65534\n");
}
