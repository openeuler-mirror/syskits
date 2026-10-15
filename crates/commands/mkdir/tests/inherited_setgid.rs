#[cfg(unix)]
#[test]
fn explicit_mode_preserves_only_unmentioned_inherited_setgid() {
    use std::os::unix::{fs::PermissionsExt, process::CommandExt};
    use std::process::Command;

    for (mode, expected_mode) in [
        ("755", 0o2755),
        ("00755", 0o0755),
        ("1755", 0o3755),
        ("01755", 0o1755),
        ("g=rw", 0o2767),
    ] {
        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        let parent = temp_dir.path().join("parent");
        std::fs::create_dir(&parent).expect("create setgid parent directory");
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o2775))
            .expect("set parent setgid bit");

        let output = Command::new(env!("CARGO_BIN_EXE_mkdir"))
            .arg0("mkdir")
            .current_dir(&parent)
            .env("LC_ALL", "C")
            .args(["-m", mode, "child"])
            .output()
            .expect("run mkdir below a setgid directory");

        assert_eq!(output.status.code(), Some(0), "{mode}");
        assert!(output.stdout.is_empty(), "{mode}");
        assert!(output.stderr.is_empty(), "{mode}");
        assert_eq!(
            std::fs::metadata(parent.join("child"))
                .expect("read created directory metadata")
                .permissions()
                .mode()
                & 0o7777,
            expected_mode,
            "{mode}"
        );
    }
}
