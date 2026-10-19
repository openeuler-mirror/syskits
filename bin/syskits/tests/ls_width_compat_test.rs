use std::fs;
use std::process::{Command, Output};

use tempfile::TempDir;

fn run_gnu(directory: &TempDir, arguments: &[&str]) -> Output {
    Command::new("/usr/bin/ls")
        .current_dir(directory.path())
        .env("LC_ALL", "C.UTF-8")
        .args(arguments)
        .output()
        .expect("run GNU ls")
}

fn run_syskits(directory: &TempDir, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(directory.path())
        .env("LC_ALL", "C.UTF-8")
        .arg("ls")
        .args(arguments)
        .output()
        .expect("run syskits ls")
}

#[test]
fn ls_uses_codepoint_width_for_emoji_sequences() {
    let directory = TempDir::new().expect("tempdir");
    let keycap_dir = directory.path().join("keycap");
    let zwj_dir = directory.path().join("zwj");
    fs::create_dir(&keycap_dir).expect("create keycap directory");
    fs::create_dir(&zwj_dir).expect("create ZWJ directory");

    fs::write(keycap_dir.join("a"), b"").expect("create ASCII keycap peer");
    fs::write(keycap_dir.join("1\u{fe0f}\u{20e3}"), b"").expect("create keycap fixture");
    fs::write(zwj_dir.join("a"), b"").expect("create ASCII ZWJ peer");
    fs::write(zwj_dir.join("\u{1f469}\u{200d}\u{1f4bb}"), b"").expect("create ZWJ fixture");

    for arguments in [
        vec!["-C", "-T0", "-w5", "--quoting-style=literal", "keycap"],
        vec!["-C", "-T0", "-w5", "--quoting-style=literal", "zwj"],
        vec!["--sort=width", "-1", "--quoting-style=literal", "keycap"],
    ] {
        let gnu = run_gnu(&directory, &arguments);
        let syskits = run_syskits(&directory, &arguments);

        assert_eq!(syskits.status.code(), gnu.status.code(), "{arguments:?}");
        assert_eq!(syskits.stderr, gnu.stderr, "{arguments:?}");
        assert_eq!(syskits.stdout, gnu.stdout, "{arguments:?}");
    }
}

#[test]
fn ls_ignores_invalid_columns_when_width_is_unused() {
    let directory = TempDir::new().expect("tempdir");
    fs::write(directory.path().join("a"), b"").expect("create fixture");

    for arguments in [vec!["-1", "."], vec!["-l", "--time-style=+%s", "."]] {
        let gnu = Command::new("/usr/bin/ls")
            .current_dir(directory.path())
            .env("LC_ALL", "C")
            .env("COLUMNS", "bad")
            .args(&arguments)
            .output()
            .expect("run GNU ls");
        let syskits = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .current_dir(directory.path())
            .env("LC_ALL", "C")
            .env("COLUMNS", "bad")
            .arg("ls")
            .args(&arguments)
            .output()
            .expect("run syskits ls");

        assert_eq!(syskits.status.code(), gnu.status.code(), "{arguments:?}");
        assert_eq!(syskits.stderr, gnu.stderr, "{arguments:?}");
    }
}
