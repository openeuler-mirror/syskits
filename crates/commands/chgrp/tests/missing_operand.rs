#[cfg(unix)]
#[test]
fn chgrp_reports_missing_operands_like_gnu() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    for (locale, quoted_group) in [("C", "'root'"), ("C.UTF-8", "‘root’")] {
        let cases = [
            (Vec::new(), "missing operand".to_string()),
            (
                vec!["root"],
                format!("missing operand after {quoted_group}"),
            ),
            (vec!["--reference=/tmp"], "missing operand".to_string()),
        ];

        for (args, message) in cases {
            let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
                .env("LC_ALL", locale)
                .arg0("chgrp")
                .args(args)
                .output()
                .expect("run chgrp");

            assert_eq!(output.status.code(), Some(1), "locale {locale}");
            assert_eq!(
                String::from_utf8(output.stderr).expect("stderr is UTF-8"),
                format!("chgrp: {message}\nTry 'chgrp --help' for more information.\n"),
                "locale {locale}"
            );
        }
    }
}
