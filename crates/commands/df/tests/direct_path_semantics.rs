use std::os::unix::fs::symlink;
use std::os::unix::process::CommandExt;
use std::process::Command;

fn run_direct_df(path: &std::path::Path) -> Vec<String> {
    let output = Command::new(env!("CARGO_BIN_EXE_df"))
        .arg0("df")
        .args(["--direct", "--output=source,fstype,file,target"])
        .arg(path)
        .env("LC_ALL", "C")
        .output()
        .expect("run df");

    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("df stdout is UTF-8")
        .lines()
        .nth(1)
        .expect("df output row")
        .split_ascii_whitespace()
        .map(str::to_string)
        .collect()
}

#[test]
fn direct_output_uses_unknown_source_and_type() {
    let temporary_directory = tempfile::tempdir().expect("create temporary directory");
    let path = temporary_directory.path().join("file");
    std::fs::write(&path, b"").expect("create file");
    let path = path.to_string_lossy().into_owned();

    assert_eq!(
        run_direct_df(std::path::Path::new(&path)),
        ["-", "-", path.as_str(), path.as_str()]
    );
}

#[test]
fn direct_output_keeps_file_operand_and_resolves_target() {
    let temporary_directory = tempfile::tempdir().expect("create temporary directory");
    let target = temporary_directory.path().join("target");
    let link = temporary_directory.path().join("link");
    std::fs::write(&target, b"").expect("create target file");
    symlink(&target, &link).expect("create symbolic link");
    let target = target.to_string_lossy().into_owned();
    let link = link.to_string_lossy().into_owned();

    assert_eq!(
        run_direct_df(std::path::Path::new(&link)),
        ["-", "-", link.as_str(), target.as_str()]
    );
}

#[test]
fn direct_missing_path_uses_missing_file_diagnostic() {
    let temporary_directory = tempfile::tempdir().expect("create temporary directory");
    let path = temporary_directory.path().join("missing");
    let output = Command::new(env!("CARGO_BIN_EXE_df"))
        .arg0("df")
        .args(["--direct", path.to_str().expect("path is UTF-8")])
        .env("LC_ALL", "C")
        .output()
        .expect("run df");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).expect("df stderr is UTF-8"),
        format!("df: {}: No such file or directory\n", path.display())
    );
}
