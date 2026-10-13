#[cfg(unix)]
#[test]
fn dollar_anchor_keeps_carriage_return_in_the_search_text() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp_dir.path().join("input"), b"a\r\nb\r\nc\r\n").expect("write input");

    let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
        .arg0("csplit")
        .current_dir(temp_dir.path())
        .arg("input")
        .arg("/b$/")
        .output()
        .expect("run csplit");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stdout, b"9\n");
    assert_eq!(
        output.stderr,
        format!(
            "csplit: {}: match not found\n",
            ctcore::ct_display::locale_quote("/b$/")
        )
        .as_bytes()
    );
    assert!(!temp_dir.path().join("xx00").exists());
}
