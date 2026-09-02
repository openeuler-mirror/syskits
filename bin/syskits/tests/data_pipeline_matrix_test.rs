#![cfg(feature = "feat_data_pipeline")]

use std::ffi::OsString;
use std::fs;
use std::io::{ErrorKind, Write};
use std::process::{Command, Output, Stdio};

use ctengine::error::CtDiagnosticError;
use ctengine::execution::{OutputFormat, OutputProfile};
use ctengine::interpreter::try_print_pipeline_data_with_profile;
use ctengine::{CommandRegistry, DataCommand, DataCommandFactory, DataEngineContext, exit_code};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::DataSignature;
use tempfile::TempDir;

fn from_factory() -> Box<dyn DataCommand> {
    Box::new(cmd_from::CmdFrom)
}

fn to_factory() -> Box<dyn DataCommand> {
    Box::new(cmd_to::CmdTo)
}

fn timeout_factory() -> Box<dyn DataCommand> {
    Box::new(TimeoutCmd)
}

fn make_registry() -> CommandRegistry {
    let factories: Vec<(&'static str, DataCommandFactory)> = vec![
        ("from", from_factory as DataCommandFactory),
        ("to", to_factory as DataCommandFactory),
        ("timeout-cmd", timeout_factory as DataCommandFactory),
    ];
    CommandRegistry::from_factories(&factories)
}

#[derive(Default)]
struct TimeoutCmd;

impl DataCommand for TimeoutCmd {
    fn signature(&self) -> DataSignature {
        DataSignature::new("timeout-cmd", "always returns timeout")
            .input(CtType::Any)
            .output(CtType::Nothing)
    }

    fn run(
        &self,
        _call: &ctsig::DataCall,
        _input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        Err(CtDiagnosticError::simple("timeout for matrix test").with_code(exit_code::TIMEOUT))
    }
}

fn sample_list_record_data() -> CtPipelineData {
    CtPipelineData::Value(
        CtValue::List(vec![
            CtValue::Record(vec![
                ("name".to_string(), CtValue::String("a.txt".to_string())),
                ("size".to_string(), CtValue::Size(42)),
            ]),
            CtValue::Record(vec![
                ("name".to_string(), CtValue::String("b.txt".to_string())),
                ("size".to_string(), CtValue::Size(1024)),
            ]),
        ]),
        CtPipelineMetadata::default(),
    )
}

fn write_pr_fixture() -> (TempDir, String) {
    let temp_dir = TempDir::new().expect("tempdir");
    let path = temp_dir.path().join("pr.txt");
    fs::write(&path, "alpha\nbeta\n").expect("write pr fixture");
    (temp_dir, path.display().to_string())
}

fn write_ptx_fixture() -> (TempDir, String) {
    let temp_dir = TempDir::new().expect("tempdir");
    let path = temp_dir.path().join("ptx.txt");
    fs::write(&path, "alpha beta gamma\n").expect("write ptx fixture");
    (temp_dir, path.display().to_string())
}

fn write_expand_fixture() -> (TempDir, String) {
    let temp_dir = TempDir::new().expect("tempdir");
    let path = temp_dir.path().join("expand.txt");
    fs::write(&path, "a\tb\n\tc\n").expect("write expand fixture");
    (temp_dir, path.display().to_string())
}

fn assert_same_process_output(actual: &Output, expected: &Output) {
    assert_eq!(actual.status.code(), expected.status.code());
    assert_eq!(actual.stdout, expected.stdout);
    assert_eq!(actual.stderr, expected.stderr);
}

fn run_syskits_with_stdin(args: &[&str], stdin: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn syskits");

    let mut child_stdin = child.stdin.take().expect("stdin handle");
    if let Err(err) = child_stdin.write_all(stdin)
        && err.kind() != ErrorKind::BrokenPipe
    {
        panic!("write stdin: {err}");
    }
    drop(child_stdin);

    child.wait_with_output().expect("wait for syskits")
}

fn assert_data_classic_matches_direct(args: &[&str]) {
    assert_data_classic_matches_direct_with_data_args(args[0], args, args);
}

fn assert_data_classic_matches_direct_with_data_args(
    label: &str,
    direct_args: &[&str],
    data_command_args: &[&str],
) {
    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(direct_args)
        .output()
        .expect("run direct syskits command");

    let mut data_args = vec!["data", "format=classic"];
    data_args.extend_from_slice(data_command_args);
    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(data_args)
        .output()
        .expect("run syskits data classic command");

    assert_eq!(
        out.status.code(),
        expected.status.code(),
        "exit code mismatch for {label}"
    );
    assert_eq!(out.stdout, expected.stdout, "stdout mismatch for {label}");
    assert_eq!(out.stderr, expected.stderr, "stderr mismatch for {label}");
}

fn split_chunks(dir: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let mut chunks = fs::read_dir(dir)
        .expect("read split output dir")
        .filter_map(|entry| {
            let entry = entry.expect("read split dir entry");
            let file_name = entry.file_name().to_string_lossy().into_owned();
            if !file_name.starts_with("chunk_") {
                return None;
            }
            Some((
                file_name,
                fs::read(entry.path()).expect("read split output file"),
            ))
        })
        .collect::<Vec<_>>();
    chunks.sort_by(|left, right| left.0.cmp(&right.0));
    chunks
}

#[test]
fn test_output_profile_matrix_single_axis_formats() {
    let tty_modes = [false, true];
    let formats = [
        OutputFormat::Classic,
        OutputFormat::Auto,
        OutputFormat::Text,
        OutputFormat::Table,
        OutputFormat::Json,
    ];

    for stdout_is_tty in tty_modes {
        for format in formats {
            let profile = OutputProfile {
                format,
                stdout_is_tty,
                use_pager: false,
            };
            let out = try_print_pipeline_data_with_profile(sample_list_record_data(), &profile);
            assert!(out.is_ok(), "matrix failed for profile: {profile:?}");
        }
    }
}

#[test]
fn test_run_data_entry_status_matrix_success_failure_timeout() {
    let cases = [
        (
            r#"from json "{\"name\":\"CTyunOS\"}" | to json"#,
            exit_code::SUCCESS,
        ),
        ("__syskits_no_such_cmd_xyz__", exit_code::RUNTIME_ERROR),
        ("| bad", exit_code::USAGE_ERROR),
        ("timeout-cmd", exit_code::TIMEOUT),
    ];

    for (expr, expected) in cases {
        let args = vec![OsString::from(expr)];
        let code = ctengine::run_data_entry_with_registry(&args, make_registry());
        assert_eq!(code, expected, "unexpected exit code for expr `{expr}`");
    }
}

#[test]
fn data_phase_a_gnu_flags_match_direct_classic_output() {
    let temp_dir = TempDir::new().expect("tempdir");
    let file = temp_dir.path().join("sample.txt");
    let link = temp_dir.path().join("sample.link");
    fs::write(&file, "alpha\n").expect("write phase-a fixture");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&file, &link).expect("create phase-a symlink");
    #[cfg(not(unix))]
    fs::write(&link, "alpha\n").expect("write phase-a link fallback");

    let dir = temp_dir.path().display().to_string();
    let file = file.display().to_string();
    let link = link.display().to_string();

    assert_data_classic_matches_direct(&["basename", "-s", ".txt", &file]);
    assert_data_classic_matches_direct_with_data_args(
        "date",
        &["date", "-u", "+%Y"],
        &["date -u \"+%Y\""],
    );
    assert_data_classic_matches_direct(&["df", "-a", "--output=source", "/"]);
    assert_data_classic_matches_direct_with_data_args(
        "env",
        &["env", "-0", "-i", "FOO=bar"],
        &["env -0 -i \"FOO=bar\""],
    );
    assert_data_classic_matches_direct(&["ls", "-a", &dir]);
    assert_data_classic_matches_direct(&["nproc", "--all"]);
    assert_data_classic_matches_direct_with_data_args(
        "printenv",
        &["printenv", "-0", "PATH"],
        &["printenv -0 PATH"],
    );
    assert_data_classic_matches_direct(&["pwd", "-P"]);
    assert_data_classic_matches_direct(&["readlink", "-f", &link]);
    assert_data_classic_matches_direct(&["realpath", "--relative-to", &dir, &file]);
    assert_data_classic_matches_direct(&["tty", "-s"]);
    assert_data_classic_matches_direct(&["uptime", "-s"]);
}

#[test]
fn data_phase_b_gnu_flags_match_direct_classic_output() {
    let temp_dir = TempDir::new().expect("tempdir");
    let file = temp_dir.path().join("sample.txt");
    fs::write(&file, "alpha\n").expect("write phase-b fixture");

    let dir = temp_dir.path().display().to_string();
    let file = file.display().to_string();

    assert_data_classic_matches_direct(&["dir", "-d", "-a", &dir]);
    assert_data_classic_matches_direct(&["dirname", "-z", "/tmp/alpha"]);
    assert_data_classic_matches_direct(&["echo", "-n", "alpha"]);
    assert_data_classic_matches_direct_with_data_args(
        "expr",
        &["expr", "--", "1", "=", "1"],
        &["expr -- \"1\" \"=\" \"1\""],
    );
    assert_data_classic_matches_direct(&["hostname", "-s"]);
    assert_data_classic_matches_direct(&["id", "-u"]);
    assert_data_classic_matches_direct(&["logname", "--help"]);
    assert_data_classic_matches_direct(&["pathchk", "-p", "alpha"]);
    assert_data_classic_matches_direct(&["pinky", "-s"]);
    assert_data_classic_matches_direct_with_data_args(
        "printf",
        &["printf", "--", "%s", "alpha"],
        &["printf -- \"%s\" alpha"],
    );
    assert_data_classic_matches_direct_with_data_args(
        "seq",
        &["seq", "-s", ",", "1", "3"],
        &["seq -s \",\" \"1\" \"3\""],
    );
    assert_data_classic_matches_direct_with_data_args(
        "stat",
        &["stat", "-c", "%s", &file],
        &[&format!("stat -c \"%s\" {}", file)],
    );
    assert_data_classic_matches_direct(&["vdir", "-d", "-a", &dir]);
    assert_data_classic_matches_direct(&["who", "-q"]);
    assert_data_classic_matches_direct(&["whoami", "--help"]);
}

#[test]
fn data_whoami_json_is_username_record() {
    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .arg("whoami")
        .output()
        .expect("run direct syskits whoami");
    assert!(
        expected.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&expected.stderr)
    );
    let expected_username = String::from_utf8_lossy(&expected.stdout)
        .trim_end_matches(['\r', '\n'])
        .to_string();

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=json", "whoami"])
        .output()
        .expect("run syskits data whoami json");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.trim_start().starts_with('{'), "stdout: {stdout:?}");
    assert!(
        stdout.contains(&format!("\"username\":\"{expected_username}\"")),
        "stdout: {stdout:?}"
    );
}

#[test]
fn direct_printf_warns_about_excess_arguments() {
    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["printf", "we", "are", "ok."])
        .output()
        .expect("run direct syskits printf excess args");

    assert_eq!(out.status.code(), Some(0));
    assert_eq!(out.stdout, b"we");
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "printf: warning: ignoring excess arguments, starting with ‘are’\n"
    );
}

#[test]
fn data_gnu_numeric_and_clustered_short_flags_match_direct_classic_output() {
    let temp_dir = TempDir::new().expect("tempdir");
    let file = temp_dir.path().join("sample.txt");
    fs::write(&file, "1\n2\n3\n4\n5\n6\n").expect("write gnu short flags fixture");
    let dir = temp_dir.path().display().to_string();
    let file = file.display().to_string();

    assert_data_classic_matches_direct(&["ls", "-1", &dir]);
    assert_data_classic_matches_direct(&["ls", "-la", &file]);
    assert_data_classic_matches_direct(&["head", "-5", &file]);
}

#[test]
fn data_split_classic_help_matches_direct_split() {
    assert_data_classic_matches_direct(&["split", "--help"]);
}

#[test]
fn data_split_classic_flagged_files_match_direct_split() {
    let direct_dir = TempDir::new().expect("direct tempdir");
    let data_dir = TempDir::new().expect("data tempdir");
    fs::write(direct_dir.path().join("input.txt"), "a\nb\nc\n").expect("write direct split input");
    fs::write(data_dir.path().join("input.txt"), "a\nb\nc\n").expect("write data split input");

    let direct = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(direct_dir.path())
        .args(["split", "-l", "1", "input.txt", "chunk_"])
        .output()
        .expect("run direct split");
    let data = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(data_dir.path())
        .args(["data", "format=classic", "split -l \"1\" input.txt chunk_"])
        .output()
        .expect("run data split classic");

    assert_eq!(data.status.code(), direct.status.code());
    assert_eq!(data.stdout, direct.stdout);
    assert_eq!(data.stderr, direct.stderr);
    assert_eq!(
        split_chunks(data_dir.path()),
        split_chunks(direct_dir.path())
    );
}

#[test]
fn data_base32_json_default_is_information_rich() {
    let temp_dir = TempDir::new().expect("tempdir");
    let path = temp_dir.path().join("sample.txt");
    fs::write(&path, "abc").expect("write base32 fixture");
    let path = path.display().to_string();

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=json", "base32", &path])
        .output()
        .expect("run syskits data base32");

    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("\"mode\":\"encode\""), "stdout: {stdout:?}");
    assert!(stdout.contains("\"wrap\":76"), "stdout: {stdout:?}");
    assert!(
        stdout.contains("\"ignore_garbage\":false"),
        "stdout: {stdout:?}"
    );
    assert!(stdout.contains("\"input\":\"file\""), "stdout: {stdout:?}");
    assert!(
        stdout.contains("\"output_text\":\"MFRGG===\""),
        "stdout: {stdout:?}"
    );
    assert!(stdout.contains(&path), "stdout: {stdout:?}");
}

#[test]
fn data_uname_classic_all_flag_matches_direct_uname() {
    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["uname", "-a"])
        .output()
        .expect("run direct syskits uname -a");

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=classic", "uname", "-a"])
        .output()
        .expect("run syskits data uname -a classic");

    assert_eq!(out.status.code(), expected.status.code());
    assert_eq!(out.stdout, expected.stdout);
    assert_eq!(out.stderr, expected.stderr);
}

#[test]
fn data_arch_classic_invalid_flag_matches_direct_arch() {
    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["arch", "-a"])
        .output()
        .expect("run direct syskits arch -a");

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=classic", "arch", "-a"])
        .output()
        .expect("run syskits data arch -a classic");

    assert_eq!(out.status.code(), expected.status.code());
    assert_eq!(out.stdout, expected.stdout);
    assert_eq!(out.stderr, expected.stderr);
}

#[test]
fn data_tilde_prefix_forces_external_for_native_command() {
    let expected = Command::new("uname")
        .arg("-a")
        .output()
        .expect("run system uname -a");

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=classic", "~uname", "-a"])
        .output()
        .expect("run syskits data ~uname -a classic");

    assert_same_process_output(&out, &expected);
}

#[test]
fn data_tilde_prefix_forces_external_for_direct_only_command() {
    let expected = Command::new("chroot")
        .arg("--help")
        .output()
        .expect("run system chroot --help");

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=classic", "~chroot", "--help"])
        .output()
        .expect("run syskits data ~chroot --help classic");

    assert_same_process_output(&out, &expected);
}

#[cfg(feature = "feat_shell_init")]
#[test]
fn data_direct_only_command_prefers_internal_tool_before_external_path() {
    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["shell-init", "--help"])
        .output()
        .expect("run direct syskits shell-init --help");

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=classic", "shell-init", "--help"])
        .output()
        .expect("run syskits data shell-init --help classic");

    assert_same_process_output(&out, &expected);
}

#[test]
fn shell_entry_accepts_gnu_flags_like_data_entry() {
    let expected_uname = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["uname", "-a"])
        .output()
        .expect("run direct syskits uname -a");
    let shell_uname = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["shell", "format=classic", "uname", "-a"])
        .output()
        .expect("run syskits shell uname -a classic");

    assert_eq!(shell_uname.status.code(), expected_uname.status.code());
    assert_eq!(shell_uname.stdout, expected_uname.stdout);
    assert_eq!(shell_uname.stderr, expected_uname.stderr);

    let expected_arch = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["arch", "-a"])
        .output()
        .expect("run direct syskits arch -a");
    let shell_arch = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["shell", "format=classic", "arch", "-a"])
        .output()
        .expect("run syskits shell arch -a classic");

    assert_eq!(shell_arch.status.code(), expected_arch.status.code());
    assert_eq!(shell_arch.stdout, expected_arch.stdout);
    assert_eq!(shell_arch.stderr, expected_arch.stderr);
}

#[test]
fn shell_multicall_binary_name_enters_data_shell_entry() {
    let temp_dir = TempDir::new().expect("tempdir");
    let shell_path = temp_dir.path().join("shell");

    #[cfg(unix)]
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_syskits"), &shell_path)
        .expect("create shell symlink");
    #[cfg(not(unix))]
    fs::copy(env!("CARGO_BIN_EXE_syskits"), &shell_path).expect("copy shell binary");

    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["uname", "-m"])
        .output()
        .expect("run direct syskits uname -m");
    let actual = Command::new(&shell_path)
        .args(["format=classic", "uname", "-m"])
        .output()
        .expect("run shell multicall entry uname -m classic");

    assert_same_process_output(&actual, &expected);
}

#[test]
fn shell_direct_only_command_prefers_internal_tool_before_external_path() {
    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["chroot", "--help"])
        .output()
        .expect("run direct syskits chroot --help");
    let shell = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["shell", "format=classic", "chroot", "--help"])
        .output()
        .expect("run syskits shell chroot --help classic");

    assert_same_process_output(&shell, &expected);
}

#[test]
fn shell_tilde_prefix_forces_external_for_native_command() {
    let expected = Command::new("uname")
        .arg("-a")
        .output()
        .expect("run system uname -a");
    let shell = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["shell", "format=classic", "~uname", "-a"])
        .output()
        .expect("run syskits shell ~uname -a classic");

    assert_same_process_output(&shell, &expected);
}

#[test]
fn external_path_policy_skips_priority_syskits_for_tilde_and_run_external() {
    let priority_dir = std::path::Path::new("/usr/local/priority_syskits");
    if !priority_dir.is_dir() {
        return;
    }

    let expected = Command::new("/usr/bin/uname")
        .arg("--version")
        .output()
        .expect("run /usr/bin/uname --version");

    let original_path = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![priority_dir.to_path_buf()];
    paths.extend(std::env::split_paths(&original_path));
    let path = std::env::join_paths(paths).expect("join PATH");

    let tilde = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("PATH", &path)
        .args(["data", "format=classic", "~uname", "--version"])
        .output()
        .expect("run syskits data ~uname --version classic");
    assert_same_process_output(&tilde, &expected);

    let run_external = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("PATH", &path)
        .args(["data", "format=classic", "run-external uname --version"])
        .output()
        .expect("run syskits data run-external uname --version classic");
    assert_same_process_output(&run_external, &expected);
}

#[test]
fn external_csv_is_decoded_with_from_csv() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args([
            "data",
            "format=json",
            "~printf \"name,age,gender\\nalice,18,male\\nbob,20,female\\n\" | from csv",
        ])
        .output()
        .expect("run forced external printf csv through from csv");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stderr, b"");

    let stdout = String::from_utf8(output.stdout).expect("json stdout");
    assert!(stdout.contains(r#""name":"alice""#), "stdout: {stdout}");
    assert!(stdout.contains(r#""age":18"#), "stdout: {stdout}");
    assert!(stdout.contains(r#""gender":"female""#), "stdout: {stdout}");
    assert!(!stdout.starts_with("[110,"), "stdout: {stdout}");
}

#[test]
fn external_awk_reads_builtin_classic_stdout() {
    let first = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args([
            "data",
            "printf \"CTyunOS 4\\n\" | awk \"{print $1}\" | from text",
        ])
        .output()
        .expect("run builtin printf through external awk print first field");
    assert!(
        first.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(first.stderr, b"");
    assert_eq!(String::from_utf8_lossy(&first.stdout), "[CTyunOS]\n");

    let second = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args([
            "data",
            "printf \"CTyunOS 4\\n\" | awk \"{print $2}\" | from text",
        ])
        .output()
        .expect("run builtin printf through external awk print second field");
    assert!(
        second.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(second.stderr, b"");
    assert_eq!(String::from_utf8_lossy(&second.stdout), "[4]\n");
}

#[test]
fn data_csv_transpose_round_trip() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args([
            "data",
            "format=json",
            "from csv --transpose \"name, a, b\\nage, 12, 11\\ngender, male, female\" | to csv --transpose | from csv --transpose",
        ])
        .output()
        .expect("run data csv transpose round trip");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stderr, b"");

    let stdout = String::from_utf8(output.stdout).expect("json stdout");
    assert!(stdout.contains(r#""name":"a""#), "stdout: {stdout}");
    assert!(stdout.contains(r#""age":12"#), "stdout: {stdout}");
    assert!(stdout.contains(r#""gender":"female""#), "stdout: {stdout}");
}

#[test]
fn data_to_ssv_round_trip() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args([
            "data",
            "format=json",
            "from json \"[{\\\"name\\\":\\\"alice\\\",\\\"age\\\":30},{\\\"name\\\":\\\"bob\\\",\\\"age\\\":20}]\" | to ssv | from ssv",
        ])
        .output()
        .expect("run data to ssv round trip");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stderr, b"");

    let stdout = String::from_utf8(output.stdout).expect("json stdout");
    assert!(stdout.contains(r#""name":"alice""#), "stdout: {stdout}");
    assert!(stdout.contains(r#""age":30"#), "stdout: {stdout}");
    assert!(stdout.contains(r#""name":"bob""#), "stdout: {stdout}");
}

#[test]
fn data_ssv_transpose_round_trip() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args([
            "data",
            "format=json",
            "from ssv --transpose \"name a b\\nage 12 11\\ngender male female\" | to ssv --transpose | from ssv --transpose",
        ])
        .output()
        .expect("run data ssv transpose round trip");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stderr, b"");

    let stdout = String::from_utf8(output.stdout).expect("json stdout");
    assert!(stdout.contains(r#""name":"a""#), "stdout: {stdout}");
    assert!(stdout.contains(r#""age":12"#), "stdout: {stdout}");
    assert!(stdout.contains(r#""gender":"female""#), "stdout: {stdout}");
}

#[test]
fn data_to_empty_record_outputs_empty_text_for_plain_text_formats() {
    for format in ["yaml", "csv", "toml", "text"] {
        let expr = format!("from json \"{{}}\" | to {format}");
        let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .args(["data", "format=text", &expr])
            .output()
            .unwrap_or_else(|_| panic!("run data empty record to {format}"));

        assert!(
            output.status.success(),
            "{format} stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stderr, b"", "{format}");
        assert_eq!(output.stdout, b"\n", "{format}");
    }
}

#[test]
fn data_to_toml_empty_list_outputs_empty_text() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=text", "from json \"[]\" | to toml"])
        .output()
        .expect("run data empty list to toml");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stderr, b"");
    assert_eq!(output.stdout, b"\n");
}

#[test]
fn shell_tilde_prefix_forces_external_for_direct_only_command() {
    let expected = Command::new("chroot")
        .arg("--help")
        .output()
        .expect("run system chroot --help");
    let shell = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["shell", "format=classic", "~chroot", "--help"])
        .output()
        .expect("run syskits shell ~chroot --help classic");

    assert_same_process_output(&shell, &expected);
}

#[test]
fn shell_tilde_prefix_preserves_external_nonzero_status() {
    let expected = Command::new("false").output().expect("run system false");
    let shell = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["shell", "format=classic", "~false"])
        .output()
        .expect("run syskits shell ~false classic");

    assert_same_process_output(&shell, &expected);
}

#[test]
fn shell_native_wrapper_meta_options_use_legacy_text_path() {
    for (command_name, option) in [
        ("cat", "--version"),
        ("cat", "--help"),
        ("ls", "--version"),
        ("ls", "--help"),
        ("uname", "--version"),
        ("uname", "--help"),
    ] {
        let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .args([command_name, option])
            .output()
            .unwrap_or_else(|err| panic!("run direct syskits {command_name} {option}: {err}"));
        let shell = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .args(["shell", "format=classic", command_name, option])
            .output()
            .unwrap_or_else(|err| panic!("run syskits shell {command_name} {option}: {err}"));

        assert_same_process_output(&shell, &expected);
    }
}

#[test]
fn shell_direct_only_nonzero_command_matches_internal_tool() {
    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["false"])
        .output()
        .expect("run direct syskits false");
    let shell = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["shell", "format=classic", "false"])
        .output()
        .expect("run syskits shell false classic");

    assert_same_process_output(&shell, &expected);
}

#[test]
fn data_uname_json_machine_flag_uses_explicit_flags() {
    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=json", "uname", "-m"])
        .output()
        .expect("run syskits data uname -m");

    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("\"machine\""), "stdout: {stdout:?}");
    assert!(!stdout.contains("\"kernel_release\""), "stdout: {stdout:?}");
}

#[test]
fn data_base32_classic_format_matches_direct_base32() {
    let temp_dir = TempDir::new().expect("tempdir");
    let path = temp_dir.path().join("sample.txt");
    fs::write(&path, "abc").expect("write base32 sample");
    let path = path.display().to_string();

    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["base32", &path])
        .output()
        .expect("run direct syskits base32");
    assert_eq!(expected.status.code(), Some(0));

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=classic", "base32", &path])
        .output()
        .expect("run syskits data base32 classic");

    assert_eq!(out.status.code(), expected.status.code());
    assert_eq!(out.stdout, expected.stdout);
    assert_eq!(out.stderr, expected.stderr);
}

#[test]
fn data_base32_classic_error_path_matches_direct_base32() {
    let temp_dir = TempDir::new().expect("tempdir");
    let path = temp_dir.path().join("invalid.txt");
    fs::write(&path, "%%%").expect("write invalid base32 sample");
    let path = path.display().to_string();

    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["base32", "--decode", &path])
        .output()
        .expect("run direct syskits base32 invalid decode");

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=classic", "base32", "--decode", &path])
        .output()
        .expect("run syskits data base32 classic invalid decode");

    assert_eq!(out.status.code(), expected.status.code());
    assert_eq!(out.stdout, expected.stdout);
    assert_eq!(out.stderr, expected.stderr);
}

#[test]
fn data_base32_classic_binary_decode_matches_direct_bytes() {
    let temp_dir = TempDir::new().expect("tempdir");
    let path = temp_dir.path().join("binary.b32");
    fs::write(&path, "74======\n").expect("write binary base32 sample");
    let path = path.display().to_string();

    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["base32", "--decode", &path])
        .output()
        .expect("run direct syskits base32 binary decode");
    assert_eq!(expected.status.code(), Some(0));
    assert_eq!(expected.stdout, vec![0xff]);

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=classic", "base32", "--decode", &path])
        .output()
        .expect("run syskits data base32 classic binary decode");

    assert_eq!(out.status.code(), expected.status.code());
    assert_eq!(out.stdout, expected.stdout);
    assert_eq!(out.stderr, expected.stderr);
}

#[test]
fn data_base64_json_default_is_information_rich() {
    let temp_dir = TempDir::new().expect("tempdir");
    let path = temp_dir.path().join("sample.txt");
    fs::write(&path, "abc").expect("write base64 fixture");
    let path = path.display().to_string();

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=json", "base64", &path])
        .output()
        .expect("run syskits data base64");

    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("\"mode\":\"encode\""), "stdout: {stdout:?}");
    assert!(stdout.contains("\"wrap\":76"), "stdout: {stdout:?}");
    assert!(
        stdout.contains("\"ignore_garbage\":false"),
        "stdout: {stdout:?}"
    );
    assert!(stdout.contains("\"input\":\"file\""), "stdout: {stdout:?}");
    assert!(
        stdout.contains("\"output_text\":\"YWJj\""),
        "stdout: {stdout:?}"
    );
    assert!(stdout.contains(&path), "stdout: {stdout:?}");
}

#[test]
fn data_base64_classic_format_matches_direct_base64() {
    let temp_dir = TempDir::new().expect("tempdir");
    let path = temp_dir.path().join("sample.txt");
    fs::write(&path, "abc").expect("write base64 sample");
    let path = path.display().to_string();

    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["base64", &path])
        .output()
        .expect("run direct syskits base64");
    assert_eq!(expected.status.code(), Some(0));

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=classic", "base64", &path])
        .output()
        .expect("run syskits data base64 classic");

    assert_eq!(out.status.code(), expected.status.code());
    assert_eq!(out.stdout, expected.stdout);
    assert_eq!(out.stderr, expected.stderr);
}

#[test]
fn data_base64_classic_error_path_matches_direct_base64() {
    let temp_dir = TempDir::new().expect("tempdir");
    let path = temp_dir.path().join("invalid.txt");
    fs::write(&path, "%%%").expect("write invalid base64 sample");
    let path = path.display().to_string();

    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["base64", "--decode", &path])
        .output()
        .expect("run direct syskits base64 invalid decode");

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=classic", "base64", "--decode", &path])
        .output()
        .expect("run syskits data base64 classic invalid decode");

    assert_eq!(out.status.code(), expected.status.code());
    assert_eq!(out.stdout, expected.stdout);
    assert_eq!(out.stderr, expected.stderr);
}

#[test]
fn data_base64_classic_binary_decode_matches_direct_bytes() {
    let temp_dir = TempDir::new().expect("tempdir");
    let path = temp_dir.path().join("binary.b64");
    fs::write(&path, "/w==\n").expect("write binary base64 sample");
    let path = path.display().to_string();

    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["base64", "--decode", &path])
        .output()
        .expect("run direct syskits base64 binary decode");
    assert_eq!(expected.status.code(), Some(0));
    assert_eq!(expected.stdout, vec![0xff]);

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=classic", "base64", "--decode", &path])
        .output()
        .expect("run syskits data base64 classic binary decode");

    assert_eq!(out.status.code(), expected.status.code());
    assert_eq!(out.stdout, expected.stdout);
    assert_eq!(out.stderr, expected.stderr);
}

#[test]
fn data_basenc_json_default_is_information_rich() {
    let temp_dir = TempDir::new().expect("tempdir");
    let path = temp_dir.path().join("sample.txt");
    fs::write(&path, "abc").expect("write basenc fixture");
    let path = path.display().to_string();

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=json", "basenc", "--base64", &path])
        .output()
        .expect("run syskits data basenc");

    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("\"encoding\":\"base64\""),
        "stdout: {stdout:?}"
    );
    assert!(stdout.contains("\"type\":\"text\""), "stdout: {stdout:?}");
    assert!(stdout.contains("\"mode\":\"encode\""), "stdout: {stdout:?}");
    assert!(stdout.contains("\"wrap\":76"), "stdout: {stdout:?}");
    assert!(
        stdout.contains("\"ignore_garbage\":false"),
        "stdout: {stdout:?}"
    );
    assert!(
        stdout.contains("\"output_text\":\"YWJj\""),
        "stdout: {stdout:?}"
    );
    assert!(stdout.contains(&path), "stdout: {stdout:?}");
}

#[test]
fn data_basenc_classic_format_matches_direct_basenc() {
    let temp_dir = TempDir::new().expect("tempdir");
    let path = temp_dir.path().join("sample.txt");
    fs::write(&path, "abc").expect("write basenc sample");
    let path = path.display().to_string();

    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["basenc", "--base64", &path])
        .output()
        .expect("run direct syskits basenc");
    assert_eq!(expected.status.code(), Some(0));

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=classic", "basenc", "--base64", &path])
        .output()
        .expect("run syskits data basenc classic");

    assert_eq!(out.status.code(), expected.status.code());
    assert_eq!(out.stdout, expected.stdout);
    assert_eq!(out.stderr, expected.stderr);
}

#[test]
fn data_basenc_classic_error_path_matches_direct_basenc() {
    let temp_dir = TempDir::new().expect("tempdir");
    let path = temp_dir.path().join("invalid.txt");
    fs::write(&path, "%%%").expect("write invalid basenc sample");
    let path = path.display().to_string();

    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["basenc", "--decode", "--base64", &path])
        .output()
        .expect("run direct syskits basenc invalid decode");

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args([
            "data",
            "format=classic",
            "basenc",
            "--decode",
            "--base64",
            &path,
        ])
        .output()
        .expect("run syskits data basenc classic invalid decode");

    assert_eq!(out.status.code(), expected.status.code());
    assert_eq!(out.stdout, expected.stdout);
    assert_eq!(out.stderr, expected.stderr);
}

#[test]
fn data_basenc_classic_binary_decode_matches_direct_bytes() {
    let temp_dir = TempDir::new().expect("tempdir");
    let path = temp_dir.path().join("binary.b64");
    fs::write(&path, "/w==\n").expect("write binary basenc sample");
    let path = path.display().to_string();

    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["basenc", "--decode", "--base64", &path])
        .output()
        .expect("run direct syskits basenc binary decode");
    assert_eq!(expected.status.code(), Some(0));
    assert_eq!(expected.stdout, vec![0xff]);

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args([
            "data",
            "format=classic",
            "basenc",
            "--decode",
            "--base64",
            &path,
        ])
        .output()
        .expect("run syskits data basenc classic binary decode");

    assert_eq!(out.status.code(), expected.status.code());
    assert_eq!(out.stdout, expected.stdout);
    assert_eq!(out.stderr, expected.stderr);
}

#[test]
fn data_expand_json_default_is_information_rich() {
    let (_temp_dir, path) = write_expand_fixture();

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=json", "expand", "-t", "4", &path])
        .output()
        .expect("run syskits data expand");

    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("\"tabstop_mode\":\"none\""),
        "stdout: {stdout:?}"
    );
    assert!(stdout.contains("\"tabstops\":[4]"), "stdout: {stdout:?}");
    assert!(stdout.contains("\"row_index\":1"), "stdout: {stdout:?}");
    assert!(stdout.contains("\"line\":\"a   b\""), "stdout: {stdout:?}");
    assert!(stdout.contains("\"had_tabs\":true"), "stdout: {stdout:?}");
}

#[test]
fn data_expand_classic_format_matches_direct_expand() {
    let (_temp_dir, path) = write_expand_fixture();

    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["expand", "-t", "4", &path])
        .output()
        .expect("run direct syskits expand");

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=classic", "expand", "-t", "4", &path])
        .output()
        .expect("run syskits data expand classic");

    assert_eq!(out.status.code(), expected.status.code());
    assert_eq!(out.stdout, expected.stdout);
    assert_eq!(out.stderr, expected.stderr);
}

#[test]
fn data_expand_classic_error_path_matches_direct_expand() {
    let temp_dir = TempDir::new().expect("tempdir");
    let dir = temp_dir.path().join("input-dir");
    fs::create_dir(&dir).expect("create expand directory fixture");
    let dir = dir.display().to_string();

    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["expand", &dir])
        .output()
        .expect("run direct syskits expand directory");

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=classic", "expand", &dir])
        .output()
        .expect("run syskits data expand classic directory");

    assert_eq!(out.status.code(), expected.status.code());
    assert_eq!(out.stdout, expected.stdout);
    assert_eq!(out.stderr, expected.stderr);
}

#[test]
fn data_cksum_classic_flagged_file_matches_direct_cksum() {
    let temp_dir = TempDir::new().expect("tempdir");
    let path = temp_dir.path().join("sample.txt");
    fs::write(&path, "abc").expect("write cksum fixture");
    let path = path.display().to_string();

    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["cksum", "--algorithm", "crc", &path])
        .output()
        .expect("run direct syskits cksum");

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args([
            "data",
            "format=classic",
            "cksum",
            "--algorithm",
            "crc",
            &path,
        ])
        .output()
        .expect("run syskits data cksum classic");

    assert_same_process_output(&out, &expected);
}

#[test]
fn data_cksum_classic_error_path_matches_direct_cksum() {
    let temp_dir = TempDir::new().expect("tempdir");
    let missing = temp_dir.path().join("missing.txt");
    let missing = missing.display().to_string();

    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["cksum", &missing])
        .output()
        .expect("run direct syskits cksum missing file");

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=classic", "cksum", &missing])
        .output()
        .expect("run syskits data cksum classic missing file");

    assert_same_process_output(&out, &expected);
}

#[test]
fn data_fold_classic_flagged_file_matches_direct_fold() {
    let temp_dir = TempDir::new().expect("tempdir");
    let path = temp_dir.path().join("fold.txt");
    fs::write(&path, "abcdef\n").expect("write fold fixture");
    let path = path.display().to_string();

    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["fold", "-w", "3", &path])
        .output()
        .expect("run direct syskits fold");

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=classic", "fold", "-w", "3", &path])
        .output()
        .expect("run syskits data fold classic");

    assert_same_process_output(&out, &expected);
}

#[test]
fn data_comm_classic_flagged_files_matches_direct_comm() {
    let temp_dir = TempDir::new().expect("tempdir");
    let left = temp_dir.path().join("left.txt");
    let right = temp_dir.path().join("right.txt");
    fs::write(&left, "alpha\nbeta\n").expect("write comm left fixture");
    fs::write(&right, "alpha\ngamma\n").expect("write comm right fixture");
    let left = left.display().to_string();
    let right = right.display().to_string();

    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["comm", "-1", &left, &right])
        .output()
        .expect("run direct syskits comm");

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=classic", "comm", "-1", &left, &right])
        .output()
        .expect("run syskits data comm classic");

    assert_same_process_output(&out, &expected);
}

#[test]
fn data_paste_classic_flagged_files_matches_direct_paste() {
    let temp_dir = TempDir::new().expect("tempdir");
    let left = temp_dir.path().join("left.txt");
    let right = temp_dir.path().join("right.txt");
    fs::write(&left, "alpha\nbeta\n").expect("write paste left fixture");
    fs::write(&right, "1\n2\n").expect("write paste right fixture");
    let left = left.display().to_string();
    let right = right.display().to_string();

    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["paste", "-d", "x", &left, &right])
        .output()
        .expect("run direct syskits paste");

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=classic", "paste", "-d", "x", &left, &right])
        .output()
        .expect("run syskits data paste classic");

    assert_same_process_output(&out, &expected);
}

#[test]
fn data_cat_classic_flagged_file_matches_direct_cat() {
    let temp_dir = TempDir::new().expect("tempdir");
    let path = temp_dir.path().join("cat.txt");
    fs::write(&path, "alpha\nbeta\n").expect("write cat fixture");
    let path = path.display().to_string();

    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["cat", "-n", &path])
        .output()
        .expect("run direct syskits cat");

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=classic", "cat", "-n", &path])
        .output()
        .expect("run syskits data cat classic");

    assert_same_process_output(&out, &expected);
}

#[test]
fn data_tee_classic_flagged_file_matches_direct_tee() {
    let temp_dir = TempDir::new().expect("tempdir");
    let direct_path = temp_dir.path().join("direct.txt");
    let data_path = temp_dir.path().join("data.txt");
    fs::write(&direct_path, "before\n").expect("write tee direct fixture");
    fs::write(&data_path, "before\n").expect("write tee data fixture");
    let direct_path = direct_path.display().to_string();
    let data_path = data_path.display().to_string();
    let stdin = b"after\n";

    let expected = run_syskits_with_stdin(&["tee", "-a", &direct_path], stdin);
    let out = run_syskits_with_stdin(&["data", "format=classic", "tee", "-a", &data_path], stdin);

    assert_same_process_output(&out, &expected);
    assert_eq!(
        fs::read_to_string(&data_path).expect("read data tee target"),
        fs::read_to_string(&direct_path).expect("read direct tee target")
    );
}

#[test]
fn data_more_classic_flagged_file_matches_direct_more() {
    let temp_dir = TempDir::new().expect("tempdir");
    let path = temp_dir.path().join("more.txt");
    fs::write(&path, "alpha\nbeta\n").expect("write more fixture");
    let path = path.display().to_string();

    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["more", "-5", &path])
        .output()
        .expect("run direct syskits more");

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=classic", "more", "-5", &path])
        .output()
        .expect("run syskits data more classic");

    assert_same_process_output(&out, &expected);
}

#[test]
fn data_wc_classic_error_path_matches_direct_wc() {
    let temp_dir = TempDir::new().expect("tempdir");
    let missing = temp_dir.path().join("missing.txt");
    let missing = missing.display().to_string();

    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["wc", &missing])
        .output()
        .expect("run direct syskits wc missing file");

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=classic", "wc", &missing])
        .output()
        .expect("run syskits data wc classic missing file");

    assert_same_process_output(&out, &expected);
}

#[test]
fn data_pr_json_default_is_information_rich() {
    let (_temp_dir, path) = write_pr_fixture();

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=json", "pr", "-t", &path])
        .output()
        .expect("run syskits data pr");

    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("\"page\":1"), "stdout: {stdout:?}");
    assert!(stdout.contains("\"kind\":\"body\""), "stdout: {stdout:?}");
    assert!(stdout.contains("\"line_index\":1"), "stdout: {stdout:?}");
    assert!(stdout.contains("\"text\":\"alpha\""), "stdout: {stdout:?}");
    assert!(stdout.contains(&path), "stdout: {stdout:?}");
}

#[test]
fn data_pr_classic_format_matches_direct_pr() {
    let (_temp_dir, path) = write_pr_fixture();

    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["pr", "-t", &path])
        .output()
        .expect("run direct syskits pr");

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=classic", "pr", "-t", &path])
        .output()
        .expect("run syskits data pr classic");

    assert_eq!(out.status.code(), expected.status.code());
    assert_eq!(out.stdout, expected.stdout);
    assert_eq!(out.stderr, expected.stderr);
}

#[test]
fn data_pr_classic_pipeline_input_matches_direct_pr() {
    let stdin = b"alpha\nbeta\n";

    let expected = run_syskits_with_stdin(&["pr", "-t"], stdin);
    let out = run_syskits_with_stdin(&["data", "format=classic", "from text | pr -t"], stdin);

    assert_eq!(out.status.code(), expected.status.code());
    assert_eq!(out.stdout, expected.stdout);
    assert_eq!(out.stderr, expected.stderr);
}

#[test]
fn data_pr_classic_error_path_matches_direct_pr() {
    let temp_dir = TempDir::new().expect("tempdir");
    let missing = temp_dir.path().join("missing.txt");
    let missing = missing.display().to_string();

    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["pr", "-t", &missing])
        .output()
        .expect("run direct syskits pr missing file");

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=classic", "pr", "-t", &missing])
        .output()
        .expect("run syskits data pr classic missing file");

    assert_eq!(out.status.code(), expected.status.code());
    assert_eq!(out.stdout, expected.stdout);
    assert_eq!(out.stderr, expected.stderr);
}

#[test]
fn data_pr_classic_invalid_flag_matches_direct_pr() {
    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["pr", "--definitely-invalid"])
        .output()
        .expect("run direct syskits pr invalid flag");
    assert_eq!(
        expected.status.code(),
        Some(1),
        "direct stdout: {:?}, stderr: {:?}",
        String::from_utf8_lossy(&expected.stdout),
        String::from_utf8_lossy(&expected.stderr)
    );

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=classic", "pr", "--definitely-invalid"])
        .output()
        .expect("run syskits data pr classic invalid flag");

    assert_eq!(out.status.code(), expected.status.code());
    assert_eq!(out.stdout, expected.stdout);
    assert_eq!(out.stderr, expected.stderr);
}

#[test]
fn data_ptx_json_default_is_information_rich() {
    let (_temp_dir, path) = write_ptx_fixture();

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=json", "ptx", "-w", "30", &path])
        .output()
        .expect("run syskits data ptx");

    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("\"keyword\""), "stdout: {stdout:?}");
    assert!(stdout.contains("\"before\""), "stdout: {stdout:?}");
    assert!(stdout.contains("\"after\""), "stdout: {stdout:?}");
    assert!(stdout.contains("\"reference\""), "stdout: {stdout:?}");
    assert!(stdout.contains("\"file\""), "stdout: {stdout:?}");
    assert!(stdout.contains("\"line_index\""), "stdout: {stdout:?}");
    assert!(stdout.contains("\"rendered_text\""), "stdout: {stdout:?}");
    assert!(stdout.contains("\"alpha\""), "stdout: {stdout:?}");
    assert!(stdout.contains(&path), "stdout: {stdout:?}");
}

#[test]
fn data_ptx_json_traditional_output_file_does_not_create_file() {
    let temp_dir = TempDir::new().expect("tempdir");
    let input = temp_dir.path().join("ptx.txt");
    let output = temp_dir.path().join("ptx.out");
    fs::write(&input, "alpha beta gamma\n").expect("write ptx traditional fixture");
    let input = input.display().to_string();
    let output = output.display().to_string();

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=json", "ptx", "-G", &input, &output])
        .output()
        .expect("run syskits data ptx traditional");

    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !std::path::Path::new(&output).exists(),
        "data/json ptx must not create traditional output file"
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("\"keyword\""), "stdout: {stdout:?}");
    assert!(stdout.contains("\"alpha\""), "stdout: {stdout:?}");
}

#[test]
fn data_ptx_json_zero_length_regex_error_preserves_diagnostics_without_output_file() {
    let temp_dir = TempDir::new().expect("tempdir");
    let input = temp_dir.path().join("ptx.txt");
    let output = temp_dir.path().join("ptx.out");
    fs::write(&input, "alpha beta gamma\n").expect("write ptx zero-length regex fixture");
    let input = input.display().to_string();
    let output = output.display().to_string();

    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["ptx", "-S", "^", &input])
        .output()
        .expect("run direct syskits ptx zero-length regex");

    assert_ne!(expected.status.code(), Some(0));

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args([
            "data",
            "format=json",
            &format!("ptx -G -S \"^\" {input} {output}"),
        ])
        .output()
        .expect("run syskits data ptx zero-length regex");

    assert_eq!(out.status.code(), expected.status.code());
    assert_eq!(out.stderr, expected.stderr);
    assert!(
        !std::path::Path::new(&output).exists(),
        "data/json ptx must not create traditional output file on errors"
    );
}

#[test]
fn data_ptx_classic_format_matches_direct_ptx() {
    let (_temp_dir, path) = write_ptx_fixture();

    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["ptx", "-w", "30", &path])
        .output()
        .expect("run direct syskits ptx");

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=classic", "ptx", "-w", "30", &path])
        .output()
        .expect("run syskits data ptx classic");

    assert_eq!(out.status.code(), expected.status.code());
    assert_eq!(out.stdout, expected.stdout);
    assert_eq!(out.stderr, expected.stderr);
}

#[test]
fn data_cut_allows_empty_output_delimiter_long_flag_value() {
    let temp_dir = TempDir::new().expect("tempdir");
    let input = temp_dir.path().join("cut.txt");
    fs::write(&input, "Lf8e\n").expect("write cut fixture");
    let input = input.display().to_string();

    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["cut", "-c1-2,3-4", "--output-delimiter=", &input])
        .output()
        .expect("run direct syskits cut empty output delimiter");

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args([
            "data",
            "format=classic",
            &format!("cut -c1-2,3-4 --output-delimiter= {input}"),
        ])
        .output()
        .expect("run syskits data cut empty output delimiter");

    assert_eq!(out.status.code(), expected.status.code());
    assert_eq!(out.stdout, expected.stdout);
    assert_eq!(out.stderr, expected.stderr);
}

#[test]
fn data_ptx_classic_stdin_matches_direct_ptx() {
    let stdin = b"alpha beta gamma\n";

    let expected = run_syskits_with_stdin(&["ptx", "-w", "30"], stdin);
    let out = run_syskits_with_stdin(&["data", "format=classic", "ptx", "-w", "30"], stdin);

    assert_eq!(out.status.code(), expected.status.code());
    assert_eq!(out.stdout, expected.stdout);
    assert_eq!(out.stderr, expected.stderr);
}

#[test]
fn data_ptx_classic_pipeline_input_matches_direct_ptx() {
    let stdin = b"alpha beta gamma\n";

    let expected = run_syskits_with_stdin(&["ptx", "-w", "30"], stdin);
    let out = run_syskits_with_stdin(&["data", "format=classic", "from text | ptx -w 30"], stdin);

    assert_eq!(out.status.code(), expected.status.code());
    assert_eq!(out.stdout, expected.stdout);
    assert_eq!(out.stderr, expected.stderr);
}

#[test]
fn data_ptx_classic_error_path_matches_direct_ptx() {
    let temp_dir = TempDir::new().expect("tempdir");
    let missing = temp_dir.path().join("missing.txt");
    let missing = missing.display().to_string();

    let expected = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["ptx", "-w", "30", &missing])
        .output()
        .expect("run direct syskits ptx missing file");

    let out = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["data", "format=classic", "ptx", "-w", "30", &missing])
        .output()
        .expect("run syskits data ptx classic missing file");

    assert_eq!(out.status.code(), expected.status.code());
    assert_eq!(out.stdout, expected.stdout);
    assert_eq!(out.stderr, expected.stderr);
}

#[test]
fn data_ptx_classic_invalid_flag_with_stdin_matches_direct_ptx() {
    let stdin = vec![b'a'; 200 * 1024];

    let expected = run_syskits_with_stdin(&["ptx", "--definitely-invalid"], &stdin);
    let out = run_syskits_with_stdin(
        &["data", "format=classic", "ptx", "--definitely-invalid"],
        &stdin,
    );

    assert_eq!(out.status.code(), expected.status.code());
    assert_eq!(out.stdout, expected.stdout);
    assert_eq!(out.stderr, expected.stderr);
}
