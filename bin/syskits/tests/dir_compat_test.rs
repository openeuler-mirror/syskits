use std::fs;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::process::CommandExt;
use std::process::{Command, Output};

use tempfile::TempDir;

fn assert_same_output(case: &str, gnu: Output, syskits: Output) {
    assert_eq!(
        syskits.status.code(),
        gnu.status.code(),
        "{case}: GNU stderr: {}, syskits stderr: {}",
        String::from_utf8_lossy(&gnu.stderr),
        String::from_utf8_lossy(&syskits.stderr)
    );
    assert_eq!(
        syskits.stdout, gnu.stdout,
        "{case}: GNU stdout: {:02x?}, syskits stdout: {:02x?}",
        gnu.stdout, syskits.stdout
    );
    assert_eq!(
        syskits.stderr,
        gnu.stderr,
        "{case}: GNU stderr: {}, syskits stderr: {}",
        String::from_utf8_lossy(&gnu.stderr),
        String::from_utf8_lossy(&syskits.stderr)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn dir_preserves_non_utf8_invalid_option_byte() {
    let invalid_option = std::ffi::OsString::from_vec(b"-\xff".to_vec());
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C")
        .args([std::ffi::OsStr::new("dir"), invalid_option.as_os_str()])
        .output()
        .expect("run syskits dir");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(output.stdout, b"");
    assert_eq!(
        output.stderr,
        b"dir: invalid option -- '\xff'\nTry 'dir --help' for more information.\n"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn dir_accepts_non_utf8_ignore_and_time_style_values() {
    let temp_dir = TempDir::new().expect("tempdir");
    let listing_dir = temp_dir.path().join("d");
    fs::create_dir(&listing_dir).expect("create listing directory");
    fs::write(listing_dir.join("x"), b"").expect("create utf8 file");
    fs::write(listing_dir.join(std::ffi::OsStr::from_bytes(b"\xff")), b"")
        .expect("create non-UTF-8 file");
    let timestamped = temp_dir.path().join("f");
    fs::write(&timestamped, b"").expect("create timestamped file");

    let ignore = std::ffi::OsString::from_vec(b"--ignore=\xff".to_vec());
    let ignore_output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C")
        .args([
            std::ffi::OsStr::new("dir"),
            ignore.as_os_str(),
            std::ffi::OsStr::new("-1"),
            listing_dir.as_os_str(),
        ])
        .output()
        .expect("run syskits dir with non-UTF-8 ignore pattern");
    assert!(ignore_output.status.success());
    assert_eq!(ignore_output.stdout, b"x\n");
    assert_eq!(ignore_output.stderr, b"");

    let time_style = std::ffi::OsString::from_vec(b"--time-style=+\xff".to_vec());
    let time_output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C")
        .args([
            std::ffi::OsStr::new("dir"),
            std::ffi::OsStr::new("-ln"),
            time_style.as_os_str(),
            std::ffi::OsStr::new("-d"),
            timestamped.as_os_str(),
        ])
        .output()
        .expect("run syskits dir with non-UTF-8 time style");
    assert!(time_output.status.success());
    assert!(
        time_output
            .stdout
            .windows(3)
            .any(|window| window == b" \xff "),
        "stdout: {:02x?}",
        time_output.stdout
    );
    assert_eq!(time_output.stderr, b"");
}

#[cfg(target_os = "linux")]
#[test]
fn dir_handles_non_utf8_environment_values_like_gnu() {
    let temp_dir = TempDir::new().expect("tempdir");
    let listing_dir = temp_dir.path().join("d");
    fs::create_dir(&listing_dir).expect("create listing directory");
    fs::write(listing_dir.join("a"), b"").expect("create file");
    let raw = std::ffi::OsString::from_vec(b"\xff".to_vec());

    let cases: [(&str, &[u8]); 5] = [
        (
            "COLUMNS",
            b"dir: ignoring invalid width in environment variable COLUMNS: '\\377'\n",
        ),
        (
            "TABSIZE",
            b"dir: ignoring invalid tab size in environment variable TABSIZE: '\\377'\n",
        ),
        (
            "QUOTING_STYLE",
            b"dir: ignoring invalid value of environment variable QUOTING_STYLE: '\\377'\n",
        ),
        ("TIME_STYLE", b""),
        (
            "LS_COLORS",
            b"dir: unparsable value for LS_COLORS environment variable\n",
        ),
    ];

    for (variable, expected_stderr) in cases {
        let mut command = Command::new(env!("CARGO_BIN_EXE_syskits"));
        command
            .current_dir(temp_dir.path())
            .env("LC_ALL", "C")
            .env(variable, &raw)
            .args(["dir"]);
        if variable == "LS_COLORS" {
            command.args(["--color=always", "-1"]);
        }
        command.arg(listing_dir.as_os_str());

        let output = command.output().expect("run syskits dir");
        assert!(output.status.success(), "{variable}");
        assert_eq!(output.stderr, expected_stderr, "{variable}");
    }
}

#[test]
fn dir_honors_quoting_style_environment() {
    let temp_dir = TempDir::new().expect("tempdir");
    let file = temp_dir.path().join("a space");
    fs::write(&file, b"").expect("create file");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .env("QUOTING_STYLE", "shell")
        .args(["dir", "-1", "-d", "a space"])
        .output()
        .expect("run syskits dir");

    assert!(output.status.success());
    assert_eq!(output.stdout, b"'a space'\n");
    assert_eq!(output.stderr, b"");
}

#[test]
fn dir_applies_tabsize_environment_to_default_columns() {
    let temp_dir = TempDir::new().expect("tempdir");
    let listing_dir = temp_dir.path().join("d");
    fs::create_dir(&listing_dir).expect("create listing directory");
    for name in ["a", "bbbbbb", "cccccccccc", "dddddddddddddd"] {
        fs::write(listing_dir.join(name), b"").expect("create file");
    }

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .env("COLUMNS", "40")
        .env("TABSIZE", "3")
        .args(["dir", "d"])
        .output()
        .expect("run syskits dir");

    assert!(output.status.success());
    assert_eq!(output.stdout, b"a\tbbbbbb  cccccccccc  dddddddddddddd\n");
    assert_eq!(output.stderr, b"");
}

#[test]
fn dir_warns_for_invalid_tabsize_environment() {
    let temp_dir = TempDir::new().expect("tempdir");
    let listing_dir = temp_dir.path().join("d");
    fs::create_dir(&listing_dir).expect("create listing directory");
    fs::write(listing_dir.join("a"), b"").expect("create file");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .env("TABSIZE", "bad")
        .args(["dir", "d"])
        .output()
        .expect("run syskits dir");

    assert!(output.status.success());
    assert_eq!(output.stdout, b"a\n");
    assert_eq!(
        output.stderr,
        b"dir: ignoring invalid tab size in environment variable TABSIZE: 'bad'\n"
    );
}

#[test]
fn dir_ignores_empty_columns_environment() {
    let temp_dir = TempDir::new().expect("tempdir");
    let listing_dir = temp_dir.path().join("d");
    fs::create_dir(&listing_dir).expect("create listing directory");
    fs::write(listing_dir.join("a"), b"").expect("create file");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .env("COLUMNS", "")
        .args(["dir", "d"])
        .output()
        .expect("run syskits dir");

    assert!(output.status.success());
    assert_eq!(output.stdout, b"a\n");
    assert_eq!(output.stderr, b"");
}

#[test]
fn dir_ignores_empty_time_style_outside_long_format() {
    let temp_dir = TempDir::new().expect("tempdir");
    let listing_dir = temp_dir.path().join("d");
    fs::create_dir(&listing_dir).expect("create listing directory");
    fs::write(listing_dir.join("a"), b"").expect("create file");

    let from_environment = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .env("TIME_STYLE", "")
        .args(["dir", "d"])
        .output()
        .expect("run syskits dir with empty TIME_STYLE");
    let from_command_line = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args(["dir", "--time-style=", "d"])
        .output()
        .expect("run syskits dir with empty --time-style");

    for output in [from_environment, from_command_line] {
        assert!(output.status.success());
        assert_eq!(output.stdout, b"a\n");
        assert_eq!(output.stderr, b"");
    }
}

#[test]
fn dir_f_resets_long_format_to_default_columns() {
    let temp_dir = TempDir::new().expect("tempdir");
    let listing_dir = temp_dir.path().join("d");
    fs::create_dir(&listing_dir).expect("create listing directory");
    for name in ["a", "b", "c", "d"] {
        fs::write(listing_dir.join(name), b"").expect("create file");
    }

    for options in [["-lfA"], ["-flA"], ["-lf1A"], ["-l1fA"]] {
        let gnu = Command::new("/usr/bin/dir")
            .current_dir(temp_dir.path())
            .env("LC_ALL", "C")
            .env("COLUMNS", "20")
            .args(options)
            .arg("d")
            .output()
            .expect("run GNU dir");
        let syskits = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .current_dir(temp_dir.path())
            .env("LC_ALL", "C")
            .env("COLUMNS", "20")
            .arg("dir")
            .args(options)
            .arg("d")
            .output()
            .expect("run syskits dir");

        assert_same_output(options[0], gnu, syskits);
    }
}

#[test]
fn dir_matches_gnu_multi_column_edge_layout() {
    let temp_dir = TempDir::new().expect("tempdir");
    let listing_dir = temp_dir.path().join("d");
    fs::create_dir(&listing_dir).expect("create listing directory");
    for name in [
        "a",
        "bb",
        "ccc",
        "dddd",
        "eeeee",
        "ffffff",
        "ggggggg",
        "hhhhhhhh",
        "iiiiiiiii",
        "jjjjjjjjjj",
        "k space",
        "l'quote",
        "m*",
    ] {
        fs::write(listing_dir.join(name), b"").expect("create file");
    }

    let gnu = Command::new("/usr/bin/dir")
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args(["-T0", "--width=40", "-r", "d"])
        .output()
        .expect("run GNU dir");
    let syskits = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args(["dir", "-T0", "--width=40", "-r", "d"])
        .output()
        .expect("run syskits dir");

    assert_same_output("multi-column layout edge", gnu, syskits);
}

#[test]
fn dir_classify_does_not_overquote_literal_asterisk() {
    let temp_dir = TempDir::new().expect("tempdir");
    let listing_dir = temp_dir.path().join("d");
    fs::create_dir(&listing_dir).expect("create listing directory");
    fs::write(listing_dir.join("plain*"), b"").expect("create file");

    let gnu = Command::new("/usr/bin/dir")
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args(["-1F", "d"])
        .output()
        .expect("run GNU dir");
    let syskits = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args(["dir", "-1F", "d"])
        .output()
        .expect("run syskits dir");

    assert_same_output("classify literal asterisk", gnu, syskits);
}

#[test]
fn dir_width_sort_accounts_for_shell_quote_alignment() {
    let temp_dir = TempDir::new().expect("tempdir");
    let listing_dir = temp_dir.path().join("d");
    fs::create_dir(&listing_dir).expect("create listing directory");
    for name in ["a", "bb", "ccc", "dddd", "m*"] {
        fs::write(listing_dir.join(name), b"").expect("create file");
    }

    let gnu = Command::new("/usr/bin/dir")
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args(["--width=5", "--quoting-style=shell", "--sort=width", "d"])
        .output()
        .expect("run GNU dir");
    let syskits = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args([
            "dir",
            "--width=5",
            "--quoting-style=shell",
            "--sort=width",
            "d",
        ])
        .output()
        .expect("run syskits dir");

    assert_same_output("width sort shell quote alignment", gnu, syskits);
}

#[test]
fn dir_column_formats_preserve_zero_terminator_order() {
    let temp_dir = TempDir::new().expect("tempdir");
    let listing_dir = temp_dir.path().join("d");
    fs::create_dir(&listing_dir).expect("create listing directory");
    for name in ["a", "b", "line\n name"] {
        fs::write(listing_dir.join(name), b"").expect("create file");
    }

    for options in [
        ["--zero", "-C"],
        ["--zero", "-x"],
        ["-C", "--zero"],
        ["-x", "--zero"],
    ] {
        let gnu = Command::new("/usr/bin/dir")
            .current_dir(temp_dir.path())
            .env("LC_ALL", "C")
            .env("COLUMNS", "20")
            .args(options)
            .arg("d")
            .output()
            .expect("run GNU dir");
        let syskits = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .current_dir(temp_dir.path())
            .env("LC_ALL", "C")
            .env("COLUMNS", "20")
            .arg("dir")
            .args(options)
            .arg("d")
            .output()
            .expect("run syskits dir");

        assert_same_output(&options.join(" "), gnu, syskits);
    }
}

#[cfg(target_os = "linux")]
fn set_selinux_xattr(path: &std::path::Path, value: &[u8]) -> bool {
    let Ok(c_path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };

    unsafe {
        libc::setxattr(
            c_path.as_ptr(),
            c"security.selinux".as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
        ) == 0
    }
}

#[cfg(target_os = "linux")]
#[test]
fn dir_long_shows_selinux_context_marker_from_xattr() {
    let temp_dir = TempDir::new().expect("tempdir");
    let file = temp_dir.path().join("with-context");
    fs::write(&file, b"content").expect("write fixture");

    if !set_selinux_xattr(&file, b"system_u:object_r:tmp_t:s0\0") {
        eprintln!("skipping: current filesystem does not allow setting security.selinux");
        return;
    }

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["dir", "-l", file.to_str().expect("utf8 path")])
        .output()
        .expect("run syskits dir");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    let mode = stdout
        .split_whitespace()
        .next()
        .expect("long listing mode field");
    assert_eq!(mode, "-rw-r--r--.");
}

#[test]
fn dir_f_color_strictly_fits_columns_like_gnu() {
    let temp_dir = TempDir::new().expect("tempdir");
    let tree = temp_dir.path().join("tree");
    fs::create_dir_all(tree.join("sub")).expect("create fixture");
    let tree_arg = tree.to_str().expect("utf8 path");

    let gnu = Command::new("/usr/bin/dir")
        .env("COLUMNS", "10")
        .env("LC_ALL", "C")
        .env("TERM", "xterm")
        .env("LS_COLORS", "di=01;34")
        .args(["-f", "--color=always", "--sort=size", tree_arg])
        .output()
        .expect("run GNU dir");
    let syskits = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("COLUMNS", "10")
        .env("LC_ALL", "C")
        .env("TERM", "xterm")
        .env("LS_COLORS", "di=01;34")
        .args(["dir", "-f", "--color=always", "--sort=size", tree_arg])
        .output()
        .expect("run syskits dir");

    assert_same_output(
        "dir -f --color=always --sort=size narrow columns",
        gnu,
        syskits,
    );
}

#[cfg(unix)]
#[test]
fn dir_long_symlink_target_omits_slash_indicator() {
    let temp_dir = TempDir::new().expect("tempdir");
    let target = temp_dir.path().join("target");
    let link = temp_dir.path().join("link");
    fs::create_dir(&target).expect("create target directory");
    std::os::unix::fs::symlink("target", &link).expect("create symlink");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args(["dir", "-lpn", "--time-style=+T", "-d", "link"])
        .output()
        .expect("run syskits dir");

    assert!(output.status.success());
    assert!(output.stdout.ends_with(b" link -> target\n"));
    assert_eq!(output.stderr, b"");
}

#[cfg(unix)]
#[test]
fn dir_does_not_color_long_symlink_target_for_plain_ln_style() {
    let temp_dir = TempDir::new().expect("tempdir");
    fs::create_dir(temp_dir.path().join("target")).expect("create target directory");
    std::os::unix::fs::symlink("target", temp_dir.path().join("link")).expect("create symlink");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .env("LS_COLORS", "di=34:ln=35")
        .args([
            "dir",
            "-ln",
            "--time-style=+T",
            "--color=always",
            "-d",
            "link",
        ])
        .output()
        .expect("run syskits dir");

    assert!(output.status.success());
    assert!(
        output
            .stdout
            .ends_with(b" \x1b[0m\x1b[35mlink\x1b[0m -> target\n")
    );
    assert_eq!(output.stderr, b"");
}

#[test]
fn dir_keeps_differently_colored_extension_rules_case_sensitive() {
    let temp_dir = TempDir::new().expect("tempdir");
    for name in ["img1.jpg", "IMG2.JPG", "img3.JpG"] {
        fs::write(temp_dir.path().join(name), b"").expect("create image fixture");
    }

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .env("LS_COLORS", "*.jpg=01;35:*.JPG=01;35;46")
        .args([
            "dir",
            "-U1",
            "--color=always",
            "-d",
            "img1.jpg",
            "IMG2.JPG",
            "img3.JpG",
        ])
        .output()
        .expect("run syskits dir");

    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        b"\x1b[0m\x1b[01;35mimg1.jpg\x1b[0m\n\x1b[01;35;46mIMG2.JPG\x1b[0m\nimg3.JpG\n"
    );
    assert_eq!(output.stderr, b"");
}

#[test]
fn dir_applies_normal_color_to_long_fields_and_explicit_file_color_to_names() {
    let temp_dir = TempDir::new().expect("tempdir");
    fs::write(temp_dir.path().join("plain"), b"").expect("create plain file");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .env("LS_COLORS", "no=7:fi=4")
        .args([
            "dir",
            "-gG",
            "--time-style=+T",
            "--color=always",
            "-d",
            "plain",
        ])
        .output()
        .expect("run syskits dir");

    assert!(output.status.success());
    assert!(output.stdout.starts_with(b"\x1b[0m\x1b[7m-rw-r--r--"));
    assert!(output.stdout.ends_with(b" T \x1b[m\x1b[4mplain\x1b[0m\n"));
    assert_eq!(output.stderr, b"");
}

#[test]
fn dir_clears_to_eol_when_a_colored_long_name_crosses_the_line_edge() {
    let temp_dir = TempDir::new().expect("tempdir");
    let name = "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz.foo";
    fs::write(temp_dir.path().join(name), b"").expect("create long file");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .env("COLUMNS", "80")
        .env("LS_COLORS", "*.foo=0;31;42")
        .args([
            "dir",
            "-og",
            "--time-style=+T",
            "--color=always",
            "-d",
            name,
        ])
        .output()
        .expect("run syskits dir");

    assert!(output.status.success());
    assert!(output.stdout.ends_with(b".foo\x1b[0m\x1b[K\n"));
    assert_eq!(output.stderr, b"");
}

#[test]
fn dir_diagnoses_invalid_ls_colors_and_disables_coloring() {
    let temp_dir = TempDir::new().expect("tempdir");
    let listing_dir = temp_dir.path().join("d");
    fs::create_dir(&listing_dir).expect("create listing directory");
    fs::write(listing_dir.join("a"), b"").expect("create file");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .env("LS_COLORS", "bogus")
        .args(["dir", "--color=always", "-1", "d"])
        .output()
        .expect("run syskits dir");

    assert!(output.status.success());
    assert_eq!(output.stdout, b"a\n");
    assert_eq!(
        output.stderr,
        b"dir: unparsable value for LS_COLORS environment variable\n"
    );
}

#[cfg(unix)]
#[test]
fn dir_reports_dereference_eloop_and_uses_placeholder_metadata() {
    let temp_dir = TempDir::new().expect("tempdir");
    let listing_dir = temp_dir.path().join("d");
    fs::create_dir(&listing_dir).expect("create listing directory");
    std::os::unix::fs::symlink("loop", listing_dir.join("loop")).expect("create symlink loop");

    let mut gnu_command = Command::new("/usr/bin/dir");
    gnu_command
        .arg0("dir")
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args(["-Li", "d"]);
    let gnu = gnu_command.output().expect("run GNU dir");
    let syskits = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args(["dir", "-Li", "d"])
        .output()
        .expect("run syskits dir");

    assert_same_output("dir -Li ELOOP", gnu, syskits);
}

#[test]
fn dir_reports_invalid_options_like_gnu() {
    for option in ["--definitely-invalid", "--h", "-Fnever"] {
        let mut gnu_command = Command::new("/usr/bin/dir");
        gnu_command.arg0("dir").env("LC_ALL", "C").arg(option);
        let gnu = gnu_command.output().expect("run GNU dir");
        let syskits = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .env("LC_ALL", "C")
            .args(["dir", option])
            .output()
            .expect("run syskits dir");

        assert_same_output(option, gnu, syskits);
    }
}

#[test]
fn dir_reports_missing_option_arguments_like_gnu() {
    for option in ["--block-size", "-I", "--time-style"] {
        let mut gnu_command = Command::new("/usr/bin/dir");
        gnu_command.arg0("dir").env("LC_ALL", "C").arg(option);
        let gnu = gnu_command.output().expect("run GNU dir");
        let syskits = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .env("LC_ALL", "C")
            .args(["dir", option])
            .output()
            .expect("run syskits dir");

        assert_same_output(option, gnu, syskits);
    }
}

#[test]
fn dir_rejects_help_and_version_arguments_like_gnu() {
    for option in ["--help=x", "--version=x"] {
        let mut gnu_command = Command::new("/usr/bin/dir");
        gnu_command.arg0("dir").env("LC_ALL", "C").arg(option);
        let gnu = gnu_command.output().expect("run GNU dir");
        let syskits = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .env("LC_ALL", "C")
            .args(["dir", option])
            .output()
            .expect("run syskits dir");

        assert_same_output(option, gnu, syskits);
    }
}

#[test]
fn dir_uses_failure_status_for_invalid_enum_values() {
    for option in [
        "--quoting-style=bogus",
        "--indicator-style=bogus",
        "--hyperlink=bogus",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .env("LC_ALL", "C")
            .args(["dir", option])
            .output()
            .expect("run syskits dir");

        assert_eq!(
            output.status.code(),
            Some(1),
            "unexpected status for {option}"
        );
    }
}

#[test]
fn dir_reports_invalid_enum_values_like_gnu() {
    for option in ["--sort=bogus", "--color=bogus", "--quoting-style=bogus"] {
        let mut gnu_command = Command::new("/usr/bin/dir");
        gnu_command.arg0("dir").env("LC_ALL", "C").arg(option);
        let gnu = gnu_command.output().expect("run GNU dir");
        let syskits = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .env("LC_ALL", "C")
            .args(["dir", option])
            .output()
            .expect("run syskits dir");

        assert_same_output(option, gnu, syskits);
    }
}
