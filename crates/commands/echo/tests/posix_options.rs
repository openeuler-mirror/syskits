use std::os::unix::process::CommandExt;
use std::process::Command;

#[test]
fn posix_mode_consumes_valid_options_after_initial_n() {
    let cases = [
        (vec!["-n", "-e", "\\nX"], b"\nX".as_slice()),
        (vec!["-n", "-n", "text"], b"text".as_slice()),
        (vec!["-n", "-E", "\\nX"], b"\nX".as_slice()),
        (vec!["-n", "text", "-E"], b"text -E".as_slice()),
        (vec!["-ne", "text"], b"-ne text\n".as_slice()),
    ];

    for (arguments, expected_stdout) in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_echo"))
            .arg0("echo")
            .args(arguments)
            .env("LC_ALL", "C")
            .env("POSIXLY_CORRECT", "1")
            .output()
            .unwrap();

        assert_eq!(output.status.code(), Some(0));
        assert!(output.stderr.is_empty());
        assert_eq!(output.stdout, expected_stdout);
    }
}
