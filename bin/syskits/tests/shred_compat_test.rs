use std::fs;
use std::process::Command;

use tempfile::TempDir;

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

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

#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
struct FileState {
    mode: u32,
    ctime: i64,
    len: u64,
    blocks: u64,
    contents: Vec<u8>,
}

#[cfg(unix)]
fn file_state(path: &std::path::Path) -> FileState {
    let metadata = fs::metadata(path).expect("metadata");
    FileState {
        mode: metadata.permissions().mode() & 0o7777,
        ctime: metadata.ctime(),
        len: metadata.len(),
        blocks: metadata.blocks(),
        contents: fs::read(path).expect("read file"),
    }
}

#[cfg(unix)]
fn write_readonly_fixture(path: &std::path::Path) {
    fs::write(path, b"readonly").expect("write fixture");
    let mut perms = fs::metadata(path).expect("metadata").permissions();
    perms.set_mode(0o400);
    fs::set_permissions(path, perms).expect("chmod fixture");
}

#[cfg(unix)]
#[test]
fn shred_force_zero_iterations_does_not_change_metadata_unnecessarily() {
    let temp_dir = TempDir::new().expect("tempdir");
    let core_file = temp_dir.path().join("core.bin");
    let syskits_file = temp_dir.path().join("syskits.bin");
    write_readonly_fixture(&core_file);
    write_readonly_fixture(&syskits_file);

    let core_before = file_state(&core_file);
    std::thread::sleep(std::time::Duration::from_secs(1));
    let core = Command::new("/usr/bin/shred")
        .args(["-f", "-n", "0", core_file.to_str().expect("utf8 path")])
        .output()
        .expect("run GNU shred");
    let core_after = file_state(&core_file);

    let syskits_before = file_state(&syskits_file);
    std::thread::sleep(std::time::Duration::from_secs(1));
    let syskits = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args([
            "shred",
            "-f",
            "-n",
            "0",
            syskits_file.to_str().expect("utf8 path"),
        ])
        .output()
        .expect("run syskits shred");
    let syskits_after = file_state(&syskits_file);

    assert_eq!(syskits.status.code(), core.status.code());
    assert_eq!(syskits.stdout, core.stdout);
    assert_eq!(syskits.stderr, core.stderr);
    assert_eq!(
        syskits_after.mode == syskits_before.mode,
        core_after.mode == core_before.mode
    );
    assert_eq!(
        syskits_after.ctime == syskits_before.ctime,
        core_after.ctime == core_before.ctime
    );
    assert_eq!(syskits_after.mode, core_after.mode);
    assert_eq!(syskits_after.contents, syskits_before.contents);
    assert_eq!(core_after.contents, core_before.contents);
}

#[cfg(unix)]
#[test]
fn shred_default_rounds_to_filesystem_block_size() {
    let temp_dir = TempDir::new().expect("tempdir");
    let core_file = temp_dir.path().join("core.bin");
    let syskits_file = temp_dir.path().join("syskits.bin");
    fs::write(&core_file, b"0123456789abcdef").expect("write GNU fixture");
    fs::write(&syskits_file, b"0123456789abcdef").expect("write syskits fixture");

    let core = Command::new("/usr/bin/shred")
        .args([
            "-n",
            "1",
            "--random-source=/dev/zero",
            core_file.to_str().expect("utf8 path"),
        ])
        .output()
        .expect("run GNU shred");
    let syskits = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args([
            "shred",
            "-n",
            "1",
            "--random-source=/dev/zero",
            syskits_file.to_str().expect("utf8 path"),
        ])
        .output()
        .expect("run syskits shred");

    let core_after = file_state(&core_file);
    let syskits_after = file_state(&syskits_file);

    assert_eq!(syskits.status.code(), core.status.code());
    assert_eq!(syskits.stdout, core.stdout);
    assert_eq!(syskits.stderr, core.stderr);
    assert_eq!(syskits_after.mode, core_after.mode);
    assert_eq!(syskits_after.len, core_after.len);
    assert_eq!(syskits_after.blocks, core_after.blocks);
    assert_eq!(syskits_after.contents, core_after.contents);
}
