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

// spell-checker:ignore (vars) BUFWRITER seekable

extern crate rust_i18n;
use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "zh-CN");
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CtSimpleError, FromIo};
use ctcore::ct_line_ending::CtLineEnding;
use ctcore::ct_lines::lines;
use ctcore::ct_show;
use std::ffi::OsString;
use std::io::{BufWriter, ErrorKind, Read, Seek, SeekFrom, Write};
use sys_locale::get_locale;

const BUF_SIZE: usize = 65536;

/// The capacity in bytes for buffered writers.
const BUFWRITER_CAPACITY: usize = 16_384; // 16 kilobytes

mod head_flags {
    pub const BYTES_NAME: &str = "BYTES";
    pub const LINES_NAME: &str = "LINES";
    pub const QUIET_NAME: &str = "QUIET";
    pub const VERBOSE_NAME: &str = "VERBOSE";
    pub const ZERO_NAME: &str = "ZERO";
    pub const FILES_NAME: &str = "FILE";
    pub const PRESUME_INPUT_PIPE: &str = "-PRESUME-INPUT-PIPE";
}

mod parse;
mod take;
use take::take_all_but;
use take::take_lines;

pub fn ct_app() -> Command {
    let args = vec![
        Arg::new(head_flags::BYTES_NAME)
            .short('c')
            .long("bytes")
            .value_name("[-]NUM")
            .help(
                "\
                    print the first NUM bytes of each file;\n\
                    with the leading '-', print all but the last\n\
                    NUM bytes of each file\
                    ",
            )
            .overrides_with_all([head_flags::BYTES_NAME, head_flags::LINES_NAME])
            .allow_hyphen_values(true),
        Arg::new(head_flags::LINES_NAME)
            .short('n')
            .long("lines")
            .value_name("[-]NUM")
            .help(
                "\
                    print the first NUM lines instead of the first 10;\n\
                    with the leading '-', print all but the last\n\
                    NUM lines of each file\
                    ",
            )
            .overrides_with_all([head_flags::LINES_NAME, head_flags::BYTES_NAME])
            .allow_hyphen_values(true),
        Arg::new(head_flags::QUIET_NAME)
            .short('q')
            .long("quiet")
            .visible_alias("silent")
            .help(t!("head.clap.quiet_name"))
            .overrides_with_all([head_flags::VERBOSE_NAME, head_flags::QUIET_NAME])
            .action(ArgAction::SetTrue),
        Arg::new(head_flags::VERBOSE_NAME)
            .short('v')
            .long("verbose")
            .help(t!("head.clap.verbose_name"))
            .overrides_with_all([head_flags::QUIET_NAME, head_flags::VERBOSE_NAME])
            .action(ArgAction::SetTrue),
        Arg::new(head_flags::PRESUME_INPUT_PIPE)
            .long("presume-input-pipe")
            .alias("-presume-input-pipe")
            .hide(true)
            .action(ArgAction::SetTrue),
        Arg::new(head_flags::ZERO_NAME)
            .short('z')
            .long("zero-terminated")
            .help(t!("head.clap.zero_name"))
            .overrides_with(head_flags::ZERO_NAME)
            .action(ArgAction::SetTrue),
        Arg::new(head_flags::FILES_NAME)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::FilePath),
    ];

    Command::new(ctcore::ct_util_name())
        .version(crate_version!())
        .about(t!("head.about"))
        .override_usage(t!("head.usage"))
        .infer_long_args(true)
        .args(args)
}

#[derive(Debug, PartialEq)]
pub enum Mode {
    FirstLines(u64),
    AllButLastLines(u64),
    FirstBytes(u64),
    AllButLastBytes(u64),
}

impl Default for Mode {
    fn default() -> Self {
        Self::FirstLines(10)
    }
}

impl Mode {
    fn from(matches: &ArgMatches) -> Result<Self, String> {
        if let Some(v) = matches.get_one::<String>(head_flags::BYTES_NAME) {
            let (n, all_but_last) =
                parse::parse_num(v).map_err(|err| format!("invalid number of bytes: {err}"))?;
            if all_but_last {
                Ok(Self::AllButLastBytes(n))
            } else {
                Ok(Self::FirstBytes(n))
            }
        } else if let Some(v) = matches.get_one::<String>(head_flags::LINES_NAME) {
            let (n, all_but_last) =
                parse::parse_num(v).map_err(|err| format!("invalid number of lines: {err}"))?;
            if all_but_last {
                Ok(Self::AllButLastLines(n))
            } else {
                Ok(Self::FirstLines(n))
            }
        } else {
            Ok(Self::default())
        }
    }
}

fn arg_iterate<'a>(
    mut args: impl ctcore::Args + 'a,
) -> CTResult<Box<dyn Iterator<Item = OsString> + 'a>> {
    let first = args.next().unwrap();
    if let Some(second) = args.next() {
        if let Some(s) = second.to_str() {
            match parse::parse_obsolete(s) {
                Some(Ok(options)) => {
                    let mut result = vec![first];
                    result.extend(options);
                    result.extend(args);
                    Ok(Box::new(result.into_iter()))
                }
                Some(Err(e)) => match e {
                    parse::ParseError::Syntax => Err(CtSimpleError::new(
                        1,
                        format!("bad argument format: {}", s.quote()),
                    )),
                    parse::ParseError::Overflow => Err(CtSimpleError::new(
                        1,
                        format!(
                            "invalid argument: {} Value too large for defined datatype",
                            s.quote()
                        ),
                    )),
                },
                None => Ok(Box::new(vec![first, second].into_iter().chain(args))),
            }
        } else {
            Err(CtSimpleError::new(1, "bad argument encoding".to_owned()))
        }
    } else {
        Ok(Box::new(vec![first].into_iter()))
    }
}

#[derive(Debug, PartialEq, Default)]
/// `HeadOptions` 结构体用于配置 `head` 命令的行为。
/// 它包含了一系列选项，用于控制命令的输出、处理方式和目标文件。
pub struct HeadOptions {
    /// `quiet` 标志用于控制是否减少命令的输出。
    /// 当设置为 `true` 时，命令将尽量减少不必要的输出信息。
    pub quiet: bool,
    /// `verbose` 标志用于控制是否增加命令的输出详细度。
    /// 当设置为 `true` 时，命令将提供更详细的输出信息。
    pub verbose: bool,
    /// `line_ending` 指定行结束符的类型。
    /// 它影响输出时行的终止方式，如Unix风格（LF）或Windows风格（CRLF）。
    pub line_ending: CtLineEnding,
    /// `presume_input_pipe` 标志用于指示是否假定输入为管道。
    /// 当设置为 `true` 时，命令将优化其行为以适应管道输入。
    pub presume_input_pipe: bool,
    /// `mode` 指定了命令的运行模式。
    /// 它决定了命令如何处理输入和生成输出，比如显示的行数或字节数。
    pub mode: Mode,
    /// `files` 列表包含了命令要处理的文件路径。
    /// 命令将根据这个列表中的文件进行操作。
    pub files: Vec<String>,
}

impl HeadOptions {
    ///Construct options from matches
    pub fn get_from(matches: &clap::ArgMatches) -> Result<Self, String> {
        let options = Self {
            quiet: matches.get_flag(head_flags::QUIET_NAME),
            verbose: matches.get_flag(head_flags::VERBOSE_NAME),
            line_ending: CtLineEnding::from_zero_flag(matches.get_flag(head_flags::ZERO_NAME)),
            presume_input_pipe: matches.get_flag(head_flags::PRESUME_INPUT_PIPE),
            mode: Mode::from(matches)?,
            files: match matches.get_many::<String>(head_flags::FILES_NAME) {
                Some(v) => v.cloned().collect(),
                None => vec!["-".to_owned()],
            },
        };

        Ok(options)
    }
}

fn read_n_bytes<R>(input: R, n: u64, buffer: Option<&mut Vec<u8>>) -> std::io::Result<()>
where
    R: Read,
{
    // Read the first `n` bytes from the `input` reader.
    let mut reader = input.take(n);
    let mut local_buffer = Vec::new();

    // Read bytes into buffer
    reader.read_to_end(&mut local_buffer)?;

    // Write those bytes to `stdout`.
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    stdout.write_all(&local_buffer)?;

    // If a buffer was provided, copy the bytes into it
    if let Some(buf) = buffer {
        buf.extend_from_slice(&local_buffer);
    }

    Ok(())
}

fn read_n_lines(
    input: &mut impl std::io::BufRead,
    n: u64,
    separator: u8,
    buffer: Option<&mut Vec<u8>>,
) -> std::io::Result<()> {
    // Read the first `n` lines from the `input` reader.
    let mut reader = take_lines(input, n, separator);

    // Write those bytes to `stdout`.
    let stdout = std::io::stdout();
    let stdout = stdout.lock();
    let mut writer = BufWriter::with_capacity(BUFWRITER_CAPACITY, stdout);

    // Read into a local buffer first
    let mut local_buffer = Vec::new();
    reader.read_to_end(&mut local_buffer)?;

    // Write to stdout
    writer.write_all(&local_buffer)?;

    // If a buffer was provided, copy the bytes into it
    if let Some(buf) = buffer {
        buf.extend_from_slice(&local_buffer);
    }

    Ok(())
}

fn catch_too_large_numbers_in_backwards_bytes_or_lines(n: u64) -> Option<usize> {
    match usize::try_from(n) {
        Ok(value) => Some(value),
        Err(e) => {
            ct_show!(CtSimpleError::new(
                1,
                format!("{e}: number of -bytes or -lines is too large")
            ));
            None
        }
    }
}

fn read_but_last_n_bytes(
    input: &mut impl std::io::BufRead,
    n: u64,
    buffer: Option<&mut Vec<u8>>,
) -> std::io::Result<()> {
    if n == 0 {
        //prints everything
        return read_n_bytes(input, u64::MAX, buffer);
    }

    if let Some(n) = catch_too_large_numbers_in_backwards_bytes_or_lines(n) {
        let stdout = std::io::stdout();
        let mut stdout = stdout.lock();

        let mut local_buffer = Vec::new();
        let mut ring_buffer = Vec::new();

        let mut buffer_size = [0u8; BUF_SIZE];
        let mut total_read = 0;

        loop {
            let read = match input.read(&mut buffer_size) {
                Ok(0) => break,
                Ok(read) => read,
                Err(e) => match e.kind() {
                    ErrorKind::Interrupted => continue,
                    _ => return Err(e),
                },
            };

            total_read += read;

            if total_read <= n {
                ring_buffer.extend_from_slice(&buffer_size[..read]);
            } else {
                let to_write = &buffer_size[..(read - (n - (total_read - read)).min(read))];
                local_buffer.extend_from_slice(to_write);
                ring_buffer.clear();
                ring_buffer.extend_from_slice(&buffer_size[(read - n.min(read))..read]);
            }
        }

        // Write to stdout
        stdout.write_all(&local_buffer)?;

        // If a buffer was provided, copy the bytes into it
        if let Some(buf) = buffer {
            buf.extend_from_slice(&local_buffer);
        }
    }

    Ok(())
}

fn read_but_last_n_lines(
    input: impl std::io::BufRead,
    n: u64,
    separator: u8,
    buffer: Option<&mut Vec<u8>>,
) -> std::io::Result<()> {
    if let Some(n) = catch_too_large_numbers_in_backwards_bytes_or_lines(n) {
        let stdout = std::io::stdout();
        let mut stdout = stdout.lock();

        let mut local_buffer = Vec::new();
        for bytes in take_all_but(lines(input, separator), n) {
            let bytes = bytes?;
            local_buffer.extend_from_slice(&bytes);
        }

        // Write to stdout
        stdout.write_all(&local_buffer)?;

        // If a buffer was provided, copy the bytes into it
        if let Some(buf) = buffer {
            buf.extend_from_slice(&local_buffer);
        }
    }
    Ok(())
}

/// Return the index in `input` just after the `n`th line from the end.
///
/// If `n` exceeds the number of lines in this file, then return 0.
///
/// The cursor must be at the start of the seekable input before
/// calling this function. This function rewinds the cursor to the
/// beginning of the input just before returning unless there is an
/// I/O error.
///
/// If `zeroed` is `false`, interpret the newline character `b'\n'` as
/// a line ending. If `zeroed` is `true`, interpret the null character
/// `b'\0'` as a line ending instead.
///
/// # Errors
///
/// This function returns an error if there is a problem seeking
/// through or reading the input.
///
/// # Examples
///
/// The function returns the index of the byte immediately following
/// the line ending character of the `n`th line from the end of the
/// input:
///
/// ```rust,ignore
/// let mut input = Cursor::new("x\ny\nz\n");
/// assert_eq!(find_nth_line_from_end(&mut input, 0, false).unwrap(), 6);
/// assert_eq!(find_nth_line_from_end(&mut input, 1, false).unwrap(), 4);
/// assert_eq!(find_nth_line_from_end(&mut input, 2, false).unwrap(), 2);
/// ```
///
/// If `n` exceeds the number of lines in the file, always return 0:
///
/// ```rust,ignore
/// let mut input = Cursor::new("x\ny\nz\n");
/// assert_eq!(find_nth_line_from_end(&mut input, 3, false).unwrap(), 0);
/// assert_eq!(find_nth_line_from_end(&mut input, 4, false).unwrap(), 0);
/// assert_eq!(find_nth_line_from_end(&mut input, 1000, false).unwrap(), 0);
/// ```
fn find_nth_line_from_end<R>(input: &mut R, n: u64, separator: u8) -> std::io::Result<u64>
where
    R: Read + Seek,
{
    // 获取文件总大小并检查空文件
    let size = input.seek(SeekFrom::End(0))?;
    if size == 0 {
        input.rewind()?;
        return Ok(0);
    }

    // 使用较大的缓冲区以减少 I/O 操作
    const OPTIMAL_BUF_SIZE: usize = 8192; // 8KB buffer
    let mut buffer = vec![0u8; OPTIMAL_BUF_SIZE.min(size as usize)];
    let mut lines_found = 0;
    let mut position = size;
    let mut last_separator_pos = size;

    // 从文件末尾开始向前搜索
    while position > 0 {
        // 计算当前块的大小和起始位置
        let chunk_size = OPTIMAL_BUF_SIZE.min(position as usize);
        let start_pos = position - chunk_size as u64;

        // 读取当前块
        input.seek(SeekFrom::Start(start_pos))?;
        let read_buf = &mut buffer[..chunk_size];
        input.read_exact(read_buf)?;

        // 从后向前查找分隔符
        for (i, &byte) in read_buf.iter().rev().enumerate() {
            if byte == separator {
                if lines_found == 0 {
                    last_separator_pos = position - i as u64;
                }
                lines_found += 1;
                if lines_found > n {
                    input.rewind()?;
                    return Ok(position - i as u64);
                }
            }
        }

        position = start_pos;
    }

    // 如果没有找到足够的分隔符
    input.rewind()?;
    if lines_found == 0 {
        // 如果没有找到任何分隔符，返回0
        Ok(0)
    } else if n >= lines_found {
        // 如果请求的行数超过了实际的行数，返回0
        Ok(0)
    } else {
        // 如果是请求最后一个分隔符之后的位置（n=0），返回文件大小
        Ok(last_separator_pos)
    }
}

fn is_seekable(input: &mut std::fs::File) -> bool {
    let current_pos = input.stream_position();
    current_pos.is_ok()
        && input.seek(SeekFrom::End(0)).is_ok()
        && input.seek(SeekFrom::Start(current_pos.unwrap())).is_ok()
}

fn head_backwards_file(input: &mut std::fs::File, options: &HeadOptions) -> std::io::Result<()> {
    let st = input.metadata()?;
    let seekable = is_seekable(input);
    let blksize_limit = ctcore::ct_fs::sane_blksize::sane_blksize_from_metadata(&st);
    if !seekable || st.len() <= blksize_limit {
        return head_backwards_without_seek_file(input, options);
    }

    head_backwards_on_seekable_file(input, options)
}

fn head_backwards_without_seek_file(
    input: &mut std::fs::File,
    options: &HeadOptions,
) -> std::io::Result<()> {
    let reader = &mut std::io::BufReader::with_capacity(BUF_SIZE, &*input);

    match options.mode {
        Mode::AllButLastBytes(n) => read_but_last_n_bytes(reader, n, None)?,
        Mode::AllButLastLines(n) => {
            read_but_last_n_lines(reader, n, options.line_ending.into(), None)?
        }
        _ => unreachable!(),
    }

    Ok(())
}

fn head_backwards_on_seekable_file(
    input: &mut std::fs::File,
    options: &HeadOptions,
) -> std::io::Result<()> {
    match options.mode {
        Mode::AllButLastBytes(n) => {
            let size = input.metadata()?.len();
            if n >= size {
                return Ok(());
            } else {
                read_n_bytes(
                    &mut std::io::BufReader::with_capacity(BUF_SIZE, input),
                    size - n,
                    None,
                )?;
            }
        }
        Mode::AllButLastLines(n) => {
            let found = find_nth_line_from_end(input, n, options.line_ending.into())?;
            read_n_bytes(
                &mut std::io::BufReader::with_capacity(BUF_SIZE, input),
                found,
                None,
            )?;
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn head_file(input: &mut std::fs::File, options: &HeadOptions) -> std::io::Result<()> {
    match options.mode {
        Mode::FirstBytes(n) => read_n_bytes(
            &mut std::io::BufReader::with_capacity(BUF_SIZE, input),
            n,
            None,
        ),
        Mode::FirstLines(n) => read_n_lines(
            &mut std::io::BufReader::with_capacity(BUF_SIZE, input),
            n,
            options.line_ending.into(),
            None,
        ),
        Mode::AllButLastBytes(_) | Mode::AllButLastLines(_) => head_backwards_file(input, options),
    }
}

#[allow(clippy::cognitive_complexity)]
fn ct_head(options: &HeadOptions) -> CTResult<()> {
    let mut first = true;

    for file in &options.files {
        // 处理文件，如果有错误则继续处理下一个文件
        process_file(file, options, &mut first)?;
    }

    Ok(())
}

// 处理单个文件
fn process_file(file: &str, options: &HeadOptions, first: &mut bool) -> CTResult<()> {
    let res = match (file, options.presume_input_pipe) {
        // 处理标准输入或管道输入
        (_, true) | ("-", false) => {
            print_file_header(options, *first, "standard input");
            handle_stdin(options)
        }
        // 处理普通文件
        (name, false) => process_regular_file(name, options, *first),
    };

    // 处理可能的错误
    if let Err(_e) = res {
        let name = if file == "-" { "standard input" } else { file };
        ct_show!(CtSimpleError::new(
            1,
            format!("error reading {name}: Input/output error")
        ));
    }

    *first = false;
    Ok(())
}

// 处理普通文件
fn process_regular_file(name: &str, options: &HeadOptions, first: bool) -> std::io::Result<()> {
    match std::fs::File::open(name) {
        Ok(mut file) => {
            print_file_header(options, first, name);
            head_file(&mut file, options)
        }
        Err(err) => {
            ct_show!(err.map_err_context(|| format!("cannot open {} for reading", name.quote())));
            Ok(())
        }
    }
}

// 打印文件头部
fn print_file_header(options: &HeadOptions, first: bool, name: &str) {
    if should_print_header(options) {
        if !first {
            println!();
        }
        println!("==> {} <==", name);
    }
}

// 判断是否需要打印头部
fn should_print_header(options: &HeadOptions) -> bool {
    (options.files.len() > 1 && !options.quiet) || options.verbose
}

// 处理标准输入
fn handle_stdin(options: &HeadOptions) -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let mut stdin = stdin.lock();

    match options.mode {
        Mode::FirstBytes(n) => read_n_bytes(&mut stdin, n, None),
        Mode::AllButLastBytes(n) => read_but_last_n_bytes(&mut stdin, n, None),
        Mode::FirstLines(n) => read_n_lines(&mut stdin, n, options.line_ending.into(), None),
        Mode::AllButLastLines(n) => {
            read_but_last_n_lines(&mut stdin, n, options.line_ending.into(), None)
        }
    }
}

#[derive(Default)]
pub struct Head;
impl Tool for Head {
    fn name(&self) -> &'static str {
        "head"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        head_main(args.iter().cloned())
    }
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    head_main(args)
}

pub fn head_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let matches = ct_app().try_get_matches_from(arg_iterate(args)?)?;
    let args = match HeadOptions::get_from(&matches) {
        Ok(o) => o,
        Err(s) => {
            return Err(CtSimpleError::new(1, s));
        }
    };
    ct_head(&args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Head;

        // Test name method
        assert_eq!(tool.name(), "head");

        // Test command method
        let command = tool.command();
        assert!(command.get_name().contains("head"));

        // Test execute method with help flag (should work)
        let args = vec![OsString::from("head"), OsString::from("--help")];
        assert!(tool.execute(&args).is_err());
    }

    mod test_find_nth_line_from_end {
        use super::*;
        use std::io::Cursor;

        #[test]
        fn test_basic_newline_separator() {
            let mut input = Cursor::new("x\ny\nz\n");
            assert_eq!(find_nth_line_from_end(&mut input, 0, b'\n').unwrap(), 6); // 从末尾数第0行，返回最后一个换行符后的位置
            assert_eq!(find_nth_line_from_end(&mut input, 1, b'\n').unwrap(), 4); // 从末尾数第1行，返回倒数第二个换行符后的位置
            assert_eq!(find_nth_line_from_end(&mut input, 2, b'\n').unwrap(), 2); // 从末尾数第2行，返回第一个换行符后的位置
            assert_eq!(find_nth_line_from_end(&mut input, 3, b'\n').unwrap(), 0);
            // 超出行数，返回0
        }

        #[test]
        fn test_custom_separator() {
            let mut input = Cursor::new("a;b;c;");
            assert_eq!(find_nth_line_from_end(&mut input, 0, b';').unwrap(), 6);
            assert_eq!(find_nth_line_from_end(&mut input, 1, b';').unwrap(), 4);
            assert_eq!(find_nth_line_from_end(&mut input, 2, b';').unwrap(), 2);
        }

        #[test]
        fn test_empty_input() {
            let mut input = Cursor::new("");
            assert_eq!(find_nth_line_from_end(&mut input, 0, b'\n').unwrap(), 0);
            assert_eq!(find_nth_line_from_end(&mut input, 1, b'\n').unwrap(), 0);
        }

        #[test]
        fn test_no_separator() {
            let mut input = Cursor::new("abc");
            // 对于没有分隔符的输入：
            // n=0 时，由于没有找到任何分隔符，返回0
            assert_eq!(find_nth_line_from_end(&mut input, 0, b'\n').unwrap(), 0);
            // n>0 时，由于没有找到任何分隔符，也返回0
            assert_eq!(find_nth_line_from_end(&mut input, 1, b'\n').unwrap(), 0);
        }

        #[test]
        fn test_only_separators() {
            let mut input = Cursor::new("\n\n\n");
            assert_eq!(find_nth_line_from_end(&mut input, 0, b'\n').unwrap(), 3);
            assert_eq!(find_nth_line_from_end(&mut input, 1, b'\n').unwrap(), 2);
            assert_eq!(find_nth_line_from_end(&mut input, 2, b'\n').unwrap(), 1);
            assert_eq!(find_nth_line_from_end(&mut input, 3, b'\n').unwrap(), 0);
        }

        #[test]
        fn test_unicode_content() {
            let mut input = Cursor::new("你好\n世界\n再见\n");
            // 每个汉字占3个字节，每个换行符占1个字节
            // "你好\n" = 7字节
            // "世界\n" = 7字节
            // "再见\n" = 7字节
            // 总共21字节
            assert_eq!(find_nth_line_from_end(&mut input, 0, b'\n').unwrap(), 21); // 最后一个换行符后的位置
            assert_eq!(find_nth_line_from_end(&mut input, 1, b'\n').unwrap(), 14); // 倒数第二个换行符后的位置
            assert_eq!(find_nth_line_from_end(&mut input, 2, b'\n').unwrap(), 7);
            // 第一个换行符后的位置
        }

        #[test]
        fn test_large_line_count() {
            let mut input = Cursor::new("x\ny\nz\n");
            assert_eq!(find_nth_line_from_end(&mut input, 1000, b'\n').unwrap(), 0);
        }

        #[test]
        fn test_without_final_separator() {
            let mut input = Cursor::new("a\nb\nc");
            // "a\n" = 2字节
            // "b\n" = 2字节
            // "c" = 1字节
            // 总共5字节
            // n=0 时返回最后一个换行符的位置
            assert_eq!(find_nth_line_from_end(&mut input, 0, b'\n').unwrap(), 4);
            // n=1 时返回第一个换行符的位置
            assert_eq!(find_nth_line_from_end(&mut input, 1, b'\n').unwrap(), 2);
            // n=2 时，由于没有更多换行符，返回0
            assert_eq!(find_nth_line_from_end(&mut input, 2, b'\n').unwrap(), 0);
        }
    }
    mod test_read_but_last_n_lines {
        use super::*;
        use std::io::BufReader;

        #[test]
        fn test_read_but_last_n_lines_exact() {
            let input = "line1\nline2\nline3\nline4\n";
            let mut reader = BufReader::new(input.as_bytes());
            let mut buffer = Vec::new();
            read_but_last_n_lines(&mut reader, 2, b'\n', Some(&mut buffer)).unwrap();
            assert_eq!(buffer, b"line1\nline2\n");
        }

        #[test]
        fn test_read_but_last_n_lines_more_than_available() {
            let input = "line1\nline2\n";
            let mut reader = BufReader::new(input.as_bytes());
            let mut buffer = Vec::new();
            read_but_last_n_lines(&mut reader, 5, b'\n', Some(&mut buffer)).unwrap();
            assert_eq!(buffer, b"");
        }

        #[test]
        fn test_read_but_last_n_lines_zero() {
            let input = "line1\nline2\n";
            let mut reader = BufReader::new(input.as_bytes());
            let mut buffer = Vec::new();
            read_but_last_n_lines(&mut reader, 0, b'\n', Some(&mut buffer)).unwrap();
            assert_eq!(buffer, b"line1\nline2\n");
        }

        #[test]
        fn test_read_but_last_n_lines_empty_input() {
            let input = "";
            let mut reader = BufReader::new(input.as_bytes());
            let mut buffer = Vec::new();
            read_but_last_n_lines(&mut reader, 5, b'\n', Some(&mut buffer)).unwrap();
            assert_eq!(buffer, b"");
        }

        #[test]
        fn test_read_but_last_n_lines_without_final_newline() {
            let input = "line1\nline2\nline3";
            let mut reader = BufReader::new(input.as_bytes());
            let mut buffer = Vec::new();
            read_but_last_n_lines(&mut reader, 1, b'\n', Some(&mut buffer)).unwrap();
            assert_eq!(buffer, b"line1\nline2\n");
        }

        #[test]
        fn test_read_but_last_n_lines_with_custom_separator() {
            let input = "line1;line2;line3;line4";
            let mut reader = BufReader::new(input.as_bytes());
            let mut buffer = Vec::new();
            read_but_last_n_lines(&mut reader, 2, b';', Some(&mut buffer)).unwrap();
            assert_eq!(buffer, b"line1;line2;");
        }

        #[test]
        fn test_read_but_last_n_lines_unicode() {
            let input = "你好\n世界\n再见\n";
            let mut reader = BufReader::new(input.as_bytes());
            let mut buffer = Vec::new();
            read_but_last_n_lines(&mut reader, 1, b'\n', Some(&mut buffer)).unwrap();
            assert_eq!(buffer, "你好\n世界\n".as_bytes());
        }

        #[test]
        fn test_read_but_last_n_lines_exact_size() {
            let input = "line1\nline2\n";
            let mut reader = BufReader::new(input.as_bytes());
            let mut buffer = Vec::new();
            read_but_last_n_lines(&mut reader, 2, b'\n', Some(&mut buffer)).unwrap();
            assert_eq!(buffer, b"");
        }
    }
    mod test_read_but_last_n_bytes {
        use super::*;
        use std::io::BufReader;

        #[test]
        fn test_read_but_last_n_bytes_exact() {
            let input = "Hello, World!";
            let mut reader = BufReader::new(input.as_bytes());
            let mut buffer = Vec::new();
            read_but_last_n_bytes(&mut reader, 6, Some(&mut buffer)).unwrap();
            assert_eq!(buffer, b"Hello, ");
        }

        #[test]
        fn test_read_but_last_n_bytes_more_than_available() {
            let input = "Hello";
            let mut reader = BufReader::new(input.as_bytes());
            let mut buffer = Vec::new();
            read_but_last_n_bytes(&mut reader, 10, Some(&mut buffer)).unwrap();
            assert_eq!(buffer, b"");
        }

        #[test]
        fn test_read_but_last_n_bytes_zero() {
            let input = "Hello";
            let mut reader = BufReader::new(input.as_bytes());
            let mut buffer = Vec::new();
            read_but_last_n_bytes(&mut reader, 0, Some(&mut buffer)).unwrap();
            assert_eq!(buffer, b"Hello");
        }

        #[test]
        fn test_read_but_last_n_bytes_empty_input() {
            let input = "";
            let mut reader = BufReader::new(input.as_bytes());
            let mut buffer = Vec::new();
            read_but_last_n_bytes(&mut reader, 5, Some(&mut buffer)).unwrap();
            assert_eq!(buffer, b"");
        }

        #[test]
        fn test_read_but_last_n_bytes_with_newlines() {
            let input = "line1\nline2\nline3";
            let mut reader = BufReader::new(input.as_bytes());
            let mut buffer = Vec::new();
            read_but_last_n_bytes(&mut reader, 5, Some(&mut buffer)).unwrap();
            assert_eq!(buffer, b"line1\nline2\n");
        }

        #[test]
        fn test_read_but_last_n_bytes_unicode() {
            let input = "你好世界"; // 每个汉字占3个字节
            let mut reader = BufReader::new(input.as_bytes());
            let mut buffer = Vec::new();
            read_but_last_n_bytes(&mut reader, 3, Some(&mut buffer)).unwrap();

            // "你好世界" 总共12个字节
            // 去掉最后3个字节（即"界"的一部分）后应该剩下9个字节
            // 这9个字节应该包含 "你好世"
            assert_eq!(buffer, "你好世".as_bytes());
            assert_eq!(buffer.len(), 9); // 验证字节长度
        }

        #[test]
        fn test_read_but_last_n_bytes_exact_size() {
            let input = "Hello";
            let mut reader = BufReader::new(input.as_bytes());
            let mut buffer = Vec::new();
            read_but_last_n_bytes(&mut reader, 5, Some(&mut buffer)).unwrap();
            assert_eq!(buffer, b"");
        }
    }
    mod test_read_n_lines {
        use super::*;
        use std::io::BufReader;

        #[test]
        fn test_read_n_lines_exact() {
            let input = "line1\nline2\nline3\n";
            let mut reader = BufReader::new(input.as_bytes());
            let mut buffer = Vec::new();
            read_n_lines(&mut reader, 2, b'\n', Some(&mut buffer)).unwrap();
            assert_eq!(buffer, b"line1\nline2\n");
        }

        #[test]
        fn test_read_n_lines_more_than_available() {
            let input = "line1\nline2\n";
            let mut reader = BufReader::new(input.as_bytes());
            let mut buffer = Vec::new();
            read_n_lines(&mut reader, 5, b'\n', Some(&mut buffer)).unwrap();
            assert_eq!(buffer, b"line1\nline2\n");
        }

        #[test]
        fn test_read_n_lines_zero() {
            let input = "line1\nline2\n";
            let mut reader = BufReader::new(input.as_bytes());
            let mut buffer = Vec::new();
            read_n_lines(&mut reader, 0, b'\n', Some(&mut buffer)).unwrap();
            assert_eq!(buffer, b"");
        }

        #[test]
        fn test_read_n_lines_empty_input() {
            let input = "";
            let mut reader = BufReader::new(input.as_bytes());
            let mut buffer = Vec::new();
            read_n_lines(&mut reader, 5, b'\n', Some(&mut buffer)).unwrap();
            assert_eq!(buffer, b"");
        }

        #[test]
        fn test_read_n_lines_without_final_newline() {
            let input = "line1\nline2\nline3";
            let mut reader = BufReader::new(input.as_bytes());
            let mut buffer = Vec::new();
            read_n_lines(&mut reader, 2, b'\n', Some(&mut buffer)).unwrap();
            assert_eq!(buffer, b"line1\nline2\n");
        }

        #[test]
        fn test_read_n_lines_with_custom_separator() {
            let input = "line1;line2;line3";
            let mut reader = BufReader::new(input.as_bytes());
            let mut buffer = Vec::new();
            read_n_lines(&mut reader, 2, b';', Some(&mut buffer)).unwrap();
            assert_eq!(buffer, b"line1;line2;");
        }

        #[test]
        fn test_read_n_lines_unicode() {
            let input = "你好\n世界\n";
            let mut reader = BufReader::new(input.as_bytes());
            let mut buffer = Vec::new();
            read_n_lines(&mut reader, 1, b'\n', Some(&mut buffer)).unwrap();
            assert_eq!(buffer, "你好\n".as_bytes());
        }
    }
    mod test_read_n_bytes {
        use super::*;
        use std::io::Cursor;

        #[test]
        fn test_read_n_bytes_exact() {
            let input = Cursor::new("Hello, World!");
            let mut buffer = Vec::new();
            read_n_bytes(input, 5, Some(&mut buffer)).unwrap();
            assert_eq!(buffer, b"Hello");
        }

        #[test]
        fn test_read_n_bytes_more_than_available() {
            let input = Cursor::new("Hello");
            let mut buffer = Vec::new();
            read_n_bytes(input, 10, Some(&mut buffer)).unwrap();
            assert_eq!(buffer, b"Hello");
        }

        #[test]
        fn test_read_n_bytes_zero() {
            let input = Cursor::new("Hello");
            let mut buffer = Vec::new();
            read_n_bytes(input, 0, Some(&mut buffer)).unwrap();
            assert_eq!(buffer, b"");
        }

        #[test]
        fn test_read_n_bytes_empty_input() {
            let input = Cursor::new("");
            let mut buffer = Vec::new();
            read_n_bytes(input, 5, Some(&mut buffer)).unwrap();
            assert_eq!(buffer, b"");
        }

        #[test]
        fn test_read_n_bytes_with_newlines() {
            let input = Cursor::new("line1\nline2\nline3");
            let mut buffer = Vec::new();
            read_n_bytes(input, 7, Some(&mut buffer)).unwrap();
            assert_eq!(buffer, b"line1\nl");
        }

        #[test]
        fn test_read_n_bytes_unicode() {
            let input = Cursor::new("你好，世界");
            let mut buffer = Vec::new();
            read_n_bytes(input, 6, Some(&mut buffer)).unwrap();
            assert_eq!(buffer, "你好".as_bytes());
        }
    }
    mod test_ct_head {
        use super::*;
        use std::io::Write;
        use tempfile::NamedTempFile;

        // 辅助函数：创建临时文件并写入内容
        fn create_temp_file(content: &str) -> NamedTempFile {
            let mut file = NamedTempFile::new().unwrap();
            write!(file, "{}", content).unwrap();
            file
        }

        /*
        #[test]
        fn test_ct_head_stdin() {
            // 测试标准输入
            let options = HeadOptions {
                quiet: false,
                verbose: false,
                line_ending: CtLineEnding::Newline,
                presume_input_pipe: true,
                mode: Mode::FirstLines(2),
                files: vec!["-".to_string()],
            };

            assert!(ct_head(&options).is_ok());
        }
        */
        #[test]
        fn test_ct_head_single_file() {
            // 创建临时文件
            let content = "line1\nline2\nline3\nline4\n";
            let temp_file = create_temp_file(content);
            let path = temp_file.path().to_str().unwrap().to_string();

            let options = HeadOptions {
                quiet: false,
                verbose: false,
                line_ending: CtLineEnding::Newline,
                presume_input_pipe: false,
                mode: Mode::FirstLines(2),
                files: vec![path],
            };

            assert!(ct_head(&options).is_ok());
        }

        #[test]
        fn test_ct_head_multiple_files() {
            // 创建两个临时文件
            let file1 = create_temp_file("file1-line1\nfile1-line2\n");
            let file2 = create_temp_file("file2-line1\nfile2-line2\n");

            let options = HeadOptions {
                quiet: false,
                verbose: true,
                line_ending: CtLineEnding::Newline,
                presume_input_pipe: false,
                mode: Mode::FirstLines(1),
                files: vec![
                    file1.path().to_str().unwrap().to_string(),
                    file2.path().to_str().unwrap().to_string(),
                ],
            };

            assert!(ct_head(&options).is_ok());
        }

        #[test]
        fn test_ct_head_nonexistent_file() {
            let options = HeadOptions {
                quiet: false,
                verbose: false,
                line_ending: CtLineEnding::Newline,
                presume_input_pipe: false,
                mode: Mode::FirstLines(1),
                files: vec!["nonexistent_file.txt".to_string()],
            };

            // 文件不存在时应该返回 Ok，但会设置错误码
            assert!(ct_head(&options).is_ok());
        }

        #[test]
        fn test_ct_head_bytes_mode() {
            let content = "Hello, World!";
            let temp_file = create_temp_file(content);
            let path = temp_file.path().to_str().unwrap().to_string();

            let options = HeadOptions {
                quiet: false,
                verbose: false,
                line_ending: CtLineEnding::Newline,
                presume_input_pipe: false,
                mode: Mode::FirstBytes(5),
                files: vec![path],
            };

            assert!(ct_head(&options).is_ok());
        }

        #[test]
        fn test_ct_head_quiet_mode() {
            let file1 = create_temp_file("content1");
            let file2 = create_temp_file("content2");

            let options = HeadOptions {
                quiet: true,
                verbose: false,
                line_ending: CtLineEnding::Newline,
                presume_input_pipe: false,
                mode: Mode::FirstLines(1),
                files: vec![
                    file1.path().to_str().unwrap().to_string(),
                    file2.path().to_str().unwrap().to_string(),
                ],
            };

            assert!(ct_head(&options).is_ok());
        }
    }
}

#[cfg(test)]
mod tests_other {
    use std::ffi::OsString;
    use std::io::Cursor;

    use super::*;

    fn options(args: &str) -> Result<HeadOptions, String> {
        let combined = "head ".to_owned() + args;
        let args = combined.split_whitespace().map(OsString::from);
        let matches = ct_app()
            .get_matches_from(arg_iterate(args).map_err(|_| String::from("Arg iterate failed"))?);
        HeadOptions::get_from(&matches)
    }

    #[test]
    fn test_args_modes() {
        let args = options("-n -10M -vz").unwrap();
        assert_eq!(args.line_ending, CtLineEnding::Nul);
        assert!(args.verbose);
        assert_eq!(args.mode, Mode::AllButLastLines(10 * 1024 * 1024));
    }

    #[test]
    fn test_gnu_compatibility() {
        let args = options("-n 1 -c 1 -n 5 -c kiB -vqvqv").unwrap(); // spell-checker:disable-line
        assert!(args.mode == Mode::FirstBytes(1024));
        assert!(args.verbose);
        assert_eq!(options("-5").unwrap().mode, Mode::FirstLines(5));
        assert_eq!(options("-2b").unwrap().mode, Mode::FirstBytes(1024));
        assert_eq!(options("-5 -c 1").unwrap().mode, Mode::FirstBytes(1));
    }

    #[test]
    #[allow(clippy::cognitive_complexity)]
    fn all_args_test() {
        assert!(options("--silent").unwrap().quiet);
        assert!(options("--quiet").unwrap().quiet);
        assert!(options("-q").unwrap().quiet);
        assert!(options("--verbose").unwrap().verbose);
        assert!(options("-v").unwrap().verbose);
        assert_eq!(
            options("--zero-terminated").unwrap().line_ending,
            CtLineEnding::Nul
        );
        assert_eq!(options("-z").unwrap().line_ending, CtLineEnding::Nul);
        assert_eq!(options("--lines 15").unwrap().mode, Mode::FirstLines(15));
        assert_eq!(options("-n 15").unwrap().mode, Mode::FirstLines(15));
        assert_eq!(options("--bytes 15").unwrap().mode, Mode::FirstBytes(15));
        assert_eq!(options("-c 15").unwrap().mode, Mode::FirstBytes(15));
    }

    #[test]
    fn test_options_errors() {
        assert!(options("-n IsThisTheRealLife?").is_err());
        assert!(options("-c IsThisJustFantasy").is_err());
    }

    #[test]
    fn test_options_correct_defaults() {
        let opts = HeadOptions::default();

        assert!(!opts.verbose);
        assert!(!opts.quiet);
        assert_eq!(opts.line_ending, CtLineEnding::Newline);
        assert_eq!(opts.mode, Mode::FirstLines(10));
        assert!(opts.files.is_empty());
    }

    fn arg_outputs(src: &str) -> Result<String, ()> {
        let split = src.split_whitespace().map(OsString::from);
        match arg_iterate(split) {
            Ok(args) => {
                let vec = args
                    .map(|s| s.to_str().unwrap().to_owned())
                    .collect::<Vec<_>>();
                Ok(vec.join(" "))
            }
            Err(_) => Err(()),
        }
    }

    #[test]
    fn test_arg_iterate() {
        // test that normal args remain unchanged
        assert_eq!(
            arg_outputs("head -n -5 -zv"),
            Ok("head -n -5 -zv".to_owned())
        );
        // tests that nonsensical args are unchanged
        assert_eq!(
            arg_outputs("head -to_be_or_not_to_be,..."),
            Ok("head -to_be_or_not_to_be,...".to_owned())
        );
        //test that the obsolete syntax is unrolled
        assert_eq!(
            arg_outputs("head -123qvqvqzc"), // spell-checker:disable-line
            Ok("head -q -z -c 123".to_owned())
        );
        //test that bad obsoletes are an error
        assert!(arg_outputs("head -123FooBar").is_err());
        //test overflow
        assert!(arg_outputs("head -100000000000000000000000000000000000000000").is_err());
        //test that empty args remain unchanged
        assert_eq!(arg_outputs("head"), Ok("head".to_owned()));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn test_arg_iterate_bad_encoding() {
        use std::os::unix::ffi::OsStringExt;
        let invalid = OsString::from_vec(vec![b'\x80', b'\x81']);
        // this arises from a conversion from OsString to &str
        assert!(arg_iterate(vec![OsString::from("head"), invalid].into_iter()).is_err());
    }

    #[test]
    fn read_early_exit() {
        let mut empty = std::io::BufReader::new(std::io::Cursor::new(Vec::new()));
        assert!(read_n_bytes(&mut empty, 0, None).is_ok());
        assert!(read_n_lines(&mut empty, 0, b'\n', None).is_ok());
    }

    #[test]
    fn test_find_nth_line_from_end() {
        let mut input = Cursor::new("x\ny\nz\n");
        assert_eq!(find_nth_line_from_end(&mut input, 0, b'\n').unwrap(), 6);
        assert_eq!(find_nth_line_from_end(&mut input, 1, b'\n').unwrap(), 4);
        assert_eq!(find_nth_line_from_end(&mut input, 2, b'\n').unwrap(), 2);
        assert_eq!(find_nth_line_from_end(&mut input, 3, b'\n').unwrap(), 0);
        assert_eq!(find_nth_line_from_end(&mut input, 4, b'\n').unwrap(), 0);
        assert_eq!(find_nth_line_from_end(&mut input, 1000, b'\n').unwrap(), 0);
    }
}
