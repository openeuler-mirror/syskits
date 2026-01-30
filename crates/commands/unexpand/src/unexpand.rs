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

//! unexpand命令通常用于将行首的空格转换为制表符，这样可以使得文本在显示时按照固定的列对齐，尤其是在处理纯文本表格时。

extern crate rust_i18n;
use rust_i18n::t;
use std::error::Error;
rust_i18n::i18n!("locales", fallback = "en-US");
use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use std::fmt;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read, Write, stdin, stdout};
use std::num::IntErrorKind;
use std::path::Path;
use std::str::from_utf8;
use sys_locale::get_locale;
use unicode_width::UnicodeWidthChar;

use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTError, CTResult, CtSimpleError, FromIo};
use ctcore::ct_show;
use std::ffi::OsString;

const UNEXPAND_DEFAULT_TABSTOP: usize = 8;

#[derive(Debug, PartialEq)]
enum UnexpandParseError {
    InvalidCharacter(String),
    SpecifierNotAtStartOfNumber(String, String),
    SpecifierOnlyAllowedWithLastValue(String),
    SpecifierMutuallyExclusive,
    TabSizeCannotBeZero,
    TabStopTooLarge(String),
    TabStopValueTooLarge,
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
            Self::SpecifierNotAtStartOfNumber(specifier, s) => write!(
                f,
                "{} specifier not at start of number: {}",
                specifier.quote(),
                s.quote()
            ),
            Self::SpecifierOnlyAllowedWithLastValue(specifier) => write!(
                f,
                "{} specifier only allowed with the last value",
                specifier.quote()
            ),
            Self::SpecifierMutuallyExclusive => {
                write!(f, "'/' specifier is mutually exclusive with '+'")
            }
            Self::TabSizeCannotBeZero => write!(f, "tab size cannot be 0"),
            Self::TabStopTooLarge(s) => write!(f, "tab stop is too large {}", s.quote()),
            Self::TabStopValueTooLarge => write!(f, "tab stop value is too large"),
            Self::TabSizesMustBeAscending => write!(f, "tab sizes must be ascending"),
        }
    }
}

#[derive(PartialEq, Debug, Clone, Copy)]
enum RemainingMode {
    None,
    Slash,
    Plus,
}

/// 判断字符是否为空格或逗号。
fn is_space_or_comma(c: char) -> bool {
    c == ' ' || c == ','
}

fn unexpand_tabstops_parse(
    s: &str,
    from_short_tabs: bool,
) -> Result<(RemainingMode, Vec<usize>), UnexpandParseError> {
    let str = s.trim_start_matches(is_space_or_comma);
    if str.is_empty() {
        return Ok((RemainingMode::None, vec![UNEXPAND_DEFAULT_TABSTOP]));
    }

    let mut numbers = vec![];
    let mut remaining_mode = RemainingMode::None;
    let mut specifier_used = false;

    for word in str.split(is_space_or_comma) {
        if word.is_empty() {
            continue;
        }
        let bytes = word.as_bytes();
        for index in 0..bytes.len() {
            match bytes[index] {
                b'+' => {
                    if remaining_mode == RemainingMode::Slash {
                        return Err(UnexpandParseError::SpecifierMutuallyExclusive);
                    }
                    remaining_mode = RemainingMode::Plus;
                }
                b'/' => {
                    if remaining_mode == RemainingMode::Plus {
                        return Err(UnexpandParseError::SpecifierMutuallyExclusive);
                    }
                    remaining_mode = RemainingMode::Slash;
                }
                _ => {
                    let s = from_utf8(&bytes[index..]).unwrap_or_default();
                    match s.parse::<usize>() {
                        Ok(num) => {
                            if num == 0 {
                                return Err(UnexpandParseError::TabSizeCannotBeZero);
                            }
                            if let Some(last) = numbers.last() {
                                if *last >= num {
                                    return Err(UnexpandParseError::TabSizesMustBeAscending);
                                }
                            }
                            if specifier_used {
                                let specifier = match remaining_mode {
                                    RemainingMode::Slash => "/",
                                    RemainingMode::Plus => "+",
                                    RemainingMode::None => "",
                                };
                                return Err(UnexpandParseError::SpecifierOnlyAllowedWithLastValue(
                                    specifier.to_string(),
                                ));
                            } else if remaining_mode != RemainingMode::None {
                                specifier_used = true;
                            }
                            numbers.push(num);
                            break;
                        }
                        Err(e) => {
                            if *e.kind() == IntErrorKind::PosOverflow {
                                return Err(if from_short_tabs {
                                    UnexpandParseError::TabStopValueTooLarge
                                } else {
                                    UnexpandParseError::TabStopTooLarge(s.to_string())
                                });
                            }

                            let s = s.trim_start_matches(char::is_numeric);
                            if s.starts_with('/') || s.starts_with('+') {
                                return Err(UnexpandParseError::SpecifierNotAtStartOfNumber(
                                    s[0..1].to_string(),
                                    s.to_string(),
                                ));
                            }
                            return Err(UnexpandParseError::InvalidCharacter(s.to_string()));
                        }
                    }
                }
            }
        }
    }

    if numbers.is_empty() {
        numbers = vec![UNEXPAND_DEFAULT_TABSTOP];
    }

    if numbers.len() < 2 {
        remaining_mode = RemainingMode::None;
    }

    Ok((remaining_mode, numbers))
}

mod unexpand_flags {
    pub const FILE: &str = "file";
    pub const ALL: &str = "all";
    pub const FIRST_ONLY: &str = "first-only";
    pub const TABS: &str = "tabs";
    pub const NO_UTF8: &str = "no-utf8";
    pub const SHORT_TABS: &str = "short-tabs";
}

struct UnexpandFlags {
    files: Vec<String>,
    tabstops: Vec<usize>,
    remaining_mode: RemainingMode,
    is_a_flag: bool,
    is_u_flag: bool,
}

impl UnexpandFlags {
    fn new(matches: &clap::ArgMatches) -> Result<Self, UnexpandParseError> {
        let (remaining_mode, tabstops) = Self::parse_tabstops(matches)?;

        let is_a_flag = Self::parse_a_flag(matches);
        let is_u_flag = Self::parse_u_flag(matches);
        let files = Self::parse_files(matches);

        Ok(Self {
            files,
            tabstops,
            remaining_mode,
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

    fn parse_tabstops(
        matches: &ArgMatches,
    ) -> Result<(RemainingMode, Vec<usize>), UnexpandParseError> {
        let from_short_tabs = matches.get_flag(unexpand_flags::SHORT_TABS);
        if let Some(s) = matches.get_many::<String>(unexpand_flags::TABS) {
            let input = s.map(|s| s.as_str()).collect::<Vec<_>>().join(",");
            return unexpand_tabstops_parse(&input, from_short_tabs);
        }
        Ok((RemainingMode::None, vec![UNEXPAND_DEFAULT_TABSTOP]))
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
    if is_has_shortcuts {
        processed_args_string.push("--short-tabs".into());
    }

    processed_args_string
}

pub fn unexpand_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let args = args.collect_ignore();

    let matches = ct_app().try_get_matches_from(expand_shortcuts(&args))?;

    unexpand(&UnexpandFlags::new(&matches)?)
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("unexpand.about");
    let usage_description = t!("unexpand.usage");
    let args = vec![
        Arg::new(unexpand_flags::FILE)
            .hide(true)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::FilePath),
        Arg::new(unexpand_flags::ALL)
            .short('a')
            .long(unexpand_flags::ALL)
            .help(t!("unexpand.clap.all"))
            .action(ArgAction::SetTrue),
        Arg::new(unexpand_flags::FIRST_ONLY)
            .long(unexpand_flags::FIRST_ONLY)
            .help(t!("unexpand.clap.first_only"))
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
            .help(t!("unexpand.clap.no_utf8"))
            .action(ArgAction::SetTrue),
        Arg::new(unexpand_flags::SHORT_TABS)
            .long(unexpand_flags::SHORT_TABS)
            .hide(true)
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

#[cfg(test)]
fn unexpand_next_tabstop(tabstops: &[usize], col: usize) -> Option<usize> {
    match tabstops.len() {
        1 => Some(tabstops[0] - col % tabstops[0]),
        _ => {
            // 查找下一个较大的标签。
            // 如果列表中没有下一个更大的标签，那么当前的"tab"将被替换为一个空格。
            tabstops.iter().find(|&&t| t > col).map(|t| t - col)
        }
    }
}

#[cfg(test)]
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

            ctcore::ct_crash_if_err!(1, out.write_all(b"\t"));
            s_col += nts;
        }
    }

    while col > s_col {
        ctcore::ct_crash_if_err!(1, out.write_all(b" "));
        s_col += 1;
    }
}

fn unexpand_next_tab_column(
    tabstops: &[usize],
    remaining_mode: RemainingMode,
    col: usize,
    tab_index: &mut usize,
) -> (usize, bool) {
    let total = tabstops.len();

    if total == 0 {
        return (
            col + (UNEXPAND_DEFAULT_TABSTOP - col % UNEXPAND_DEFAULT_TABSTOP),
            false,
        );
    }

    match remaining_mode {
        RemainingMode::None => {
            if total == 1 {
                let size = tabstops[0];
                return (col + (size - col % size), false);
            }
            while *tab_index < total {
                let tab = tabstops[*tab_index];
                if col < tab {
                    return (tab, false);
                }
                *tab_index += 1;
            }
            (0, true)
        }
        RemainingMode::Slash => {
            if total == 1 {
                let size = tabstops[0];
                return (col + (size - col % size), false);
            }
            let last_index = total - 1;
            while *tab_index < last_index {
                let tab = tabstops[*tab_index];
                if col < tab {
                    return (tab, false);
                }
                *tab_index += 1;
            }
            let size = tabstops[last_index];
            (col + (size - col % size), false)
        }
        RemainingMode::Plus => {
            if total == 1 {
                let size = tabstops[0];
                return (col + (size - col % size), false);
            }
            let last_index = total - 1;
            while *tab_index < last_index {
                let tab = tabstops[*tab_index];
                if col < tab {
                    return (tab, false);
                }
                *tab_index += 1;
            }
            let step = tabstops[last_index];
            let end_tab = tabstops[last_index - 1];
            let offset = col.saturating_sub(end_tab);
            (col + (step - (offset % step)), false)
        }
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
    if !is_u_flag {
        let c_type = match buf[byte] {
            0x20 => UnexpandCharType::Space,
            0x09 => UnexpandCharType::Tab,
            0x08 => UnexpandCharType::Backspace,
            _ => UnexpandCharType::Other,
        };
        return (c_type, 1, 1);
    }

    let slice = &buf[byte..];
    let (ch, n_bytes) = match from_utf8(slice) {
        Ok(s) => match s.chars().next() {
            Some(ch) => (ch, ch.len_utf8()),
            None => return (UnexpandCharType::Other, 1, 1),
        },
        Err(e) => {
            let valid = e.valid_up_to();
            if valid > 0 {
                let prefix = from_utf8(&slice[..valid]).unwrap_or_default();
                if let Some(ch) = prefix.chars().next() {
                    (ch, ch.len_utf8())
                } else {
                    return (UnexpandCharType::Other, 1, 1);
                }
            } else {
                return (UnexpandCharType::Other, 1, 1);
            }
        }
    };

    let c_type = if ch == '\t' {
        UnexpandCharType::Tab
    } else if ch == '\x08' {
        UnexpandCharType::Backspace
    } else if is_blank_char(ch) {
        UnexpandCharType::Space
    } else {
        UnexpandCharType::Other
    };
    let c_width = if matches!(c_type, UnexpandCharType::Tab | UnexpandCharType::Backspace) {
        0
    } else {
        UnicodeWidthChar::width(ch).unwrap_or(0)
    };
    (c_type, c_width, n_bytes)
}

fn is_blank_char(ch: char) -> bool {
    if ch == ' ' {
        return true;
    }
    if ch == '\n' || ch == '\r' {
        return false;
    }
    ch.is_whitespace()
}

#[allow(clippy::cognitive_complexity)]
fn unexpand_line<W: Write>(
    buf: &mut Vec<u8>,
    output: &mut W,
    flags: &UnexpandFlags,
    tabstops: &[usize],
    remaining_mode: RemainingMode,
) -> std::io::Result<()> {
    let mut byte = 0;
    let mut column: usize = 0;
    let mut tab_index: usize = 0;
    let mut one_blank_before_tab_stop = false;
    let mut prev_blank = true;
    let mut pending: Vec<Vec<u8>> = Vec::new();
    let convert_entire_line = flags.is_a_flag;
    let mut convert = true;

    while byte < buf.len() {
        let (c_type, c_width, n_bytes) = unexpand_next_char_info(flags.is_u_flag, buf, byte);
        let mut emit_tab = false;

        if convert {
            let blank = matches!(c_type, UnexpandCharType::Space | UnexpandCharType::Tab);
            if blank {
                let (next_tab_column, last_tab) =
                    unexpand_next_tab_column(tabstops, remaining_mode, column, &mut tab_index);
                if last_tab {
                    convert = false;
                }
                if convert {
                    if next_tab_column < column {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "input line is too long",
                        ));
                    }
                    if c_type == UnexpandCharType::Tab {
                        column = next_tab_column;
                        if !pending.is_empty() {
                            pending[0] = vec![b'\t'];
                        }
                    } else {
                        let next_column = column.saturating_add(c_width);
                        if next_column < column {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "input line is too long",
                            ));
                        }
                        column = next_column;

                        if !(prev_blank && column == next_tab_column) {
                            if column == next_tab_column {
                                one_blank_before_tab_stop = true;
                            }
                            pending.push(buf[byte..byte + n_bytes].to_vec());
                            prev_blank = true;
                            byte += n_bytes;
                            continue;
                        }

                        emit_tab = true;
                        if !pending.is_empty() {
                            pending[0] = vec![b'\t'];
                        }
                    }

                    pending.truncate(if one_blank_before_tab_stop { 1 } else { 0 });
                }
            } else if c_type == UnexpandCharType::Backspace {
                column = column.saturating_sub(1);
                tab_index = tab_index.saturating_sub(1);
            } else {
                let orig = column;
                column = column.saturating_add(c_width);
                if column < orig {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "input line is too long",
                    ));
                }
            }

            if !pending.is_empty() {
                if pending.len() > 1 && one_blank_before_tab_stop {
                    pending[0] = vec![b'\t'];
                }
                for blank in &pending {
                    output.write_all(blank)?;
                }
                pending.clear();
                one_blank_before_tab_stop = false;
            }

            prev_blank = matches!(c_type, UnexpandCharType::Space | UnexpandCharType::Tab);
            convert = convert && (convert_entire_line || prev_blank);
        }

        if emit_tab {
            output.write_all(b"\t")?;
        } else {
            output.write_all(&buf[byte..byte + n_bytes])?;
        }
        byte += n_bytes;
    }

    if !pending.is_empty() {
        if pending.len() > 1 && one_blank_before_tab_stop {
            pending[0] = vec![b'\t'];
        }
        for blank in &pending {
            output.write_all(blank)?;
        }
    }

    output.flush()?;
    buf.truncate(0);
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
    let tabstops = &flags.tabstops[..];
    let remaining_mode = flags.remaining_mode;
    let mut data_buf = Vec::new();
    let mut is_first_file = true;
    let mut first_file_has_bom = false;

    for file in &flags.files {
        let mut fh = match unexpand_open(file) {
            Ok(reader) => reader,
            Err(err) => {
                ct_show!(err);
                continue;
            }
        };
        let mut is_first_chunk = true;

        while match fh.read_until(b'\n', &mut data_buf) {
            Ok(size) => size > 0,
            Err(_) => !data_buf.is_empty(),
        } {
            if is_first_chunk {
                if data_buf.starts_with(&[0xEF, 0xBB, 0xBF]) {
                    if is_first_file && !first_file_has_bom {
                        output.write_all(&[0xEF, 0xBB, 0xBF])?;
                        first_file_has_bom = true;
                    }
                    data_buf.drain(0..3);
                }
                is_first_chunk = false;
            }

            if data_buf.is_empty() {
                data_buf.clear();
                continue;
            }

            unexpand_line(&mut data_buf, &mut output, flags, tabstops, remaining_mode)?;
        }
        is_first_file = false;
    }
    Ok(())
}

#[derive(Default)]
pub struct Unexpand;
impl Tool for Unexpand {
    fn name(&self) -> &'static str {
        "unexpand"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        unexpand_main(args.iter().cloned()).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use crate::is_digit_or_comma;
    use std::ffi::OsString;

    use super::*;

    #[test]
    fn test_tool_implementation() {
        let tool = Unexpand;

        // Test name method
        assert_eq!(tool.name(), "unexpand");

        // Test command method
        let command = tool.command();
        assert!(command.get_name().contains("unexpand"));

        // Test execute method with help flag (should work)
        let args: Vec<OsString> = vec![OsString::from("unexpand"), OsString::from("--help")];
        let result = tool.execute(&args);
        assert!(result.is_err());
    }

    #[cfg(test)]
    mod unexpand_tests {
        use std::fs::write;

        use tempfile::{NamedTempFile, tempdir};

        use super::*;

        #[test]
        fn test_unexpand_exe_with_single_file() {
            let file = NamedTempFile::new().unwrap();
            write(file.path(), b"    Hello\tWorld\n").unwrap();

            let flags = UnexpandFlags {
                files: vec![file.path().to_str().unwrap().to_string()],
                tabstops: vec![4],
                remaining_mode: RemainingMode::None,
                is_a_flag: false,
                is_u_flag: false,
            };

            let mut output = Vec::new();
            unexpand_exe(&flags, &mut output).unwrap();

            let result = String::from_utf8(output).unwrap();
            assert_eq!(result, "\tHello\tWorld\n");
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
                remaining_mode: RemainingMode::None,
                is_a_flag: false,
                is_u_flag: false,
            };

            let mut output = Vec::new();
            unexpand_exe(&flags, &mut output).unwrap();

            let result = String::from_utf8(output).unwrap();
            assert_eq!(result, "\tHello\n\tWorld\n");
        }

        #[test]
        fn test_unexpand_exe_with_utf8_characters() {
            let file = NamedTempFile::new().unwrap();
            write(file.path(), "    Hello 世界\n".as_bytes()).unwrap();

            let flags = UnexpandFlags {
                files: vec![file.path().to_str().unwrap().to_string()],
                tabstops: vec![4],
                remaining_mode: RemainingMode::None,
                is_a_flag: false,
                is_u_flag: true,
            };

            let mut output = Vec::new();
            unexpand_exe(&flags, &mut output).unwrap();

            let result = String::from_utf8(output).unwrap();
            assert_eq!(result, "\tHello 世界\n");
        }

        #[test]
        fn test_unexpand_exe_with_backspaces() {
            let file = NamedTempFile::new().unwrap();
            write(file.path(), b"Hello\n\nWorld\n").unwrap();

            let flags = UnexpandFlags {
                files: vec![file.path().to_str().unwrap().to_string()],
                tabstops: vec![4],
                remaining_mode: RemainingMode::None,
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
                remaining_mode: RemainingMode::None,
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
                remaining_mode: RemainingMode::None,
                is_a_flag: false,
                is_u_flag: false,
            };

            unexpand_line(&mut buf, &mut output, &flags, &[4], RemainingMode::None).unwrap();
            assert_eq!(
                String::from_utf8(output.into_inner()).unwrap(),
                "\t\tHello".to_string()
            );
        }

        #[test]
        fn test_unexpand_line_with_mixed_characters() {
            let mut buf = b"Hello\tWorld".to_vec();
            let mut output = Cursor::new(Vec::new());
            let flags = UnexpandFlags {
                files: vec![],
                tabstops: vec![8],
                remaining_mode: RemainingMode::None,
                is_a_flag: true,
                is_u_flag: false,
            };

            unexpand_line(&mut buf, &mut output, &flags, &[8], RemainingMode::None).unwrap();
            assert_eq!(
                String::from_utf8(output.into_inner()).unwrap(),
                "Hello\tWorld".to_string()
            );
        }

        #[test]
        fn test_unexpand_line_with_utf8_characters() {
            let mut buf = "Hello 世界".as_bytes().to_vec();
            let mut output = Cursor::new(Vec::new());
            let flags = UnexpandFlags {
                files: vec![],
                tabstops: vec![8],
                remaining_mode: RemainingMode::None,
                is_a_flag: true,
                is_u_flag: true,
            };

            unexpand_line(&mut buf, &mut output, &flags, &[8], RemainingMode::None).unwrap();
            assert_eq!(
                String::from_utf8(output.into_inner()).unwrap(),
                "Hello 世界".to_string()
            );
        }

        #[test]
        fn test_unexpand_line_with_backspace() {
            let mut buf = b"Hello\n\nWorld".to_vec();
            let mut output = Cursor::new(Vec::new());
            let flags = UnexpandFlags {
                files: vec![],
                tabstops: vec![8],
                remaining_mode: RemainingMode::None,
                is_a_flag: true,
                is_u_flag: false,
            };

            unexpand_line(&mut buf, &mut output, &flags, &[8], RemainingMode::None).unwrap();
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
                remaining_mode: RemainingMode::None,
                is_a_flag: true,
                is_u_flag: false,
            };

            unexpand_line(&mut buf, &mut output, &flags, &[8], RemainingMode::None).unwrap();
            assert_eq!(output.into_inner(), b"Hello\tWorld");
        }

        #[test]
        fn test_unexpand_line_with_no_tabstops() {
            let mut buf = b"Hello World".to_vec();
            let mut output = Cursor::new(Vec::new());
            let flags = UnexpandFlags {
                files: vec![],
                tabstops: vec![],
                remaining_mode: RemainingMode::None,
                is_a_flag: true,
                is_u_flag: false,
            };

            unexpand_line(&mut buf, &mut output, &flags, &[], RemainingMode::None).unwrap();
            assert_eq!(
                String::from_utf8(output.into_inner()).unwrap(),
                "Hello World".to_string()
            );
        }

        #[test]
        fn test_unexpand_line_with_initial_whitespace() {
            let mut buf = b"   Hello".to_vec();
            let mut output = Cursor::new(Vec::new());
            let flags = UnexpandFlags {
                files: vec![],
                tabstops: vec![4],
                remaining_mode: RemainingMode::None,
                is_a_flag: false,
                is_u_flag: false,
            };

            unexpand_line(&mut buf, &mut output, &flags, &[4], RemainingMode::None).unwrap();
            assert_eq!(
                String::from_utf8(output.into_inner()).unwrap(),
                "   Hello".to_string()
            );
        }

        #[test]
        fn test_unexpand_line_with_multiple_tabstops() {
            let mut buf = b"       Hello".to_vec();
            let mut output = Cursor::new(Vec::new());
            let flags = UnexpandFlags {
                files: vec![],
                tabstops: vec![4, 8],
                remaining_mode: RemainingMode::None,
                is_a_flag: false,
                is_u_flag: false,
            };

            unexpand_line(&mut buf, &mut output, &flags, &[4, 8], RemainingMode::None).unwrap();
            assert_eq!(
                String::from_utf8(output.into_inner()).unwrap(),
                "\t   Hello".to_string()
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
            assert_eq!(cwidth, 2); // "世"的字符宽度
            assert_eq!(nbytes, 3); // "世"的UTF-8字节数
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

    #[cfg(test)]
    mod write_tabs_tests {
        use std::io::Cursor;

        use super::*;

        #[test]
        fn test_unexpand_write_tabs_single_tabstop() {
            let mut output = Cursor::new(Vec::new());
            unexpand_write_tabs(&mut output, &[4], 0, 8, false, true, false);
            assert_eq!(output.into_inner(), b"\t\t");
        }

        #[test]
        fn test_unexpand_write_tabs_multiple_tabstops() {
            let mut output = Cursor::new(Vec::new());
            unexpand_write_tabs(&mut output, &[4, 8], 0, 12, false, true, false);
            assert_eq!(output.into_inner(), b"\t\t    ");
        }

        #[test]
        fn test_unexpand_write_tabs_no_tabstops() {
            let mut output = Cursor::new(Vec::new());
            unexpand_write_tabs(&mut output, &[], 0, 8, false, true, false);
            assert_eq!(output.into_inner(), b"        ");
        }

        #[test]
        fn test_unexpand_write_tabs_with_prevtab() {
            let mut output = Cursor::new(Vec::new());
            unexpand_write_tabs(&mut output, &[4], 0, 8, true, true, false);
            assert_eq!(output.into_inner(), b"\t\t");
        }

        #[test]
        fn test_unexpand_write_tabs_with_amode() {
            let mut output = Cursor::new(Vec::new());
            unexpand_write_tabs(&mut output, &[4], 0, 8, false, false, true);
            assert_eq!(output.into_inner(), b"\t\t");
        }

        #[test]
        fn test_unexpand_write_tabs_no_init_no_amode() {
            let mut output = Cursor::new(Vec::new());
            unexpand_write_tabs(&mut output, &[4], 0, 8, false, false, false);
            assert_eq!(output.into_inner(), b"        ");
        }

        #[test]
        fn test_unexpand_write_tabs_col_less_than_scol() {
            let mut output = Cursor::new(Vec::new());
            unexpand_write_tabs(&mut output, &[4], 8, 4, false, true, false);
            assert_eq!(output.into_inner(), b"");
        }

        #[test]
        fn test_unexpand_write_tabs_col_equals_scol() {
            let mut output = Cursor::new(Vec::new());
            unexpand_write_tabs(&mut output, &[4], 4, 4, false, true, false);
            assert_eq!(output.into_inner(), b"");
        }

        #[test]
        fn test_unexpand_write_tabs_col_greater_than_scol() {
            let mut output = Cursor::new(Vec::new());
            unexpand_write_tabs(&mut output, &[4], 2, 4, false, true, false);
            assert_eq!(output.into_inner(), b"\t");
        }
    }

    #[cfg(test)]
    mod next_tabstop_tests {
        use super::*;

        #[test]
        fn test_single_tabstop_before_column() {
            let tabstops = vec![8];
            let col = 3;
            assert_eq!(unexpand_next_tabstop(&tabstops, col), Some(5));
        }

        #[test]
        fn test_single_tabstop_at_column() {
            let tabstops = vec![8];
            let col = 8;
            assert_eq!(unexpand_next_tabstop(&tabstops, col), Some(8));
        }

        #[test]
        fn test_single_tabstop_after_column() {
            let tabstops = vec![8];
            let col = 9;
            assert_eq!(unexpand_next_tabstop(&tabstops, col), Some(7));
        }

        #[test]
        fn test_multiple_tabstops_before_column() {
            let tabstops = vec![4, 8, 12];
            let col = 3;
            assert_eq!(unexpand_next_tabstop(&tabstops, col), Some(1));
        }

        #[test]
        fn test_multiple_tabstops_between_columns() {
            let tabstops = vec![4, 8, 12];
            let col = 5;
            assert_eq!(unexpand_next_tabstop(&tabstops, col), Some(3));
        }

        #[test]
        fn test_multiple_tabstops_at_column() {
            let tabstops = vec![4, 8, 12];
            let col = 8;
            assert_eq!(unexpand_next_tabstop(&tabstops, col), Some(4));
        }

        #[test]
        fn test_multiple_tabstops_after_last() {
            let tabstops = vec![4, 8, 12];
            let col = 13;
            assert_eq!(unexpand_next_tabstop(&tabstops, col), None);
        }

        #[test]
        fn test_empty_tabstops() {
            let tabstops = vec![];
            let col = 5;
            assert_eq!(unexpand_next_tabstop(&tabstops, col), None);
        }

        #[test]
        fn test_column_equal_to_tabstop() {
            let tabstops = vec![4, 8, 12];
            let col = 4;
            assert_eq!(unexpand_next_tabstop(&tabstops, col), Some(4));
        }

        #[test]
        fn test_column_greater_than_all_tabstops() {
            let tabstops = vec![4, 8, 12];
            let col = 15;
            assert_eq!(unexpand_next_tabstop(&tabstops, col), None);
        }

        #[test]
        fn test_column_zero() {
            let tabstops = vec![4, 8, 12];
            let col = 0;
            assert_eq!(unexpand_next_tabstop(&tabstops, col), Some(4));
        }

        #[test]
        fn test_large_column_value() {
            let tabstops = vec![4, 8, 12];
            let col = 100;
            assert_eq!(unexpand_next_tabstop(&tabstops, col), None);
        }
    }

    #[cfg(test)]
    mod unexpand_open_tests {
        use std::fs::File;
        use std::io::{Read, Write};

        use tempfile::tempdir;

        use super::*;

        #[test]
        fn test_unexpand_open_with_file() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("testfile.txt");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Test content").unwrap();

            let result = unexpand_open(file_path.to_str().unwrap());
            assert!(result.is_ok());

            let mut reader = result.unwrap();
            let mut content = String::new();
            reader.read_to_string(&mut content).unwrap();
            assert_eq!(content, "Test content\n");
        }

        #[test]
        fn test_unexpand_open_with_stdin() {
            // This test is a bit tricky because it involves stdin,
            // so we won't actually test reading from stdin here
            let result = unexpand_open("-");
            assert!(result.is_ok());
        }
    }

    #[cfg(test)]
    mod expand_shortcuts_tests {
        use super::*;

        #[test]
        fn test_expand_shortcuts_no_shortcuts() {
            let args = vec![
                "--all".to_string(),
                "file1".to_string(),
                "file2".to_string(),
            ];
            let expected = vec![
                "--all".to_string(),
                "file1".to_string(),
                "file2".to_string(),
            ];
            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_with_tabs_shortcut() {
            let args = vec!["-4,8,12".to_string(), "file1".to_string()];
            let expected = vec![
                "--tabs=4".to_string(),
                "--tabs=8".to_string(),
                "--tabs=12".to_string(),
                "file1".to_string(),
                "--first-only".to_string(),
                "--short-tabs".to_string(),
            ];
            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_with_multiple_shortcuts() {
            let args = vec![
                "-4,8".to_string(),
                "-12,16".to_string(),
                "file1".to_string(),
            ];
            let expected = vec![
                "--tabs=4".to_string(),
                "--tabs=8".to_string(),
                "--tabs=12".to_string(),
                "--tabs=16".to_string(),
                "file1".to_string(),
                "--first-only".to_string(),
                "--short-tabs".to_string(),
            ];
            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_with_all_flag() {
            let args = vec!["-4,8".to_string(), "--all".to_string(), "file1".to_string()];
            let expected = vec![
                "--tabs=4".to_string(),
                "--tabs=8".to_string(),
                "--all".to_string(),
                "file1".to_string(),
                "--short-tabs".to_string(),
            ];
            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_with_a_flag() {
            let args = vec!["-4,8".to_string(), "-a".to_string(), "file1".to_string()];
            let expected = vec![
                "--tabs=4".to_string(),
                "--tabs=8".to_string(),
                "-a".to_string(),
                "file1".to_string(),
                "--short-tabs".to_string(),
            ];
            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_empty_input() {
            let args: Vec<String> = vec![];
            let expected: Vec<String> = vec![];
            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_with_mixed_args() {
            let args = vec![
                "--all".to_string(),
                "-4,8".to_string(),
                "--some-flag".to_string(),
                "file1".to_string(),
                "-12".to_string(),
            ];
            let expected = vec![
                "--all".to_string(),
                "--tabs=4".to_string(),
                "--tabs=8".to_string(),
                "--some-flag".to_string(),
                "file1".to_string(),
                "--tabs=12".to_string(),
                "--short-tabs".to_string(),
            ];
            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_with_invalid_shortcuts() {
            let args = vec!["-4,a".to_string(), "file1".to_string()];
            let expected = vec!["-4,a".to_string(), "file1".to_string()];
            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_with_leading_dash() {
            let args = vec!["file1".to_string(), "-4".to_string()];
            let expected = vec![
                "file1".to_string(),
                "--tabs=4".to_string(),
                "--first-only".to_string(),
                "--short-tabs".to_string(),
            ];
            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_with_repeated_tabs_shortcut() {
            let args = vec!["-4,8,8,12".to_string(), "file1".to_string()];
            let expected = vec![
                "--tabs=4".to_string(),
                "--tabs=8".to_string(),
                "--tabs=8".to_string(),
                "--tabs=12".to_string(),
                "file1".to_string(),
                "--first-only".to_string(),
                "--short-tabs".to_string(),
            ];
            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_with_only_shortcuts() {
            let args = vec!["-4,8,12".to_string()];
            let expected = vec![
                "--tabs=4".to_string(),
                "--tabs=8".to_string(),
                "--tabs=12".to_string(),
                "--first-only".to_string(),
                "--short-tabs".to_string(),
            ];
            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_with_only_files() {
            let args = vec!["file1".to_string(), "file2".to_string()];
            let expected = vec!["file1".to_string(), "file2".to_string()];
            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_with_empty_tabs_shortcut() {
            let args = vec!["-4,,8".to_string(), "file1".to_string()];
            let expected = vec![
                "--tabs=4".to_string(),
                "--tabs=8".to_string(),
                "file1".to_string(),
                "--first-only".to_string(),
                "--short-tabs".to_string(),
            ];
            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_with_tabs_and_other_flags() {
            let args = vec![
                "-4,8".to_string(),
                "--no-utf8".to_string(),
                "file1".to_string(),
            ];
            let expected = vec![
                "--tabs=4".to_string(),
                "--tabs=8".to_string(),
                "--no-utf8".to_string(),
                "file1".to_string(),
                "--first-only".to_string(),
                "--short-tabs".to_string(),
            ];
            assert_eq!(expand_shortcuts(&args), expected);
        }

        #[test]
        fn test_expand_shortcuts_with_only_flags() {
            let args = vec!["--all".to_string(), "--no-utf8".to_string()];
            let expected = vec!["--all".to_string(), "--no-utf8".to_string()];
            assert_eq!(expand_shortcuts(&args), expected);
        }
    }

    #[cfg(test)]
    mod is_digit_or_comma_tests {
        use super::*;

        #[test]
        fn test_is_digit_or_comma() {
            assert_eq!(is_digit_or_comma('0'), true);
            assert_eq!(is_digit_or_comma('1'), true);
            assert_eq!(is_digit_or_comma('2'), true);
            assert_eq!(is_digit_or_comma('3'), true);
            assert_eq!(is_digit_or_comma('4'), true);
            assert_eq!(is_digit_or_comma('5'), true);
            assert_eq!(is_digit_or_comma('6'), true);
            assert_eq!(is_digit_or_comma('7'), true);
            assert_eq!(is_digit_or_comma('8'), true);
            assert_eq!(is_digit_or_comma('9'), true);
            assert_eq!(is_digit_or_comma(','), true);
            assert_eq!(is_digit_or_comma('a'), false);
            assert_eq!(is_digit_or_comma('A'), false);
            assert_eq!(is_digit_or_comma('!'), false);
            assert_eq!(is_digit_or_comma('('), false);
            assert_eq!(is_digit_or_comma(')'), false);
        }
    }

    #[cfg(test)]
    mod unexpand_flags_tests {
        use super::*;

        #[test]
        fn test_unexpand_flags_new_default() {
            let app = ct_app();
            let matches = app.get_matches_from(vec!["unexpand"]);
            let flags = UnexpandFlags::new(&matches).unwrap();
            assert_eq!(flags.tabstops, vec![UNEXPAND_DEFAULT_TABSTOP]);
            assert_eq!(flags.remaining_mode, RemainingMode::None);
            assert_eq!(flags.files, vec!["-".to_string()]);
            assert_eq!(flags.is_a_flag, false);
            assert_eq!(flags.is_u_flag, true);
        }

        #[test]
        fn test_unexpand_flags_new_with_tabs() {
            let app = ct_app();
            let matches = app.get_matches_from(vec!["unexpand", "--tabs", "4,8,12"]);
            let flags = UnexpandFlags::new(&matches).unwrap();
            assert_eq!(flags.tabstops, vec![4, 8, 12]);
            assert_eq!(flags.remaining_mode, RemainingMode::None);
        }

        #[test]
        fn test_unexpand_flags_new_with_all_flag() {
            let app = ct_app();
            let matches = app.get_matches_from(vec!["unexpand", "--all"]);
            let flags = UnexpandFlags::new(&matches).unwrap();
            assert_eq!(flags.is_a_flag, true);
        }

        #[test]
        fn test_unexpand_flags_new_with_first_only_flag() {
            let app = ct_app();
            let matches = app.get_matches_from(vec!["unexpand", "--first-only"]);
            let flags = UnexpandFlags::new(&matches).unwrap();
            assert_eq!(flags.is_a_flag, false);
        }

        #[test]
        fn test_unexpand_flags_new_with_no_utf8_flag() {
            let app = ct_app();
            let matches = app.get_matches_from(vec!["unexpand", "--no-utf8"]);
            let flags = UnexpandFlags::new(&matches).unwrap();
            assert_eq!(flags.is_u_flag, false);
        }

        #[test]
        fn test_unexpand_flags_new_with_files() {
            let app = ct_app();
            let matches = app.get_matches_from(vec!["unexpand", "file1", "file2"]);
            let flags = UnexpandFlags::new(&matches).unwrap();
            assert_eq!(flags.files, vec!["file1".to_string(), "file2".to_string()]);
        }

        #[test]
        fn test_unexpand_flags_new_with_invalid_tabstops() {
            let app = ct_app();
            let matches = app.get_matches_from(vec!["unexpand", "--tabs", "4,x,12"]);
            let result = UnexpandFlags::new(&matches);
            assert!(result.is_err());
            assert_eq!(
                result.err(),
                Some(UnexpandParseError::InvalidCharacter("x".to_string()))
            );
        }

        #[test]
        fn test_unexpand_flags_new_with_zero_tabstops() {
            let app = ct_app();
            let matches = app.get_matches_from(vec!["unexpand", "--tabs", "4,0,12"]);
            let result = UnexpandFlags::new(&matches);
            assert!(result.is_err());
            assert_eq!(result.err(), Some(UnexpandParseError::TabSizeCannotBeZero));
        }

        #[test]
        fn test_unexpand_flags_new_with_non_ascending_tabstops() {
            let app = ct_app();
            let matches = app.get_matches_from(vec!["unexpand", "--tabs", "4,12,8"]);
            let result = UnexpandFlags::new(&matches);
            assert!(result.is_err());
            assert_eq!(
                result.err(),
                Some(UnexpandParseError::TabSizesMustBeAscending)
            );
        }

        #[test]
        fn test_unexpand_flags_new_with_too_large_tabstops() {
            let app = ct_app();
            let matches =
                app.get_matches_from(vec!["unexpand", "--tabs", "4,999999999999999999999,12"]);
            let result = UnexpandFlags::new(&matches);
            assert!(result.is_err());
            assert_eq!(
                result.err(),
                Some(UnexpandParseError::TabStopTooLarge(
                    "999999999999999999999".to_string()
                ))
            );
        }
    }
    #[test]
    fn test_unexpand_flags_new_with_combined_flags() {
        let app = ct_app();
        let matches = app.get_matches_from(vec![
            "unexpand",
            "--tabs",
            "4,8",
            "--all",
            "--no-utf8",
            "file1",
            "file2",
        ]);
        let flags = UnexpandFlags::new(&matches).unwrap();
        assert_eq!(flags.tabstops, vec![4, 8]);
        assert_eq!(flags.files, vec!["file1".to_string(), "file2".to_string()]);
        assert_eq!(flags.is_a_flag, true);
        assert_eq!(flags.is_u_flag, false);
    }

    #[test]
    fn test_unexpand_flags_new_with_tabs_and_all_but_not_first_only() {
        let app = ct_app();
        let matches = app.get_matches_from(vec![
            "unexpand",
            "--tabs",
            "4,8",
            "--all",
            "--first-only",
            "file1",
            "file2",
        ]);
        let flags = UnexpandFlags::new(&matches).unwrap();
        assert_eq!(flags.tabstops, vec![4, 8]);
        assert_eq!(flags.files, vec!["file1".to_string(), "file2".to_string()]);
        assert_eq!(flags.is_a_flag, false);
    }

    #[test]
    fn test_unexpand_flags_new_with_tabs_and_default_flags() {
        let app = ct_app();
        let matches = app.get_matches_from(vec!["unexpand", "--tabs", "4,8"]);
        let flags = UnexpandFlags::new(&matches).unwrap();
        assert_eq!(flags.tabstops, vec![4, 8]);
        assert_eq!(flags.files, vec!["-".to_string()]);
        assert_eq!(flags.is_a_flag, true);
        assert_eq!(flags.is_u_flag, true);
    }

    #[test]
    fn test_unexpand_flags_new_with_all_flags() {
        let app = ct_app();
        let matches = app.get_matches_from(vec![
            "unexpand",
            "--tabs",
            "4,8,12",
            "--all",
            "--first-only",
            "--no-utf8",
            "file1",
            "file2",
        ]);
        let flags = UnexpandFlags::new(&matches).unwrap();
        assert_eq!(flags.tabstops, vec![4, 8, 12]);
        assert_eq!(flags.files, vec!["file1".to_string(), "file2".to_string()]);
        assert_eq!(flags.is_a_flag, false); // Because --first-only is present
        assert_eq!(flags.is_u_flag, false);
    }

    #[test]
    fn test_unexpand_flags_new_with_default_file() {
        let app = ct_app();
        let matches = app.get_matches_from(vec!["unexpand", "--tabs", "4,8"]);
        let flags = UnexpandFlags::new(&matches).unwrap();
        assert_eq!(flags.files, vec!["-".to_string()]);
    }

    #[cfg(test)]
    mod tabstops_parse_tests {
        use super::*;

        #[test]
        fn test_unexpand_tabstops_parse_valid_input() {
            let input = "1,2,3,4,5";
            let expected = Ok((RemainingMode::None, vec![1, 2, 3, 4, 5]));
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }

        #[test]
        fn test_unexpand_tabstops_parse_invalid_character() {
            let input = "1,2,x,4,5";
            let expected = Err(UnexpandParseError::InvalidCharacter("x".to_string()));
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }

        #[test]
        fn test_unexpand_tabstops_parse_zero_value() {
            let input = "1,2,0,4,5";
            let expected = Err(UnexpandParseError::TabSizeCannotBeZero);
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }

        #[test]
        fn test_unexpand_tabstops_parse_non_ascending_values() {
            let input = "1,3,2,4,5";
            let expected = Err(UnexpandParseError::TabSizesMustBeAscending);
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }

        #[test]
        fn test_unexpand_tabstops_parse_too_large_value() {
            let input = "1,2,99999999999999999999999999,4,5";
            let expected = Err(UnexpandParseError::TabStopTooLarge(
                "99999999999999999999999999".to_string(),
            ));
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }

        #[test]
        fn test_unexpand_tabstops_parse_empty_input() {
            let input = "";
            let expected = Ok((RemainingMode::None, vec![UNEXPAND_DEFAULT_TABSTOP]));
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }

        #[test]
        fn test_unexpand_tabstops_parse_single_value() {
            let input = "5";
            let expected = Ok((RemainingMode::None, vec![5]));
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }

        #[test]
        fn test_unexpand_tabstops_parse_trailing_comma() {
            let input = "1,2,3,4,5,";
            let expected = Ok((RemainingMode::None, vec![1, 2, 3, 4, 5]));
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }

        #[test]
        fn test_unexpand_tabstops_parse_leading_comma() {
            let input = ",1,2,3,4,5";
            let expected = Ok((RemainingMode::None, vec![1, 2, 3, 4, 5]));
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }

        #[test]
        fn test_unexpand_tabstops_parse_multiple_commas() {
            let input = "1,,2,3,4,5";
            let expected = Ok((RemainingMode::None, vec![1, 2, 3, 4, 5]));
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }

        #[test]
        fn test_unexpand_tabstops_parse_spaces_in_values() {
            let input = "1, 2,3, 4,5";
            let expected = Ok((RemainingMode::None, vec![1, 2, 3, 4, 5]));
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }

        #[test]
        fn test_unexpand_tabstops_parse_spaces_around_commas() {
            let input = "1 ,2 ,3 ,4 ,5";
            let expected = Ok((RemainingMode::None, vec![1, 2, 3, 4, 5]));
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }

        #[test]
        fn test_unexpand_tabstops_parse_mixed_invalid_characters() {
            let input = "1,2,3,a4,5";
            let expected = Err(UnexpandParseError::InvalidCharacter("a4".to_string()));
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }

        #[test]
        fn test_unexpand_tabstops_parse_with_leading_zeros() {
            let input = "01,02,03,04,05";
            let expected = Ok((RemainingMode::None, vec![1, 2, 3, 4, 5]));
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }

        #[test]
        fn test_unexpand_tabstops_parse_large_numbers() {
            let input = "1,1000000000,2000000000,3000000000,4000000000";
            let expected = Ok((
                RemainingMode::None,
                vec![1, 1000000000, 2000000000, 3000000000, 4000000000],
            ));
            assert_eq!(unexpand_tabstops_parse(input, false), expected);
        }
    }

    #[cfg(test)]
    mod parse_error_tests {
        use super::*;

        #[test]
        fn test_invalid_character_display() {
            let error = UnexpandParseError::InvalidCharacter("x".to_string());
            assert_eq!(
                format!("{}", error),
                "tab size contains invalid character(s): 'x'"
            );
        }

        #[test]
        fn test_tab_size_cannot_be_zero_display() {
            let error = UnexpandParseError::TabSizeCannotBeZero;
            assert_eq!(format!("{}", error), "tab size cannot be 0");
        }

        #[test]
        fn test_tab_size_too_large_display() {
            let error = UnexpandParseError::TabStopValueTooLarge;
            assert_eq!(format!("{}", error), "tab stop value is too large");
        }

        #[test]
        fn test_tab_sizes_must_be_ascending_display() {
            let error = UnexpandParseError::TabSizesMustBeAscending;
            assert_eq!(format!("{}", error), "tab sizes must be ascending");
        }
    }

    #[cfg(test)]
    mod ct_app_tests {
        use std::fs;

        use clap::error::ErrorKind;

        use crate::is_digit_or_comma;

        use super::*;

        #[test]
        fn test_is_digit_or_comma() {
            assert!(is_digit_or_comma('1'));
            assert!(is_digit_or_comma(','));
            assert!(!is_digit_or_comma('a'));
        }

        // unexpand 接口测试: unexpand [OPTION]... [FILE]...
        //   -a, --all             convert all blanks, instead of just initial blanks
        //       --first-only      convert only leading sequences of blanks (overrides -a)
        //   -t, --tabs <N, LIST>  use comma separated LIST of tab positions or have tabs N characters apart instead of 8 (enables -a)
        //   -U, --no-utf8         interpret input file as 8-bit ASCII rather than UTF-8
        //   -h, --help            Print help
        //   -V, --version         Print version
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

        #[test]
        fn test_ct_app_long_option_file() {
            // Create a regular file for testing , 默认带文件
            let regular_file_path = "test_file";
            File::create(regular_file_path).expect("Failed to create regular file");

            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), regular_file_path];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());

            // Clean up: remove the regular file after the test
            fs::remove_file(regular_file_path).expect("Failed to remove regular file");
        }

        #[test]
        fn test_ct_app_long_option_all() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--all"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_first_only() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--first-only"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_tabs() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--tabs", "N, LIST"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_no_utf8() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--no-utf8"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_short_option_a() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-a"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_short_option_t() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-t", "N, LIST"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_short_option_uppercase_u() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-U"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }
    }
}
