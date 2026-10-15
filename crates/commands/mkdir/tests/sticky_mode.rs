#[cfg(unix)]
#[test]
fn symbolic_sticky_modes_preserve_gnu_write_bit_behavior() {
    use std::os::unix::{fs::PermissionsExt, process::CommandExt};
    use std::process::Command;

    for (mode, directory, expected_mode) in [
        ("+t", "implicit", 0o1755),
        ("a+t", "all", 0o1755),
        ("o+t", "other", 0o1755),
        ("a+t,g+w", "group-write", 0o1777),
        ("1777", "numeric", 0o1777),
    ] {
        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        let output = Command::new(env!("CARGO_BIN_EXE_mkdir"))
            .arg0("mkdir")
            .current_dir(temp_dir.path())
            .env("LC_ALL", "C")
            .args(["-m", mode, directory])
            .output()
            .expect("run mkdir with a sticky symbolic mode");

        assert_eq!(output.status.code(), Some(0), "{mode}");
        assert!(output.stdout.is_empty(), "{mode}");
        assert!(output.stderr.is_empty(), "{mode}");
        assert_eq!(
            std::fs::metadata(temp_dir.path().join(directory))
                .expect("read created directory metadata")
                .permissions()
                .mode()
                & 0o7777,
            expected_mode,
            "{mode}"
        );
    }
}
