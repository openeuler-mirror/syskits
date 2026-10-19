use std::fs;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::process::{Command, Output};

use tempfile::TempDir;

fn assert_same_output(case: &str, gnu: Output, syskits: Output) {
    assert_eq!(
        syskits.status.code(),
        gnu.status.code(),
        "{case}: GNU stderr: {}, syskits stderr: {}",
        String::from_utf8_lossy(&gnu.stderr),
        String::from_utf8_lossy(&syskits.stderr)
    );
    assert_eq!(
        syskits.stdout, gnu.stdout,
        "{case}: GNU stdout: {:02x?}, syskits stdout: {:02x?}",
        gnu.stdout, syskits.stdout
    );
    assert_eq!(
        syskits.stderr,
        gnu.stderr,
        "{case}: GNU stderr: {}, syskits stderr: {}",
        String::from_utf8_lossy(&gnu.stderr),
        String::from_utf8_lossy(&syskits.stderr)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn dir_preserves_non_utf8_invalid_option_byte() {
    let invalid_option = std::ffi::OsString::from_vec(b"-\xff".to_vec());
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C")
        .args([std::ffi::OsStr::new("dir"), invalid_option.as_os_str()])
        .output()
        .expect("run syskits dir");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(output.stdout, b"");
    assert_eq!(
        output.stderr,
        b"dir: invalid option -- '\xff'\nTry 'dir --help' for more information.\n"
    );
}

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

#[test]
fn dir_f_color_strictly_fits_columns_like_gnu() {
    let temp_dir = TempDir::new().expect("tempdir");
    let tree = temp_dir.path().join("tree");
    fs::create_dir_all(tree.join("sub")).expect("create fixture");
    let tree_arg = tree.to_str().expect("utf8 path");

    let gnu = Command::new("/usr/bin/dir")
        .env("COLUMNS", "10")
        .env("LC_ALL", "C")
        .env("TERM", "xterm")
        .env("LS_COLORS", "di=01;34")
        .args(["-f", "--color=always", "--sort=size", tree_arg])
        .output()
        .expect("run GNU dir");
    let syskits = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("COLUMNS", "10")
        .env("LC_ALL", "C")
        .env("TERM", "xterm")
        .env("LS_COLORS", "di=01;34")
        .args(["dir", "-f", "--color=always", "--sort=size", tree_arg])
        .output()
        .expect("run syskits dir");

    assert_same_output(
        "dir -f --color=always --sort=size narrow columns",
        gnu,
        syskits,
    );
}
