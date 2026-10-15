#![cfg(unix)]

use std::os::unix::process::CommandExt;
use std::process::Command;

fn run_mv(argument: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mv"))
        .arg0("mv")
        .env("LC_ALL", "C")
        .arg(argument)
        .output()
        .expect("run mv")
}

#[test]
fn help_and_version_identify_syskits_without_gnu_branding() {
    let help = run_mv("--help");
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(!help.contains("GNU"));
    assert!(!help.contains("coreutils"));

    let version = run_mv("--version");
    assert!(version.status.success());
    assert!(
        String::from_utf8(version.stdout)
            .unwrap()
            .contains("syskits")
    );
}
