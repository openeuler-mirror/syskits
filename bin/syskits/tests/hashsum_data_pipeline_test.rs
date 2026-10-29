#![cfg(feature = "feat_data_pipeline")]

use std::io::Write;
use std::process::{Command, Stdio};

fn run_hash_command(command: &str, input: &[u8]) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", command])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn syskits");
    child
        .stdin
        .take()
        .expect("stdin handle")
        .write_all(input)
        .expect("write stdin");
    child.wait_with_output().expect("wait for syskits")
}

#[test]
fn hashsum_aliases_produce_structured_json() {
    for (command, algorithm) in [
        ("md5sum | to json", "MD5"),
        ("sha256sum | to json", "SHA256"),
        ("sha3-256sum | to json", "SHA3-256"),
    ] {
        let output = run_hash_command(command, b"x");
        assert_eq!(output.status.code(), Some(0), "stderr: {:?}", output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains(&format!("\"algorithm\": \"{algorithm}\"")));
        assert!(!stdout.contains("cannot serialize ByteStream"));
    }
}

#[test]
fn hashsum_alias_with_bits_keeps_alias_specific_validation() {
    let output = run_hash_command("sha3sum --bits 256 | to json", b"x");
    assert_eq!(output.status.code(), Some(0), "stderr: {:?}", output.stderr);
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"algorithm\": \"SHA3-256\""));
}
