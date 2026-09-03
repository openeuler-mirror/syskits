use std::process::Command;

use tempfile::TempDir;

#[test]
fn b2sum_verify_only_options_fail_outside_check_mode() {
    let temp_dir = TempDir::new().expect("tempdir");
    let testfile = temp_dir.path().join("testfile");
    std::fs::write(&testfile, "hello\n").expect("write testfile");

    for (option, expected_option) in [
        ("--quiet", "--quiet"),
        ("--status", "--status"),
        ("--strict", "--strict"),
        ("--warn", "--warn"),
        ("-w", "--warn"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .args(["b2sum", option, testfile.to_str().expect("utf8 path")])
            .output()
            .expect("run syskits b2sum");

        assert!(!output.status.success(), "option {option} should fail");
        assert!(
            output.stdout.is_empty(),
            "option {option} should not write stdout"
        );

        let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
        assert!(
            stderr.contains(&format!(
                "b2sum: the {expected_option} option is meaningful only when verifying checksums\n"
            )),
            "stderr for {option}: {stderr:?}"
        );
        assert!(
            stderr.contains("Try 'b2sum --help' for more information.\n"),
            "stderr for {option}: {stderr:?}"
        );
    }
}

#[test]
fn b2sum_check_missing_file_reports_only_unreadable_warning() {
    let temp_dir = TempDir::new().expect("tempdir");
    let missing = temp_dir.path().join("missing");
    let sums = temp_dir.path().join("sums");

    std::fs::write(
        &sums,
        format!(
            "02545c2918a2d7a52c13d0c7b62829c93c9fc44775025e15cd1573d1b6ac934449421c6803803e3e8dcd5849fa5b8a2f2cbbcbcd4cab1868735c2da1b74fefc2  {}\n",
            missing.display()
        ),
    )
    .expect("write checksum manifest");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["b2sum", "-c", sums.to_str().expect("utf8 path")])
        .output()
        .expect("run syskits b2sum");

    assert!(!output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");

    assert!(
        stdout.contains(&format!("{}: FAILED open or read\n", missing.display())),
        "stdout: {stdout:?}"
    );
    assert!(
        stderr.contains("b2sum: WARNING: 1 listed file could not be read\n"),
        "stderr: {stderr:?}"
    );
    assert!(
        !stderr.contains("computed checksum did NOT match"),
        "stderr should not report a checksum mismatch for an unreadable file: {stderr:?}"
    );
}
