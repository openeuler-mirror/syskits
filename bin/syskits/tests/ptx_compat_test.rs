use std::fs;
use std::process::Command;

use tempfile::TempDir;

#[test]
fn ptx_auto_reference_keeps_physical_input_lines() {
    let temp_dir = TempDir::new().expect("tempdir");
    let input = temp_dir.path().join("testfile");
    fs::write(&input, "first line.\nsecond line.\nthis is third line.\n").expect("write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .args(["ptx", "-A", "testfile"])
        .output()
        .expect("run syskits ptx -A");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stderr, b"");

    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    let path = "testfile";
    let expected = format!(
        "{path}:1:                               first line.\n\
{path}:3:                        this   is third line.\n\
{path}:1:                       first   line.\n\
{path}:2:                      second   line.\n\
{path}:3:               this is third   line.\n\
{path}:2:                               second line.\n\
{path}:3:                     this is   third line.\n\
{path}:3:                               this is third line.\n"
    );
    assert_eq!(stdout, expected);
}
