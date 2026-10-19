use std::fs;
use std::process::{Command, Output};

use filetime::{FileTime, set_file_mtime};
use tempfile::TempDir;

fn run_gnu(directory: &TempDir, environment: (&str, &str), arguments: &[&str]) -> Output {
    Command::new("/usr/bin/ls")
        .current_dir(directory.path())
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C")
        .env(environment.0, environment.1)
        .args(arguments)
        .output()
        .expect("run GNU ls")
}

fn run_syskits(directory: &TempDir, environment: (&str, &str), arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(directory.path())
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C")
        .env(environment.0, environment.1)
        .arg("ls")
        .args(arguments)
        .output()
        .expect("run syskits ls")
}

#[test]
fn ls_environment_block_size_autoscales_with_suffixes() {
    let directory = TempDir::new().expect("tempdir");
    fs::create_dir(directory.path().join("d")).expect("create fixture directory");
    fs::write(directory.path().join("d/f"), vec![0; 12_345]).expect("create fixture file");

    for key in ["LS_BLOCK_SIZE", "BLOCK_SIZE"] {
        for value in ["human-readable", "si"] {
            let environment = (key, value);
            let arguments = ["-s1", "d"];
            let gnu = run_gnu(&directory, environment, &arguments);
            let syskits = run_syskits(&directory, environment, &arguments);

            assert_eq!(syskits.status.code(), gnu.status.code(), "{key}={value}");
            assert_eq!(syskits.stderr, gnu.stderr, "{key}={value}");
            assert_eq!(syskits.stdout, gnu.stdout, "{key}={value}");
        }
    }
}

#[test]
fn ls_blocksize_suffix_does_not_leak_into_long_file_size() {
    let directory = TempDir::new().expect("tempdir");
    fs::create_dir(directory.path().join("d")).expect("create fixture directory");
    let file = directory.path().join("d/f");
    fs::write(&file, vec![0; 12_345]).expect("create fixture file");
    set_file_mtime(&file, FileTime::from_unix_time(1_000_000_000, 0))
        .expect("set fixture timestamp");

    for value in ["K", "kB"] {
        let environment = ("BLOCKSIZE", value);
        let arguments = ["-lns", "--time-style=+%s", "d"];
        let gnu = run_gnu(&directory, environment, &arguments);
        let syskits = run_syskits(&directory, environment, &arguments);

        assert_eq!(
            syskits.status.code(),
            gnu.status.code(),
            "BLOCKSIZE={value}"
        );
        assert_eq!(syskits.stderr, gnu.stderr, "BLOCKSIZE={value}");
        assert_eq!(syskits.stdout, gnu.stdout, "BLOCKSIZE={value}");
    }
}
