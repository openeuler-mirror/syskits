use std::process::Command;

fn run_syskits(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(args)
        .output()
        .unwrap_or_else(|err| panic!("run syskits {args:?}: {err}"))
}

#[test]
fn test_length_operand_in_integer_comparisons() {
    for args in [
        &["[", "-l", "1", "-eq", "1", "]"][..],
        &["test", "-l", "abc", "-eq", "3"][..],
        &["[", "3", "-eq", "-l", "abc", "]"][..],
    ] {
        let output = run_syskits(args);
        assert!(
            output.status.success(),
            "expected success for {args:?}; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.stdout.is_empty(),
            "unexpected stdout for {args:?}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(
            output.stderr.is_empty(),
            "unexpected stderr for {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn test_length_operand_false_integer_comparison_exits_one() {
    let output = run_syskits(&["[", "-l", "ab", "-eq", "3", "]"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
