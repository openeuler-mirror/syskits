use std::ffi::OsString;
use std::os::unix::{ffi::OsStringExt, process::CommandExt};
use std::path::PathBuf;
use std::process::Command;

#[test]
fn missing_path_diagnostics_use_gnu_shell_escaping_for_control_characters() {
    let temporary_directory = tempfile::tempdir().expect("create temporary directory");
    let prefix = temporary_directory
        .path()
        .join("missing")
        .to_string_lossy()
        .into_owned();
    let cases = [
        ('\t', "\\t"),
        ('\n', "\\n"),
        ('\x07', "\\a"),
        ('\x7f', "\\177"),
    ];

    for (control, escaped) in cases {
        let path = PathBuf::from(format!("{prefix}{control}path"));
        let output = Command::new(env!("CARGO_BIN_EXE_df"))
            .arg0("df")
            .arg(&path)
            .env("LC_ALL", "C")
            .output()
            .expect("run df");

        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(
            String::from_utf8(output.stderr).expect("df stderr is UTF-8"),
            format!("df: '{prefix}'$'{escaped}''path': No such file or directory\n")
        );
    }
}

#[test]
fn non_utf8_path_is_accepted_as_an_operand() {
    let temporary_directory = tempfile::tempdir().expect("create temporary directory");
    let mut path = temporary_directory
        .path()
        .as_os_str()
        .as_encoded_bytes()
        .to_vec();
    path.extend_from_slice(b"/non-utf8-\xf3");
    let path = OsString::from_vec(path);

    let output = Command::new(env!("CARGO_BIN_EXE_df"))
        .arg0("df")
        .arg(&path)
        .env("LC_ALL", "C")
        .output()
        .expect("run df");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.starts_with(b"error: invalid UTF-8"));
    assert!(output.stderr.ends_with(b": No such file or directory\n"));
}
