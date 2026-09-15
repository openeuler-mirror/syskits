/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2.
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2.
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

// nohup命令的作用是在Unix/Linux系统中允许一个命令在用户退出终端后继续在后台运行

extern crate rust_i18n;
use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::ct_error::{CTError, CTResult, CtSimpleError, UClapError, set_ct_exit_code};
use ctcore::ct_quoting_style::escape_shell_bytes_with_classifier;

use libc::{SIG_IGN, SIGHUP};
use libc::{c_char, dup2, execvp, signal};

use ctcore::Tool;
use std::borrow::Cow;
use std::env;
use std::ffi::{CStr, CString, OsStr, OsString};
use std::fmt::{Display, Formatter};
use std::fs::{File, OpenOptions};
use std::io::{self, Error, IsTerminal, Write, stderr};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use sys_locale::get_locale;

// 定义常量和模块，用于处理nohup命令的逻辑。
static NOHUP_OUT: &str = "nohup.out"; // 默认的nohup输出文件名

use crate::exit_codes::EXIT_CANCELED;
use crate::exit_codes::EXIT_CANNOT_INVOKE;
use crate::exit_codes::EXIT_ENOENT;
use crate::exit_codes::POSIX_NOHUP_FAILURE;
// 与GNU实现相匹配的退出码
mod exit_codes {
    pub static EXIT_CANCELED: i32 = 125;
    pub static EXIT_CANNOT_INVOKE: i32 = 126;
    pub static EXIT_ENOENT: i32 = 127;
    pub static POSIX_NOHUP_FAILURE: i32 = 127;
}

mod options {
    pub const CMD: &str = "cmd"; // 命令参数的标识符
}

// 定义NohupError枚举，处理可能出现的错误类型
#[derive(Debug)]
enum NohupError {
    CannotDetach,                            // 无法从控制台分离
    CannotRenderStdin(i32, Error),           // 无法使标准输入不可读
    CannotRedirectStderr(i32, Error),        // 无法重定向标准错误
    OpenFailed(i32, Error),                  // 打开文件失败
    OpenFailed2(i32, Error, PathBuf, Error), // 打开文件失败（备选路径）
}

enum ExecFailureStderr {
    NotRedirected,
    Saved(OwnedFd),
    Unavailable,
}

#[derive(Debug)]
struct NohupUsageError {
    code: i32,
    message: Vec<u8>,
    usage_hint: Vec<u8>,
}

impl NohupUsageError {
    fn boxed(code: i32, message: Vec<u8>) -> Box<dyn CTError> {
        let usage_hint = format!(
            "Try '{} --help' for more information.",
            ctcore::ct_help_utility_name()
        )
        .into_bytes();
        Box::new(Self {
            code,
            message,
            usage_hint,
        })
    }
}

impl std::error::Error for NohupUsageError {}

impl Display for NohupUsageError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        String::from_utf8_lossy(&self.message).fmt(formatter)
    }
}

impl CTError for NohupUsageError {
    fn code(&self) -> i32 {
        self.code
    }

    fn diagnostic_bytes(&self) -> Cow<'_, [u8]> {
        Cow::Borrowed(&self.message)
    }

    fn usage_hint_bytes(&self) -> Option<Cow<'_, [u8]>> {
        Some(Cow::Borrowed(&self.usage_hint))
    }

    fn usage(&self) -> bool {
        true
    }
}

impl std::error::Error for NohupError {}

impl CTError for NohupError {
    fn code(&self) -> i32 {
        match self {
            Self::CannotRenderStdin(code, _)
            | Self::CannotRedirectStderr(code, _)
            | Self::OpenFailed(code, _)
            | Self::OpenFailed2(code, _, _, _) => *code,
            _ => 2,
        }
    }

    fn diagnostic_bytes(&self) -> Cow<'_, [u8]> {
        match self {
            Self::OpenFailed2(_, first_error, fallback_path, second_error) => Cow::Owned(
                format!(
                    "{}\n{}: {}",
                    nohup_open_failure_message(Path::new(NOHUP_OUT), first_error),
                    ctcore::ct_util_name(),
                    nohup_open_failure_message(fallback_path, second_error)
                )
                .into_bytes(),
            ),
            _ => Cow::Owned(self.to_string().into_bytes()),
        }
    }
}

impl Display for NohupError {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        match self {
            Self::CannotDetach => write!(f, "Cannot detach from console"),
            Self::CannotRenderStdin(_, error) => write!(
                f,
                "failed to render standard input unusable: {}",
                gnu_errno_text(error)
            ),
            Self::CannotRedirectStderr(_, error) => write!(
                f,
                "failed to redirect standard error: {}",
                gnu_errno_text(error)
            ),
            Self::OpenFailed(_, error) => {
                f.write_str(&nohup_open_failure_message(Path::new(NOHUP_OUT), error))
            }
            Self::OpenFailed2(_, e1, path, e2) => write!(
                f,
                "{}\n{}",
                nohup_open_failure_message(Path::new(NOHUP_OUT), e1),
                nohup_open_failure_message(path, e2)
            ),
        }
    }
}

fn write_nohup_msg(msg: &str, stderr_was_closed: bool) -> io::Result<()> {
    // Rust may sanitize a closed stderr before nohup restores the original fd state.
    if stderr_was_closed {
        return Err(Error::from_raw_os_error(libc::EBADF));
    }

    let mut handle = stderr();
    writeln!(handle, "nohup: {msg}")?;
    handle.flush()
}

unsafe extern "C" {
    fn mbrtowc(
        wide: *mut libc::wchar_t,
        bytes: *const libc::c_char,
        length: usize,
        state: *mut libc::mbstate_t,
    ) -> usize;
    fn iswprint(wide: libc::c_uint) -> libc::c_int;
}

fn nohup_quote_path(path: &Path) -> String {
    let bytes = path.as_os_str().as_bytes();
    let mut quoted = escape_shell_bytes_with_classifier(bytes, |remaining| unsafe {
        let mut state: libc::mbstate_t = std::mem::zeroed();
        let mut wide = 0 as libc::wchar_t;
        let length = mbrtowc(
            &mut wide,
            remaining.as_ptr().cast(),
            remaining.len(),
            &mut state,
        );
        if length == usize::MAX {
            return (1, false);
        }
        if length == usize::MAX - 1 {
            return (remaining.len(), false);
        }

        let length = if length == 0 { 1 } else { length };
        let is_utf8 = std::str::from_utf8(&remaining[..length]).is_ok();
        (length, is_utf8 && iswprint(wide as libc::c_uint) != 0)
    });

    if quoted.as_slice() == bytes {
        quoted.insert(0, b'\'');
        quoted.push(b'\'');
    }

    String::from_utf8(quoted).expect("shell-escaped file names are valid UTF-8")
}

fn nohup_append_msg(path: &Path, ignoring_input: bool) -> String {
    if ignoring_input {
        format!(
            "ignoring input and appending output to {}",
            nohup_quote_path(path)
        )
    } else {
        format!("appending output to {}", nohup_quote_path(path))
    }
}

fn nohup_stderr_redirect_msg(ignoring_input: bool) -> &'static str {
    if ignoring_input {
        "ignoring input and redirecting stderr to stdout"
    } else {
        "redirecting stderr to stdout"
    }
}

fn should_open_nohup_output(
    stdout_is_tty: bool,
    stderr_is_tty: bool,
    stdout_was_closed: bool,
) -> bool {
    stdout_is_tty || (stderr_is_tty && stdout_was_closed)
}

fn nohup_internal_failure_code() -> i32 {
    nohup_failure_code(env::var_os("POSIXLY_CORRECT").as_deref())
}

fn nohup_failure_code(posixly_correct: Option<&OsStr>) -> i32 {
    if posixly_correct.is_some() {
        POSIX_NOHUP_FAILURE
    } else {
        EXIT_CANCELED
    }
}

fn gnu_errno_text(error: &Error) -> String {
    error.raw_os_error().map_or_else(
        || error.to_string(),
        |errno| {
            unsafe { CStr::from_ptr(libc::strerror(errno)) }
                .to_string_lossy()
                .into_owned()
        },
    )
}

fn nohup_open_failure_message(path: &Path, error: &Error) -> String {
    format!(
        "failed to open {}: {}",
        nohup_quote_path(path),
        gnu_errno_text(error)
    )
}

fn exec_failure_message(command: &OsStr, error: &Error) -> String {
    format!(
        "failed to run command {}: {}",
        nohup_quote_path(Path::new(command)),
        gnu_errno_text(error)
    )
}

fn open_nohup_out(path: &Path) -> io::Result<File> {
    // GNU nohup temporarily restricts the umask so newly created files are
    // always user-readable and user-writable, regardless of the caller's umask.
    let previous_umask = unsafe { libc::umask(!0o600) };
    let result = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path);
    unsafe { libc::umask(previous_umask) };
    result
}

fn nohup_home_output_path(home: &OsStr) -> PathBuf {
    let home_bytes = home.as_bytes();
    // GNU file_name_concat removes trailing slashes from non-root directories,
    // while retaining an all-slash root prefix exactly as supplied.
    let home_length = home_bytes
        .iter()
        .rposition(|byte| *byte != b'/')
        .map_or(home_bytes.len(), |index| index + 1);
    let mut path = Vec::with_capacity(home_length + 1 + NOHUP_OUT.len());
    path.extend_from_slice(&home_bytes[..home_length]);
    if home_length > 0 && path.last() != Some(&b'/') {
        path.push(b'/');
    }
    path.extend_from_slice(NOHUP_OUT.as_bytes());

    PathBuf::from(OsString::from_vec(path))
}

fn save_stderr_for_exec_failure() -> Option<OwnedFd> {
    let saved_fd = unsafe {
        libc::fcntl(
            libc::STDERR_FILENO,
            libc::F_DUPFD_CLOEXEC,
            libc::STDERR_FILENO + 1,
        )
    };
    (saved_fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(saved_fd) })
}

const NOHUP_LONG_OPTIONS: &[&str] = &["help", "version"];

fn nohup_match_long_option(option: &[u8]) -> Option<&'static str> {
    let mut matches = NOHUP_LONG_OPTIONS
        .iter()
        .copied()
        .filter(|candidate| !option.is_empty() && candidate.as_bytes().starts_with(option));
    let matching_option = matches.next()?;
    matches.next().is_none().then_some(matching_option)
}

fn nohup_validate_standard_options(args: &[OsString], error_code: i32) -> CTResult<()> {
    let Some(argument) = args.get(1) else {
        return Ok(());
    };
    let bytes = argument.as_bytes();
    if bytes == b"--" || bytes.len() <= 1 || bytes[0] != b'-' {
        return Ok(());
    }

    if bytes[1] != b'-' {
        let mut message = b"invalid option -- '".to_vec();
        message.push(bytes[1]);
        message.push(b'\'');
        return Err(NohupUsageError::boxed(error_code, message));
    }

    let option = &bytes[2..];
    let (name, has_argument) = option
        .iter()
        .position(|byte| *byte == b'=')
        .map_or((option, false), |equals| (&option[..equals], true));
    match nohup_match_long_option(name) {
        Some(canonical) if has_argument => Err(NohupUsageError::boxed(
            error_code,
            format!("option '--{canonical}' doesn't allow an argument").into_bytes(),
        )),
        Some(_) => Ok(()),
        None => {
            let mut message = b"unrecognized option '".to_vec();
            message.extend_from_slice(bytes);
            message.push(b'\'');
            Err(NohupUsageError::boxed(error_code, message))
        }
    }
}

fn nohup_command_args(matches: &ArgMatches, error_code: i32) -> CTResult<Vec<OsString>> {
    matches
        .get_many::<OsString>(options::CMD)
        .map(|arguments| arguments.cloned().collect())
        .ok_or_else(|| {
            CtSimpleError::new(
                error_code,
                format!(
                    "missing operand\nTry '{} --help' for more information.",
                    ctcore::ct_help_utility_name()
                ),
            )
        })
}

pub fn nohup_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let arg_error_code = nohup_internal_failure_code();

    let args = args.collect::<Vec<_>>();
    nohup_validate_standard_options(&args, arg_error_code)?;
    let args_match = ct_app()
        .try_get_matches_from(args)
        .with_exit_code(arg_error_code)?;
    let command_args = nohup_command_args(&args_match, arg_error_code)?;

    restore_initially_closed_standard_fds();
    let exec_failure_stderr = nohup_replace_fds()?;

    unsafe { signal(SIGHUP, SIG_IGN) }; // 忽略SIGHUP信号

    if unsafe { !_vprocmgr_detach_from_console(0).is_null() } {
        return Err(NohupError::CannotDetach.into());
    };

    let cstrings: Vec<CString> = command_args
        .iter()
        .map(|x| CString::new(x.as_bytes()).unwrap())
        .collect();
    let mut args: Vec<*const c_char> = cstrings.iter().map(|s| s.as_ptr()).collect();
    args.push(std::ptr::null());

    let result = unsafe { execvp(args[0], args.as_mut_ptr()) };
    if result == -1 {
        let err = std::io::Error::last_os_error();
        // 获取命令名用于错误信息
        let err_msg = exec_failure_message(command_args[0].as_os_str(), &err);
        let can_report_exec_failure = can_report_exec_failure(&exec_failure_stderr);
        // 尝试输出错误，如果 stderr 写入失败则退出 125
        if can_report_exec_failure
            && write_nohup_msg(&err_msg, ctcore::ct_stderr_was_closed()).is_err()
        {
            std::process::exit(125);
        }
        match err.raw_os_error() {
            Some(libc::ENOENT) => set_ct_exit_code(EXIT_ENOENT),
            _ => set_ct_exit_code(EXIT_CANNOT_INVOKE),
        }
    }
    Ok(())
}

fn restore_initially_closed_standard_fds() {
    for (fd, was_closed) in [
        (libc::STDIN_FILENO, ctcore::ct_stdin_was_closed()),
        (libc::STDOUT_FILENO, ctcore::ct_stdout_was_closed()),
        (libc::STDERR_FILENO, ctcore::ct_stderr_was_closed()),
    ] {
        close_fd_if_initially_closed(fd, was_closed);
    }
}

fn close_fd_if_initially_closed(fd: RawFd, was_closed: bool) {
    if was_closed {
        unsafe {
            libc::close(fd);
        }
    }
}

// 构建命令行解析器
pub fn ct_app() -> Command {
    Command::new(ctcore::ct_util_name())
        .version(crate_version!())
        .about(t!("nohup.about"))
        .after_help(t!("nohup.after_help"))
        .override_usage(t!("nohup.usage"))
        .disable_help_flag(true)
        .disable_version_flag(true)
        .arg(
            Arg::new("help")
                .long("help")
                .action(ArgAction::Help)
                .help("Print help"),
        )
        .arg(
            Arg::new("version")
                .long("version")
                .action(ArgAction::Version)
                .help("Print version"),
        )
        .arg(
            Arg::new(options::CMD)
                .hide(true)
                .action(ArgAction::Append)
                .value_parser(clap::builder::OsStringValueParser::new())
                .value_hint(clap::ValueHint::CommandName),
        )
        .trailing_var_arg(true)
        .infer_long_args(true)
}

// 替换标准输入、输出和错误输出文件描述符
fn nohup_replace_fds() -> CTResult<ExecFailureStderr> {
    let stdin_is_tty = std::io::stdin().is_terminal();
    let stdout_is_tty = std::io::stdout().is_terminal();
    let stderr_is_tty = std::io::stderr().is_terminal();
    let stdout_was_closed = ctcore::ct_stdout_was_closed();

    if stdin_is_tty {
        let new_stdin = OpenOptions::new()
            .write(true)
            .open(Path::new("/dev/null"))
            .map_err(|e| NohupError::CannotRenderStdin(nohup_internal_failure_code(), e))?;
        if unsafe { dup2(new_stdin.as_raw_fd(), 0) } != 0 {
            return Err(NohupError::CannotRenderStdin(
                nohup_internal_failure_code(),
                Error::last_os_error(),
            )
            .into());
        }

        if !stdout_is_tty
            && !stderr_is_tty
            && write_nohup_msg("ignoring input", ctcore::ct_stderr_was_closed()).is_err()
        {
            std::process::exit(nohup_internal_failure_code());
        }
    }

    let output_file = if should_open_nohup_output(stdout_is_tty, stderr_is_tty, stdout_was_closed) {
        nohup_find_stdout(stdin_is_tty, stdout_is_tty)?
    } else {
        None
    };

    if stderr_is_tty {
        let exec_failure_stderr = save_stderr_for_exec_failure()
            .map_or(ExecFailureStderr::Unavailable, ExecFailureStderr::Saved);
        if !stdout_is_tty
            && write_nohup_msg(
                nohup_stderr_redirect_msg(stdin_is_tty),
                ctcore::ct_stderr_was_closed(),
            )
            .is_err()
        {
            std::process::exit(nohup_internal_failure_code());
        }
        let stderr_target_fd = output_file.as_ref().map_or(1, AsRawFd::as_raw_fd);
        if unsafe { dup2(stderr_target_fd, 2) } != 2 {
            return Err(NohupError::CannotRedirectStderr(
                nohup_internal_failure_code(),
                Error::last_os_error(),
            )
            .into());
        }
        return Ok(exec_failure_stderr);
    }
    Ok(ExecFailureStderr::NotRedirected)
}

fn can_report_exec_failure(stderr: &ExecFailureStderr) -> bool {
    match stderr {
        ExecFailureStderr::NotRedirected => true,
        ExecFailureStderr::Unavailable => false,
        ExecFailureStderr::Saved(saved) => unsafe {
            dup2(saved.as_raw_fd(), libc::STDERR_FILENO) == libc::STDERR_FILENO
        },
    }
}

fn redirect_stdout(file: &File) -> io::Result<()> {
    redirect_stdout_from_fd(file.as_raw_fd())
}

fn redirect_stdout_from_fd(source_fd: RawFd) -> io::Result<()> {
    if unsafe { dup2(source_fd, libc::STDOUT_FILENO) } == libc::STDOUT_FILENO {
        Ok(())
    } else {
        Err(Error::last_os_error())
    }
}

fn open_nohup_output(path: &Path, redirecting_stdout: bool) -> io::Result<Option<File>> {
    let file = open_nohup_out(path)?;
    if redirecting_stdout {
        redirect_stdout(&file)?;
        Ok(None)
    } else {
        Ok(Some(file))
    }
}

// 查找或创建nohup输出文件
fn nohup_find_stdout(ignoring_input: bool, redirecting_stdout: bool) -> CTResult<Option<File>> {
    let internal_failure_code = nohup_internal_failure_code();

    match open_nohup_output(Path::new(NOHUP_OUT), redirecting_stdout) {
        Ok(file) => {
            let msg = nohup_append_msg(Path::new(NOHUP_OUT), ignoring_input);
            if write_nohup_msg(&msg, ctcore::ct_stderr_was_closed()).is_err() {
                std::process::exit(nohup_internal_failure_code());
            }
            Ok(file)
        }
        Err(err1) => {
            let home = match env::var_os("HOME") {
                None => return Err(NohupError::OpenFailed(internal_failure_code, err1).into()),
                Some(home) => home,
            };
            let path_buf = nohup_home_output_path(&home);
            match open_nohup_output(&path_buf, redirecting_stdout) {
                Ok(file) => {
                    let msg = nohup_append_msg(&path_buf, ignoring_input);
                    if write_nohup_msg(&msg, ctcore::ct_stderr_was_closed()).is_err() {
                        std::process::exit(nohup_internal_failure_code());
                    }
                    Ok(file)
                }
                Err(err2) => {
                    Err(NohupError::OpenFailed2(internal_failure_code, err1, path_buf, err2).into())
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
unsafe fn _vprocmgr_detach_from_console(_: u32) -> *const libc::c_int {
    std::ptr::null()
}

#[derive(Default)]
pub struct Nohup;
impl Tool for Nohup {
    fn name(&self) -> &'static str {
        "nohup"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        nohup_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    mod tests_messages {
        use crate::{
            ExecFailureStderr, NohupError, can_report_exec_failure, close_fd_if_initially_closed,
            exec_failure_message, nohup_append_msg, nohup_command_args, nohup_home_output_path,
            nohup_stderr_redirect_msg, nohup_validate_standard_options, redirect_stdout_from_fd,
            save_stderr_for_exec_failure, should_open_nohup_output, write_nohup_msg,
        };
        use ctcore::ct_error::CTError;
        use std::ffi::{OsStr, OsString};
        use std::io::Error;
        use std::os::fd::{AsRawFd, IntoRawFd};
        use std::os::unix::ffi::OsStrExt;
        use std::path::Path;
        use std::path::PathBuf;

        #[test]
        fn test_nohup_append_msg_uses_actual_path() {
            assert_eq!(
                nohup_append_msg(Path::new("/tmp/home/nohup.out"), true),
                "ignoring input and appending output to '/tmp/home/nohup.out'"
            );
            assert_eq!(
                nohup_append_msg(Path::new("nohup.out"), false),
                "appending output to 'nohup.out'"
            );
            assert_eq!(
                nohup_append_msg(Path::new(OsStr::from_bytes(b"home-\xff/nohup.out")), true),
                "ignoring input and appending output to 'home-'$'\\377''/nohup.out'"
            );
        }

        #[test]
        fn test_nohup_home_output_path_matches_gnu_file_name_concat() {
            assert_eq!(
                nohup_home_output_path(OsStr::from_bytes(b"home//")),
                Path::new("home/nohup.out")
            );
            assert_eq!(
                nohup_home_output_path(OsStr::from_bytes(b"logs//archive///")),
                Path::new("logs//archive/nohup.out")
            );
            assert_eq!(
                nohup_home_output_path(OsStr::from_bytes(b"///")),
                Path::new("///nohup.out")
            );
        }

        #[test]
        fn test_nohup_stderr_redirect_message_tracks_input() {
            assert_eq!(
                nohup_stderr_redirect_msg(false),
                "redirecting stderr to stdout"
            );
            assert_eq!(
                nohup_stderr_redirect_msg(true),
                "ignoring input and redirecting stderr to stdout"
            );
        }

        #[test]
        fn test_nohup_diagnostic_fails_when_stderr_started_closed() {
            let error = write_nohup_msg("ignored", true).unwrap_err();

            assert_eq!(error.raw_os_error(), Some(libc::EBADF));
        }

        #[test]
        fn test_nohup_output_is_opened_for_tty_stderr_when_stdout_started_closed() {
            assert!(should_open_nohup_output(true, false, false));
            assert!(should_open_nohup_output(false, true, true));
            assert!(!should_open_nohup_output(false, true, false));
            assert!(!should_open_nohup_output(false, false, true));
        }

        #[test]
        fn test_nohup_requires_command_before_fd_replacement() {
            let matches = crate::ct_app()
                .try_get_matches_from([OsString::from("nohup")])
                .unwrap();
            let error = nohup_command_args(&matches, crate::EXIT_CANCELED).unwrap_err();

            assert_eq!(error.code(), crate::EXIT_CANCELED);
            assert_eq!(
                error.to_string(),
                format!(
                    "missing operand\nTry '{} --help' for more information.",
                    ctcore::ct_help_utility_name()
                )
            );
        }

        #[test]
        fn test_exec_failure_message_uses_gnu_wording_without_rust_error_suffix() {
            assert_eq!(
                exec_failure_message(
                    OsStr::new("no-such-command"),
                    &Error::from_raw_os_error(libc::ENOENT)
                ),
                "failed to run command 'no-such-command': No such file or directory"
            );
        }

        #[test]
        fn test_exec_failure_message_preserves_non_utf8_command_bytes() {
            let command = OsStr::from_bytes(b"./missing-\xff");

            assert_eq!(
                exec_failure_message(command, &Error::from_raw_os_error(libc::ENOENT)),
                "failed to run command './missing-'$'\\377': No such file or directory"
            );
        }

        #[test]
        fn test_nohup_output_open_error_uses_libc_errno_text() {
            assert_eq!(
                NohupError::OpenFailed(
                    crate::EXIT_CANCELED,
                    Error::from_raw_os_error(libc::EISDIR)
                )
                .to_string(),
                "failed to open 'nohup.out': Is a directory"
            );
        }

        #[test]
        fn test_nohup_stdin_replacement_error_matches_gnu() {
            let error = NohupError::CannotRenderStdin(
                crate::EXIT_CANCELED,
                Error::from_raw_os_error(libc::EACCES),
            );

            assert_eq!(error.code(), crate::EXIT_CANCELED);
            assert_eq!(
                error.to_string(),
                "failed to render standard input unusable: Permission denied"
            );
        }

        #[test]
        fn test_nohup_stdout_redirection_error_uses_open_failure_semantics() {
            let error =
                NohupError::OpenFailed(crate::EXIT_CANCELED, Error::from_raw_os_error(libc::EBADF));

            assert_eq!(error.code(), crate::EXIT_CANCELED);
            assert_eq!(
                error.to_string(),
                "failed to open 'nohup.out': Bad file descriptor"
            );
        }

        #[test]
        fn test_nohup_stderr_redirection_error_matches_gnu() {
            let error = NohupError::CannotRedirectStderr(
                crate::EXIT_CANCELED,
                Error::from_raw_os_error(libc::EBADF),
            );

            assert_eq!(error.code(), crate::EXIT_CANCELED);
            assert_eq!(
                error.to_string(),
                "failed to redirect standard error: Bad file descriptor"
            );
        }

        #[test]
        fn test_redirect_stdout_rejects_invalid_file_descriptor() {
            let error = redirect_stdout_from_fd(-1).unwrap_err();

            assert_eq!(error.raw_os_error(), Some(libc::EBADF));
        }

        #[test]
        fn test_nohup_fallback_open_error_prefixes_each_diagnostic() {
            let error = NohupError::OpenFailed2(
                crate::EXIT_CANCELED,
                Error::from_raw_os_error(libc::EISDIR),
                PathBuf::from("missing-home/nohup.out"),
                Error::from_raw_os_error(libc::ENOENT),
            );

            assert_eq!(
                error.diagnostic_bytes().as_ref(),
                format!(
                    "failed to open 'nohup.out': Is a directory\n{}: failed to open 'missing-home/nohup.out': No such file or directory",
                    ctcore::ct_util_name()
                )
                .as_bytes()
            );
        }

        #[test]
        fn test_saved_stderr_for_exec_failure_is_close_on_exec_duplicate() {
            let saved = save_stderr_for_exec_failure().expect("stderr can be duplicated");

            assert_ne!(saved.as_raw_fd(), libc::STDERR_FILENO);
            let flags = unsafe { libc::fcntl(saved.as_raw_fd(), libc::F_GETFD) };
            assert_ne!(flags & libc::FD_CLOEXEC, 0);
        }

        #[test]
        fn test_exec_failure_reporting_requires_a_saved_tty_stderr() {
            assert!(can_report_exec_failure(&ExecFailureStderr::NotRedirected));
            assert!(!can_report_exec_failure(&ExecFailureStderr::Unavailable));
        }

        #[test]
        fn test_close_fd_if_initially_closed_only_closes_marked_descriptor() {
            let retained = std::fs::File::open("/dev/null").unwrap().into_raw_fd();
            close_fd_if_initially_closed(retained, false);
            assert_ne!(unsafe { libc::fcntl(retained, libc::F_GETFD) }, -1);
            unsafe {
                libc::close(retained);
            }

            let closed = std::fs::File::open("/dev/null").unwrap().into_raw_fd();
            close_fd_if_initially_closed(closed, true);
            assert_eq!(unsafe { libc::fcntl(closed, libc::F_GETFD) }, -1);
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::EBADF)
            );
        }

        #[test]
        fn test_nohup_standard_option_validation_matches_gnu() {
            let invalid_short = nohup_validate_standard_options(
                &["nohup".into(), "-h".into()],
                crate::EXIT_CANCELED,
            )
            .unwrap_err();
            assert_eq!(
                invalid_short.diagnostic_bytes().as_ref(),
                b"invalid option -- 'h'"
            );
            assert_eq!(
                invalid_short.usage_hint_bytes().unwrap().as_ref(),
                format!(
                    "Try '{} --help' for more information.",
                    ctcore::ct_help_utility_name()
                )
                .as_bytes()
            );

            let help_with_value = nohup_validate_standard_options(
                &["nohup".into(), "--help=value".into()],
                crate::EXIT_CANCELED,
            )
            .unwrap_err();
            assert_eq!(
                help_with_value.diagnostic_bytes().as_ref(),
                b"option '--help' doesn't allow an argument"
            );

            assert!(
                nohup_validate_standard_options(
                    &["nohup".into(), "--ver".into(), "ignored".into()],
                    crate::EXIT_CANCELED,
                )
                .is_ok()
            );
            assert!(
                nohup_validate_standard_options(
                    &["nohup".into(), "echo".into(), "-h".into()],
                    crate::EXIT_CANCELED,
                )
                .is_ok()
            );
        }
    }

    mod tests_tool_implementation {
        use crate::Nohup;
        use ctcore::Tool;
        use std::ffi::OsString;

        #[test]
        fn test_tool_implementation() {
            let tool = Nohup;

            // 测试 name 方法
            assert_eq!(tool.name(), "nohup");

            // 测试 command 方法
            let command = tool.command();
            assert!(command.get_name().contains("nohup"));

            // 测试 execute 方法
            let args = vec![OsString::from("nohup"), OsString::from("--help")];
            assert!(tool.execute(&args).is_err()); // --help参数通常会返回错误
        }
    }

    mod tests_echo_main {
        use crate::{EXIT_CANCELED, nohup_failure_code, nohup_main};

        use std::ffi::{OsStr, OsString};
        use std::os::unix::ffi::OsStrExt;

        #[test]
        fn test_false_main_version() {
            let args = [ctcore::ct_util_name(), "--version"];

            let result = nohup_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_false_main_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = nohup_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_nohup_main_reports_gnu_missing_operand() {
            let args = [ctcore::ct_util_name()];
            let error = nohup_main(args.iter().map(OsString::from)).unwrap_err();

            assert_eq!(error.code(), EXIT_CANCELED);
            assert_eq!(
                error.to_string(),
                format!(
                    "missing operand\nTry '{} --help' for more information.",
                    ctcore::ct_help_utility_name()
                )
            );
        }

        #[test]
        fn test_nohup_non_utf8_posixly_correct_uses_posix_failure_status() {
            assert_eq!(
                nohup_failure_code(Some(OsStr::from_bytes(b"\xff"))),
                crate::EXIT_ENOENT
            );
        }
    }

    mod tests_false_app {
        use crate::{ct_app, options};

        use clap::error::ErrorKind;
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        #[test]
        fn test_ct_app_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_accepts_non_utf8_command_path() {
            let command = OsString::from_vec(b"command-\xff".to_vec());
            let matches = ct_app()
                .try_get_matches_from([OsString::from("nohup"), command.clone()])
                .unwrap();

            assert_eq!(matches.get_one::<OsString>(options::CMD), Some(&command));
        }
    }
}
