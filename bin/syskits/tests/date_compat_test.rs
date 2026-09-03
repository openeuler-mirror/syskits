use std::process::Command;

#[test]
fn date_debug_for_ymd_date_reports_parse_trace() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("TZ", "Asia/Shanghai")
        .args(["date", "--debug", "-d", "2020-01-02", "+%F"])
        .output()
        .expect("run syskits date --debug");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"2020-01-02\n");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("date: parsed date part: (Y-M-D) 2020-01-02"));
    assert!(stderr.contains("date: input timezone: system default"));
    assert!(stderr.contains("date: warning: using midnight as starting time: 00:00:00"));
    assert!(stderr.contains("date: starting date/time: '(Y-M-D) 2020-01-02 00:00:00'"));
    assert!(stderr.contains("date: final: 1577894400.000000000 (epoch-seconds)"));
    assert!(stderr.contains("date: final: (Y-M-D) 2020-01-01 16:00:00 (UTC)"));
    assert!(stderr.contains("date: final: (Y-M-D) 2020-01-02 00:00:00 (UTC+08)"));
    assert!(stderr.contains("date: output format: ‘%F’"));
}

#[test]
fn date_numeric_date_operand_is_parsed_as_today_time() {
    for (input, expected_time) in [
        ("1", "01:00:00"),
        ("100", "01:00:00"),
        ("1234", "12:34:00"),
        ("2359", "23:59:00"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .env("TZ", "Asia/Shanghai")
            .args(["date", "-d", input, "+%T"])
            .output()
            .expect("run syskits date -d compact time");

        assert!(
            output.status.success(),
            "input {input}, stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("{expected_time}\n"),
            "input {input}"
        );
    }
}

#[test]
fn date_numeric_date_operand_rejects_invalid_compact_time() {
    for input in ["090", "99", "12345", "2400"] {
        let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .env("TZ", "Asia/Shanghai")
            .args(["date", "-d", input, "+%T"])
            .output()
            .expect("run syskits date -d invalid compact time");

        assert!(!output.status.success(), "input {input} should fail");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("invalid date"),
            "input {input}, stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn date_set_prints_target_time_even_when_clock_set_fails() {
    let output = Command::new("runuser")
        .args([
            "-u",
            "nobody",
            "--",
            env!("CARGO_BIN_EXE_syskits"),
            "date",
            "-s",
            "2012-09-23 01:01:00",
            "+%F_%T_%Z",
        ])
        .env("TZ", "Asia/Shanghai")
        .output()
        .expect("run syskits date -s as unprivileged user");

    assert!(
        !output.status.success(),
        "date -s should fail without permission to set CLOCK_REALTIME"
    );
    assert_eq!(output.stdout, b"2012-09-23_01:01:00_CST\n");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("cannot set date"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn date_set_numeric_operand_is_parsed_as_today_time_even_when_clock_set_fails() {
    let output = Command::new("runuser")
        .args([
            "-u",
            "nobody",
            "--",
            env!("CARGO_BIN_EXE_syskits"),
            "date",
            "-s",
            "1",
            "+%T",
        ])
        .env("TZ", "Asia/Shanghai")
        .output()
        .expect("run syskits date -s compact time as unprivileged user");

    assert!(
        !output.status.success(),
        "date -s should fail without permission to set CLOCK_REALTIME"
    );
    assert_eq!(output.stdout, b"01:00:00\n");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("cannot set date"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn date_set_utc_epoch_prints_utc_target_time_even_when_clock_set_fails() {
    let output = Command::new("runuser")
        .args([
            "-u",
            "nobody",
            "--",
            env!("CARGO_BIN_EXE_syskits"),
            "date",
            "-u",
            "-s",
            "@1782268970",
            "+%F_%T_%Z",
        ])
        .env("TZ", "Asia/Shanghai")
        .output()
        .expect("run syskits date -u -s as unprivileged user");

    assert!(
        !output.status.success(),
        "date -s should fail without permission to set CLOCK_REALTIME"
    );
    assert_eq!(output.stdout, b"2026-06-24_02:42:50_UTC\n");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("cannot set date"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
