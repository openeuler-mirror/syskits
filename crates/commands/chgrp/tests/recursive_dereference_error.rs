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

#[cfg(unix)]
#[test]
fn chgrp_verbose_dangling_symlink_reports_its_link_group_like_gnu() {
    use std::os::unix::{fs::symlink, process::CommandExt};
    use std::process::Command;
    use tempfile::tempdir;

    let temp = tempdir().expect("create temporary directory");
    let dangling = temp.path().join("dangling");
    symlink("missing", &dangling).expect("create dangling symlink");

    let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
        .current_dir(temp.path())
        .env("LC_ALL", "C")
        .arg0("chgrp")
        .args(["-v", "0", "dangling"])
        .output()
        .expect("run chgrp");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        "failed to change group of 'dangling' from root to 0\n"
    );
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr is UTF-8"),
        "chgrp: cannot dereference 'dangling': No such file or directory\n"
    );
}

#[cfg(unix)]
#[test]
fn chgrp_recursive_verbose_dangling_symlink_reports_its_link_group_like_gnu() {
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::os::unix::process::CommandExt;
    use std::process::Command;
    use tempfile::tempdir;

    let temp = tempdir().expect("create temporary directory");
    let tree = temp.path().join("tree");
    fs::create_dir(&tree).expect("create tree");
    let dangling = tree.join("dangling");
    symlink("missing", &dangling).expect("create dangling symlink");

    let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
        .current_dir(temp.path())
        .env("LC_ALL", "C")
        .arg0("chgrp")
        .args(["-RLv", "0", "tree"])
        .output()
        .expect("run chgrp");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        "failed to change group of 'tree/dangling' from root to 0\n\
         group of 'tree' retained as root\n"
    );
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr is UTF-8"),
        "chgrp: cannot dereference 'tree/dangling': No such file or directory\n"
    );
}

#[cfg(unix)]
#[test]
fn chgrp_logical_recursive_no_dereference_changes_dangling_links_like_gnu() {
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::os::unix::process::CommandExt;
    use std::process::Command;
    use tempfile::tempdir;

    let temp = tempdir().expect("create temporary directory");
    let tree = temp.path().join("tree");
    fs::create_dir(&tree).expect("create tree");
    let dangling = tree.join("dangling");
    symlink("missing", &dangling).expect("create dangling symlink");

    let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
        .current_dir(temp.path())
        .env("LC_ALL", "C")
        .arg0("chgrp")
        .args(["-RLhv", "0", "tree"])
        .output()
        .expect("run chgrp");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        "group of 'tree/dangling' retained as root\n\
         group of 'tree' retained as root\n"
    );
    assert!(output.stderr.is_empty());
}

#[cfg(unix)]
#[test]
fn chgrp_recursive_symbolic_link_loop_uses_access_diagnostics_like_gnu() {
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::os::unix::process::CommandExt;
    use std::process::Command;
    use tempfile::tempdir;

    let temp = tempdir().expect("create temporary directory");
    let tree = temp.path().join("tree");
    fs::create_dir(&tree).expect("create tree");
    symlink("b", tree.join("a")).expect("create first link");
    symlink("a", tree.join("b")).expect("create second link");

    let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
        .current_dir(temp.path())
        .env("LC_ALL", "C")
        .arg0("chgrp")
        .args(["-RLv", "0", "tree"])
        .output()
        .expect("run chgrp");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        "failed to change group of 'tree/b' to 0\n\
         failed to change group of 'tree/a' to 0\n\
         group of 'tree' retained as root\n"
    );
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr is UTF-8"),
        "chgrp: cannot access 'tree/b': Too many levels of symbolic links\n\
         chgrp: cannot access 'tree/a': Too many levels of symbolic links\n"
    );
}
