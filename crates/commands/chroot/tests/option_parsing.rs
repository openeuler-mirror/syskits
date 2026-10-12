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

#[cfg(unix)]
#[test]
fn chroot_stops_option_parsing_at_newroot() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    if unsafe { libc::geteuid() } != 0 {
        return;
    }

    for command in ["-h", "--group=65534", "--userspec=65534", "--skip-chdir"] {
        let output = Command::new(env!("CARGO_BIN_EXE_chroot"))
            .env("LC_ALL", "C")
            .arg0("chroot")
            .args(["/", command])
            .output()
            .expect("run chroot");

        assert_eq!(output.status.code(), Some(127), "command: {command}");
    }
}

#[cfg(unix)]
#[test]
fn chroot_skip_chdir_rejects_unresolvable_newroot_without_panicking() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    for newroot in [".", "/definitely-missing-chroot-root"] {
        let output = Command::new(env!("CARGO_BIN_EXE_chroot"))
            .env("LC_ALL", "C")
            .arg0("chroot")
            .args(["--skip-chdir", newroot, "true"])
            .output()
            .expect("run chroot");

        assert_eq!(output.status.code(), Some(125));
        assert_eq!(
            String::from_utf8(output.stderr).expect("stderr is UTF-8"),
            "chroot: option --skip-chdir only permitted if NEWROOT is old '/'\nTry 'chroot --help' for more information.\n"
        );
    }
}

#[cfg(unix)]
#[test]
fn chroot_reports_kernel_root_directory_errors() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    if unsafe { libc::geteuid() } != 0 {
        return;
    }

    for (newroot, error) in [
        (
            "/definitely-missing-chroot-root",
            "No such file or directory",
        ),
        ("/dev/null", "Not a directory"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_chroot"))
            .env("LC_ALL", "C")
            .arg0("chroot")
            .args([newroot, "true"])
            .output()
            .expect("run chroot");

        assert_eq!(output.status.code(), Some(125));
        assert_eq!(
            String::from_utf8(output.stderr).expect("stderr is UTF-8"),
            format!("chroot: cannot change root directory to '{newroot}': {error}\n")
        );
    }
}

#[cfg(unix)]
#[test]
fn chroot_reports_missing_newroot_like_gnu() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    for arguments in [
        Vec::new(),
        vec!["--groups=0"],
        vec!["--userspec=0"],
        vec!["--skip-chdir"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_chroot"))
            .env("LC_ALL", "C")
            .arg0("chroot")
            .args(arguments)
            .output()
            .expect("run chroot");

        assert_eq!(output.status.code(), Some(125));
        assert_eq!(
            String::from_utf8(output.stderr).expect("stderr is UTF-8"),
            "chroot: missing operand\nTry 'chroot --help' for more information.\n"
        );
    }
}

#[cfg(unix)]
#[test]
fn chroot_reports_unknown_long_options_like_gnu() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    for argument in ["---", "--not-an-option", "--unknown=1"] {
        let output = Command::new(env!("CARGO_BIN_EXE_chroot"))
            .env("LC_ALL", "C")
            .arg0("chroot")
            .args([argument, "/", "true"])
            .output()
            .expect("run chroot");

        assert_eq!(output.status.code(), Some(125));
        assert_eq!(
            String::from_utf8(output.stderr).expect("stderr is UTF-8"),
            format!(
                "chroot: unrecognized option '{argument}'\nTry 'chroot --help' for more information.\n"
            )
        );
    }
}

#[cfg(unix)]
#[test]
fn chroot_reports_long_option_argument_errors_like_gnu() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    for (argument, message) in [
        ("--groups", "option '--groups' requires an argument"),
        ("--userspec", "option '--userspec' requires an argument"),
        ("--help=1", "option '--help' doesn't allow an argument"),
        (
            "--version=1",
            "option '--version' doesn't allow an argument",
        ),
        (
            "--skip-chdir=1",
            "option '--skip-chdir' doesn't allow an argument",
        ),
        (
            "--=1",
            "option '--=1' is ambiguous; possibilities: '--groups' '--userspec' '--skip-chdir' '--help' '--version'",
        ),
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
            format!("chroot: {message}\nTry 'chroot --help' for more information.\n")
        );
    }
}

#[cfg(unix)]
#[test]
fn chroot_parses_supplementary_group_lists_like_gnu() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    if unsafe { libc::geteuid() } != 0 {
        return;
    }

    for (groups, message) in [
        (" ", "invalid group ' '"),
        (",", "invalid group list ','"),
        ("invalid ignored", "invalid group 'invalid ignored'"),
        (" root", "invalid group ' root'"),
    ] {
        let option = format!("--groups={groups}");
        let output = Command::new(env!("CARGO_BIN_EXE_chroot"))
            .env("LC_ALL", "C")
            .arg0("chroot")
            .args([option.as_str(), "/", "true"])
            .output()
            .expect("run chroot");

        assert_eq!(output.status.code(), Some(125));
        assert_eq!(
            String::from_utf8(output.stderr).expect("stderr is UTF-8"),
            format!("chroot: {message}\n")
        );
    }

    let output = Command::new(env!("CARGO_BIN_EXE_chroot"))
        .env("LC_ALL", "C")
        .arg0("chroot")
        .args(["--groups= +0, 65534", "/", "/usr/bin/id", "-G"])
        .output()
        .expect("run chroot");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"0 65534\n");
}

#[cfg(unix)]
#[test]
fn chroot_accepts_gnu_userspec_empty_forms() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    if unsafe { libc::geteuid() } != 0 {
        return;
    }

    for userspec in ["", ":", "::", ":0:"] {
        let option = format!("--userspec={userspec}");
        let output = Command::new(env!("CARGO_BIN_EXE_chroot"))
            .env("LC_ALL", "C")
            .arg0("chroot")
            .args([option.as_str(), "/", "/usr/bin/id", "-u"])
            .output()
            .expect("run chroot");

        assert_eq!(output.status.code(), Some(0), "userspec: {userspec:?}");
        assert_eq!(output.stdout, b"0\n");
    }
}

#[cfg(unix)]
#[test]
fn chroot_uses_explicit_primary_gid_for_supplementary_groups() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    if unsafe { libc::geteuid() } != 0 {
        return;
    }

    let output = Command::new(env!("CARGO_BIN_EXE_chroot"))
        .env("LC_ALL", "C")
        .arg0("chroot")
        .args(["--userspec=0:65534", "/", "/usr/bin/id", "-G"])
        .output()
        .expect("run chroot");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"65534\n");
}

#[cfg(unix)]
#[test]
fn chroot_reports_invalid_userspec_principals_like_gnu() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    if unsafe { libc::geteuid() } != 0 {
        return;
    }

    for (userspec, message) in [
        ("definitely-no-such-user", "invalid user"),
        ("definitely-no-such-user:0", "invalid user"),
        ("0:definitely-no-such-group", "invalid group"),
        ("999999999", "no group specified for unknown uid: 999999999"),
        ("+12342", "no group specified for unknown uid: 12342"),
    ] {
        let option = format!("--userspec={userspec}");
        let output = Command::new(env!("CARGO_BIN_EXE_chroot"))
            .env("LC_ALL", "C")
            .arg0("chroot")
            .args([option.as_str(), "/", "true"])
            .output()
            .expect("run chroot");

        assert_eq!(output.status.code(), Some(125));
        assert_eq!(
            String::from_utf8(output.stderr).expect("stderr is UTF-8"),
            format!("chroot: {message}\n")
        );
    }
}

#[cfg(unix)]
#[test]
fn chroot_rejects_numeric_userspec_with_omitted_login_group() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    if unsafe { libc::geteuid() } != 0 {
        return;
    }

    let output = Command::new(env!("CARGO_BIN_EXE_chroot"))
        .env("LC_ALL", "C")
        .arg0("chroot")
        .args(["--userspec=0::", "/", "true"])
        .output()
        .expect("run chroot");

    assert_eq!(output.status.code(), Some(125));
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr is UTF-8"),
        "chroot: invalid spec\n"
    );
}

#[cfg(unix)]
#[test]
fn chroot_reports_command_execution_errors_without_rust_errno_suffixes() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    if unsafe { libc::geteuid() } != 0 {
        return;
    }

    for (command, status, error) in [
        (
            "definitely-no-such-command",
            127,
            "No such file or directory",
        ),
        (".", 126, "Permission denied"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_chroot"))
            .env("LC_ALL", "C")
            .arg0("chroot")
            .args(["/", command])
            .output()
            .expect("run chroot");

        assert_eq!(output.status.code(), Some(status));
        assert_eq!(
            String::from_utf8(output.stderr).expect("stderr is UTF-8"),
            format!("chroot: failed to run command '{command}': {error}\n")
        );
    }
}

#[cfg(unix)]
#[test]
fn chroot_propagates_signal_exit_status_like_gnu() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    if unsafe { libc::geteuid() } != 0 {
        return;
    }

    for (signal, expected_status) in [("TERM", 143), ("KILL", 137), ("INT", 130)] {
        let output = Command::new(env!("CARGO_BIN_EXE_chroot"))
            .env("LC_ALL", "C")
            .arg0("chroot")
            .args(["/", "/bin/sh", "-c", &format!("kill -{signal} $$")])
            .output()
            .expect("run chroot");

        assert_eq!(output.status.code(), Some(expected_status));
    }
}
