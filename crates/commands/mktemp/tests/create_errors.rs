#[cfg(unix)]
#[test]
fn create_errors_include_the_original_template() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let template = format!("/tmp/{}XXX", "x".repeat(255));
    for (args, kind) in [
        (&[template.as_str()][..], "file"),
        (&["-d", template.as_str()][..], "directory"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_mktemp"))
            .arg0("mktemp")
            .env("LC_ALL", "C")
            .args(args)
            .output()
            .expect("run mktemp with a long template");

        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            format!(
                "mktemp: failed to create {kind} via template '{template}': File name too long\n"
            )
        );
    }
}
