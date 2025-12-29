/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

//! unexpand命令通常用于将行首的空格转换为制表符，这样可以使得文本在显示时按照固定的列对齐，尤其是在处理纯文本表格时。

use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::{stdin, stdout, BufRead, BufReader, BufWriter, Read, Write};
use std::num::IntErrorKind;
use std::path::Path;
use std::str::from_utf8;

use clap::{crate_version, Arg, ArgAction, ArgMatches, Command};
use unicode_width::UnicodeWidthChar;

use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTError, CTResult, CtSimpleError, FromIo};
use ctcore::{ct_crash_if_err, ct_format_usage, ct_help_about, ct_help_usage, ct_show};

const UNEXPAND_USAGE: &str = ct_help_usage!("unexpand.md");
const UNEXPAND_ABOUT: &str = ct_help_about!("unexpand.md");

const UNEXPAND_DEFAULT_TABSTOP: usize = 8;

#[derive(Debug, PartialEq)]
enum UnexpandParseError {
    InvalidCharacter(String),
    TabSizeCannotBeZero,
    TabSizeTooLarge,
    TabSizesMustBeAscending,
}

impl Error for UnexpandParseError {}

impl CTError for UnexpandParseError {}

impl fmt::Display for UnexpandParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::InvalidCharacter(s) => {
                write!(f, "tab size contains invalid character(s): {}", s.quote())
            }
            Self::TabSizeCannotBeZero => write!(f, "tab size cannot be 0"),
            Self::TabSizeTooLarge => write!(f, "tab stop value is too large"),
            Self::TabSizesMustBeAscending => write!(f, "tab sizes must be ascending"),
        }
    }
}

fn unexpand_tabstops_parse(s: &str) -> Result<Vec<usize>, UnexpandParseError> {
    let words = s.split(',');

    let mut nums = Vec::new();

    for word in words {
        match word.parse::<usize>() {
            Ok(num) => nums.push(num),
            Err(e) => match e.kind() {
                IntErrorKind::PosOverflow => return Err(UnexpandParseError::TabSizeTooLarge),
                _ => {
                    return Err(UnexpandParseError::InvalidCharacter(
                        word.trim_start_matches(char::is_numeric).to_string(),
                    ));
                }
            },
        }
    }

    if nums.iter().any(|&n| n == 0) {
        return Err(UnexpandParseError::TabSizeCannotBeZero);
    }

    if let (false, _) = nums
        .iter()
        .fold((true, 0), |(acc, last), &n| (acc && last < n, n))
    {
        return Err(UnexpandParseError::TabSizesMustBeAscending);
    }

    Ok(nums)
}

mod unexpand_flags {
    pub const FILE: &str = "file";
    pub const ALL: &str = "all";
    pub const FIRST_ONLY: &str = "first-only";
    pub const TABS: &str = "tabs";
    pub const NO_UTF8: &str = "no-utf8";
}

struct UnexpandFlags {
    files: Vec<String>,
    tabstops: Vec<usize>,
    is_a_flag: bool,
    is_u_flag: bool,
}

impl UnexpandFlags {
    fn new(matches: &clap::ArgMatches) -> Result<Self, UnexpandParseError> {
        let tabstops = Self::parse_tabstops(matches)?;

        let is_a_flag = Self::parse_a_flag(matches);
        let is_u_flag = Self::parse_u_flag(matches);
        let files = Self::parse_files(matches);

        Ok(Self {
            files,
            tabstops,
            is_a_flag,
            is_u_flag,
        })
    }

    fn parse_u_flag(matches: &ArgMatches) -> bool {
        !matches.get_flag(unexpand_flags::NO_UTF8)
    }

    fn parse_files(matches: &ArgMatches) -> Vec<String> {
        if let Some(v) = matches.get_many::<String>(unexpand_flags::FILE) {
            v.cloned().collect()
        } else {
            vec!["-".to_owned()]
        }
    }

    fn parse_a_flag(matches: &ArgMatches) -> bool {
        (matches.get_flag(unexpand_flags::ALL) || matches.contains_id(unexpand_flags::TABS))
            && !matches.get_flag(unexpand_flags::FIRST_ONLY)
    }

    fn parse_tabstops(matches: &ArgMatches) -> Result<Vec<usize>, UnexpandParseError> {
        let tabstops = if let Some(s) = matches.get_many::<String>(unexpand_flags::TABS) {
            unexpand_tabstops_parse(&s.map(|s| s.as_str()).collect::<Vec<_>>().join(","))?
        } else {
            vec![UNEXPAND_DEFAULT_TABSTOP]
        };

        Ok(tabstops)
    }
}

/// 判断字符是否为数字或逗号。
fn is_digit_or_comma(c: char) -> bool {
    c.is_ascii_digit() || c == ','
}

/// 预处理命令行参数并展开快捷方式。例如，"-7"会被扩展为"--tabs=7 --first-only"，
/// 而"-1,3"会扩展为"--tabs=1 --tabs=3 --first-only"。
/// 但是，如果提供了"-a"或"--all"选项，则不会包含"--first-only"。
fn expand_shortcuts(args: &[String]) -> Vec<String> {
    let mut processed_args_string = Vec::with_capacity(args.len());
    let mut is_all_arg_provided = false;
    let mut is_has_shortcuts = false;

    for arg_str in args {
        if arg_str.starts_with('-') && arg_str[1..].chars().all(is_digit_or_comma) {
            arg_str[1..]
                .split(',')
                .filter(|s| !s.is_empty())
                .for_each(|s| processed_args_string.push(format!("--tabs={s}")));
            is_has_shortcuts = true;
        } else {
            processed_args_string.push(arg_str.to_string());

            if arg_str == "--all" || arg_str == "-a" {
                is_all_arg_provided = true;
            }
        }
    }

    if is_has_shortcuts && !is_all_arg_provided {
        processed_args_string.push("--first-only".into());
    }

    processed_args_string
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    unexpand_main(args)
}

pub fn unexpand_main(args: impl ctcore::Args) -> CTResult<()> {
    let args = args.collect_ignore();

    let matches = ct_app().try_get_matches_from(expand_shortcuts(&args))?;

    unexpand(&UnexpandFlags::new(&matches)?)
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = UNEXPAND_ABOUT;
    let usage_description = ct_format_usage(UNEXPAND_USAGE);
    let args = vec![
        Arg::new(unexpand_flags::FILE)
            .hide(true)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::FilePath),
        Arg::new(unexpand_flags::ALL)
            .short('a')
            .long(unexpand_flags::ALL)
            .help("convert all blanks, instead of just initial blanks")
            .action(ArgAction::SetTrue),
        Arg::new(unexpand_flags::FIRST_ONLY)
            .long(unexpand_flags::FIRST_ONLY)
            .help("convert only leading sequences of blanks (overrides -a)")
            .action(ArgAction::SetTrue),
        Arg::new(unexpand_flags::TABS)
            .short('t')
            .long(unexpand_flags::TABS)
            .help(
                "use comma separated LIST of tab positions or have tabs N characters \
                apart instead of 8 (enables -a)",
            )
            .action(ArgAction::Append)
            .value_name("N, LIST"),
        Arg::new(unexpand_flags::NO_UTF8)
            .short('U')
            .long(unexpand_flags::NO_UTF8)
            .help("interpret input file as 8-bit ASCII rather than UTF-8")
            .action(ArgAction::SetTrue),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(args)
}

fn unexpand_open(path: &str) -> CTResult<BufReader<Box<dyn Read + 'static>>> {
    let file_buf;
    let filename = Path::new(path);
    if filename.is_dir() {
        Err(Box::new(CtSimpleError {
            code: 1,
            message: format!("{}: Is a directory", filename.display()),
        }))
    } else if path == "-" {
        Ok(BufReader::new(Box::new(stdin()) as Box<dyn Read>))
    } else {
        file_buf = File::open(path).map_err_context(|| path.to_string())?;
        Ok(BufReader::new(Box::new(file_buf) as Box<dyn Read>))
    }
}

fn unexpand_next_tabstop(tabstops: &[usize], col: usize) -> Option<usize> {
    match tabstops.len() {
        1 => Some(tabstops[0] - col % tabstops[0]),
        _ => {
            // 查找下一个较大的标签。
            // 如果列表中没有下一个更大的标签，那么当前的“tab”将被替换为一个空格。
            tabstops.iter().find(|&&t| t > col).map(|t| t - col)
        }
    }
}

fn unexpand_write_tabs<W: Write>(
    out: &mut W,
    tabstops: &[usize],
    mut s_col: usize,
    col: usize,
    is_prevtab: bool,
    is_init: bool,
    is_amode: bool,
) {
    // 这个条件语句确保以下行为：我们从不将非空白字符前的一个空格转换为制表符，除非这个空格位于行首。
    let is_ai = is_init || is_amode;
    if (is_ai && !is_prevtab && col > s_col + 1)
        || (col > s_col && (is_init || is_ai && is_prevtab))
    {
        while let Some(nts) = unexpand_next_tabstop(tabstops, s_col) {
            if col < s_col + nts {
                break;
            }

            ct_crash_if_err!(1, out.write_all(b"\t"));
            s_col += nts;
        }
    }

    while col > s_col {
        ct_crash_if_err!(1, out.write_all(b" "));
        s_col += 1;
    }
}

#[derive(PartialEq, Eq, Debug)]
enum UnexpandCharType {
    Backspace,
    Space,
    Tab,
    Other,
}

fn unexpand_next_char_info(
    is_u_flag: bool,
    buf: &[u8],
    byte: usize,
) -> (UnexpandCharType, usize, usize) {
    let (c_type, c_width, n_bytes) = if is_u_flag {
        let n_bytes = char::from(buf[byte]).len_utf8();

        if byte + n_bytes > buf.len() {
            // 确保因无效的UTF-8编码导致不会超出缓冲区范围。
            (UnexpandCharType::Other, 1, 1)
        } else if let Ok(t) = from_utf8(&buf[byte..byte + n_bytes]) {
            // 既然我们认为它是UTF-8编码，接下来确定它属于哪种字符类型。
            match t.chars().next() {
                Some(' ') => (UnexpandCharType::Space, 0, 1),
                Some('\t') => (UnexpandCharType::Tab, 0, 1),
                Some('\x08') => (UnexpandCharType::Backspace, 0, 1),
                Some(c) => (
                    UnexpandCharType::Other,
                    UnicodeWidthChar::width(c).unwrap_or(0),
                    n_bytes,
                ),
                None => {
                    //  有一个无效的字符不知何故绕过了utf8_validation_iterator???
                    (UnexpandCharType::Other, 1, 1)
                }
            }
        } else {
            // 否则，它被认为是无效的
            (UnexpandCharType::Other, 1, 1) // 假设：非UTF-8字符的显示宽度为1
        }
    } else {
        (
            match buf[byte] {
                // 在严格的ASCII模式下，始终占用精确的1字节
                0x20 => UnexpandCharType::Space,
                0x09 => UnexpandCharType::Tab,
                0x08 => UnexpandCharType::Backspace,
                _ => UnexpandCharType::Other,
            },
            1,
            1,
        )
    };

    (c_type, c_width, n_bytes)
}

#[allow(clippy::cognitive_complexity)]
fn unexpand_line<W: Write>(
    buf: &mut Vec<u8>,
    output: &mut W,
    flags: &UnexpandFlags,
    lastcol: usize,
    ts: &[usize],
) -> std::io::Result<()> {
    let mut byte = 0; // 缓冲区中的偏移量
    let mut col = 0; // 当前列
    let mut s_col = 0; // 当前跨度的起始列，即已打印的宽度
    let mut is_init = true; // 我们是否在行的开始？
    let mut pctype = UnexpandCharType::Other;

    while byte < buf.len() {
        // 当我们有有限的列数时，永远不要转换超过最后一列
        if lastcol > 0 && col >= lastcol {
            unexpand_write_tabs(
                output,
                ts,
                s_col,
                col,
                pctype == UnexpandCharType::Tab,
                is_init,
                true,
            );
            output.write_all(&buf[byte..])?;
            s_col = col;
            break;
        }

        // 计算下一个字符的大小，如果它是UTF-8编码的
        let (c_type, c_width, n_bytes) = unexpand_next_char_info(flags.is_u_flag, buf, byte);

        // 现在确定这个字符占用了多少列，并可能将其打印出来
        let tabs_buffered = is_init || flags.is_a_flag;
        match c_type {
            UnexpandCharType::Space | UnexpandCharType::Tab => {
                // 计算下一行列，但只有在不缓冲空间或制表符字符时才写入它们
                col += if c_type == UnexpandCharType::Space {
                    1
                } else {
                    unexpand_next_tabstop(ts, col).unwrap_or(1)
                };

                if !tabs_buffered {
                    output.write_all(&buf[byte..byte + n_bytes])?;
                    s_col = col; // 现在已经打印到这一列了
                }
            }
            UnexpandCharType::Other | UnexpandCharType::Backspace => {
                // always
                unexpand_write_tabs(
                    output,
                    ts,
                    s_col,
                    col,
                    pctype == UnexpandCharType::Tab,
                    is_init,
                    flags.is_a_flag,
                );
                is_init = false; // 不再位于行的开头
                col = if c_type == UnexpandCharType::Other {
                    // 使用计算出的宽度
                    col + c_width
                } else if col > 0 {
                    // 退格情况，但仅当列数大于0时
                    col - 1
                } else {
                    0
                };
                output.write_all(&buf[byte..byte + n_bytes])?;
                s_col = col; // 我们现在已打印到这一列
            }
        }

        byte += n_bytes; // 移动到下一个字符
        pctype = c_type; // 保存上一个类型
    }

    // 写入任何剩余的内容
    unexpand_write_tabs(
        output,
        ts,
        s_col,
        col,
        pctype == UnexpandCharType::Tab,
        is_init,
        true,
    );
    output.flush()?;
    buf.truncate(0); // 清空缓冲区

    Ok(())
}

fn unexpand(flags: &UnexpandFlags) -> CTResult<()> {
    let mut output = BufWriter::new(stdout());

    unexpand_exe(flags, &mut output)?;
    Ok(())
}

fn unexpand_exe<W: Write>(
    flags: &UnexpandFlags,
    mut output: &mut W,
) -> Result<(), Box<dyn CTError>> {
    let ts = &flags.tabstops[..];
    let mut data_buf = Vec::new();
    let last_col = if ts.len() > 1 { *ts.last().unwrap() } else { 0 };

    for file in &flags.files {
        let mut fh = match unexpand_open(file) {
            Ok(reader) => reader,
            Err(err) => {
                ct_show!(err);
                continue;
            }
        };

        while match fh.read_until(b'\n', &mut data_buf) {
            Ok(size) => size > 0,
            Err(_) => !data_buf.is_empty(),
        } {
            unexpand_line(&mut data_buf, &mut output, flags, last_col, ts)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::is_digit_or_comma;

    use super::*;

    #[cfg(test)]
    mod unexpand_tests {
        use std::fs::write;

        use tempfile::{tempdir, NamedTempFile};

        use super::*;

        #[test]
        fn test_unexpand_exe_with_single_file() {
            let file = NamedTempFile::new().unwrap();
            write(file.path(), b"    Hello\tWorld\n").unwrap();

            let flags = UnexpandFlags {
                files: vec![file.path().to_str().unwrap().to_string()],
                tabstops: vec![4],
                is_a_flag: false,
                is_u_flag: false,
            };

            let mut output = Vec::new();
            unexpand_exe(&flags, &mut output).unwrap();

            let result = String::from_utf8(output).unwrap();
            assert_eq!(result, "\t\tHello\tWorld\n");
        }

        #[test]
        fn test_unexpand_exe_with_multiple_files() {
            let dir = tempdir().unwrap();
            let file1_path = dir.path().join("file1.txt");
            let file2_path = dir.path().join("file2.txt");

            write(&file1_path, b"    Hello\n").unwrap();
            write(&file2_path, b"\tWorld\n").unwrap();

            let flags = UnexpandFlags {
                files: vec![
                    file1_path.to_str().unwrap().to_string(),
                    file2_path.to_str().unwrap().to_string(),
                ],
                tabstops: vec![4],
                is_a_flag: false,
                is_u_flag: false,
            };

            let mut output = Vec::new();
            unexpand_exe(&flags, &mut output).unwrap();

            let result = String::from_utf8(output).unwrap();
            assert_eq!(result, "\t\tHello\n\t\tWorld\n");
        }

        #[test]
        fn test_unexpand_exe_with_utf8_characters() {
            let file = NamedTempFile::new().unwrap();
            write(file.path(), "    Hello 世界\n".as_bytes()).unwrap();

            let flags = UnexpandFlags {
                files: vec![file.path().to_str().unwrap().to_string()],
                tabstops: vec![4],
                is_a_flag: false,
                is_u_flag: true,
            };

            let mut output = Vec::new();
            unexpand_exe(&flags, &mut output).unwrap();

            let result = String::from_utf8(output).unwrap();
            assert_eq!(result, "\t\tHello 世界\n");
        }

        #[test]
        fn test_unexpand_exe_with_backspaces() {
            let file = NamedTempFile::new().unwrap();
            write(file.path(), b"Hello\n\nWorld\n").unwrap();

            let flags = UnexpandFlags {
                files: vec![file.path().to_str().unwrap().to_string()],
                tabstops: vec![4],
                is_a_flag: false,
                is_u_flag: false,
            };

            let mut output = Vec::new();
            unexpand_exe(&flags, &mut output).unwrap();

            let result = String::from_utf8(output).unwrap();
            assert_eq!(result, "Hello\n\nWorld\n");
        }

        #[test]
        fn test_unexpand_exe_with_no_files() {
            let flags = UnexpandFlags {
                files: vec![],
                tabstops: vec![4],
                is_a_flag: false,
                is_u_flag: false,
            };

            let mut output = Vec::new();
            unexpand_exe(&flags, &mut output).unwrap();

            let result = String::from_utf8(output).unwrap();
            assert_eq!(result, "");
        }
    }

    #[cfg(test)]
    mod unexpand_line_tests {
        use std::io::Cursor;

        use super::*;

        #[test]
        fn test_unexpand_line_with_spaces_and_tabs() {
            let mut buf = b"    \tHello".to_vec();
            let mut output = Cursor::new(Vec::new());
            let flags = UnexpandFlags {
                files: vec![],
                tabstops: vec![4],
                is_a_flag: false,
                is_u_flag: false,
            };

            unexpand_line(&mut buf, &mut output, &flags, 0, &[4]).unwrap();
            assert_eq!(
                String::from_utf8(output.into_inner()).unwrap(),
                "\t\t\t\tHello".to_string()
            );
        }

        #[test]
        fn test_unexpand_line_with_mixed_characters() {
            let mut buf = b"Hello\tWorld".to_vec();
            let mut output = Cursor::new(Vec::new());
            let flags = UnexpandFlags {
                files: vec![],
                tabstops: vec![8],
                is_a_flag: true,
                is_u_flag: false,
            };

            unexpand_line(&mut buf, &mut output, &flags, 0, &[8]).unwrap();
            assert_eq!(
                String::from_utf8(output.into_inner()).unwrap(),
                "Hello\t\tWorld".to_string()
            );
        }

        #[test]
        fn test_unexpand_line_with_utf8_characters() {
            let mut buf = "Hello 世界".as_bytes().to_vec();
            let mut output = Cursor::new(Vec::new());
            let flags = UnexpandFlags {
                files: vec![],
                tabstops: vec![8],
                is_a_flag: true,
                is_u_flag: true,
            };

            unexpand_line(&mut buf, &mut output, &flags, 0, &[8]).unwrap();
            assert_eq!(
                String::from_utf8(output.into_inner()).unwrap(),
                "Hello  世界".to_string()
            );
        }

        #[test]
        fn test_unexpand_line_with_backspace() {
            let mut buf = b"Hello\n\nWorld".to_vec();
            let mut output = Cursor::new(Vec::new());
            let flags = UnexpandFlags {
                files: vec![],
                tabstops: vec![8],
                is_a_flag: true,
                is_u_flag: false,
            };

            unexpand_line(&mut buf, &mut output, &flags, 0, &[8]).unwrap();
            // println!("{:?}", output);
            assert_eq!(
                String::from_utf8(output.into_inner()).unwrap(),
                "Hello\n\nWorld".to_string()
            );
        }

        #[test]
        fn test_unexpand_line_with_lastcol_limit() {
            let mut buf = b"Hello\tWorld".to_vec();
            let mut output = Cursor::new(Vec::new());
            let flags = UnexpandFlags {
                files: vec![],
                tabstops: vec![8],
                is_a_flag: true,
                is_u_flag: false,
            };

            unexpand_line(&mut buf, &mut output, &flags, 5, &[8]).unwrap();
            assert_eq!(output.into_inner(), b"Hello\tWorld");
        }

        #[test]
        fn test_unexpand_line_with_no_tabstops() {
            let mut buf = b"Hello World".to_vec();
            let mut output = Cursor::new(Vec::new());
            let flags = UnexpandFlags {
                files: vec![],
                tabstops: vec![],
                is_a_flag: true,
                is_u_flag: false,
            };

            unexpand_line(&mut buf, &mut output, &flags, 0, &[]).unwrap();
            assert_eq!(
                String::from_utf8(output.into_inner()).unwrap(),
                "Hello  World".to_string()
            );
        }

        #[test]
        fn test_unexpand_line_with_initial_whitespace() {
            let mut buf = b"   Hello".to_vec();
            let mut output = Cursor::new(Vec::new());
            let flags = UnexpandFlags {
                files: vec![],
                tabstops: vec![4],
                is_a_flag: false,
                is_u_flag: false,
            };

            unexpand_line(&mut buf, &mut output, &flags, 0, &[4]).unwrap();
            assert_eq!(
                String::from_utf8(output.into_inner()).unwrap(),
                "      Hello".to_string()
            );
        }

        #[test]
        fn test_unexpand_line_with_multiple_tabstops() {
            let mut buf = b"       Hello".to_vec();
            let mut output = Cursor::new(Vec::new());
            let flags = UnexpandFlags {
                files: vec![],
                tabstops: vec![4, 8],
                is_a_flag: false,
                is_u_flag: false,
            };

            unexpand_line(&mut buf, &mut output, &flags, 0, &[4, 8]).unwrap();
            assert_eq!(
                String::from_utf8(output.into_inner()).unwrap(),
                "\t\t      Hello".to_string()
            );
        }
    }

    #[cfg(test)]
    mod next_char_info_tests {
        use super::*;

        #[test]
        fn test_next_char_info_with_utf8() {
            let buf = "Hello, 世界!".as_bytes();
            let (ctype, cwidth, nbytes) = unexpand_next_char_info(true, buf, 7);
            assert_eq!(ctype, UnexpandCharType::Other);
            assert_eq!(cwidth, 1); // "世"的字符宽度
            assert_eq!(nbytes, 1); // "世"的UTF-8字节数
        }

        #[test]
        fn test_next_char_info_with_ascii_space() {
            let buf = "Hello world".as_bytes();
            let (ctype, cwidth, nbytes) = unexpand_next_char_info(false, buf, 5);
            assert_eq!(ctype, UnexpandCharType::Space);
            assert_eq!(cwidth, 1);
            assert_eq!(nbytes, 1);
        }

        #[test]
        fn test_next_char_info_with_ascii_tab() {
            let buf = "Hello\tworld".as_bytes();
            let (ctype, cwidth, nbytes) = unexpand_next_char_info(false, buf, 5);
            assert_eq!(ctype, UnexpandCharType::Tab);
            assert_eq!(cwidth, 1);
            assert_eq!(nbytes, 1);
        }

        #[test]
        fn test_next_char_info_with_backspace() {
            let buf = "Hello\nworld".as_bytes();
            let (ctype, cwidth, nbytes) = unexpand_next_char_info(false, buf, 5);
            assert_eq!(ctype, UnexpandCharType::Other);
            assert_eq!(cwidth, 1);
            assert_eq!(nbytes, 1);
        }

        #[test]
        fn test_next_char_info_with_invalid_utf8() {
            let buf = [0xff, 0xfe, 0xfd];
            let (ctype, cwidth, nbytes) = unexpand_next_char_info(true, &buf, 0);
            assert_eq!(ctype, UnexpandCharType::Other);
            assert_eq!(cwidth, 1);
            assert_eq!(nbytes, 1);
        }
    }
}