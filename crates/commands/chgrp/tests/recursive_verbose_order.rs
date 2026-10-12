#[cfg(unix)]
#[test]
fn chgrp_reports_recursive_directories_after_their_contents_like_gnu() {
    use std::fs;
    use std::os::unix::process::CommandExt;
    use std::process::Command;
    use tempfile::tempdir;

    let temp = tempdir().expect("create temporary directory");
    let tree = temp.path().join("tree");
    let subdir = tree.join("a-sub");
    let file = tree.join("a-file");
    let nested_file = subdir.join("z-file");
    fs::create_dir_all(&subdir).expect("create tree");
    fs::write(&file, b"data").expect("create regular file");
    fs::write(&nested_file, b"data").expect("create nested regular file");

    let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
        .env("LC_ALL", "C")
        .arg0("chgrp")
        .args(["-Rv", "0"])
        .arg(&tree)
        .output()
        .expect("run chgrp");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        format!(
            "group of '{}' retained as root\n\
             group of '{}' retained as root\n\
             group of '{}' retained as root\n\
             group of '{}' retained as root\n",
            file.display(),
            nested_file.display(),
            subdir.display(),
            tree.display(),
        )
    );
}

#[cfg(unix)]
#[test]
fn chgrp_h_recursive_processes_nested_directories_under_a_root_symlink() {
    use std::fs;
    use std::os::unix::{fs::symlink, process::CommandExt};
    use std::process::Command;
    use tempfile::tempdir;

    let temp = tempdir().expect("create temporary directory");
    let tree = temp.path().join("tree");
    let subdir = tree.join("subdir");
    let file = tree.join("file");
    let nested_file = subdir.join("file");
    let root_link = temp.path().join("tree-link");
    let linked_file = root_link.join("file");
    let linked_subdir = root_link.join("subdir");
    let linked_nested_file = linked_subdir.join("file");
    fs::create_dir_all(&subdir).expect("create tree");
    fs::write(&file, b"data").expect("create regular file");
    fs::write(&nested_file, b"data").expect("create nested regular file");
    symlink(&tree, &root_link).expect("create root symlink");

    let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
        .env("LC_ALL", "C")
        .arg0("chgrp")
        .args(["-HRv", "0"])
        .arg(&root_link)
        .output()
        .expect("run chgrp");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        format!(
            "group of '{}' retained as root\n\
             group of '{}' retained as root\n\
             group of '{}' retained as root\n\
             group of '{}' retained as root\n",
            linked_file.display(),
            linked_nested_file.display(),
            linked_subdir.display(),
            root_link.display(),
        )
    );
}
