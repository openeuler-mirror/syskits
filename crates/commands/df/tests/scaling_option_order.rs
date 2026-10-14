use std::os::unix::process::CommandExt;
use std::process::Command;

fn header(arguments: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_df"))
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

#[test]
fn last_scaling_option_overrides_human_readable_mode() {
    let expected = "Filesystem               1K-blocks     Used Available Use% Mounted on";

    assert_eq!(header(&["-h", "-B1K", "/"]), expected);
    assert_eq!(header(&["-H", "-k", "/"]), expected);
}

#[test]
fn human_readable_option_overrides_an_earlier_fixed_block_size() {
    assert_eq!(
        header(&["-B1K", "-h", "/"]),
        "Filesystem                Size  Used Avail Use% Mounted on"
    );
}
