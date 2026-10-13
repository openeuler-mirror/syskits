#[cfg(target_os = "linux")]
#[test]
fn oversized_bre_intervals_use_gnu_diagnostics() {
    use std::process::Command;

    for pattern in [
        "/a\\{32768\\}/",
        "/a\\{,32768\\}/",
        "/a\\{32767,32768\\}/",
        "/a\\{2147483648\\}/",
        "/a\\{,999999999999999999999999\\}/",
        "/a\\{999999999999999999999999,\\}/",
    ] {
        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        std::fs::write(temp_dir.path().join("input"), b"a\n").expect("write input");

        let output = Command::new("bash")
            .args(["-c", "exec -a csplit \"$0\" \"$@\""])
            .arg(env!("CARGO_BIN_EXE_csplit"))
            .args(["-q", "input", pattern])
            .current_dir(temp_dir.path())
            .env("LC_ALL", "C")
            .output()
            .expect("run csplit");

        assert_eq!(output.status.code(), Some(1), "{pattern}: {output:?}");
        assert!(output.stdout.is_empty(), "{pattern}: {output:?}");
        assert_eq!(
            output.stderr,
            format!(
                "csplit: '{}': invalid regular expression: Regular expression too big\n",
                pattern.replace('\\', "\\\\")
            )
            .as_bytes(),
            "{pattern}"
        );
    }

    for pattern in [
        "/a\\{32768,32767\\}/",
        "/a\\{999999999999999999999999,32767\\}/",
    ] {
        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        std::fs::write(temp_dir.path().join("input"), b"a\n").expect("write input");
        let output = Command::new("bash")
            .args(["-c", "exec -a csplit \"$0\" \"$@\""])
            .arg(env!("CARGO_BIN_EXE_csplit"))
            .args(["-q", "input", pattern])
            .current_dir(temp_dir.path())
            .env("LC_ALL", "C")
            .output()
            .expect("run csplit");

        assert_eq!(output.status.code(), Some(1), "{pattern}: {output:?}");
        assert!(output.stdout.is_empty(), "{pattern}: {output:?}");
        assert_eq!(
            output.stderr,
            format!(
                "csplit: '{}': invalid regular expression: Invalid content of \\{{\\}}\n",
                pattern.replace('\\', "\\\\")
            )
            .as_bytes(),
            "{pattern}"
        );
    }
}
