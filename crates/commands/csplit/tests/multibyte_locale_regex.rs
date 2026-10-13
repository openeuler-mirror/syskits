#[cfg(all(target_os = "linux", target_env = "gnu"))]
const INPUT: &[u8] = b"\xe4\xb8\xad\n\xc3\xa9\nA\na\n\xe3\x80\x82\n";

#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[test]
fn multibyte_locale_regexes_follow_gnu_character_semantics() {
    use std::process::Command;

    if !locale_is_available("C.utf8") {
        return;
    }

    for (pattern, first_split, second_split) in [
        ("/^[[:alpha:]]$/", b"".as_slice(), INPUT),
        ("/^.$/", b"".as_slice(), INPUT),
        (
            "/^[[:punct:]]$/",
            b"\xe4\xb8\xad\n\xc3\xa9\nA\na\n".as_slice(),
            b"\xe3\x80\x82\n".as_slice(),
        ),
    ] {
        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        std::fs::write(temp_dir.path().join("input"), INPUT).expect("write input");

        let output = Command::new("bash")
            .args(["-c", "exec -a csplit \"$0\" \"$@\""])
            .arg(env!("CARGO_BIN_EXE_csplit"))
            .args(["-q", "input", pattern])
            .current_dir(temp_dir.path())
            .env("LC_ALL", "C.utf8")
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
#[test]
fn multibyte_locale_keeps_gnu_empty_range_syntax() {
    use std::process::Command;

    if !locale_is_available("C.utf8") {
        return;
    }

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    std::fs::write(temp_dir.path().join("input"), INPUT).expect("write input");

    let output = Command::new("bash")
        .args(["-c", "exec -a csplit \"$0\" \"$@\""])
        .arg(env!("CARGO_BIN_EXE_csplit"))
        .args(["-q", "input", "/[z-a]/"])
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C.utf8")
        .output()
        .expect("run csplit");

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).ends_with("match not found\n"),
        "{output:?}"
    );
    assert_eq!(output_file_count(temp_dir.path()), 0);
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
