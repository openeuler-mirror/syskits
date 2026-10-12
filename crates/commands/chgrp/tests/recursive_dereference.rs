#[cfg(unix)]
#[test]
fn chgrp_reports_recursive_dereference_mismatch_like_gnu() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
        .env("LC_ALL", "C")
        .arg0("chgrp")
        .args(["-R", "--dereference"])
        .output()
        .expect("run chgrp");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr is UTF-8"),
        "chgrp: -R --dereference requires either -H or -L\n"
    );
}
