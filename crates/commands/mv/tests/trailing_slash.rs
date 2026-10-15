#![cfg(unix)]

use std::fs;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

fn run_mv(option: Option<&str>, source: &Path, destination: &Path) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mv"));
    command.arg0("mv").env("LC_ALL", "C");
    if let Some(option) = option {
        command.arg(option);
    }
    command
        .arg(source)
        .arg(format!("{}/", destination.display()))
        .output()
        .expect("run mv")
}

#[test]
fn trailing_slash_target_diagnostics_match_gnu() {
    for option in [None, Some("-T"), Some("--update")] {
        for source_is_directory in [false, true] {
            for destination_exists in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let source = directory.path().join("source");
                let destination = directory.path().join("destination");
                if source_is_directory {
                    fs::create_dir(&source).unwrap();
                } else {
                    fs::write(&source, b"source").unwrap();
                }
                if destination_exists {
                    fs::write(&destination, b"destination").unwrap();
                }

                let output = run_mv(option, &source, &destination);
                let label = format!("{option:?} {source_is_directory} {destination_exists}");

                if destination_exists {
                    assert_eq!(output.status.code(), Some(1), "{label}");
                    assert_eq!(
                        String::from_utf8(output.stderr).unwrap(),
                        format!(
                            "mv: cannot stat '{}/': Not a directory\n",
                            destination.display()
                        ),
                        "{label}"
                    );
                    assert!(source.exists(), "{label}");
                } else if source_is_directory {
                    assert!(output.status.success(), "{label}");
                    assert!(!source.exists(), "{label}");
                    assert!(destination.is_dir(), "{label}");
                } else {
                    assert_eq!(output.status.code(), Some(1), "{label}");
                    assert_eq!(
                        String::from_utf8(output.stderr).unwrap(),
                        format!(
                            "mv: cannot move '{}' to '{}/': Not a directory\n",
                            source.display(),
                            destination.display()
                        ),
                        "{label}"
                    );
                    assert!(source.exists(), "{label}");
                    assert!(!destination.exists(), "{label}");
                }
            }
        }
    }
}
