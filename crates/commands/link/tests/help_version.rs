#[cfg(unix)]
#[test]
fn help_and_version_identify_syskits() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let help = Command::new(env!("CARGO_BIN_EXE_link"))
        .arg0("link")
        .env("LC_ALL", "C")
        .arg("--help")
        .output()
        .expect("run link --help");
    assert_eq!(help.status.code(), Some(0));
    assert!(help.stderr.is_empty());
    assert!(
        help.stdout
            .windows(b"--help".len())
            .any(|part| part == b"--help")
    );
    assert!(
        help.stdout
            .windows(b"--version".len())
            .any(|part| part == b"--version")
    );
    assert!(
        help.stdout
            .windows(b"syskits online help".len())
            .any(|part| part == b"syskits online help")
    );
    assert!(!help.stdout.windows(3).any(|part| part == b"GNU"));

    let version = Command::new(env!("CARGO_BIN_EXE_link"))
        .arg0("link")
        .env("LC_ALL", "C")
        .arg("--version")
        .output()
        .expect("run link --version");
    assert_eq!(version.status.code(), Some(0));
    assert!(version.stderr.is_empty());
    assert!(
        version
            .stdout
            .windows(b"link (syskits)".len())
            .any(|part| part == b"link (syskits)")
    );
    assert!(
        version
            .stdout
            .windows(b"Mulan PSL v2".len())
            .any(|part| part == b"Mulan PSL v2")
    );
    assert!(!version.stdout.windows(3).any(|part| part == b"GNU"));
}
