use std::os::unix::process::CommandExt;
use std::process::Command;

#[test]
fn output_conflicts_report_the_first_gnu_parser_conflict() {
    for (arguments, option) in [
        (&["--output", "-P", "-i", "/"][..], 'P'),
        (&["--output", "-T", "-i", "/"][..], 'T'),
        (&["-P", "--output", "-i", "/"][..], 'P'),
        (&["-T", "--output", "-i", "/"][..], 'T'),
        (&["--output", "-T", "-P", "/"][..], 'T'),
        (&["-P", "-i", "--output", "/"][..], 'i'),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_df"))
            .arg0("df")
            .args(arguments)
            .env("LC_ALL", "C")
            .output()
            .expect("run df");

        assert_eq!(output.status.code(), Some(1), "{arguments:?}");
        assert!(output.stdout.is_empty(), "{arguments:?}");
        assert_eq!(
            String::from_utf8(output.stderr).expect("df stderr is UTF-8"),
            format!(
                "df: options -{option} and --output are mutually exclusive\nTry 'df --help' for more information.\n"
            ),
            "{arguments:?}"
        );
    }
}
