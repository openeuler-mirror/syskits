#[cfg(target_os = "linux")]
use std::fs::File;
#[cfg(target_os = "linux")]
use std::process::Stdio;
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

#[cfg(target_os = "linux")]
#[test]
fn sum_missing_file_with_full_stderr_exits_failure_without_panicking() {
    let missing = "/__syskits_sum_missing_stderr_compat__";
    let gnu_stderr = File::options()
        .write(true)
        .open("/dev/full")
        .expect("open /dev/full for GNU sum");
    let syskits_stderr = File::options()
        .write(true)
        .open("/dev/full")
        .expect("open /dev/full for syskits sum");

    let gnu = Command::new("/usr/bin/sum")
        .arg(missing)
        .env("LC_ALL", "C")
        .stderr(Stdio::from(gnu_stderr))
        .status()
        .expect("run GNU sum");
    let syskits = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .arg("sum")
        .arg(missing)
        .env("LC_ALL", "C")
        .stderr(Stdio::from(syskits_stderr))
        .status()
        .expect("run syskits sum");

    assert_eq!(gnu.code(), Some(1));
    assert_eq!(syskits.code(), gnu.code());
}
