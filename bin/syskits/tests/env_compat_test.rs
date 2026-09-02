use std::process::Command;

#[test]
fn env_verbose_without_changes_does_not_dump_input_args() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["env", "-v"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run syskits env -v");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"PATH=/usr/bin:/bin\n");
    assert_eq!(output.stderr, b"");
}
