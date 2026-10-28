use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::process::Command;

use tempfile::TempDir;

#[cfg(target_os = "linux")]
fn set_selinux_xattr(path: &std::path::Path, value: &[u8]) -> bool {
    let Ok(c_path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };

    unsafe {
        libc::setxattr(
            c_path.as_ptr(),
            c"security.selinux".as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
        ) == 0
    }
}

#[cfg(target_os = "linux")]
#[test]
fn dir_long_shows_selinux_context_marker_from_xattr() {
    let temp_dir = TempDir::new().expect("tempdir");
    let file = temp_dir.path().join("with-context");
    fs::write(&file, b"content").expect("write fixture");

    if !set_selinux_xattr(&file, b"system_u:object_r:tmp_t:s0\0") {
        eprintln!("skipping: current filesystem does not allow setting security.selinux");
        return;
    }

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["dir", "-l", file.to_str().expect("utf8 path")])
        .output()
        .expect("run syskits dir");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    let mode = stdout
        .split_whitespace()
        .next()
        .expect("long listing mode field");
    assert_eq!(mode, "-rw-r--r--.");
}
