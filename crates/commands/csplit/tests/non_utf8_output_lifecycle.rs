#[cfg(target_os = "linux")]
#[test]
fn non_utf8_output_names_remain_raw_through_the_writer_lifecycle() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::symlink;
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let write_error_case = tempfile::tempdir().expect("create temporary directory");
    std::fs::write(write_error_case.path().join("input"), b"a\nb\n").expect("write input");
    let prefix = OsString::from_vec(b"\xff".to_vec());
    symlink(
        "/dev/full",
        write_error_case
            .path()
            .join(OsString::from_vec(b"\xff00".to_vec())),
    )
    .expect("link output to /dev/full");

    let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
        .arg0("csplit")
        .args(["-f"])
        .arg(&prefix)
        .args(["input", "2"])
        .env("LC_ALL", "C")
        .current_dir(write_error_case.path())
        .output()
        .expect("run csplit with write error");

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert_eq!(
        output.stderr, b"csplit: ''$'\\377''00': No space left on device\n",
        "{output:?}"
    );

    let elide_case = tempfile::tempdir().expect("create temporary directory");
    std::fs::write(elide_case.path().join("input"), b"a\n").expect("write input");
    let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
        .arg0("csplit")
        .args(["-z", "-f"])
        .arg(prefix)
        .args(["input", "1"])
        .env("LC_ALL", "C")
        .current_dir(elide_case.path())
        .output()
        .expect("run csplit with elided empty split");

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, b"2\n", "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        std::fs::read(
            elide_case
                .path()
                .join(OsString::from_vec(b"\xff00".to_vec()))
        )
        .expect("read output split"),
        b"a\n"
    );
}
