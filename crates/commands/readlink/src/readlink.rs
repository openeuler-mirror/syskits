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

//! readlink命令是Linux中用于读取符号链接（symlink）并显示其指向的文件或目录的命令。

extern crate rust_i18n;
use clap::{Arg, ArgAction, ArgMatches, Command, builder::OsStringValueParser, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::Tool;
use ctcore::ct_error::{CTResult, CTsageError, CtSimpleError, FromIo};
use ctcore::ct_fs::{MissingHandling, ResolveMode, canonicalize};
use ctcore::ct_line_ending::CtLineEnding;
use ctcore::ct_posix::GnuGetoptCommandExt;
use ctcore::ct_quoting_style::gnu_quote_shell;
use ctcore::ct_show_error;
use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::fmt::{Display, Formatter};
use std::fs;
use std::io::{Write, stdout};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicUsize, Ordering};
use sys_locale::get_locale;

#[cfg(target_os = "linux")]
static INHERITED_SIGPIPE_HANDLER: AtomicUsize = AtomicUsize::new(ctcore::libc::SIG_ERR);

#[cfg(target_os = "linux")]
#[used]
#[unsafe(link_section = ".init_array")]
static CAPTURE_INHERITED_SIGPIPE: unsafe extern "C" fn() = capture_inherited_sigpipe;

#[cfg(target_os = "linux")]
unsafe extern "C" fn capture_inherited_sigpipe() {
    let mut action = std::mem::MaybeUninit::<ctcore::libc::sigaction>::uninit();
    if unsafe {
        ctcore::libc::sigaction(ctcore::libc::SIGPIPE, std::ptr::null(), action.as_mut_ptr())
    } == 0
    {
        let action = unsafe { action.assume_init() };
        INHERITED_SIGPIPE_HANDLER.store(action.sa_sigaction, Ordering::Relaxed);
    }
}

mod readlink_flags {
    pub const READLINK_CANONICALIZE: &str = "canonicalize";
    pub const READLINK_CANONICALIZE_MISSING: &str = "canonicalize-missing";
    pub const READLINK_CANONICALIZE_EXISTING: &str = "canonicalize-existing";
    pub const READLINK_NO_NEWLINE: &str = "no-newline";
    pub const READLINK_QUIET: &str = "quiet";
    pub const READLINK_SILENT: &str = "silent";
    pub const READLINK_VERBOSE: &str = "verbose";
    pub const READLINK_ZERO: &str = "zero";

    pub const READLINK_ARG_FILES: &str = "files";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadlinkMode {
    Readlink,
    Canonicalize,
    CanonicalizeExisting,
    CanonicalizeMissing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadlinkSemanticRow {
    pub input: String,
    pub resolved_path: String,
    pub mode: ReadlinkMode,
    pub no_newline: bool,
    pub zero: bool,
    pub quiet: bool,
    pub verbose: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadlinkSemantic {
    pub rows: Vec<ReadlinkSemanticRow>,
    pub classic_text: String,
}

struct ReadlinkOptions {
    files: Vec<OsString>,
    mode: ReadlinkMode,
    resolve_mode: ResolveMode,
    missing_handling: MissingHandling,
    quiet: bool,
    verbose: bool,
    line_ending: Option<CtLineEnding>,
    no_newline: bool,
    zero: bool,
}

#[derive(Debug)]
struct ReadlinkUsageError {
    message: Vec<u8>,
    usage_hint: Vec<u8>,
}

impl ReadlinkUsageError {
    fn boxed(message: Vec<u8>) -> Box<dyn ctcore::ct_error::CTError> {
        let usage_hint = format!(
            "Try '{} --help' for more information.",
            ctcore::ct_help_utility_name()
        )
        .into_bytes();
        Box::new(Self {
            message,
            usage_hint,
        })
    }
}

impl std::error::Error for ReadlinkUsageError {}

impl Display for ReadlinkUsageError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        String::from_utf8_lossy(&self.message).fmt(formatter)
    }
}

impl ctcore::ct_error::CTError for ReadlinkUsageError {
    fn code(&self) -> i32 {
        1
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

impl ReadlinkOptions {
    fn from_matches(arg_matches: &ArgMatches) -> CTResult<Self> {
        Self::from_matches_with_posix(arg_matches, ctcore::ct_posix::posixly_correct())
    }

    fn from_matches_with_posix(arg_matches: &ArgMatches, _posixly_correct: bool) -> CTResult<Self> {
        let mut is_no_trailing_delimiter =
            arg_matches.get_flag(readlink_flags::READLINK_NO_NEWLINE);
        let is_use_zero = arg_matches.get_flag(readlink_flags::READLINK_ZERO);
        let is_silent = arg_matches.get_count(readlink_flags::READLINK_SILENT) > 0
            || arg_matches.get_count(readlink_flags::READLINK_QUIET) > 0;

        let is_verbose = [
            (readlink_flags::READLINK_QUIET, false),
            (readlink_flags::READLINK_SILENT, false),
            (readlink_flags::READLINK_VERBOSE, true),
        ]
        .into_iter()
        .filter_map(|(name, verbose)| {
            (arg_matches.get_count(name) > 0)
                .then(|| {
                    arg_matches
                        .indices_of(name)
                        .and_then(|mut indices| indices.next_back())
                })
                .flatten()
                .map(|index| (index, verbose))
        })
        .max_by_key(|(index, _)| *index)
        .is_some_and(|(_, verbose)| verbose);

        let mode = [
            (
                readlink_flags::READLINK_CANONICALIZE,
                ReadlinkMode::Canonicalize,
            ),
            (
                readlink_flags::READLINK_CANONICALIZE_EXISTING,
                ReadlinkMode::CanonicalizeExisting,
            ),
            (
                readlink_flags::READLINK_CANONICALIZE_MISSING,
                ReadlinkMode::CanonicalizeMissing,
            ),
        ]
        .into_iter()
        .filter_map(|(name, mode)| {
            (arg_matches.get_count(name) > 0)
                .then(|| {
                    arg_matches
                        .indices_of(name)
                        .and_then(|mut indices| indices.next_back())
                })
                .flatten()
                .map(|index| (index, mode))
        })
        .max_by_key(|(index, _)| *index)
        .map_or(ReadlinkMode::Readlink, |(_, mode)| mode);

        let resolve_mode = match mode {
            ReadlinkMode::Readlink => ResolveMode::None,
            _ => ResolveMode::Physical,
        };

        let missing_handling = match mode {
            ReadlinkMode::CanonicalizeExisting => MissingHandling::Existing,
            ReadlinkMode::CanonicalizeMissing => MissingHandling::Missing,
            _ => MissingHandling::Normal,
        };

        let files: Vec<OsString> = arg_matches
            .get_many::<OsString>(readlink_flags::READLINK_ARG_FILES)
            .map(|value| value.cloned().collect())
            .unwrap_or_default();
        if files.is_empty() {
            return Err(CTsageError::new(1, "missing operand"));
        }

        if is_no_trailing_delimiter && files.len() > 1 {
            ct_show_error!("ignoring --no-newline with multiple arguments");
            is_no_trailing_delimiter = false;
        }

        let line_ending = match is_no_trailing_delimiter {
            true => None,
            false => Some(CtLineEnding::from_zero_flag(is_use_zero)),
        };

        Ok(Self {
            files,
            mode,
            resolve_mode,
            missing_handling,
            quiet: is_silent,
            verbose: is_verbose,
            line_ending,
            no_newline: is_no_trailing_delimiter,
            zero: is_use_zero,
        })
    }
}

#[derive(Default)]
pub struct Readlink;
impl Tool for Readlink {
    fn name(&self) -> &'static str {
        "readlink"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        readlink_main(args.iter().cloned())
    }
}

pub fn readlink_main(args: impl ctcore::Args) -> CTResult<()> {
    let _sigpipe_guard = SigpipeGuard::for_cli();
    let stdout = stdout();
    let mut writer = stdout.lock();
    readlink_main_with_writer(args, &mut writer)
}

#[cfg(target_os = "linux")]
struct SigpipeGuard {
    previous: ctcore::libc::sighandler_t,
}

#[cfg(target_os = "linux")]
impl SigpipeGuard {
    fn for_cli() -> Option<Self> {
        // Rust ignores SIGPIPE before main. GNU readlink keeps the caller's
        // default disposition, except when the caller explicitly ignored it.
        if inherited_sigpipe_is_ignored() {
            return None;
        }

        let previous =
            unsafe { ctcore::libc::signal(ctcore::libc::SIGPIPE, ctcore::libc::SIG_DFL) };
        (previous != ctcore::libc::SIG_ERR).then_some(Self { previous })
    }
}

#[cfg(not(target_os = "linux"))]
struct SigpipeGuard;

#[cfg(not(target_os = "linux"))]
impl SigpipeGuard {
    fn for_cli() -> Option<Self> {
        None
    }
}

#[cfg(target_os = "linux")]
impl Drop for SigpipeGuard {
    fn drop(&mut self) {
        unsafe {
            ctcore::libc::signal(ctcore::libc::SIGPIPE, self.previous);
        }
    }
}

#[cfg(target_os = "linux")]
fn inherited_sigpipe_is_ignored() -> bool {
    sigpipe_handler_is_ignored(INHERITED_SIGPIPE_HANDLER.load(Ordering::Relaxed))
}

#[cfg(target_os = "linux")]
fn sigpipe_handler_is_ignored(handler: usize) -> bool {
    handler == ctcore::libc::SIG_IGN
}

fn readlink_main_with_writer(args: impl ctcore::Args, writer: &mut dyn Write) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let arg_matches = parse_readlink_args(args)?;
    let options = ReadlinkOptions::from_matches(&arg_matches)?;
    let mut failed = false;

    for input in &options.files {
        let path_buf = PathBuf::from(input);
        let path_result = match options.resolve_mode {
            ResolveMode::None => fs::read_link(&path_buf),
            _ => readlink_canonicalize(&path_buf, options.missing_handling, options.resolve_mode),
        };

        match path_result {
            Ok(path) => {
                readlink_show_with_writer(&path, options.line_ending, writer)
                    .map_err_context(|| "write error".to_owned())?;
            }
            Err(err) => {
                failed = true;
                if options.verbose {
                    let error = err.map_err_context(|| readlink_quote_path(input));
                    ctcore::ct_error::write_error_diagnostic(error.as_ref())
                        .map_err_context(String::new)?;
                }
            }
        }
    }

    if failed { Err(1.into()) } else { Ok(()) }
}

pub fn readlink_native_semantic(args: impl ctcore::Args) -> CTResult<ReadlinkSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let arg_matches = parse_readlink_args(args)?;
    let options = ReadlinkOptions::from_matches(&arg_matches)?;

    let mut rows = Vec::with_capacity(options.files.len());
    let mut classic_text = String::new();

    for input in &options.files {
        let path_buf = PathBuf::from(input);
        let path_result = match options.resolve_mode {
            ResolveMode::None => fs::read_link(&path_buf),
            _ => readlink_canonicalize(&path_buf, options.missing_handling, options.resolve_mode),
        };

        match path_result {
            Ok(path) => {
                let resolved_path = path.to_string_lossy().to_string();
                classic_text.push_str(&resolved_path);
                if let Some(line_ending) = options.line_ending {
                    classic_text.push_str(&line_ending.to_string());
                }
                rows.push(ReadlinkSemanticRow {
                    input: input.to_string_lossy().into_owned(),
                    resolved_path,
                    mode: options.mode,
                    no_newline: options.no_newline,
                    zero: options.zero,
                    quiet: options.quiet,
                    verbose: options.verbose,
                });
            }
            Err(err) => {
                if options.verbose {
                    return Err(CtSimpleError::new(
                        1,
                        err.map_err_context(move || readlink_quote_path(input))
                            .to_string(),
                    ));
                }
                return Err(1.into());
            }
        }
    }

    Ok(ReadlinkSemantic { rows, classic_text })
}

fn readlink_canonicalize(
    path: &Path,
    missing_handling: MissingHandling,
    resolve_mode: ResolveMode,
) -> std::io::Result<PathBuf> {
    // GNU canonicalize_filename_mode rejects an empty operand instead of
    // resolving it relative to the current working directory.
    if path.as_os_str().is_empty() {
        return Err(std::io::Error::from_raw_os_error(ctcore::libc::ENOENT));
    }

    canonicalize(path, missing_handling, resolve_mode)
}

fn parse_readlink_args(args: impl ctcore::Args) -> CTResult<ArgMatches> {
    let args = normalize_gnu_options(args.collect())?;
    validate_readlink_short_options(&args)?;
    Ok(ct_app().try_get_matches_from(args)?)
}

fn normalize_gnu_options(args: Vec<OsString>) -> CTResult<Vec<OsString>> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        const LONG_OPTIONS: &[&[u8]] = &[
            b"canonicalize",
            b"canonicalize-existing",
            b"canonicalize-missing",
            b"no-newline",
            b"quiet",
            b"silent",
            b"verbose",
            b"zero",
            b"help",
            b"version",
        ];

        let mut normalized = Vec::with_capacity(args.len());
        let posixly_correct = ctcore::ct_posix::posixly_correct();
        let mut index = 0;

        while index < args.len() {
            let argument = &args[index];
            let bytes = argument.as_os_str().as_bytes();

            if index == 0 {
                normalized.push(argument.clone());
                index += 1;
                continue;
            }

            if bytes == b"--" {
                normalized.extend(args[index..].iter().cloned());
                break;
            }

            if bytes == b"-" || !bytes.starts_with(b"-") {
                if posixly_correct {
                    normalized.extend(args[index..].iter().cloned());
                    break;
                }
                normalized.push(argument.clone());
                index += 1;
                continue;
            }

            let Some(long_option) = bytes.strip_prefix(b"--") else {
                normalized.push(argument.clone());
                index += 1;
                continue;
            };

            let (name, has_argument) = long_option
                .iter()
                .position(|byte| *byte == b'=')
                .map_or((long_option, false), |equals| {
                    (&long_option[..equals], true)
                });
            let exact_match = LONG_OPTIONS
                .iter()
                .copied()
                .find(|candidate| *candidate == name);
            let candidates = LONG_OPTIONS
                .iter()
                .copied()
                .filter(|candidate| candidate.starts_with(name))
                .collect::<Vec<_>>();
            let canonical = match exact_match {
                Some(option) => option,
                None if candidates.is_empty() => {
                    let mut message = b"unrecognized option '".to_vec();
                    message.extend_from_slice(bytes);
                    message.push(b'\'');
                    return Err(ReadlinkUsageError::boxed(message));
                }
                None if candidates.len() == 1 => candidates[0],
                None => {
                    let mut message = b"option '--".to_vec();
                    message.extend_from_slice(long_option);
                    message.extend_from_slice(b"' is ambiguous; possibilities:");
                    for candidate in candidates {
                        message.extend_from_slice(b" '--");
                        message.extend_from_slice(candidate);
                        message.push(b'\'');
                    }
                    return Err(ReadlinkUsageError::boxed(message));
                }
            };

            if has_argument {
                return Err(ReadlinkUsageError::boxed(
                    format!(
                        "option '--{}' doesn't allow an argument",
                        String::from_utf8_lossy(canonical)
                    )
                    .into_bytes(),
                ));
            }

            let mut rewritten = b"--".to_vec();
            rewritten.extend_from_slice(canonical);
            normalized.push(OsString::from_vec(rewritten));
            index += 1;
        }

        Ok(normalized)
    }

    #[cfg(not(unix))]
    Ok(args)
}

fn validate_readlink_short_options(args: &[OsString]) -> CTResult<()> {
    #[cfg(unix)]
    {
        let mut options_allowed = true;
        let posixly_correct = ctcore::ct_posix::posixly_correct();

        for arg in args.iter().skip(1) {
            let bytes = arg.as_os_str().as_bytes();
            if !options_allowed {
                continue;
            }
            if bytes == b"--" {
                options_allowed = false;
                continue;
            }
            if bytes.starts_with(b"--") {
                continue;
            }
            if bytes.starts_with(b"-") && bytes.len() > 1 {
                for option in &bytes[1..] {
                    if !matches!(
                        option,
                        b'e' | b'f' | b'm' | b'n' | b'q' | b's' | b'v' | b'z'
                    ) {
                        let mut message = b"invalid option -- '".to_vec();
                        message.push(*option);
                        message.push(b'\'');
                        return Err(ReadlinkUsageError::boxed(message));
                    }
                }
            } else if posixly_correct {
                options_allowed = false;
            }
        }
    }

    Ok(())
}

pub fn ct_app() -> Command {
    ct_app_with_getopt_mode(ctcore::ct_posix::posixly_correct())
}

fn ct_app_with_getopt_mode(posixly_correct: bool) -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("readlink.about");
    let usage_description = t!("readlink.usage");
    let args = vec![
        Arg::new(readlink_flags::READLINK_CANONICALIZE)
            .short('f')
            .long(readlink_flags::READLINK_CANONICALIZE)
            .help(
                "canonicalize by following every symlink in every component of the \
                     given name recursively; all but the last component must exist",
            )
            .action(ArgAction::Count),
        Arg::new(readlink_flags::READLINK_CANONICALIZE_EXISTING)
            .short('e')
            .long("canonicalize-existing")
            .help(
                "canonicalize by following every symlink in every component of the \
                     given name recursively, all components must exist",
            )
            .action(ArgAction::Count),
        Arg::new(readlink_flags::READLINK_CANONICALIZE_MISSING)
            .short('m')
            .long(readlink_flags::READLINK_CANONICALIZE_MISSING)
            .help(
                "canonicalize by following every symlink in every component of the \
                     given name recursively, without requirements on components existence",
            )
            .action(ArgAction::Count),
        Arg::new(readlink_flags::READLINK_NO_NEWLINE)
            .short('n')
            .long(readlink_flags::READLINK_NO_NEWLINE)
            .help(t!("readlink.clap.readlink_no_newline"))
            .action(ArgAction::SetTrue),
        Arg::new(readlink_flags::READLINK_QUIET)
            .short('q')
            .long(readlink_flags::READLINK_QUIET)
            .help(t!("readlink.clap.readlink_quiet"))
            .action(ArgAction::Count),
        Arg::new(readlink_flags::READLINK_SILENT)
            .short('s')
            .long(readlink_flags::READLINK_SILENT)
            .help(t!("readlink.clap.readlink_silent"))
            .action(ArgAction::Count),
        Arg::new(readlink_flags::READLINK_VERBOSE)
            .short('v')
            .long(readlink_flags::READLINK_VERBOSE)
            .help(t!("readlink.clap.readlink_verbose"))
            .action(ArgAction::Count),
        Arg::new(readlink_flags::READLINK_ZERO)
            .short('z')
            .long(readlink_flags::READLINK_ZERO)
            .help(t!("readlink.clap.readlink_zero"))
            .action(ArgAction::SetTrue),
        Arg::new(readlink_flags::READLINK_ARG_FILES)
            .action(ArgAction::Append)
            .value_parser(OsStringValueParser::new())
            .value_hint(clap::ValueHint::AnyPath),
        Arg::new("help").long("help").action(ArgAction::Help),
        Arg::new("version")
            .long("version")
            .action(ArgAction::Version),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .args(args)
        .gnu_getopt_with_mode(posixly_correct)
}

fn readlink_show_with_writer(
    path: &Path,
    line_ending: Option<CtLineEnding>,
    writer: &mut dyn Write,
) -> std::io::Result<()> {
    #[cfg(unix)]
    writer.write_all(path.as_os_str().as_bytes())?;
    #[cfg(not(unix))]
    write!(writer, "{}", path.display())?;

    if let Some(line_ending) = line_ending {
        write!(writer, "{line_ending}")?;
    }
    writer.flush()
}

fn readlink_quote_path(path: &OsStr) -> String {
    gnu_quote_shell(path, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctcore::Tool;
    use std::ffi::OsString;

    #[cfg(target_os = "linux")]
    #[test]
    fn inherited_sigpipe_handler_distinguishes_ignore_from_default() {
        assert!(sigpipe_handler_is_ignored(ctcore::libc::SIG_IGN));
        assert!(!sigpipe_handler_is_ignored(ctcore::libc::SIG_DFL));
    }

    #[cfg(unix)]
    #[test]
    fn native_semantic_quotes_non_utf8_path_errors_like_gnu() {
        use std::os::unix::ffi::OsStringExt;

        let missing = OsString::from_vec(b"missing-\xff".to_vec());
        let error = readlink_native_semantic(
            [
                OsString::from(ctcore::ct_util_name()),
                OsString::from("-v"),
                missing,
            ]
            .into_iter(),
        )
        .expect_err("a missing path must fail in verbose mode");

        assert_eq!(
            error.diagnostic_bytes().as_ref(),
            b"'missing-'$'\\377': No such file or directory"
        );
    }

    #[test]
    fn test_tool_implementation() {
        let tool = Readlink;

        // 测试 name 方法
        assert_eq!(tool.name(), "readlink");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("readlink"));

        // 测试 execute 方法
        let args = vec![OsString::from("readlink"), OsString::from("--version")];
        assert!(tool.execute(&args).is_err());
    }

    #[cfg(test)]
    mod show_tests {
        use super::*;

        fn test_show_output(path: &str, line_ending: Option<CtLineEnding>, expected_output: &str) {
            let path = Path::new(path);
            let mut output = Vec::new();
            show_with_writer(path, line_ending, &mut output).unwrap();
            assert_eq!(String::from_utf8(output).unwrap(), expected_output);
        }

        #[test]
        fn test_show_with_newline() {
            test_show_output("test/path", Some(CtLineEnding::Newline), "test/path\n");
        }

        #[test]
        fn test_show_with_null() {
            test_show_output("test/path", Some(CtLineEnding::Nul), "test/path\0");
        }

        #[test]
        fn test_show_without_line_ending() {
            test_show_output("test/path", None, "test/path");
        }

        #[test]
        fn test_show_empty_path() {
            test_show_output("", Some(CtLineEnding::Newline), "\n");
        }

        #[test]
        fn test_show_path_with_spaces() {
            test_show_output(
                "test path with spaces",
                Some(CtLineEnding::Newline),
                "test path with spaces\n",
            );
        }

        #[test]
        fn test_show_path_with_unicode() {
            test_show_output("测试/路径", Some(CtLineEnding::Newline), "测试/路径\n");
        }

        #[test]
        fn test_show_very_long_path() {
            let long_path = "a".repeat(1000);
            let expected_output = format!("{long_path}\n");
            test_show_output(&long_path, Some(CtLineEnding::Newline), &expected_output);
        }

        #[test]
        fn test_show_multiple_calls() {
            let path = "repeated_call";
            let mut output = Vec::new();
            let line_ending = Some(CtLineEnding::Newline);
            for _ in 0..3 {
                show_with_writer(Path::new(path), line_ending, &mut output).unwrap();
            }
            let expected_output = format!("{path}\n{path}\n{path}\n");
            assert_eq!(String::from_utf8(output).unwrap(), expected_output);
        }

        fn show_with_writer(
            path: &Path,
            line_ending: Option<CtLineEnding>,
            writer: &mut dyn Write,
        ) -> std::io::Result<()> {
            let path = path.to_str().unwrap();
            write!(writer, "{path}")?;
            if let Some(line_ending) = line_ending {
                write!(writer, "{line_ending}")?;
            }
            writer.flush()
        }
    }
    mod options_tests {
        use super::*;
        use clap::error::ErrorKind;

        #[cfg(unix)]
        #[test]
        fn gnu_long_option_errors_preserve_gnu_diagnostics() {
            let cases = [
                (
                    "--can",
                    b"option '--can' is ambiguous; possibilities: '--canonicalize' '--canonicalize-existing' '--canonicalize-missing'"
                        .as_slice(),
                ),
                (
                    "--ver",
                    b"option '--ver' is ambiguous; possibilities: '--verbose' '--version'".as_slice(),
                ),
                (
                    "--canonicalize=x",
                    b"option '--canonicalize' doesn't allow an argument".as_slice(),
                ),
                ("--unknown", b"unrecognized option '--unknown'".as_slice()),
            ];

            for (option, expected) in cases {
                let error = normalize_gnu_options(vec![
                    OsString::from(ctcore::ct_util_name()),
                    OsString::from(option),
                ])
                .expect_err("invalid GNU long options must fail during normalization");
                assert_eq!(error.diagnostic_bytes().as_ref(), expected);
            }
        }

        #[cfg(unix)]
        #[test]
        fn gnu_long_option_abbreviation_is_rewritten_to_its_canonical_name() {
            let normalized = normalize_gnu_options(vec![
                OsString::from(ctcore::ct_util_name()),
                OsString::from("--qui"),
                OsString::from("file"),
            ])
            .unwrap();

            assert_eq!(normalized, [ctcore::ct_util_name(), "--quiet", "file"]);
        }

        #[cfg(unix)]
        #[test]
        fn invalid_non_utf8_short_option_preserves_raw_diagnostic_bytes() {
            use std::os::unix::ffi::OsStringExt;

            let error = parse_readlink_args(
                [
                    OsString::from(ctcore::ct_util_name()),
                    OsString::from_vec(b"-\xff".to_vec()),
                ]
                .into_iter(),
            )
            .expect_err("a non-UTF-8 short option must fail");

            assert_eq!(
                error.diagnostic_bytes().as_ref(),
                b"invalid option -- '\xff'"
            );
        }

        #[test]
        fn short_help_and_version_options_are_invalid() {
            for option in ["-h", "-V"] {
                let error = ct_app().try_get_matches_from([ctcore::ct_util_name(), option]);
                assert_eq!(error.unwrap_err().kind(), ErrorKind::UnknownArgument);

                let error = parse_readlink_args(
                    [ctcore::ct_util_name(), option]
                        .into_iter()
                        .map(OsString::from),
                )
                .unwrap_err();
                assert_eq!(
                    error.to_string(),
                    format!("invalid option -- '{}'", &option[1..])
                );
            }
        }

        #[test]
        fn posixly_correct_stops_option_parsing_at_the_first_file() {
            let matches = ct_app_with_getopt_mode(true)
                .try_get_matches_from([ctcore::ct_util_name(), "link", "-m", "missing"])
                .unwrap();
            let options = ReadlinkOptions::from_matches(&matches).unwrap();
            assert_eq!(options.mode, ReadlinkMode::Readlink);
            assert_eq!(options.files, ["link", "-m", "missing"]);
        }

        #[test]
        fn posixly_correct_does_not_implicitly_enable_verbose() {
            let matches = ct_app_with_getopt_mode(true)
                .try_get_matches_from([ctcore::ct_util_name(), "missing"])
                .unwrap();
            let options = ReadlinkOptions::from_matches_with_posix(&matches, true).unwrap();

            assert!(!options.verbose);
        }

        #[test]
        fn verbose_uses_the_last_silence_or_verbose_option() {
            let cases = [
                (vec![ctcore::ct_util_name(), "-s", "-v", "missing"], true),
                (vec![ctcore::ct_util_name(), "-v", "-q", "missing"], false),
                (vec![ctcore::ct_util_name(), "-q", "-s", "missing"], false),
            ];

            for (args, expected_verbose) in cases {
                let matches = ct_app().try_get_matches_from(args).unwrap();
                let options = ReadlinkOptions::from_matches(&matches).unwrap();

                assert_eq!(options.verbose, expected_verbose);
            }
        }

        #[test]
        fn canonicalize_mode_uses_the_last_mode_option() {
            let cases = [
                (
                    vec![ctcore::ct_util_name(), "-e", "-m", "missing"],
                    ReadlinkMode::CanonicalizeMissing,
                ),
                (
                    vec![ctcore::ct_util_name(), "-m", "-f", "missing"],
                    ReadlinkMode::Canonicalize,
                ),
                (
                    vec![ctcore::ct_util_name(), "-f", "-e", "missing"],
                    ReadlinkMode::CanonicalizeExisting,
                ),
            ];

            for (args, expected_mode) in cases {
                let matches = ct_app().try_get_matches_from(args).unwrap();
                let options = ReadlinkOptions::from_matches(&matches).unwrap();

                assert_eq!(options.mode, expected_mode);
            }
        }

        #[test]
        fn canonicalize_modes_use_physical_resolution() {
            for option in ["-f", "-e", "-m"] {
                let matches = ct_app()
                    .try_get_matches_from([ctcore::ct_util_name(), option, "path"])
                    .unwrap();
                let options = ReadlinkOptions::from_matches(&matches).unwrap();

                assert_eq!(options.resolve_mode, ResolveMode::Physical);
            }
        }
    }

    #[cfg(test)]
    mod ct_main_tests {
        use super::*;
        use std::ffi::OsString;
        use std::fs::File;
        #[cfg(unix)]
        use std::os::unix::ffi::OsStringExt;
        use std::os::unix::fs::symlink;
        use tempfile::tempdir;
        #[test]
        fn test_readlink_main_execution_version() {
            let args = [ctcore::ct_util_name(), "--version"];
            let result = readlink_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_execution_other_version() {
            let args = [ctcore::ct_util_name(), "-V"];

            let result = readlink_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_execution_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_execution_help_short() {
            let args = [ctcore::ct_util_name(), "-h"];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_execution_unsupport_help() {
            let args = [ctcore::ct_util_name(), "-H"];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_invalid_argument() {
            let args = [ctcore::ct_util_name(), "--invalid-argument"];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_support_missing_argument() {
            let args = [ctcore::ct_util_name()];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn readlink_main_processes_operands_after_an_error() {
            let dir = tempdir().unwrap();
            let link_one = dir.path().join("link-one");
            let missing = dir.path().join("missing");
            let link_two = dir.path().join("link-two");
            symlink("first", &link_one).unwrap();
            symlink("second", &link_two).unwrap();

            let args = vec![
                OsString::from(ctcore::ct_util_name()),
                link_one.into_os_string(),
                missing.into_os_string(),
                link_two.into_os_string(),
            ];
            let mut output = Vec::new();

            assert!(readlink_main_with_writer(args.into_iter(), &mut output).is_err());
            assert_eq!(output, b"first\nsecond\n");
        }

        #[cfg(unix)]
        #[test]
        fn readlink_main_preserves_non_utf8_link_targets() {
            let dir = tempdir().unwrap();
            let link = dir.path().join(OsString::from_vec(b"link\xff".to_vec()));
            let target = PathBuf::from(OsString::from_vec(b"target\xff".to_vec()));
            symlink(&target, &link).unwrap();

            let args = vec![
                OsString::from(ctcore::ct_util_name()),
                link.into_os_string(),
            ];
            let mut output = Vec::new();

            assert!(readlink_main_with_writer(args.into_iter(), &mut output).is_ok());
            assert_eq!(output, b"target\xff\n");
        }

        #[test]
        fn readlink_main_ignores_no_newline_for_multiple_operands_even_when_silent() {
            let dir = tempdir().unwrap();
            let link_one = dir.path().join("link-one");
            let link_two = dir.path().join("link-two");
            symlink("first", &link_one).unwrap();
            symlink("second", &link_two).unwrap();

            let args = vec![
                OsString::from(ctcore::ct_util_name()),
                OsString::from("-n"),
                OsString::from("-s"),
                link_one.into_os_string(),
                link_two.into_os_string(),
            ];
            let mut output = Vec::new();

            assert!(readlink_main_with_writer(args.into_iter(), &mut output).is_ok());
            assert_eq!(output, b"first\nsecond\n");
        }

        #[test]
        fn canonicalize_modes_reject_an_empty_operand() {
            for option in ["-f", "-e", "-m"] {
                let args = [ctcore::ct_util_name(), option, ""];
                let mut output = Vec::new();

                let error =
                    readlink_main_with_writer(args.into_iter().map(OsString::from), &mut output)
                        .expect_err("GNU canonicalize modes reject an empty operand");
                assert_eq!(error.code(), 1);
                assert!(output.is_empty());
            }
        }

        #[test]
        fn test_readlink_main_canonicalize_long() {
            let filename = "test_readlink_main_canonicalize_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--canonicalize", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_canonicalize_short() {
            let filename = "test_readlink_main_canonicalize_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-f", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_canonicalize_existing_long() {
            let filename = "test_readlink_main_canonicalize_existing_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--canonicalize-existing", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_canonicalize_existing_short() {
            let filename = "test_readlink_main_canonicalize_existing_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-e", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_canonicalize_missing_long() {
            let filename = "test_readlink_main_canonicalize_existing_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--canonicalize-missing", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_canonicalize_missing_short() {
            let filename = "test_readlink_main_canonicalize_missing_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-m", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_readlink_main_no_newline_long() {
            let filename = "test_readlink_main_no_newline_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--no-newline", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_no_newline_short() {
            let filename = "test_readlink_main_no_newline_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-n", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_quiet_long() {
            let filename = "test_readlink_main_quiet_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--quiet", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_quiet_short() {
            let filename = "test_readlink_main_quiet_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-q", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_silent_short() {
            let filename = "test_readlink_main_quiet_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-s", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_silent_long() {
            let filename = "test_readlink_main_silent_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--silent", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_verbose_long() {
            let filename = "test_readlink_main_verbose_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--verbose", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_verbose_short() {
            let filename = "test_readlink_main_verbose_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-v", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_zero_long() {
            let filename = "test_readlink_main_zero_long";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--zero", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_readlink_main_zero_short() {
            let filename = "test_readlink_main_zero_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-z", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        // -->         let symlink_path = tmp_dir.path().join("symlink_dir");
        //             symlink(&dir_path, &symlink_path).unwrap();

        #[test]
        fn test_readlink_main_no_newline_long_with_symlink() {
            let filename = "test_readlink_main_no_newline_long_with_symlink";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();

            let symlink_path = dir.path().join("symlink_file");
            symlink(&file_path, &symlink_path).unwrap();
            let file_name = symlink_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--no-newline", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_no_newline_short_with_symlink() {
            let filename = "test_readlink_main_no_newline_short_with_symlink";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();

            let symlink_path = dir.path().join("symlink_file");
            symlink(&file_path, &symlink_path).unwrap();
            let file_name = symlink_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-n", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_quiet_long_with_symlink() {
            let filename = "test_readlink_main_quiet_long_with_symlink";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();

            let symlink_path = dir.path().join("symlink_file");
            symlink(&file_path, &symlink_path).unwrap();
            let file_name = symlink_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--quiet", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_quiet_short_with_symlink() {
            let filename = "test_readlink_main_quiet_short_with_symlink";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();

            let symlink_path = dir.path().join("symlink_file");
            symlink(&file_path, &symlink_path).unwrap();
            let file_name = symlink_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-q", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_silent_short_with_symlink() {
            let filename = "test_readlink_main_silent_short_with_symlink";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();

            let symlink_path = dir.path().join("symlink_file");
            symlink(&file_path, &symlink_path).unwrap();
            let file_name = symlink_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-s", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_silent_long_with_symlink() {
            let filename = "test_readlink_main_silent_long_with_symlink";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();

            let symlink_path = dir.path().join("symlink_file");
            symlink(&file_path, &symlink_path).unwrap();
            let file_name = symlink_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--silent", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_verbose_long_with_symlink() {
            let filename = "test_readlink_main_verbose_long_with_symlink";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();

            let symlink_path = dir.path().join("symlink_file");
            symlink(&file_path, &symlink_path).unwrap();
            let file_name = symlink_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--verbose", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_verbose_short_with_symlink() {
            let filename = "test_readlink_main_verbose_short_with_symlink";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();

            let symlink_path = dir.path().join("symlink_file");
            symlink(&file_path, &symlink_path).unwrap();
            let file_name = symlink_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-v", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_zero_long_with_symlink() {
            let filename = "test_readlink_main_zero_long_with_symlink";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();

            let symlink_path = dir.path().join("symlink_file");
            symlink(&file_path, &symlink_path).unwrap();
            let file_name = symlink_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--zero", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_readlink_main_zero_short_with_symlink() {
            let filename = "test_readlink_main_zero_short_with_symlink";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let _ = File::create(&file_path).unwrap();

            let symlink_path = dir.path().join("symlink_file");
            symlink(&file_path, &symlink_path).unwrap();
            let file_name = symlink_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-z", file_name];
            let result = readlink_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
    }
    #[cfg(test)]
    mod ct_app_tests {
        use clap::error::ErrorKind;

        use super::*;

        // readlink 接口: readlink [OPTION]... FILE...
        //
        // Arguments:
        //   [files]...
        //
        // Options:
        //   -f, --canonicalize           canonicalize by following every symlink in every component of the given name recursively; all but the last component must exist
        //   -e, --canonicalize-existing  canonicalize by following every symlink in every component of the given name recursively, all components must exist
        //   -m, --canonicalize-missing   canonicalize by following every symlink in every component of the given name recursively, without requirements on components existence
        //   -n, --no-newline             do not output the trailing delimiter
        //   -q, --quiet                  suppress most error messages
        //   -s, --silent                 suppress most error messages
        //   -v, --verbose                report error message
        //   -z, --zero                   separate output with NUL rather than newline
        #[test]
        fn test_ct_app_execution_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];
            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_rejects_short_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];

            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_execution_help() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "--help"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_rejects_short_help() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-h"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_execution_unsupport_help() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-H"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_invalid_argument() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_support_missing_argument() {
            let command = ct_app();

            let missing_args = vec![ctcore::ct_util_name()]; // 缺少任何参数
            let result = command.try_get_matches_from(missing_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_canonicalize_long() {
            let file_name = "test_ct_app_canonicalize_long";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "--canonicalize", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_canonicalize_short() {
            let file_name = "test_ct_app_canonicalize_short";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-f", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_canonicalize_existing_long() {
            let file_name = "test_ct_app_canonicalize_existing_long";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "--canonicalize-existing", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_canonicalize_existing_short() {
            let file_name = "test_ct_app_canonicalize_existing_short";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-e", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_canonicalize_missing_long() {
            let file_name = "test_ct_app_canonicalize_existing_long";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "--canonicalize-missing", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_canonicalize_missing_short() {
            let file_name = "test_ct_app_canonicalize_missing_short";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-m", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_no_newline_long() {
            let file_name = "test_ct_app_no_newline_long";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "--no-newline", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_no_newline_short() {
            let file_name = "test_ct_app_no_newline_short";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-n", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_quiet_long() {
            let file_name = "test_ct_app_quiet_long";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "--quiet", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_quiet_short() {
            let file_name = "test_ct_app_quiet_short";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-q", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_silent_short() {
            let file_name = "test_ct_app_quiet_short";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-s", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_silent_long() {
            let file_name = "test_ct_app_silent_long";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "--silent", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_verbose_long() {
            let file_name = "test_ct_app_verbose_long";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "--verbose", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_verbose_short() {
            let file_name = "test_ct_app_verbose_short";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-v", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_zero_long() {
            let file_name = "test_ct_app_zero_long";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "--zero", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_zero_short() {
            let file_name = "test_ct_app_zero_short";
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-z", file_name];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }
    }
}
