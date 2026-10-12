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
