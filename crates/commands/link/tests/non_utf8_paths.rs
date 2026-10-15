#[cfg(unix)]
#[test]
fn non_utf8_paths_use_gnu_diagnostics() {
    use std::ffi::OsString;
    use std::os::unix::{ffi::OsStringExt, process::CommandExt};
    use std::process::Command;

    let invalid_name = OsString::from_vec(vec![0xff]);

    let missing_target = Command::new(env!("CARGO_BIN_EXE_link"))
        .arg0("link")
        .env("LC_ALL", "C")
        .arg(&invalid_name)
        .output()
        .expect("run link without a target");
    assert_eq!(missing_target.status.code(), Some(1));
    assert!(missing_target.stdout.is_empty());
    assert_eq!(
        missing_target.stderr,
        b"link: missing operand after '\\377'\nTry 'link --help' for more information.\n"
    );

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    let missing_source = Command::new(env!("CARGO_BIN_EXE_link"))
        .arg0("link")
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .arg(&invalid_name)
        .arg("target")
        .output()
        .expect("run link with a missing non-UTF-8 source");
    assert_eq!(missing_source.status.code(), Some(1));
    assert!(missing_source.stdout.is_empty());
    assert_eq!(
        missing_source.stderr,
        b"link: cannot create link 'target' to ''$'\\377': No such file or directory\n"
    );
}
