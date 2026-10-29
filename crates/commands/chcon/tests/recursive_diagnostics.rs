#[cfg(unix)]
#[test]
fn recursive_physical_diagnostics_use_fts_access_names() {
    use std::os::unix::{fs::symlink, process::CommandExt};
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("tempdir");
    let root = temp_dir.path().join("rootdir");
    let target = temp_dir.path().join("targetdir");
    std::fs::create_dir_all(&root).expect("create root directory");
    std::fs::create_dir_all(&target).expect("create target directory");
    std::fs::write(root.join("f1"), []).expect("create f1");
    std::fs::write(root.join("f2"), []).expect("create f2");
    std::fs::write(target.join("inside"), []).expect("create symlink target file");
    symlink("../targetdir", root.join("linkdir")).expect("create directory symlink");

    let output = Command::new(env!("CARGO_BIN_EXE_chcon"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .arg0("chcon")
        .args(["-R", "-P", "-t", "tmp_t", "rootdir"])
        .output()
        .expect("run chcon");

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    for path in ["f1", "f2", "linkdir", "rootdir"] {
        assert!(
            stderr.contains(&format!(
                "chcon: can't apply partial context to unlabeled file '{path}'\n"
            )),
            "missing diagnostic for {path}: {stderr:?}"
        );
    }
    assert!(
        !stderr.contains("'rootdir/"),
        "unexpected full child path: {stderr:?}"
    );
    assert!(
        !stderr.contains("'inside'"),
        "-P must not traverse linkdir: {stderr:?}"
    );
}
