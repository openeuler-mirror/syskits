use std::process::Command;

#[test]
fn tsort_preserves_cycle_diagnostic_output_order() {
    let output = Command::new("bash")
        .args([
            "-c",
            "printf 'a b\\nb a\\n' | \"$1\" tsort 2>&1",
            "bash",
            env!("CARGO_BIN_EXE_syskits"),
        ])
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .output()
        .expect("run syskits tsort");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    assert_eq!(
        output.stdout,
        b"tsort: -: input contains a loop:\ntsort: a\ntsort: b\na\nb\n"
    );
}
