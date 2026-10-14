use std::os::unix::process::CommandExt;
use std::process::{Command, Output};

fn run_dircolors(input: &str) -> (Output, String) {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), input).unwrap();
    let file = file.path().to_str().unwrap().to_string();

    let output = Command::new(env!("CARGO_BIN_EXE_dircolors"))
        .arg0("dircolors")
        .args(["-b", file.as_str()])
        .env("LC_ALL", "C")
        .env_remove("TERM")
        .env_remove("COLORTERM")
        .output()
        .unwrap();

    (output, file)
}

#[test]
fn valid_input_exits_successfully() {
    let (output, _) = run_dircolors("DIR 01;34\n");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "LS_COLORS='di=01;34:';\nexport LS_COLORS\n"
    );
}

#[test]
fn parse_warnings_suppress_output_and_fail() {
    let (output, file) = run_dircolors("DIR 01;34\nINVALID\n");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        format!("dircolors: {file}:2: invalid line;  missing second token\n")
    );
}

#[test]
fn unrecognized_keywords_suppress_output_and_fail() {
    let (output, file) = run_dircolors("TERM none\nDIR 01;34\nUNKNOWN 01;32\n");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        format!("dircolors: {file}:3: unrecognized keyword UNKNOWN\n")
    );
}

#[test]
fn missing_file_reports_the_gnu_error_message() {
    let directory = tempfile::tempdir().unwrap();
    let missing_file = directory.path().join("missing");
    let missing_file = missing_file.to_str().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_dircolors"))
        .arg0("dircolors")
        .args(["-b", missing_file])
        .env("LC_ALL", "C")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        format!("dircolors: {missing_file}: No such file or directory\n")
    );
}

#[test]
fn directory_operand_reports_a_gnu_read_error() {
    let directory = tempfile::tempdir().unwrap();
    let directory = directory.path().to_str().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_dircolors"))
        .arg0("dircolors")
        .args(["-b", directory])
        .env("LC_ALL", "C")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        format!("dircolors: {directory}: read error: Is a directory\n")
    );
}

#[test]
fn inline_comment_is_not_part_of_the_color_value() {
    let (output, _) = run_dircolors("DIR 01;34#comment\n");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "LS_COLORS='di=01;34:';\nexport LS_COLORS\n"
    );
}

#[test]
fn print_ls_colors_does_not_escape_color_separators() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), "DIR 01=34:56\n").unwrap();
    let file = file.path().to_str().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_dircolors"))
        .arg0("dircolors")
        .args(["--print-ls-colors", file])
        .env("LC_ALL", "C")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "\x1b[01=34:56mdi\t01=34:56\x1b[0m\n"
    );
}

#[test]
fn unterminated_term_glob_uses_gnu_fnmatch_semantics() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), "TERM [\nDIR 01;34\n").unwrap();
    let file = file.path().to_str().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_dircolors"))
        .arg0("dircolors")
        .args(["-b", file])
        .env("LC_ALL", "C")
        .env("TERM", "[")
        .env_remove("COLORTERM")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "LS_COLORS='di=01;34:';\nexport LS_COLORS\n"
    );
}

#[test]
fn non_utf8_color_values_are_preserved_in_stdout() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), b"DIR 01;34\xff\n").unwrap();
    let file = file.path().to_str().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_dircolors"))
        .arg0("dircolors")
        .args(["-b", file])
        .env("LC_ALL", "C")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(
        output.stdout,
        b"LS_COLORS='di=01;34\xff:';\nexport LS_COLORS\n"
    );
}

#[test]
fn repeated_output_options_are_accepted() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), "DIR 01;34\n").unwrap();
    let file = file.path().to_str().unwrap();

    let shell_output = Command::new(env!("CARGO_BIN_EXE_dircolors"))
        .arg0("dircolors")
        .args(["-b", "-b", file])
        .env("LC_ALL", "C")
        .output()
        .unwrap();

    assert_eq!(shell_output.status.code(), Some(0));
    assert!(shell_output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(shell_output.stdout).unwrap(),
        "LS_COLORS='di=01;34:';\nexport LS_COLORS\n"
    );

    let database_output = Command::new(env!("CARGO_BIN_EXE_dircolors"))
        .arg0("dircolors")
        .args(["-p", "-p"])
        .env("LC_ALL", "C")
        .output()
        .unwrap();

    assert_eq!(database_output.status.code(), Some(0));
    assert!(database_output.stderr.is_empty());
    assert!(
        String::from_utf8(database_output.stdout)
            .unwrap()
            .starts_with("# Configuration file for dircolors,")
    );
}

#[test]
fn extra_operand_uses_utf8_locale_quote_marks() {
    let output = Command::new(env!("CARGO_BIN_EXE_dircolors"))
        .arg0("dircolors")
        .args(["-b", "first", "extra"])
        .env("LC_ALL", "C.UTF-8")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "dircolors: extra operand \u{2018}extra\u{2019}\nTry 'dircolors --help' for more information.\n"
    );
}
