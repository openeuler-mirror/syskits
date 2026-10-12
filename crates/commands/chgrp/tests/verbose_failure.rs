#[cfg(unix)]
#[test]
fn chgrp_verbose_failure_separates_status_from_diagnostic() {
    use std::fs::{self, Permissions};
    use std::os::unix::{fs::PermissionsExt, process::CommandExt};
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("tempdir");
    fs::set_permissions(temp_dir.path(), Permissions::from_mode(0o755)).expect("make tempdir traversable");
    let file = temp_dir.path().join("file");
    fs::write(&file, []).expect("create file");
    fs::set_permissions(&file, Permissions::from_mode(0o644)).expect("set file permissions");

    let mut command = Command::new(env!("CARGO_BIN_EXE_chgrp"));
    command
        .env("LC_ALL", "C")
        .arg0("chgrp")
        .args(["-v", "55555"])
        .arg(&file);
    if unsafe { libc::geteuid() } == 0 {
        command.uid(65534).gid(65534);
    }

    let output = command.output().expect("run chgrp");
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");

    assert_eq!(output.status.code(), Some(1));
    assert!(stdout.starts_with("failed to change group of "));
    assert!(stdout.ends_with(" to 55555\n"));
    assert_eq!(stdout.lines().count(), 1);
    assert!(stderr.starts_with("chgrp: changing group of "));
    assert!(stderr.ends_with(": Operation not permitted\n"));
    assert_eq!(stderr.lines().count(), 1);
}
