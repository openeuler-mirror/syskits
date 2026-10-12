#[cfg(unix)]
#[test]
fn chgrp_recursive_failure_is_not_hidden_by_later_success() {
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::os::unix::process::CommandExt;
    use std::process::Command;
    use tempfile::tempdir;

    let temp = tempdir().expect("create temporary directory");
    let tree = temp.path().join("tree");
    fs::create_dir(&tree).expect("create tree");
    symlink("missing", tree.join("a-dangling")).expect("create dangling symlink");
    fs::write(tree.join("z-good"), b"data").expect("create regular file");

    let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
        .env("LC_ALL", "C")
        .arg0("chgrp")
        .args(["-RL", "0"])
        .arg(&tree)
        .output()
        .expect("run chgrp");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
}
