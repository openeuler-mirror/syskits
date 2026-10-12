#[cfg(unix)]
#[test]
fn chgrp_verbose_maximum_gid_is_retained_like_gnu() {
    use std::fs;
    use std::os::unix::{fs::MetadataExt, process::CommandExt};
    use std::process::Command;
    use tempfile::tempdir;

    let temp = tempdir().expect("create temporary directory");
    let file = temp.path().join("file");
    fs::write(&file, b"data").expect("create file");
    let group = fs::metadata(&file).expect("read file metadata").gid();
    let group_name = ctcore::ct_entries::gid2grp(group).unwrap_or_else(|_| group.to_string());

    let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
        .env("LC_ALL", "C")
        .arg0("chgrp")
        .args(["-v", &u32::MAX.to_string()])
        .arg(&file)
        .output()
        .expect("run chgrp");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        format!("group of '{}' retained as {group_name}\n", file.display())
    );
}
