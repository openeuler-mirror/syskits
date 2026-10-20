use std::fs;
use std::process::{Command, Output};

use filetime::{FileTime, set_file_mtime};
use tempfile::TempDir;

fn installed_german_locale() -> Option<String> {
    let output = Command::new("/usr/bin/locale").arg("-a").output().ok()?;
    let locales = String::from_utf8(output.stdout).ok()?;
    for candidate in ["de_DE.utf8", "de_DE.UTF-8", "de_DE"] {
        if let Some(locale) = locales
            .lines()
            .find(|locale| locale.eq_ignore_ascii_case(candidate))
        {
            return Some(locale.to_string());
        }
    }
    None
}

fn installed_chinese_locale() -> Option<String> {
    let output = Command::new("/usr/bin/locale").arg("-a").output().ok()?;
    let locales = String::from_utf8(output.stdout).ok()?;
    for candidate in ["zh_CN.utf8", "zh_CN.UTF-8", "zh_CN"] {
        if let Some(locale) = locales
            .lines()
            .find(|locale| locale.eq_ignore_ascii_case(candidate))
        {
            return Some(locale.to_string());
        }
    }
    None
}

fn run_gnu(directory: &TempDir, arguments: &[&str]) -> Output {
    Command::new("/usr/bin/ls")
        .current_dir(directory.path())
        .env("LC_ALL", "C")
        .args(arguments)
        .output()
        .expect("run GNU ls")
}

fn run_syskits(directory: &TempDir, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(directory.path())
        .env("LC_ALL", "C")
        .arg("ls")
        .args(arguments)
        .output()
        .expect("run syskits ls")
}

#[test]
fn ls_zero_comma_wraps_with_nul_records() {
    let directory = TempDir::new().expect("tempdir");
    fs::create_dir(directory.path().join("d")).expect("create fixture directory");
    for name in ["a", "bb", "ccc", "dddd", "eeeee"] {
        fs::write(directory.path().join("d").join(name), b"").expect("create fixture file");
    }
    let arguments = ["--zero", "-m", "-w5", "d"];

    let gnu = run_gnu(&directory, &arguments);
    let syskits = run_syskits(&directory, &arguments);

    assert_eq!(syskits.status.code(), gnu.status.code());
    assert_eq!(syskits.stderr, gnu.stderr);
    assert_eq!(syskits.stdout, gnu.stdout);
}

#[test]
fn ls_hyperlink_percent_encodes_colons_in_file_uri() {
    let directory = TempDir::new().expect("tempdir");
    fs::write(directory.path().join("colon:"), b"").expect("create colon fixture");
    let arguments = ["--hyperlink=always", "-1", "colon:"];

    let gnu = run_gnu(&directory, &arguments);
    let syskits = run_syskits(&directory, &arguments);

    assert_eq!(syskits.status.code(), gnu.status.code());
    assert_eq!(syskits.stderr, gnu.stderr);
    assert_eq!(syskits.stdout, gnu.stdout);
    assert!(syskits.stdout.windows(3).any(|bytes| bytes == b"%3a"));
}

#[cfg(unix)]
#[test]
fn ls_hyperlink_normalizes_dangling_target_components() {
    use std::os::unix::fs::symlink;

    let directory = TempDir::new().expect("tempdir");
    fs::create_dir(directory.path().join("d")).expect("create link directory");
    fs::create_dir(directory.path().join("other")).expect("create target directory");
    symlink("../other/../missing", directory.path().join("d/link"))
        .expect("create dangling symlink");
    let arguments = ["--hyperlink=always", "-1", "d/link"];

    let gnu = run_gnu(&directory, &arguments);
    let syskits = run_syskits(&directory, &arguments);

    assert_eq!(syskits.status.code(), gnu.status.code());
    assert_eq!(syskits.stderr, gnu.stderr);
    assert_eq!(syskits.stdout, gnu.stdout);
}

#[test]
fn ls_localized_long_output_matches_gnu() {
    let Some(locale) = installed_german_locale() else {
        return;
    };
    let directory = TempDir::new().expect("tempdir");
    fs::create_dir(directory.path().join("d")).expect("create fixture directory");
    let file = directory.path().join("d/old");
    fs::write(&file, b"").expect("create fixture file");
    set_file_mtime(&file, FileTime::from_unix_time(1_000_000_000, 0))
        .expect("set fixture timestamp");
    let arguments = ["-ln", "d"];

    let gnu = Command::new("/usr/bin/ls")
        .current_dir(directory.path())
        .env("LC_ALL", &locale)
        .env("TZ", "UTC")
        .args(arguments)
        .output()
        .expect("run GNU ls");
    let syskits = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(directory.path())
        .env("LC_ALL", &locale)
        .env("TZ", "UTC")
        .arg("ls")
        .args(arguments)
        .output()
        .expect("run syskits ls");

    assert_eq!(syskits.status.code(), gnu.status.code());
    assert_eq!(syskits.stderr, gnu.stderr);
    assert_eq!(syskits.stdout, gnu.stdout);
}

#[test]
fn ls_chinese_long_time_pads_numeric_month() {
    let Some(locale) = installed_chinese_locale() else {
        return;
    };
    let directory = TempDir::new().expect("tempdir");
    fs::create_dir(directory.path().join("d")).expect("create fixture directory");
    let file = directory.path().join("d/old");
    fs::write(&file, b"").expect("create fixture file");
    set_file_mtime(&file, FileTime::from_unix_time(1_000_000_000, 0))
        .expect("set fixture timestamp");
    let arguments = ["-ln", "d"];

    let gnu = Command::new("/usr/bin/ls")
        .current_dir(directory.path())
        .env("LC_ALL", &locale)
        .env("TZ", "UTC")
        .args(arguments)
        .output()
        .expect("run GNU ls");
    let syskits = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(directory.path())
        .env("LC_ALL", &locale)
        .env("TZ", "UTC")
        .arg("ls")
        .args(arguments)
        .output()
        .expect("run syskits ls");

    assert_eq!(syskits.status.code(), gnu.status.code());
    assert_eq!(syskits.stderr, gnu.stderr);
    assert_eq!(syskits.stdout, gnu.stdout);
}

#[cfg(target_os = "linux")]
#[test]
fn ls_long_marks_acl_containing_only_an_added_mask() {
    let directory = TempDir::new().expect("tempdir");
    let file = directory.path().join("f");
    fs::write(&file, b"").expect("create fixture file");
    let Ok(setfacl) = Command::new("/usr/bin/setfacl")
        .args(["-m", "m::r--"])
        .arg(&file)
        .output()
    else {
        return;
    };
    if !setfacl.status.success() {
        return;
    }
    let arguments = ["-l", "--time-style=+%s", "f"];

    let gnu = run_gnu(&directory, &arguments);
    let syskits = run_syskits(&directory, &arguments);

    assert_eq!(syskits.status.code(), gnu.status.code());
    assert_eq!(syskits.stderr, gnu.stderr);
    assert_eq!(syskits.stdout, gnu.stdout);
    assert!(syskits.stdout.starts_with(b"-rw-r--r--+"));
}
