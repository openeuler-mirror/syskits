#![cfg(unix)]

use std::os::unix::process::CommandExt;
use std::process::Command;

#[test]
fn version_output_matches_gnu_coreutils() {
    let output = Command::new(env!("CARGO_BIN_EXE_echo"))
        .arg0("echo")
        .arg("--version")
        .env("LC_ALL", "C")
        .output()
        .expect("run echo --version");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(
        output.stdout,
        br#"echo (GNU coreutils) 9.4
Copyright (C) 2023 Free Software Foundation, Inc.
License GPLv3+: GNU GPL version 3 or later <https://gnu.org/licenses/gpl.html>.
This is free software: you are free to change and redistribute it.
There is NO WARRANTY, to the extent permitted by law.

Written by Brian Fox and Chet Ramey.
"#
    );
}
