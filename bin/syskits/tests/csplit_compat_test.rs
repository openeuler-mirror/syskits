use std::fs;
use std::process::Command;

use tempfile::TempDir;

#[test]
fn csplit_keeps_invalid_utf8_input_without_decode_error() {
    let temp_dir = TempDir::new().expect("tempdir");
    fs::write(temp_dir.path().join("testfile"), [0xff]).expect("write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .args(["csplit", "-k", "-f", "keep", "testfile", "/nomatch/"])
        .output()
        .expect("run syskits csplit");

    assert!(!output.status.success());
    assert_eq!(output.stdout, b"1\n");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("match not found"), "stderr: {stderr}");
    assert!(
        !stderr.contains("valid UTF-8"),
        "stderr should not report UTF-8 decoding: {stderr}"
    );
    assert_eq!(fs::read(temp_dir.path().join("keep00")).unwrap(), [0xff]);
}

#[test]
fn csplit_does_not_append_newline_to_binary_final_line() {
    let temp_dir = TempDir::new().expect("tempdir");
    let data = vec![0; 1024];
    fs::write(temp_dir.path().join("testfile"), &data).expect("write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .args(["csplit", "-k", "-f", "keep", "testfile", "/nomatch/"])
        .output()
        .expect("run syskits csplit");

    assert!(!output.status.success());
    assert_eq!(output.stdout, b"1024\n");
    assert_eq!(fs::read(temp_dir.path().join("keep00")).unwrap(), data);
}

#[test]
fn csplit_matches_ascii_regex_inside_invalid_utf8_line() {
    let temp_dir = TempDir::new().expect("tempdir");
    let data = b"abc\xffnomatch\n";
    fs::write(temp_dir.path().join("testfile"), data).expect("write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .args(["csplit", "-k", "-f", "keep", "testfile", "/nomatch/"])
        .output()
        .expect("run syskits csplit");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"0\n12\n");
    assert_eq!(fs::read(temp_dir.path().join("keep00")).unwrap(), b"");
    assert_eq!(fs::read(temp_dir.path().join("keep01")).unwrap(), data);
}

#[test]
fn csplit_prints_match_error_before_kept_byte_count() {
    let temp_dir = TempDir::new().expect("tempdir");
    let data = vec![0; 1024];
    fs::write(temp_dir.path().join("testfile"), &data).expect("write fixture");

    let syskits = env!("CARGO_BIN_EXE_syskits");
    let output = Command::new("sh")
        .current_dir(temp_dir.path())
        .arg("-c")
        .arg(format!(
            "{syskits} csplit -k -f keep testfile /nomatch/ 2>&1"
        ))
        .output()
        .expect("run syskits csplit");

    assert!(!output.status.success());
    let combined = String::from_utf8_lossy(&output.stdout);

    assert!(
        combined.starts_with("csplit: '/nomatch/': match not found\n1024\n"),
        "combined output: {combined:?}"
    );
}
