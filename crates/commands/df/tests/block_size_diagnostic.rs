use std::os::unix::process::CommandExt;
use std::process::Command;

#[test]
fn block_size_diagnostics_preserve_the_option_spelling() {
    let cases = [
        (vec!["-B", "-V"], "df: invalid -B argument '-V'\n"),
        (
            vec!["--block-size", "-V"],
            "df: invalid --block-size argument '-V'\n",
        ),
        (vec!["-B1V"], "df: invalid suffix in -B argument '1V'\n"),
        (
            vec!["--block-size=1V"],
            "df: invalid suffix in --block-size argument '1V'\n",
        ),
        (
            vec!["-B18446744073709551616"],
            "df: -B argument '18446744073709551616' too large\n",
        ),
        (
            vec!["--block-size=18446744073709551616"],
            "df: --block-size argument '18446744073709551616' too large\n",
        ),
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
