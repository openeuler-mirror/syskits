#[cfg(unix)]
#[test]
fn chgrp_treats_dash_prefixed_reference_values_like_gnu() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    for (args, stderr) in [
        (
            vec!["--reference", "--", "/tmp"],
            "chgrp: failed to get attributes of '--': No such file or directory\n",
        ),
        (
            vec!["--reference", "--verbose"],
            "chgrp: missing operand\nTry 'chgrp --help' for more information.\n",
        ),
        (
            vec!["--reference", "-v", "/tmp"],
            "chgrp: failed to get attributes of '-v': No such file or directory\n",
        ),
        (
            vec!["--ref", "--reference", "/tmp"],
            "chgrp: failed to get attributes of '--reference': No such file or directory\n",
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
            .env("LC_ALL", "C")
            .arg0("chgrp")
            .args(args)
            .output()
            .expect("run chgrp");

        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(
            String::from_utf8(output.stderr).expect("stderr is UTF-8"),
            stderr
        );
    }
}
