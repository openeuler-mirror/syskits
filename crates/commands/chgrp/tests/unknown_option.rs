#[cfg(unix)]
#[test]
fn chgrp_reports_unknown_options_like_gnu() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    for (args, message) in [
        (
            vec!["-1", "/definitely-missing-chgrp-file"],
            "invalid option -- '1'",
        ),
        (
            vec!["--definitely-unknown", "0", "/tmp"],
            "unrecognized option '--definitely-unknown'",
        ),
        (
            vec!["--from=0:0", "0", "/tmp"],
            "unrecognized option '--from=0:0'",
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
            .env("LC_ALL", "C")
            .arg0("chgrp")
            .args(args)
            .output()
            .expect("run chgrp");

        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            String::from_utf8(output.stderr).expect("stderr is UTF-8"),
            format!("chgrp: {message}\nTry 'chgrp --help' for more information.\n")
        );
    }
}
