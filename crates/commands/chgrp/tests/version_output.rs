#[cfg(unix)]
#[test]
fn chgrp_version_output_uses_syskits_information() {
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
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    assert_eq!(
        stdout,
        format!(
            r#"chgrp (syskits) {}
Copyright (C) 2022-2025 China Telecom Cloud Technologies Co., Ltd.
License Mulan PSL v2: <http://license.coscl.org.cn/MulanPSL2>.
This is free software: you are free to change and redistribute it.
There is NO WARRANTY, to the extent permitted by law.
"#,
            env!("CARGO_PKG_VERSION")
        )
    );
    assert!(!stdout.contains("GNU"));
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
