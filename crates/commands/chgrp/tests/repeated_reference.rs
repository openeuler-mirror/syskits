#[cfg(unix)]
#[test]
fn chgrp_uses_the_final_repeated_reference_option_like_gnu() {
    use std::fs;
    use std::os::unix::process::CommandExt;
    use std::process::Command;
    use tempfile::tempdir;

    let temp = tempdir().expect("create temporary directory");
    let reference = temp.path().join("reference");
    fs::write(&reference, b"data").expect("create reference file");

    let path = "/definitely-missing-chgrp-file";
    let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
        .env("LC_ALL", "C")
        .arg0("chgrp")
        .arg("--reference=/definitely-missing-chgrp-reference")
        .arg(format!("--reference={}", reference.display()))
        .arg(path)
        .output()
        .expect("run chgrp");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr is UTF-8"),
        format!("chgrp: cannot access '{path}': No such file or directory\n")
    );
}
