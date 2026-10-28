#[cfg(unix)]
mod tests {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    fn ptx_command(locale: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ptx"));
        command
            .arg0("ptx")
            .env("LC_ALL", locale)
            .env_remove("LANGUAGE");
        command
    }

    #[test]
    fn missing_file_uses_locale_strerror_bytes() {
        let output = ptx_command("zh_CN.gbk")
            .arg(OsString::from_vec(vec![0xd6, 0xd0]))
            .output()
            .expect("run ptx");

        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(
            output.stderr,
            b"ptx: \xd6\xd0: \xc3\xbb\xd3\xd0\xc4\xc7\xb8\xf6\xce\xc4\xbc\xfe\xbb\xf2\xc4\xbf\xc2\xbc\n"
        );
    }

    #[test]
    fn invalid_regex_uses_locale_gettext_bytes() {
        let directory = tempfile::tempdir().expect("create temporary directory");
        std::fs::write(directory.path().join("input"), b"alpha beta\n").expect("write input");

        let output = ptx_command("zh_CN.gbk")
            .current_dir(directory.path())
            .arg("-W")
            .arg(OsString::from_vec(vec![0xd6, 0xd0, b'[']))
            .arg("input")
            .output()
            .expect("run ptx");

        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(
            output.stderr,
            b"ptx: \xce\xde\xd0\xa7\xb5\xc4\xb3\xa3\xb9\xe6\xb1\xed\xb4\xef\xca\xbd\xa3\xa8\xd5\xfd\xd4\xf2\xb1\xed\xb4\xef\xca\xbd \"\xd6\xd0[\"\xa3\xa9\n"
        );
    }
}
