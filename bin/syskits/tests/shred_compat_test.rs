use std::fs;
use std::process::Command;

use tempfile::TempDir;

#[test]
fn shred_verbose_pass_counter_has_single_space() {
    let temp_dir = TempDir::new().expect("tempdir");
    fs::write(temp_dir.path().join("testfile"), b"sensitive data").expect("write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .args(["shred", "-n", "1", "-v", "testfile"])
        .output()
        .expect("run syskits shred");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "shred: testfile: pass 1/1 (random)...\n"
    );
}
