#![cfg(target_os = "linux")]

use std::process::Command;

#[test]
fn sync_option_syncs_with_and_without_a_path() {
    let temp_dir = tempfile::tempdir().expect("create temporary directory");

    for (index, (arguments, expected_sync_calls)) in [
        (["--sync"].as_slice(), 1),
        (["--sync", "/"].as_slice(), 1),
        (["--no-sync", "/"].as_slice(), 0),
    ]
    .into_iter()
    .enumerate()
    {
        let trace_path = temp_dir.path().join(format!("case-{index}.trace"));
        let output = match Command::new("strace")
            .args(["-qq", "-e", "trace=sync", "-o"])
            .arg(&trace_path)
            .arg(env!("CARGO_BIN_EXE_df"))
            .args(arguments)
            .env("LC_ALL", "C")
            .output()
        {
            Ok(output) => output,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => panic!("run df under strace: {error}"),
        };

        assert!(
            output.status.success(),
            "arguments={arguments:?}, {output:?}"
        );
        let trace = std::fs::read_to_string(&trace_path).expect("read strace output");
        assert_eq!(
            trace.lines().count(),
            expected_sync_calls,
            "arguments={arguments:?}, trace={trace:?}"
        );
    }
}
