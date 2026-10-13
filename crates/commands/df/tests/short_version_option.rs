use std::os::unix::process::CommandExt;
use std::process::Command;

#[test]
fn short_version_option_is_rejected_like_gnu_df() {
    for locale in ["C", "C.UTF-8"] {
        for arguments in [["-V"], ["-Vh"], ["-hV"], ["-kV"]] {
            let output = Command::new(env!("CARGO_BIN_EXE_df"))
                .arg0("df")
                .args(arguments)
                .env("LC_ALL", locale)
                .output()
                .unwrap();

            assert_eq!(output.status.code(), Some(1));
            assert!(output.stdout.is_empty());
            assert_eq!(
                String::from_utf8(output.stderr).unwrap(),
                "df: invalid option -- 'V'\nTry 'df --help' for more information.\n"
            );
        }
    }
}
