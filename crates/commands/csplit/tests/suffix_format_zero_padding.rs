#[cfg(target_os = "linux")]
#[test]
fn alternate_hex_prefix_precedes_zero_width_padding() {
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    std::fs::write(temp_dir.path().join("input"), b"a\nb\n").expect("write input");

    let output = Command::new("bash")
        .args(["-c", "exec -a csplit \"$0\" \"$@\""])
        .arg(env!("CARGO_BIN_EXE_csplit"))
        .args(["-q", "-b", "%#06x", "input", "/./", "{*}"])
        .current_dir(temp_dir.path())
        .output()
        .expect("run csplit");

    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let mut split_names = std::fs::read_dir(temp_dir.path())
        .expect("read output directory")
        .map(|entry| entry.expect("read output entry").file_name())
        .filter(|name| name.as_encoded_bytes().starts_with(b"xx"))
        .collect::<Vec<_>>();
    split_names.sort();
    assert_eq!(
        split_names,
        ["xx000000", "xx0x0001", "xx0x0002"].map(std::ffi::OsString::from)
    );
    assert_eq!(
        std::fs::read(temp_dir.path().join("xx0x0001")).expect("read first split"),
        b"a\n"
    );
    assert_eq!(
        std::fs::read(temp_dir.path().join("xx0x0002")).expect("read second split"),
        b"b\n"
    );
}
