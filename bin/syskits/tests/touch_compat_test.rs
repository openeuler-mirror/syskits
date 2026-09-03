use std::process::Command;

use tempfile::TempDir;

#[test]
fn touch_date_numeric_operand_is_today_time() {
    let temp_dir = TempDir::new().expect("tempdir");
    let file = temp_dir.path().join("touch-out");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("TZ", "Asia/Shanghai")
        .args(["touch", "--date=1", file.to_str().unwrap()])
        .output()
        .expect("run syskits touch --date=1");

    assert!(
        output.status.success(),
        "stdout: {}, stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"");
    assert_eq!(output.stderr, b"");

    let stat = Command::new("stat")
        .env("TZ", "Asia/Shanghai")
        .args(["-c", "%y", file.to_str().unwrap()])
        .output()
        .expect("stat touched file");
    assert!(
        stat.status.success(),
        "stat stderr: {}",
        String::from_utf8_lossy(&stat.stderr)
    );
    assert!(
        String::from_utf8_lossy(&stat.stdout).contains(" 01:00:00."),
        "stat stdout: {}",
        String::from_utf8_lossy(&stat.stdout)
    );
}
