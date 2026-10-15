#[cfg(unix)]
#[test]
fn invalid_operand_counts_use_gnu_diagnostics() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let cases = [
        (Vec::<&str>::new(), "missing operand".to_string()),
        (vec!["file1"], "missing operand after 'file1'".to_string()),
        (
            vec!["file1", "file2", "file3"],
            "extra operand 'file3'".to_string(),
        ),
    ];

    for (arguments, message) in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_link"))
            .arg0("link")
            .env("LC_ALL", "C")
            .args(arguments)
            .output()
            .expect("run link");

        assert_eq!(output.status.code(), Some(1), "{message}");
        assert!(output.stdout.is_empty(), "{message}");
        assert_eq!(
            output.stderr,
            format!("link: {message}\nTry 'link --help' for more information.\n").as_bytes(),
            "{message}"
        );
    }
}
