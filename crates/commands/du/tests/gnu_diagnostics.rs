#![cfg(target_os = "linux")]

use std::ffi::OsString;
use std::fs::{create_dir_all, write};
use std::os::unix::{fs::symlink, process::CommandExt};
use std::path::Path;
use std::process::{Command, Output};

fn run_du(binary: &str, working_dir: &Path, args: &[OsString]) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0("du")
        .current_dir(working_dir)
        .env("LC_ALL", "C.UTF-8")
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG")
        .env_remove("LANGUAGE")
        .args(args);
    command.output().expect("run du")
}

fn assert_matches_gnu(working_dir: &Path, args: &[OsString]) {
    let gnu = run_du("/usr/bin/du", working_dir, args);
    let syskits = run_du(env!("CARGO_BIN_EXE_du"), working_dir, args);

    assert_eq!(syskits.status.code(), gnu.status.code(), "{args:?}");
    assert_eq!(syskits.stdout, gnu.stdout, "{args:?}");
    assert_eq!(syskits.stderr, gnu.stderr, "{args:?}");
}

#[test]
fn missing_threshold_argument_matches_gnu() {
    let working_dir = tempfile::tempdir().expect("create working directory");
    assert_matches_gnu(working_dir.path(), &[OsString::from("--threshold")]);
}

#[test]
fn invalid_threshold_option_name_matches_gnu() {
    let working_dir = tempfile::tempdir().expect("create working directory");
    for args in [
        vec![OsString::from("-t"), OsString::from("SIZE")],
        vec![OsString::from("-tSIZE")],
        vec![OsString::from("-htSIZE")],
        vec![OsString::from("--threshold=SIZE")],
        vec![OsString::from("--thres"), OsString::from("SIZE")],
        vec![
            OsString::from("--time"),
            OsString::from("-t"),
            OsString::from("SIZE"),
        ],
    ] {
        assert_matches_gnu(working_dir.path(), &args);
    }
}

#[test]
fn files0_from_rejects_file_operands_like_gnu() {
    let working_dir = tempfile::tempdir().expect("create working directory");
    let input = working_dir.path().join("input");
    let list = working_dir.path().join("list");
    write(&input, b"").expect("create input file");
    let mut record = input.as_os_str().as_encoded_bytes().to_vec();
    record.push(0);
    write(&list, record).expect("write files0 list");

    assert_matches_gnu(
        working_dir.path(),
        &[
            OsString::from(format!("--files0-from={}", list.display())),
            input.into_os_string(),
        ],
    );
}

#[test]
fn dereferenced_dangling_symlinks_match_gnu() {
    let working_dir = tempfile::tempdir().expect("create working directory");
    let directory = working_dir.path().join("directory");
    let dangling = directory.join("dangling");
    create_dir_all(&directory).expect("create input directory");
    symlink("missing", &dangling).expect("create dangling symlink");

    for path in [dangling, directory] {
        assert_matches_gnu(
            working_dir.path(),
            &[OsString::from("-L"), path.into_os_string()],
        );
    }
}

#[test]
fn long_dereferenced_symlink_chain_matches_gnu() {
    let working_dir = tempfile::tempdir().expect("create working directory");
    let chain_length = 64;

    for index in 1..=chain_length {
        let directory = working_dir.path().join(index.to_string());
        create_dir_all(&directory).expect("create chain directory");
        symlink(
            format!("../{}", index + 1),
            directory.join("s"),
        )
        .expect("create chain symlink");
    }
    create_dir_all(working_dir.path().join((chain_length + 1).to_string()))
        .expect("create chain target");

    assert_matches_gnu(
        working_dir.path(),
        &[OsString::from("-L"), OsString::from("1")],
    );
}

#[test]
fn summarize_depth_conflict_matches_gnu() {
    let working_dir = tempfile::tempdir().expect("create working directory");
    let directory = working_dir.path().join("directory");
    create_dir_all(&directory).expect("create input directory");

    assert_matches_gnu(
        working_dir.path(),
        &[
            OsString::from("-s"),
            OsString::from("--max-depth=1"),
            directory.into_os_string(),
        ],
    );
}
