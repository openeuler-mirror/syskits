#![cfg(feature = "feat_data_pipeline")]

use std::fs;
use std::io::Write;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

fn run_data_with_args(args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .arg("data")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn syskits");

    child
        .stdin
        .as_mut()
        .expect("stdin handle")
        .write_all(stdin.as_bytes())
        .expect("write stdin");

    child.wait_with_output().expect("wait for syskits")
}

fn run_data(expr: &str, stdin: &str) -> Output {
    run_data_with_args(&[expr], stdin)
}

fn run_data_classic(expr: &str, stdin: &str) -> Output {
    run_data_with_args(&["format=classic", expr], stdin)
}

fn run_direct(args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn syskits");

    child
        .stdin
        .as_mut()
        .expect("stdin handle")
        .write_all(stdin.as_bytes())
        .expect("write stdin");

    child.wait_with_output().expect("wait for syskits")
}

fn assert_data_classic_matches_direct(direct_args: &[&str], data_expr: &str, stdin: &str) {
    let expected = run_direct(direct_args, stdin);
    let actual = run_data_classic(data_expr, stdin);

    assert_eq!(actual.status.code(), expected.status.code());
    assert_eq!(
        String::from_utf8_lossy(&actual.stdout),
        String::from_utf8_lossy(&expected.stdout)
    );
    assert_eq!(
        String::from_utf8_lossy(&actual.stderr),
        String::from_utf8_lossy(&expected.stderr)
    );
}

#[test]
fn head_reads_pipeline_input() {
    let output = run_data_classic("from text | head -n 1", "a\nb\n");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "a\n");
}

#[test]
fn sort_reads_pipeline_input() {
    let output = run_data_classic("from text | sort", "b\na\n");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "a\nb\n");
}

#[test]
fn cat_reads_pipeline_input() {
    let output = run_data_classic("from text | cat", "alpha\nbeta\n");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "alpha\nbeta\n");
}

#[test]
fn tee_reads_pipeline_input_and_writes_target() {
    let temp_dir = TempDir::new().expect("tempdir");
    let path = temp_dir.path().join("tee.out");
    let expr = format!("from text | tee {}", path.display());

    let output = run_data_classic(&expr, "alpha\nbeta\n");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "alpha\nbeta\n");
    assert_eq!(
        fs::read_to_string(&path).expect("read tee output"),
        "alpha\nbeta\n"
    );
}

#[test]
fn fold_reads_pipeline_input() {
    let output = run_data_classic("from text | fold -w 3", "abcdef\n");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "abc\ndef\n");
}

#[test]
fn tail_reads_pipeline_input() {
    assert_data_classic_matches_direct(
        &["tail", "-n", "1"],
        "from text | tail -n 1",
        "alpha\nbeta\n",
    );
}

#[test]
fn uniq_reads_pipeline_input() {
    assert_data_classic_matches_direct(&["uniq"], "from text | uniq", "alpha\nalpha\nbeta\n");
}

#[test]
fn unexpand_reads_pipeline_input() {
    assert_data_classic_matches_direct(
        &["unexpand", "-a"],
        "from text | unexpand -a",
        "    alpha\n",
    );
}

#[test]
fn tr_reads_pipeline_input() {
    assert_data_classic_matches_direct(&["tr", "a-z", "A-Z"], "from text | tr a-z A-Z", "alpha\n");
}

#[test]
fn tsort_reads_pipeline_input() {
    assert_data_classic_matches_direct(&["tsort"], "from text | tsort", "a b\nb c\n");
}

#[test]
fn cut_reads_pipeline_input() {
    assert_data_classic_matches_direct(
        &["cut", "-c", "1"],
        "from text | cut -c 1",
        "alpha\nbeta\n",
    );
}

#[test]
fn nl_reads_pipeline_input() {
    assert_data_classic_matches_direct(&["nl"], "from text | nl", "alpha\nbeta\n");
}

#[test]
fn fmt_reads_pipeline_input() {
    assert_data_classic_matches_direct(
        &["fmt", "-w", "12"],
        "from text | fmt -w 12",
        "alpha beta gamma delta\n",
    );
}

#[test]
fn tac_reads_pipeline_input() {
    assert_data_classic_matches_direct(&["tac"], "from text | tac", "alpha\nbeta\n");
}

#[test]
fn od_reads_pipeline_input() {
    assert_data_classic_matches_direct(&["od", "-An", "-tx1"], "from text | od -An -tx1", "A\n");
}

#[test]
fn shuf_reads_pipeline_input_without_echo_or_input_range() {
    assert_data_classic_matches_direct(&["shuf"], "from text | shuf", "solo\n");
}

#[test]
fn shuf_echo_does_not_consume_pipeline_input() {
    let output = run_data_classic("from text | shuf -e alpha", "ignored\n");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "alpha\n");
}

#[test]
fn join_reads_pipeline_input_for_dash_operand() {
    let temp_dir = TempDir::new().expect("tempdir");
    let right = temp_dir.path().join("right.txt");
    fs::write(&right, "a X\nb Y\n").expect("write join fixture");
    let right = right.display().to_string();
    let expr = format!("from text | join - {right}");

    assert_data_classic_matches_direct(&["join", "-", &right], &expr, "a 1\nb 2\n");
}

#[test]
fn sum_reads_pipeline_input() {
    assert_data_classic_matches_direct(&["sum"], "from text | sum", "abc\n");
}

#[test]
fn factor_reads_pipeline_input() {
    assert_data_classic_matches_direct(&["factor"], "from text | factor", "12\n15\n");
}

#[test]
fn hashsum_reads_pipeline_input() {
    assert_data_classic_matches_direct(
        &["hashsum", "--sha256"],
        "from text | hashsum --sha256",
        "abc\n",
    );
}

#[test]
fn cksum_reads_pipeline_input() {
    assert_data_classic_matches_direct(&["cksum"], "from text | cksum", "abc\n");
}

#[test]
fn more_reads_pipeline_input() {
    assert_data_classic_matches_direct(&["more"], "from text | more", "alpha\nbeta\n");
}

#[test]
fn numfmt_reads_pipeline_input() {
    assert_data_classic_matches_direct(
        &["numfmt", "--to", "si"],
        "from text | numfmt --to si",
        "1000\n",
    );
}

#[test]
fn numfmt_accepts_gnu_long_flag_equals_value() {
    assert_data_classic_matches_direct(
        &["numfmt", "--to=si"],
        "from text | numfmt --to=si",
        "1000\n",
    );
}

#[test]
fn dircolors_reads_pipeline_input_for_dash_operand() {
    assert_data_classic_matches_direct(
        &["dircolors", "-b", "-"],
        "from text | dircolors -b -",
        "TERM xterm\nDIR 01;34\n",
    );
}

#[test]
fn du_reads_files0_from_pipeline_input() {
    let temp_dir = TempDir::new().expect("tempdir");
    let path = temp_dir.path().join("du-input.txt");
    fs::write(&path, "abc\n").expect("write du fixture");
    let path = path.display().to_string();
    let expr = "from text | du --bytes --summarize --files0-from -";
    let stdin = format!("{path}\0");

    assert_data_classic_matches_direct(
        &["du", "--bytes", "--summarize", "--files0-from", "-"],
        expr,
        &stdin,
    );
}

#[test]
fn base64_preserves_text_lines() {
    let output = run_data_classic("from text | base64", "hello world\n");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim_end(),
        "aGVsbG8gd29ybGQK"
    );
}

#[test]
fn base64_json_pipeline_input_is_information_rich() {
    let output = run_data_with_args(&["format=json", "from text | base64"], "hello world\n");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"input\":\"stdin\""), "stdout: {stdout:?}");
    assert!(
        stdout.contains("\"output_text\":\"aGVsbG8gd29ybGQK\""),
        "stdout: {stdout:?}"
    );
}

#[test]
fn wc_supports_line_flag_in_pipeline_mode() {
    let output = run_data("from text | wc -l | to json", "a\nb\n");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"lines\": 2") || stdout.contains("\"lines\":2"));
}

#[test]
fn wc_total_only_reads_pipeline_input() {
    let output = run_data("from text | wc --total only | to json", "a\nb\n");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"lines\": 2") || stdout.contains("\"lines\":2"));
}

#[cfg(unix)]
#[test]
fn run_external_timeout_kills_child_after_stdout_eof() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args([
            "data",
            "run-external sh -c 'exec 1>&-; sleep 60' --stdout-mode raw --timeout-ms 100",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn syskits");

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(_status) = child.try_wait().expect("poll syskits") {
            let output = child.wait_with_output().expect("collect syskits output");
            assert!(
                !output.status.success(),
                "timeout command unexpectedly succeeded: stdout={}, stderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                stderr.contains("timed out"),
                "expected timeout diagnostic, stderr: {stderr}"
            );
            return;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("run-external timeout did not terminate after stdout EOF");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn data_format_raw_preserves_external_raw_stdout() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("SYSKITS_DATA_FORMAT", "raw")
        .args(["data", "run-external printf abc --stdout-mode raw"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn syskits");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"abc");
}

#[test]
fn data_help_matches_enabled_workflow_feature() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "--help"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn syskits");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);

    #[cfg(feature = "feat_data_workflow")]
    assert!(
        stdout.contains("syskits data -f <workflow.skd>"),
        "stdout: {stdout}"
    );

    #[cfg(not(feature = "feat_data_workflow"))]
    assert!(
        !stdout.contains("syskits data -f <workflow.skd>"),
        "stdout: {stdout}"
    );
}
