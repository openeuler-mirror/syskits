#[cfg(unix)]
use std::ffi::OsString;
#[cfg(unix)]
use std::fs;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;
use std::process::{Command, Stdio};

#[test]
fn tsort_preserves_cycle_diagnostic_output_order() {
    let output = Command::new("bash")
        .args([
            "-c",
            "printf 'a b\\nb a\\n' | \"$1\" tsort 2>&1",
            "bash",
            env!("CARGO_BIN_EXE_syskits"),
        ])
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .output()
        .expect("run syskits tsort");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    assert_eq!(
        output.stdout,
        b"tsort: -: input contains a loop:\ntsort: a\ntsort: b\na\nb\n"
    );
}

#[test]
fn tsort_preserves_duplicate_relations_in_cycles() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .arg("tsort")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run syskits tsort");

    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"a b\na b\nb a\nb a\n")
        .expect("write tsort input");

    let output = child.wait_with_output().expect("wait for syskits tsort");
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stderr.matches("input contains a loop").count(), 2);
}

#[test]
fn tsort_preserves_non_utf8_cycle_tokens_in_diagnostics() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .arg("tsort")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run syskits tsort");

    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"\xff a\na \xff\n")
        .expect("write tsort input");

    let output = child.wait_with_output().expect("wait for syskits tsort");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        output.stderr,
        b"tsort: -: input contains a loop:\ntsort: a\ntsort: \xff\n"
    );
}

#[cfg(unix)]
#[test]
fn tsort_accepts_non_utf8_file_operand() {
    let temp_dir = tempfile::tempdir().expect("create temp directory");
    let mut file_path = temp_dir.path().to_path_buf();
    file_path.push(OsString::from_vec(b"graph-\xff".to_vec()));
    fs::write(&file_path, b"a b\n").expect("write tsort input");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .arg("tsort")
        .arg(&file_path)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .output()
        .expect("run syskits tsort");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"a\nb\n");
    assert!(output.stderr.is_empty());
}
