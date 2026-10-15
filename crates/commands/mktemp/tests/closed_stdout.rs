#![cfg(target_os = "linux")]

use std::process::Command;

#[test]
fn stdout_closed_before_startup_reports_write_error_and_cleans_up() {
    const SCRIPT: &str = r#"
exec 3>&1
exec 1>&-
bash -c 'exec -a mktemp "$0" "$@"' "$1" "${@:2}"
printf '%s' "$?" >&3
"#;

    for (options, kind) in [(&[][..], "file"), (&["-d"][..], "directory")] {
        let directory = tempfile::tempdir().expect("create temporary directory");
        let template = directory.path().join("mktemp-closed.XXXXXX");
        let output = Command::new("bash")
            .args(["-c", SCRIPT, "bash", env!("CARGO_BIN_EXE_mktemp")])
            .args(options)
            .arg(&template)
            .env("LC_ALL", "C")
            .output()
            .expect("run mktemp with stdout closed before startup");

        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, b"1");
        assert_eq!(
            output.stderr, b"mktemp: write error: Bad file descriptor\n",
            "{kind}"
        );
        assert!(
            directory.path().read_dir().unwrap().next().is_none(),
            "{kind}"
        );
    }
}
