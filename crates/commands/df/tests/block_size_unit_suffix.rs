use std::os::unix::process::CommandExt;
use std::process::{Command, Output};

fn run_df(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_df"))
        .arg0("df")
        .args(arguments)
        .env("LC_ALL", "C")
        .env_remove("DF_BLOCK_SIZE")
        .env_remove("BLOCK_SIZE")
        .env_remove("BLOCKSIZE")
        .output()
        .expect("run df")
}

fn size_value(output: Output) -> String {
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout)
        .expect("df output is UTF-8")
        .lines()
        .nth(1)
        .expect("df output has a data row")
        .trim()
        .to_string()
}

fn header(output: Output) -> String {
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout)
        .expect("df output is UTF-8")
        .lines()
        .next()
        .expect("df output has a header")
        .to_string()
}

#[test]
fn block_size_without_number_retains_gnu_unit_suffixes() {
    for (argument, suffix) in [
        ("-BK", "K"),
        ("-BkB", "kB"),
        ("-BKiB", "KiB"),
        ("-BM", "M"),
        ("-BmB", "MB"),
        ("-BMiB", "MiB"),
        ("-BP", "P"),
        ("-BPB", "PB"),
        ("-BPiB", "PiB"),
        ("-BE", "E"),
        ("-BEB", "EB"),
        ("-BEiB", "EiB"),
    ] {
        let value = size_value(run_df(&[argument, "--output=size", "/"]));
        let number = value.strip_suffix(suffix).expect("value has unit suffix");
        assert!(number.chars().all(|character| character.is_ascii_digit()));
    }
}

#[test]
fn block_size_without_number_retains_gnu_unit_suffixes_in_the_header() {
    for (argument, suffix) in [
        ("-BK", "K"),
        ("-BkB", "kB"),
        ("-BKiB", "KiB"),
        ("-BM", "M"),
        ("-BmB", "MB"),
        ("-BMiB", "MiB"),
        ("-BP", "P"),
        ("-BPB", "PB"),
        ("-BPiB", "PiB"),
        ("-BE", "E"),
        ("-BEB", "EB"),
        ("-BEiB", "EiB"),
    ] {
        let header = header(run_df(&[argument, "/"]));
        assert!(
            header.contains(&format!("1{suffix}-blocks")),
            "{argument}: {header}"
        );
    }
}

#[test]
fn numeric_block_size_does_not_retain_a_unit_suffix() {
    for argument in ["-B1K", "-B1kB", "-B1KiB", "-B1M"] {
        let value = size_value(run_df(&[argument, "--output=size", "/"]));
        assert!(
            value.chars().all(|character| character.is_ascii_digit()),
            "{argument}: {value}"
        );
    }
}

#[test]
fn invalid_df_block_size_environment_uses_gnu_partial_results() {
    for (value, argument) in [("e", "-B1"), ("1B", "-B1"), ("Z", "-B18446744073709551615")] {
        let from_env = Command::new(env!("CARGO_BIN_EXE_df"))
            .arg0("df")
            .args(["--output=size", "/"])
            .env("LC_ALL", "C")
            .env("DF_BLOCK_SIZE", value)
            .env_remove("BLOCK_SIZE")
            .env_remove("BLOCKSIZE")
            .output()
            .expect("run df");
        let from_option = run_df(&[argument, "--output=size", "/"]);

        assert!(from_env.status.success(), "DF_BLOCK_SIZE={value}");
        assert_eq!(from_env.stdout, from_option.stdout, "DF_BLOCK_SIZE={value}");
        assert_eq!(from_env.stderr, from_option.stderr, "DF_BLOCK_SIZE={value}");
    }
}

#[test]
fn df_block_size_environment_retains_unit_suffixes() {
    let output = Command::new(env!("CARGO_BIN_EXE_df"))
        .arg0("df")
        .args(["--output=size", "/"])
        .env("LC_ALL", "C")
        .env("DF_BLOCK_SIZE", "KiB")
        .env_remove("BLOCK_SIZE")
        .env_remove("BLOCKSIZE")
        .output()
        .expect("run df");

    let value = size_value(output);
    let number = value.strip_suffix("KiB").expect("value has KiB suffix");
    assert!(number.chars().all(|character| character.is_ascii_digit()));
}
