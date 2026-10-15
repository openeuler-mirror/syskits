#![cfg(target_os = "linux")]

use std::process::Command;

#[test]
fn help_and_version_report_bad_file_descriptor_when_stdout_is_closed() {
    const SCRIPT: &str = r#"
exec 3>&1
exec 1>&-
bash -c 'exec -a link "$0" "$1"' "$1" "$2"
printf '%s' "$?" >&3
"#;

    for option in ["--help", "--version"] {
        let output = Command::new("bash")
            .args(["-c", SCRIPT, "bash", env!("CARGO_BIN_EXE_link"), option])
            .env("LC_ALL", "C")
            .output()
            .expect("run link with stdout closed before startup");

        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, b"1", "{option}");
        assert_eq!(
            output.stderr, b"link: write error: Bad file descriptor\n",
            "{option}"
        );
    }
}
