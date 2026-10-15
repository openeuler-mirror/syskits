#[cfg(unix)]
#[test]
fn repeated_mode_options_use_the_last_value() {
    use std::os::unix::{fs::PermissionsExt, process::CommandExt};
    use std::process::Command;

    for (arguments, fifo_name) in [
        (vec!["-m", "600", "-m", "700", "first"], "first"),
        (vec!["--mode=600", "--mode=700", "second"], "second"),
        (vec!["-m", "-w", "-m", "700", "third"], "third"),
        (vec!["--mode", "-w", "--mode=700", "fourth"], "fourth"),
    ] {
        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        let output = Command::new(env!("CARGO_BIN_EXE_mkfifo"))
            .arg0("mkfifo")
            .current_dir(temp_dir.path())
            .env("LC_ALL", "C")
            .args(arguments)
            .output()
            .expect("run mkfifo with repeated mode options");

        assert_eq!(output.status.code(), Some(0), "{fifo_name}");
        assert!(output.stdout.is_empty(), "{fifo_name}");
        assert!(output.stderr.is_empty(), "{fifo_name}");
        assert_eq!(
            std::fs::metadata(temp_dir.path().join(fifo_name))
                .expect("read FIFO metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700,
            "{fifo_name}"
        );
    }
}
