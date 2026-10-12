#[cfg(unix)]
#[test]
fn chcon_reports_a_dangling_symlink_like_gnu() {
    use std::os::unix::{fs::symlink, process::CommandExt};
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("tempdir");
    let link = temp_dir.path().join("link");
    symlink("missing", &link).expect("create dangling symlink");

    for (locale, quoted_context) in [("C", "'context'"), ("C.UTF-8", "‘context’")] {
        let output = Command::new(env!("CARGO_BIN_EXE_chcon"))
            .env("LC_ALL", locale)
            .arg0("chcon")
            .arg("context")
            .arg(&link)
            .output()
            .expect("run chcon");

        assert_eq!(output.status.code(), Some(1), "locale {locale}");
        assert_eq!(
            String::from_utf8(output.stderr).expect("stderr is UTF-8"),
            format!(
                "chcon: failed to change context of '{}' to {quoted_context}: No such file or directory\n",
                link.display()
            ),
            "locale {locale}"
        );
    }
}
