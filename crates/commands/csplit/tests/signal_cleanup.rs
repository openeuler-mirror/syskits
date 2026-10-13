#[cfg(target_os = "linux")]
#[test]
fn terminating_signals_remove_splits_unless_keep_files_is_set() {
    use std::process::Command;

    const SCRIPT: &str = r#"
binary=$1
keep=$2
signal=$3
mkfifo input
if [ "$keep" = yes ]; then
  "$binary" -k input '/^/' '{*}' >/dev/null 2>stderr &
else
  "$binary" input '/^/' '{*}' >/dev/null 2>stderr &
fi
child=$!
(exec 3>input; printf 'a\nb\n' >&3; sleep 1) &
writer=$!
for _ in $(seq 1 100); do
  test -e xx01 && break
  sleep 0.01
done
test -e xx01 || exit 70
kill -"$signal" "$child"
wait "$child"
status=$?
kill -TERM "$writer" 2>/dev/null || true
case "$signal" in
  TERM) expected_status=143 ;;
  PIPE) expected_status=141 ;;
esac
test "$status" -eq "$expected_status" || exit 71
if [ "$keep" = yes ]; then
  test -e xx00 && test -e xx01 && test -e xx02 || exit 72
else
  test ! -e xx00 && test ! -e xx01 && test ! -e xx02 || exit 73
fi
"#;

    for signal in ["TERM", "PIPE"] {
        for keep in ["no", "yes"] {
            let temp_dir = tempfile::tempdir().expect("tempdir");
            let output = Command::new("bash")
                .args([
                    "-c",
                    SCRIPT,
                    "bash",
                    env!("CARGO_BIN_EXE_csplit"),
                    keep,
                    signal,
                ])
                .env("LC_ALL", "C")
                .current_dir(temp_dir.path())
                .output()
                .expect("run interrupted csplit");

            assert!(output.status.success(), "{signal}/{keep}: {output:?}");
            assert!(output.stdout.is_empty(), "{signal}/{keep}");
            assert!(output.stderr.is_empty(), "{signal}/{keep}");
        }
    }
}
