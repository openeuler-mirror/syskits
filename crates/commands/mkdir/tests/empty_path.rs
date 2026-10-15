#[cfg(unix)]
#[test]
fn empty_directory_operand_reports_enoent() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let output = Command::new(env!("CARGO_BIN_EXE_mkdir"))
        .arg0("mkdir")
        .env("LC_ALL", "C")
        .arg("")
        .output()
        .expect("run mkdir with an empty directory operand");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"mkdir: cannot create directory '': No such file or directory\n"
    );
}
