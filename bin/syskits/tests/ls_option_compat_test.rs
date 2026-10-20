use std::fs;
use std::process::{Command, Output};

use tempfile::TempDir;

fn run_gnu(directory: &TempDir, arguments: &[&str]) -> Output {
    Command::new("/usr/bin/ls")
        .current_dir(directory.path())
        .env("LC_ALL", "C")
        .args(arguments)
        .output()
        .expect("run GNU ls")
}

fn run_syskits(directory: &TempDir, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(directory.path())
        .env("LC_ALL", "C")
        .arg("ls")
        .args(arguments)
        .output()
        .expect("run syskits ls")
}

#[test]
fn ls_accepts_separated_required_long_option_values() {
    let directory = TempDir::new().expect("tempdir");
    fs::write(directory.path().join("a"), b"").expect("create fixture");

    for arguments in [
        ["--format", "long", "."],
        ["--sort", "size", "."],
        ["--time", "atime", "."],
        ["--block-size", "1", "."],
    ] {
        let gnu = run_gnu(&directory, &arguments);
        let syskits = run_syskits(&directory, &arguments);

        assert_eq!(syskits.status.code(), gnu.status.code(), "{arguments:?}");
        assert_eq!(syskits.stderr, gnu.stderr, "{arguments:?}");
    }
}

#[test]
fn ls_extensions_do_not_collide_with_gnu_value_abbreviations() {
    let directory = TempDir::new().expect("tempdir");
    fs::write(directory.path().join("a"), b"").expect("create fixture a");
    fs::write(directory.path().join("b"), b"").expect("create fixture b");

    for arguments in [["--format=c", "."], ["--format=co", "."], ["--sort=n", "."]] {
        let gnu = run_gnu(&directory, &arguments);
        let syskits = run_syskits(&directory, &arguments);

        assert_eq!(syskits.status.code(), gnu.status.code(), "{arguments:?}");
        assert_eq!(syskits.stderr, gnu.stderr, "{arguments:?}");
        assert_eq!(syskits.stdout, gnu.stdout, "{arguments:?}");
    }
}

#[test]
fn ls_invalid_enum_values_use_gnu_failure_status() {
    let directory = TempDir::new().expect("tempdir");
    fs::write(directory.path().join("a"), b"").expect("create fixture");

    for argument in [
        "--hyperlink=bad",
        "--indicator-style=bad",
        "--quoting-style=bad",
    ] {
        let gnu = run_gnu(&directory, &[argument, "a"]);
        let syskits = run_syskits(&directory, &[argument, "a"]);

        assert_eq!(syskits.status.code(), gnu.status.code(), "{argument}");
        assert_eq!(syskits.status.code(), Some(1), "{argument}");
    }
}

fn valid_argument_lines(output: &Output) -> Vec<String> {
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .skip_while(|line| *line != "Valid arguments are:")
        .take_while(|line| !line.starts_with("Try "))
        .map(str::to_string)
        .collect()
}

#[test]
fn ls_invalid_values_list_gnu_valid_arguments() {
    let directory = TempDir::new().expect("tempdir");
    fs::write(directory.path().join("a"), b"").expect("create fixture");

    for argument in [
        "--format=bad",
        "--sort=bad",
        "--time=bad",
        "--classify=bad",
        "--color=bad",
        "--hyperlink=bad",
        "--indicator-style=bad",
        "--quoting-style=bad",
    ] {
        let gnu = run_gnu(&directory, &[argument, "a"]);
        let syskits = run_syskits(&directory, &[argument, "a"]);

        assert_eq!(syskits.status.code(), gnu.status.code(), "{argument}");
        assert_eq!(
            valid_argument_lines(&syskits),
            valid_argument_lines(&gnu),
            "{argument}"
        );
    }
}

#[test]
fn ls_unknown_option_diagnostic_omits_clap_usage() {
    let directory = TempDir::new().expect("tempdir");
    let gnu = run_gnu(&directory, &["--definitely-unknown"]);
    let syskits = run_syskits(&directory, &["--definitely-unknown"]);
    let stderr = String::from_utf8_lossy(&syskits.stderr);

    assert_eq!(syskits.status.code(), gnu.status.code());
    assert_eq!(stderr.lines().count(), 2, "{stderr}");
    assert!(stderr.contains("unrecognized option '--definitely-unknown'"));
    assert!(!stderr.lines().any(|line| line.starts_with("Usage:")));
}

#[test]
fn ls_unknown_color_prefix_reports_prefix_before_parse_failure() {
    let directory = TempDir::new().expect("tempdir");
    fs::write(directory.path().join("a"), b"").expect("create fixture");
    let gnu = Command::new("/usr/bin/ls")
        .current_dir(directory.path())
        .env("LC_ALL", "C")
        .env("LS_COLORS", "xx=31")
        .args(["--color=always", "-1", "a"])
        .output()
        .expect("run GNU ls");
    let syskits = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(directory.path())
        .env("LC_ALL", "C")
        .env("LS_COLORS", "xx=31")
        .args(["ls", "--color=always", "-1", "a"])
        .output()
        .expect("run syskits ls");
    let messages = |output: &Output| {
        String::from_utf8_lossy(&output.stderr)
            .lines()
            .map(|line| line.split_once(": ").map_or(line, |(_, message)| message))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };

    assert_eq!(syskits.status.code(), gnu.status.code());
    assert_eq!(messages(&syskits), messages(&gnu));
    assert_eq!(
        messages(&syskits),
        [
            "unrecognized prefix: 'xx'",
            "unparsable value for LS_COLORS environment variable"
        ]
    );
}
