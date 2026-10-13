use std::os::unix::process::CommandExt;
use std::process::Command;

#[test]
fn numeric_escapes_write_raw_bytes() {
    let cases = [
        ("\\x80", &[0x80, b'\n'][..]),
        ("\\xFF", &[0xff, b'\n'][..]),
        ("\\777", &[0xff, b'\n'][..]),
        ("\\400", &[0, b'\n'][..]),
        ("\\x100", &[0x10, b'0', b'\n'][..]),
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
