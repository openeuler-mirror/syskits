use std::os::unix::process::CommandExt;
use std::process::Command;

fn command_header(command: &str, arguments: &[&str]) -> String {
    let output = Command::new(command)
        .arg0("df")
        .args(arguments)
        .env("LC_ALL", "C")
        .output()
        .expect("run df");

    assert!(
        output.status.success(),
        "arguments={arguments:?}, {output:?}"
    );
    String::from_utf8(output.stdout)
        .expect("df output is UTF-8")
        .lines()
        .next()
        .expect("df output has a header")
        .to_string()
}

fn header(arguments: &[&str]) -> String {
    command_header(env!("CARGO_BIN_EXE_df"), arguments)
}

fn gnu_header(arguments: &[&str]) -> String {
    command_header("/usr/bin/df", arguments)
}

#[test]
fn last_scaling_option_overrides_human_readable_mode() {
    for arguments in [["-h", "-B1K", "/"], ["-H", "-k", "/"]] {
        assert_eq!(header(&arguments), gnu_header(&arguments));
    }
}

#[test]
fn human_readable_option_overrides_an_earlier_fixed_block_size() {
    let arguments = ["-B1K", "-h", "/"];
    assert_eq!(header(&arguments), gnu_header(&arguments));
}

#[test]
fn output_size_header_follows_the_active_scaling_mode() {
    for option in ["-h", "-H"] {
        assert_eq!(
            header(&[option, "--output=size", "/"]).trim(),
            "Size",
            "{option}"
        );
    }
    assert_eq!(header(&["-B1K", "--output=size", "/"]).trim(), "1K-blocks");
}
