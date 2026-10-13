#[cfg(target_os = "linux")]
#[test]
fn non_utf8_digits_uses_the_gnu_invalid_number_diagnostic() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::process::Command;

    let output = Command::new("bash")
        .args(["-c", "exec -a csplit \"$0\" \"$@\""])
        .arg(env!("CARGO_BIN_EXE_csplit"))
        .args(["-n"])
        .arg(OsString::from_vec(vec![0xff]))
        .args(["-", "1"])
        .env("LC_ALL", "C")
        .output()
        .expect("run csplit");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "{output:?}");
    assert_eq!(output.stderr, b"csplit: invalid number: '\\377'\n");
}
