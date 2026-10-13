use std::os::unix::process::CommandExt;
use std::process::Command;

#[test]
fn type_options_accept_hyphen_values_like_gnu_df() {
    for arguments in [["-t", "-V"], ["-F", "-V"], ["--type", "-V"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_df"))
            .arg0("df")
            .args(arguments)
            .env("LC_ALL", "C")
            .output()
            .unwrap();

        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            "df: no file systems processed\n"
        );
    }
}
