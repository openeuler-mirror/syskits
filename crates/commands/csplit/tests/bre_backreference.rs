#[cfg(unix)]
#[test]
fn bre_backreferences_match_gnu_csplit() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp_dir.path().join("input"), b"aa\nbb\nab\n").expect("write input");

    let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
        .arg0("csplit")
        .current_dir(temp_dir.path())
        .args(["-f", "split-", "input", "/^\\(.\\)\\1$/"])
        .output()
        .expect("run csplit with BRE backreference");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"0\n9\n");
    assert!(output.stderr.is_empty());
    assert_eq!(
        std::fs::read(temp_dir.path().join("split-00")).expect("read first split"),
        b""
    );
    assert_eq!(
        std::fs::read(temp_dir.path().join("split-01")).expect("read second split"),
        b"aa\nbb\nab\n"
    );
}
