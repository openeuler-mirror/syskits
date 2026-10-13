use std::os::unix::process::CommandExt;
use std::process::Command;

#[test]
fn block_size_values_are_validated_in_argument_order() {
    let cases = [
        (
            vec!["-Bfoo", "--block-size=1K"],
            "df: invalid -B argument 'foo'\n",
        ),
        (
            vec!["--block-size=foo", "-B1K"],
            "df: invalid --block-size argument 'foo'\n",
        ),
        (vec!["-Bfoo", "-k"], "df: invalid -B argument 'foo'\n"),
        (vec!["-B1K", "-Bfoo"], "df: invalid -B argument 'foo'\n"),
    ];

    for (arguments, expected_stderr) in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_df"))
            .arg0("df")
            .args(arguments)
            .env("LC_ALL", "C")
            .output()
            .unwrap();

        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(String::from_utf8(output.stderr).unwrap(), expected_stderr);
    }
}
