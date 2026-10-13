#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[test]
fn quoted_decimal_suffix_uses_locale_grouping() {
    use std::process::Command;

    if !locale_is_available("en_US.utf8") {
        return;
    }

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    let input = (1..=1000)
        .map(|number| format!("{number}\n"))
        .collect::<String>();
    std::fs::write(temp_dir.path().join("input"), input).expect("write input");

    let output = Command::new("bash")
        .args(["-c", "exec -a csplit \"$0\" \"$@\""])
        .arg(env!("CARGO_BIN_EXE_csplit"))
        .args(["-q", "-b", "%'d", "input", "/./", "{*}"])
        .current_dir(temp_dir.path())
        .env("LC_ALL", "en_US.utf8")
        .output()
        .expect("run csplit");

    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        std::fs::read(temp_dir.path().join("xx1,000")).expect("read grouped split"),
        b"1000\n"
    );
    assert!(!temp_dir.path().join("xx1000").exists());
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn locale_is_available(locale: &str) -> bool {
    std::process::Command::new("locale")
        .arg("-a")
        .output()
        .expect("list locales")
        .stdout
        .split(|byte| *byte == b'\n')
        .any(|name| name == locale.as_bytes())
}
