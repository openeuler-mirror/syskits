#[cfg(unix)]
#[test]
fn help_and_version_identify_syskits() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let help = Command::new(env!("CARGO_BIN_EXE_mkdir"))
        .arg0("mkdir")
        .env("LC_ALL", "C")
        .arg("--help")
        .output()
        .expect("run mkdir --help");
    assert_eq!(help.status.code(), Some(0));
    assert!(help.stderr.is_empty());
    assert!(help.stdout.windows(6).any(|part| part == b"--help"));
    assert!(help.stdout.windows(9).any(|part| part == b"--version"));
    assert!(
        help.stdout
            .windows(19)
            .any(|part| part == b"syskits online help")
    );
    assert!(!help.stdout.windows(3).any(|part| part == b"GNU"));

    let version = Command::new(env!("CARGO_BIN_EXE_mkdir"))
        .arg0("mkdir")
        .env("LC_ALL", "C")
        .arg("--version")
        .output()
        .expect("run mkdir --version");
    assert_eq!(version.status.code(), Some(0));
    assert!(version.stderr.is_empty());
    assert!(
        version
            .stdout
            .windows(b"mkdir (syskits)".len())
            .any(|part| part == b"mkdir (syskits)")
    );
    assert!(
        version
            .stdout
            .windows(12)
            .any(|part| part == b"Mulan PSL v2")
    );
    assert!(!version.stdout.windows(3).any(|part| part == b"GNU"));
}
