#[cfg(all(target_os = "linux", target_env = "gnu"))]
const INPUT: &[u8] = b"\xe9\nE\ne\n";

#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[test]
fn single_byte_locale_regexes_follow_gnu_character_and_collation_rules() {
    use std::process::Command;

    if !locale_is_available("en_US.iso88591") {
        return;
    }

    for (pattern, first_split, second_split) in [
        ("/^[[:alpha:]]$/", b"".as_slice(), INPUT),
        ("/^[a-z]$/", b"".as_slice(), INPUT),
        ("/^[^a-z]$/", b"\xe9\n".as_slice(), b"E\ne\n".as_slice()),
    ] {
        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        std::fs::write(temp_dir.path().join("input"), INPUT).expect("write input");

        let output = Command::new("bash")
            .args(["-c", "exec -a csplit \"$0\" \"$@\""])
            .arg(env!("CARGO_BIN_EXE_csplit"))
            .args(["-q", "input", pattern])
            .current_dir(temp_dir.path())
            .env("LC_ALL", "en_US.iso88591")
            .output()
            .expect("run csplit");

        assert!(output.status.success(), "{pattern}: {output:?}");
        assert!(output.stdout.is_empty(), "{pattern}: {output:?}");
        assert!(output.stderr.is_empty(), "{pattern}: {output:?}");
        assert_eq!(
            std::fs::read(temp_dir.path().join("xx00")).expect("read first split"),
            first_split,
            "{pattern}"
        );
        assert_eq!(
            std::fs::read(temp_dir.path().join("xx01")).expect("read second split"),
            second_split,
            "{pattern}"
        );
        assert_eq!(output_file_count(temp_dir.path()), 2, "{pattern}");
    }
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn output_file_count(directory: &std::path::Path) -> usize {
    std::fs::read_dir(directory)
        .expect("read output directory")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("xx"))
        .count()
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
