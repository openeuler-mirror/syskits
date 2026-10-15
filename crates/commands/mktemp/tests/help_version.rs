#[cfg(unix)]
#[test]
fn help_and_version_identify_syskits() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    for option in ["--help", "--version"] {
        let output = Command::new(env!("CARGO_BIN_EXE_mktemp"))
            .arg0("mktemp")
            .env("LC_ALL", "C")
            .arg(option)
            .output()
            .expect("run mktemp informational option");

        assert_eq!(output.status.code(), Some(0), "{option}");
        assert!(output.stderr.is_empty(), "{option}");
        let stdout = String::from_utf8(output.stdout).expect("informational output is UTF-8");
        assert!(stdout.contains("syskits"), "{option}: {stdout}");
        assert!(!stdout.contains("GNU"), "{option}: {stdout}");
        assert!(!stdout.contains("coreutils"), "{option}: {stdout}");
        assert!(!stdout.contains("mktemp.clap"), "{option}: {stdout}");
    }
}
