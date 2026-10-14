use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::process::Command;

fn run_df(arguments: &[&str], path: &str) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_df"));
    command
        .arg0("df")
        .args(arguments)
        .arg(path)
        .env("LC_ALL", "C");

    if ctcore::ct_process::geteuid() == 0 {
        command.uid(65_534).gid(65_534);
    }

    command.output().expect("run df")
}

#[test]
fn inaccessible_path_preserves_the_stat_error() {
    let temporary_directory = tempfile::tempdir().expect("create temporary directory");
    std::fs::set_permissions(
        temporary_directory.path(),
        std::fs::Permissions::from_mode(0o755),
    )
    .expect("make temporary directory traversable");
    let private_directory = temporary_directory.path().join("private");
    std::fs::create_dir(&private_directory).expect("create private directory");
    let path = private_directory.join("file");
    std::fs::write(&path, b"").expect("create file");
    std::fs::set_permissions(&private_directory, std::fs::Permissions::from_mode(0o000))
        .expect("make private directory inaccessible");
    let path = path.to_string_lossy().into_owned();

    for arguments in [&[][..], &["--direct"][..]] {
        let output = run_df(arguments, &path);

        assert_eq!(output.status.code(), Some(1), "{arguments:?}");
        assert!(output.stdout.is_empty(), "{arguments:?}");
        assert_eq!(
            String::from_utf8(output.stderr).expect("df stderr is UTF-8"),
            format!("df: {path}: Permission denied\n"),
            "{arguments:?}"
        );
    }
}
