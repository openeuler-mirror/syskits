#![cfg(unix)]

use std::os::unix::process::CommandExt;
use std::process::Command;

#[test]
fn invalid_update_value_diagnostic_matches_gnu() {
    for value in ["bad", "older=bad"] {
        let output = Command::new(env!("CARGO_BIN_EXE_mv"))
            .arg0("mv")
            .env("LC_ALL", "C")
            .arg(format!("--update={value}"))
            .output()
            .expect("run mv");

        assert_eq!(output.status.code(), Some(1), "{value}");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            format!(
                "mv: invalid argument '{value}' for '--update'\nValid arguments are:\n  - 'all'\n  - 'none'\n  - 'older'\nTry 'mv --help' for more information.\n"
            ),
            "{value}"
        );
    }
}
