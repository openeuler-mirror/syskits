#[cfg(unix)]
#[test]
fn non_ascii_utf8_source_is_escaped_in_c_locale() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    let output = Command::new(env!("CARGO_BIN_EXE_link"))
        .arg0("link")
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args(["中文", "target"])
        .output()
        .expect("run link with a non-ASCII source");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"link: cannot create link 'target' to ''$'\\344\\270\\255\\346\\226\\207': No such file or directory\n"
    );
}
