#[cfg(target_os = "linux")]
#[test]
fn non_utf8_prefix_creates_output_files_with_the_original_bytes() {
    use std::collections::BTreeSet;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    let prefix = OsString::from_vec(vec![0xff]);
    std::fs::write(temp_dir.path().join("input"), b"a\nb\n").expect("write input");
    let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
        .args(["-q", "-f"])
        .arg(prefix)
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
        BTreeSet::from([b"\xff00".to_vec(), b"\xff01".to_vec()])
    );
}
