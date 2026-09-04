use std::process::Command;

fn locale_available(locale: &str) -> bool {
    let output = Command::new("locale")
        .arg("-a")
        .output()
        .expect("list installed locales");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|installed| installed == locale)
}

#[test]
fn seq_terminates_with_sigpipe_when_the_reader_closes() {
    let output = Command::new("/bin/bash")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .args([
            "-c",
            "set -o pipefail; \"$1\" seq inf | head -n1 >/dev/null; printf '%s' \"${PIPESTATUS[0]}\"",
            "bash",
            env!("CARGO_BIN_EXE_syskits"),
        ])
        .output()
        .expect("run syskits seq through a closing pipe");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "141");
    assert!(output.stderr.is_empty());
}

#[test]
fn seq_reports_epipe_when_sigpipe_is_ignored() {
    let output = Command::new("/bin/bash")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .args([
            "-c",
            "d=$(mktemp -d); trap 'rm -rf \"$d\"' EXIT; trap '' PIPE; { \"$1\" seq inf 2>\"$d/err\"; printf '%s' $? >\"$d/code\"; } | head -n1 >/dev/null; cat \"$d/code\"; cat \"$d/err\" >&2",
            "bash",
            env!("CARGO_BIN_EXE_syskits"),
        ])
        .output()
        .expect("run syskits seq with SIGPIPE ignored");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "1");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("write error: Broken pipe"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
