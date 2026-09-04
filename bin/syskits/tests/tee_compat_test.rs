#[cfg(unix)]
use std::process::{Command, Stdio};

#[cfg(unix)]
#[test]
fn tee_exit_mode_does_not_write_a_block_after_stdout_breaks() {
    let temp_dir = tempfile::TempDir::new().expect("tempdir");
    let output_file = temp_dir.path().join("out");

    let (input_read, input_write) = nix::unistd::pipe().expect("input pipe");
    let payload = [b'x'; 4096];
    assert_eq!(
        nix::unistd::write(&input_write, &payload).expect("write input pipe"),
        payload.len()
    );
    drop(input_write);

    let (output_read, output_write) = nix::unistd::pipe().expect("output pipe");
    drop(output_read);

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args([
            "tee",
            "--output-error=exit",
            output_file.to_str().expect("utf8 path"),
        ])
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(Stdio::from(input_read))
        .stdout(Stdio::from(output_write))
        .stderr(Stdio::piped())
        .output()
        .expect("run syskits tee");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"tee: 'standard output': Broken pipe\n");
    assert_eq!(
        std::fs::metadata(output_file)
            .expect("output metadata")
            .len(),
        0
    );
}
