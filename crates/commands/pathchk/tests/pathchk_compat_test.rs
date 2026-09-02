use std::process::Command;

#[test]
fn pathchk_posix_component_limit_uses_locale_quotes() {
    let output = Command::new(env!("CARGO_BIN_EXE_pathchk"))
        .env_remove("LC_ALL")
        .env("LANG", "en_US.UTF-8")
        .args(["-p", "/tmp/testfile123456789"])
        .output()
        .expect("run pathchk");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(
        stderr.contains("component ‘testfile123456789’"),
        "stderr: {stderr}"
    );
}

#[test]
fn pathchk_posix_component_limit_uses_ascii_quotes_in_c_locale() {
    let output = Command::new(env!("CARGO_BIN_EXE_pathchk"))
        .env("LC_ALL", "C")
        .args(["-p", "/tmp/testfile123456789"])
        .output()
        .expect("run pathchk");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(
        stderr.contains("component 'testfile123456789'"),
        "stderr: {stderr}"
    );
}
