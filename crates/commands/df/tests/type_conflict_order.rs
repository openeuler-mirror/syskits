use std::os::unix::process::CommandExt;
use std::process::Command;

#[test]
fn type_conflicts_are_reported_in_gnu_reverse_option_order() {
    let output = Command::new(env!("CARGO_BIN_EXE_df"))
        .arg0("df")
        .args(["-t", "proc", "-t", "sysfs", "-x", "proc", "-x", "sysfs"])
        .env("LC_ALL", "C")
        .output()
        .expect("run df");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).expect("df stderr is UTF-8"),
        "df: file system type 'sysfs' both selected and excluded\n\
         df: file system type 'proc' both selected and excluded\n"
    );
}
