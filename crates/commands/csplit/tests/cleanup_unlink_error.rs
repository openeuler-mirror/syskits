#[cfg(target_os = "linux")]
#[test]
fn failed_error_cleanup_reports_each_unremovable_output() {
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    let script = r#"
mkfifo input
bash -c 'exec -a csplit "$1" -q input 2 "/x/"' csplit-child "$1" >stdout 2>stderr &
pid=$!
exec 3>input
printf 'a\nb\n' >&3

attempts=0
while test ! -e xx01 && test "$attempts" -lt 500; do
    sleep 0.01
    attempts=$((attempts + 1))
done
if test ! -e xx01; then
    exec 3>&-
    kill "$pid"
    wait "$pid"
    exit 99
fi

for file in xx00 xx01; do
    unlink "$file"
    mkdir "$file"
done
exec 3>&-
wait "$pid"
"#;

    let output = Command::new("/usr/bin/timeout")
        .args(["5", "bash", "-c", script, "csplit-test"])
        .arg(env!("CARGO_BIN_EXE_csplit"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .output()
        .expect("run csplit race test");

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        std::fs::read(temp_dir.path().join("stdout")).expect("read stdout"),
        b""
    );
    assert_eq!(
        std::fs::read(temp_dir.path().join("stderr")).expect("read stderr"),
        b"csplit: '/x/': match not found\n\
csplit: xx01: Is a directory\n\
csplit: xx00: Is a directory\n"
    );
    assert!(temp_dir.path().join("xx00").is_dir());
    assert!(temp_dir.path().join("xx01").is_dir());
}
