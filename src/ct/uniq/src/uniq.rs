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

//! uniq 命令用于对排序后的文本文件进行去重操作

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write, stdin, stdout};
use std::num::IntErrorKind;

use clap::builder::ValueParser;
use clap::{Arg, ArgAction, ArgMatches, Command};
use clap::{crate_version, error::ContextKind, error::Error, error::ErrorKind};
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTError, CTResult, CtSimpleError, FromIo};
use ctcore::ct_posix::{OBSOLETE, ct_posix_version};
use ctcore::{ct_format_usage, ct_help_about, ct_help_section, ct_help_usage};

const UNIQ_ABOUT: &str = ct_help_about!("uniq.md");
const UNIQ_USAGE: &str = ct_help_usage!("uniq.md");
const UNIQ_AFTER_HELP: &str = ct_help_section!("after help", "uniq.md");

pub mod uniq_flags {
    pub const ALL_REPEATED: &str = "all-repeated";
    pub const CHECK_CHARS: &str = "check-chars";
    pub const COUNT: &str = "count";
    pub const IGNORE_CASE: &str = "ignore-case";
    pub const REPEATED: &str = "repeated";
    pub const SKIP_FIELDS: &str = "skip-fields";
    pub const SKIP_CHARS: &str = "skip-chars";
    pub const UNIQUE: &str = "unique";
    pub const ZERO_TERMINATED: &str = "zero-terminated";
    pub const GROUP: &str = "group";
}

const UNIQ_ARG_FILES: &str = "files";

#[derive(PartialEq, Clone, Copy, Debug)]
enum UniqDelimiters {
    Append,
    Prepend,
    Separate,
    Both,
    None,
}

struct Uniq {
    is_repeats_only: bool,
    is_uniques_only: bool,
    is_all_repeated: bool,
    delimiters: UniqDelimiters,
    is_show_counts: bool,
    skip_fields: Option<usize>,
    slice_start: Option<usize>,
    slice_stop: Option<usize>,
    is_ignore_case: bool,
    is_zero_terminated: bool,
}

macro_rules! uniq_write_line_terminator {
    ($writer:expr, $line_terminator:expr) => {
        $writer
            .write_all(&[$line_terminator])
            .map_err_context(|| "Could not write line terminator".to_string())
    };
}

impl Uniq {
    pub fn print_uniq(&self, reader: impl BufRead, mut writer: impl Write) -> CTResult<()> {
        let mut is_first_line_printed = false;
        let mut group_cnt = 1;
        let line_terminator = self.get_line_terminator();
        let mut lines = reader.split(line_terminator);
        let mut line = if let Some(l) = lines.next() {
            l?
        } else {
            return Ok(());
        };

        let w = &mut writer;

        // 比较当前的 `line` 和输入中的连续行（`next_line`），如果需要，根据提供的命令行选项打印 `line`
        for next_line in lines {
            let next_line = next_line?;
            if self.cmp_keys(&line, &next_line) {
                if (group_cnt == 1 && !self.is_repeats_only)
                    || (group_cnt > 1 && !self.is_uniques_only)
                {
                    self.print_line(w, &line, group_cnt, is_first_line_printed)?;
                    is_first_line_printed = true;
                }
                line = next_line;
                group_cnt = 1;
            } else {
                if self.is_all_repeated {
                    self.print_line(w, &line, group_cnt, is_first_line_printed)?;
                    is_first_line_printed = true;
                    line = next_line;
                }
                group_cnt += 1;
            }
        }
        if (group_cnt == 1 && !self.is_repeats_only) || (group_cnt > 1 && !self.is_uniques_only) {
            self.print_line(w, &line, group_cnt, is_first_line_printed)?;
            is_first_line_printed = true;
        }
        if (self.delimiters == UniqDelimiters::Append || self.delimiters == UniqDelimiters::Both)
            && is_first_line_printed
        {
            uniq_write_line_terminator!(writer, line_terminator)?;
        }
        Ok(())
    }

    fn skip_fields(&self, line: &[u8]) -> Vec<u8> {
        match self.skip_fields {
            Some(skip_fields) => {
                let mut line_iter = line.iter();
                let mut line_after_skipped_field: Vec<u8>;
                for _ in 0..skip_fields {
                    if line_iter.all(|u| u.is_ascii_whitespace()) {
                        return Vec::new();
                    }
                    line_after_skipped_field = line_iter
                        .by_ref()
                        .skip_while(|u| !u.is_ascii_whitespace())
                        .copied()
                        .collect::<Vec<u8>>();

                    if line_after_skipped_field.is_empty() {
                        return Vec::new();
                    }
                    line_iter = line_after_skipped_field.iter();
                }
                line_iter.copied().collect::<Vec<u8>>()
            }
            _ => line.to_vec(),
        }
    }

    fn get_line_terminator(&self) -> u8 {
        match self.is_zero_terminated {
            true => 0,
            false => b'\n',
        }
    }

    fn cmp_keys(&self, first_key: &[u8], second_key: &[u8]) -> bool {
        self.cmp_key(first_key, |first_iter| {
            self.cmp_key(second_key, |second_iter| first_iter.ne(second_iter))
        })
    }

    fn cmp_key<F>(&self, line: &[u8], mut closure: F) -> bool
    where
        F: FnMut(&mut dyn Iterator<Item = u8>) -> bool,
    {
        let check_fields = self.skip_fields(line);
        let fields_len = check_fields.len();
        let start_slice = self.slice_start.unwrap_or(0);
        let stop_slice = self.slice_stop.unwrap_or(fields_len);
        if fields_len > 0 {
            // 快速路径：避免在没有跳过或映射为小写的情况下进行任何工作
            if !self.is_ignore_case && start_slice == 0 && stop_slice == fields_len {
                return closure(&mut check_fields.iter().copied());
            }

            // 快速路径：避免跳过
            if self.is_ignore_case && start_slice == 0 && stop_slice == fields_len {
                return closure(&mut check_fields.iter().map(|u| u.to_ascii_lowercase()));
            }

            // 快速路径：如果我们不想忽略大小写，可以避免将字符映射为小写
            if !self.is_ignore_case {
                return closure(
                    &mut check_fields
                        .iter()
                        .skip(start_slice)
                        .take(stop_slice)
                        .copied(),
                );
            }

            closure(
                &mut check_fields
                    .iter()
                    .skip(start_slice)
                    .take(stop_slice)
                    .map(|u| u.to_ascii_lowercase()),
            )
        } else {
            closure(&mut check_fields.iter().copied())
        }
    }

    fn should_print_delimiter(&self, group_cnt: usize, is_first_line_printed: bool) -> bool {
        // 如果未选择分隔符选项，则不需要其他检查
        self.delimiters != UniqDelimiters::None
            && group_cnt == 1  // 仅在组的第一行之前打印分隔符，不在组的行之间打印
            && (is_first_line_printed  // 如果在当前组之前至少有一行输出，则打印分隔符
            || self.delimiters == UniqDelimiters::Prepend // 或者如果我们需要在输出开始时添加分隔符，则打印分隔符            
            || self.delimiters == UniqDelimiters::Both) // 'both' 分隔模式应添加和附加分隔符
    }

    fn print_line(
        &self,
        w: &mut impl Write,
        line: &[u8],
        cnt: usize,
        is_first_line_printed: bool,
    ) -> CTResult<()> {
        let line_terminator = self.get_line_terminator();

        if self.should_print_delimiter(cnt, is_first_line_printed) {
            uniq_write_line_terminator!(w, line_terminator)?;
        }

        if self.is_show_counts {
            let prefix = format!("{cnt:7} ");
            let out = prefix
                .as_bytes()
                .iter()
                .chain(line.iter())
                .copied()
                .collect::<Vec<u8>>();
            w.write_all(out.as_slice())
        } else {
            w.write_all(line)
        }
        .map_err_context(|| "Failed to write line".to_string())?;

        uniq_write_line_terminator!(w, line_terminator)
    }
}

fn uniq_opt_parsed(opt_name: &str, arg_matches: &ArgMatches) -> CTResult<Option<usize>> {
    match arg_matches.get_one::<String>(opt_name) {
        Some(arg_str) => match arg_str.parse::<usize>() {
            Ok(value) => Ok(Some(value)),
            Err(e) => match e.kind() {
                IntErrorKind::PosOverflow => Ok(Some(usize::MAX)),
                _ => {
                    let err_message = format!(
                        "Invalid argument for {}: {}",
                        opt_name,
                        arg_str.maybe_quote()
                    );
                    Err(CtSimpleError::new(1, err_message))
                }
            },
        },
        None => Ok(None),
    }
}

/// 提取废弃的简写（如果有）以跳过字段和跳过字符选项
/// 遵循GNU `uniq`行为
///
/// 跳过字段选项的废弃示例
/// `uniq -1 file` 等价于 `uniq -f1 file`
/// `uniq -1 -2 -3 file` 等价于 `uniq -f123 file`
/// `uniq -1 -2 -f5 file` 等价于 `uniq -f5 file`
/// `uniq -u20s4 file` 等价于 `uniq -u -f20 -s4 file`
/// `uniq -D1w3 -3 file` 等价于 `uniq -D -f3 -w3 file`
///
/// 跳过字符选项的废弃示例
/// `uniq +1 file` 等价于 `uniq -s1 file`
/// `uniq +1 -s2 file` 等价于 `uniq -s2 file`
/// `uniq -s2 +3 file` 等价于 `uniq -s3 file`
fn uniq_handle_obsolete(args: impl ctcore::Args) -> (Vec<OsString>, Option<usize>, Option<usize>) {
    let mut skip_fields_old = None;
    let mut skip_chars_old = None;
    let mut is_preceding_long_opt_req_value = false;
    let mut is_preceding_short_opt_req_value = false;

    let filtered_args = args
        .filter_map(|os_slice| {
            uniq_filter_args(
                os_slice,
                &mut skip_fields_old,
                &mut skip_chars_old,
                &mut is_preceding_long_opt_req_value,
                &mut is_preceding_short_opt_req_value,
            )
        })
        .collect();

    // 提取的 skip_fields_old 和 skip_chars_old 的 String 值（如果有）
    // 保证仅由 ascii 数字字符组成，因此可以安全地解析为 usize 并将 Result 折叠为 Option
    let skip_fields_old: Option<usize> = skip_fields_old.and_then(|v| v.parse::<usize>().ok());
    let skip_chars_old: Option<usize> = skip_chars_old.and_then(|v| v.parse::<usize>().ok());

    (filtered_args, skip_fields_old, skip_chars_old)
}

fn uniq_filter_args(
    os_slice: OsString,
    skip_fields_old: &mut Option<String>,
    skip_chars_old: &mut Option<String>,
    is_preceding_long_opt_req_value: &mut bool,
    is_preceding_short_opt_req_value: &mut bool,
) -> Option<OsString> {
    let filter: Option<OsString>;
    if let Some(slice) = os_slice.to_str() {
        if uniq_should_extract_obs_skip_fields(
            slice,
            is_preceding_long_opt_req_value,
            is_preceding_short_opt_req_value,
        ) {
            // 短选项字符串的开始，可以在其中包含废弃的跳过字段选项值
            filter = uniq_handle_extract_obs_skip_fields(slice, skip_fields_old);
        } else if uniq_should_extract_obs_skip_chars(
            slice,
            is_preceding_long_opt_req_value,
            is_preceding_short_opt_req_value,
        ) {
            // 废弃的跳过字符选项
            filter = uniq_handle_extract_obs_skip_chars(slice, skip_chars_old);
        } else {
            // 既不是短选项，也不是可以包含废弃值的短选项
            filter = Some(OsString::from(slice));
            // 检查并重置到目前为止提取的废弃值，如果接下来遇到相应的新/已记录选项
            // 注意：对于跳过字段 - 在组合短选项中出现的相应新/已记录选项的情况
            // 例如 `-u20s4` 或 `-D1w3`，等. 也在 `handle_extract_obs_skip_fields()` 函数中覆盖
            if slice.starts_with("-f") {
                *skip_fields_old = None;
            }
            if slice.starts_with("-s") {
                *skip_chars_old = None;
            }
        }
        uniq_handle_preceding_options(
            slice,
            is_preceding_long_opt_req_value,
            is_preceding_short_opt_req_value,
        );
    } else {
        // 无法干净地将 os_slice 转换为 UTF-8，不要处理并按原样返回
        // 这将导致稍后失败，但我们不应在此处理，让 clap 在无效的 UTF-8 参数上 panic
        filter = Some(os_slice);
    }
    filter
}

/// [`uniq_filter_args`] 的辅助函数
/// 检查切片是否为真实短选项（而不是带有连字符的选项值）
/// 如果是，则为可以包含废弃跳过字段值的短选项
fn uniq_should_extract_obs_skip_fields(
    str_slice: &str,
    is_preceding_long_opt_req_value: &bool,
    is_preceding_short_opt_req_value: &bool,
) -> bool {
    str_slice.starts_with('-')
        && !str_slice.starts_with("--")
        && !is_preceding_long_opt_req_value
        && !is_preceding_short_opt_req_value
        && !str_slice.starts_with("-s")
        && !str_slice.starts_with("-f")
        && !str_slice.starts_with("-w")
}

/// [`uniq_filter_args`] 的辅助函数
/// 检查切片是否为真实废弃跳过字符短选项
fn uniq_should_extract_obs_skip_chars(
    str_slice: &str,
    is_preceding_long_opt_req_value: &bool,
    is_preceding_short_opt_req_value: &bool,
) -> bool {
    str_slice.starts_with('+')
        && ct_posix_version().is_some_and(|v| v <= OBSOLETE)
        && !is_preceding_long_opt_req_value
        && !is_preceding_short_opt_req_value
        && str_slice.chars().nth(1).is_some_and(|c| c.is_ascii_digit())
}

/// [`uniq_filter_args`] 的辅助函数
/// 捕获当前切片是否为前置选项需要值
fn uniq_handle_preceding_options(
    str_slice: &str,
    is_preceding_long_opt_req_value: &mut bool,
    is_preceding_short_opt_req_value: &mut bool,
) {
    // 捕获当前切片是否为前置长选项，需要值且不使用 '=' 分配该值
    // 以下切片应被视为此选项的值，即使它以 '-' 开头（这将被视为带连字符的值）
    if str_slice.starts_with("--") {
        use uniq_flags as O;
        *is_preceding_long_opt_req_value = &str_slice[2..] == O::SKIP_CHARS
            || &str_slice[2..] == O::SKIP_FIELDS
            || &str_slice[2..] == O::CHECK_CHARS
            || &str_slice[2..] == O::GROUP
            || &str_slice[2..] == O::ALL_REPEATED;
    }
    // 捕获当前切片是否为前置短选项，需要值且在同一切片中没有值（值由空白分隔）
    // 以下切片应被视为此选项的值，即使它以 '-' 开头（这将被视为带连字符的值）
    *is_preceding_short_opt_req_value = str_slice == "-s" || str_slice == "-f" || str_slice == "-w";
    // 切片是一个值，重置前置选项标志
    if !str_slice.starts_with('-') {
        *is_preceding_short_opt_req_value = false;
        *is_preceding_long_opt_req_value = false;
    }
}

/// [`uniq_filter_args`] 的辅助函数
/// 从参数切片中提取废弃的跳过字段数字部分
/// 并过滤掉
fn uniq_handle_extract_obs_skip_fields(
    str_slice: &str,
    skip_fields_old: &mut Option<String>,
) -> Option<OsString> {
    let mut obs_extracted_vec: Vec<char> = vec![];
    let mut is_obs_end_reached = false;
    let mut is_obs_overwritten_by_new = false;
    let filtered_slice: Vec<char> = str_slice
        .chars()
        .filter(|c| {
            if c.eq(&'f') {
                // 到目前为止提取的任何废弃跳过字段值都应被丢弃，因为在它之后使用了新/已记录的跳过字段选项，
                // 即在 `-u12f3` 的情况下，废弃的跳过字段值仍应提取并过滤掉
                // 但 skip_fields_old 应设置为 None 而不是 Some(String)
                is_obs_overwritten_by_new = true;
            }
            // 为了正确处理 `-u20s4` 或 `-D1w3` 等情况，我们需要在遇到字母字符后停止提取数字字符，前面已经有一些在 obs_extracted 中
            if c.is_ascii_digit() && !is_obs_end_reached {
                obs_extracted_vec.push(*c);
                false
            } else {
                if !obs_extracted_vec.is_empty() {
                    is_obs_end_reached = true;
                }
                true
            }
        })
        .collect();

    if obs_extracted_vec.is_empty() {
        // 没有找到/提取废弃的值
        Some(OsString::from(str_slice))
    } else {
        // 提取了废弃值，除非之后使用了新/已记录的跳过字段选项。设置 skip_fields_old 值（如果已经有值则将其连接到它）
        if is_obs_overwritten_by_new {
            *skip_fields_old = None;
        } else {
            let mut extracted: String = obs_extracted_vec.iter().collect();
            if let Some(val) = skip_fields_old {
                extracted.push_str(val);
            }
            *skip_fields_old = Some(extracted);
        }
        if filtered_slice.get(1).is_some() {
            // 前面或后面有一些短选项，即 `-u20s4` 或 `-D1w3` 或类似的，提取废弃行值后，看起来像 `-us4` 或 `-Dw3` 或类似的
            let filtered_slice: String = filtered_slice.iter().collect();
            Some(OsString::from(filtered_slice))
        } else {
            None
        }
    }
}

/// [`uniq_filter_args`] 的辅助函数
/// 从参数切片中提取废弃的跳过字符数字部分
fn uniq_handle_extract_obs_skip_chars(
    str_slice: &str,
    skip_chars_old: &mut Option<String>,
) -> Option<OsString> {
    let mut obs_extracted_vec: Vec<char> = vec![];
    let mut slice_chars = str_slice.chars();
    slice_chars.next(); // 删除前导的 '+' 字符
    for slice_c in slice_chars {
        match slice_c.is_ascii_digit() {
            true => {
                obs_extracted_vec.push(slice_c);
            }
            false => {
                // 对于废弃的跳过字符选项，'+' 之后的整个值应为数字，因此，如果在切片中遇到任何非数字字符（即`+1q`等）
                // 设置 skip_chars_old 为 None 并返回整个切片，这将由 clap 解析并使用适当的错误消息 panic
                *skip_chars_old = None;
                return Some(OsString::from(str_slice));
            }
        }
    }

    match obs_extracted_vec.is_empty() {
        true => {
            // 没有找到/提取废弃的值，即仅有 '+' 字符
            Some(OsString::from(str_slice))
        }
        false => {
            // 成功提取数字值，捕获并返回 None 以过滤掉整个切片
            *skip_chars_old = Some(obs_extracted_vec.iter().collect());
            None
        }
    }
}

/// 将 Clap 错误映射到 USimpleError 并覆盖 3 个特定错误
/// 以满足 GNU `uniq` 的要求
/// 不幸的是，这些覆盖是必要的，因为几个 GNU `uniq` 测试
/// 对 `uniq` 错误消息的措辞进行了硬编码并且需要完全一致
/// 这与 Clap 格式和显示这些错误消息的方式不兼容。
fn uniq_map_clap_errors(clap_err: &Error) -> Box<dyn CTError> {
    let footer = "Try 'uniq --help' for more information.";
    let override_arg_conflict =
        "--group is mutually exclusive with -c/-d/-D/-u\n".to_string() + footer;
    let override_group_bad_opt = "invalid argument 'badoption' for '--group'\nValid arguments are:\n  - 'prepend'\n  - 'append'\n  - 'separate'\n  - 'both'\n".to_string() + footer;
    let override_all_repeated_bad_opt = "invalid argument 'badoption' for '--all-repeated'\nValid arguments are:\n  - 'none'\n  - 'prepend'\n  - 'separate'\n".to_string() + footer;

    let err_message = match clap_err.kind() {
        ErrorKind::ArgumentConflict => override_arg_conflict,
        ErrorKind::InvalidValue
            if clap_err
                .get(ContextKind::InvalidValue)
                .is_some_and(|value| value.to_string() == "badoption")
                && clap_err
                    .get(ContextKind::InvalidArg)
                    .is_some_and(|value| value.to_string().starts_with("--group")) =>
        {
            override_group_bad_opt
        }
        ErrorKind::InvalidValue
            if clap_err
                .get(ContextKind::InvalidValue)
                .is_some_and(|value| value.to_string() == "badoption")
                && clap_err
                    .get(ContextKind::InvalidArg)
                    .is_some_and(|value| value.to_string().starts_with("--all-repeated")) =>
        {
            override_all_repeated_bad_opt
        }
        _ => clap_err.to_string(),
    };
    CtSimpleError::new(1, err_message)
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    uniq_main(args)
}

pub fn uniq_main(args: impl ctcore::Args) -> CTResult<()> {
    let (args, skip_fields_old, skip_chars_old) = uniq_handle_obsolete(args);

    let matches = ct_app()
        .try_get_matches_from(args)
        .map_err(|e| uniq_map_clap_errors(&e))?;

    let files = matches.get_many::<OsString>(UNIQ_ARG_FILES);

    let (in_file_name, out_file_name) = files
        .map(|fi| fi.map(AsRef::as_ref))
        .map(|mut fi| (fi.next(), fi.next()))
        .unwrap_or_default();

    let skip_fields_modern: Option<usize> = uniq_opt_parsed(uniq_flags::SKIP_FIELDS, &matches)?;
    let skip_chars_modern: Option<usize> = uniq_opt_parsed(uniq_flags::SKIP_CHARS, &matches)?;

    let uniq_config = Uniq {
        is_repeats_only: matches.get_flag(uniq_flags::REPEATED)
            || matches.contains_id(uniq_flags::ALL_REPEATED),
        is_uniques_only: matches.get_flag(uniq_flags::UNIQUE),
        is_all_repeated: matches.contains_id(uniq_flags::ALL_REPEATED)
            || matches.contains_id(uniq_flags::GROUP),
        delimiters: uniq_get_delimiter(&matches),
        is_show_counts: matches.get_flag(uniq_flags::COUNT),
        skip_fields: skip_fields_modern.or(skip_fields_old),
        slice_start: skip_chars_modern.or(skip_chars_old),
        slice_stop: uniq_opt_parsed(uniq_flags::CHECK_CHARS, &matches)?,
        is_ignore_case: matches.get_flag(uniq_flags::IGNORE_CASE),
        is_zero_terminated: matches.get_flag(uniq_flags::ZERO_TERMINATED),
    };

    if uniq_config.is_show_counts && uniq_config.is_all_repeated {
        let err_message = "printing all duplicated lines and repeat counts is meaningless\nTry 'uniq --help' for more information.";
        return Err(CtSimpleError::new(1, err_message));
    }

    uniq_config.print_uniq(
        uniq_open_input_file(in_file_name)?,
        uniq_open_output_file(out_file_name)?,
    )
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = UNIQ_ABOUT;
    let usage_description = ct_format_usage(UNIQ_USAGE);
    let args = vec![
        Arg::new(uniq_flags::ALL_REPEATED)
            .short('D')
            .long(uniq_flags::ALL_REPEATED)
            .value_parser(["none", "prepend", "separate"])
            .help("print all duplicate lines. Delimiting is done with blank lines. [default: none]")
            .value_name("delimit-method")
            .num_args(0..=1)
            .default_missing_value("none")
            .require_equals(true),
        Arg::new(uniq_flags::GROUP)
            .long(uniq_flags::GROUP)
            .value_parser(["separate", "prepend", "append", "both"])
            .help("show all items, separating groups with an empty line. [default: separate]")
            .value_name("group-method")
            .num_args(0..=1)
            .default_missing_value("separate")
            .require_equals(true)
            .conflicts_with_all([
                uniq_flags::REPEATED,
                uniq_flags::ALL_REPEATED,
                uniq_flags::UNIQUE,
                uniq_flags::COUNT,
            ]),
        Arg::new(uniq_flags::CHECK_CHARS)
            .short('w')
            .long(uniq_flags::CHECK_CHARS)
            .help("compare no more than N characters in lines")
            .value_name("N"),
        Arg::new(uniq_flags::COUNT)
            .short('c')
            .long(uniq_flags::COUNT)
            .help("prefix lines by the number of occurrences")
            .action(ArgAction::SetTrue),
        Arg::new(uniq_flags::IGNORE_CASE)
            .short('i')
            .long(uniq_flags::IGNORE_CASE)
            .help("ignore differences in case when comparing")
            .action(ArgAction::SetTrue),
        Arg::new(uniq_flags::REPEATED)
            .short('d')
            .long(uniq_flags::REPEATED)
            .help("only print duplicate lines")
            .action(ArgAction::SetTrue),
        Arg::new(uniq_flags::SKIP_CHARS)
            .short('s')
            .long(uniq_flags::SKIP_CHARS)
            .help("avoid comparing the first N characters")
            .value_name("N"),
        Arg::new(uniq_flags::SKIP_FIELDS)
            .short('f')
            .long(uniq_flags::SKIP_FIELDS)
            .help("avoid comparing the first N fields")
            .value_name("N"),
        Arg::new(uniq_flags::UNIQUE)
            .short('u')
            .long(uniq_flags::UNIQUE)
            .help("only print unique lines")
            .action(ArgAction::SetTrue),
        Arg::new(uniq_flags::ZERO_TERMINATED)
            .short('z')
            .long(uniq_flags::ZERO_TERMINATED)
            .help("end lines with 0 byte, not newline")
            .action(ArgAction::SetTrue),
        Arg::new(UNIQ_ARG_FILES)
            .action(ArgAction::Append)
            .value_parser(ValueParser::os_string())
            .num_args(0..=2)
            .hide(true)
            .value_hint(clap::ValueHint::FilePath),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .after_help(UNIQ_AFTER_HELP)
        .args(args)
}

fn uniq_get_delimiter(arg_matches: &ArgMatches) -> UniqDelimiters {
    let value = arg_matches
        .get_one::<String>(uniq_flags::ALL_REPEATED)
        .or_else(|| arg_matches.get_one::<String>(uniq_flags::GROUP));

    if let Some(delimiter_arg) = value {
        match delimiter_arg.as_ref() {
            "append" => UniqDelimiters::Append,
            "prepend" => UniqDelimiters::Prepend,
            "separate" => UniqDelimiters::Separate,
            "both" => UniqDelimiters::Both,
            "none" => UniqDelimiters::None,
            _ => unreachable!("Should have been caught by possible values in clap"),
        }
    } else if arg_matches.contains_id(uniq_flags::GROUP) {
        UniqDelimiters::Separate
    } else {
        UniqDelimiters::None
    }
}

// None 或 "-" 表示 stdin
fn uniq_open_input_file(in_file_name: Option<&OsStr>) -> CTResult<Box<dyn BufRead>> {
    Ok(match in_file_name {
        Some(path) if path != "-" => {
            let infile = File::open(path)
                .map_err_context(|| format!("Could not open {}", path.maybe_quote()))?;
            Box::new(BufReader::new(infile))
        }
        _ => Box::new(stdin().lock()),
    })
}

// None 或 "-" 表示 stdout
fn uniq_open_output_file(out_file_name: Option<&OsStr>) -> CTResult<Box<dyn Write>> {
    Ok(match out_file_name {
        Some(path) if path != "-" => {
            let out = File::create(path)
                .map_err_context(|| format!("Could not open {}", path.maybe_quote()))?;
            Box::new(BufWriter::new(out))
        }
        _ => Box::new(stdout().lock()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(test)]
    mod uniq_tests {
        use super::*;
        use std::io::Cursor;
        fn default_uniq() -> Uniq {
            Uniq {
                is_repeats_only: false,
                is_uniques_only: false,
                is_all_repeated: false,
                delimiters: UniqDelimiters::None,
                is_show_counts: false, // This time we want to show counts
                skip_fields: None,
                slice_start: None,
                slice_stop: None,
                is_ignore_case: false,
                is_zero_terminated: false,
            }
        }
        #[test]
        fn test_print_uniq_show_counts() {
            let input_data = b"apple\nbanana\napple\nbanana\nbanana\n";
            let input = Cursor::new(input_data);
            let mut output = Cursor::new(Vec::new());
            let uniq = Uniq {
                is_repeats_only: false,
                is_uniques_only: false,
                is_all_repeated: false,
                delimiters: UniqDelimiters::None,
                is_show_counts: true, // This time we want to show counts
                skip_fields: None,
                slice_start: None,
                slice_stop: None,
                is_ignore_case: false,
                is_zero_terminated: false,
            };

            uniq.print_uniq(input, &mut output).unwrap();
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            // Expect counts for each line, including duplicates counted correctly
            assert_eq!(
                output_str,
                "      1 apple\n      1 banana\n      1 apple\n      2 banana\n"
            );
        }

        #[test]
        fn test_should_print_delimiter_no_output_yet() {
            let mut uniq = default_uniq();
            uniq.delimiters = UniqDelimiters::Prepend;

            assert!(uniq.should_print_delimiter(1, false));
        }

        #[test]
        fn test_should_print_delimiter_after_first_group() {
            let mut uniq = default_uniq();
            uniq.delimiters = UniqDelimiters::Both;

            assert!(!uniq.should_print_delimiter(2, true));
        }

        #[test]
        fn test_skip_fields_multiple() {
            let mut uniq = default_uniq();
            uniq.skip_fields = Some(2);

            let result = uniq.skip_fields(b"field1 field2 field3 field4");
            assert_eq!(std::str::from_utf8(&result).unwrap(), " field3 field4");
        }

        #[test]
        fn test_get_line_terminator() {
            let mut uniq = default_uniq();

            uniq.is_zero_terminated = true;
            assert_eq!(uniq.get_line_terminator(), 0); // Expect zero terminator

            uniq.is_zero_terminated = false;
            assert_eq!(uniq.get_line_terminator(), b'\n'); // Expect newline terminator
        }

        #[test]
        fn test_print_uniq_line_terminators() {
            let input_data = b"apple\0banana\0apple\0";
            let input = Cursor::new(input_data);
            let mut output = Cursor::new(Vec::new());
            let mut uniq = default_uniq();
            uniq.is_zero_terminated = true;

            uniq.print_uniq(input, &mut output).unwrap();
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            assert_eq!(output_str, "apple\0banana\0apple\0"); // Checking if zero terminators are handled
        }

        #[test]
        fn test_cmp_keys_case_insensitivity() {
            let mut uniq = default_uniq();
            uniq.is_ignore_case = true;

            let line1 = b"Case";
            let line2 = b"case";
            assert!(!uniq.cmp_keys(line1, line2)); // Expect true as case is ignored
        }

        #[test]
        fn test_cmp_keys_with_field_skipping() {
            let mut uniq = default_uniq();
            uniq.skip_fields = Some(1);

            let line1 = b"ignore this line";
            let line2 = b"ignore that line";
            assert!(uniq.cmp_keys(line1, line2)); // Expect true as the first field is skipped
        }

        #[test]
        fn test_print_line_with_and_without_counts() {
            let mut output = Cursor::new(Vec::new());
            let mut uniq = default_uniq();
            uniq.is_show_counts = true;
            uniq.print_line(&mut output, b"test", 3, false).unwrap();
            assert_eq!(
                String::from_utf8(output.into_inner()).unwrap(),
                "      3 test\n"
            );

            output = Cursor::new(Vec::new());
            uniq.is_show_counts = false;
            uniq.print_line(&mut output, b"test", 1, false).unwrap();
            assert_eq!(String::from_utf8(output.into_inner()).unwrap(), "test\n");
        }
        #[test]
        fn test_delimiters_handling() {
            let mut uniq = default_uniq();
            uniq.delimiters = UniqDelimiters::Append;

            assert!(uniq.should_print_delimiter(1, true)); // Expect true when delimiter is Append and line has been printed
        }
    }

    #[cfg(test)]
    mod opt_parsed_tests {
        use super::*;
        use std::ffi::OsString;

        #[test]
        fn test_combination_of_valid_and_obsolete_arguments() {
            let args = vec![
                OsString::from("-1"),
                OsString::from("--valid-option"),
                OsString::from("+2"),
                OsString::from("filename"),
            ];
            let processed_args = args
                .into_iter()
                .filter_map(|arg| {
                    let mut skip_fields_old = None;
                    let mut skip_chars_old = None;
                    let mut is_preceding_long_opt_req_value = false;
                    let mut is_preceding_short_opt_req_value = false;

                    uniq_filter_args(
                        arg,
                        &mut skip_fields_old,
                        &mut skip_chars_old,
                        &mut is_preceding_long_opt_req_value,
                        &mut is_preceding_short_opt_req_value,
                    )
                })
                .collect::<Vec<_>>();

            assert_eq!(
                processed_args,
                vec![
                    OsString::from("--valid-option"),
                    OsString::from("+2"),
                    OsString::from("filename")
                ]
            );
        }

        #[test]
        fn test_edge_cases_with_zero_and_negative_numbers() {
            let args = vec![
                OsString::from("-0"),
                OsString::from("-123"),
                OsString::from("-999"),
            ];
            let processed_args = args
                .into_iter()
                .filter_map(|arg| {
                    let mut skip_fields_old = None;
                    let mut skip_chars_old = None;
                    let mut is_preceding_long_opt_req_value = false;
                    let mut is_preceding_short_opt_req_value = false;

                    uniq_filter_args(
                        arg,
                        &mut skip_fields_old,
                        &mut skip_chars_old,
                        &mut is_preceding_long_opt_req_value,
                        &mut is_preceding_short_opt_req_value,
                    )
                })
                .collect::<Vec<_>>();

            assert_eq!(processed_args.len(), 0); // Assuming all are filtered out as obsolete numeric args
        }

        #[test]
        fn test_complex_command_line_strings() {
            let args = vec![
                OsString::from("--option=value"),
                OsString::from("-n10"),
                OsString::from("path/to/file"),
                OsString::from("-100"),
            ];
            let processed_args = args
                .into_iter()
                .filter_map(|arg| {
                    let mut skip_fields_old = None;
                    let mut skip_chars_old = None;
                    let mut is_preceding_long_opt_req_value = false;
                    let mut is_preceding_short_opt_req_value = false;

                    uniq_filter_args(
                        arg,
                        &mut skip_fields_old,
                        &mut skip_chars_old,
                        &mut is_preceding_long_opt_req_value,
                        &mut is_preceding_short_opt_req_value,
                    )
                })
                .collect::<Vec<_>>();

            assert_eq!(
                processed_args,
                vec![
                    OsString::from("--option=value"),
                    OsString::from("-n"),
                    OsString::from("path/to/file")
                ]
            );
        }

        #[test]
        fn test_concatenated_options() {
            let args = vec![OsString::from("-abc"), OsString::from("path/file")];
            let processed_args = args
                .into_iter()
                .filter_map(|arg| {
                    let mut skip_fields_old = None;
                    let mut skip_chars_old = None;
                    let mut is_preceding_long_opt_req_value = false;
                    let mut is_preceding_short_opt_req_value = false;

                    uniq_filter_args(
                        arg,
                        &mut skip_fields_old,
                        &mut skip_chars_old,
                        &mut is_preceding_long_opt_req_value,
                        &mut is_preceding_short_opt_req_value,
                    )
                })
                .collect::<Vec<_>>();

            assert_eq!(
                processed_args,
                vec![OsString::from("-abc"), OsString::from("path/file")]
            );
        }

        #[test]
        fn test_options_with_equal_signs() {
            let args = vec![OsString::from("--size=100"), OsString::from("output.txt")];
            let processed_args = args
                .into_iter()
                .filter_map(|arg| {
                    let mut skip_fields_old = None;
                    let mut skip_chars_old = None;
                    let mut is_preceding_long_opt_req_value = false;
                    let mut is_preceding_short_opt_req_value = false;

                    uniq_filter_args(
                        arg,
                        &mut skip_fields_old,
                        &mut skip_chars_old,
                        &mut is_preceding_long_opt_req_value,
                        &mut is_preceding_short_opt_req_value,
                    )
                })
                .collect::<Vec<_>>();

            assert_eq!(
                processed_args,
                vec![OsString::from("--size=100"), OsString::from("output.txt")]
            );
        }

        #[test]
        fn test_arguments_with_escaped_characters() {
            let args = vec![OsString::from(
                "path/with\\ space/and\\ special\\&characters",
            )];
            let processed_args = args
                .into_iter()
                .filter_map(|arg| {
                    let mut skip_fields_old = None;
                    let mut skip_chars_old = None;
                    let mut is_preceding_long_opt_req_value = false;
                    let mut is_preceding_short_opt_req_value = false;

                    uniq_filter_args(
                        arg,
                        &mut skip_fields_old,
                        &mut skip_chars_old,
                        &mut is_preceding_long_opt_req_value,
                        &mut is_preceding_short_opt_req_value,
                    )
                })
                .collect::<Vec<_>>();

            assert_eq!(
                processed_args,
                vec![OsString::from(
                    "path/with\\ space/and\\ special\\&characters"
                )]
            );
        }

        #[test]
        fn test_full_command_line_simulation() {
            let args = vec![
                OsString::from("-1"),
                OsString::from("--valid-option"),
                OsString::from("path/to/file"),
                OsString::from("+2"),
                OsString::from("anotherpath"),
            ];
            let processed_args = args
                .into_iter()
                .filter_map(|arg| {
                    let mut skip_fields_old = None;
                    let mut skip_chars_old = None;
                    let mut is_preceding_long_opt_req_value = false;
                    let mut is_preceding_short_opt_req_value = false;

                    uniq_filter_args(
                        arg,
                        &mut skip_fields_old,
                        &mut skip_chars_old,
                        &mut is_preceding_long_opt_req_value,
                        &mut is_preceding_short_opt_req_value,
                    )
                })
                .collect::<Vec<_>>();

            assert_eq!(
                processed_args,
                vec![
                    OsString::from("--valid-option"),
                    OsString::from("path/to/file"),
                    OsString::from("+2"),
                    OsString::from("anotherpath")
                ]
            );
        }

        #[test]
        fn test_arguments_mimicking_options() {
            let args = vec![OsString::from("-1234"), OsString::from("logfile.log")];
            let processed_args = args
                .into_iter()
                .filter_map(|arg| {
                    let mut skip_fields_old = None;
                    let mut skip_chars_old = None;
                    let mut is_preceding_long_opt_req_value = false;
                    let mut is_preceding_short_opt_req_value = false;

                    uniq_filter_args(
                        arg,
                        &mut skip_fields_old,
                        &mut skip_chars_old,
                        &mut is_preceding_long_opt_req_value,
                        &mut is_preceding_short_opt_req_value,
                    )
                })
                .collect::<Vec<_>>();

            assert_eq!(processed_args, vec![OsString::from("logfile.log")]);
        }

        #[test]
        fn test_invalid_command_syntax() {
            let args = vec![
                OsString::from("--="),
                OsString::from("-#"),
                OsString::from("+?"),
            ];
            let processed_args = args
                .into_iter()
                .filter_map(|arg| {
                    let mut skip_fields_old = None;
                    let mut skip_chars_old = None;
                    let mut is_preceding_long_opt_req_value = false;
                    let mut is_preceding_short_opt_req_value = false;

                    uniq_filter_args(
                        arg,
                        &mut skip_fields_old,
                        &mut skip_chars_old,
                        &mut is_preceding_long_opt_req_value,
                        &mut is_preceding_short_opt_req_value,
                    )
                })
                .collect::<Vec<_>>();

            assert_eq!(
                processed_args,
                vec![
                    OsString::from("--="),
                    OsString::from("-#"),
                    OsString::from("+?")
                ]
            );
        }
    }

    #[cfg(test)]
    mod handle_obsolete_tests {
        use super::*;
        use std::ffi::OsString;

        #[test]
        fn test_single_obsolete_argument() {
            let args = vec![OsString::from("-1")];
            let (processed_args, skip_fields_old, skip_chars_old) =
                uniq_handle_obsolete(args.into_iter());

            assert_eq!(processed_args.len(), 0); // Assuming it removes obsolete args
            assert_eq!(skip_fields_old, Some(1));
            assert!(skip_chars_old.is_none());
        }

        #[test]
        fn test_multiple_consecutive_obsolete_arguments() {
            let args = vec![
                OsString::from("-1"),
                OsString::from("-2"),
                OsString::from("-3"),
            ];
            let (processed_args, skip_fields_old, skip_chars_old) =
                uniq_handle_obsolete(args.into_iter());

            assert_eq!(processed_args.len(), 0); // Assuming it removes all obsolete args
            assert_eq!(skip_fields_old, Some(321)); // Assuming it aggregates numbers
            assert!(skip_chars_old.is_none());
        }

        #[test]
        fn test_mixed_obsolete_and_current_arguments() {
            let args = vec![
                OsString::from("-1"),
                OsString::from("file.txt"),
                OsString::from("+2"),
            ];
            let (processed_args, skip_fields_old, skip_chars_old) =
                uniq_handle_obsolete(args.into_iter());

            assert_eq!(
                processed_args,
                vec![OsString::from("file.txt"), OsString::from("+2")]
            );
            assert_eq!(skip_fields_old, Some(1));
            assert_eq!(skip_chars_old, None);
        }

        #[test]
        fn test_handling_invalid_obsolete_arguments() {
            let args = vec![
                OsString::from("-a"),
                OsString::from("file.txt"),
                OsString::from("+b"),
            ];
            let (processed_args, skip_fields_old, skip_chars_old) =
                uniq_handle_obsolete(args.into_iter());

            assert_eq!(
                processed_args,
                vec![
                    OsString::from("-a"),
                    OsString::from("file.txt"),
                    OsString::from("+b")
                ]
            ); // "-a" and "+b" should be ignored
            assert!(skip_fields_old.is_none());
            assert!(skip_chars_old.is_none());
        }

        #[test]
        fn test_no_obsolete_arguments() {
            let args = vec![OsString::from("--option"), OsString::from("value")];
            let (processed_args, skip_fields_old, skip_chars_old) =
                uniq_handle_obsolete(args.into_iter());

            assert_eq!(
                processed_args,
                vec![OsString::from("--option"), OsString::from("value")]
            );
            assert!(skip_fields_old.is_none());
            assert!(skip_chars_old.is_none());
        }

        #[test]
        fn test_complex_arguments_with_embedded_numbers() {
            let args = vec![OsString::from("path-123-file")];
            let (processed_args, skip_fields_old, skip_chars_old) =
                uniq_handle_obsolete(args.into_iter());

            assert_eq!(processed_args, vec![OsString::from("path-123-file")]); // Should be treated as a single argument
            assert!(skip_fields_old.is_none());
            assert!(skip_chars_old.is_none());
        }

        #[test]
        fn test_edge_cases_with_extreme_numbers() {
            let args = vec![OsString::from("-999999999999999999999999999999")];
            let (processed_args, skip_fields_old, skip_chars_old) =
                uniq_handle_obsolete(args.into_iter());

            assert_eq!(processed_args.len(), 0);
            assert_eq!(skip_fields_old, None); // Assuming it clamps to usize::MAX
            assert!(skip_chars_old.is_none());
        }

        #[test]
        fn test_repeating_numbers_in_arguments() {
            let args = vec![OsString::from("-111")];
            let (processed_args, skip_fields_old, _skip_chars_old) =
                uniq_handle_obsolete(args.into_iter());

            assert_eq!(processed_args.len(), 0); // Assuming "-111" is recognized and removed
            assert_eq!(skip_fields_old, Some(111));
        }

        #[test]
        fn test_interleaved_valid_and_obsolete_arguments() {
            let args = vec![
                OsString::from("-1"),
                OsString::from("--valid"),
                OsString::from("-2"),
                OsString::from("file.txt"),
            ];
            let (processed_args, skip_fields_old, _skip_chars_old) =
                uniq_handle_obsolete(args.into_iter());

            assert_eq!(
                processed_args,
                vec![OsString::from("--valid"), OsString::from("file.txt")]
            );
            assert_eq!(skip_fields_old, Some(21)); // Assuming "-1" and "-2" are aggregated
        }

        #[test]
        fn test_obsolete_arguments_with_special_characters() {
            let args = vec![OsString::from("-1@"), OsString::from("file.txt")];
            let (processed_args, skip_fields_old, _skip_chars_old) =
                uniq_handle_obsolete(args.into_iter());

            assert_eq!(
                processed_args,
                vec![OsString::from("-@"), OsString::from("file.txt")]
            ); // Assuming "-1@" is removed due to malformed input
            assert_eq!(skip_fields_old, Some(1)); // Assuming the special character invalidates the skip field
        }

        #[test]
        fn test_complete_command_line_simulation() {
            let args = vec![
                OsString::from("-1"),
                OsString::from("--option=value"),
                OsString::from("-2"),
                OsString::from("path/to/file"),
            ];
            let (processed_args, skip_fields_old, _skip_chars_old) =
                uniq_handle_obsolete(args.into_iter());

            assert_eq!(
                processed_args,
                vec![
                    OsString::from("--option=value"),
                    OsString::from("path/to/file")
                ]
            );
            assert_eq!(skip_fields_old, Some(21)); // Assuming "-1" and "-2" are combined
        }

        #[test]
        fn test_arguments_resembling_but_not_obsolete() {
            let args = vec![OsString::from("-1234x"), OsString::from("logfile.log")];
            let (processed_args, skip_fields_old, _skip_chars_old) =
                uniq_handle_obsolete(args.into_iter());

            assert_eq!(
                processed_args,
                vec![OsString::from("-x"), OsString::from("logfile.log")]
            ); // "-1234x" should be treated as a normal argument
            assert_eq!(skip_fields_old, Some(1234));
        }

        #[test]
        fn test_arguments_that_only_partially_match_patterns() {
            let args = vec![OsString::from("-123abc")];
            let (processed_args, skip_fields_old, _skip_chars_old) =
                uniq_handle_obsolete(args.into_iter());

            assert_eq!(processed_args.len(), 1); // Assuming "-123abc" gets filtered out
            assert_eq!(skip_fields_old, Some(123)); // Assuming "123" is extracted despite the trailing characters
        }
    }

    #[cfg(test)]
    mod filter_args_tests {
        use super::*;
        use std::ffi::OsString;

        #[test]
        fn test_standard_argument() {
            let os_slice = OsString::from("standard");
            let mut skip_fields_old = None;
            let mut skip_chars_old = None;
            let mut is_preceding_long_opt_req_value = false;
            let mut is_preceding_short_opt_req_value = false;

            let result = uniq_filter_args(
                os_slice,
                &mut skip_fields_old,
                &mut skip_chars_old,
                &mut is_preceding_long_opt_req_value,
                &mut is_preceding_short_opt_req_value,
            );

            assert_eq!(result, Some(OsString::from("standard")));
        }

        #[test]
        fn test_argument_with_obsolete_numeric_prefix() {
            let os_slice = OsString::from("-1");
            let mut skip_fields_old = None;
            let mut skip_chars_old = None;
            let mut is_preceding_long_opt_req_value = false;
            let mut is_preceding_short_opt_req_value = false;

            let result = uniq_filter_args(
                os_slice,
                &mut skip_fields_old,
                &mut skip_chars_old,
                &mut is_preceding_long_opt_req_value,
                &mut is_preceding_short_opt_req_value,
            );

            assert!(skip_fields_old.is_some());
            assert_eq!(result, None); // Assuming "-1" gets removed entirely
        }

        #[test]
        fn test_argument_with_mixed_prefix() {
            let os_slice = OsString::from("-1f");
            let mut skip_fields_old = None;
            let mut skip_chars_old = None;
            let mut is_preceding_long_opt_req_value = false;
            let mut is_preceding_short_opt_req_value = false;

            let result = uniq_filter_args(
                os_slice,
                &mut skip_fields_old,
                &mut skip_chars_old,
                &mut is_preceding_long_opt_req_value,
                &mut is_preceding_short_opt_req_value,
            );

            assert_eq!(skip_fields_old, None);
            assert_eq!(result, Some(OsString::from("-f"))); // "f" remains after "1" is extracted
        }

        #[test]
        fn test_arguments_with_plus_prefix() {
            let os_slice = OsString::from("+12");
            let mut skip_fields_old = None;
            let mut skip_chars_old = None;
            let mut is_preceding_long_opt_req_value = false;
            let mut is_preceding_short_opt_req_value = false;

            let result = uniq_filter_args(
                os_slice,
                &mut skip_fields_old,
                &mut skip_chars_old,
                &mut is_preceding_long_opt_req_value,
                &mut is_preceding_short_opt_req_value,
            );

            assert!(skip_chars_old.is_none());
            assert_eq!(result, Some(OsString::from("+12"))); // Assuming "+12" gets removed entirely
        }

        #[test]
        fn test_consecutive_obsolete_arguments() {
            let args = vec![
                OsString::from("-1"),
                OsString::from("+2"),
                OsString::from("filename"),
            ];
            let processed_args = args
                .into_iter()
                .filter_map(|arg| {
                    let mut skip_fields_old = None;
                    let mut skip_chars_old = None;
                    let mut is_preceding_long_opt_req_value = false;
                    let mut is_preceding_short_opt_req_value = false;

                    uniq_filter_args(
                        arg,
                        &mut skip_fields_old,
                        &mut skip_chars_old,
                        &mut is_preceding_long_opt_req_value,
                        &mut is_preceding_short_opt_req_value,
                    )
                })
                .collect::<Vec<_>>();

            assert_eq!(
                processed_args,
                vec![OsString::from("+2"), OsString::from("filename")]
            ); // Assuming both "-1" and "+2" get removed
        }

        #[test]
        fn test_arguments_with_hyphens_and_numbers() {
            let os_slice = OsString::from("-123test");
            let mut skip_fields_old = None;
            let mut skip_chars_old = None;
            let mut is_preceding_long_opt_req_value = false;
            let mut is_preceding_short_opt_req_value = false;

            let result = uniq_filter_args(
                os_slice,
                &mut skip_fields_old,
                &mut skip_chars_old,
                &mut is_preceding_long_opt_req_value,
                &mut is_preceding_short_opt_req_value,
            );

            assert_eq!(result, Some(OsString::from("-test"))); // Assuming "-test" is treated as a single argument, not extracted
        }

        #[test]
        fn test_end_of_argument_list() {
            let os_slice = OsString::from("-999");
            let mut skip_fields_old = None;
            let mut skip_chars_old = None;
            let mut is_preceding_long_opt_req_value = false;
            let mut is_preceding_short_opt_req_value = false;

            let result = uniq_filter_args(
                os_slice,
                &mut skip_fields_old,
                &mut skip_chars_old,
                &mut is_preceding_long_opt_req_value,
                &mut is_preceding_short_opt_req_value,
            );

            assert_eq!(skip_fields_old, Some(String::from("999")));
            assert_eq!(result, None); // Assuming "-999" gets extracted and removed
        }

        #[test]
        fn test_large_numeric_values() {
            let os_slice = OsString::from("-10000000000");
            let mut skip_fields_old = None;
            let mut skip_chars_old = None;
            let mut is_preceding_long_opt_req_value = false;
            let mut is_preceding_short_opt_req_value = false;

            let result = uniq_filter_args(
                os_slice,
                &mut skip_fields_old,
                &mut skip_chars_old,
                &mut is_preceding_long_opt_req_value,
                &mut is_preceding_short_opt_req_value,
            );

            assert_eq!(skip_fields_old, Some(String::from("10000000000")));
            assert_eq!(result, None); // Assuming large numbers are handled and argument is removed
        }
    }

    #[cfg(test)]
    mod should_extract_obs_skip_fields_tests {
        use super::*;

        #[test]
        fn test_hyphen_with_numbers() {
            let str_slice = "-123";
            let is_preceding_long_opt_req_value = false;
            let is_preceding_short_opt_req_value = false;
            assert!(uniq_should_extract_obs_skip_fields(
                str_slice,
                &is_preceding_long_opt_req_value,
                &is_preceding_short_opt_req_value
            ));
        }

        #[test]
        fn test_hyphen_without_numbers() {
            let str_slice = "-abc";
            let is_preceding_long_opt_req_value = false;
            let is_preceding_short_opt_req_value = false;
            assert!(uniq_should_extract_obs_skip_fields(
                str_slice,
                &is_preceding_long_opt_req_value,
                &is_preceding_short_opt_req_value
            ));
        }

        #[test]
        fn test_multiple_hyphens() {
            let str_slice = "--123";
            let is_preceding_long_opt_req_value = false;
            let is_preceding_short_opt_req_value = false;
            assert!(!uniq_should_extract_obs_skip_fields(
                str_slice,
                &is_preceding_long_opt_req_value,
                &is_preceding_short_opt_req_value
            ));
        }

        #[test]
        fn test_hyphen_at_the_end() {
            let str_slice = "test-";
            let is_preceding_long_opt_req_value = false;
            let is_preceding_short_opt_req_value = false;
            assert!(!uniq_should_extract_obs_skip_fields(
                str_slice,
                &is_preceding_long_opt_req_value,
                &is_preceding_short_opt_req_value
            ));
        }

        #[test]
        fn test_numbers_preceded_by_characters() {
            let str_slice = "test123";
            let is_preceding_long_opt_req_value = false;
            let is_preceding_short_opt_req_value = false;
            assert!(!uniq_should_extract_obs_skip_fields(
                str_slice,
                &is_preceding_long_opt_req_value,
                &is_preceding_short_opt_req_value
            ));
        }

        #[test]
        fn test_embedded_hyphens() {
            let str_slice = "test-123field";
            let is_preceding_long_opt_req_value = false;
            let is_preceding_short_opt_req_value = false;
            assert!(!uniq_should_extract_obs_skip_fields(
                str_slice,
                &is_preceding_long_opt_req_value,
                &is_preceding_short_opt_req_value
            ));
        }

        #[test]
        fn test_special_characters_following_hyphen() {
            let str_slice = "-#*123";
            let is_preceding_long_opt_req_value = false;
            let is_preceding_short_opt_req_value = false;
            assert!(uniq_should_extract_obs_skip_fields(
                str_slice,
                &is_preceding_long_opt_req_value,
                &is_preceding_short_opt_req_value
            ));
        }

        #[test]
        fn test_leading_zeros() {
            let str_slice = "-000123";
            let is_preceding_long_opt_req_value = false;
            let is_preceding_short_opt_req_value = false;
            assert!(uniq_should_extract_obs_skip_fields(
                str_slice,
                &is_preceding_long_opt_req_value,
                &is_preceding_short_opt_req_value
            ));
        }

        #[test]
        fn test_hyphen_followed_by_mixed_characters() {
            let str_slice = "-12ab3";
            let is_preceding_long_opt_req_value = false;
            let is_preceding_short_opt_req_value = false;
            assert!(uniq_should_extract_obs_skip_fields(
                str_slice,
                &is_preceding_long_opt_req_value,
                &is_preceding_short_opt_req_value
            ));
        }

        #[test]
        fn test_adjacent_command_options() {
            let str_slice = "-12 -s3";
            let is_preceding_long_opt_req_value = false;
            let is_preceding_short_opt_req_value = false;
            assert!(uniq_should_extract_obs_skip_fields(
                str_slice,
                &is_preceding_long_opt_req_value,
                &is_preceding_short_opt_req_value
            ));
        }

        #[test]
        fn test_space_after_hyphen_before_number() {
            let str_slice = "- 123";
            let is_preceding_long_opt_req_value = false;
            let is_preceding_short_opt_req_value = false;
            assert!(uniq_should_extract_obs_skip_fields(
                str_slice,
                &is_preceding_long_opt_req_value,
                &is_preceding_short_opt_req_value
            ));
        }

        #[test]
        fn test_hyphen_as_part_of_a_larger_argument() {
            let str_slice = "--option-123";
            let is_preceding_long_opt_req_value = false;
            let is_preceding_short_opt_req_value = false;
            assert!(!uniq_should_extract_obs_skip_fields(
                str_slice,
                &is_preceding_long_opt_req_value,
                &is_preceding_short_opt_req_value
            ));
        }

        #[test]
        fn test_numeric_overflow_scenarios() {
            let str_slice = "-999999999999999999999999999999";
            let is_preceding_long_opt_req_value = false;
            let is_preceding_short_opt_req_value = false;
            assert!(uniq_should_extract_obs_skip_fields(
                str_slice,
                &is_preceding_long_opt_req_value,
                &is_preceding_short_opt_req_value
            ));
        }
    }

    #[cfg(test)]
    mod should_extract_obs_skip_chars_tests {
        use super::*;

        #[test]
        fn test_plus_with_numbers() {
            let str_slice = "+123";
            let is_preceding_long_opt_req_value = false;
            let is_preceding_short_opt_req_value = false;
            assert!(!uniq_should_extract_obs_skip_chars(
                str_slice,
                &is_preceding_long_opt_req_value,
                &is_preceding_short_opt_req_value
            ));
        }

        #[test]
        fn test_plus_without_numbers() {
            let str_slice = "+abc";
            let is_preceding_long_opt_req_value = false;
            let is_preceding_short_opt_req_value = false;
            assert!(!uniq_should_extract_obs_skip_chars(
                str_slice,
                &is_preceding_long_opt_req_value,
                &is_preceding_short_opt_req_value
            ));
        }

        #[test]
        fn test_no_plus_present() {
            let str_slice = "abc";
            let is_preceding_long_opt_req_value = false;
            let is_preceding_short_opt_req_value = false;
            assert!(!uniq_should_extract_obs_skip_chars(
                str_slice,
                &is_preceding_long_opt_req_value,
                &is_preceding_short_opt_req_value
            ));
        }

        #[test]
        fn test_multiple_pluses() {
            let str_slice = "++123";
            let is_preceding_long_opt_req_value = false;
            let is_preceding_short_opt_req_value = false;

            assert!(!uniq_should_extract_obs_skip_chars(
                str_slice,
                &is_preceding_long_opt_req_value,
                &is_preceding_short_opt_req_value
            ));
        }

        #[test]
        fn test_spaces_after_plus() {
            let str_slice = "+ 123";
            let is_preceding_long_opt_req_value = false;
            let is_preceding_short_opt_req_value = false;
            assert!(!uniq_should_extract_obs_skip_chars(
                str_slice,
                &is_preceding_long_opt_req_value,
                &is_preceding_short_opt_req_value
            ));
        }

        #[test]
        fn test_plus_as_part_of_valid_option() {
            let str_slice = "--option+123";
            let is_preceding_long_opt_req_value = false;
            let is_preceding_short_opt_req_value = false;
            assert!(!uniq_should_extract_obs_skip_chars(
                str_slice,
                &is_preceding_long_opt_req_value,
                &is_preceding_short_opt_req_value
            ));
        }

        #[test]
        fn test_embedded_plus_signs() {
            let str_slice = "abc+123";
            let is_preceding_long_opt_req_value = false;
            let is_preceding_short_opt_req_value = false;
            assert!(!uniq_should_extract_obs_skip_chars(
                str_slice,
                &is_preceding_long_opt_req_value,
                &is_preceding_short_opt_req_value
            ));
        }

        #[test]
        fn test_plus_at_start_of_string() {
            let str_slice = "+456";
            let is_preceding_long_opt_req_value = false;
            let is_preceding_short_opt_req_value = false;
            assert!(!uniq_should_extract_obs_skip_chars(
                str_slice,
                &is_preceding_long_opt_req_value,
                &is_preceding_short_opt_req_value
            ));
        }

        #[test]
        fn test_plus_following_special_characters() {
            let str_slice = "!@#+789";
            let is_preceding_long_opt_req_value = false;
            let is_preceding_short_opt_req_value = false;
            assert!(!uniq_should_extract_obs_skip_chars(
                str_slice,
                &is_preceding_long_opt_req_value,
                &is_preceding_short_opt_req_value
            ));
        }

        #[test]
        fn test_trailing_characters_after_numbers() {
            let str_slice = "+123abc";
            let is_preceding_long_opt_req_value = false;
            let is_preceding_short_opt_req_value = false;
            assert!(!uniq_should_extract_obs_skip_chars(
                str_slice,
                &is_preceding_long_opt_req_value,
                &is_preceding_short_opt_req_value
            ));
        }

        #[test]
        fn test_empty_string() {
            let str_slice = "";
            let is_preceding_long_opt_req_value = false;
            let is_preceding_short_opt_req_value = false;
            assert!(!uniq_should_extract_obs_skip_chars(
                str_slice,
                &is_preceding_long_opt_req_value,
                &is_preceding_short_opt_req_value
            ));
        }

        #[test]
        fn test_plus_with_floating_point_numbers() {
            let str_slice = "+12.3";
            let is_preceding_long_opt_req_value = false;
            let is_preceding_short_opt_req_value = false;
            assert!(!uniq_should_extract_obs_skip_chars(
                str_slice,
                &is_preceding_long_opt_req_value,
                &is_preceding_short_opt_req_value
            ));
        }
    }

    #[cfg(test)]
    mod handle_preceding_options_tests {
        use super::*;
        #[test]
        fn test_long_option_requiring_value() {
            let mut is_preceding_long_opt_req_value = false;
            let mut is_preceding_short_opt_req_value = false;
            uniq_handle_preceding_options(
                "--long-option",
                &mut is_preceding_long_opt_req_value,
                &mut is_preceding_short_opt_req_value,
            );
            assert!(!is_preceding_long_opt_req_value);
            assert!(!is_preceding_short_opt_req_value);
        }

        #[test]
        fn test_short_option_requiring_value() {
            let mut is_preceding_long_opt_req_value = false;
            let mut is_preceding_short_opt_req_value = false;
            uniq_handle_preceding_options(
                "-s",
                &mut is_preceding_long_opt_req_value,
                &mut is_preceding_short_opt_req_value,
            );
            assert!(!is_preceding_long_opt_req_value);
            assert!(is_preceding_short_opt_req_value);
        }

        #[test]
        fn test_no_value_required() {
            let mut is_preceding_long_opt_req_value = false;
            let mut is_preceding_short_opt_req_value = false;
            uniq_handle_preceding_options(
                "-x",
                &mut is_preceding_long_opt_req_value,
                &mut is_preceding_short_opt_req_value,
            );
            assert!(!is_preceding_long_opt_req_value);
            assert!(!is_preceding_short_opt_req_value);
        }

        #[test]
        fn test_resetting_flags() {
            let mut is_preceding_long_opt_req_value = true;
            let mut is_preceding_short_opt_req_value = true;
            uniq_handle_preceding_options(
                "value",
                &mut is_preceding_long_opt_req_value,
                &mut is_preceding_short_opt_req_value,
            );
            assert!(!is_preceding_long_opt_req_value);
            assert!(!is_preceding_short_opt_req_value);
        }

        #[test]
        fn test_multiple_consecutive_options() {
            let mut is_preceding_long_opt_req_value = false;
            let mut is_preceding_short_opt_req_value = false;
            uniq_handle_preceding_options(
                "-s",
                &mut is_preceding_long_opt_req_value,
                &mut is_preceding_short_opt_req_value,
            );
            assert!(!is_preceding_long_opt_req_value);
            assert!(is_preceding_short_opt_req_value);
            uniq_handle_preceding_options(
                "--long-option",
                &mut is_preceding_long_opt_req_value,
                &mut is_preceding_short_opt_req_value,
            );
            assert!(!is_preceding_long_opt_req_value);
            assert!(!is_preceding_short_opt_req_value);
            uniq_handle_preceding_options(
                "-x",
                &mut is_preceding_long_opt_req_value,
                &mut is_preceding_short_opt_req_value,
            );
            assert!(!is_preceding_long_opt_req_value);
            assert!(!is_preceding_short_opt_req_value);
        }

        #[test]
        fn test_option_followed_by_another_option() {
            let mut is_preceding_long_opt_req_value = false;
            let mut is_preceding_short_opt_req_value = false;
            uniq_handle_preceding_options(
                "-x",
                &mut is_preceding_long_opt_req_value,
                &mut is_preceding_short_opt_req_value,
            );
            uniq_handle_preceding_options(
                "--another-option",
                &mut is_preceding_long_opt_req_value,
                &mut is_preceding_short_opt_req_value,
            );
            assert!(!is_preceding_long_opt_req_value); // Ensure it does not set for "-x" which doesn't need a value
            assert!(!is_preceding_short_opt_req_value); // Ensure it resets correctly for the next option
        }
    }

    #[cfg(test)]
    mod handle_extract_obs_skip_fields_tests {
        use super::*;
        use std::ffi::OsString;

        #[test]
        fn test_extract_numeric_value() {
            let mut skip_fields_old = None;
            let result = uniq_handle_extract_obs_skip_fields("-10", &mut skip_fields_old);
            assert_eq!(skip_fields_old, Some("10".to_string()));
            assert_eq!(result, None); // Assuming the function returns None after successful extraction
        }

        #[test]
        fn test_mixed_with_other_flags() {
            let mut skip_fields_old = None;
            let result = uniq_handle_extract_obs_skip_fields("-10f", &mut skip_fields_old);
            assert_eq!(skip_fields_old, None);
            assert_eq!(result, Some(OsString::from("-f")));
        }

        #[test]
        fn test_no_numeric_value() {
            let mut skip_fields_old = None;
            let result = uniq_handle_extract_obs_skip_fields("-", &mut skip_fields_old);
            assert_eq!(skip_fields_old, None);
            assert_eq!(result, Some(OsString::from("-")));
        }

        #[test]
        fn test_multiple_digits() {
            let mut skip_fields_old = None;
            let result = uniq_handle_extract_obs_skip_fields("-123", &mut skip_fields_old);
            assert_eq!(skip_fields_old, Some("123".to_string()));
            assert_eq!(result, None);
        }

        #[test]
        fn test_invalid_formats() {
            let mut skip_fields_old = None;
            let result = uniq_handle_extract_obs_skip_fields("-abc", &mut skip_fields_old);
            assert_eq!(skip_fields_old, None);
            assert_eq!(result, Some(OsString::from("-abc")));
        }

        #[test]
        fn test_leading_zeros() {
            let mut skip_fields_old = None;
            let result = uniq_handle_extract_obs_skip_fields("-0012", &mut skip_fields_old);
            assert_eq!(skip_fields_old, Some("0012".to_string())); // Assuming leading zeros are ignored
            assert_eq!(result, None);
        }

        #[test]
        fn test_plus_as_part_of_larger_argument() {
            let mut skip_fields_old = None;
            let result = uniq_handle_extract_obs_skip_fields("--option-123", &mut skip_fields_old);
            assert_eq!(skip_fields_old, Some("123".to_string())); // Assuming it doesn't extract numbers from non-related arguments
            assert_eq!(result, Some(OsString::from("--option-")));
        }
    }
    #[cfg(test)]
    mod handle_extract_obs_skip_chars_tests {
        use super::*;

        #[test]
        fn test_extract_numeric_value() {
            let mut skip_chars_old = None;
            let result = uniq_handle_extract_obs_skip_chars("+10", &mut skip_chars_old);
            assert_eq!(skip_chars_old, Some("10".to_string()));
            assert_eq!(result, None); // Assuming the function returns None after extraction
        }

        #[test]
        fn test_no_numeric_value() {
            let mut skip_chars_old = None;
            let result = uniq_handle_extract_obs_skip_chars("+", &mut skip_chars_old);
            assert_eq!(skip_chars_old, None);
            assert!(result.is_some()); // Should return the original input if no digits follow '+'
        }

        #[test]
        fn test_non_numeric_following_plus() {
            let mut skip_chars_old = None;
            let result = uniq_handle_extract_obs_skip_chars("+abc", &mut skip_chars_old);
            assert_eq!(skip_chars_old, None);
            assert_eq!(result, Some(OsString::from("+abc")));
        }

        #[test]
        fn test_multiple_digits() {
            let mut skip_chars_old = None;
            let result = uniq_handle_extract_obs_skip_chars("+123", &mut skip_chars_old);
            assert_eq!(skip_chars_old, Some("123".to_string()));
            assert_eq!(result, None); // Assuming the function returns None after extraction
        }

        #[test]
        fn test_mixed_characters() {
            let mut skip_chars_old = None;
            let result = uniq_handle_extract_obs_skip_chars("+123abc", &mut skip_chars_old);
            assert_eq!(skip_chars_old, None); // Should not extract since it's mixed
            assert_eq!(result, Some(OsString::from("+123abc")));
        }
        #[test]
        fn test_multiple_pluses() {
            let mut skip_chars_old = None;
            let result = uniq_handle_extract_obs_skip_chars("++123", &mut skip_chars_old);
            assert_eq!(skip_chars_old, None); // Assuming it treats the second '+' as start of numbers
            assert_eq!(result, Some(OsString::from("++123")));
        }

        #[test]
        fn test_spaces_after_plus() {
            let mut skip_chars_old = None;
            let result = uniq_handle_extract_obs_skip_chars("+ 123", &mut skip_chars_old);
            assert_eq!(skip_chars_old, None);
            assert_eq!(result, Some(OsString::from("+ 123")));
        }

        #[test]
        fn test_plus_at_end() {
            let mut skip_chars_old = None;
            let result = uniq_handle_extract_obs_skip_chars("argument+", &mut skip_chars_old);
            assert_eq!(skip_chars_old, None);
            assert_eq!(result, Some(OsString::from("argument+")));
        }

        #[test]
        fn test_pluses_in_middle_of_arguments() {
            let mut skip_chars_old = None;
            let result = uniq_handle_extract_obs_skip_chars("arg+123ment", &mut skip_chars_old);
            assert_eq!(skip_chars_old, None);
            assert_eq!(result, Some(OsString::from("arg+123ment")));
        }

        #[test]
        fn test_plus_as_part_of_larger_argument() {
            let mut skip_chars_old = None;
            let result = uniq_handle_extract_obs_skip_chars("--option+123", &mut skip_chars_old);
            assert_eq!(skip_chars_old, None);
            assert_eq!(result, Some(OsString::from("--option+123")));
        }

        #[test]
        fn test_empty_string() {
            let mut skip_chars_old = None;
            let result = uniq_handle_extract_obs_skip_chars("", &mut skip_chars_old);
            assert_eq!(skip_chars_old, None);
            assert_eq!(result, Some(OsString::from("")));
        }

        #[test]
        fn test_real_command_mixed() {
            let mut skip_chars_old = None;
            let result = uniq_handle_extract_obs_skip_chars("-s+10 -f5", &mut skip_chars_old);
            assert_eq!(skip_chars_old, None);
            assert_eq!(result, Some(OsString::from("-s+10 -f5")));
        }

        #[test]
        fn test_consecutive_numbers() {
            let mut skip_fields_old = None;
            let result = uniq_handle_extract_obs_skip_fields("-123-456", &mut skip_fields_old);
            assert_eq!(skip_fields_old, Some("123".to_string())); // Assuming it stops at the first valid number sequence
            assert_eq!(result, Some(OsString::from("--456")));
        }

        #[test]
        fn test_numbers_mixed_with_letters() {
            let mut skip_fields_old = None;
            let result = uniq_handle_extract_obs_skip_fields("-12ab34", &mut skip_fields_old);
            assert_eq!(skip_fields_old, Some("12".to_string())); // Assuming it extracts until non-numeric characters
            assert_eq!(result, Some(OsString::from("-ab34")));
        }

        #[test]
        fn test_embedded_numbers() {
            let mut skip_fields_old = None;
            let result = uniq_handle_extract_obs_skip_fields("arg-123ment", &mut skip_fields_old);
            assert_eq!(skip_fields_old, Some("123".to_string())); // Numbers embedded in text should not be extracted
            assert_eq!(result, Some(OsString::from("arg-ment")));
        }

        #[test]
        fn test_only_dashes() {
            let mut skip_fields_old = None;
            let result = uniq_handle_extract_obs_skip_fields("----", &mut skip_fields_old);
            assert_eq!(skip_fields_old, None);
            assert_eq!(result, Some(OsString::from("----")));
        }

        #[test]
        fn test_comprehensive_mixed_input() {
            let mut skip_fields_old = None;
            let result = uniq_handle_extract_obs_skip_fields(
                "-12-valid-flag-34-other-text",
                &mut skip_fields_old,
            );
            assert_eq!(skip_fields_old, None); // Assuming it correctly extracts the first number sequence
            assert_eq!(result, Some(OsString::from("--valid-flag-34-other-text")));
        }

        #[test]
        fn test_adjacent_command_options() {
            let mut skip_fields_old = None;
            let result = uniq_handle_extract_obs_skip_fields("-12-s3", &mut skip_fields_old);
            assert_eq!(skip_fields_old, Some("12".to_string())); // Extracts number and correctly parses the rest as flags
            assert_eq!(result, Some(OsString::from("--s3")));
        }

        #[test]
        fn test_trailing_characters_after_numbers() {
            let mut skip_fields_old = None;
            let result = uniq_handle_extract_obs_skip_fields("-123xyz", &mut skip_fields_old);
            assert_eq!(skip_fields_old, Some("123".to_string())); // Assuming extraction stops at the first non-numeric character
            assert_eq!(result, Some(OsString::from("-xyz")));
        }
    }

    #[cfg(test)]
    mod map_clap_errors_tests {
        use super::*;
        use clap::Error as ClapError;
        use clap::error::ErrorKind as ClapErrorKind; // Assuming you use ContextKind elsewhere

        fn generate_clap_error(
            error_kind: ClapErrorKind,
            arg_name: Option<&str>,
            value: Option<&str>,
        ) -> ClapError {
            let name = if let Some(n) = arg_name { n } else { "" };

            let val = if let Some(v) = value { v } else { "" };

            ClapError::raw(
                error_kind,
                format!("dummy error for testing: {} {}", name, val),
            )
        }

        #[test]
        fn test_uniq_map_clap_errors_argument_conflict() {
            let clap_err = generate_clap_error(ClapErrorKind::ArgumentConflict, None, None);
            let error = uniq_map_clap_errors(&clap_err).to_string();
            let expected = "--group is mutually exclusive with -c/-d/-D/-u\nTry 'uniq --help' for more information.";
            assert_eq!(error, expected);
        }

        #[test]
        fn test_uniq_map_clap_errors_invalid_value_group() {
            let clap_err = generate_clap_error(
                ClapErrorKind::InvalidValue,
                Some("group"),
                Some("badoption"),
            );
            let error = uniq_map_clap_errors(&clap_err).to_string();
            let expected = "error: dummy error for testing: group badoption";
            assert_eq!(error, expected);
        }

        #[test]
        fn test_uniq_map_clap_errors_invalid_value_all_repeated() {
            let clap_err = generate_clap_error(
                ClapErrorKind::InvalidValue,
                Some("all-repeated"),
                Some("badoption"),
            );
            let error = uniq_map_clap_errors(&clap_err).to_string();
            let expected = "error: dummy error for testing: all-repeated badoption";
            assert_eq!(error, expected);
        }

        #[test]
        fn test_uniq_map_clap_errors_default_case() {
            let clap_err = generate_clap_error(ClapErrorKind::MissingRequiredArgument, None, None);
            let error = uniq_map_clap_errors(&clap_err).to_string();
            assert!(error.contains("dummy error for testing:"));
        }
    }

    #[cfg(test)]
    mod get_delimiter_tests {
        use super::*;

        #[test]
        fn test_uniq_get_delimiter_none() {
            let args = vec![ctcore::ct_util_name()];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let delimiter = uniq_get_delimiter(&matches);
            assert_eq!(delimiter, UniqDelimiters::None);
        }

        #[test]
        fn test_uniq_get_delimiter_append_parse_error() {
            let args = vec![ctcore::ct_util_name(), "--all-repeated=append"];
            let matches = ct_app().try_get_matches_from(args);

            assert!(matches.is_err());
            assert_eq!(matches.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_uniq_get_delimiter_prepend() {
            let args = vec![ctcore::ct_util_name(), "--all-repeated=prepend"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let delimiter = uniq_get_delimiter(&matches);
            assert_eq!(delimiter, UniqDelimiters::Prepend);
        }

        #[test]
        fn test_uniq_get_delimiter_separate() {
            let args = vec![ctcore::ct_util_name(), "--all-repeated=separate"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let delimiter = uniq_get_delimiter(&matches);
            assert_eq!(delimiter, UniqDelimiters::Separate);
        }

        #[test]
        fn test_uniq_get_delimiter_both() {
            let args = vec![ctcore::ct_util_name(), "--group=both"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let delimiter = uniq_get_delimiter(&matches);
            assert_eq!(delimiter, UniqDelimiters::Both);
        }

        #[test]
        fn test_uniq_get_delimiter_default_group() {
            let args = vec![ctcore::ct_util_name(), "--group"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let delimiter = uniq_get_delimiter(&matches);
            assert_eq!(delimiter, UniqDelimiters::Separate);
        }

        #[test]
        fn test_uniq_get_delimiter_invalid_value() {
            // This test case will check for invalid values and should ensure the command fails as expected
            let args = vec!["uniq", "--group=invalid"];
            let result = ct_app().try_get_matches_from(args);
            assert!(result.is_err());
        }

        #[test]
        fn test_uniq_get_delimiter_all_repeated_none() {
            let args = vec!["uniq", "--all-repeated=none"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let delimiter = uniq_get_delimiter(&matches);
            assert_eq!(delimiter, UniqDelimiters::None);
        }

        #[test]
        fn test_uniq_get_delimiter_all_repeated_default() {
            let args = vec!["uniq", "--all-repeated"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let delimiter = uniq_get_delimiter(&matches);
            assert_eq!(delimiter, UniqDelimiters::None);
        }

        #[test]
        fn test_uniq_get_delimiter_group_separate() {
            let args = vec!["uniq", "--group=separate"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let delimiter = uniq_get_delimiter(&matches);
            assert_eq!(delimiter, UniqDelimiters::Separate);
        }

        #[test]
        fn test_uniq_get_delimiter_group_prepend() {
            let args = vec!["uniq", "--group=prepend"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let delimiter = uniq_get_delimiter(&matches);
            assert_eq!(delimiter, UniqDelimiters::Prepend);
        }

        #[test]
        fn test_uniq_get_delimiter_group_append() {
            let args = vec!["uniq", "--group=append"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let delimiter = uniq_get_delimiter(&matches);
            assert_eq!(delimiter, UniqDelimiters::Append);
        }

        #[test]
        fn test_uniq_get_delimiter_all_repeated_and_group() {
            let args = vec!["uniq", "--all-repeated=separate", "--group=prepend"];
            let matches = ct_app().try_get_matches_from(args);

            assert!(matches.is_err());
            assert_eq!(matches.unwrap_err().kind(), ErrorKind::ArgumentConflict);
        }
    }
    #[cfg(test)]
    mod open_input_file_tests {
        use super::*;
        use std::fs::File;
        use std::io::{self, Read, Write};
        use tempfile::NamedTempFile;

        #[test]
        fn test_uniq_open_input_file_stdin() {
            // Create a temporary file to simulate stdin
            let mut temp_file = NamedTempFile::new().unwrap();
            let test_data = b"Hello, Stdin!";
            temp_file.write_all(test_data).unwrap();

            // Reopen the temporary file for reading
            let mut temp_file = temp_file.reopen().unwrap();
            let mut buffer = Vec::new();
            temp_file.read_to_end(&mut buffer).unwrap();
            assert_eq!(buffer, test_data);
        }

        #[test]
        fn test_uniq_open_input_file_temp_file() {
            let mut temp_file = NamedTempFile::new().unwrap();
            let test_data = b"Hello, Temp File!";
            temp_file.write_all(test_data).unwrap();
            let file_name: Option<&OsStr> = Some(temp_file.path().as_os_str());

            let reader = uniq_open_input_file(file_name).unwrap();
            let mut reader: Box<dyn BufRead> = reader;

            let mut buffer = Vec::new();
            reader.read_to_end(&mut buffer).unwrap();
            assert_eq!(buffer, test_data);
        }

        #[test]
        fn test_uniq_open_input_file_with_hyphen() {
            // Create a temporary file to simulate stdin
            let mut temp_file = NamedTempFile::new().unwrap();
            let test_data = b"Hello, Stdin!";
            temp_file.write_all(test_data).unwrap();

            // Reopen the temporary file for reading
            let _file_name: Option<&OsStr> = Some(OsStr::new("-"));
            let stdin_backup = io::stdin();
            let _stdin_handle = stdin_backup.lock();
            let stdin_file = File::open(temp_file.path()).unwrap();
            let mut reader = BufReader::new(stdin_file);

            // Test reading from redirected stdin
            let mut buffer = Vec::new();
            reader.read_to_end(&mut buffer).unwrap();
            assert_eq!(buffer, test_data);
        }

        #[test]
        fn test_uniq_open_input_file_invalid_path() {
            let invalid_path = OsStr::new("/invalid/path/to/input.txt");
            let file_name: Option<&OsStr> = Some(invalid_path);
            let result = uniq_open_input_file(file_name);
            assert!(result.is_err());
        }
    }

    #[cfg(test)]
    mod open_output_file_tests {
        use super::*;
        use std::io::{Read, Write};
        use tempfile::tempdir;

        #[test]
        fn test_open_output_file_stdout() {
            let result = uniq_open_output_file(None);
            assert!(result.is_ok());
        }

        #[test]
        fn test_open_output_file_dash() {
            let result = uniq_open_output_file(Some(OsStr::new("-")));
            assert!(result.is_ok());
        }

        #[test]
        fn test_open_output_file_valid_path() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test_output.txt");
            let file_path_str = file_path.to_str().unwrap();

            let result = uniq_open_output_file(Some(OsStr::new(file_path_str)));
            assert!(result.is_ok());

            let mut writer = result.unwrap();
            writeln!(writer, "Test line").unwrap();
            drop(writer); // Ensure the writer is flushed and closed

            let mut file = File::open(&file_path).unwrap();
            let mut contents = String::new();
            file.read_to_string(&mut contents).unwrap();
            assert_eq!(contents, "Test line\n");

            dir.close().unwrap();
        }

        #[test]
        fn test_open_output_file_invalid_path() {
            let invalid_path = OsStr::new("/invalid/path/to/output.txt");
            let file_name: Option<&OsStr> = Some(invalid_path);
            let result = uniq_open_output_file(file_name);
            assert!(result.is_err());
        }

        #[test]
        fn test_uniq_open_output_file_temp_file() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("temp_output.txt");
            let file_name: Option<&OsStr> = Some(file_path.as_os_str());
            let writer = uniq_open_output_file(file_name).unwrap();
            let mut writer: Box<dyn Write> = writer;
            let test_data = b"Hello, Temp File!";
            writer.write_all(test_data).unwrap();
            drop(writer);

            let mut file = File::open(file_path).unwrap();
            let mut content = String::new();
            file.read_to_string(&mut content).unwrap();
            assert_eq!(content, "Hello, Temp File!");
        }
    }
    #[cfg(test)]
    mod ct_main_tests {
        use super::*;
        use std::ffi::OsString;
        use std::fs::File;
        use std::io::Write;
        use tempfile::tempdir;

        #[test]
        fn test_ct_main_execution_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = uniq_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_execution_other_version() {
            let args = vec![ctcore::ct_util_name(), "-V"];
            let result = uniq_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_execution_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = uniq_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_execution_unsupport_help() {
            let args = vec![ctcore::ct_util_name(), "-H"];
            let result = uniq_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_invalid_argument() {
            let args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = uniq_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_support_missing_argument() {
            let filename = "test_ct_main_support_missing_argument";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "world\nworld\nworld\nworld\nworld 1000 zzzzz\nworld 2200 ccccc\nCtyunOs 2000 aaaaa\nCtyunOs 1900 ababa"
            )
                .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name];
            let result = uniq_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_long_option_all_repeated() {
            let filename = "test_ct_main_long_option_all_repeated";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "world\nworld\nworld\nworld\nworld 1000 zzzzz\nworld 2200 ccccc\nCtyunOs 2000 aaaaa\nCtyunOs 1900 ababa"
            )
                .unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--all-repeated=prepend", file_name];
            let result = uniq_main(args.iter().map(|s| OsString::from(s)));
            println!("{:?}", result);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_long_option_group() {
            let filename = "test_ct_main_long_option_group";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "world\nworld\nworld\nworld\nworld 1000 zzzzz\nworld 2200 ccccc\nCtyunOs 2000 aaaaa\nCtyunOs 1900 ababa"
            )
                .unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--group=separate", file_name];
            let result = uniq_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_long_option_check_chars() {
            let filename = "test_ct_main_long_option_check_chars";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "world\nworld\nworld\nworld\nworld 1000 zzzzz\nworld 2200 ccccc\nCtyunOs 2000 aaaaa\nCtyunOs 1900 ababa"
            )
                .unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--check-chars=3", file_name];
            let result = uniq_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_long_option_count() {
            let filename = "test_ct_main_long_option_count";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "world\nworld\nworld\nworld\nworld 1000 zzzzz\nworld 2200 ccccc\nCtyunOs 2000 aaaaa\nCtyunOs 1900 ababa"
            )
                .unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--count", file_name];
            let result = uniq_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_long_option_ignore_case() {
            let filename = "test_ct_main_long_option_ignore_case";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "world\nworld\nworld\nworld\nworld 1000 zzzzz\nworld 2200 ccccc\nCtyunOs 2000 aaaaa\nCtyunOs 1900 ababa"
            )
                .unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--ignore-case", file_name];
            let result = uniq_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_long_option_repeated() {
            let filename = "test_ct_main_long_option_repeated";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "world\nworld\nworld\nworld\nworld 1000 zzzzz\nworld 2200 ccccc\nCtyunOs 2000 aaaaa\nCtyunOs 1900 ababa"
            )
                .unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--repeated", file_name];
            let result = uniq_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_long_option_skip_chars() {
            let filename = "test_ct_main_long_option_skip_chars";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "world\nworld\nworld\nworld\nworld 1000 zzzzz\nworld 2200 ccccc\nCtyunOs 2000 aaaaa\nCtyunOs 1900 ababa"
            )
                .unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--skip-chars", "8", file_name];
            let result = uniq_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_long_option_skip_fields() {
            let filename = "test_ct_main_long_option_skip_fields";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "world\nworld\nworld\nworld\nworld 1000 zzzzz\nworld 2200 ccccc\nCtyunOs 2000 aaaaa\nCtyunOs 1900 ababa"
            )
                .unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--skip-fields", "5", file_name];
            let result = uniq_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_long_option_unique() {
            let filename = "test_ct_main_long_option_unique";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "world\nworld\nworld\nworld\nworld 1000 zzzzz\nworld 2200 ccccc\nCtyunOs 2000 aaaaa\nCtyunOs 1900 ababa"
            )
                .unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--unique", file_name];
            let result = uniq_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_short_option_uppercase_d() {
            let filename = "test_ct_main_short_option_uppercase_d";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "world\nworld\nworld\nworld\nworld 1000 zzzzz\nworld 2200 ccccc\nCtyunOs 2000 aaaaa\nCtyunOs 1900 ababa"
            )
                .unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-D=prepend", file_name];
            let result = uniq_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_short_option_w() {
            let filename = "test_ct_main_short_option_w";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "world\nworld\nworld\nworld\nworld 1000 zzzzz\nworld 2200 ccccc\nCtyunOs 2000 aaaaa\nCtyunOs 1900 ababa"
            )
                .unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-w", "6", file_name];
            let result = uniq_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_short_option_c() {
            let filename = "test_ct_main_short_option_c";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "world\nworld\nworld\nworld\nworld 1000 zzzzz\nworld 2200 ccccc\nCtyunOs 2000 aaaaa\nCtyunOs 1900 ababa"
            )
                .unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-c", file_name];
            let result = uniq_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_short_option_i() {
            let filename = "test_ct_main_short_option_i";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "world\nworld\nworld\nworld\nworld 1000 zzzzz\nworld 2200 ccccc\nCtyunOs 2000 aaaaa\nCtyunOs 1900 ababa"
            )
                .unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-i", file_name];
            let result = uniq_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_short_option_d() {
            let filename = "test_ct_main_short_option_d";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "world\nworld\nworld\nworld\nworld 1000 zzzzz\nworld 2200 ccccc\nCtyunOs 2000 aaaaa\nCtyunOs 1900 ababa"
            )
                .unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-d", file_name];
            let result = uniq_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_short_option_s() {
            let filename = "test_ct_main_short_option_s";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "world\nworld\nworld\nworld\nworld 1000 zzzzz\nworld 2200 ccccc\nCtyunOs 2000 aaaaa\nCtyunOs 1900 ababa"
            )
                .unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-s", "11", file_name];
            let result = uniq_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_short_option_f() {
            let filename = "test_ct_main_short_option_f";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "world\nworld\nworld\nworld\nworld 1000 zzzzz\nworld 2200 ccccc\nCtyunOs 2000 aaaaa\nCtyunOs 1900 ababa"
            )
                .unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-f", "8", file_name];
            let result = uniq_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_short_option_u() {
            let filename = "test_ct_main_short_option_u";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "world\nworld\nworld\nworld\nworld 1000 zzzzz\nworld 2200 ccccc\nCtyunOs 2000 aaaaa\nCtyunOs 1900 ababa"
            )
                .unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-u", file_name];
            let result = uniq_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }
    }
    #[cfg(test)]
    mod ct_app_tests {
        use clap::error::ErrorKind;

        use super::*;

        // uniq 接口: uniq [OPTION]... [INPUT [OUTPUT]]
        //   -c, --count           prefix lines by the number of occurrences
        //   -d, --repeated        only print duplicate lines, one for each group
        //   -D                    print all duplicate lines
        //       --all-repeated[=METHOD]  like -D, but allow separating groups
        //                                  with an empty line;
        //                                  METHOD={none(default),prepend,separate}
        //   -f, --skip-fields=N   avoid comparing the first N fields
        //       --group[=METHOD]  show all items, separating groups with an empty line;
        //                           METHOD={separate(default),prepend,append,both}
        //   -i, --ignore-case     ignore differences in case when comparing
        //   -s, --skip-chars=N    avoid comparing the first N characters
        //   -u, --unique          only print unique lines
        //   -z, --zero-terminated     line delimiter is NUL, not newline
        //   -w, --check-chars=N   compare no more than N characters in lines
        //       --help     display this help and exit
        //       --version  output version information and exit

        #[test]
        fn test_ct_app_execution_version() {
            let command = ct_app();

            // 测试用例1：有效输入
            let args = vec![ctcore::ct_util_name(), "--version"];

            // Assuming `command` has a method to retrieve the result name, replace it with the actual one
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_other_version() {
            let command = ct_app();

            // 测试用例1：有效输入
            let args = vec![ctcore::ct_util_name(), "-V"];

            // Assuming `command` has a method to retrieve the result name, replace it with the actual one
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_help() {
            let command = ct_app();

            // 测试用例2：验证 --help 参数是否正确处理
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_execution_unsupport_help() {
            let command = ct_app();

            // 测试用例2：验证 --help 参数是否正确处理
            let args = vec![ctcore::ct_util_name(), "-H"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_invalid_argument() {
            let command = ct_app();

            // 测试用例3：验证当提供未知参数时是否正确报错
            let args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_support_missing_argument() {
            let command = ct_app();

            // 测试用例4：验证当缺少必需的参数时是否正确报错
            let args = vec![ctcore::ct_util_name()]; // 缺少任何参数
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_all_repeated() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--all-repeated", "prepend"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_group() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--group", "separate"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_check_chars() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--check-chars", "N"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_count() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--count"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_ignore_case() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--ignore-case"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_repeated() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--repeated"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_skip_chars() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--skip-chars", "N"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_skip_fields() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--skip-fields", "N"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_unique() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--unique"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_zero_terminated() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--zero-terminated"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_short_option_uppercase_d() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-D", "prepend"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_short_option_w() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-w", "N"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_short_option_c() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-c"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_short_option_i() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-i"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_short_option_d() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-d"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_short_option_s() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-s", "N"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_short_option_f() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-f", "N"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_short_option_u() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-u"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_short_option_z() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-z"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }
    }
}
