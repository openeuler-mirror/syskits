#[cfg(unix)]
#[test]
fn chgrp_quotes_invalid_group_names_like_gnu() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    for (locale, group, quoted_group) in [
        ("C", "definitely-not-a-group", "'definitely-not-a-group'"),
        ("C", "invalid\tgroup", "'invalid\\tgroup'"),
        (
            "C",
            "invalid中文group",
            "'invalid\\344\\270\\255\\346\\226\\207group'",
        ),
        (
            "C.UTF-8",
            "definitely-not-a-group",
            "‘definitely-not-a-group’",
        ),
        ("C.UTF-8", "invalid\tgroup", "‘invalid\\tgroup’"),
        ("C.UTF-8", "invalid中文group", "‘invalid中文group’"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
            .env("LC_ALL", locale)
            .arg0("chgrp")
            .args([group, "/tmp"])
            .output()
            .expect("run chgrp");

        assert_eq!(output.status.code(), Some(1), "locale {locale}");
        assert_eq!(
            String::from_utf8(output.stderr).expect("stderr is UTF-8"),
            format!("chgrp: invalid group: {quoted_group}\n"),
            "locale {locale}"
        );
    }
}
