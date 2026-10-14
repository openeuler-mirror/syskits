#![cfg(target_os = "linux")]

use std::os::unix::process::CommandExt;
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

fn expected_help(program_name: &str) -> String {
    format!(
        "\
Usage: {program_name} [OPTION]
Print the numeric identifier (in hexadecimal) for the current host.

      --help        display this help and exit
      --version     output version information and exit

syskits online help: <https://www.ctyun.cn/developer>
"
    )
}

#[test]
fn help_output_uses_syskits_information() {
    for program_name in ["hostid", "custom", "/tmp/custom"] {
        for args in [
            ["--help"].as_slice(),
            ["--hel"].as_slice(),
            ["--h"].as_slice(),
            ["value", "--help"].as_slice(),
        ] {
            let syskits = run_hostid(env!("CARGO_BIN_EXE_hostid"), program_name, args);

            assert_eq!(syskits.status.code(), Some(0), "{program_name}: {args:?}");
            assert_eq!(
                syskits.stdout,
                expected_help(program_name).as_bytes(),
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
