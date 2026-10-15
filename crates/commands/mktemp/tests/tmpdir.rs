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
