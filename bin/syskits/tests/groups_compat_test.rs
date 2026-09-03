#[cfg(feature = "groups")]
use std::process::Command;

#[cfg(feature = "groups")]
#[test]
fn groups_help_and_version_exit_success() {
    for option in ["--help", "--version"] {
        let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .args(["groups", option])
            .output()
            .expect("run syskits groups");

        assert!(
            output.status.success(),
            "{option} status: {:?}, stdout: {}, stderr: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.stderr.is_empty(),
            "{option} stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!output.stdout.is_empty(), "{option} should write stdout");
    }
}
