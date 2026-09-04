use std::process::Command;

use tempfile::TempDir;

fn expected_crc_debug_stderr() -> &'static str {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if std::arch::is_x86_feature_detected!("pclmulqdq")
            && std::arch::is_x86_feature_detected!("avx")
        {
            "cksum: using pclmul hardware support\n"
        } else {
            "cksum: pclmul support not detected\n"
        }
    }

    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    {
        ""
    }
}

#[test]
fn cksum_verify_only_options_fail_outside_check_mode() {
    let temp_dir = TempDir::new().expect("tempdir");
    let testfile = temp_dir.path().join("testfile");
    std::fs::write(&testfile, "hello\n").expect("write testfile");

    for (option, expected_option) in [
        ("--ignore-missing", "--ignore-missing"),
        ("--ig", "--ignore-missing"),
        ("--quiet", "--quiet"),
        ("--q", "--quiet"),
        ("--status", "--status"),
        ("--sta", "--status"),
        ("--strict", "--strict"),
        ("--str", "--strict"),
        ("--warn", "--warn"),
        ("--w", "--warn"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .args(["cksum", option, testfile.to_str().expect("utf8 path")])
            .output()
            .expect("run syskits cksum");

        assert!(!output.status.success(), "option {option} should fail");
        assert!(
            output.stdout.is_empty(),
            "option {option} should not write stdout"
        );

        let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
        assert!(
            stderr.contains(&format!(
                "cksum: the {expected_option} option is meaningful only when verifying checksums\n"
            )),
            "stderr for {option}: {stderr:?}"
        );
        assert!(
            stderr.contains("Try 'cksum --help' for more information.\n"),
            "stderr for {option}: {stderr:?}"
        );
    }
}
