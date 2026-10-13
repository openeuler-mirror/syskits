#[cfg(target_os = "linux")]
#[test]
fn non_utf8_suffix_format_creates_output_files_with_the_original_bytes() {
    use std::collections::BTreeSet;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    let suffix_format = OsString::from_vec(b"\xff%d".to_vec());
    std::fs::write(temp_dir.path().join("input"), b"a\nb\n").expect("write input");
    let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
        .args(["-q", "-b"])
        .arg(suffix_format)
        .args(["input", "1"])
        .current_dir(temp_dir.path())
        .output()
        .expect("run csplit");

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");

    let names = std::fs::read_dir(temp_dir.path())
        .expect("read output directory")
        .map(|entry| entry.expect("read entry").file_name().into_vec())
        .filter(|name| name.as_slice() != b"input")
        .collect::<BTreeSet<_>>();
    assert_eq!(
        names,
        BTreeSet::from([b"xx\xff0".to_vec(), b"xx\xff1".to_vec()])
    );
}
