use std::process::Command;

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
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args([
            "env",
            "--block-signal=PIPE",
            "--list-signal-handling",
            "true",
        ])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
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
