use std::fs;
use std::process::Command;

use tempfile::TempDir;

#[test]
fn invalid_tabsize_is_fatal_for_ls_dir_and_vdir() {
    for command in ["ls", "dir", "vdir"] {
        let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .args([command, "--tabsize=bad", "."])
            .output()
            .unwrap_or_else(|err| panic!("run syskits {command}: {err}"));

        assert_eq!(output.status.code(), Some(2), "{command}");
        assert!(
            output.stdout.is_empty(),
            "{command}: stdout should be empty, got {:?}",
            String::from_utf8_lossy(&output.stdout)
        );

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("invalid tab size") && stderr.contains("bad"),
            "{command}: stderr was {stderr:?}",
        );
    }
}

#[test]
fn quoted_ls_grid_uses_tabs_like_gnu() {
    let directory = TempDir::new().expect("tempdir");
    for name in [
        "plain",
        "space name",
        "bracket[",
        "alpha",
        "beta",
        "gamma",
        "delta",
        "epsilon",
    ] {
        fs::write(directory.path().join(name), b"").expect("create grid fixture");
    }
    let arguments = ["-C", "-w40", "--quoting-style=shell"];

    let gnu = Command::new("/usr/bin/ls")
        .current_dir(directory.path())
        .env("LC_ALL", "C")
        .args(arguments)
        .output()
        .expect("run GNU ls");
    let syskits = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(directory.path())
        .env("LC_ALL", "C")
        .arg("ls")
        .args(arguments)
        .output()
        .expect("run syskits ls");

    assert!(
        gnu.stdout.contains(&b'\t'),
        "GNU fixture must exercise tabs"
    );
    assert_eq!(syskits.status.code(), gnu.status.code());
    assert_eq!(syskits.stderr, gnu.stderr);
    assert_eq!(syskits.stdout, gnu.stdout);
}
