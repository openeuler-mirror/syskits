#[cfg(unix)]
#[test]
fn chgrp_accepts_ascii_whitespace_before_numeric_groups_like_gnu() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    for group in [" 0", "\t0", "\n0", "\r0", "\u{000B}0", "\u{000C}0"] {
        let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
            .env("LC_ALL", "C")
            .arg0("chgrp")
            .args([group, "/definitely-missing-chgrp-file"])
            .output()
            .expect("run chgrp");

        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(
            String::from_utf8(output.stderr).expect("stderr is UTF-8"),
            "chgrp: cannot access '/definitely-missing-chgrp-file': No such file or directory\n"
        );
    }
}
