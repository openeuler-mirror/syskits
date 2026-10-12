#[cfg(unix)]
#[test]
fn chgrp_version_output_matches_gnu_coreutils() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
        .env("LC_ALL", "C")
        .arg0("chgrp")
        .arg("--version")
        .output()
        .expect("run chgrp");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        r#"chgrp (GNU coreutils) 9.4
Copyright (C) 2023 Free Software Foundation, Inc.
License GPLv3+: GNU GPL version 3 or later <https://gnu.org/licenses/gpl.html>.
This is free software: you are free to change and redistribute it.
There is NO WARRANTY, to the extent permitted by law.

Written by David MacKenzie and Jim Meyering.
"#
    );
}

#[cfg(unix)]
#[test]
fn chgrp_version_does_not_override_an_earlier_parse_error() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    for args in [
        vec!["--definitely-unknown", "--version"],
        vec!["--reference", "--version"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
            .env("LC_ALL", "C")
            .arg0("chgrp")
            .args(args)
            .output()
            .expect("run chgrp");

        assert_eq!(output.status.code(), Some(1));
    }
}
