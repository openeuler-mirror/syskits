#[cfg(unix)]
#[test]
fn suffix_separator_is_reported_before_template_validation() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    for args in [
        &["-t", "mktemp-invalid/pathXXX/suffix"][..],
        &["mktemp-invalid/pathX/suffix"][..],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_mktemp"))
            .arg0("mktemp")
            .env("LC_ALL", "C")
            .args(args)
            .output()
            .expect("run mktemp with invalid suffix");

        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            "mktemp: invalid suffix '/suffix', contains directory separator\n"
        );
    }
}
