#[cfg(unix)]
#[test]
fn chgrp_verbose_empty_group_reports_retained_ownership_like_gnu() {
    use std::fs;
    use std::os::unix::process::CommandExt;
    use std::process::Command;
    use tempfile::tempdir;

    let temp = tempdir().expect("create temporary directory");
    let file = temp.path().join("file");
    fs::write(&file, b"data").expect("create file");

    let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
        .env("LC_ALL", "C")
        .arg0("chgrp")
        .args(["-v", ""])
        .arg(&file)
        .output()
        .expect("run chgrp");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        format!("ownership of '{}' retained\n", file.display())
    );
}
