#[cfg(unix)]
#[test]
fn non_utf8_template_and_suffix_are_preserved() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let template = OsString::from_vec(b"mktemp-\xff.XXX".to_vec());
    let output = Command::new(env!("CARGO_BIN_EXE_mktemp"))
        .arg0("mktemp")
        .arg("-u")
        .arg(template)
        .output()
        .expect("run mktemp with non-UTF-8 template");
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert!(output.stdout.starts_with(b"mktemp-\xff."));
    assert_eq!(output.stdout.last(), Some(&b'\n'));

    let mut suffix = OsString::from("--suffix=");
    suffix.push(OsString::from_vec(b".\xff".to_vec()));
    let output = Command::new(env!("CARGO_BIN_EXE_mktemp"))
        .arg0("mktemp")
        .args(["-u", "mktemp-suffix.XXX"])
        .arg(suffix)
        .output()
        .expect("run mktemp with non-UTF-8 suffix");
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert!(output.stdout.starts_with(b"mktemp-suffix."));
    assert!(output.stdout.ends_with(b".\xff\n"));

    let directory = tempfile::tempdir().expect("create temporary directory");
    let template = OsString::from_vec(b"mktemp-created-\xff.XXX".to_vec());
    let output = Command::new(env!("CARGO_BIN_EXE_mktemp"))
        .arg0("mktemp")
        .arg("-d")
        .arg("-p")
        .arg(directory.path())
        .arg(template)
        .output()
        .expect("create a non-UTF-8 temporary directory");
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout.last(), Some(&b'\n'));

    let path = std::path::PathBuf::from(OsString::from_vec(
        output.stdout[..output.stdout.len() - 1].to_vec(),
    ));
    assert!(path.is_dir());
    std::fs::remove_dir(path).expect("remove created temporary directory");
}
