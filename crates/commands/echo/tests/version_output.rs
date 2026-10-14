#![cfg(unix)]

use std::os::unix::process::CommandExt;
use std::process::Command;

#[test]
fn version_output_uses_syskits_information() {
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
        format!(
            r#"echo (syskits) {}
Copyright (C) 2022-2025 China Telecom Cloud Technologies Co., Ltd.
License Mulan PSL v2: <http://license.coscl.org.cn/MulanPSL2>.
This is free software: you are free to change and redistribute it.
There is NO WARRANTY, to the extent permitted by law.
"#,
            env!("CARGO_PKG_VERSION")
        )
        .as_bytes()
    );
    assert!(!output.stdout.windows(3).any(|window| window == b"GNU"));
}
