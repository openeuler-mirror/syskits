use std::os::unix::process::CommandExt;
use std::process::{Command, Output};

fn run_df(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_df"))
        .arg0("df")
        .args(arguments)
        .env("LC_ALL", "C")
        .output()
        .expect("run df")
}

#[test]
fn bare_output_does_not_append_fields_to_an_explicit_list() {
    let explicit = run_df(&["--output=source", "/"]);
    assert!(explicit.status.success());

    for arguments in [
        &["--output=source", "--output", "/"][..],
        &["--output", "--output=source", "/"][..],
    ] {
        let output = run_df(arguments);

        assert!(output.status.success(), "{arguments:?}");
        assert_eq!(output.stdout, explicit.stdout, "{arguments:?}");
        assert_eq!(output.stderr, explicit.stderr, "{arguments:?}");
    }
}

#[test]
fn bare_output_uses_all_fields_when_no_field_is_explicit() {
    let bare_output = run_df(&["--output", "/"]);
    let explicit_output = run_df(&[
        "--output=source,fstype,itotal,iused,iavail,ipcent,size,used,avail,pcent,file,target",
        "/",
    ]);

    assert!(bare_output.status.success());
    assert_eq!(bare_output.stdout, explicit_output.stdout);
    assert_eq!(bare_output.stderr, explicit_output.stderr);
}
