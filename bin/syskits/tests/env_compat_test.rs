use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(target_os = "linux")]
use std::os::unix::process::ExitStatusExt;
use std::process::Command;

use tempfile::TempDir;

#[cfg(unix)]
fn set_child_sigpipe_disposition(command: &mut Command, disposition: libc::sighandler_t) {
    unsafe {
        command.pre_exec(move || {
            if libc::signal(libc::SIGPIPE, disposition) == libc::SIG_ERR {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[test]
fn env_verbose_without_changes_does_not_dump_input_args() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["env", "-v"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run syskits env -v");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"PATH=/usr/bin:/bin\n");
    assert_eq!(output.stderr, b"");
}

#[cfg(target_os = "linux")]
#[test]
fn env_prints_non_utf8_environment_values_as_raw_bytes() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["env"])
        .env_clear()
        .env("GOOD", OsString::from_vec(b"value\xff".to_vec()))
        .output()
        .expect("run syskits env with a non-UTF-8 environment value");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"GOOD=value\xff\n");
    assert_eq!(output.stderr, b"");
}

#[test]
fn env_debug_reports_clean_environment_and_setenv_steps() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["env", "-i", "--debug", "A=1", "B=2"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run syskits env -i --debug A=1 B=2");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"A=1\nB=2\n");
    assert_eq!(
        output.stderr,
        b"cleaning environ\nsetenv:   A=1\nsetenv:   B=2\n"
    );
}

#[test]
fn env_debug_reports_gnu_execution_diagnostics() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["env", "--debug", "-i", "A=1", "/usr/bin/printf", "ok"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run syskits env --debug -i A=1 /usr/bin/printf ok");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"ok");
    assert_eq!(
        output.stderr,
        b"cleaning environ\nsetenv:   A=1\nexecuting: /usr/bin/printf\n   arg[0]= '/usr/bin/printf'\n   arg[1]= 'ok'\n"
    );
}

#[test]
fn env_debug_reports_chdir_before_execution() {
    let temp_dir = TempDir::new().expect("create working directory");
    let working_directory = temp_dir.path().to_str().expect("UTF-8 working directory");
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args([
            "env",
            "--debug",
            "-i",
            "-C",
            working_directory,
            "A=1",
            "/usr/bin/true",
        ])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run syskits env --debug -C");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"");
    assert_eq!(
        String::from_utf8(output.stderr).expect("UTF-8 stderr"),
        format!(
            "cleaning environ\nsetenv:   A=1\nchdir:    '{working_directory}'\nexecuting: /usr/bin/true\n   arg[0]= '/usr/bin/true'\n"
        )
    );
}

#[test]
fn env_debug_reports_assignment_before_null_command_conflict() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["env", "--debug", "-0", "A=1", "/usr/bin/true"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run syskits env --debug -0 with a command");

    assert_eq!(output.status.code(), Some(125));
    assert_eq!(output.stdout, b"");
    assert_eq!(
        output.stderr,
        b"setenv:   A=1\nenv: cannot specify --null (-0) with command\nTry 'env --help' for more information.\n"
    );
}

#[cfg(unix)]
#[test]
fn env_suggests_split_string_after_whitespace_command_is_not_found() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["env", "true -x"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run syskits env with a whitespace command name");

    assert_eq!(output.status.code(), Some(127));
    assert_eq!(output.stdout, b"");
    assert_eq!(
        output.stderr,
        b"env: 'true -x': No such file or directory\nenv: use -[v]S to pass options in shebang lines\n"
    );
}

#[cfg(unix)]
#[test]
fn env_suggests_split_string_for_whitespace_in_shebang_option() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["env", "-i /usr/bin/true"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run syskits env with a whitespace shebang option");

    assert_eq!(output.status.code(), Some(125));
    assert_eq!(output.stdout, b"");
    assert_eq!(
        output.stderr,
        b"env: invalid option -- ' '\nenv: use -[v]S to pass options in shebang lines\nTry 'env --help' for more information.\n"
    );
}

#[test]
fn env_debug_reports_unset_step_before_execution() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["env", "--debug", "-u", "A", "/usr/bin/true"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("A", "1")
        .output()
        .expect("run syskits env --debug -u A");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"");
    assert_eq!(
        output.stderr,
        b"unset:    A\nexecuting: /usr/bin/true\n   arg[0]= '/usr/bin/true'\n"
    );
}

#[test]
fn env_expands_split_string_in_combined_short_options() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["env", "-iS", "A=1"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run syskits env -iS A=1");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"A=1\n");
    assert_eq!(output.stderr, b"");
}

#[test]
fn env_debug_reports_split_string_expansion() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["env", "-ivS", "A=1"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run syskits env -ivS A=1");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"A=1\n");
    assert_eq!(
        output.stderr,
        b"split -S:  'A=1'\n into:    'A=1'\ncleaning environ\nsetenv:   A=1\n"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn env_list_signal_handling_uses_gnu_poll_signal_name() {
    let mut command = Command::new(env!("CARGO_BIN_EXE_syskits"));
    command
        .args([
            "env",
            "--block-signal=IO",
            "--list-signal-handling",
            "/usr/bin/true",
        ])
        .env_clear()
        .env("PATH", "/usr/bin:/bin");
    set_child_sigpipe_disposition(&mut command, libc::SIG_DFL);

    let output = command
        .output()
        .expect("run syskits env --list-signal-handling for IO");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"");
    assert_eq!(output.stderr, b"POLL       (29): BLOCK\n");
}

#[cfg(unix)]
#[test]
fn env_list_signal_handling_reports_inherited_ignored_sigpipe() {
    let mut command = Command::new(env!("CARGO_BIN_EXE_syskits"));
    command
        .args(["env", "--list-signal-handling", "/usr/bin/true"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin");

    set_child_sigpipe_disposition(&mut command, libc::SIG_IGN);

    let output = command
        .output()
        .expect("run syskits env with inherited ignored SIGPIPE");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"");
    assert_eq!(output.stderr, b"PIPE       (13): IGNORE\n");
}

#[cfg(unix)]
#[test]
fn env_debug_reports_ignored_immutable_signal_failures_for_all_signals() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["env", "--debug", "--ignore-signal", "/usr/bin/true"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run syskits env --debug --ignore-signal");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"");
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    assert!(
        stderr.contains("Reset signal KILL (9) to IGNORE (failure ignored)\n"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("Reset signal STOP (19) to IGNORE (failure ignored)\n"),
        "stderr: {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn env_debug_reports_requested_immutable_signal_mask_changes() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args([
            "env",
            "--debug",
            "--block-signal=KILL,STOP",
            "/usr/bin/true",
        ])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run syskits env --debug --block-signal=KILL,STOP");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"");
    assert_eq!(
        output.stderr,
        b"signal KILL (9) mask set to BLOCK\nsignal STOP (19) mask set to BLOCK\nexecuting: /usr/bin/true\n   arg[0]= '/usr/bin/true'\n"
    );
}

#[cfg(unix)]
#[test]
fn env_debug_reports_signal_handling_before_execution() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args([
            "env",
            "--debug",
            "--ignore-signal=HUP",
            "--block-signal=PIPE",
            "/usr/bin/true",
        ])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run syskits env --debug with signal options");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"");
    assert_eq!(
        output.stderr,
        b"Reset signal HUP (1) to IGNORE\nsignal PIPE (13) mask set to BLOCK\nexecuting: /usr/bin/true\n   arg[0]= '/usr/bin/true'\n"
    );
}

#[cfg(unix)]
#[test]
fn env_default_signal_unblocks_even_when_ignore_signal_overrides_disposition() {
    let syskits = env!("CARGO_BIN_EXE_syskits");
    let output = Command::new(syskits)
        .args([
            "env",
            "--block-signal=HUP",
            syskits,
            "env",
            "--default-signal=HUP",
            "--ignore-signal=HUP",
            "/bin/sh",
            "-c",
            "awk '/SigBlk/ { print $2 }' /proc/self/status",
        ])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run nested syskits env with default and ignore signal options");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"0000000000000000\n");
    assert_eq!(output.stderr, b"");
}

#[test]
fn env_split_string_requires_argument() {
    for args in [["env", "-S"], ["env", "--split-string"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .args(args)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .output()
            .expect("run syskits env split-string without argument");

        assert_eq!(
            output.status.code(),
            Some(125),
            "stdout: {}, stderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("requires an argument"),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn env_split_string_separate_argument_is_split() {
    for args in [
        ["env", "-S", "/usr/bin/printf ok"],
        ["env", "--split-string=/usr/bin/printf ok", ""],
    ] {
        let args = args.iter().copied().filter(|arg| !arg.is_empty());
        let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .args(args)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .output()
            .expect("run syskits env split-string");

        assert!(
            output.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"ok");
        assert_eq!(output.stderr, b"");
    }
}

#[test]
fn env_split_string_rejects_invalid_braced_variable_syntax() {
    for (split_string, fragment) in [
        ("A=${MISSING:default}", "${MISSING:default}"),
        ("A=${}", "${}"),
        ("A=${é}", "${é}"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .args(["env", "-i", "-S", split_string])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .output()
            .expect("run syskits env with invalid -S variable syntax");

        assert_eq!(output.status.code(), Some(125), "{split_string}");
        assert_eq!(output.stdout, b"", "{split_string}");
        assert_eq!(
            output.stderr,
            format!("env: only ${{VARNAME}} expansion is supported, error at: {fragment}\n")
                .as_bytes(),
            "{split_string}"
        );
    }
}

#[test]
fn env_split_string_after_command_is_not_env_option() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["env", "/usr/bin/printf", "-S", "ok"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run syskits env /usr/bin/printf -S ok");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"-S");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("ignoring excess arguments"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn env_exec_replaces_the_env_process() {
    let expected_parent =
        std::fs::read("/proc/self/comm").expect("read integration test process name");
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args([
            "env",
            "/usr/bin/python3",
            "-c",
            "import os; print(open(f'/proc/{os.getppid()}/comm').read(), end='')",
        ])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run syskits env with a child that prints its parent name");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, expected_parent);
    assert_eq!(output.stderr, b"");
}

#[cfg(unix)]
#[test]
fn env_rejects_whitespace_in_signal_list_operand() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["env", "--block-signal=HUP, INT", "/usr/bin/true"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run syskits env with whitespace in a signal list operand");

    assert_eq!(output.status.code(), Some(125));
    assert_eq!(output.stdout, b"");
    assert_eq!(output.stderr, b"env: invalid signal ' INT'\n");
}

#[cfg(target_os = "linux")]
#[test]
fn env_explicit_empty_ignore_signal_argument_is_a_noop() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args([
            "env",
            "--ignore-signal=",
            "/bin/sh",
            "-c",
            "kill -TERM $$; printf survived",
        ])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run syskits env with explicit empty ignore-signal argument");

    assert_eq!(output.status.code(), None);
    assert_eq!(output.status.signal(), Some(libc::SIGTERM));
    assert_eq!(output.stdout, b"");
    assert_eq!(output.stderr, b"");
}

#[cfg(target_os = "linux")]
#[test]
fn env_default_signal_unblocks_a_previously_blocked_signal() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args([
            "env",
            "--block-signal=HUP",
            "--default-signal=HUP",
            "/bin/sh",
            "-c",
            "kill -HUP $$; printf survived",
        ])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run syskits env that defaults and unblocks SIGHUP");

    assert_eq!(output.status.code(), None);
    assert_eq!(output.status.signal(), Some(libc::SIGHUP));
    assert_eq!(output.stdout, b"");
    assert_eq!(output.stderr, b"");
}

#[cfg(target_os = "linux")]
#[test]
fn env_blocks_linux_realtime_signals() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args([
            "env",
            "--block-signal=RTMIN",
            "/bin/sh",
            "-c",
            "kill -RTMIN $$; printf survived",
        ])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run syskits env with a blocked realtime signal");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"survived");
    assert_eq!(output.stderr, b"");
}

#[cfg(target_os = "linux")]
#[test]
fn env_block_signal_without_argument_blocks_linux_realtime_signals() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args([
            "env",
            "--block-signal",
            "/bin/sh",
            "-c",
            "kill -RTMIN $$; printf survived",
        ])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run syskits env blocking all known signals");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"survived");
    assert_eq!(output.stderr, b"");
}

#[cfg(target_os = "linux")]
#[test]
fn env_rejects_explicit_immutable_signal_actions() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["env", "--ignore-signal=KILL", "/usr/bin/true"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run syskits env with an explicit immutable signal action");

    assert_eq!(output.status.code(), Some(125));
    assert_eq!(output.stdout, b"");
    assert_eq!(
        output.stderr,
        b"env: failed to set signal action for signal 9: Invalid argument\n"
    );
}

#[cfg(unix)]
#[test]
fn env_list_signal_handling_omits_internal_handlers() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["env", "--list-signal-handling"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run syskits env --list-signal-handling");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"PATH=/usr/bin:/bin\n");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(stderr, "");
    assert!(
        !stderr.contains("HANDLED"),
        "unexpected internal handler listing: {stderr}"
    );
    assert!(
        !stderr.contains("PIPE       (13): IGNORE"),
        "unexpected internal SIGPIPE listing: {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn env_ignore_signal_without_argument_keeps_child_waitable() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["env", "--ignore-signal", "/usr/bin/true"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run syskits env --ignore-signal /usr/bin/true");

    assert!(
        output.status.success(),
        "stdout: {}, stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"");
    assert_eq!(output.stderr, b"");
}

#[cfg(unix)]
#[test]
fn env_list_signal_handling_reports_explicit_ignore_for_command() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args([
            "env",
            "--ignore-signal=PIPE",
            "--list-signal-handling",
            "true",
        ])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run syskits env --ignore-signal=PIPE --list-signal-handling true");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"");
    assert_eq!(output.stderr, b"PIPE       (13): IGNORE\n");
}

#[cfg(unix)]
#[test]
fn env_list_signal_handling_reports_explicit_block_for_command() {
    let mut command = Command::new(env!("CARGO_BIN_EXE_syskits"));
    command
        .args([
            "env",
            "--block-signal=PIPE",
            "--list-signal-handling",
            "true",
        ])
        .env_clear()
        .env("PATH", "/usr/bin:/bin");
    set_child_sigpipe_disposition(&mut command, libc::SIG_DFL);

    let output = command
        .output()
        .expect("run syskits env --block-signal=PIPE --list-signal-handling true");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"");
    assert_eq!(output.stderr, b"PIPE       (13): BLOCK\n");
}

#[cfg(target_os = "linux")]
#[test]
fn env_list_signal_handling_reports_blocked_realtime_signal() {
    let mut command = Command::new(env!("CARGO_BIN_EXE_syskits"));
    command
        .args([
            "env",
            "--block-signal=RTMIN",
            "--list-signal-handling",
            "/usr/bin/true",
        ])
        .env_clear()
        .env("PATH", "/usr/bin:/bin");
    set_child_sigpipe_disposition(&mut command, libc::SIG_DFL);

    let output = command
        .output()
        .expect("run syskits env listing a blocked realtime signal");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"");
    assert_eq!(
        output.stderr,
        format!("{:<10} ({:2}): BLOCK\n", "RTMIN", libc::SIGRTMIN()).as_bytes()
    );
}
