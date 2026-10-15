#[cfg(unix)]
#[test]
fn repeated_mode_options_use_the_last_value() {
    use std::os::unix::{fs::PermissionsExt, process::CommandExt};
    use std::process::Command;

    for (arguments, directory) in [
        (vec!["-m", "755", "-m", "700", "first"], "first"),
        (vec!["-m", "-x", "-m", "700", "second"], "second"),
        (vec!["--mode=-x", "--mode=700", "third"], "third"),
    ] {
        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        let output = Command::new(env!("CARGO_BIN_EXE_mkdir"))
            .arg0("mkdir")
            .current_dir(temp_dir.path())
            .env("LC_ALL", "C")
            .args(arguments)
            .output()
            .expect("run mkdir with repeated mode options");

        assert_eq!(output.status.code(), Some(0), "{directory}");
        assert!(output.stdout.is_empty(), "{directory}");
        assert!(output.stderr.is_empty(), "{directory}");
        assert_eq!(
            std::fs::metadata(temp_dir.path().join(directory))
                .expect("read created directory metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700,
            "{directory}"
        );
    }
}
