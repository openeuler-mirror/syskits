#[cfg(unix)]
#[test]
fn missing_operand_uses_gnu_diagnostic() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let output = Command::new(env!("CARGO_BIN_EXE_mkdir"))
        .arg0("mkdir")
        .env("LC_ALL", "C")
        .output()
        .expect("run mkdir without operands");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"mkdir: missing operand\nTry 'mkdir --help' for more information.\n"
    );
}
