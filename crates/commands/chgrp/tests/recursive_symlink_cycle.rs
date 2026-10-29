#[cfg(unix)]
#[test]
fn chgrp_ignores_recursive_symlink_cycles_like_gnu() {
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::os::unix::process::CommandExt;
    use std::process::Command;
    use tempfile::tempdir;

    let temp = tempdir().expect("create temporary directory");
    let tree = temp.path().join("tree");
    fs::create_dir(&tree).expect("create tree");
    fs::write(tree.join("file"), b"data").expect("create regular file");
    symlink(".", tree.join("self")).expect("create recursive symlink");

    let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
        .env("LC_ALL", "C")
        .arg0("chgrp")
        .args(["-RL", "0"])
        .arg(&tree)
        .output()
        .expect("run chgrp");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[cfg(unix)]
#[test]
fn chgrp_logical_recursive_cycle_is_processed_as_a_leaf_like_gnu() {
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::os::unix::process::CommandExt;
    use std::process::Command;
    use tempfile::tempdir;

    let temp = tempdir().expect("create temporary directory");
    let tree = temp.path().join("tree");
    fs::create_dir(&tree).expect("create tree");
    fs::write(tree.join("file"), b"data").expect("create regular file");
    symlink(".", tree.join("self")).expect("create recursive symlink");

    let arguments = ["-RLv", "0"];
    let gnu_output = Command::new("/usr/bin/chgrp")
        .env("LC_ALL", "C")
        .arg0("chgrp")
        .args(arguments)
        .arg(&tree)
        .output()
        .expect("run GNU chgrp");

    assert_eq!(gnu_output.status.code(), Some(0));
    assert!(gnu_output.stderr.is_empty());

    let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
        .env("LC_ALL", "C")
        .arg0("chgrp")
        .args(arguments)
        .arg(&tree)
        .output()
        .expect("run chgrp");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, gnu_output.stdout);
    assert_eq!(output.stderr, gnu_output.stderr);
}
