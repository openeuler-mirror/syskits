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
use std::io::{stdin, stdout, BufRead, BufReader, BufWriter, Write};
use std::num::IntErrorKind;

use clap::builder::ValueParser;
use clap::{crate_version, error::ContextKind, error::Error, error::ErrorKind};
use clap::{Arg, ArgAction, ArgMatches, Command};
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTError, CTResult, CtSimpleError, FromIo};
use ctcore::ct_posix::{ct_posix_version, OBSOLETE};
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
        && str_slice
            .chars()
            .nth(1)
            .map_or(false, |c| c.is_ascii_digit())
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

}
