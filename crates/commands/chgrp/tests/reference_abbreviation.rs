#[cfg(unix)]
#[test]
fn chgrp_accepts_the_unique_reference_option_abbreviation() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let path = "/definitely-missing-chgrp-file";
    let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
        .env("LC_ALL", "C")
        .arg0("chgrp")
        .args(["--ref=/tmp", path])
        .output()
        .expect("run chgrp");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr is UTF-8"),
        format!("chgrp: cannot access '{path}': No such file or directory\n")
    );
}
