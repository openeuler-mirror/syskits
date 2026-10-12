#[cfg(unix)]
#[test]
fn chgrp_posixly_correct_stops_option_parsing_at_the_first_operand() {
    use std::fs;
    use std::os::unix::{fs::MetadataExt, process::CommandExt};
    use std::process::Command;
    use tempfile::tempdir;

    let temp = tempdir().expect("create temporary directory");
    let file = temp.path().join("file");
    fs::write(&file, b"data").expect("create file");
    let group = fs::metadata(&file)
        .expect("read file metadata")
        .gid()
        .to_string();

    let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
        .current_dir(temp.path())
        .env("LC_ALL", "C")
        .env("POSIXLY_CORRECT", "1")
        .arg0("chgrp")
        .args([group.as_str(), "file", "-v"])
        .output()
        .expect("run chgrp");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr is UTF-8"),
        "chgrp: cannot access '-v': No such file or directory\n"
    );
}
