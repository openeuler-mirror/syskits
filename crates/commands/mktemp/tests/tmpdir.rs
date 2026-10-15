use std::process::Command;

fn run_mktemp(args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_mktemp"))
        .args(args)
        .env("TMPDIR", "")
        .output()
        .expect("run mktemp");

    assert!(
        output.status.success(),
        "mktemp failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout)
        .expect("mktemp stdout is UTF-8")
        .trim_end()
        .to_owned()
}

#[test]
fn empty_tmpdir_uses_the_gnu_default_directory() {
    for args in [
        &["-u"][..],
        &["-u", "--tmpdir", "empty-tmpdir.XXX"],
        &["-u", "-t", "empty-tmpdir.XXX"],
        &[
            "-u",
            "-t",
            "-p",
            "/tmp/mktemp-empty-tmpdir",
            "empty-tmpdir.XXX",
        ],
    ] {
        let output = run_mktemp(args);
        let expected_prefix = if args.contains(&"-p") {
            "/tmp/mktemp-empty-tmpdir/"
        } else {
            "/tmp/"
        };
        assert!(
            output.starts_with(expected_prefix),
            "expected {output:?} to start with {expected_prefix:?}"
        );
    }
}

#[test]
fn last_tmpdir_option_wins() {
    for (args, expected_prefix) in [
        (
            &[
                "-u",
                "-p",
                "/tmp/mktemp-p-first",
                "-p",
                "/tmp/mktemp-p-last",
                "repeat-tmpdir.XXX",
            ][..],
            "/tmp/mktemp-p-last/",
        ),
        (
            &[
                "-u",
                "--tmpdir=/tmp/mktemp-long-first",
                "--tmpdir=/tmp/mktemp-long-last",
                "repeat-tmpdir.XXX",
            ][..],
            "/tmp/mktemp-long-last/",
        ),
        (
            &[
                "-u",
                "-p",
                "/tmp/mktemp-p-first",
                "--tmpdir=/tmp/mktemp-long-last",
                "repeat-tmpdir.XXX",
            ][..],
            "/tmp/mktemp-long-last/",
        ),
        (
            &[
                "-u",
                "--tmpdir=/tmp/mktemp-long-first",
                "-p",
                "/tmp/mktemp-p-last",
                "repeat-tmpdir.XXX",
            ][..],
            "/tmp/mktemp-p-last/",
        ),
    ] {
        let output = run_mktemp(args);
        assert!(
            output.starts_with(expected_prefix),
            "expected {output:?} to start with {expected_prefix:?}"
        );
    }
}

#[test]
fn last_suffix_option_wins() {
    let output = run_mktemp(&[
        "-u",
        "--suffix=.first",
        "--suffix=.last",
        "suffix-repeat.XXX",
    ]);
    assert!(output.starts_with("suffix-repeat."));
    assert!(output.ends_with(".last"));

    let output = run_mktemp(&["-u", "--suffix=.first", "--suffix=", "suffix-repeat.XXX"]);
    assert!(output.starts_with("suffix-repeat."));
    assert!(!output.ends_with(".first"));
}
