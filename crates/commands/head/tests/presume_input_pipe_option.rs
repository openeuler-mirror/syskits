#![cfg(target_os = "linux")]

use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};

fn run_head(binary: &str, argument: &str) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0("head")
        .args([argument, "--lines=1", "/dev/null"])
        .stdin(Stdio::null())
        .env("LC_ALL", "C")
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG");
    command.output().expect("run head")
}

#[test]
fn only_the_gnu_internal_option_spelling_is_accepted() {
    let gnu_internal = run_head("/usr/bin/head", "---presume-input-pipe");
    let syskits_internal = run_head(env!("CARGO_BIN_EXE_head"), "---presume-input-pipe");
    assert_eq!(syskits_internal.status.code(), gnu_internal.status.code());
    assert_eq!(syskits_internal.stdout, gnu_internal.stdout);
    assert_eq!(syskits_internal.stderr, gnu_internal.stderr);

    let gnu_public = run_head("/usr/bin/head", "--presume-input-pipe");
    let syskits_public = run_head(env!("CARGO_BIN_EXE_head"), "--presume-input-pipe");
    assert_eq!(syskits_public.status.code(), gnu_public.status.code());
    assert_eq!(syskits_public.stdout, gnu_public.stdout);
    assert_eq!(syskits_public.stderr, gnu_public.stderr);
}
