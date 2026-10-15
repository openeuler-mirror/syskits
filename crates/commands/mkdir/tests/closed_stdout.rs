#![cfg(target_os = "linux")]

use std::process::Command;

#[test]
fn stdout_closed_before_startup_reports_bad_file_descriptor() {
    const SCRIPT: &str = r#"
exec 3>&1
exec 1>&-
bash -c 'exec -a mkdir "$0" "$@"' "$1" "${@:2}"
printf '%s' "$?" >&3
"#;

    for arguments in [vec!["--help"], vec!["--version"], vec!["-v", "created"]] {
        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        let output = Command::new("bash")
            .args(["-c", SCRIPT, "bash", env!("CARGO_BIN_EXE_mkdir")])
            .args(&arguments)
            .current_dir(temp_dir.path())
            .env("LC_ALL", "C")
            .output()
            .expect("run mkdir with stdout closed before startup");

        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, b"1", "{arguments:?}");
        assert_eq!(
            output.stderr, b"mkdir: write error: Bad file descriptor\n",
            "{arguments:?}"
        );
        if arguments[0] == "-v" {
            assert!(temp_dir.path().join("created").is_dir());
        }
    }
}
