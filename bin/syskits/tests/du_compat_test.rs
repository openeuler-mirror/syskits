use std::fs;
use std::process::Command;

use filetime::{FileTime, set_file_times};
use tempfile::TempDir;

fn set_times(path: impl AsRef<std::path::Path>, atime: i64, mtime: i64) {
    set_file_times(
        path,
        FileTime::from_unix_time(atime, 0),
        FileTime::from_unix_time(mtime, 0),
    )
    .expect("set file times");
}

#[test]
fn du_time_atime_reports_latest_descendant_time_for_directories() {
    let temp_dir = TempDir::new().expect("tempdir");
    let case = temp_dir.path().join("case");
    let sub = case.join("sub");
    let deep = sub.join("deep");
    fs::create_dir_all(&deep).expect("create fixture dirs");
    fs::write(case.join("a"), b"a").expect("write a");
    fs::write(sub.join("b"), b"b").expect("write b");
    fs::write(deep.join("c"), b"c").expect("write c");

    let file_atime_2030 = 1_893_456_000;
    let time_2000 = 946_684_800;
    for path in [case.join("a"), sub.join("b"), deep.join("c")] {
        set_times(path, file_atime_2030, time_2000);
    }
    for path in [&case, &sub, &deep] {
        set_times(path, time_2000, time_2000);
    }

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C")
        .env("TZ", "UTC0")
        .current_dir(temp_dir.path())
        .args(["du", "--time=atime", "--time-style=full-iso", "case"])
        .output()
        .expect("run syskits du --time=atime");

    assert!(
        output.status.success(),
        "stdout: {}, stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stderr, b"");

    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 3, "stdout: {stdout}");
    for line in lines {
        let fields = line.split('\t').collect::<Vec<_>>();
        assert_eq!(fields.len(), 3, "line: {line}");
        assert!(
            fields[1].starts_with("2030-01-01 00:00:00.000000000 +0000"),
            "line: {line}"
        );
    }
}

#[test]
fn du_si_and_suffix_only_block_sizes_match_gnu() {
    let temp_dir = TempDir::new().expect("tempdir");
    let case = temp_dir.path().join("case");
    fs::create_dir(&case).expect("create case dir");
    for name in ["a", "b", "c"] {
        fs::write(case.join(name), b"x").expect("write fixture file");
    }

    for flags in [
        &["-s", "--si"][..],
        &["-s", "--block-size=M"][..],
        &["-s", "--block-size=MB"][..],
    ] {
        let gnu = Command::new("/usr/bin/du")
            .env("LC_ALL", "C")
            .args(flags)
            .arg(&case)
            .output()
            .expect("run GNU du");
        let syskits = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .env("LC_ALL", "C")
            .arg("du")
            .args(flags)
            .arg(&case)
            .output()
            .expect("run syskits du");

        assert_eq!(
            syskits.status.code(),
            gnu.status.code(),
            "flags: {flags:?}, GNU stderr: {}, syskits stderr: {}",
            String::from_utf8_lossy(&gnu.stderr),
            String::from_utf8_lossy(&syskits.stderr)
        );
        assert_eq!(
            syskits.stdout,
            gnu.stdout,
            "flags: {flags:?}, GNU stdout: {}, syskits stdout: {}",
            String::from_utf8_lossy(&gnu.stdout),
            String::from_utf8_lossy(&syskits.stdout)
        );
        assert_eq!(
            syskits.stderr,
            gnu.stderr,
            "flags: {flags:?}, GNU stderr: {}, syskits stderr: {}",
            String::from_utf8_lossy(&gnu.stderr),
            String::from_utf8_lossy(&syskits.stderr)
        );
    }
}
