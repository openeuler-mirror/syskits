#[cfg(target_os = "linux")]
#[test]
fn eliding_an_externally_removed_empty_split_ignores_enoent() {
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    let script = r#"
mkfifo input
"$1" -q -z input 1 >stdout 2>stderr &
pid=$!
exec 3>input

attempts=0
while test ! -e xx00 && test "$attempts" -lt 500; do
    sleep 0.01
    attempts=$((attempts + 1))
done
if test ! -e xx00; then
    exec 3>&-
    kill "$pid"
    wait "$pid"
    exit 99
fi

unlink xx00
printf 'a\n' >&3
exec 3>&-
wait "$pid"
"#;

    let output = Command::new("timeout")
        .args(["5", "bash", "-c", script, "csplit-test"])
        .arg(env!("CARGO_BIN_EXE_csplit"))
        .current_dir(temp_dir.path())
        .output()
        .expect("run csplit race test");

    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        std::fs::read(temp_dir.path().join("stdout")).expect("read stdout"),
        b""
    );
    assert_eq!(
        std::fs::read(temp_dir.path().join("stderr")).expect("read stderr"),
        b""
    );
    assert_eq!(
        std::fs::read(temp_dir.path().join("xx00")).expect("read final split"),
        b"a\n"
    );
}
