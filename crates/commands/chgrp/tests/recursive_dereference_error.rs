#[cfg(unix)]
#[test]
fn chgrp_reports_recursive_dangling_symlink_dereference_like_gnu() {
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::os::unix::process::CommandExt;
    use std::process::Command;
    use tempfile::tempdir;

    let temp = tempdir().expect("create temporary directory");
    let tree = temp.path().join("tree");
    fs::create_dir(&tree).expect("create tree");
    let dangling = tree.join("a-dangling");
    symlink("missing", &dangling).expect("create dangling symlink");

    let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
        .env("LC_ALL", "C")
        .arg0("chgrp")
        .args(["-RL", "0"])
        .arg(&tree)
        .output()
        .expect("run chgrp");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr is UTF-8"),
        format!(
            "chgrp: cannot dereference '{}': No such file or directory\n",
            dangling.display()
        )
    );
}
