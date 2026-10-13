use std::os::unix::process::CommandExt;
use std::process::Command;

#[test]
fn short_type_alias_is_accepted_like_gnu_df() {
    let output = Command::new(env!("CARGO_BIN_EXE_df"))
        .arg0("df")
        .args(["-F", "syskits-unknown-filesystem"])
        .env("LC_ALL", "C")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "df: no file systems processed\n"
    );
}
