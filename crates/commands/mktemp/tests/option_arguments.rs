use std::os::unix::process::CommandExt;
use std::process::Command;

fn run_mktemp(arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mktemp"))
        .arg0("mktemp")
        .env("LC_ALL", "C")
        .args(arguments)
        .output()
        .expect("run mktemp")
}

#[test]
fn missing_option_arguments_use_gnu_diagnostics() {
    for (arguments, expected) in [
        (
            &["-p"][..],
            "mktemp: option requires an argument -- 'p'\nTry 'mktemp --help' for more information.\n",
        ),
        (
            &["-up"][..],
            "mktemp: option requires an argument -- 'p'\nTry 'mktemp --help' for more information.\n",
        ),
        (
            &["--suffix"][..],
            "mktemp: option '--suffix' requires an argument\nTry 'mktemp --help' for more information.\n",
        ),
        (
            &["--suff"][..],
            "mktemp: option '--suffix' requires an argument\nTry 'mktemp --help' for more information.\n",
        ),
    ] {
        let output = run_mktemp(arguments);
        assert_eq!(output.status.code(), Some(1), "{arguments:?}");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            expected,
            "{arguments:?}"
        );
    }
}

#[test]
fn option_arguments_starting_with_a_hyphen_are_consumed() {
    let output = run_mktemp(&["-u", "-p", "-d"]);
    assert!(output.status.success(), "{output:?}");
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .starts_with("-d/tmp.")
    );

    let output = run_mktemp(&["-u", "--suffix", "--"]);
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8(output.stdout).unwrap().ends_with("--\n"));

    let output = run_mktemp(&["-u", "-p", "--suffix"]);
    assert!(output.status.success(), "{output:?}");
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .starts_with("--suffix/tmp.")
    );
}
