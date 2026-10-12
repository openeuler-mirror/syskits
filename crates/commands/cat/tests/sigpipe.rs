#[cfg(target_os = "linux")]
#[test]
fn cat_terminates_on_a_closed_stdout_pipe() {
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("tempdir");
    let input = temp_dir.path().join("large-input");
    std::fs::write(&input, vec![b'x'; 8 * 1024 * 1024]).expect("write input");

    for option in [None, Some("-T")] {
        let mut command = Command::new("bash");
        command
            .arg("-o")
            .arg("pipefail")
            .arg("-c")
            .arg(
                "if [ \"$2\" = --no-option ]; then \"$1\" \"$3\"; else \"$1\" \"$2\" \"$3\"; fi | head -c 1 > /dev/null",
            )
            .arg("sigpipe-test")
            .arg(env!("CARGO_BIN_EXE_cat"));

        if let Some(option) = option {
            command.arg(option);
        } else {
            command.arg("--no-option");
        }

        let status = command.arg(&input).status().expect("run cat pipeline");
        assert_eq!(status.code(), Some(141), "option: {option:?}");
    }
}
