#![cfg(target_os = "linux")]

use std::os::unix::process::CommandExt;
use std::process::Command;

fn run_df(arguments: &[&str]) -> Vec<Vec<String>> {
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
        .skip(1)
        .map(|line| line.split_whitespace().map(str::to_owned).collect())
        .collect()
}

#[test]
fn explicitly_named_pseudo_filesystems_are_not_replaced_by_root() {
    assert_eq!(
        run_df(&["--output=source,target", "/proc", "/sys"]),
        vec![
            vec!["proc".to_string(), "/proc".to_string()],
            vec!["sysfs".to_string(), "/sys".to_string()],
        ]
    );
}

#[test]
fn direct_mode_uses_the_named_pseudo_filesystem_statistics() {
    assert_eq!(
        run_df(&["--direct", "--output=size,file", "/proc", "/sys"]),
        vec![
            vec!["0".to_string(), "/proc".to_string()],
            vec!["0".to_string(), "/sys".to_string()],
        ]
    );
}
