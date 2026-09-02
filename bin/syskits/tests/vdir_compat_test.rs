use std::fs;
use std::process::Command;

use tempfile::TempDir;

#[test]
fn vdir_sort_size_breaks_equal_sizes_by_name() {
    let temp_dir = TempDir::new().expect("tempdir");
    for name in ["c-file", "b-file", "a-file"] {
        fs::write(temp_dir.path().join(name), b"same").expect("write fixture");
    }

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args([
            "vdir",
            "--sort=size",
            temp_dir.path().to_str().expect("utf8 path"),
        ])
        .output()
        .expect("run syskits vdir");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stderr, b"");

    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    let listed_names = stdout
        .lines()
        .filter(|line| !line.starts_with("total "))
        .filter_map(|line| line.split_whitespace().last())
        .collect::<Vec<_>>();

    assert_eq!(listed_names, ["a-file", "b-file", "c-file"]);
}
