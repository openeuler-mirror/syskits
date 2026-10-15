#[cfg(unix)]
#[test]
fn short_options_are_rejected_like_gnu_link() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    for (argument, option) in [("-h", "h"), ("-V", "V"), ("-anything", "a")] {
        let output = Command::new(env!("CARGO_BIN_EXE_link"))
            .arg0("link")
            .env("LC_ALL", "C")
            .arg(argument)
            .output()
            .expect("run link");

        assert_eq!(output.status.code(), Some(1), "{argument}");
        assert!(output.stdout.is_empty(), "{argument}");
        assert_eq!(
            output.stderr,
            format!(
                "link: invalid option -- '{option}'\nTry 'link --help' for more information.\n"
            )
            .as_bytes(),
            "{argument}"
        );
    }
}
