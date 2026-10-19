use std::fs;
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStrExt;
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStringExt;
use std::process::Command;
#[cfg(target_os = "linux")]
use std::process::Stdio;

use tempfile::TempDir;

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
fn vdir_quotes_non_utf8_file_name_bytes() {
    let temp_dir = TempDir::new().expect("tempdir");
    let name = std::ffi::OsString::from_vec(b"bad\xffname".to_vec());
    fs::write(temp_dir.path().join(&name), b"").expect("write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args([std::ffi::OsStr::new("vdir"), std::ffi::OsStr::new("-1")])
        .output()
        .expect("run syskits vdir");

    assert!(output.status.success());
    assert_eq!(output.stderr, b"");
    assert_eq!(output.stdout, b"bad\\377name\n");
}

#[cfg(target_os = "linux")]
#[test]
fn vdir_keeps_non_utf8_hidden_names_hidden() {
    let temp_dir = TempDir::new().expect("tempdir");
    let hidden = std::ffi::OsString::from_vec(b".hidden\xff".to_vec());
    fs::write(temp_dir.path().join(hidden), b"").expect("write hidden fixture");
    fs::write(temp_dir.path().join("visible"), b"").expect("write visible fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args(["vdir", "-1"])
        .output()
        .expect("run syskits vdir");

    assert!(output.status.success());
    assert_eq!(output.stderr, b"");
    assert_eq!(output.stdout, b"visible\n");
}

#[cfg(target_os = "linux")]
#[test]
fn vdir_quotes_non_utf8_symlink_target_bytes() {
    let temp_dir = TempDir::new().expect("tempdir");
    let target = std::ffi::OsString::from_vec(b"target\xff".to_vec());
    std::os::unix::fs::symlink(target, temp_dir.path().join("link"))
        .expect("create symlink fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args(["vdir", "-ln", "--time-style=+T", "-d", "link"])
        .output()
        .expect("run syskits vdir");

    assert!(output.status.success());
    assert_eq!(output.stderr, b"");
    assert!(output.stdout.ends_with(b" T link -> target\\377\n"));
}

#[cfg(target_os = "linux")]
#[test]
fn vdir_version_sorts_original_non_utf8_name_bytes() {
    let temp_dir = TempDir::new().expect("tempdir");
    let lower = std::ffi::OsString::from_vec(b"v\xfe".to_vec());
    let higher = std::ffi::OsString::from_vec(b"v\xff".to_vec());
    fs::write(temp_dir.path().join(&lower), b"").expect("write lower fixture");
    fs::write(temp_dir.path().join(&higher), b"").expect("write higher fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args([
            std::ffi::OsStr::new("vdir"),
            std::ffi::OsStr::new("-1v"),
            &higher,
            &lower,
        ])
        .output()
        .expect("run syskits vdir");

    assert!(output.status.success());
    assert_eq!(output.stderr, b"");
    assert_eq!(output.stdout, b"v\\376\nv\\377\n");
}

#[cfg(target_os = "linux")]
#[test]
fn vdir_preserves_non_utf8_invalid_option_byte() {
    let invalid_option = std::ffi::OsString::from_vec(b"-\xff".to_vec());
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C")
        .args([std::ffi::OsStr::new("vdir"), invalid_option.as_os_str()])
        .output()
        .expect("run syskits vdir");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(output.stdout, b"");
    assert_eq!(
        output.stderr,
        b"vdir: invalid option -- '\xff'\nTry 'vdir --help' for more information.\n"
    );
}

#[test]
fn vdir_reports_invalid_time_style_like_gnu() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C")
        .args(["vdir", "--time-style=bogus"])
        .output()
        .expect("run syskits vdir");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(output.stdout, b"");
    assert_eq!(
        output.stderr,
        concat!(
            "vdir: invalid argument 'bogus' for 'time style'\n",
            "Valid arguments are:\n",
            "  - [posix-]full-iso\n",
            "  - [posix-]long-iso\n",
            "  - [posix-]iso\n",
            "  - [posix-]locale\n",
            "  - +FORMAT (e.g., +%H:%M) for a 'date'-style format\n",
            "Try 'vdir --help' for more information.\n",
        )
        .as_bytes()
    );
}

#[test]
fn vdir_accepts_unambiguous_option_value_abbreviations() {
    for argument in [
        "--sort=ver",
        "--time=acc",
        "--format=vert",
        "--indicator-style=cla",
        "--color=nev",
        "--hyperlink=nev",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .env("LC_ALL", "C")
            .args(["vdir", argument, "."])
            .output()
            .expect("run syskits vdir");

        assert!(
            output.status.success(),
            "{argument}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn vdir_reports_unknown_option_like_gnu() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C")
        .args(["vdir", "--definitely-invalid"])
        .output()
        .expect("run syskits vdir");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(output.stdout, b"");
    assert_eq!(
        output.stderr,
        b"vdir: unrecognized option '--definitely-invalid'\n\
Try 'vdir --help' for more information.\n"
    );
}

#[test]
fn vdir_reports_help_and_version_arguments_like_gnu() {
    for option in ["--help=x", "--version=x"] {
        let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .env("LC_ALL", "C")
            .args(["vdir", option])
            .output()
            .expect("run syskits vdir");
        let option_name = option.split('=').next().expect("option name");

        assert_eq!(output.status.code(), Some(2));
        assert_eq!(output.stdout, b"");
        assert_eq!(
            String::from_utf8(output.stderr).expect("UTF-8 diagnostic"),
            format!(
                "vdir: option '{option_name}' doesn't allow an argument\n\
                 Try 'vdir --help' for more information.\n"
            )
        );
    }
}

#[test]
fn vdir_help_uses_gnu_compatible_content() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C")
        .args(["vdir", "--help"])
        .output()
        .expect("run syskits vdir");

    assert!(output.status.success());
    assert!(
        output
            .stdout
            .starts_with(b"Usage: vdir [OPTION]... [FILE]...\n")
    );
    assert!(
        output
            .stdout
            .windows(12)
            .any(|bytes| bytes == b"Exit status:")
    );
    assert!(
        output
            .stdout
            .ends_with(b"or available locally via: info '(coreutils) vdir invocation'\n")
    );
}

#[test]
fn vdir_help_and_version_display_do_not_write_stderr() {
    for option in ["--help", "--version", "--he", "--ver"] {
        let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .env("LC_ALL", "C")
            .args(["vdir", option])
            .output()
            .expect("run syskits vdir");

        assert!(output.status.success(), "failed to display {option}");
        assert_eq!(output.stderr, b"", "unexpected stderr for {option}");
    }
}

#[test]
fn vdir_posixly_correct_stops_parsing_options_after_first_operand() {
    let temp_dir = TempDir::new().expect("tempdir");
    fs::write(temp_dir.path().join("a"), b"").expect("write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .env("POSIXLY_CORRECT", "1")
        .args(["vdir", "a", "-d"])
        .output()
        .expect("run syskits vdir");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        output.stdout.iter().filter(|&&byte| byte == b'\n').count(),
        1
    );
    assert_eq!(
        output.stderr.iter().filter(|&&byte| byte == b'\n').count(),
        1
    );
    assert!(output.stderr.windows(4).any(|bytes| bytes == b"'-d'"));
}

#[test]
fn vdir_invalid_quoting_environment_warning_matches_gnu() {
    let temp_dir = TempDir::new().expect("tempdir");
    fs::write(temp_dir.path().join("f"), b"").expect("write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .env("QUOTING_STYLE", "bogus")
        .args(["vdir", "-1", "-d", "f"])
        .output()
        .expect("run syskits vdir");

    assert!(output.status.success());
    assert_eq!(
        output.stderr,
        b"vdir: ignoring invalid value of environment variable QUOTING_STYLE: 'bogus'\n"
    );
}

#[test]
fn vdir_localizes_unknown_option_diagnostic() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "fr_FR.UTF-8")
        .args(["vdir", "--definitely-invalid"])
        .output()
        .expect("run syskits vdir");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(output.stdout, b"");
    assert_eq!(
        output.stderr,
        "vdir\u{a0}: option non reconnue '--definitely-invalid'\n\n\
         Saisissez «\u{a0}vdir --help\u{a0}» pour plus d'informations.\n"
            .as_bytes()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn vdir_localizes_missing_file_diagnostic() {
    let locale_available = Command::new("locale")
        .arg("-a")
        .output()
        .expect("list locales")
        .stdout
        .split(|byte| *byte == b'\n')
        .any(|locale| locale.eq_ignore_ascii_case(b"fr_FR.utf8"));
    if !locale_available {
        return;
    }

    let temp_dir = TempDir::new().expect("tempdir");
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "fr_FR.UTF-8")
        .args(["vdir", "missing"])
        .output()
        .expect("run syskits vdir");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(output.stdout, b"");
    assert_eq!(
        output.stderr,
        "vdir: impossible d'accéder à 'missing': Aucun fichier ou dossier de ce type\n".as_bytes()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn vdir_localizes_help_output() {
    let locale_available = Command::new("locale")
        .arg("-a")
        .output()
        .expect("list locales")
        .stdout
        .split(|byte| *byte == b'\n')
        .any(|locale| locale.eq_ignore_ascii_case(b"fr_FR.utf8"));
    if !locale_available {
        return;
    }

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "fr_FR.UTF-8")
        .args(["vdir", "--help"])
        .output()
        .expect("run syskits vdir");

    assert!(output.status.success());
    assert_eq!(output.stderr, b"");
    assert!(
        output
            .stdout
            .starts_with("Utilisation\u{a0}: vdir [OPTION]... [FICHIER]...\n".as_bytes())
    );
    assert!(output.stdout.ends_with(
        "ou disponible localement via: info '(coreutils) vdir invocation'\n".as_bytes()
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn vdir_reports_closed_stdout() {
    use std::os::unix::process::CommandExt;

    let temp_dir = TempDir::new().expect("tempdir");
    fs::write(temp_dir.path().join("f"), b"").expect("write fixture");
    let mut command = Command::new(env!("CARGO_BIN_EXE_syskits"));
    command
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args(["vdir", "-1", "-d", "f"])
        .stdout(Stdio::piped());
    unsafe {
        command.pre_exec(|| {
            libc::close(libc::STDOUT_FILENO);
            Ok(())
        });
    }
    let output = command.output().expect("run syskits vdir");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(output.stdout, b"");
    assert_eq!(output.stderr, b"vdir: write error: Bad file descriptor\n");
}

#[cfg(target_os = "linux")]
#[test]
fn vdir_terminates_on_broken_stdout_pipe() {
    use std::os::unix::process::{CommandExt, ExitStatusExt};

    let temp_dir = TempDir::new().expect("tempdir");
    fs::write(temp_dir.path().join("f"), b"").expect("write fixture");
    let mut command = Command::new(env!("CARGO_BIN_EXE_syskits"));
    command
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args(["vdir", "-1", "-d", "f"])
        .stdout(Stdio::piped());
    unsafe {
        command.pre_exec(|| {
            let mut pipe_fds = [0; 2];
            if libc::pipe(pipe_fds.as_mut_ptr()) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            libc::close(pipe_fds[0]);
            if libc::dup2(pipe_fds[1], libc::STDOUT_FILENO) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            libc::close(pipe_fds[1]);
            Ok(())
        });
    }
    let output = command.output().expect("run syskits vdir");

    assert_eq!(output.status.signal(), Some(libc::SIGPIPE));
    assert_eq!(output.stdout, b"");
    assert_eq!(output.stderr, b"");
}

#[cfg(target_os = "linux")]
#[test]
fn vdir_preserves_inherited_ignored_sigpipe() {
    use std::os::unix::process::CommandExt;

    let temp_dir = TempDir::new().expect("tempdir");
    fs::write(temp_dir.path().join("f"), b"").expect("write fixture");
    let mut command = Command::new(env!("CARGO_BIN_EXE_syskits"));
    command
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args(["vdir", "-1", "-d", "f"])
        .stdout(Stdio::piped());
    unsafe {
        command.pre_exec(|| {
            let mut pipe_fds = [0; 2];
            if libc::pipe(pipe_fds.as_mut_ptr()) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            libc::close(pipe_fds[0]);
            if libc::dup2(pipe_fds[1], libc::STDOUT_FILENO) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            libc::close(pipe_fds[1]);
            libc::signal(libc::SIGPIPE, libc::SIG_IGN);
            Ok(())
        });
    }
    let output = command.output().expect("run syskits vdir");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(output.stdout, b"");
    assert_eq!(output.stderr, b"vdir: write error: Broken pipe\n");
}

#[cfg(target_os = "linux")]
#[test]
fn vdir_does_not_panic_on_broken_stderr_pipe() {
    use std::os::unix::process::CommandExt;

    let temp_dir = TempDir::new().expect("tempdir");
    let mut command = Command::new(env!("CARGO_BIN_EXE_syskits"));
    command
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args(["vdir", "missing"])
        .stderr(Stdio::piped());
    unsafe {
        command.pre_exec(|| {
            let mut pipe_fds = [0; 2];
            if libc::pipe(pipe_fds.as_mut_ptr()) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            libc::close(pipe_fds[0]);
            if libc::dup2(pipe_fds[1], libc::STDERR_FILENO) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            libc::close(pipe_fds[1]);
            libc::signal(libc::SIGPIPE, libc::SIG_IGN);
            Ok(())
        });
    }
    let output = command.output().expect("run syskits vdir");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(output.stdout, b"");
    assert_eq!(output.stderr, b"");
}

#[cfg(target_os = "linux")]
#[test]
fn vdir_help_and_version_report_closed_stdout() {
    use std::os::unix::process::CommandExt;

    for option in ["--help", "--version"] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_syskits"));
        command
            .env("LC_ALL", "C")
            .args(["vdir", option])
            .stdout(Stdio::piped());
        unsafe {
            command.pre_exec(|| {
                libc::close(libc::STDOUT_FILENO);
                Ok(())
            });
        }
        let output = command.output().expect("run syskits vdir");

        assert_eq!(output.status.code(), Some(2), "{option}");
        assert_eq!(output.stdout, b"", "{option}");
        assert_eq!(
            output.stderr, b"vdir: write error: Bad file descriptor\n",
            "{option}"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn vdir_help_and_version_report_broken_stdout_pipe() {
    use std::os::unix::process::CommandExt;

    for option in ["--help", "--version"] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_syskits"));
        command
            .env("LC_ALL", "C")
            .args(["vdir", option])
            .stdout(Stdio::piped());
        unsafe {
            command.pre_exec(|| {
                let mut pipe_fds = [0; 2];
                if libc::pipe(pipe_fds.as_mut_ptr()) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                libc::close(pipe_fds[0]);
                if libc::dup2(pipe_fds[1], libc::STDOUT_FILENO) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                libc::close(pipe_fds[1]);
                libc::signal(libc::SIGPIPE, libc::SIG_IGN);
                Ok(())
            });
        }
        let output = command.output().expect("run syskits vdir");

        assert_eq!(output.status.code(), Some(2), "{option}");
        assert_eq!(output.stdout, b"", "{option}");
        assert_eq!(
            output.stderr, b"vdir: write error: Broken pipe\n",
            "{option}"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn vdir_quotes_utf8_bytes_that_are_invalid_in_c_locale() {
    let temp_dir = TempDir::new().expect("tempdir");
    fs::write(temp_dir.path().join("é"), b"").expect("write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args(["vdir", "-1", "."])
        .output()
        .expect("run syskits vdir");

    assert!(output.status.success());
    assert_eq!(output.stderr, b"");
    assert_eq!(output.stdout, b"\\303\\251\n");
}

#[cfg(target_os = "linux")]
#[test]
fn vdir_shell_style_does_not_quote_unescaped_bell() {
    let temp_dir = TempDir::new().expect("tempdir");
    let name = std::ffi::OsString::from_vec(b"q\x07".to_vec());
    fs::write(temp_dir.path().join(name), b"").expect("write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args(["vdir", "-1", "--quoting-style=shell", "."])
        .output()
        .expect("run syskits vdir");

    assert!(output.status.success());
    assert_eq!(output.stderr, b"");
    assert_eq!(output.stdout, b"q\x07\n");
}

#[test]
fn vdir_long_aligns_single_and_double_shell_quotes() {
    let temp_dir = TempDir::new().expect("tempdir");
    for name in ["plain", "a space", "a'quote"] {
        fs::write(temp_dir.path().join(name), b"").expect("write fixture");
    }

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args([
            "vdir",
            "-ln",
            "--time-style=+T",
            "--quoting-style=shell",
            ".",
        ])
        .output()
        .expect("run syskits vdir");

    assert!(output.status.success());
    assert_eq!(output.stderr, b"");
    let lines = String::from_utf8(output.stdout).expect("utf8 output");
    let names = lines
        .lines()
        .skip(1)
        .map(|line| line.rsplit_once(" T ").expect("time separator").1)
        .collect::<Vec<_>>();
    assert_eq!(names, ["'a space'", "\"a'quote\"", " plain"]);
}

#[test]
fn vdir_uses_locale_specific_quoting_marks() {
    let temp_dir = TempDir::new().expect("tempdir");
    fs::write(temp_dir.path().join("a space"), b"").expect("write fixture");

    for style in ["locale", "clocale"] {
        let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .current_dir(temp_dir.path())
            .env("LC_ALL", "C.UTF-8")
            .args(["vdir", "-1", &format!("--quoting-style={style}"), "."])
            .output()
            .expect("run syskits vdir");

        assert!(output.status.success());
        assert_eq!(output.stderr, b"");
        assert_eq!(output.stdout, "‘a space’\n".as_bytes());
    }
}

#[test]
fn vdir_honors_quoting_style_environment_variable() {
    let temp_dir = TempDir::new().expect("tempdir");
    fs::write(temp_dir.path().join("a space"), b"").expect("write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .env("QUOTING_STYLE", "shell")
        .args(["vdir", "-1", "-d", "a space"])
        .output()
        .expect("run syskits vdir");

    assert!(output.status.success());
    assert_eq!(output.stderr, b"");
    assert_eq!(output.stdout, b"'a space'\n");
}

#[test]
fn vdir_quotes_literal_indicator_characters_when_indicators_are_enabled() {
    let temp_dir = TempDir::new().expect("tempdir");
    for name in ["plain*", "plain@", "plain|"] {
        fs::write(temp_dir.path().join(name), b"").expect("write fixture");
    }

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args(["vdir", "-1", "--file-type", "."])
        .output()
        .expect("run syskits vdir");

    assert!(output.status.success());
    assert_eq!(output.stderr, b"");
    assert_eq!(output.stdout, b"plain\\*\nplain\\@\nplain\\|\n");
}

#[test]
fn vdir_quotes_colons_in_directory_titles() {
    let temp_dir = TempDir::new().expect("tempdir");
    fs::create_dir(temp_dir.path().join("a:b")).expect("create colon directory");
    fs::create_dir(temp_dir.path().join("other")).expect("create other directory");
    fs::write(temp_dir.path().join("a:b/f"), b"").expect("write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args(["vdir", "-1", "a:b", "other"])
        .output()
        .expect("run syskits vdir");

    assert!(output.status.success());
    assert_eq!(output.stderr, b"");
    assert!(output.stdout.starts_with(b"a\\:b:\n"));
}

#[test]
fn vdir_uses_shell_aware_quoting_for_diagnostic_filenames() {
    let temp_dir = TempDir::new().expect("tempdir");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C.UTF-8")
        .args(["vdir", "a'b"])
        .output()
        .expect("run syskits vdir");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(output.stdout, b"");
    assert_eq!(
        output.stderr,
        b"vdir: cannot access \"a'b\": No such file or directory\n"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn vdir_percent_encodes_original_hyperlink_path_bytes() {
    let temp_dir = TempDir::new().expect("tempdir");
    fs::write(temp_dir.path().join("é"), b"").expect("write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args(["vdir", "-1", "--hyperlink=always", "-d", "é"])
        .output()
        .expect("run syskits vdir");

    assert!(output.status.success());
    assert_eq!(output.stderr, b"");
    assert!(
        output
            .stdout
            .windows(b"%c3%a9\x07\\303\\251".len())
            .any(|window| window == b"%c3%a9\x07\\303\\251")
    );
}

#[test]
fn vdir_sorts_by_rendered_display_width() {
    let temp_dir = TempDir::new().expect("tempdir");
    for name in ["aa", "é", "中"] {
        fs::write(temp_dir.path().join(name), b"").expect("write fixture");
    }

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C.UTF-8")
        .args(["vdir", "-1", "--sort=width", "-d", "aa", "é", "中"])
        .output()
        .expect("run syskits vdir");

    assert!(output.status.success());
    assert_eq!(output.stderr, b"");
    assert_eq!(output.stdout, "é\naa\n中\n".as_bytes());
}

#[test]
fn vdir_sorts_extensions_from_the_last_dot() {
    let temp_dir = TempDir::new().expect("tempdir");
    for name in [
        ".leading",
        ".profile.txt",
        "foo.",
        "foo..a",
        "b.txt",
        "c.tar.gz",
    ] {
        fs::write(temp_dir.path().join(name), b"").expect("write fixture");
    }

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args([
            "vdir",
            "-X",
            "-1",
            "-d",
            ".leading",
            ".profile.txt",
            "foo.",
            "foo..a",
            "b.txt",
            "c.tar.gz",
        ])
        .output()
        .expect("run syskits vdir");

    assert!(output.status.success());
    assert_eq!(output.stderr, b"");
    assert_eq!(
        output.stdout,
        b"foo.\nfoo..a\nc.tar.gz\n.leading\n.profile.txt\nb.txt\n"
    );
}

#[test]
fn vdir_ignores_reverse_when_sorting_is_disabled() {
    let temp_dir = TempDir::new().expect("tempdir");
    for name in ["first", "second", "third"] {
        fs::write(temp_dir.path().join(name), b"").expect("write fixture");
    }

    for no_sort in ["-U", "-f"] {
        let normal = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .current_dir(temp_dir.path())
            .env("LC_ALL", "C")
            .args(["vdir", "-1", no_sort, "."])
            .output()
            .expect("run syskits vdir");
        let reversed = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .current_dir(temp_dir.path())
            .env("LC_ALL", "C")
            .args(["vdir", "-1", no_sort, "-r", "."])
            .output()
            .expect("run reversed syskits vdir");

        assert!(normal.status.success());
        assert!(reversed.status.success());
        assert_eq!(normal.stderr, b"");
        assert_eq!(reversed.stderr, b"");
        assert_eq!(reversed.stdout, normal.stdout, "mode {no_sort}");
    }
}

#[test]
fn vdir_uses_fnmatch_semantics_for_ignore_patterns() {
    let temp_dir = TempDir::new().expect("tempdir");
    for name in ["[", "abc", "axc", "x"] {
        fs::write(temp_dir.path().join(name), b"").expect("write fixture");
    }

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args(["vdir", "-1", "--ignore=[", "--ignore=a**c", "."])
        .output()
        .expect("run syskits vdir");

    assert!(output.status.success());
    assert_eq!(output.stderr, b"");
    assert_eq!(output.stdout, b"x\n");
}

#[test]
fn vdir_applies_time_style_to_its_default_long_format() {
    let temp_dir = TempDir::new().expect("tempdir");
    let file = temp_dir.path().join("f");
    fs::write(&file, b"").expect("write fixture");
    filetime::set_file_mtime(&file, filetime::FileTime::from_unix_time(946_782_245, 0))
        .expect("set fixture mtime");

    let cli = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .env("TZ", "UTC0")
        .env_remove("TIME_STYLE")
        .args(["vdir", "--time-style=long-iso", "-d", "f"])
        .output()
        .expect("run syskits vdir with time-style option");
    let environment = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .env("TZ", "UTC0")
        .env("TIME_STYLE", "full-iso")
        .args(["vdir", "-d", "f"])
        .output()
        .expect("run syskits vdir with TIME_STYLE");

    assert!(cli.status.success());
    assert!(environment.status.success());
    assert_eq!(cli.stderr, b"");
    assert_eq!(environment.stderr, b"");
    assert!(cli.stdout.ends_with(b"2000-01-02 03:04 f\n"));
    assert!(
        environment
            .stdout
            .ends_with(b"2000-01-02 03:04:05.000000000 +0000 f\n")
    );
}

#[test]
fn vdir_expands_nanoseconds_in_custom_time_format() {
    let temp_dir = TempDir::new().expect("tempdir");
    let file = temp_dir.path().join("f");
    fs::write(&file, b"").expect("write fixture");
    filetime::set_file_mtime(
        &file,
        filetime::FileTime::from_unix_time(1_577_934_245, 123_456_789),
    )
    .expect("set fixture mtime");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .env("TZ", "UTC0")
        .args(["vdir", "-ln", "--time-style=+%s.%N", "-d", "f"])
        .output()
        .expect("run syskits vdir");

    assert!(output.status.success());
    assert_eq!(output.stderr, b"");
    assert!(output.stdout.ends_with(b"1577934245.123456789 f\n"));
}

#[test]
fn vdir_aligns_unavailable_birth_time_placeholder() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C")
        .env("TZ", "UTC0")
        .args([
            "vdir",
            "-ln",
            "--time=birth",
            "--full-time",
            "-d",
            "/proc/self/status",
        ])
        .output()
        .expect("run syskits vdir");

    assert!(output.status.success());
    assert_eq!(output.stderr, b"");
    let line = String::from_utf8(output.stdout).expect("utf8 output");
    let prefix = line
        .strip_suffix(" /proc/self/status\n")
        .expect("listed proc file");
    assert!(prefix.ends_with(&format!("{:>35}", "?")));
}

#[test]
fn vdir_localizes_locale_time_fields() {
    let temp_dir = TempDir::new().expect("tempdir");
    let file = temp_dir.path().join("f");
    fs::write(&file, b"").expect("write fixture");
    filetime::set_file_mtime(&file, filetime::FileTime::from_unix_time(946_782_245, 0))
        .expect("set fixture mtime");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "fr_FR.UTF-8")
        .env("TZ", "UTC0")
        .args(["vdir", "-ln", "--time-style=locale", "-d", "f"])
        .output()
        .expect("run syskits vdir");

    assert!(output.status.success());
    assert_eq!(output.stderr, b"");
    assert!(output.stdout.ends_with(" 2 janv.  2000 f\n".as_bytes()));
}

#[test]
fn vdir_promotes_rounded_human_readable_sizes() {
    let temp_dir = TempDir::new().expect("tempdir");
    let binary = temp_dir.path().join("binary");
    let decimal = temp_dir.path().join("decimal");
    fs::File::create(&binary)
        .expect("create binary fixture")
        .set_len(1_048_575)
        .expect("size binary fixture");
    fs::File::create(&decimal)
        .expect("create decimal fixture")
        .set_len(999_999)
        .expect("size decimal fixture");

    for (option, name) in [("-h", "binary"), ("--si", "decimal")] {
        let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .current_dir(temp_dir.path())
            .env("LC_ALL", "C")
            .args(["vdir", "-ln", option, "--full-time", "-d", name])
            .output()
            .expect("run syskits vdir");

        assert!(output.status.success());
        assert_eq!(output.stderr, b"");
        let stdout = String::from_utf8(output.stdout).expect("utf8 output");
        assert_eq!(stdout.split_whitespace().nth(4), Some("1.0M"));
    }
}

#[test]
fn vdir_handles_zero_block_size_without_panicking() {
    let temp_dir = TempDir::new().expect("tempdir");
    fs::write(temp_dir.path().join("f"), b"").expect("write fixture");

    let environment = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .env("BLOCK_SIZE", "0")
        .args(["vdir", "-s", "-1", "."])
        .output()
        .expect("run syskits vdir");
    let option = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args(["vdir", "-s", "-1", "--block-size=0", "."])
        .output()
        .expect("run syskits vdir with zero option");

    assert!(environment.status.success());
    assert_eq!(environment.stderr, b"");
    assert_eq!(option.status.code(), Some(2));
    assert!(
        option
            .stderr
            .starts_with(b"vdir: invalid --block-size argument")
    );
    assert!(!option.stderr.windows(8).any(|bytes| bytes == b"panicked"));
}

#[test]
fn vdir_accepts_gnu_block_size_numeric_syntax() {
    let temp_dir = TempDir::new().expect("tempdir");
    fs::write(temp_dir.path().join("f"), b"").expect("write fixture");

    for block_size in [" 3", "+3"] {
        let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .current_dir(temp_dir.path())
            .env("LC_ALL", "C")
            .args([
                "vdir",
                "-s",
                "-1",
                &format!("--block-size={block_size}"),
                ".",
            ])
            .output()
            .expect("run syskits vdir");

        assert!(output.status.success(), "block size {block_size:?}");
        assert_eq!(output.stderr, b"", "block size {block_size:?}");
    }
}

#[test]
fn vdir_canonicalizes_block_size_suffixes() {
    let temp_dir = TempDir::new().expect("tempdir");
    fs::File::create(temp_dir.path().join("f"))
        .expect("create fixture")
        .set_len(1001)
        .expect("size fixture");

    for (block_size, expected) in [("KB", "2kB"), ("k", "1K"), ("KiB", "1KiB")] {
        let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .current_dir(temp_dir.path())
            .env("LC_ALL", "C")
            .args([
                "vdir",
                "-ln",
                "--full-time",
                &format!("--block-size={block_size}"),
                "-d",
                "f",
            ])
            .output()
            .expect("run syskits vdir");

        assert!(output.status.success(), "block size {block_size}");
        assert_eq!(output.stderr, b"", "block size {block_size}");
        let stdout = String::from_utf8(output.stdout).expect("utf8 output");
        assert_eq!(stdout.split_whitespace().nth(4), Some(expected));
    }
}

#[test]
fn vdir_accepts_gnu_width_numeric_syntax_and_range() {
    let temp_dir = TempDir::new().expect("tempdir");
    fs::write(temp_dir.path().join("a"), b"").expect("write fixture");
    fs::write(temp_dir.path().join("b"), b"").expect("write fixture");

    let hexadecimal = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args(["vdir", "-C", "--width=0x10", "."])
        .output()
        .expect("run syskits vdir");
    let large_environment = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .env("COLUMNS", "70000")
        .args(["vdir", "-C", "."])
        .output()
        .expect("run syskits vdir with COLUMNS");

    assert!(hexadecimal.status.success());
    assert_eq!(hexadecimal.stderr, b"");
    assert!(large_environment.status.success());
    assert_eq!(large_environment.stderr, b"");
}

#[test]
fn vdir_accepts_gnu_tabsize_syntax_and_rejects_ptrdiff_overflow() {
    let temp_dir = TempDir::new().expect("tempdir");
    fs::write(temp_dir.path().join("a"), b"").expect("write fixture");

    let whitespace = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args(["vdir", "-C", "--tabsize= 3", "."])
        .output()
        .expect("run syskits vdir");
    let overflow = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args(["vdir", "-C", "--tabsize=18446744073709551615", "."])
        .output()
        .expect("run syskits vdir with overflowing tab size");

    assert!(whitespace.status.success());
    assert_eq!(whitespace.stderr, b"");
    assert_eq!(overflow.status.code(), Some(2));
}

#[cfg(unix)]
#[test]
fn vdir_places_long_symlink_indicator_after_target() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let temp_dir = TempDir::new().expect("tempdir");
    let target = temp_dir.path().join("target");
    fs::write(&target, b"").expect("write fixture");
    fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).expect("chmod fixture");
    symlink("target", temp_dir.path().join("link")).expect("create symlink");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args(["vdir", "-lFn", "--full-time", "-d", "link"])
        .output()
        .expect("run syskits vdir");

    assert!(output.status.success());
    assert_eq!(output.stderr, b"");
    assert!(output.stdout.ends_with(b" link -> target*\n"));
    assert!(!output.stdout.windows(9).any(|bytes| bytes == b" link@ ->"));
}

#[cfg(unix)]
#[test]
fn vdir_hyperlinks_both_symlink_name_and_target() {
    use std::os::unix::fs::symlink;

    let temp_dir = TempDir::new().expect("tempdir");
    fs::write(temp_dir.path().join("target"), b"").expect("write fixture");
    symlink("target", temp_dir.path().join("link")).expect("create symlink");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args([
            "vdir",
            "-ln",
            "--full-time",
            "--hyperlink=always",
            "-d",
            "link",
        ])
        .output()
        .expect("run syskits vdir");

    assert!(output.status.success());
    assert_eq!(output.stderr, b"");
    assert_eq!(
        output
            .stdout
            .windows(b"\x1b]8;;".len())
            .filter(|bytes| *bytes == b"\x1b]8;;")
            .count(),
        4
    );
}

#[test]
fn vdir_disables_dired_metadata_when_hyperlinks_are_enabled() {
    let temp_dir = TempDir::new().expect("tempdir");
    fs::write(temp_dir.path().join("f"), b"").expect("write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .args([
            "vdir",
            "-ln",
            "--full-time",
            "--dired",
            "--hyperlink=always",
            "-d",
            "f",
        ])
        .output()
        .expect("run syskits vdir");

    assert!(output.status.success());
    assert_eq!(output.stderr, b"");
    assert!(!output.stdout.windows(7).any(|bytes| bytes == b"//DIRED"));
}

#[test]
fn vdir_preserves_ls_colors_control_sequences_and_sgr_order() {
    let temp_dir = TempDir::new().expect("tempdir");
    fs::create_dir(temp_dir.path().join("d")).expect("create fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .env("LS_COLORS", "lc=LEFT:rc=RIGHT:ec=END:rs=9:di=34;01")
        .args(["vdir", "-1", "--color=always", "."])
        .output()
        .expect("run syskits vdir");

    assert!(output.status.success());
    assert_eq!(output.stderr, b"");
    assert_eq!(output.stdout, b"ENDLEFT34;01RIGHTdEND\nLEFTRIGHT");
}

#[cfg(unix)]
#[test]
fn vdir_colors_ln_target_using_symlink_target_type() {
    use std::os::unix::fs::symlink;

    let temp_dir = TempDir::new().expect("tempdir");
    fs::create_dir(temp_dir.path().join("target")).expect("create fixture");
    symlink("target", temp_dir.path().join("link")).expect("create symlink");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .env("LS_COLORS", "ln=target:di=31")
        .args(["vdir", "-1", "--color=always", "-d", "link"])
        .output()
        .expect("run syskits vdir");

    assert!(output.status.success());
    assert_eq!(output.stderr, b"");
    assert_eq!(output.stdout, b"\x1b[0m\x1b[31mlink\x1b[0m\n");
}

#[cfg(unix)]
#[test]
fn vdir_colors_dangling_symlink_without_diagnosing_target() {
    use std::os::unix::fs::symlink;

    let temp_dir = TempDir::new().expect("tempdir");
    symlink("nowhere", temp_dir.path().join("link")).expect("create symlink");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(temp_dir.path())
        .env("LC_ALL", "C")
        .env("LS_COLORS", "ln=32:or=33:mi=34")
        .args([
            "vdir",
            "-l",
            "--time-style=+T",
            "--color=always",
            "-d",
            "link",
        ])
        .output()
        .expect("run syskits vdir");

    assert!(output.status.success());
    assert_eq!(output.stderr, b"");
    assert!(
        output
            .stdout
            .ends_with(b"\x1b[0m\x1b[33mlink\x1b[0m -> \x1b[34mnowhere\x1b[0m\n")
    );
}

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

#[test]
fn vdir_long_omits_acl_context_marker_column_when_no_entry_needs_it() {
    let temp_dir = TempDir::new().expect("tempdir");
    let file = temp_dir.path().join("plain");
    fs::write(&file, b"").expect("write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["vdir", "-a", temp_dir.path().to_str().expect("utf8 path")])
        .output()
        .expect("run syskits vdir");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stderr, b"");

    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    let plain_line = stdout
        .lines()
        .find(|line| line.ends_with(" plain"))
        .expect("plain file line");

    assert!(
        plain_line.starts_with("-rw-r--r--  1 "),
        "plain file line should not reserve the ACL/context marker column: {plain_line:?}"
    );
    assert_eq!(plain_line.as_bytes().get(10), Some(&b' '));
    assert_eq!(plain_line.as_bytes().get(11), Some(&b' '));
    assert_eq!(plain_line.as_bytes().get(12), Some(&b'1'));
}

#[cfg(target_os = "linux")]
#[test]
fn vdir_long_keeps_acl_context_marker_column_when_any_entry_needs_it() {
    let temp_dir = TempDir::new().expect("tempdir");
    let with_context = temp_dir.path().join("with-context");
    let plain = temp_dir.path().join("plain");
    fs::write(&with_context, b"").expect("write fixture");
    fs::write(&plain, b"").expect("write fixture");

    if !set_selinux_xattr(&with_context, b"system_u:object_r:tmp_t:s0\0") {
        eprintln!("skipping: current filesystem does not allow setting security.selinux");
        return;
    }

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["vdir", "-a", temp_dir.path().to_str().expect("utf8 path")])
        .output()
        .expect("run syskits vdir");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stderr, b"");

    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    let context_line = stdout
        .lines()
        .find(|line| line.ends_with(" with-context"))
        .expect("context file line");
    let plain_line = stdout
        .lines()
        .find(|line| line.ends_with(" plain"))
        .expect("plain file line");

    assert_eq!(
        context_line.as_bytes().get(10),
        Some(&b'.'),
        "context file line should show a context marker: {context_line:?}"
    );
    assert_eq!(
        plain_line.as_bytes().get(10),
        Some(&b' '),
        "plain file line should reserve an empty ACL/context marker column: {plain_line:?}"
    );
    assert_eq!(
        plain_line.as_bytes().get(11),
        Some(&b' '),
        "plain file line should include the separator after the marker column: {plain_line:?}"
    );
}
