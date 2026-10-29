use std::fs;
#[cfg(target_os = "linux")]
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

#[test]
fn vdir_sort_size_breaks_equal_sizes_by_name() {
    let temp_dir = TempDir::new().expect("tempdir");
    for name in ["c-file", "b-file", "a-file"] {
        fs::write(temp_dir.path().join(name), b"same").expect("write fixture");
    }

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args([
            "vdir",
            "--sort=size",
            temp_dir.path().to_str().expect("utf8 path"),
        ])
        .output()
        .expect("run syskits vdir");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stderr, b"");

    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    let listed_names = stdout
        .lines()
        .filter(|line| !line.starts_with("total "))
        .filter_map(|line| line.split_whitespace().last())
        .collect::<Vec<_>>();

    assert_eq!(listed_names, ["a-file", "b-file", "c-file"]);
}

#[test]
fn vdir_long_omits_acl_context_marker_column_when_no_entry_needs_it() {
    let temp_dir = TempDir::new().expect("tempdir");
    let file = temp_dir.path().join("plain");
    fs::write(&file, b"").expect("write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["vdir", "-a", temp_dir.path().to_str().expect("utf8 path")])
        .output()
        .expect("run syskits vdir");

    let gnu_output = Command::new("/usr/bin/vdir")
        .args(["-a", temp_dir.path().to_str().expect("utf8 path")])
        .output()
        .expect("run GNU vdir");
    assert!(
        gnu_output.status.success(),
        "GNU vdir stderr: {}",
        String::from_utf8_lossy(&gnu_output.stderr)
    );

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stderr, b"");

    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    let plain_line = stdout
        .lines()
        .find(|line| line.ends_with(" plain"))
        .expect("plain file line");
    let gnu_stdout = String::from_utf8(gnu_output.stdout).expect("GNU vdir stdout is UTF-8");
    let gnu_plain_line = gnu_stdout
        .lines()
        .find(|line| line.ends_with(" plain"))
        .expect("GNU vdir plain file line");

    assert_eq!(
        plain_line.as_bytes().get(10..13),
        gnu_plain_line.as_bytes().get(10..13),
        "syskits vdir should use the same ACL/context marker spacing as GNU vdir:\n  syskits: {plain_line:?}\n  GNU: {gnu_plain_line:?}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn vdir_long_keeps_acl_context_marker_column_when_any_entry_needs_it() {
    let temp_dir = TempDir::new().expect("tempdir");
    let with_context = temp_dir.path().join("with-context");
    let plain = temp_dir.path().join("plain");
    fs::write(&with_context, b"").expect("write fixture");
    fs::write(&plain, b"").expect("write fixture");

    if !set_selinux_xattr(&with_context, b"system_u:object_r:tmp_t:s0\0") {
        eprintln!("skipping: current filesystem does not allow setting security.selinux");
        return;
    }

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["vdir", "-a", temp_dir.path().to_str().expect("utf8 path")])
        .output()
        .expect("run syskits vdir");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stderr, b"");

    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    let context_line = stdout
        .lines()
        .find(|line| line.ends_with(" with-context"))
        .expect("context file line");
    let plain_line = stdout
        .lines()
        .find(|line| line.ends_with(" plain"))
        .expect("plain file line");

    assert_eq!(
        context_line.as_bytes().get(10),
        Some(&b'.'),
        "context file line should show a context marker: {context_line:?}"
    );
    assert_eq!(
        plain_line.as_bytes().get(10),
        Some(&b' '),
        "plain file line should reserve an empty ACL/context marker column: {plain_line:?}"
    );
    assert_eq!(
        plain_line.as_bytes().get(11),
        Some(&b' '),
        "plain file line should include the separator after the marker column: {plain_line:?}"
    );
}
