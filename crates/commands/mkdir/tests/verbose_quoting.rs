#![cfg(target_os = "linux")]

use std::ffi::{OsStr, OsString};
use std::os::unix::{ffi::OsStringExt, process::CommandExt};
use std::process::{Command, Output};

fn run_mkdir(binary: &str, locale: &str, working_dir: &std::path::Path, path: &OsStr) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0("mkdir")
        .current_dir(working_dir)
        .env("LC_ALL", locale)
        .args(["-p", "-v"])
        .arg(path);
    command.output().expect("run mkdir")
}

#[test]
fn verbose_paths_use_gnu_shell_quoting() {
    let cases = [
        ("C", OsString::from("foo/a/b/c")),
        ("C.UTF-8", OsString::from("has space")),
        ("C.UTF-8", OsString::from("has'apostrophe")),
        ("C.UTF-8", OsString::from("α")),
        (
            "en_US.iso88591",
            OsString::from_vec(b"printable-\xe9".to_vec()),
        ),
    ];

    for (locale, path) in cases {
        let gnu_dir = tempfile::tempdir().expect("create GNU working directory");
        let syskits_dir = tempfile::tempdir().expect("create syskits working directory");

        let gnu = run_mkdir("/usr/bin/mkdir", locale, gnu_dir.path(), &path);
        let syskits = run_mkdir(
            env!("CARGO_BIN_EXE_mkdir"),
            locale,
            syskits_dir.path(),
            &path,
        );

        assert_eq!(
            syskits.status.code(),
            gnu.status.code(),
            "{locale}: {path:?}"
        );
        assert_eq!(syskits.stdout, gnu.stdout, "{locale}: {path:?}");
        assert_eq!(syskits.stderr, gnu.stderr, "{locale}: {path:?}");
    }
}
