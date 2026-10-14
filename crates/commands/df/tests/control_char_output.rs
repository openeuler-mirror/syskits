use std::os::unix::process::CommandExt;
use std::process::Command;

#[test]
fn direct_output_replaces_control_characters_in_path_fields() {
    let temporary_directory = tempfile::tempdir().expect("create temporary directory");
    let path = temporary_directory.path().join("line\nbreak");
    std::fs::write(&path, b"").expect("create file with a newline in its name");

    let output = Command::new(env!("CARGO_BIN_EXE_df"))
        .arg0("df")
        .args(["--direct", "--output=file,target"])
        .arg(&path)
        .env("LC_ALL", "C")
        .output()
        .expect("run df");

    assert!(output.status.success());

    let sanitized_path = path.to_string_lossy().replace('\n', "?");
    let width = sanitized_path.len();
    assert_eq!(
        String::from_utf8(output.stdout).expect("df stdout is UTF-8"),
        format!(
            "{:<width$} File\n{sanitized_path} {sanitized_path}\n",
            "File"
        )
    );
}
