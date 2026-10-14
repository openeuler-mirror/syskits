use std::os::unix::process::CommandExt;
use std::process::Command;

#[test]
fn posix_mode_treats_options_after_a_path_as_paths() {
    let output = Command::new(env!("CARGO_BIN_EXE_df"))
        .arg0("df")
        .args(["/", "-h"])
        .env("LC_ALL", "C")
        .env("POSIXLY_CORRECT", "1")
        .output()
        .expect("run df");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stdout)
            .expect("df stdout is UTF-8")
            .lines()
            .next(),
        Some("Filesystem               512B-blocks     Used Available Use% Mounted on")
    );
    assert_eq!(
        String::from_utf8(output.stderr).expect("df stderr is UTF-8"),
        "df: -h: No such file or directory\n"
    );
}

#[test]
fn posix_mode_quotes_a_long_option_treated_as_a_path() {
    let output = Command::new(env!("CARGO_BIN_EXE_df"))
        .arg0("df")
        .args(["/", "--output=source"])
        .env("LC_ALL", "C")
        .env("POSIXLY_CORRECT", "1")
        .output()
        .expect("run df");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).expect("df stderr is UTF-8"),
        "df: '--output=source': No such file or directory\n"
    );
}
