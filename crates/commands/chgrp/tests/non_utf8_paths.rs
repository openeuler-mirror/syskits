#[cfg(unix)]
#[test]
fn chgrp_accepts_non_utf8_file_names_like_gnu() {
    use std::ffi::OsString;
    use std::fs;
    use std::os::unix::{ffi::OsStringExt, fs::MetadataExt, process::CommandExt};
    use std::process::Command;
    use tempfile::tempdir;

    let temp = tempdir().expect("create temporary directory");
    let file = temp.path().join(OsString::from_vec(b"file_\xff".to_vec()));
    fs::write(&file, b"data").expect("create file");
    let group = fs::metadata(&file)
        .expect("read file metadata")
        .gid()
        .to_string();

    let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
        .env("LC_ALL", "C")
        .arg0("chgrp")
        .arg(group)
        .arg(&file)
        .output()
        .expect("run chgrp");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[cfg(unix)]
#[test]
fn chgrp_quotes_non_utf8_file_names_like_gnu() {
    use std::ffi::OsString;
    use std::fs;
    use std::os::unix::{ffi::OsStringExt, fs::MetadataExt, process::CommandExt};
    use std::process::Command;
    use tempfile::tempdir;

    let temp = tempdir().expect("create temporary directory");
    let file = temp.path().join(OsString::from_vec(b"file_\xff".to_vec()));
    fs::write(&file, b"data").expect("create file");
    let group = fs::metadata(&file)
        .expect("read file metadata")
        .gid()
        .to_string();

    let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
        .env("LC_ALL", "C")
        .arg0("chgrp")
        .args(["-v", &group])
        .arg(&file)
        .output()
        .expect("run chgrp");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        format!(
            "group of '{}'$'\\377' retained as root\n",
            temp.path().join("file_").display()
        )
    );
}

#[cfg(unix)]
#[test]
fn chgrp_quotes_non_utf8_file_names_starting_with_an_apostrophe_like_gnu() {
    use std::ffi::OsString;
    use std::fs;
    use std::os::unix::{ffi::OsStringExt, fs::MetadataExt, process::CommandExt};
    use std::process::Command;
    use tempfile::tempdir;

    let temp = tempdir().expect("create temporary directory");
    let file = OsString::from_vec(b"'file_\xff".to_vec());
    let path = temp.path().join(&file);
    fs::write(&path, b"data").expect("create file");
    let group = fs::metadata(&path)
        .expect("read file metadata")
        .gid()
        .to_string();

    let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
        .current_dir(temp.path())
        .env("LC_ALL", "C")
        .arg0("chgrp")
        .args(["-v", &group])
        .arg(&file)
        .output()
        .expect("run chgrp");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        "group of ''\\''file_'$'\\377' retained as root\n"
    );
}

#[cfg(unix)]
#[test]
fn chgrp_accepts_non_utf8_reference_file_names_like_gnu() {
    use std::ffi::OsString;
    use std::fs;
    use std::os::unix::{ffi::OsStringExt, process::CommandExt};
    use std::process::Command;
    use tempfile::tempdir;

    let temp = tempdir().expect("create temporary directory");
    let reference = temp
        .path()
        .join(OsString::from_vec(b"reference_\xff".to_vec()));
    let file = temp.path().join("file");
    fs::write(&reference, b"reference").expect("create reference file");
    fs::write(&file, b"data").expect("create file");

    let mut reference_option = OsString::from("--reference=");
    reference_option.push(&reference);
    let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
        .env("LC_ALL", "C")
        .arg0("chgrp")
        .arg(reference_option)
        .arg(&file)
        .output()
        .expect("run chgrp");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}
