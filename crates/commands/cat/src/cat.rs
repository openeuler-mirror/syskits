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

extern crate rust_i18n;
use clap::Arg;
use clap::ArgAction;
use clap::Command;
use clap::crate_version;

use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CtSimpleError, set_ct_exit_code, strip_errno};
use ctcore::ct_fs::CtFileInformation;
use rust_i18n::t;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::fs::metadata;
use std::io::{self, IsTerminal, Read, Write};
use std::path::Path;
use sys_locale::get_locale;
use thiserror::Error;
rust_i18n::i18n!("locales", fallback = "en-US");

#[cfg(target_os = "linux")]
use ctcore::ct_signals::enable_pipe_errors;
#[cfg(unix)]
use nix::libc;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;

/// Linux splice support
#[cfg(target_os = "linux")]
#[allow(dead_code)]
mod splice;

#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;

#[derive(Error, Debug)]
enum CatError {
    /// Wrapper around `io::Error`
    #[error("{0}")]
    Io(#[from] io::Error),
    /// An error while writing transformed or copied bytes to stdout.
    #[error("{0}")]
    Write(io::Error),
    /// Wrapper around `nix::Error`
    #[cfg(target_os = "linux")]
    #[error("{0}")]
    Nix(#[from] nix::Error),
    /// Unknown file type; it's not a regular file, socket, etc.
    #[error("unknown filetype: {}", ft_debug)]
    UnknownFiletype {
        /// A debug print of the file type
        ft_debug: String,
    },
    #[error("Is a directory")]
    IsDirectory,
    #[error("input file is output file")]
    OutputIsInput,
    #[error("Too many levels of symbolic links")]
    TooManySymlinks,
}

type CatResult<T> = Result<T, CatError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CatNumberingMode {
    None,
    NonEmpty,
    All,
}

struct CatOutputOptions {
    /// Line numbering mode
    num_mode: CatNumberingMode,

    /// Suppress repeated empty output lines
    squeeze_blank: bool,

    /// display TAB characters as `tab`
    show_tabs: bool,

    /// Show end of lines
    show_ends: bool,

    /// use ^ and M- notation, except for LF (\\n) and TAB (\\t)
    show_non_print: bool,
}

impl CatOutputOptions {
    fn cat_tab(&self) -> &'static str {
        if self.show_tabs { "^I" } else { "\t" }
    }

    fn cat_end_of_line(&self) -> &'static str {
        if self.show_ends { "$\n" } else { "\n" }
    }

    /// We can write fast if we can simply copy the contents of the file to
    /// stdout, without augmenting the output with e.g. line numbers.
    fn cat_can_write_fast(&self) -> bool {
        !(self.show_tabs
            || self.show_non_print
            || self.show_ends
            || self.squeeze_blank
            || self.num_mode != CatNumberingMode::None)
    }
}

/// State that persists between output of each file. This struct is only used
/// when we can't write fast.
struct CatOutputState {
    /// The current line number
    line_number: usize,

    /// Whether the output cursor is at the beginning of a new line
    at_line_start: bool,

    /// Whether we skipped a \r, which still needs to be printed
    skipped_carriage_return: bool,

    /// Whether we have already printed a blank line
    one_blank_kept: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatRow {
    pub row_index: usize,
    pub line: String,
    pub is_blank: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatSemantic {
    pub number_mode: String,
    pub squeeze_blank: bool,
    pub show_tabs: bool,
    pub show_ends: bool,
    pub show_non_print: bool,
    pub rows: Vec<CatRow>,
    pub classic_text: String,
    pub stderr_text: String,
    pub exit_code: i32,
}

struct CatRunOutcome {
    stderr_text: String,
    exit_code: i32,
}

trait CatFdReadable: Read {}

impl<T> CatFdReadable for T where T: Read {}

/// Represents an open file handle, stream, or other device
struct CatInputHandle<R: CatFdReadable> {
    reader: R,
    is_interactive: bool,
}

/// Concrete enum of recognized file types.
///
/// *Note*: `cat`-ing a directory should result in an
/// CatError::IsDirectory
enum CatInputType {
    Directory,
    File,
    StdIn,
    SymLink,
    #[cfg(unix)]
    BlockDevice,
    #[cfg(unix)]
    CharacterDevice,
    #[cfg(unix)]
    Fifo,
    #[cfg(unix)]
    Socket,
}

mod opt_flags {
    pub static CAT_FILE: &str = "file";
    pub static CAT_SHOW_ALL: &str = "show-all";
    pub static CAT_NUMBER_NO_NBLANK: &str = "number-nonblank";
    pub static CAT_SHOW_NON_PRINTING_ENDS: &str = "e";
    pub static CAT_SHOW_ENDS: &str = "show-ends";
    pub static CAT_NUMBER: &str = "number";
    pub static CAT_SQUEEZE_BLANK: &str = "squeeze-blank";
    pub static CAT_SHOW_NON_PRINTING_TABS: &str = "t";
    pub static CAT_SHOW_TABS: &str = "show-tabs";
    pub static CAT_SHOW_NON_PRINTING: &str = "show-nonprinting";
    pub static CAT_IGNORED_U: &str = "ignored-u";
}

pub fn cat_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);

    #[cfg(target_os = "linux")]
    enable_pipe_errors().map_err(|err| CtSimpleError::new(1, err.to_string()))?;

    let args = args.collect::<Vec<_>>();
    match parse_cat_args(&args)? {
        CatAction::Run(options, files) => {
            cat_files_info_with_program(&files, &options, &cat_usage_name(&args))
        }
        CatAction::Help => {
            println!("{}", cat_help_text(&cat_usage_name(&args)));
            Ok(())
        }
        CatAction::Version => {
            println!("{}", cat_version_text(&cat_program_name(&args)));
            Ok(())
        }
    }
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("cat.about");
    let usage_description = t!("cat.usage");

    let args = args_init();
    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        // GNU getopt accepts repeated no-argument options, including forms
        // such as `-nn` and `--number --number`.
        .args_override_self(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .args(&args)
}

fn args_init() -> Vec<Arg> {
    let args = vec![
        Arg::new(opt_flags::CAT_FILE)
            .hide(true)
            .action(clap::ArgAction::Append)
            .value_hint(clap::ValueHint::FilePath),
        Arg::new(opt_flags::CAT_SHOW_ALL)
            .short('A')
            .long(opt_flags::CAT_SHOW_ALL)
            .help(t!("cat.clap.show-all"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::CAT_NUMBER_NO_NBLANK)
            .short('b')
            .long(opt_flags::CAT_NUMBER_NO_NBLANK)
            .help(t!("cat.clap.number-nonblank"))
            // 注意：这绝对不能overrides_with(options::NUMBER)！在clap中，覆盖操作是对称的，
            // 因此"-b -n"被视为"-n"，这不是我们想要的。
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::CAT_SHOW_NON_PRINTING_ENDS)
            .short('e')
            .help(t!("cat.clap.show-nonprinting"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::CAT_SHOW_ENDS)
            .short('E')
            .long(opt_flags::CAT_SHOW_ENDS)
            .help(t!("cat.clap.show-ends"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::CAT_NUMBER)
            .short('n')
            .long(opt_flags::CAT_NUMBER)
            .help(t!("cat.clap.number"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::CAT_SQUEEZE_BLANK)
            .short('s')
            .long(opt_flags::CAT_SQUEEZE_BLANK)
            .help(t!("cat.clap.squeeze-blank"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::CAT_SHOW_NON_PRINTING_TABS)
            .short('t')
            .help(t!("cat.clap.show-nonprinting"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::CAT_SHOW_TABS)
            .short('T')
            .long(opt_flags::CAT_SHOW_TABS)
            .help(t!("cat.clap.show-tabs"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::CAT_SHOW_NON_PRINTING)
            .short('v')
            .long(opt_flags::CAT_SHOW_NON_PRINTING)
            .help(t!("cat.clap.show-nonprinting"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::CAT_IGNORED_U)
            .short('u')
            .help(t!("cat.clap.ignored-u"))
            .action(ArgAction::SetTrue),
        Arg::new("help")
            .long("help")
            .help(t!("cat.clap.help"))
            .action(ArgAction::Help),
        Arg::new("version")
            .long("version")
            .help(t!("cat.clap.version"))
            .action(ArgAction::Version),
    ];
    args
}

enum CatAction {
    Run(CatOutputOptions, Vec<OsString>),
    Help,
    Version,
}

const CAT_LONG_OPTIONS: [&str; 9] = [
    "number-nonblank",
    "number",
    "squeeze-blank",
    "show-nonprinting",
    "show-ends",
    "show-tabs",
    "show-all",
    "help",
    "version",
];

fn parse_cat_args(args: &[OsString]) -> CTResult<CatAction> {
    parse_cat_args_with_posixly_correct(args, std::env::var_os("POSIXLY_CORRECT").is_some())
}

fn parse_cat_args_with_posixly_correct(
    args: &[OsString],
    posixly_correct: bool,
) -> CTResult<CatAction> {
    let usage_name = cat_usage_name(args);
    let mut number = false;
    let mut number_nonblank = false;
    let mut squeeze_blank = false;
    let mut show_ends = false;
    let mut show_non_print = false;
    let mut show_tabs = false;
    let mut files = Vec::new();
    let mut i = 1;

    while i < args.len() {
        let arg = args[i].as_os_str();
        let arg_bytes = arg.as_bytes();

        if arg_bytes == b"--" {
            files.extend(args[i + 1..].iter().cloned());
            break;
        }

        if arg_bytes == b"-" || !arg_bytes.starts_with(b"-") {
            files.push(args[i].clone());
            if posixly_correct {
                files.extend(args[i + 1..].iter().cloned());
                break;
            }
            i += 1;
            continue;
        }

        if let Some(long) = arg_bytes.strip_prefix(b"--") {
            let (name, has_value) = match long.iter().position(|byte| *byte == b'=') {
                Some(index) => (&long[..index], true),
                None => (long, false),
            };
            let matches = cat_long_option_matches(&String::from_utf8_lossy(name));
            let display_arg = String::from_utf8_lossy(arg_bytes);

            match matches.as_slice() {
                [] => {
                    return Err(cat_usage_error(
                        &usage_name,
                        format!("unrecognized option '{display_arg}'"),
                    ));
                }
                [option] => {
                    if has_value {
                        return Err(cat_usage_error(
                            &usage_name,
                            format!("option '--{option}' doesn't allow an argument"),
                        ));
                    }

                    match *option {
                        "number-nonblank" => {
                            number = true;
                            number_nonblank = true;
                        }
                        "number" => number = true,
                        "squeeze-blank" => squeeze_blank = true,
                        "show-nonprinting" => show_non_print = true,
                        "show-ends" => show_ends = true,
                        "show-tabs" => show_tabs = true,
                        "show-all" => {
                            show_non_print = true;
                            show_ends = true;
                            show_tabs = true;
                        }
                        "help" => return Ok(CatAction::Help),
                        "version" => return Ok(CatAction::Version),
                        _ => unreachable!("all GNU cat long options are covered"),
                    }
                }
                _ => {
                    let possibilities = matches
                        .iter()
                        .map(|option| format!("'--{option}'"))
                        .collect::<Vec<_>>()
                        .join(" ");
                    return Err(cat_usage_error(
                        &usage_name,
                        format!(
                            "option '{display_arg}' is ambiguous; possibilities: {possibilities}"
                        ),
                    ));
                }
            }
        } else {
            for option in &arg_bytes[1..] {
                match option {
                    b'b' => {
                        number = true;
                        number_nonblank = true;
                    }
                    b'e' => {
                        show_ends = true;
                        show_non_print = true;
                    }
                    b'n' => number = true,
                    b's' => squeeze_blank = true,
                    b't' => {
                        show_tabs = true;
                        show_non_print = true;
                    }
                    b'u' => {}
                    b'v' => show_non_print = true,
                    b'A' => {
                        show_non_print = true;
                        show_ends = true;
                        show_tabs = true;
                    }
                    b'E' => show_ends = true,
                    b'T' => show_tabs = true,
                    invalid => {
                        return Err(cat_usage_error(
                            &usage_name,
                            format!("invalid option -- '{}'", *invalid as char),
                        ));
                    }
                }
            }
        }

        i += 1;
    }

    if files.is_empty() {
        files.push(OsString::from("-"));
    }

    Ok(CatAction::Run(
        CatOutputOptions {
            num_mode: if number_nonblank {
                CatNumberingMode::NonEmpty
            } else if number {
                CatNumberingMode::All
            } else {
                CatNumberingMode::None
            },
            squeeze_blank,
            show_tabs,
            show_ends,
            show_non_print,
        },
        files,
    ))
}

fn cat_long_option_matches(name: &str) -> Vec<&'static str> {
    CAT_LONG_OPTIONS
        .into_iter()
        .filter(|option| option.starts_with(name))
        .collect()
}

fn cat_usage_name(args: &[OsString]) -> String {
    args.first()
        .map(|arg| arg.to_string_lossy().into_owned())
        .filter(|arg| !arg.is_empty())
        .unwrap_or_else(|| ctcore::ct_util_name().to_owned())
}

fn cat_usage_error(usage_name: &str, message: String) -> Box<dyn ctcore::ct_error::CTError> {
    CtSimpleError::new(
        1,
        format!("{message}\nTry '{usage_name} --help' for more information."),
    )
}

fn cat_program_name(args: &[OsString]) -> String {
    let usage_name = cat_usage_name(args);

    Path::new(&usage_name)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or(usage_name)
}

fn cat_help_text(usage_name: &str) -> String {
    format!(
        "\
Usage: {usage_name} [OPTION]... [FILE]...
Concatenate FILE(s) to standard output.

With no FILE, or when FILE is -, read standard input.

  -A, --show-all           equivalent to -vET
  -b, --number-nonblank    number nonempty output lines, overrides -n
  -e                       equivalent to -vE
  -E, --show-ends          display $ at end of each line
  -n, --number             number all output lines
  -s, --squeeze-blank      suppress repeated empty output lines
  -t                       equivalent to -vT
  -T, --show-tabs          display TAB characters as ^I
  -u                       (ignored)
  -v, --show-nonprinting   use ^ and M- notation, except for LFD and TAB
      --help        display this help and exit
      --version     output version information and exit

Examples:
  {usage_name} f - g  Output f's contents, then standard input, then g's contents.
  {usage_name}        Copy standard input to standard output.

GNU coreutils online help: <https://www.gnu.org/software/coreutils/>
Report any translation bugs to <https://translationproject.org/team/>
Full documentation <https://www.gnu.org/software/coreutils/cat>
or available locally via: info '(coreutils) cat invocation'"
    )
}

fn cat_version_text(program_name: &str) -> String {
    format!(
        "\
{program_name} (syskits) {}
Copyright (C) 2022-2025 China Telecom Cloud Technologies Co., Ltd.
License Mulan PSL v2: <http://license.coscl.org.cn/MulanPSL2>.
This is free software: you are free to change and redistribute it.
There is NO WARRANTY, to the extent permitted by law.",
        crate_version!()
    )
}

fn cat_numbering_mode_name(mode: CatNumberingMode) -> &'static str {
    match mode {
        CatNumberingMode::None => "none",
        CatNumberingMode::NonEmpty => "nonempty",
        CatNumberingMode::All => "all",
    }
}

fn cat_handle<R: CatFdReadable>(
    input_handle: &mut CatInputHandle<R>,
    output_options: &CatOutputOptions,
    output_state: &mut CatOutputState,
) -> CatResult<()> {
    let mut cat_stdout = cat_stdout_file()?;
    cat_handle_to_writer(input_handle, output_options, output_state, &mut cat_stdout)
}

fn cat_handle_to_writer<R: CatFdReadable, W: Write>(
    input_handle: &mut CatInputHandle<R>,
    output_options: &CatOutputOptions,
    output_state: &mut CatOutputState,
    cat_writer: &mut W,
) -> CatResult<()> {
    if output_options.cat_can_write_fast() {
        cat_write_fast_to_writer(input_handle, cat_writer)
    } else {
        cat_write_lines_to_writer(input_handle, output_options, output_state, cat_writer)
    }
}

fn cat_stdout_file() -> CatResult<File> {
    let stdout = io::stdout();
    Ok(ctcore::ct_io::CtOwnedFileDescriptorOrHandle::from(&stdout)?.into_file())
}

/**
 * 处理给定路径的文件或目录，并根据选项和状态输出内容。
 *
 */
fn cat_path(
    input_path: &OsStr,
    output_options: &CatOutputOptions,
    output_state: &mut CatOutputState,
    out_info: Option<&CtFileInformation>,
) -> CatResult<()> {
    // 根据路径决定输入类型，并据此处理输入
    match cat_get_input_type(input_path)? {
        CatInputType::StdIn => {
            // 处理标准输入
            let stdin = io::stdin();

            // 检查 stdin 是否重定向到与输出文件相同的文件
            if let Some(out_info) = out_info {
                // 尝试从 stdin 获取文件信息进行比较
                if let Ok(stdin_info) = CtFileInformation::from_file(&stdin) {
                    if stdin_info == *out_info {
                        // 实现类似 GNU cat 的位置检测逻辑
                        use std::os::unix::io::AsRawFd;

                        let input_fd = stdin.as_raw_fd();
                        let stdout_fd = std::io::stdout().as_raw_fd();

                        // 获取输入文件当前位置
                        let input_pos = unsafe { libc::lseek(input_fd, 0, libc::SEEK_CUR) };

                        if input_pos >= 0 {
                            // 获取输出文件标志来确定 seek 模式
                            let out_flags = unsafe { libc::fcntl(stdout_fd, libc::F_GETFL) };
                            let whence = if out_flags >= 0 && (out_flags & libc::O_APPEND) != 0 {
                                libc::SEEK_END
                            } else {
                                libc::SEEK_CUR
                            };

                            // 获取输出文件位置
                            let output_pos = unsafe { libc::lseek(stdout_fd, 0, whence) };

                            // 只有当输入位置 < 输出位置时才报告错误
                            if input_pos < output_pos {
                                return Err(CatError::OutputIsInput);
                            }
                        }
                    }
                }
            }

            let mut handle = CatInputHandle {
                reader: stdin,
                is_interactive: std::io::stdin().is_terminal(),
            };
            cat_handle(&mut handle, output_options, output_state)
        }
        CatInputType::Directory => Err(CatError::IsDirectory), // 目录不是有效的输入
        _ => {
            // 处理普通文件输入
            let file = File::open(Path::new(input_path))?;

            // 如果提供了输出信息且尝试将输出重定向到输入，进行位置检测
            if let Some(out_info) = out_info {
                if CtFileInformation::from_file(&file).ok().as_ref() == Some(out_info) {
                    // 实现类似 GNU cat 的位置检测逻辑
                    use std::os::unix::io::AsRawFd;

                    let input_fd = file.as_raw_fd();
                    let stdout_fd = std::io::stdout().as_raw_fd();

                    // 获取输入文件当前位置
                    let input_pos = unsafe { libc::lseek(input_fd, 0, libc::SEEK_CUR) };

                    if input_pos >= 0 {
                        // 获取输出文件标志来确定 seek 模式
                        let out_flags = unsafe { libc::fcntl(stdout_fd, libc::F_GETFL) };
                        let whence = if out_flags >= 0 && (out_flags & libc::O_APPEND) != 0 {
                            libc::SEEK_END
                        } else {
                            libc::SEEK_CUR
                        };

                        // 获取输出文件位置
                        let output_pos = unsafe { libc::lseek(stdout_fd, 0, whence) };

                        // 只有当输入位置 < 输出位置时才报告错误
                        if input_pos < output_pos {
                            return Err(CatError::OutputIsInput);
                        }
                    }
                }
            }

            let mut handle = CatInputHandle {
                reader: file,
                is_interactive: false,
            };
            cat_handle(&mut handle, output_options, output_state)
        }
    }
}

fn cat_path_to_writer<W: Write>(
    input_path: &OsStr,
    output_options: &CatOutputOptions,
    output_state: &mut CatOutputState,
    out_info: Option<&CtFileInformation>,
    cat_writer: &mut W,
) -> CatResult<()> {
    match cat_get_input_type(input_path)? {
        CatInputType::StdIn => {
            let mut handle = CatInputHandle {
                reader: ctcore::ct_io::stdin_reader_box(),
                is_interactive: ctcore::ct_io::injected_stdin_bytes().is_none()
                    && std::io::stdin().is_terminal(),
            };
            cat_handle_to_writer(&mut handle, output_options, output_state, cat_writer)
        }
        CatInputType::Directory => Err(CatError::IsDirectory),
        _ => {
            let file = File::open(Path::new(input_path))?;

            if let Some(out_info) = out_info {
                if CtFileInformation::from_file(&file).ok().as_ref() == Some(out_info) {
                    use std::os::unix::io::AsRawFd;

                    let input_fd = file.as_raw_fd();
                    let stdout_fd = std::io::stdout().as_raw_fd();
                    let input_pos = unsafe { libc::lseek(input_fd, 0, libc::SEEK_CUR) };

                    if input_pos >= 0 {
                        let out_flags = unsafe { libc::fcntl(stdout_fd, libc::F_GETFL) };
                        let whence = if out_flags >= 0 && (out_flags & libc::O_APPEND) != 0 {
                            libc::SEEK_END
                        } else {
                            libc::SEEK_CUR
                        };
                        let output_pos = unsafe { libc::lseek(stdout_fd, 0, whence) };

                        if input_pos < output_pos {
                            return Err(CatError::OutputIsInput);
                        }
                    }
                }
            }

            let mut handle = CatInputHandle {
                reader: file,
                is_interactive: false,
            };
            cat_handle_to_writer(&mut handle, output_options, output_state, cat_writer)
        }
    }
}

fn render_cat_file_error(program_name: &str, file_path: &OsStr, err: &CatError) -> String {
    let quoted_path = cat_quote_path(file_path);

    match err {
        CatError::OutputIsInput => {
            format!("{program_name}: {quoted_path}: input file is output file\n")
        }
        CatError::Io(io_err) => format!("{program_name}: {quoted_path}: {}\n", strip_errno(io_err)),
        CatError::Write(io_err) => render_cat_write_error(program_name, io_err),
        other => format!("{program_name}: {quoted_path}: {other}\n"),
    }
}

fn cat_quote_path(file_path: &OsStr) -> String {
    #[cfg(unix)]
    if file_path.to_str().is_none() {
        return cat_quote_non_utf8_path(file_path.as_bytes());
    }

    file_path
        .to_string_lossy()
        .as_ref()
        .maybe_quote()
        .to_string()
}

#[cfg(unix)]
enum CatQuoteSegment {
    Plain,
    Escaped,
}

#[cfg(unix)]
fn cat_quote_non_utf8_path(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(bytes.len() + 8);
    let mut segment = None;

    for &byte in bytes {
        match byte {
            b'\x07' => cat_push_escaped_byte(&mut result, &mut segment, "a"),
            b'\x08' => cat_push_escaped_byte(&mut result, &mut segment, "b"),
            b'\x09' => cat_push_escaped_byte(&mut result, &mut segment, "t"),
            b'\x0a' => cat_push_escaped_byte(&mut result, &mut segment, "n"),
            b'\x0b' => cat_push_escaped_byte(&mut result, &mut segment, "v"),
            b'\x0c' => cat_push_escaped_byte(&mut result, &mut segment, "f"),
            b'\x0d' => cat_push_escaped_byte(&mut result, &mut segment, "r"),
            b'\x20'..=b'\x7e' => cat_push_plain_byte(&mut result, &mut segment, byte),
            _ => {
                let escaped = format!("{:03o}", byte);
                cat_push_escaped_byte(&mut result, &mut segment, &escaped);
            }
        }
    }

    match segment {
        Some(CatQuoteSegment::Plain) | Some(CatQuoteSegment::Escaped) => result.push('\''),
        None => result.push_str("''"),
    }

    result
}

#[cfg(unix)]
fn cat_push_plain_byte(result: &mut String, segment: &mut Option<CatQuoteSegment>, byte: u8) {
    if matches!(segment, Some(CatQuoteSegment::Escaped)) {
        result.push('\'');
        *segment = None;
    }
    if segment.is_none() {
        result.push('\'');
        *segment = Some(CatQuoteSegment::Plain);
    }

    if byte == b'\'' {
        result.push_str("'\\''");
    } else {
        result.push(char::from(byte));
    }
}

#[cfg(unix)]
fn cat_push_escaped_byte(
    result: &mut String,
    segment: &mut Option<CatQuoteSegment>,
    escaped: &str,
) {
    if matches!(segment, Some(CatQuoteSegment::Plain)) {
        result.push('\'');
        *segment = None;
    }
    if segment.is_none() {
        result.push_str("$'");
        *segment = Some(CatQuoteSegment::Escaped);
    }

    result.push('\\');
    result.push_str(escaped);
}

fn render_cat_write_error(program_name: &str, err: &io::Error) -> String {
    format!("{program_name}: write error: {}\n", strip_errno(err))
}

fn cat_files_to_writer_with_program<W: Write, S: AsRef<OsStr>>(
    input_files: &[S],
    output_options: &CatOutputOptions,
    cat_writer: &mut W,
    program_name: &str,
) -> CatRunOutcome {
    let output_info = CtFileInformation::from_file(&std::io::stdout()).ok();
    let mut output_state = CatOutputState {
        line_number: 1,
        at_line_start: true,
        skipped_carriage_return: false,
        one_blank_kept: false,
    };
    let mut stderr_text = String::new();
    let mut exit_code = 0;

    for file_path in input_files {
        let input_path = file_path.as_ref();
        if let Err(err) = cat_path_to_writer(
            input_path,
            output_options,
            &mut output_state,
            output_info.as_ref(),
            cat_writer,
        ) {
            let stop_processing = matches!(&err, CatError::Write(_));
            exit_code = 1;
            stderr_text.push_str(&render_cat_file_error(program_name, input_path, &err));
            if stop_processing {
                break;
            }
        }
    }

    if output_state.skipped_carriage_return {
        if let Err(err) = cat_writer.write_all(b"\r").and_then(|_| cat_writer.flush()) {
            exit_code = 1;
            stderr_text.push_str(&render_cat_write_error(program_name, &err));
        }
    }

    CatRunOutcome {
        stderr_text,
        exit_code,
    }
}

/**
 * 合并文件信息。
 *
 * 此函数遍历给定的文件路径列表，尝试合并这些文件的内容到标准输出中。
 * 它处理各种输出选项，并在遇到错误时收集错误信息，最后返回成功或失败的结果。
 */
#[cfg(test)]
fn cat_files_info<S: AsRef<OsStr>>(
    input_files: &[S],
    output_options: &CatOutputOptions,
) -> CTResult<()> {
    cat_files_info_with_program(input_files, output_options, "cat")
}

fn cat_files_info_with_program<S: AsRef<OsStr>>(
    input_files: &[S],
    output_options: &CatOutputOptions,
    program_name: &str,
) -> CTResult<()> {
    let output_info = CtFileInformation::from_file(&std::io::stdout()).ok();
    let mut output_state = CatOutputState {
        line_number: 1,
        at_line_start: true,
        skipped_carriage_return: false,
        one_blank_kept: false,
    };
    let mut stderr_text = String::new();

    for file_path in input_files {
        let input_path = file_path.as_ref();
        if let Err(err) = cat_path(
            input_path,
            output_options,
            &mut output_state,
            output_info.as_ref(),
        ) {
            let stop_processing = matches!(&err, CatError::Write(_));
            set_ct_exit_code(1);
            stderr_text.push_str(&render_cat_file_error(program_name, input_path, &err));
            if stop_processing {
                break;
            }
        }
    }

    if output_state.skipped_carriage_return {
        if let Err(err) = cat_write_pending_carriage_return() {
            set_ct_exit_code(1);
            match err {
                CatError::Write(io_err) => {
                    stderr_text.push_str(&render_cat_write_error(program_name, &io_err))
                }
                other => stderr_text.push_str(&format!("{program_name}: {other}\n")),
            }
        }
    }

    if stderr_text.is_empty() {
        Ok(())
    } else {
        eprint!("{stderr_text}");
        Err(CtSimpleError::new(1, String::new()))
    }
}

fn cat_write_pending_carriage_return() -> CatResult<()> {
    let mut writer = cat_stdout_file()?;
    writer.write_all(b"\r").map_err(CatError::Write)?;
    writer.flush().map_err(CatError::Write)
}

/// Classifies the `InputType` of file at `path` if possible
///
/// # Arguments
///
/// * `path` - Path on a file system to classify metadata
fn cat_get_input_type(input_path: &OsStr) -> CatResult<CatInputType> {
    if input_path == OsStr::new("-") {
        return Ok(CatInputType::StdIn);
    }

    let file_type = match metadata(Path::new(input_path)) {
        Ok(metadata) => metadata.file_type(),
        Err(e) => {
            if let Some(raw_error_msg) = e.raw_os_error() {
                // 在类Unix系统中，"符号链接层数过多"的错误代码为40（ELOOP）。
                // 在此情况下，我们希望提供一个恰当的错误消息。
                #[cfg(target_os = "linux")]
                let ct_symlink_code = 40;
                if raw_error_msg == ct_symlink_code {
                    return Err(CatError::TooManySymlinks);
                }
            }
            return Err(CatError::Io(e));
        }
    };
    match file_type {
        #[cfg(unix)]
        filettype if filettype.is_block_device() => Ok(CatInputType::BlockDevice),
        #[cfg(unix)]
        filettype if filettype.is_char_device() => Ok(CatInputType::CharacterDevice),
        #[cfg(unix)]
        filettype if filettype.is_fifo() => Ok(CatInputType::Fifo),
        #[cfg(unix)]
        filettype if filettype.is_socket() => Ok(CatInputType::Socket),
        filettype if filettype.is_dir() => Ok(CatInputType::Directory),
        filettype if filettype.is_file() => Ok(CatInputType::File),
        filettype if filettype.is_symlink() => Ok(CatInputType::SymLink),
        _ => Err(CatError::UnknownFiletype {
            ft_debug: format!("{file_type:?}"),
        }),
    }
}

fn cat_write_fast_to_writer<R: CatFdReadable, W: Write>(
    input_handle: &mut CatInputHandle<R>,
    cat_writer: &mut W,
) -> CatResult<()> {
    let mut buffer = [0; 1024 * 64];
    loop {
        let n = match input_handle.reader.read(&mut buffer) {
            Ok(n) => n,
            Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err.into()),
        };
        if n == 0 {
            break;
        }
        cat_writer
            .write_all(&buffer[..n])
            .map_err(CatError::Write)?;
    }
    cat_writer.flush().map_err(CatError::Write)
}

fn cat_write_lines_to_writer<R: CatFdReadable, W: Write>(
    input_handle: &mut CatInputHandle<R>,
    output_options: &CatOutputOptions,
    output_state: &mut CatOutputState,
    mut stdout_writer: &mut W,
) -> CatResult<()> {
    let mut input_buffer = [0; 1024 * 31];
    loop {
        let n = match input_handle.reader.read(&mut input_buffer) {
            Ok(n) => n,
            Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err.into()),
        };
        if n == 0 {
            break;
        }
        let in_buffer = &input_buffer[..n];
        let mut position = 0;
        while position < n {
            // 如果需要，跳过空的行号，在枚举它们时不予计数。
            if in_buffer[position] == b'\n' {
                cat_write_new_line(
                    &mut stdout_writer,
                    output_options,
                    output_state,
                    input_handle.is_interactive,
                )?;
                output_state.at_line_start = true;
                position += 1;
                continue;
            }
            if output_state.skipped_carriage_return {
                stdout_writer.write_all(b"\r").map_err(CatError::Write)?;
                output_state.skipped_carriage_return = false;
                output_state.at_line_start = false;
            }
            // 重置空行保留状态，因为我们遇到了非空行内容
            output_state.one_blank_kept = false;
            if output_state.at_line_start && output_options.num_mode != CatNumberingMode::None {
                write!(stdout_writer, "{0:6}\t", output_state.line_number)
                    .map_err(CatError::Write)?;
                output_state.line_number += 1;
            }

            // 打印至行尾或缓冲区尾部。
            let offset = cat_write_end(&mut stdout_writer, &in_buffer[position..], output_options)?;

            // 是否已到达缓冲区尾部？
            if offset + position == in_buffer.len() {
                output_state.at_line_start = false;
                break;
            }
            if in_buffer[position + offset] == b'\r' {
                // 检查是否为\r\n序列
                if position + offset + 1 < in_buffer.len()
                    && in_buffer[position + offset + 1] == b'\n'
                {
                    // 这是\r\n序列，作为一个完整的换行处理
                    if output_options.show_ends {
                        stdout_writer.write_all(b"^M").map_err(CatError::Write)?;
                    } else {
                        stdout_writer.write_all(b"\r").map_err(CatError::Write)?;
                    }
                    cat_write_end_of_line(
                        &mut stdout_writer,
                        output_options.cat_end_of_line().as_bytes(),
                        input_handle.is_interactive,
                    )?;
                    output_state.at_line_start = true;
                    position += 1; // 跳过\n，因为我们已经处理了\r\n序列
                } else {
                    output_state.skipped_carriage_return = true;
                }
            } else {
                assert_eq!(in_buffer[position + offset], b'\n');
                // 打印适当的行尾字符。
                cat_write_end_of_line(
                    &mut stdout_writer,
                    output_options.cat_end_of_line().as_bytes(),
                    input_handle.is_interactive,
                )?;
                output_state.at_line_start = true;
            }
            position += offset + 1;
        }
    }

    stdout_writer.flush().map_err(CatError::Write)
}

// 当启用show_ends时，将\r后跟\n打印为^M，以便\r\n显示为^M$。
/**
 * 在给定的写入器上写入一个新的换行符，根据提供的选项和状态进行调整。
 *
 * 此函数主要用于在处理文本时，在适当的情况下添加换行符、行号、以及文件的结束标记。
 * 它考虑了是否在交互模式下运行、是否要压缩空白行、以及是否显示文件的起止标记等选项。
 *
 */
fn cat_write_new_line<W: Write>(
    cat_writer: &mut W,
    output_options: &CatOutputOptions,
    output_state: &mut CatOutputState,
    is_interactive: bool,
) -> CatResult<()> {
    // 如果之前跳过了回车符，需要先处理它
    if output_state.skipped_carriage_return {
        if output_options.show_ends {
            cat_writer.write_all(b"^M").map_err(CatError::Write)?;
        } else {
            cat_writer.write_all(b"\r").map_err(CatError::Write)?;
        }
        output_state.skipped_carriage_return = false;
    }

    // squeeze_blank逻辑：仅当处于行首、需要压缩空行、且已经保留了一个空行时才跳过
    // 但要确保至少输出一个空行
    if output_state.at_line_start && output_options.squeeze_blank {
        if output_state.one_blank_kept {
            return Ok(()); // 跳过额外的空行
        } else {
            output_state.one_blank_kept = true; // 标记已保留一个空行
        }
    }

    // 如果处于行首且要求对所有行编号，则写入行号
    if output_state.at_line_start && output_options.num_mode == CatNumberingMode::All {
        write!(cat_writer, "{0:6}\t", output_state.line_number).map_err(CatError::Write)?;
        output_state.line_number += 1;
    }

    // 根据选项，写入行尾标记
    cat_writer
        .write_all(output_options.cat_end_of_line().as_bytes())
        .map_err(CatError::Write)?;
    // 如果处于交互模式，刷新输出
    if is_interactive {
        cat_writer.flush().map_err(CatError::Write)?;
    }
    Ok(())
}
/**
 * 将指定的字节数据写入到提供的写入器中，根据提供的选项进行特定的处理。
 *
 */
fn cat_write_end<W: Write>(
    cat_writer: &mut W,
    in_buffer: &[u8],
    output_options: &CatOutputOptions,
) -> CatResult<usize> {
    // 根据选项决定如何处理并写入数据
    if output_options.show_non_print {
        // 如果显示非打印字符，则调用相应函数处理并写入
        cat_write_non_print_to_end(in_buffer, cat_writer, output_options.cat_tab().as_bytes())
    } else if output_options.show_tabs {
        // 如果只显示制表符，则调用相应函数处理并写入
        cat_write_tab_to_end(in_buffer, cat_writer)
    } else {
        // 默认情况下，直接写入数据
        cat_write_to_end(in_buffer, cat_writer)
    }
}

// 这些是名为write***_to_end的方法：
// 目标：将所有符号写入直到遇到换行符\n、回车符\r或达到缓冲区尾部。
// 特殊处理：在遇到\r时停止，因为根据后续字节及设置，它可能被打印为^M。
// 而write_nonprint_to_end方法无需在\r处停止，因为它始终会将\r打印为^M。
// 返回值：返回已写入符号的数量。
/**
 * 将输入缓冲区中的数据写入指定的写入器，直到遇到换行符或回车符为止。
 *
 * # 参数
 * - `in_buffer`: 一个包含待写入数据的字节切片。
 * - `writer`: 指向一个实现了 `Write` 接口的写入器的可变引用，数据将被写入这个写入器。
 *
 * # 返回值
 * 返回值为写入的字节数。
 */
fn cat_write_to_end<W: Write>(in_buffer: &[u8], cat_writer: &mut W) -> CatResult<usize> {
    // 查找缓冲区中第一个换行符或回车符的位置
    match in_buffer.iter().position(|c| *c == b'\n' || *c == b'\r') {
        Some(p) => {
            // 如果找到了换行符或回车符，将从缓冲区起始位置到该位置的字节写入写入器
            cat_writer
                .write_all(&in_buffer[..p])
                .map_err(CatError::Write)?;
            Ok(p) // 返回写入的字节数
        }
        None => {
            // 如果没有找到换行符或回车符，将整个缓冲区的字节写入写入器
            cat_writer.write_all(in_buffer).map_err(CatError::Write)?;
            Ok(in_buffer.len()) // 返回写入的字节数（即整个缓冲区的长度）
        }
    }
}

/**
 * 将字节切片中的内容写入指定的写入器，遇到换行符或制表符时特殊处理。
 * - 对于制表符，将其替换为"^I"并继续处理后续字符。
 * - 对于换行符或回车符，停止处理当前行，并返回已处理字符的数量。
 *
 */
fn cat_write_tab_to_end<W: Write>(mut in_buffer: &[u8], cat_writer: &mut W) -> CatResult<usize> {
    let mut count = 0; // 已处理字符计数器。

    loop {
        // 查找切片中第一个换行符、制表符或回车符的位置。
        match in_buffer
            .iter()
            .position(|c| *c == b'\n' || *c == b'\t' || *c == b'\r')
        {
            Some(p) => {
                // 写入找到特殊字符之前的切片内容。
                cat_writer
                    .write_all(&in_buffer[..p])
                    .map_err(CatError::Write)?;

                if in_buffer[p] == b'\t' {
                    // 如果找到的是制表符，替换为"^I"并继续处理后续字符。
                    cat_writer.write_all(b"^I").map_err(CatError::Write)?;
                    in_buffer = &in_buffer[p + 1..];
                    count += p + 1;
                } else {
                    // 如果找到的是换行符或回车符，停止处理当前行。
                    return Ok(count + p);
                }
            }
            None => {
                // 如果切片中没有特殊字符，一次性写入全部内容并返回。
                cat_writer.write_all(in_buffer).map_err(CatError::Write)?;
                return Ok(count + in_buffer.len());
            }
        };
    }
}

/// 将非打印字符转义后写入指定的writer中。
///
/// # 参数
/// - `in_buffer`: 一个包含待处理字节的slice。
/// - `writer`: 指向一个可写入目标的引用，例如文件或标准输出。
/// - `tab`: 一个用于替代制表符（tab）的字节slice。
///
/// # 返回值
/// 返回写入的字节数。
fn cat_write_non_print_to_end<W: Write>(
    in_buffer: &[u8],
    cat_writer: &mut W,
    cat_tab: &[u8],
) -> CatResult<usize> {
    let mut count = 0; // 用于记录已写入的字节数

    for c in in_buffer.iter().copied() {
        if c == b'\n' {
            // 当遇到换行符时停止处理
            break;
        }
        match c {
            9 => cat_writer.write_all(cat_tab), // 处理制表符
            0..=8 | 10..=31 => cat_writer.write_all(&[b'^', c + 64]), // 将控制字符转义为'^'加上对应字符
            32..=126 => cat_writer.write_all(&[c]),                   // 直接写入可见字符
            127 => cat_writer.write_all(b"^?"),                       // 特殊处理删除符
            128..=159 => cat_writer.write_all(&[b'M', b'-', b'^', c - 64]), // 处理128-159范围的字符
            160..=254 => cat_writer.write_all(&[b'M', b'-', c - 128]), // 处理160-254范围的字符
            _ => cat_writer.write_all(b"M-^?"), // 对于其他未知字符，使用默认转义序列
        }
        .map_err(CatError::Write)?;
        count += 1; // 增加处理的字节计数
    }
    Ok(count)
}

fn cat_write_end_of_line<W: Write>(
    cat_writer: &mut W,
    end_of_line: &[u8],
    is_interactive: bool,
) -> CatResult<()> {
    cat_writer.write_all(end_of_line).map_err(CatError::Write)?;
    if is_interactive {
        cat_writer.flush().map_err(CatError::Write)?;
    }
    Ok(())
}

fn cat_rows_from_output(output: &str) -> Vec<CatRow> {
    output
        .split_inclusive('\n')
        .enumerate()
        .map(|(index, segment)| {
            let line = segment.strip_suffix('\n').unwrap_or(segment).to_string();
            CatRow {
                row_index: index + 1,
                is_blank: line.is_empty(),
                line,
            }
        })
        .collect()
}

fn cat_semantic_from_invocation(
    input_files: &[OsString],
    output_options: &CatOutputOptions,
    program_name: &str,
) -> CatSemantic {
    let mut buffer = Vec::new();
    let outcome =
        cat_files_to_writer_with_program(input_files, output_options, &mut buffer, program_name);
    let classic_text = String::from_utf8_lossy(&buffer).into_owned();

    CatSemantic {
        number_mode: cat_numbering_mode_name(output_options.num_mode).into(),
        squeeze_blank: output_options.squeeze_blank,
        show_tabs: output_options.show_tabs,
        show_ends: output_options.show_ends,
        show_non_print: output_options.show_non_print,
        rows: cat_rows_from_output(&classic_text),
        classic_text,
        stderr_text: outcome.stderr_text,
        exit_code: outcome.exit_code,
    }
}

pub fn cat_native_semantic(args: impl ctcore::Args) -> CTResult<CatSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);

    let args = args.collect::<Vec<_>>();
    match parse_cat_args(&args)? {
        CatAction::Run(options, files) => Ok(cat_semantic_from_invocation(
            &files,
            &options,
            &cat_usage_name(&args),
        )),
        CatAction::Help => Err(CtSimpleError::new(0, cat_help_text(&cat_usage_name(&args)))),
        CatAction::Version => Err(CtSimpleError::new(
            0,
            cat_version_text(&cat_program_name(&args)),
        )),
    }
}

#[derive(Default)]
pub struct Cat;
impl Tool for Cat {
    fn name(&self) -> &'static str {
        "cat"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        // 将&[OsString]转换为符合Args trait要求的iterator
        cat_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;
    use std::ffi::OsString;
    use std::fs;
    use std::io::{self, Write};
    use std::io::{BufWriter, stdout};

    // 新增：测试 Tool trait 的基本实现
    #[test]
    fn test_tool_implementation() {
        let tool = Cat;

        // 测试 name 方法
        assert_eq!(tool.name(), "cat");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("cat"));

        // 测试 execute 方法
        let args = vec![OsString::from("cat"), OsString::from("--version")];
        assert!(tool.execute(&args).is_ok());
    }

    // Usage: target/debug/syskits cat [OPTION]... [FILE]...
    //
    // Options:
    // -A, --show-all          equivalent to -vET
    // -b, --number-nonblank   number nonempty output lines, overrides -n
    // -e                      equivalent to -vE
    // -E, --show-ends         display $ at end of each line
    // -n, --number            number all output lines
    // -s, --squeeze-blank     suppress repeated empty output lines
    // -t                      equivalent to -vT
    // -T, --show-tabs         display TAB characters at ^I
    // -v, --show-nonprinting  use ^ and M- notation, except for LF (\n) and TAB (\t)
    // -u                      (ignored)
    // -h, --help              Print help
    // -V, --version           Print version
    #[test]
    fn test_ct_app_execution_version() {
        let command = ct_app();

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), "--version"];

        // Assuming `command` has a method to retrieve the executable name, replace it with the actual one
        let executable = command.try_get_matches_from(args);

        assert!(executable.is_err());
        assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
    }

    #[test]
    fn test_ct_app_execution_other_version() {
        let command = ct_app();

        // GNU cat reserves only the long form for --version.
        let args = vec![ctcore::ct_util_name(), "-V"];

        let executable = command.try_get_matches_from(args);

        assert!(executable.is_err());
        assert_eq!(executable.unwrap_err().kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn test_ct_app_execution_help() {
        let command = ct_app();

        // 测试用例2：验证 --help 参数是否正确处理
        let help_args = vec![ctcore::ct_util_name(), "--help"];
        let result = command.try_get_matches_from(help_args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
    }

    #[test]
    fn test_gnu_help_and_version_text() {
        let help = match cat_native_semantic(
            [OsString::from("cat"), OsString::from("--help")].into_iter(),
        ) {
            Ok(_) => panic!("help should be represented as an exit-zero semantic error"),
            Err(err) => err,
        };
        assert_eq!(help.code(), 0);
        assert!(
            help.to_string()
                .starts_with("Usage: cat [OPTION]... [FILE]...\n")
        );
        assert!(
            help.to_string()
                .contains("      --help        display this help and exit")
        );
        assert!(!help.to_string().contains("-h, --help"));

        let version = match cat_native_semantic(
            [OsString::from("cat"), OsString::from("--version")].into_iter(),
        ) {
            Ok(_) => panic!("version should be represented as an exit-zero semantic error"),
            Err(err) => err,
        };
        assert_eq!(version.code(), 0);
        assert!(version.to_string().starts_with("cat (syskits) "));
    }

    #[test]
    fn test_cat_files_info_error_sets_exit_code() {
        ctcore::ct_error::set_ct_exit_code(0);

        let opts = CatOutputOptions {
            num_mode: CatNumberingMode::None,
            squeeze_blank: false,
            show_tabs: false,
            show_ends: false,
            show_non_print: false,
        };

        let files = vec![String::from("does-not-exist-for-cat-test")];
        let result = cat_files_info(&files, &opts);

        assert!(result.is_err());
        assert_eq!(ctcore::ct_error::get_ct_exit_code(), 1);

        ctcore::ct_error::set_ct_exit_code(0);
    }

    #[test]
    fn test_strip_errno_permission_denied() {
        let err = std::io::Error::from_raw_os_error(13);
        assert_eq!(strip_errno(&err), "Permission denied");
    }

    #[test]
    fn test_ct_app_execution_unsupport_help() {
        for short_option in ["-h", "-H"] {
            let help_args = vec![ctcore::ct_util_name(), short_option];
            let result = ct_app().try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }
    }

    #[test]
    fn test_write_nonprint_to_end_29() {
        let mut writer = BufWriter::with_capacity(1024 * 64, stdout());
        let in_buf = &[9u8];
        let tab = b"tab";
        super::cat_write_non_print_to_end(in_buf, &mut writer, tab).unwrap();
        assert_eq!(writer.buffer(), tab);
    }

    #[test]
    fn test_ct_app_invalid_argument() {
        let command = ct_app();

        // 测试用例3：验证当提供未知参数时是否正确报错
        let invalid_args = vec![ctcore::ct_util_name(), "--invalid-argument"];
        let result = command.try_get_matches_from(invalid_args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn test_gnu_option_error_diagnostics() {
        let cases = [
            (
                vec!["cat", "-h"],
                "invalid option -- 'h'\nTry 'cat --help' for more information.",
            ),
            (
                vec!["cat", "--invalid"],
                "unrecognized option '--invalid'\nTry 'cat --help' for more information.",
            ),
            (
                vec!["cat", "--show-tabs=yes"],
                "option '--show-tabs' doesn't allow an argument\nTry 'cat --help' for more information.",
            ),
            (
                vec!["cat", "--show"],
                "option '--show' is ambiguous; possibilities: '--show-nonprinting' '--show-ends' '--show-tabs' '--show-all'\nTry 'cat --help' for more information.",
            ),
        ];

        for (args, expected) in cases {
            let args = args.into_iter().map(OsString::from).collect::<Vec<_>>();
            let err = match parse_cat_args(&args) {
                Ok(_) => panic!("expected GNU-style option parse error"),
                Err(err) => err,
            };
            assert_eq!(err.code(), 1);
            assert_eq!(err.to_string(), expected);
        }
    }

    #[test]
    fn test_gnu_option_parser_continues_after_an_operand() {
        let args = ["cat", "file", "-n"]
            .into_iter()
            .map(OsString::from)
            .collect::<Vec<_>>();

        let CatAction::Run(options, files) =
            parse_cat_args_with_posixly_correct(&args, false).unwrap()
        else {
            panic!("expected a normal cat invocation");
        };

        assert_eq!(options.num_mode, CatNumberingMode::All);
        assert_eq!(files, vec![OsString::from("file")]);
    }

    #[test]
    fn test_posixly_correct_stops_option_parsing_at_the_first_operand() {
        let args = ["cat", "file", "-n"]
            .into_iter()
            .map(OsString::from)
            .collect::<Vec<_>>();

        let CatAction::Run(options, files) =
            parse_cat_args_with_posixly_correct(&args, true).unwrap()
        else {
            panic!("expected a normal cat invocation");
        };

        assert_eq!(options.num_mode, CatNumberingMode::None);
        assert_eq!(files, vec![OsString::from("file"), OsString::from("-n")]);
    }

    #[test]
    fn test_file_error_uses_the_invocation_name() {
        let temp_dir = tempdir().expect("tempdir");
        let missing = temp_dir.path().join("missing");

        let semantic = cat_native_semantic(
            [OsString::from("cat-custom"), missing.into_os_string()].into_iter(),
        )
        .expect("semantic");

        assert_eq!(semantic.exit_code, 1);
        assert!(semantic.stderr_text.starts_with("cat-custom: "));
    }

    #[test]
    fn test_non_utf8_file_name_is_opened_as_raw_bytes() {
        use std::os::unix::ffi::OsStringExt;

        let temp_dir = tempdir().expect("tempdir");
        let path = temp_dir
            .path()
            .join(OsString::from_vec(b"cat-non-utf8-\xff".to_vec()));
        fs::write(&path, b"payload\n").expect("write fixture");

        let semantic =
            cat_native_semantic([OsString::from("cat"), path.into_os_string()].into_iter())
                .expect("cat should open a non-UTF-8 path");

        assert_eq!(semantic.classic_text, "payload\n");
        assert_eq!(semantic.stderr_text, "");
        assert_eq!(semantic.exit_code, 0);
    }

    #[cfg(unix)]
    #[test]
    fn test_non_utf8_file_name_diagnostic_uses_gnu_shell_escaping() {
        use std::os::unix::ffi::OsStringExt;

        assert_eq!(
            cat_quote_non_utf8_path(b"cat-missing-\xff-tail"),
            "'cat-missing-'$'\\377''-tail'"
        );

        let temp_dir = tempdir().expect("tempdir");
        let path = temp_dir
            .path()
            .join(OsString::from_vec(b"cat-missing-\xff-tail".to_vec()));

        let semantic =
            cat_native_semantic([OsString::from("cat"), path.into_os_string()].into_iter())
                .expect("semantic");

        assert_eq!(semantic.exit_code, 1);
        assert!(semantic.stderr_text.contains("'$'\\377''"));
        assert!(!semantic.stderr_text.contains('\u{fffd}'));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_unix_socket_uses_the_gnu_open_error_path() {
        use std::os::unix::net::UnixListener;

        let temp_dir = tempdir().expect("tempdir");
        let socket_path = temp_dir.path().join("cat.sock");
        let _listener = UnixListener::bind(&socket_path).expect("bind socket");

        let semantic =
            cat_native_semantic([OsString::from("cat"), socket_path.into_os_string()].into_iter())
                .expect("semantic");

        assert_eq!(semantic.classic_text, "");
        assert_eq!(semantic.exit_code, 1);
        assert!(semantic.stderr_text.contains("No such device or address"));
    }

    #[test]
    fn test_show_tabs_without_a_trailing_newline() {
        let temp_dir = tempdir().expect("tempdir");
        let path = temp_dir.path().join("tab-without-newline");
        fs::write(&path, b"a\tb").expect("write fixture");

        let semantic = cat_native_semantic(
            [
                OsString::from("cat"),
                OsString::from("-T"),
                path.into_os_string(),
            ]
            .into_iter(),
        )
        .expect("cat -T should not panic for a final TAB-containing line");

        assert_eq!(semantic.classic_text, "a^Ib");
        assert_eq!(semantic.exit_code, 0);
    }

    #[test]
    fn test_ct_app_support_missing_argument() {
        let command = ct_app();

        // 测试用例4：验证当缺少必需的参数时是否正确报错
        let missing_args = vec![ctcore::ct_util_name()]; // 缺少任何参数
        let result = command.try_get_matches_from(missing_args);
        assert!(result.is_ok());
    }

    fn get_command() -> Command {
        ct_app()
    }

    #[test]
    fn test_options_file() {
        // 创建文件并写入内容
        fn base_create_file_with_content(filename: &str, content: &str) -> io::Result<()> {
            let mut file = File::create(filename)?;
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
            Ok(())
        }

        // 删除指定文件
        fn base_delete_file(filename: &str) -> io::Result<()> {
            fs::remove_file(filename)?;
            Ok(())
        }

        let filename = "test_options_file.txt";
        let content = "Test cat hello world";
        // let expected_output = "Test test_base_common_handle_input_encode_base16";
        // 创建文件并写入内容
        match base_create_file_with_content(filename, content) {
            Ok(_) => println!("File '{filename}' created successfully."),
            Err(e) => eprintln!("Error creating file: {e}"),
        }

        let command = get_command();

        let args = vec![ctcore::ct_util_name(), filename];
        let matches = command.try_get_matches_from(args);

        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{filename}' deleted successfully."),
            Err(e) => eprintln!("Error deleting file: {e}"),
        }
        // assert_eq!(s, expected_output);
        assert!(matches.is_ok());
    }

    #[test]
    fn test_options_show_all() {
        let command = get_command();

        let args = vec![ctcore::ct_util_name(), "-A"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::CAT_SHOW_ALL));
    }

    #[test]
    fn test_write_nonprint_to_end_new_line() {
        let mut writer = BufWriter::with_capacity(1024 * 64, stdout());
        let in_buf = b"\n";
        let tab = b"";
        super::cat_write_non_print_to_end(in_buf, &mut writer, tab).unwrap();
        assert_eq!(writer.buffer().len(), 0);
    }

    #[test]
    fn test_options_show_all_whole() {
        let command = get_command();

        let args = vec![ctcore::ct_util_name(), "--show-all"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::CAT_SHOW_ALL));
    }

    #[test]
    fn test_options_number_nonblank() {
        let command = get_command();

        let args = vec![ctcore::ct_util_name(), "-b"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::CAT_NUMBER_NO_NBLANK));
    }

    #[test]
    fn test_options_number_nonblankwhole() {
        let command = get_command();

        let args = vec![ctcore::ct_util_name(), "--number-nonblank"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::CAT_NUMBER_NO_NBLANK));
    }

    #[test]
    fn test_write_nonprint_to_end_0_to_8() {
        for byte in 0u8..=8u8 {
            let mut writer = BufWriter::with_capacity(1024 * 64, stdout());
            let in_buf = &[byte];
            let tab = b"";
            super::cat_write_non_print_to_end(in_buf, &mut writer, tab).unwrap();
            assert_eq!(writer.buffer(), [b'^', byte + 64]);
        }
    }

    #[test]
    fn test_options_show_nonprinting_ends() {
        let command = get_command();

        let args = vec![ctcore::ct_util_name(), "-e"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::CAT_SHOW_NON_PRINTING_ENDS));
    }

    #[test]
    fn test_options_show_ends() {
        let command = get_command();

        let args = vec![ctcore::ct_util_name(), "-E"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::CAT_SHOW_ENDS));
    }

    #[test]
    fn test_options_show_ends_whole() {
        let command = get_command();

        let args = vec![ctcore::ct_util_name(), "--show-ends"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::CAT_SHOW_ENDS));
    }

    #[test]
    fn test_options_number() {
        let command = get_command();

        let args = vec![ctcore::ct_util_name(), "-n"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::CAT_NUMBER));
    }

    #[test]
    fn test_options_number_whole() {
        let command = get_command();

        let args = vec![ctcore::ct_util_name(), "--number"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::CAT_NUMBER));
    }

    #[test]
    fn test_options_squeeze_blank() {
        let command = get_command();

        let args = vec![ctcore::ct_util_name(), "-s"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::CAT_SQUEEZE_BLANK));
    }

    #[test]
    fn test_options_squeeze_blank_whole() {
        let command = get_command();

        let args = vec![ctcore::ct_util_name(), "--squeeze-blank"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::CAT_SQUEEZE_BLANK));
    }

    #[test]
    fn test_options_show_nonprinting_tabs() {
        let command = get_command();

        let args = vec![ctcore::ct_util_name(), "-t"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::CAT_SHOW_NON_PRINTING_TABS));
    }

    #[test]
    fn test_options_show_tabs() {
        let command = get_command();

        let args = vec![ctcore::ct_util_name(), "-T"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::CAT_SHOW_TABS));
    }

    #[test]
    fn test_options_show_tabs_whole() {
        let command = get_command();

        let args = vec![ctcore::ct_util_name(), "--show-tabs"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::CAT_SHOW_TABS));
    }

    #[test]
    fn test_options_show_nonprinting() {
        let command = get_command();

        let args = vec![ctcore::ct_util_name(), "-v"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::CAT_SHOW_NON_PRINTING));
    }

    #[test]
    fn test_options_show_nonprinting_whole() {
        let command = get_command();

        let args = vec![ctcore::ct_util_name(), "--show-nonprinting"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::CAT_SHOW_NON_PRINTING));
    }

    #[test]
    fn test_options_ignored_u() {
        let command = get_command();

        let args = vec![ctcore::ct_util_name(), "-u"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(opt_flags::CAT_IGNORED_U));
    }

    #[test]
    fn test_repeated_options_are_accepted() {
        for args in [
            vec![ctcore::ct_util_name(), "-nn"],
            vec![ctcore::ct_util_name(), "--number", "--number"],
            vec![ctcore::ct_util_name(), "-AA"],
        ] {
            assert!(get_command().try_get_matches_from(args).is_ok());
        }
    }

    use tempfile::tempdir;

    // Test case: Test reading an empty file
    #[test]
    fn test_read_empty_file() {
        let temp_dir = tempdir().expect("Failed to create temporary directory");
        let temp_file_path = temp_dir.path().join("empty_file.txt");
        File::create(&temp_file_path).expect("Failed to create empty file");

        let files = vec![temp_file_path.to_string_lossy().into_owned()];
        let options = CatOutputOptions {
            num_mode: CatNumberingMode::None,
            squeeze_blank: false,
            show_tabs: false,
            show_ends: false,
            show_non_print: false,
        };
        assert!(cat_files_info(&files, &options).is_ok());

        temp_dir
            .close()
            .expect("Failed to remove temporary directory");
    }

    // Test case: Test reading multiple files with default options
    #[test]
    fn test_read_multiple_files_default_options() {
        let temp_dir = tempdir().expect("Failed to create temporary directory");
        let temp_file1_path = temp_dir.path().join("temp_file1.txt");
        let mut temp_file1 =
            File::create(&temp_file1_path).expect("Failed to create temporary file");
        temp_file1
            .write_all(b"File 1\n")
            .expect("Failed to write to temporary file");

        let temp_file2_path = temp_dir.path().join("temp_file2.txt");
        let mut temp_file2 =
            File::create(&temp_file2_path).expect("Failed to create temporary file");
        temp_file2
            .write_all(b"File 2\n")
            .expect("Failed to write to temporary file");

        let files = vec![
            temp_file1_path.to_string_lossy().into_owned(),
            temp_file2_path.to_string_lossy().into_owned(),
        ];
        let options = CatOutputOptions {
            num_mode: CatNumberingMode::None,
            squeeze_blank: false,
            show_tabs: false,
            show_ends: false,
            show_non_print: false,
        };
        assert!(cat_files_info(&files, &options).is_ok());

        temp_dir
            .close()
            .expect("Failed to remove temporary directory");
    }

    // Test case: Test reading a non-existent file
    #[test]
    fn test_read_nonexistent_file() {
        let files = vec!["nonexistent_file.txt".to_string()];
        let options = CatOutputOptions {
            num_mode: CatNumberingMode::None,
            squeeze_blank: false,
            show_tabs: false,
            show_ends: false,
            show_non_print: false,
        };
        assert!(cat_files_info(&files, &options).is_err());
    }

    // Test case: Test reading a single file with non-blank line numbering
    #[test]
    fn test_read_single_file_with_line_numbering() {
        let temp_dir = tempdir().expect("Failed to create temporary directory");
        let temp_file_path = temp_dir.path().join("temp_file.txt");
        let mut temp_file = File::create(&temp_file_path).expect("Failed to create temporary file");
        temp_file
            .write_all(b"Line 1\nLine 2\nLine 3\n")
            .expect("Failed to write to temporary file");

        let files = vec![temp_file_path.to_string_lossy().into_owned()];
        let options = CatOutputOptions {
            num_mode: CatNumberingMode::None,
            squeeze_blank: false,
            show_tabs: false,
            show_ends: false,
            show_non_print: false,
        };
        assert!(cat_files_info(&files, &options).is_ok());

        temp_dir
            .close()
            .expect("Failed to remove temporary directory");
    }

    // Test case: Test reading a single file with all lines numbered
    #[test]
    fn test_read_single_file_with_all_line_numbering() {
        let temp_dir = tempdir().expect("Failed to create temporary directory");
        let temp_file_path = temp_dir.path().join("temp_file.txt");
        let mut temp_file = File::create(&temp_file_path).expect("Failed to create temporary file");
        temp_file
            .write_all(b"Line 1\nLine 2\nLine 3\n")
            .expect("Failed to write to temporary file");

        let files = vec![temp_file_path.to_string_lossy().into_owned()];
        let options = CatOutputOptions {
            num_mode: CatNumberingMode::All,
            squeeze_blank: false,
            show_tabs: false,
            show_ends: false,
            show_non_print: false,
        };
        assert!(cat_files_info(&files, &options).is_ok());

        temp_dir
            .close()
            .expect("Failed to remove temporary directory");
    }

    #[test]
    fn test_write_nonprint_to_end_9() {
        let mut writer = BufWriter::with_capacity(1024 * 64, stdout());
        let in_buf = &[9u8];
        let tab = b"tab";
        super::cat_write_non_print_to_end(in_buf, &mut writer, tab).unwrap();
        assert_eq!(writer.buffer(), tab);
    }

    #[test]
    fn test_write_nonprint_to_end_10_to_31() {
        for byte in 11u8..=31u8 {
            let mut writer = BufWriter::with_capacity(1024 * 64, stdout());
            let in_buf = &[byte];
            let tab = b"";
            super::cat_write_non_print_to_end(in_buf, &mut writer, tab).unwrap();
            assert_eq!(writer.buffer(), [b'^', byte + 64]);
        }
    }

    // Test case: Test reading a single file and squeezing blank lines
    #[test]
    fn test_read_single_file_with_squeezing_blank_lines() {
        let temp_dir = tempdir().expect("Failed to create temporary directory");
        let temp_file_path = temp_dir.path().join("temp_file.txt");
        let mut temp_file = File::create(&temp_file_path).expect("Failed to create temporary file");
        temp_file
            .write_all(b"Line 1\n\nLine 2\n\nLine 3\n")
            .expect("Failed to write to temporary file");

        let files = vec![temp_file_path.to_string_lossy().into_owned()];
        let options = CatOutputOptions {
            num_mode: CatNumberingMode::None,
            squeeze_blank: true,
            show_tabs: false,
            show_ends: false,
            show_non_print: false,
        };
        assert!(cat_files_info(&files, &options).is_ok());

        temp_dir
            .close()
            .expect("Failed to remove temporary directory");
    }

    // Test case: Test reading a single file and showing tabs
    #[test]
    fn test_read_single_file_with_showing_tabs() {
        let temp_dir = tempdir().expect("Failed to create temporary directory");
        let temp_file_path = temp_dir.path().join("temp_file.txt");
        let mut temp_file = File::create(&temp_file_path).expect("Failed to create temporary file");
        temp_file
            .write_all(b"Line\twith\ttabs\n")
            .expect("Failed to write to temporary file");

        let files = vec![temp_file_path.to_string_lossy().into_owned()];
        let options = CatOutputOptions {
            num_mode: CatNumberingMode::None,
            squeeze_blank: false,
            show_tabs: true,
            show_ends: false,
            show_non_print: false,
        };
        assert!(cat_files_info(&files, &options).is_ok());

        temp_dir
            .close()
            .expect("Failed to remove temporary directory");
    }

    // Test case: Test reading a single file and showing ends
    #[test]
    fn test_read_single_file_with_showing_ends() {
        let temp_dir = tempdir().expect("Failed to create temporary directory");
        let temp_file_path = temp_dir.path().join("temp_file.txt");
        let mut temp_file = File::create(&temp_file_path).expect("Failed to create temporary file");
        temp_file
            .write_all(b"Line with newline at the end\n")
            .expect("Failed to write to temporary file");

        let files = vec![temp_file_path.to_string_lossy().into_owned()];
        let options = CatOutputOptions {
            num_mode: CatNumberingMode::None,
            squeeze_blank: false,
            show_tabs: false,
            show_ends: true,
            show_non_print: false,
        };
        assert!(cat_files_info(&files, &options).is_ok());

        temp_dir
            .close()
            .expect("Failed to remove temporary directory");
    }

    // Test case: Test reading a single file and showing non-printable characters
    #[test]
    fn test_read_single_file_with_showing_nonprintable_characters() {
        let temp_dir = tempdir().expect("Failed to create temporary directory");
        let temp_file_path = temp_dir.path().join("temp_file.txt");
        let mut temp_file = File::create(&temp_file_path).expect("Failed to create temporary file");
        temp_file
            .write_all(b"Line with\ttab and non-printable character\x01\n")
            .expect("Failed to write to temporary file");

        let files = vec![temp_file_path.to_string_lossy().into_owned()];
        let options = CatOutputOptions {
            num_mode: CatNumberingMode::None,
            squeeze_blank: false,
            show_tabs: false,
            show_ends: false,
            show_non_print: true,
        };
        assert!(cat_files_info(&files, &options).is_ok());

        temp_dir
            .close()
            .expect("Failed to remove temporary directory");
    }

    // Test case: Test reading multiple files with line numbering and showing tabs
    #[test]
    fn test_read_multiple_files_with_line_numbering_and_showing_tabs() {
        let temp_dir = tempdir().expect("Failed to create temporary directory");

        // Create first temporary file
        let temp_file1_path = temp_dir.path().join("temp_file1.txt");
        let mut temp_file1 =
            File::create(&temp_file1_path).expect("Failed to create temporary file");
        temp_file1
            .write_all(b"Line 1\nLine 2\nLine 3\n")
            .expect("Failed to write to temporary file");

        // Create second temporary file
        let temp_file2_path = temp_dir.path().join("temp_file2.txt");
        let mut temp_file2 =
            File::create(&temp_file2_path).expect("Failed to create temporary file");
        temp_file2
            .write_all(b"Line 4\nLine 5\nLine 6\n")
            .expect("Failed to write to temporary file");

        let files = vec![
            temp_file1_path.to_string_lossy().into_owned(),
            temp_file2_path.to_string_lossy().into_owned(),
        ];
        let options = CatOutputOptions {
            num_mode: CatNumberingMode::All,
            squeeze_blank: false,
            show_tabs: true,
            show_ends: false,
            show_non_print: false,
        };
        assert!(cat_files_info(&files, &options).is_ok());

        temp_dir
            .close()
            .expect("Failed to remove temporary directory");
    }

    // Test case: Test reading a non-existent file
    #[test]
    fn test_read_non_existent_file() {
        let files = vec!["non_existent_file.txt".to_string()];
        let options = CatOutputOptions {
            num_mode: CatNumberingMode::None,
            squeeze_blank: false,
            show_tabs: false,
            show_ends: false,
            show_non_print: false,
        };
        assert!(cat_files_info(&files, &options).is_err());
    }

    // Test case: Test reading multiple files with squeezing blank lines and showing ends
    #[test]
    fn test_read_multiple_files_with_squeezing_blank_lines_and_showing_ends() {
        let temp_dir = tempdir().expect("Failed to create temporary directory");

        // Create first temporary file
        let temp_file1_path = temp_dir.path().join("temp_file1.txt");
        let mut temp_file1 =
            File::create(&temp_file1_path).expect("Failed to create temporary file");
        temp_file1
            .write_all(b"Line 1\n\n\nLine 2\n\n\n")
            .expect("Failed to write to temporary file");

        // Create second temporary file
        let temp_file2_path = temp_dir.path().join("temp_file2.txt");
        let mut temp_file2 =
            File::create(&temp_file2_path).expect("Failed to create temporary file");
        temp_file2
            .write_all(b"\n\nLine 3\n\n\nLine 4\n")
            .expect("Failed to write to temporary file");

        let files = vec![
            temp_file1_path.to_string_lossy().into_owned(),
            temp_file2_path.to_string_lossy().into_owned(),
        ];
        let options = CatOutputOptions {
            num_mode: CatNumberingMode::None,
            squeeze_blank: true,
            show_tabs: false,
            show_ends: true,
            show_non_print: false,
        };
        assert!(cat_files_info(&files, &options).is_ok());

        temp_dir
            .close()
            .expect("Failed to remove temporary directory");
    }

    // Test case: Test reading multiple files with line numbering and showing ends
    #[test]
    fn test_read_multiple_files_with_line_numbering_and_showing_ends() {
        let temp_dir = tempdir().expect("Failed to create temporary directory");

        // Create first temporary file
        let temp_file1_path = temp_dir.path().join("temp_file1.txt");
        let mut temp_file1 =
            File::create(&temp_file1_path).expect("Failed to create temporary file");
        temp_file1
            .write_all(b"Line 1\nLine 2\nLine 3\n")
            .expect("Failed to write to temporary file");

        // Create second temporary file
        let temp_file2_path = temp_dir.path().join("temp_file2.txt");
        let mut temp_file2 =
            File::create(&temp_file2_path).expect("Failed to create temporary file");
        temp_file2
            .write_all(b"Line 4\nLine 5\nLine 6\n")
            .expect("Failed to write to temporary file");

        let files = vec![
            temp_file1_path.to_string_lossy().into_owned(),
            temp_file2_path.to_string_lossy().into_owned(),
        ];
        let options = CatOutputOptions {
            num_mode: CatNumberingMode::All,
            squeeze_blank: false,
            show_tabs: false,
            show_ends: true,
            show_non_print: false,
        };
        assert!(cat_files_info(&files, &options).is_ok());

        temp_dir
            .close()
            .expect("Failed to remove temporary directory");
    }

    // Test case: Test reading multiple files with showing non-printable characters
    #[test]
    fn test_read_multiple_files_with_showing_nonprintable_characters() {
        let temp_dir = tempdir().expect("Failed to create temporary directory");

        // Create first temporary file
        let temp_file1_path = temp_dir.path().join("temp_file1.txt");
        let mut temp_file1 =
            File::create(&temp_file1_path).expect("Failed to create temporary file");
        temp_file1
            .write_all(b"Line 1\nLine \x07 2\nLine 3\n")
            .expect("Failed to write to temporary file");

        // Create second temporary file
        let temp_file2_path = temp_dir.path().join("temp_file2.txt");
        let mut temp_file2 =
            File::create(&temp_file2_path).expect("Failed to create temporary file");
        temp_file2
            .write_all(b"Line 4\nLine 5\x0b\nLine 6\n")
            .expect("Failed to write to temporary file");

        let files = vec![
            temp_file1_path.to_string_lossy().into_owned(),
            temp_file2_path.to_string_lossy().into_owned(),
        ];
        let options = CatOutputOptions {
            num_mode: CatNumberingMode::None,
            squeeze_blank: false,
            show_tabs: false,
            show_ends: false,
            show_non_print: true,
        };
        assert!(cat_files_info(&files, &options).is_ok());

        temp_dir
            .close()
            .expect("Failed to remove temporary directory");
    }

    // Test case: Test reading multiple files with squeezing blank lines and showing tabs and ends
    #[test]
    fn test_read_tabs_and_ends() {
        let temp_dir = tempdir().expect("Failed to create temporary directory");

        // Create first temporary file
        let temp_file1_path = temp_dir.path().join("temp_file1.txt");
        let mut temp_file1 =
            File::create(&temp_file1_path).expect("Failed to create temporary file");
        temp_file1
            .write_all(b"Line 1\n\n\nLine 2\t\t\n\n\n")
            .expect("Failed to write to temporary file");

        // Create second temporary file
        let temp_file2_path = temp_dir.path().join("temp_file2.txt");
        let mut temp_file2 =
            File::create(&temp_file2_path).expect("Failed to create temporary file");
        temp_file2
            .write_all(b"\n\nLine 3\t\t\n\n\nLine 4\n")
            .expect("Failed to write to temporary file");

        let files = vec![
            temp_file1_path.to_string_lossy().into_owned(),
            temp_file2_path.to_string_lossy().into_owned(),
        ];
        let options = CatOutputOptions {
            num_mode: CatNumberingMode::None,
            squeeze_blank: true,
            show_tabs: true,
            show_ends: true,
            show_non_print: false,
        };
        assert!(cat_files_info(&files, &options).is_ok());

        temp_dir
            .close()
            .expect("Failed to remove temporary directory");
    }

    // Test case: Test reading a file with only whitespace characters
    #[test]
    fn test_read_file_with_whitespace() {
        let temp_dir = tempdir().expect("Failed to create temporary directory");
        let temp_file_path = temp_dir.path().join("whitespace_file.txt");
        let mut temp_file = File::create(&temp_file_path).expect("Failed to create temporary file");
        temp_file
            .write_all(b"   \n\t   \n  ")
            .expect("Failed to write to temporary file");

        let files = vec![temp_file_path.to_string_lossy().into_owned()];
        let options = CatOutputOptions {
            num_mode: CatNumberingMode::None,
            squeeze_blank: false,
            show_tabs: false,
            show_ends: false,
            show_non_print: false,
        };
        assert!(cat_files_info(&files, &options).is_ok());

        temp_dir
            .close()
            .expect("Failed to remove temporary directory");
    }

    // Test case: Test reading a file and displaying non-printable characters
    #[test]
    fn test_read_file_with_non_printable_characters() {
        let temp_dir = tempdir().expect("Failed to create temporary directory");
        let temp_file_path = temp_dir.path().join("non_printable_file.txt");
        let mut temp_file = File::create(&temp_file_path).expect("Failed to create temporary file");
        temp_file
            .write_all(b"Non-printable\x01\x02\x03Characters")
            .expect("Failed to write to temporary file");

        let files = vec![temp_file_path.to_string_lossy().into_owned()];
        let options = CatOutputOptions {
            num_mode: CatNumberingMode::None,
            squeeze_blank: false,
            show_tabs: false,
            show_ends: false,
            show_non_print: true,
        };
        assert!(cat_files_info(&files, &options).is_ok());

        temp_dir
            .close()
            .expect("Failed to remove temporary directory");
    }

    // Test case: Test reading multiple files and concatenating them
    #[test]
    fn test_read_multiple_files() {
        let temp_dir = tempdir().expect("Failed to create temporary directory");
        let file1_path = temp_dir.path().join("file1.txt");
        let file2_path = temp_dir.path().join("file2.txt");
        let mut file1 = File::create(&file1_path).expect("Failed to create temporary file");
        let mut file2 = File::create(&file2_path).expect("Failed to create temporary file");
        file1
            .write_all(b"Content from file1")
            .expect("Failed to write to temporary file");
        file2
            .write_all(b"Content from file2")
            .expect("Failed to write to temporary file");

        let files = vec![
            file1_path.to_string_lossy().into_owned(),
            file2_path.to_string_lossy().into_owned(),
        ];
        let options = CatOutputOptions {
            num_mode: CatNumberingMode::None,
            squeeze_blank: false,
            show_tabs: false,
            show_ends: false,
            show_non_print: false,
        };
        assert!(cat_files_info(&files, &options).is_ok());

        temp_dir
            .close()
            .expect("Failed to remove temporary directory");
    }

    // Test case: Test reading a large file
    #[test]
    fn test_read_large_file() {
        let temp_dir = tempdir().expect("Failed to create temporary directory");
        let temp_file_path = temp_dir.path().join("large_file.txt");
        let content = "A".repeat(10_000); // Create a large string with 10,000 'A's
        let mut temp_file = File::create(&temp_file_path).expect("Failed to create temporary file");
        temp_file
            .write_all(content.as_bytes())
            .expect("Failed to write to temporary file");

        let files = vec![temp_file_path.to_string_lossy().into_owned()];
        let options = CatOutputOptions {
            num_mode: CatNumberingMode::None,
            squeeze_blank: false,
            show_tabs: false,
            show_ends: false,
            show_non_print: false,
        };
        assert!(cat_files_info(&files, &options).is_ok());

        temp_dir
            .close()
            .expect("Failed to remove temporary directory");
    }

    #[test]
    fn test_read_file_with_tabs_and_newlines() {
        let temp_dir = tempdir().expect("Failed to create temporary directory");
        let temp_file_path = temp_dir.path().join("tab_and_newline_file.txt");
        let mut temp_file = File::create(&temp_file_path).expect("Failed to create temporary file");
        temp_file
            .write_all(b"Line 1\tLine 2\nLine 3\tLine 4\n")
            .expect("Failed to write to temporary file");

        let files = vec![temp_file_path.to_string_lossy().into_owned()];
        let options = CatOutputOptions {
            num_mode: CatNumberingMode::None,
            squeeze_blank: false,
            show_tabs: true,
            show_ends: false,
            show_non_print: false,
        };
        assert!(cat_files_info(&files, &options).is_ok());

        temp_dir
            .close()
            .expect("Failed to remove temporary directory");
    }

    #[test]
    fn test_read_nonex_file() {
        let files = vec!["nonexistent_file.txt".to_string()];
        let options = CatOutputOptions {
            num_mode: CatNumberingMode::None,
            squeeze_blank: false,
            show_tabs: false,
            show_ends: false,
            show_non_print: false,
        };
        assert!(cat_files_info(&files, &options).is_err());
    }

    #[test]
    fn test_read_file_with_special_characters() {
        let temp_dir = tempdir().expect("Failed to create temporary directory");
        let temp_file_path = temp_dir.path().join("special_characters_file.txt");
        let mut temp_file = File::create(&temp_file_path).expect("Failed to create temporary file");
        temp_file
            .write_all("Special characters: ❤️🌟$#@!%".as_bytes())
            .expect("Failed to write to temporary file");

        let files = vec![temp_file_path.to_string_lossy().into_owned()];
        let options = CatOutputOptions {
            num_mode: CatNumberingMode::None,
            squeeze_blank: false,
            show_tabs: false,
            show_ends: false,
            show_non_print: true,
        };
        assert!(cat_files_info(&files, &options).is_ok());

        temp_dir
            .close()
            .expect("Failed to remove temporary directory");
    }

    // 测试读取超长文件名的文件：
    #[test]
    fn test_read_file_with_long_filename() {
        let temp_dir = tempdir().expect("Failed to create temporary directory");
        let long_filename = "a".repeat(220); // 255字符的文件名
        let temp_file_path = temp_dir.path().join(format!("{long_filename}.txt"));
        let mut temp_file = File::create(&temp_file_path).expect("Failed to create temporary file");
        temp_file
            .write_all(b"Content with long filename")
            .expect("Failed to write to temporary file");

        let files = vec![temp_file_path.to_string_lossy().into_owned()];
        let options = CatOutputOptions {
            num_mode: CatNumberingMode::None,
            squeeze_blank: false,
            show_tabs: false,
            show_ends: false,
            show_non_print: false,
        };
        assert!(cat_files_info(&files, &options).is_ok());

        temp_dir
            .close()
            .expect("Failed to remove temporary directory");
    }

    //测试同时读取多个空文件：
    #[test]
    fn test_read_multiple_empty_files() {
        let temp_dir = tempdir().expect("Failed to create temporary directory");
        let file1_path = temp_dir.path().join("empty_file1.txt");
        let file2_path = temp_dir.path().join("empty_file2.txt");
        File::create(&file1_path).expect("Failed to create temporary file");
        File::create(&file2_path).expect("Failed to create temporary file");

        let files = vec![
            file1_path.to_string_lossy().into_owned(),
            file2_path.to_string_lossy().into_owned(),
        ];
        let options = CatOutputOptions {
            num_mode: CatNumberingMode::None,
            squeeze_blank: false,
            show_tabs: false,
            show_ends: false,
            show_non_print: false,
        };
        assert!(cat_files_info(&files, &options).is_ok());

        temp_dir
            .close()
            .expect("Failed to remove temporary directory");
    }

    #[test]
    fn test_read_file_with_permissions_denied() {
        let temp_dir = tempdir().expect("Failed to create temporary directory");
        let temp_file_path = temp_dir
            .path()
            .join("test_read_file_with_permissions_denied.txt");
        let mut temp_file = File::create(&temp_file_path).expect("Failed to create temporary file");
        temp_file
            .write_all(b"Content with restricted permissions")
            .expect("Failed to write to temporary file");

        // 模拟设置文件权限为只读
        let metadata = temp_file.metadata().expect("Failed to get file metadata");
        let mut permissions = metadata.permissions();
        permissions.set_readonly(true);
        temp_file
            .set_permissions(permissions)
            .expect("Failed to set file permissions");

        let files = vec![temp_file_path.to_string_lossy().into_owned()];
        let options = CatOutputOptions {
            num_mode: CatNumberingMode::None,
            squeeze_blank: false,
            show_tabs: false,
            show_ends: false,
            show_non_print: false,
        };
        println!("{:?}", cat_files_info(&files, &options).is_err());
        // assert!(cat_files(&files, &options).is_err());

        temp_dir
            .close()
            .expect("Failed to remove temporary directory");
    }

    // Test case: Test squeeze_blank with \r\n sequences
    #[test]
    fn test_squeeze_blank_with_carriage_return_newline() {
        let temp_dir = tempdir().expect("Failed to create temporary directory");
        let temp_file_path = temp_dir.path().join("test_crlf.txt");
        let mut temp_file = File::create(&temp_file_path).expect("Failed to create temporary file");

        // Create test content: "1234\n\O\r\n\n"
        temp_file
            .write_all(b"1234\n")
            .expect("Failed to write to temporary file");
        temp_file
            .write_all(b"\\O\r\n")
            .expect("Failed to write to temporary file");
        temp_file
            .write_all(b"\n")
            .expect("Failed to write to temporary file");

        let files = vec![temp_file_path.to_string_lossy().into_owned()];
        let options = CatOutputOptions {
            num_mode: CatNumberingMode::None,
            squeeze_blank: true,
            show_tabs: false,
            show_ends: false,
            show_non_print: false,
        };
        assert!(cat_files_info(&files, &options).is_ok());

        temp_dir
            .close()
            .expect("Failed to remove temporary directory");
    }

    // Test case: Test that \r\n sequences are handled correctly with squeeze_blank
    #[test]
    fn test_squeeze_blank_preserves_crlf_sequences() {
        let temp_dir = tempdir().expect("Failed to create temporary directory");
        let temp_file_path = temp_dir.path().join("test_crlf_preserve.txt");
        let mut temp_file = File::create(&temp_file_path).expect("Failed to create temporary file");

        // Test content with multiple \r\n sequences and blank lines
        temp_file
            .write_all(b"line1\r\n")
            .expect("Failed to write to temporary file");
        temp_file
            .write_all(b"\r\n")
            .expect("Failed to write to temporary file");
        temp_file
            .write_all(b"\r\n")
            .expect("Failed to write to temporary file");
        temp_file
            .write_all(b"line2\r\n")
            .expect("Failed to write to temporary file");

        let files = vec![temp_file_path.to_string_lossy().into_owned()];
        let options = CatOutputOptions {
            num_mode: CatNumberingMode::None,
            squeeze_blank: true,
            show_tabs: false,
            show_ends: false,
            show_non_print: false,
        };
        assert!(cat_files_info(&files, &options).is_ok());

        temp_dir
            .close()
            .expect("Failed to remove temporary directory");
    }

    // Test case: Test mixed \n and \r\n with squeeze_blank
    #[test]
    fn test_squeeze_blank_mixed_line_endings() {
        let temp_dir = tempdir().expect("Failed to create temporary directory");
        let temp_file_path = temp_dir.path().join("test_mixed_endings.txt");
        let mut temp_file = File::create(&temp_file_path).expect("Failed to create temporary file");

        // Mixed line endings: Unix \n and Windows \r\n
        temp_file
            .write_all(b"unix\n")
            .expect("Failed to write to temporary file");
        temp_file
            .write_all(b"\n")
            .expect("Failed to write to temporary file");
        temp_file
            .write_all(b"windows\r\n")
            .expect("Failed to write to temporary file");
        temp_file
            .write_all(b"\r\n")
            .expect("Failed to write to temporary file");
        temp_file
            .write_all(b"end\n")
            .expect("Failed to write to temporary file");

        let files = vec![temp_file_path.to_string_lossy().into_owned()];
        let options = CatOutputOptions {
            num_mode: CatNumberingMode::None,
            squeeze_blank: true,
            show_tabs: false,
            show_ends: false,
            show_non_print: false,
        };
        assert!(cat_files_info(&files, &options).is_ok());

        temp_dir
            .close()
            .expect("Failed to remove temporary directory");
    }

    // Test case: Test squeeze_blank doesn't affect non-blank lines with \r
    #[test]
    fn test_squeeze_blank_with_carriage_return_only() {
        let temp_dir = tempdir().expect("Failed to create temporary directory");
        let temp_file_path = temp_dir.path().join("test_cr_only.txt");
        let mut temp_file = File::create(&temp_file_path).expect("Failed to create temporary file");

        // Content with standalone \r characters
        temp_file
            .write_all(b"line1\r")
            .expect("Failed to write to temporary file");
        temp_file
            .write_all(b"overwritten\n")
            .expect("Failed to write to temporary file");
        temp_file
            .write_all(b"\n")
            .expect("Failed to write to temporary file");
        temp_file
            .write_all(b"\n")
            .expect("Failed to write to temporary file");
        temp_file
            .write_all(b"line2\n")
            .expect("Failed to write to temporary file");

        let files = vec![temp_file_path.to_string_lossy().into_owned()];
        let options = CatOutputOptions {
            num_mode: CatNumberingMode::None,
            squeeze_blank: true,
            show_tabs: false,
            show_ends: false,
            show_non_print: false,
        };
        assert!(cat_files_info(&files, &options).is_ok());

        temp_dir
            .close()
            .expect("Failed to remove temporary directory");
    }

    mod native_semantic_tests {
        use super::{CatRow, cat_native_semantic};
        use std::ffi::OsString;
        use tempfile::tempdir;

        #[test]
        fn test_cat_native_semantic_collects_rows_and_metadata() {
            let temp_dir = tempdir().expect("tempdir");
            let path = temp_dir.path().join("cat-semantic.txt");
            std::fs::write(&path, "alpha\tbeta\n\ngamma\n").expect("write cat semantic fixture");

            let semantic = cat_native_semantic(
                [
                    OsString::from("cat"),
                    OsString::from(path.display().to_string()),
                ]
                .into_iter(),
            )
            .expect("semantic");

            assert_eq!(semantic.number_mode, "none");
            assert!(!semantic.squeeze_blank);
            assert!(!semantic.show_tabs);
            assert_eq!(
                semantic.rows,
                vec![
                    CatRow {
                        row_index: 1,
                        line: "alpha\tbeta".into(),
                        is_blank: false,
                    },
                    CatRow {
                        row_index: 2,
                        line: String::new(),
                        is_blank: true,
                    },
                    CatRow {
                        row_index: 3,
                        line: "gamma".into(),
                        is_blank: false,
                    },
                ]
            );
            assert_eq!(semantic.classic_text, "alpha\tbeta\n\ngamma\n");
            assert_eq!(semantic.stderr_text, "");
            assert_eq!(semantic.exit_code, 0);
        }

        #[test]
        fn test_cat_native_semantic_preserves_missing_file_error() {
            let temp_dir = tempdir().expect("tempdir");
            let missing = temp_dir.path().join("missing.txt");

            let semantic = cat_native_semantic(
                [
                    OsString::from("cat"),
                    OsString::from(missing.display().to_string()),
                ]
                .into_iter(),
            )
            .expect("semantic");

            assert!(semantic.rows.is_empty());
            assert_eq!(semantic.classic_text, "");
            assert!(semantic.stderr_text.contains("No such file or directory"));
            assert_eq!(semantic.exit_code, 1);
        }
    }

    // #[test]
    // fn test_read_more_large_file() {
    //     let temp_dir = tempdir().expect("Failed to create temporary directory");
    //     let temp_file_path = temp_dir.path().join("large_file.txt");
    //     let mut temp_file = File::create(&temp_file_path).expect("Failed to create temporary file");
    //
    //     // 写入1GB的数据，模拟非常大的文件
    //     const GB: usize = 1024 * 1024 * 1024;      //这里是边界测试，单元测试时使用打开
    //     const GB: usize = 1;
    //     let data: Vec<u8> = vec![0; GB];
    //     temp_file
    //         .write_all(&data)
    //         .expect("Failed to write to temporary file");
    //
    //     let files = vec![temp_file_path.to_string_lossy().into_owned()];
    //     let options = OutputOptions {
    //         number: NumberingMode::None,
    //         squeeze_blank: false,
    //         show_tabs: false,
    //         show_ends: false,
    //         show_nonprint: false,
    //     };
    //     assert!(cat_files(&files, &options).is_err());
    //
    //     temp_dir
    //         .close()
    //         .expect("Failed to remove temporary directory");
    // }
}
