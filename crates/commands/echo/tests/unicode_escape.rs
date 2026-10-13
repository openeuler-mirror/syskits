use std::os::unix::process::CommandExt;
use std::process::Command;

#[test]
fn unicode_escapes_are_output_literally() {
    let cases = [
        ("\\u0041", b"\\u0041\n".as_slice()),
        ("\\U00000041", b"\\U00000041\n".as_slice()),
        ("\\u", b"\\u\n".as_slice()),
        ("\\U", b"\\U\n".as_slice()),
    ];

    for (argument, expected_stdout) in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_echo"))
            .arg0("echo")
            .args(["-e", argument])
            .env("LC_ALL", "C")
            .output()
            .unwrap();

        assert_eq!(output.status.code(), Some(0));
        assert!(output.stderr.is_empty());
        assert_eq!(output.stdout, expected_stdout);
    }
}
