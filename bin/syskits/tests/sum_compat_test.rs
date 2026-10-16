use std::process::{Command, Output};

fn run_gnu(args: &[&str], posixly_correct: Option<&str>) -> Output {
    let mut command = Command::new("/usr/bin/sum");
    command.args(args).env("LC_ALL", "C");
    match posixly_correct {
        Some(value) => command.env("POSIXLY_CORRECT", value),
        None => command.env_remove("POSIXLY_CORRECT"),
    };
    command.output().expect("run GNU sum")
}

fn run_syskits(args: &[&str], posixly_correct: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_syskits"));
    command.arg("sum").args(args).env("LC_ALL", "C");
    match posixly_correct {
        Some(value) => command.env("POSIXLY_CORRECT", value),
        None => command.env_remove("POSIXLY_CORRECT"),
    };
    command.output().expect("run syskits sum")
}

#[test]
fn sum_posixly_correct_stops_options_at_first_operand() {
    for value in ["1", ""] {
        let gnu = run_gnu(&["/dev/null", "-s"], Some(value));
        let syskits = run_syskits(&["/dev/null", "-s"], Some(value));

        assert_eq!(syskits.status.code(), gnu.status.code());
        assert_eq!(syskits.stdout, gnu.stdout);
        assert!(gnu.stderr.ends_with(b": -s: No such file or directory\n"));
        assert!(
            syskits
                .stderr
                .ends_with(b": -s: No such file or directory\n")
        );
    }
}

#[test]
fn sum_without_posixly_correct_still_permutes_options() {
    let gnu = run_gnu(&["/dev/null", "-s"], None);
    let syskits = run_syskits(&["/dev/null", "-s"], None);

    assert_eq!(syskits.status.code(), gnu.status.code());
    assert_eq!(syskits.stdout, gnu.stdout);
    assert_eq!(syskits.stderr, gnu.stderr);
}
