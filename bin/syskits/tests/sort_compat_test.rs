use std::fs;
use std::process::Command;

use tempfile::TempDir;

#[test]
fn sort_debug_reports_text_ordering_rules() {
    let temp_dir = TempDir::new().expect("tempdir");
    let input = temp_dir.path().join("sortfile");
    fs::write(&input, b"11\n123\n22\n3333\n").expect("write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["sort", "--debug", input.to_str().expect("utf8 path")])
        .output()
        .expect("run syskits sort --debug");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"11\n__\n123\n___\n22\n__\n3333\n____\n");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("sort: text ordering performed using "),
        "stderr: {stderr}"
    );
    assert!(
        stderr.ends_with(" sorting rules\n") || stderr.ends_with(" simple byte comparison\n"),
        "stderr: {stderr}"
    );
}

#[test]
fn sort_debug_reports_simple_byte_comparison_in_c_locale() {
    let temp_dir = TempDir::new().expect("tempdir");
    let input = temp_dir.path().join("sortfile");
    fs::write(&input, b"b\na\n").expect("write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C")
        .args(["sort", "--debug", input.to_str().expect("utf8 path")])
        .output()
        .expect("run syskits sort --debug");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"a\n_\nb\n_\n");
    assert_eq!(
        output.stderr,
        b"sort: text ordering performed using simple byte comparison\n"
    );
}
