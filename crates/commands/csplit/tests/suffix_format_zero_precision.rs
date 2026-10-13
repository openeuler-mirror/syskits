#[cfg(target_os = "linux")]
#[test]
fn explicit_zero_precision_omits_zero_suffix_digits() {
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    std::fs::write(temp_dir.path().join("input"), b"a\n").expect("write input");

    let output = Command::new("bash")
        .args(["-c", "exec -a csplit \"$0\" \"$@\""])
        .arg(env!("CARGO_BIN_EXE_csplit"))
        .args(["-q", "-f", "out-", "-b", "%01.0d", "input", "/./", "{*}"])
        .current_dir(temp_dir.path())
        .output()
        .expect("run csplit");

    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert!(temp_dir.path().join("out- ").is_file());
    assert_eq!(
        std::fs::read(temp_dir.path().join("out-1")).expect("read trailing split"),
        b"a\n"
    );
    assert!(!temp_dir.path().join("out-0").exists());
}
