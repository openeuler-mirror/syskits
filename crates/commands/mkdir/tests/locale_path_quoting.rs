#[cfg(unix)]
#[test]
fn non_utf8_path_diagnostics_follow_locale_quote_rules() {
    use std::ffi::OsString;
    use std::os::unix::{ffi::OsStringExt, process::CommandExt};
    use std::process::Command;

    let path = OsString::from_vec(vec![0xff, b'/', b'c', b'h', b'i', b'l', b'd']);
    let cases = [
        (
            "C",
            b"mkdir: cannot create directory '\\377/child': No such file or directory\n".as_slice(),
        ),
        (
            "C.UTF-8",
            "mkdir: cannot create directory ‘\\377/child’: No such file or directory\n".as_bytes(),
        ),
    ];

    for (locale, expected_stderr) in cases {
        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        let output = Command::new(env!("CARGO_BIN_EXE_mkdir"))
            .arg0("mkdir")
            .current_dir(temp_dir.path())
            .env("LC_ALL", locale)
            .arg(&path)
            .output()
            .expect("run mkdir with a non-UTF-8 parent path");

        assert_eq!(output.status.code(), Some(1), "{locale}");
        assert!(output.stdout.is_empty(), "{locale}");
        assert_eq!(output.stderr, expected_stderr, "{locale}");
    }
}
