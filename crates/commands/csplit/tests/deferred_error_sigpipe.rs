#[cfg(target_os = "linux")]
#[test]
fn deferred_error_count_reports_a_write_error_when_sigpipe_is_ignored() {
    use std::process::Command;

    const SCRIPT: &str = r#"
binary=$1
mode=$2
mkfifo input output
(exec 3<output; sleep 0.05; exec 3>&-) &
reader=$!
if [ "$mode" = ignored ]; then
  bash -c 'trap "" PIPE; exec -a csplit "$0" input "/z/" "{2}"' "$binary" >output 2>stderr &
else
  bash -c 'exec -a csplit "$0" input "/z/" "{2}"' "$binary" >output 2>stderr &
fi
child=$!
exec 3>input
printf 'a\n' >&3
wait "$reader"
exec 3>&-
wait "$child"
printf '%s' "$?" >status
"#;

    for (mode, expected_status, expected_error) in [
        ("default", "141", "csplit: '/z/': match not found\n"),
        (
            "ignored",
            "1",
            "csplit: '/z/': match not found\ncsplit: write error: Broken pipe\n",
        ),
    ] {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let output = Command::new("bash")
            .args(["-c", SCRIPT, "bash", env!("CARGO_BIN_EXE_csplit"), mode])
            .env("LC_ALL", "C")
            .current_dir(temp_dir.path())
            .output()
            .expect("run csplit with a closed deferred-error output pipe");

        assert!(output.status.success(), "{mode}: {output:?}");
        assert!(output.stdout.is_empty(), "{mode}");
        assert!(output.stderr.is_empty(), "{mode}");
        assert_eq!(
            std::fs::read(temp_dir.path().join("status")).unwrap(),
            expected_status.as_bytes(),
            "{mode}"
        );
        assert_eq!(
            std::fs::read(temp_dir.path().join("stderr")).unwrap(),
            expected_error.as_bytes(),
            "{mode}"
        );
        assert!(
            !std::fs::read_dir(temp_dir.path())
                .expect("read temporary directory")
                .filter_map(Result::ok)
                .any(|entry| entry.file_name().to_string_lossy().starts_with("xx")),
            "{mode}"
        );
    }
}
