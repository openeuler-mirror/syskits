use std::fs;
use std::process::Command;

use tempfile::TempDir;

#[cfg(target_os = "linux")]
#[test]
fn cp_debug_reports_copy_offload_like_coreutils() {
    let temp_dir = TempDir::new().expect("tempdir");
    fs::write(temp_dir.path().join("testfile"), b"copy offload data\n").expect("write fixture");

    let gnu_output = Command::new("/usr/bin/cp")
        .current_dir(temp_dir.path())
        .args(["--debug", "testfile", "testfile2"])
        .output()
        .expect("run /usr/bin/cp");
    fs::remove_file(temp_dir.path().join("testfile2")).expect("remove gnu output");

    let syskits_output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .args(["cp", "--debug", "testfile", "testfile2"])
        .output()
        .expect("run syskits cp");

    assert_eq!(syskits_output.status.code(), gnu_output.status.code());
    assert_eq!(
        String::from_utf8_lossy(&syskits_output.stdout),
        String::from_utf8_lossy(&gnu_output.stdout)
    );
    assert_eq!(
        String::from_utf8_lossy(&syskits_output.stderr),
        String::from_utf8_lossy(&gnu_output.stderr)
    );
}
