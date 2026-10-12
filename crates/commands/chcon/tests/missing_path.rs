#[cfg(unix)]
#[test]
fn chcon_reports_a_missing_path_like_gnu() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("tempdir");
    let missing = temp_dir.path().join("missing");
    for locale in ["C", "C.UTF-8"] {
        let output = Command::new(env!("CARGO_BIN_EXE_chcon"))
            .env("LC_ALL", locale)
            .arg0("chcon")
            .args(["context"])
            .arg(&missing)
            .output()
            .expect("run chcon");

        assert_eq!(output.status.code(), Some(1), "locale {locale}");
        assert_eq!(
            String::from_utf8(output.stderr).expect("stderr is UTF-8"),
            format!(
                "chcon: cannot access '{}': No such file or directory\n",
                missing.display()
            ),
            "locale {locale}"
        );
    }
}
