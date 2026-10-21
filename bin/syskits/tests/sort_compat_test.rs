use std::fs;
use std::process::Command;

#[cfg(target_family = "unix")]
use std::ffi::OsString;
#[cfg(target_family = "unix")]
use std::os::unix::ffi::OsStringExt;

use tempfile::TempDir;

#[test]
fn sort_debug_reports_text_ordering_rules() {
    let temp_dir = TempDir::new().expect("tempdir");
    let input = temp_dir.path().join("sortfile");
    fs::write(&input, b"11\n123\n22\n3333\n").expect("write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["sort", "--debug", input.to_str().expect("utf8 path")])
        .output()
        .expect("run syskits sort --debug");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"11\n__\n123\n___\n22\n__\n3333\n____\n");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("sort: text ordering performed using "),
        "stderr: {stderr}"
    );
    assert!(
        stderr.ends_with(" sorting rules\n") || stderr.ends_with(" simple byte comparison\n"),
        "stderr: {stderr}"
    );
}

#[test]
fn sort_debug_reports_simple_byte_comparison_in_c_locale() {
    let temp_dir = TempDir::new().expect("tempdir");
    let input = temp_dir.path().join("sortfile");
    fs::write(&input, b"b\na\n").expect("write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C")
        .args(["sort", "--debug", input.to_str().expect("utf8 path")])
        .output()
        .expect("run syskits sort --debug");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"a\n_\nb\n_\n");
    assert_eq!(
        output.stderr,
        b"sort: text ordering performed using simple byte comparison\n"
    );
}

#[test]
fn sort_debug_reports_numeric_decimal_point_note() {
    let temp_dir = TempDir::new().expect("tempdir");
    let input = temp_dir.path().join("fields");
    fs::write(&input, b"x 10\ny 2\nz 1\n").expect("write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("TZ", "UTC0")
        .args([
            "sort",
            "--debug",
            "-k",
            "2,2n",
            input.to_str().expect("utf8 path"),
        ])
        .output()
        .expect("run syskits sort --debug -k 2,2n");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        output.stdout,
        b"z 1\n  _\n___\ny 2\n  _\n___\nx 10\n  __\n____\n"
    );
    assert_eq!(
        output.stderr,
        b"sort: text ordering performed using simple byte comparison\nsort: note numbers use '.' as a decimal point in this locale\n"
    );
}

#[test]
#[cfg(target_family = "unix")]
fn sort_buffer_size_accepts_percent_suffix() {
    let temp_dir = TempDir::new().expect("tempdir");
    let input = temp_dir.path().join("data");
    fs::write(&input, b"b\na\n").expect("write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("TZ", "UTC0")
        .args([
            "sort",
            "--buffer-size=1%",
            input.to_str().expect("utf8 path"),
        ])
        .output()
        .expect("run syskits sort --buffer-size=1%");

    assert!(
        output.status.success(),
        "stdout: {}, stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"a\nb\n");
    assert_eq!(output.stderr, b"");
}

#[test]
fn sort_random_source_matches_gnu_seeded_md5_order() {
    let temp_dir = TempDir::new().expect("tempdir");
    let input = temp_dir.path().join("data");
    let random = temp_dir.path().join("random.bin");
    fs::write(&input, b"a\nb\nc\nd\ne\nf\n").expect("write input");
    fs::write(&random, vec![0_u8; 4096]).expect("write random source");
    let random_source_arg = format!("--random-source={}", random.display());

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("TZ", "UTC0")
        .args([
            "sort",
            "-R",
            random_source_arg.as_str(),
            input.to_str().expect("utf8 path"),
        ])
        .output()
        .expect("run syskits sort -R --random-source");

    assert!(
        output.status.success(),
        "stdout: {}, stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"a\nb\nf\nd\ne\nc\n");
    assert_eq!(output.stderr, b"");
}

#[test]
fn sort_invalid_sort_and_zero_parallel_exit_codes_match_gnu() {
    let temp_dir = TempDir::new().expect("tempdir");
    let input = temp_dir.path().join("input");
    fs::write(&input, b"x\n").expect("write input");
    let path = input.to_str().expect("utf8 path");

    for (args, expected) in [(["--sort=bad", path], 1), (["--parallel=0", path], 2)] {
        let gnu = Command::new("/usr/bin/sort")
            .env("LC_ALL", "C")
            .args(args)
            .output()
            .expect("run GNU sort");
        let syskits = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .env("LC_ALL", "C")
            .arg("sort")
            .args(args)
            .output()
            .expect("run syskits sort");

        assert_eq!(gnu.status.code(), Some(expected), "{}", args[0]);
        assert_eq!(syskits.status.code(), gnu.status.code(), "{}", args[0]);

        if args[0] == "--parallel=0" {
            assert_eq!(gnu.stdout, b"");
            assert_eq!(syskits.stdout, gnu.stdout);
            assert!(
                gnu.stderr
                    .ends_with(b": number in parallel must be nonzero\n")
            );
            assert!(
                syskits
                    .stderr
                    .ends_with(b": number in parallel must be nonzero\n")
            );
        }
    }
}

#[test]
#[cfg(target_family = "unix")]
fn sort_accepts_non_utf8_option_paths() {
    let temp_dir = TempDir::new().expect("tempdir");
    let input = temp_dir.path().join("input");
    fs::write(&input, b"b\na\n").expect("write input");

    let output_path = temp_dir
        .path()
        .join(OsString::from_vec(b"output-\xff".to_vec()));
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C")
        .args([
            OsString::from("sort"),
            OsString::from("-o"),
            output_path.clone().into_os_string(),
            input.clone().into_os_string(),
        ])
        .output()
        .expect("run sort with non-UTF-8 output path");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(&output_path).expect("read output"), b"a\nb\n");

    let random_path = temp_dir
        .path()
        .join(OsString::from_vec(b"random-\xfe".to_vec()));
    fs::write(&random_path, [0_u8; 16]).expect("write random source");
    let mut random_arg = OsString::from("--random-source=");
    random_arg.push(&random_path);
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C")
        .args([
            OsString::from("sort"),
            OsString::from("-R"),
            random_arg,
            input.clone().into_os_string(),
        ])
        .output()
        .expect("run sort with non-UTF-8 random source path");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let spill_input = temp_dir.path().join("spill-input");
    let records = (1..=1000)
        .rev()
        .map(|value| format!("{value}\n"))
        .collect::<String>();
    fs::write(&spill_input, records).expect("write spill input");
    let temporary_path = temp_dir
        .path()
        .join(OsString::from_vec(b"temporary-\xfd".to_vec()));
    fs::create_dir(&temporary_path).expect("create temporary directory");
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C")
        .args([
            OsString::from("sort"),
            OsString::from("-S1b"),
            OsString::from("-T"),
            temporary_path.into_os_string(),
            spill_input.into_os_string(),
        ])
        .output()
        .expect("run sort with non-UTF-8 temporary path");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn sort_stable_and_unique_compare_non_utf8_record_bytes() {
    let temp_dir = TempDir::new().expect("tempdir");
    let input = temp_dir.path().join("input");
    fs::write(&input, b"\xff\n\xfe\n").expect("write non-UTF-8 input");

    for option in ["-s", "-su"] {
        let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .env("LC_ALL", "C")
            .args(["sort", option, input.to_str().expect("utf8 path")])
            .output()
            .expect("run sort with non-UTF-8 records");

        assert!(
            output.status.success(),
            "{option}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"\xfe\n\xff\n", "{option}");
    }
}

#[test]
fn sort_external_spill_preserves_non_utf8_record_bytes() {
    let temp_dir = TempDir::new().expect("tempdir");
    let input = temp_dir.path().join("input");
    let mut records = Vec::new();
    for byte in (0x80_u8..=0xff).rev() {
        records.extend_from_slice(&[byte, b'\n']);
    }
    fs::write(&input, records).expect("write non-UTF-8 input");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C")
        .args(["sort", "-S1b", input.to_str().expect("utf8 input path")])
        .output()
        .expect("run external sort with non-UTF-8 records");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut expected = Vec::new();
    for byte in 0x80_u8..=0xff {
        expected.extend_from_slice(&[byte, b'\n']);
    }
    assert_eq!(output.stdout, expected);
}

#[test]
fn sort_uses_locale_blank_classification() {
    let temp_dir = TempDir::new().expect("tempdir");
    let cases: [(&[u8], &[&str], &[u8]); 3] = [
        (b"a\x0b2\nb\x0b1\n", &["-k2,2"], b"a\x0b2\nb\x0b1\n"),
        (b"\x0bZ\n A\n", &["-b"], b"\x0bZ\n A\n"),
        (b"\x0b9\n 1\n", &["-n"], b"\x0b9\n 1\n"),
    ];

    for (index, (records, options, expected)) in cases.into_iter().enumerate() {
        let input = temp_dir.path().join(format!("input-{index}"));
        fs::write(&input, records).expect("write blank classification input");
        let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .env("LC_ALL", "C")
            .arg("sort")
            .args(options)
            .arg(&input)
            .output()
            .expect("run sort blank classification case");

        assert!(
            output.status.success(),
            "case {index}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, expected, "case {index}");
    }
}

#[test]
fn sort_dictionary_order_uses_locale_character_classes() {
    let temp_dir = TempDir::new().expect("tempdir");
    let cases: [(&[u8], &[u8]); 2] = [
        (b"\x0bZ\n A\n", b" A\n\x0bZ\n"),
        (b"\xc3\xa42\na10\n", b"\xc3\xa42\na10\n"),
    ];

    for (index, (records, expected)) in cases.into_iter().enumerate() {
        let input = temp_dir.path().join(format!("input-{index}"));
        fs::write(&input, records).expect("write dictionary input");
        let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .env("LC_ALL", "C")
            .args(["sort", "-d", input.to_str().expect("utf8 path")])
            .output()
            .expect("run dictionary sort");

        assert!(
            output.status.success(),
            "case {index}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, expected, "case {index}");
    }
}

#[test]
fn sort_ignore_nonprinting_uses_locale_print_class() {
    let temp_dir = TempDir::new().expect("tempdir");
    let input = temp_dir.path().join("input");
    fs::write(&input, b"\xc2\xad2\na10\n").expect("write nonprinting input");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C")
        .args(["sort", "-i", input.to_str().expect("utf8 path")])
        .output()
        .expect("run ignore-nonprinting sort");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"\xc2\xad2\na10\n");
}

#[test]
#[cfg(target_os = "linux")]
fn sort_ignore_case_uses_single_character_locale_mapping() {
    let temp_dir = TempDir::new().expect("tempdir");
    let input = temp_dir.path().join("input");
    fs::write(&input, b"\xc3\x9f\nss\n").expect("write case mapping input");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C.utf8")
        .args(["sort", "-fs", input.to_str().expect("utf8 path")])
        .output()
        .expect("run ignore-case stable sort");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"ss\n\xc3\x9f\n");
}

#[test]
#[cfg(target_family = "unix")]
fn sort_field_separator_uses_c_locale_bytes() {
    let temp_dir = TempDir::new().expect("tempdir");
    let utf8_input = temp_dir.path().join("utf8-input");
    fs::write(&utf8_input, b"b\xc3\xa9c\na\xc3\xa9b\n").expect("write UTF-8 input");
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C")
        .args([
            OsString::from("sort"),
            OsString::from("-t"),
            OsString::from_vec(b"\xc3\xa9".to_vec()),
            OsString::from("-k2,2"),
            utf8_input.into_os_string(),
        ])
        .output()
        .expect("run sort with multibyte C-locale separator");
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(output.stdout, b"");

    let raw_input = temp_dir.path().join("raw-input");
    fs::write(&raw_input, b"b\xffc\na\xffb\n").expect("write raw input");
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C")
        .args([
            OsString::from("sort"),
            OsString::from("-t"),
            OsString::from_vec(vec![0xff]),
            OsString::from("-k2,2"),
            raw_input.into_os_string(),
        ])
        .output()
        .expect("run sort with raw C-locale separator");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"a\xffb\nb\xffc\n");
}

#[test]
fn sort_files0_from_rejects_dash_in_regular_file() {
    let temp_dir = TempDir::new().expect("tempdir");
    let list = temp_dir.path().join("list");
    fs::write(&list, b"-\0").expect("write file list");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C")
        .arg("sort")
        .arg(format!("--files0-from={}", list.display()))
        .output()
        .expect("run sort --files0-from");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(output.stdout, b"");
    assert!(
        output.stderr.ends_with(b"no file name of '-' allowed\n"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
