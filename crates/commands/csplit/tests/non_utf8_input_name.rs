#[cfg(target_os = "linux")]
#[test]
fn non_utf8_input_filename_is_processed_as_raw_linux_bytes() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    let input_name = OsString::from_vec(vec![0xff]);
    std::fs::write(temp_dir.path().join(&input_name), b"a\nb\n").expect("write input");

    let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
        .arg(input_name)
        .arg("1")
        .env("LC_ALL", "C")
        .current_dir(temp_dir.path())
        .output()
        .expect("run csplit");

    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"0\n4\n");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(std::fs::read(temp_dir.path().join("xx00")).unwrap(), b"");
    assert_eq!(
        std::fs::read(temp_dir.path().join("xx01")).unwrap(),
        b"a\nb\n"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn non_utf8_input_filename_diagnostic_uses_gnu_locale_quoting() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::process::Command;

    let output = Command::new("bash")
        .args(["-c", "exec -a csplit \"$0\" \"$@\""])
        .arg(env!("CARGO_BIN_EXE_csplit"))
        .arg(OsString::from_vec(vec![0xff]))
        .env("LC_ALL", "C")
        .output()
        .expect("run csplit");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "{output:?}");
    assert_eq!(
        output.stderr,
        b"csplit: missing operand after '\\377'\nTry 'csplit --help' for more information.\n"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn non_utf8_input_open_error_uses_gnu_shell_quoting() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::process::Command;

    let input_name = OsString::from_vec(b"missing\xff".to_vec());
    let output = Command::new("bash")
        .args(["-c", "exec -a csplit \"$0\" \"$@\""])
        .arg(env!("CARGO_BIN_EXE_csplit"))
        .arg(input_name)
        .arg("1")
        .env("LC_ALL", "C")
        .output()
        .expect("run csplit");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "{output:?}");
    assert_eq!(
        output.stderr,
        b"csplit: cannot open 'missing'$'\\377' for reading: No such file or directory\n"
    );
}
