#![cfg(target_os = "linux")]

use std::os::unix::{fs::symlink, process::CommandExt};
use std::process::Command;

#[test]
fn symlink_loop_creation_errors_preserve_the_original_template_and_errno() {
    for (options, kind) in [(&[][..], "file"), (&["-d"][..], "directory")] {
        let directory = tempfile::tempdir().expect("create temporary directory");
        let loop_dir = directory.path().join("loop");
        symlink("loop", &loop_dir).expect("create symbolic link loop");
        let template = loop_dir.join("mktemp-loop.XXX");

        let output = Command::new(env!("CARGO_BIN_EXE_mktemp"))
            .arg0("mktemp")
            .env("LC_ALL", "C")
            .args(options)
            .arg("-p")
            .arg(&loop_dir)
            .arg("mktemp-loop.XXX")
            .output()
            .expect("run mktemp with a symbolic link loop");

        assert_eq!(output.status.code(), Some(1), "{kind}");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            format!(
                "mktemp: failed to create {kind} via template '{}': Too many levels of symbolic links\n",
                template.display()
            ),
            "{kind}"
        );
    }
}
