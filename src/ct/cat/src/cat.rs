/*
 *    Copyright(c) 2022-2024 China Telecom Cloud Technologies co., Ltd. All rights reserved
 *     syskits is licensed under Mulan PSL v2.
 *    You can use this software according to the terms and conditions of the Mulan PSL V2
 *    You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 *    THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 *    KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 *    NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 *    See the Mulan PSL v2 for more details.
 *
 */

use clap::crate_version;
use clap::Arg;
use clap::ArgAction;
use clap::Command;

use ctcore::ct_display::Quotable;
use ctcore::ct_error::CTResult;
use ctcore::ct_fs::CtFileInformation;
use std::fs::metadata;
use std::fs::File;
use std::io::{self, IsTerminal, Read, Write};
use thiserror::Error;

#[cfg(unix)]
use std::os::unix::io::AsRawFd;

/// Linux splice support
#[cfg(any(target_os = "linux", target_os = "android"))]
mod splice;

use ctcore::{ct_format_usage, ct_help_about, ct_help_usage};
/// Unix domain socket support
#[cfg(unix)]
use std::net::Shutdown;
#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;
#[cfg(unix)]
use std::os::unix::net::UnixStream;

const CAT_USAGE: &str = ct_help_usage!("cat.md");
const CAT_ABOUT: &str = ct_help_about!("cat.md");

#[derive(Error, Debug)]
enum CatError {
    /// Wrapper around `io::Error`
    #[error("{0}")]
    Io(#[from] io::Error),
    /// Wrapper around `nix::Error`
    #[cfg(any(target_os = "linux", target_os = "android"))]
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

#[derive(PartialEq)]
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
        if self.show_tabs {
            "^I"
        } else {
            "\t"
        }
    }

    fn cat_end_of_line(&self) -> &'static str {
        if self.show_ends {
            "$\n"
        } else {
            "\n"
        }
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

#[cfg(unix)]
trait CatFdReadable: Read + AsRawFd {}
#[cfg(not(unix))]
trait CatFdReadable: Read {}

#[cfg(unix)]
impl<T> CatFdReadable for T where T: Read + AsRawFd {}
#[cfg(not(unix))]
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

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    cat_main(args).map(|_| ())
}

pub fn cat_main(args: impl ctcore::Args) -> CTResult<()> {
    let args_match = ct_app().try_get_matches_from(args)?;

    let cat_num_mode = if args_match.get_flag(opt_flags::CAT_NUMBER_NO_NBLANK) {
        CatNumberingMode::NonEmpty
    } else if args_match.get_flag(opt_flags::CAT_NUMBER) {
        CatNumberingMode::All
    } else {
        CatNumberingMode::None
    };

    let cat_show_non_print = [
        opt_flags::CAT_SHOW_ALL.to_owned(),
        opt_flags::CAT_SHOW_NON_PRINTING_ENDS.to_owned(),
        opt_flags::CAT_SHOW_NON_PRINTING_TABS.to_owned(),
        opt_flags::CAT_SHOW_NON_PRINTING.to_owned(),
    ]
    .iter()
    .any(|v| args_match.get_flag(v));

    let cat_show_ends = [
        opt_flags::CAT_SHOW_ENDS.to_owned(),
        opt_flags::CAT_SHOW_ALL.to_owned(),
        opt_flags::CAT_SHOW_NON_PRINTING_ENDS.to_owned(),
    ]
    .iter()
    .any(|v| args_match.get_flag(v));

    let cat_show_tabs = [
        opt_flags::CAT_SHOW_ALL.to_owned(),
        opt_flags::CAT_SHOW_TABS.to_owned(),
        opt_flags::CAT_SHOW_NON_PRINTING_TABS.to_owned(),
    ]
    .iter()
    .any(|v| args_match.get_flag(v));

    let cat_squeeze_blank_status = args_match.get_flag(opt_flags::CAT_SQUEEZE_BLANK);
    let cat_files: Vec<String> = match args_match.get_many::<String>(opt_flags::CAT_FILE) {
        Some(v) => v.cloned().collect(),
        None => vec!["-".to_owned()],
    };

    let options = CatOutputOptions {
        show_ends: cat_show_ends,
        num_mode: cat_num_mode,
        show_non_print: cat_show_non_print,
        show_tabs: cat_show_tabs,
        squeeze_blank: cat_squeeze_blank_status,
    };
    cat_files_info(&cat_files, &options)
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = CAT_ABOUT;
    let usage_description = ct_format_usage(CAT_USAGE);

    let args = args_init();
    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
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
            .help("equivalent to -vET")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::CAT_NUMBER_NO_NBLANK)
            .short('b')
            .long(opt_flags::CAT_NUMBER_NO_NBLANK)
            .help("number nonempty output lines, overrides -n")
            // 注意：这绝对不能overrides_with(options::NUMBER)！在clap中，覆盖操作是对称的，
            // 因此“-b -n”被视为“-n”，这不是我们想要的。
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::CAT_SHOW_NON_PRINTING_ENDS)
            .short('e')
            .help("equivalent to -vE")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::CAT_SHOW_ENDS)
            .short('E')
            .long(opt_flags::CAT_SHOW_ENDS)
            .help("display $ at end of each line")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::CAT_NUMBER)
            .short('n')
            .long(opt_flags::CAT_NUMBER)
            .help("number all output lines")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::CAT_SQUEEZE_BLANK)
            .short('s')
            .long(opt_flags::CAT_SQUEEZE_BLANK)
            .help("suppress repeated empty output lines")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::CAT_SHOW_NON_PRINTING_TABS)
            .short('t')
            .help("equivalent to -vT")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::CAT_SHOW_TABS)
            .short('T')
            .long(opt_flags::CAT_SHOW_TABS)
            .help("display TAB characters at ^I")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::CAT_SHOW_NON_PRINTING)
            .short('v')
            .long(opt_flags::CAT_SHOW_NON_PRINTING)
            .help("use ^ and M- notation, except for LF (\\n) and TAB (\\t)")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::CAT_IGNORED_U)
            .short('u')
            .help("(ignored)")
            .action(ArgAction::SetTrue),
    ];
    args
}

fn cat_handle<R: CatFdReadable>(
    input_handle: &mut CatInputHandle<R>,
    output_options: &CatOutputOptions,
    output_state: &mut CatOutputState,
) -> CatResult<()> {
    if output_options.cat_can_write_fast() {
        cat_write_fast(input_handle)
    } else {
        cat_write_lines(input_handle, output_options, output_state)
    }
}

/**
 * 处理给定路径的文件或目录，并根据选项和状态输出内容。
 *
 */
fn cat_path(
    input_path: &str,
    output_options: &CatOutputOptions,
    output_state: &mut CatOutputState,
    out_info: Option<&CtFileInformation>,
) -> CatResult<()> {
    // 根据路径决定输入类型，并据此处理输入
    match cat_get_input_type(input_path)? {
        CatInputType::StdIn => {
            // 处理标准输入
            let stdin = io::stdin();
            let mut handle = CatInputHandle {
                reader: stdin,
                is_interactive: std::io::stdin().is_terminal(),
            };
            cat_handle(&mut handle, output_options, output_state)
        }
        CatInputType::Directory => Err(CatError::IsDirectory), // 目录不是有效的输入
        #[cfg(unix)]
        CatInputType::Socket => {
            // 连接到Unix套接字并处理输入
            let socket = UnixStream::connect(input_path)?;
            socket.shutdown(Shutdown::Write)?;
            let mut handle = CatInputHandle {
                reader: socket,
                is_interactive: false,
            };
            cat_handle(&mut handle, output_options, output_state)
        }
        _ => {
            // 处理普通文件输入
            let file = File::open(input_path)?;

            // 如果提供了输出信息且尝试将输出重定向到输入，返回错误
            if let Some(out_info) = out_info {
                if out_info.file_size() != 0
                    && CtFileInformation::from_file(&file).ok().as_ref() == Some(out_info)
                {
                    return Err(CatError::OutputIsInput);
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

/**
 * 合并文件信息。
 *
 * 此函数遍历给定的文件路径列表，尝试合并这些文件的内容到标准输出中。
 * 它处理各种输出选项，并在遇到错误时收集错误信息，最后返回成功或失败的结果。
 */
fn cat_files_info(input_files: &[String], output_options: &CatOutputOptions) -> CTResult<()> {
    // 尝试从标准输出创建文件信息，可能失败（例如没有权限）。
    let output_info = CtFileInformation::from_file(&std::io::stdout()).ok();

    // 初始化输出状态，用于跟踪输出过程中的状态，如行号、是否在行首等。
    let mut output_state = CatOutputState {
        line_number: 1,
        at_line_start: true,
        skipped_carriage_return: false,
        one_blank_kept: false,
    };
    // 用于收集处理过程中发生的错误信息。
    let mut error_msg: Vec<String> = Vec::new();

    // 遍历每个文件路径，尝试合并其内容。
    for file_path in input_files {
        if let Err(err) = cat_path(
            file_path,
            output_options,
            &mut output_state,
            output_info.as_ref(),
        ) {
            // 如果处理某个文件时发生错误，将错误信息收集到error_msg中。
            error_msg.push(format!("{}: {}", file_path.maybe_quote(), err));
        }
    }

    // 如果在处理过程中遇到回车符而没有紧接着输出，这里输出回车，以确保输出位置正确。
    if output_state.skipped_carriage_return {
        print!("\r");
    }

    // 根据是否收集到错误信息，决定是返回成功还是失败的结果。
    if error_msg.is_empty() {
        Ok(())
    } else {
        // 如果有错误信息，将它们格式化后作为错误返回。
        // 错误信息将以 "cat: 文件路径: 错误信息" 的形式呈现。
        let line_joiner = format!("\n{}: ", ctcore::ct_util_name());

        Err(ctcore::ct_error::CtSimpleError::new(
            error_msg.len() as i32,
            error_msg.join(&line_joiner),
        ))
    }
}

/// Classifies the `InputType` of file at `path` if possible
///
/// # Arguments
///
/// * `path` - Path on a file system to classify metadata
fn cat_get_input_type(input_path: &str) -> CatResult<CatInputType> {
    if input_path == "-" {
        return Ok(CatInputType::StdIn);
    }

    let file_type = match metadata(input_path) {
        Ok(metadata) => metadata.file_type(),
        Err(e) => {
            if let Some(raw_error_msg) = e.raw_os_error() {
                // 在类Unix系统中，“符号链接层数过多”的错误代码为40（ELOOP）。
                // 在此情况下，我们希望提供一个恰当的错误消息。
                #[cfg(not(any(target_os = "macos", target_os = "freebsd")))]
                let ct_symlink_code = 40;
                #[cfg(any(target_os = "macos", target_os = "freebsd"))]
                let too_many_symlink_code = 62;
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

/// Writes handle to stdout with no configuration. This allows a
/// simple memory copy.
fn cat_write_fast<R: CatFdReadable>(input_handle: &mut CatInputHandle<R>) -> CatResult<()> {
    let stdout_info = io::stdout();
    let mut cat_stdout_lock = stdout_info.lock();
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        // 在类Unix系统中，表示“符号链接层数过多”的错误代码为40（即ELOOP）。
        // 在此情况下，我们希望提供一个恰当的错误消息。
        if !splice::splice_write_fast_using_splice(input_handle, &cat_stdout_lock)? {
            return Ok(());
        }
    }
    // 如果当前运行环境不是Linux或Android系统，或者splice()系统调用执行失败，
    // 我们将回退到使用较慢的写入方式。
    let mut buffer = [0; 1024 * 64];
    while let Ok(n) = input_handle.reader.read(&mut buffer) {
        if n == 0 {
            break;
        }
        cat_stdout_lock.write_all(&buffer[..n])?;
    }
    Ok(())
}

/// Outputs file contents to stdout in a line-by-line fashion,
/// propagating any errors that might occur.
fn cat_write_lines<R: CatFdReadable>(
    input_handle: &mut CatInputHandle<R>,
    output_options: &CatOutputOptions,
    output_state: &mut CatOutputState,
) -> CatResult<()> {
    let mut input_buffer = [0; 1024 * 31];
    let stdout_info = io::stdout();
    let mut stdout_writer = stdout_info.lock();

    while let Ok(n) = input_handle.reader.read(&mut input_buffer) {
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
                stdout_writer.write_all(b"\r")?;
                output_state.skipped_carriage_return = false;
                output_state.at_line_start = false;
            }
            output_state.one_blank_kept = false;
            if output_state.at_line_start && output_options.num_mode != CatNumberingMode::None {
                write!(stdout_writer, "{0:6}\t", output_state.line_number)?;
                output_state.line_number += 1;
            }

            // 打印至行尾或缓冲区尾部。
            let offset = cat_write_end(&mut stdout_writer, &in_buffer[position..], output_options);

            // 是否已到达缓冲区尾部？
            if offset + position == in_buffer.len() {
                output_state.at_line_start = false;
                break;
            }
            if in_buffer[position + offset] == b'\r' {
                output_state.skipped_carriage_return = true;
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

    Ok(())
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
    // 如果之前跳过了回车符且设置为显示文件两端，则输出回车符的转义序列
    if output_state.skipped_carriage_return && output_options.show_ends {
        cat_writer.write_all(b"^M")?;
        output_state.skipped_carriage_return = false;
    }

    // 不在行首、或设置为不压缩空行、或已保留一个空格，则准备写入新的行
    if !output_state.at_line_start || !output_options.squeeze_blank || !output_state.one_blank_kept
    {
        output_state.one_blank_kept = true;
        // 如果处于行首且要求对所有行编号，则写入行号
        if output_state.at_line_start && output_options.num_mode == CatNumberingMode::All {
            write!(cat_writer, "{0:6}\t", output_state.line_number)?;
            output_state.line_number += 1;
        }
        // 根据选项，写入行尾标记
        cat_writer.write_all(output_options.cat_end_of_line().as_bytes())?;
        // 如果处于交互模式，刷新输出
        if is_interactive {
            cat_writer.flush()?;
        }
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
) -> usize {
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
fn cat_write_to_end<W: Write>(in_buffer: &[u8], cat_writer: &mut W) -> usize {
    // 查找缓冲区中第一个换行符或回车符的位置
    match in_buffer.iter().position(|c| *c == b'\n' || *c == b'\r') {
        Some(p) => {
            // 如果找到了换行符或回车符，将从缓冲区起始位置到该位置的字节写入写入器
            cat_writer.write_all(&in_buffer[..p]).unwrap();
            p // 返回写入的字节数
        }
        None => {
            // 如果没有找到换行符或回车符，将整个缓冲区的字节写入写入器
            cat_writer.write_all(in_buffer).unwrap();
            in_buffer.len() // 返回写入的字节数（即整个缓冲区的长度）
        }
    }
}

/**
 * 将字节切片中的内容写入指定的写入器，遇到换行符或制表符时特殊处理。
 * - 对于制表符，将其替换为"^I"并继续处理后续字符。
 * - 对于换行符或回车符，停止处理当前行，并返回已处理字符的数量。
 *
 */
fn cat_write_tab_to_end<W: Write>(mut in_buffer: &[u8], cat_writer: &mut W) -> usize {
    let mut count = 0; // 已处理字符计数器。

    loop {
        // 查找切片中第一个换行符、制表符或回车符的位置。
        match in_buffer
            .iter()
            .position(|c| *c == b'\n' || *c == b'\t' || *c == b'\r')
        {
            Some(p) => {
                // 写入找到特殊字符之前的切片内容。
                cat_writer.write_all(&in_buffer[..p]).unwrap();

                if in_buffer[p] == b'\t' {
                    // 如果找到的是制表符，替换为"^I"并继续处理后续字符。
                    cat_writer.write_all(b"^I").unwrap();
                    in_buffer = &in_buffer[p + 1..];
                    count += p + 1;
                } else {
                    // 如果找到的是换行符或回车符，停止处理当前行。
                    return count + p;
                }
            }
            None => {
                // 如果切片中没有特殊字符，一次性写入全部内容并返回。
                cat_writer.write_all(in_buffer).unwrap();
                return in_buffer.len();
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
) -> usize {
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
            127 => cat_writer.write_all(&[b'^', b'?']),               // 特殊处理删除符
            128..=159 => cat_writer.write_all(&[b'M', b'-', b'^', c - 64]), // 处理128-159范围的字符
            160..=254 => cat_writer.write_all(&[b'M', b'-', c - 128]), // 处理160-254范围的字符
            _ => cat_writer.write_all(&[b'M', b'-', b'^', b'?']), // 对于其他未知字符，使用默认转义序列
        }
        .unwrap(); // 确保写入操作成功
        count += 1; // 增加处理的字节计数
    }
    count
}

fn cat_write_end_of_line<W: Write>(
    cat_writer: &mut W,
    end_of_line: &[u8],
    is_interactive: bool,
) -> CatResult<()> {
    cat_writer.write_all(end_of_line)?;
    if is_interactive {
        cat_writer.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;
    use std::fs;
    use std::io::{self, Write};
    use std::io::{stdout, BufWriter};

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

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), "-V"];

        // Assuming `command` has a method to retrieve the executable name, replace it with the actual one
        let executable = command.try_get_matches_from(args);

        assert!(executable.is_err());
        assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
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
    fn test_ct_app_execution_unsupport_help() {
        let command = ct_app();

        // 测试用例2：验证 --help 参数是否正确处理
        let help_args = vec![ctcore::ct_util_name(), "-H"];
        let result = command.try_get_matches_from(help_args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn test_write_nonprint_to_end_29() {
        let mut writer = BufWriter::with_capacity(1024 * 64, stdout());
        let in_buf = &[9u8];
        let tab = b"tab";
        super::cat_write_non_print_to_end(in_buf, &mut writer, tab);
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
            Ok(_) => println!("File '{}' created successfully.", filename),
            Err(e) => eprintln!("Error creating file: {}", e),
        }

        let command = get_command();

        let args = vec![ctcore::ct_util_name(), filename];
        let matches = command.try_get_matches_from(args);

        // 删除文件
        match base_delete_file(filename) {
            Ok(_) => println!("File '{}' deleted successfully.", filename),
            Err(e) => eprintln!("Error deleting file: {}", e),
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
        super::cat_write_non_print_to_end(in_buf, &mut writer, tab);
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
            super::cat_write_non_print_to_end(in_buf, &mut writer, tab);
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

}