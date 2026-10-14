#![cfg(target_os = "linux")]

use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn run_hostid(binary: &str, program_name: &str, args: &[&str]) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0(program_name)
        .args(args)
        .stdin(Stdio::null())
        .env("LC_ALL", "C")
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG");
    command.output().expect("run hostid")
}

fn expected_version(program_name: &str) -> String {
    let program_name = Path::new(program_name)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(program_name);
    format!(
        "\
{program_name} (syskits) {}
Copyright (C) 2022-2025 China Telecom Cloud Technologies Co., Ltd.
License Mulan PSL v2: <http://license.coscl.org.cn/MulanPSL2>.
This is free software: you are free to change and redistribute it.
There is NO WARRANTY, to the extent permitted by law.
",
        env!("CARGO_PKG_VERSION")
    )
}

#[test]
fn version_output_uses_syskits_information() {
    for program_name in ["hostid", "custom", "/tmp/custom"] {
        for args in [
            ["--version"].as_slice(),
            ["--vers"].as_slice(),
            ["--v"].as_slice(),
            ["value", "--version"].as_slice(),
        ] {
            let syskits = run_hostid(env!("CARGO_BIN_EXE_hostid"), program_name, args);

            assert_eq!(syskits.status.code(), Some(0), "{program_name}: {args:?}");
            assert_eq!(
                syskits.stdout,
                expected_version(program_name).as_bytes(),
                "{program_name}: {args:?}"
            );
            assert!(syskits.stderr.is_empty(), "{program_name}: {args:?}");
            assert!(
                !syskits.stdout.windows(3).any(|window| window == b"GNU"),
                "{program_name}: {args:?}"
            );
        }
    }
}
