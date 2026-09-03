use std::fs;
use std::process::Command;

use tempfile::TempDir;

#[test]
fn ptx_auto_reference_keeps_physical_input_lines() {
    let temp_dir = TempDir::new().expect("tempdir");
    let input = temp_dir.path().join("testfile");
    fs::write(&input, "first line.\nsecond line.\nthis is third line.\n").expect("write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .args(["ptx", "-A", "testfile"])
        .output()
        .expect("run syskits ptx -A");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stderr, b"");

    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    let path = "testfile";
    let expected = format!(
        "{path}:1:                               first line.\n\
{path}:3:                        this   is third line.\n\
{path}:1:                       first   line.\n\
{path}:2:                      second   line.\n\
{path}:3:               this is third   line.\n\
{path}:2:                               second line.\n\
{path}:3:                     this is   third line.\n\
{path}:3:                               this is third line.\n"
    );
    assert_eq!(stdout, expected);
}

#[test]
fn ptx_default_word_matching_does_not_index_leading_numbers() {
    let temp_dir = TempDir::new().expect("tempdir");
    let input = temp_dir.path().join("testfile");
    fs::write(&input, "first line.\nsecond line.\n3 line.\n").expect("write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .args(["ptx", "-A", "testfile"])
        .output()
        .expect("run syskits ptx -A");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stderr, b"");

    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    let expected = "testfile:1:                               first line.\n\
testfile:1:                       first   line.\n\
testfile:2:                      second   line.\n\
testfile:3:                           3   line.\n\
testfile:2:                               second line.\n";
    assert_eq!(stdout, expected);
    assert!(!stdout.contains("testfile:3:                               3 line."));

    let roff = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .args(["ptx", "-O", "testfile"])
        .output()
        .expect("run syskits ptx -O");
    assert!(
        roff.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&roff.stderr)
    );
    assert_eq!(roff.stderr, b"");
    assert_eq!(
        String::from_utf8(roff.stdout).expect("utf8 stdout"),
        ".xx \"\" \"\" \"first line.\" \"\"\n\
.xx \"\" \"first\" \"line.\" \"\"\n\
.xx \"\" \"second\" \"line.\" \"\"\n\
.xx \"\" \"3\" \"line.\" \"\"\n\
.xx \"\" \"\" \"second line.\" \"\"\n"
    );
}

#[test]
fn ptx_default_context_spans_physical_lines_like_gnu() {
    let temp_dir = TempDir::new().expect("tempdir");
    let input = temp_dir.path().join("testfile");
    fs::write(
        &input,
        "first line.\nsecond line.\n3 line.\nafsagsafsal\n12443 422\nax1     2\n    aaa\n",
    )
    .expect("write fixture");

    let roff = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .args(["ptx", "-O", "testfile"])
        .output()
        .expect("run syskits ptx -O");
    assert!(
        roff.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&roff.stderr)
    );
    assert_eq!(roff.stderr, b"");
    assert_eq!(
        String::from_utf8(roff.stdout).expect("utf8 stdout"),
        ".xx \"\" \"afsagsafsal 12443 422 ax1     2\" \"aaa\" \"\"\n\
.xx \"aaa\" \"\" \"afsagsafsal 12443 422 ax1     2\" \"\"\n\
.xx \"\" \"afsagsafsal 12443 422\" \"ax1     2     aaa\" \"\"\n\
.xx \"\" \"\" \"first line.\" \"\"\n\
.xx \"\" \"first\" \"line.\" \"\"\n\
.xx \"\" \"second\" \"line.\" \"\"\n\
.xx \"\" \"3\" \"line.\" \"\"\n\
.xx \"\" \"\" \"second line.\" \"\"\n"
    );

    let auto_ref = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .args(["ptx", "-A", "testfile"])
        .output()
        .expect("run syskits ptx -A");
    assert!(
        auto_ref.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&auto_ref.stderr)
    );
    assert_eq!(auto_ref.stderr, b"");
    assert_eq!(
        String::from_utf8(auto_ref.stdout).expect("utf8 stdout"),
        "testfile:7:         12443 422 ax1     2   aaa               afsagsafsal\n\
testfile:4:  2     aaa                    afsagsafsal 12443 422 ax1\n\
testfile:6:       afsagsafsal 12443 422   ax1     2     aaa\n\
testfile:1:                               first line.\n\
testfile:1:                       first   line.\n\
testfile:2:                      second   line.\n\
testfile:3:                           3   line.\n\
testfile:2:                               second line.\n"
    );
}

#[test]
fn ptx_default_field_splitting_matches_gnu_for_punctuation() {
    let temp_dir = TempDir::new().expect("tempdir");
    let input = temp_dir.path().join("testfile");
    fs::write(
        &input,
        "GNU ptx is a tool to produce a permuted index of words.\n\
.NH This line starts with a dot, testing roff macro escaping.\n\
The backslash \\ character and braces {} need careful handling.\n\
Testing, testing, one, two, three! Is the sorting correct?\n\
Mixed case words: apple, Apple, APPLE, apple-pie, and applejack.\n\
Numbers and symbols: 12345, @#$%^&*()_+, end of file.\n",
    )
    .expect("write fixture");

    for mode in ["-A", "-O"] {
        let syskits = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .current_dir(temp_dir.path())
            .args(["ptx", mode, "testfile"])
            .output()
            .expect("run syskits ptx");
        assert!(
            syskits.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&syskits.stderr)
        );

        let gnu = Command::new("/usr/bin/ptx")
            .current_dir(temp_dir.path())
            .args([mode, "testfile"])
            .output()
            .expect("run GNU ptx");
        assert!(
            gnu.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&gnu.stderr)
        );

        assert_eq!(syskits.stdout, gnu.stdout, "stdout differed for ptx {mode}");
        assert_eq!(syskits.stderr, gnu.stderr, "stderr differed for ptx {mode}");
    }
}

#[test]
fn ptx_unicode_input_does_not_panic_and_matches_gnu_bytes() {
    let temp_dir = TempDir::new().expect("tempdir");
    let input = temp_dir.path().join("testfile");
    fs::write(
        &input,
        "This is an extremely long line designed to test the default output width truncation and wrapping behavior of the ptx command under various terminal sizes.\n\
 \n\
   Leading and trailing spaces are here.  \n\
Line with               tabs    and   multiple   spaces.\n\
Words with apostrophes like don't and hyphens like state-of-the-art are tricky.\n\
包含中文字符的测试行，English mixed with 中文 punctuation。\n\
123 456 789 !@# $%^ &*()\n",
    )
    .expect("write fixture");

    for mode in ["-A", "-O"] {
        let syskits = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .current_dir(temp_dir.path())
            .args(["ptx", mode, "testfile"])
            .output()
            .expect("run syskits ptx");
        assert!(
            syskits.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&syskits.stderr)
        );

        let gnu = Command::new("/usr/bin/ptx")
            .current_dir(temp_dir.path())
            .args([mode, "testfile"])
            .output()
            .expect("run GNU ptx");
        assert!(
            gnu.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&gnu.stderr)
        );

        assert_eq!(syskits.stdout, gnu.stdout, "stdout differed for ptx {mode}");
        assert_eq!(syskits.stderr, gnu.stderr, "stderr differed for ptx {mode}");
    }
}
