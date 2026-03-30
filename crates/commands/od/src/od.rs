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

//! od - 八进制、十进制、十六进制、ASCII 转储
//!
//! # 功能描述
//!
//! od 命令用于以不同格式显示文件内容。它可以以八进制、十进制、十六进制或 ASCII 格式显示文件，
//! 支持多种数据类型（如字节、字、双字等）和不同的字节序。
//!
//! # 主要特性
//!
//! * 支持多种输出格式：八进制、十进制、十六进制、ASCII
//! * 支持不同的数据类型：1、2、4、8 字节整数和浮点数
//! * 支持大端和小端字节序
//! * 支持跳过指定字节数
//! * 支持限制读取字节数
//! * 支持重复行压缩
//! * 支持多文件输入
//!
//! # 输出示例
//! ```text
//! 0000000    6548    6c6c    2c6f    7720    726f    646c    0a21
//! 0000016
//! ```
//!
//! # 实现说明
//!
//! 代码主要分为以下几个部分：
//! * 命令行参数解析
//! * 输入处理（文件读取、字节序转换）
//! * 格式化输出
//! * 重复行处理

// spell-checker:ignore (clap) dont
// spell-checker:ignore (ToDO) formatteriteminfo inputdecoder inputoffset mockstream nrofbytes partialreader odfunc multifile exitcode

extern crate rust_i18n;
mod byteorder_io;
mod formatteriteminfo;
mod inputdecoder;
mod inputoffset;
#[cfg(test)]
mod mockstream;
mod multifilereader;
mod output_info;
mod parse_formats;
mod parse_inputs;
mod parse_nrofbytes;
mod partialreader;
mod peekreader;
mod prn_format;

use rust_i18n::t;
use std::cmp;
rust_i18n::i18n!("locales", fallback = "en-US");
use std::fmt::Write;
use std::io::Read; // <--- 新增引用

use crate::byteorder_io::ByteOrder;
use crate::formatteriteminfo::OdFormatWriter;
use crate::inputdecoder::{OdInputDecoder, OdMemoryDecoder};
use crate::inputoffset::{OdInputOffset, OdRadix};
use crate::multifilereader::{HasError, OdInputSource, OdMultifileReader};
use crate::output_info::{OutputInfo, SpacedFormatterItemInfo};
use crate::parse_formats::{od_parse_format_flags, ParsedFormatterItemInfo};
use crate::parse_inputs::{od_parse_inputs, CommandLineInputs};
use crate::parse_nrofbytes::od_parse_number_of_bytes;
use crate::partialreader::PartialReader;
use crate::peekreader::{PeekRead, PeekReader};
use crate::prn_format::format_ascii_dump;
use clap::ArgAction;
use clap::{crate_version, parser::ValueSource, Arg, ArgMatches, Command};
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CtSimpleError};
use ctcore::ct_parse_size::ParseSizeError;
use ctcore::Tool;
use ctcore::{ct_show_error, ct_show_warning};
use std::ffi::OsString;
use sys_locale::get_locale;

const OD_PEEK_BUFFER_SIZE: usize = 4; // utf-8 can be 4 bytes

pub(crate) mod od_options {
    pub const OD_HELP: &str = "help";
    pub const OD_ADDRESS_RADIX: &str = "address-radix";
    pub const OD_SKIP_BYTES: &str = "skip-bytes";
    pub const OD_READ_BYTES: &str = "read-bytes";
    pub const OD_ENDIAN: &str = "endian";
    pub const OD_STRINGS: &str = "strings";
    pub const OD_FORMAT: &str = "format";
    pub const OD_OUTPUT_DUPLICATES: &str = "output-duplicates";
    pub const OD_TRADITIONAL: &str = "traditional";
    pub const OD_WIDTH: &str = "width";
    pub const OD_FILENAME: &str = "FILENAME";
}

/// OD命令的设置参数结构体
struct OdSettings {
    byte_order: ByteOrder,                 // 字节序：大端、小端或本机字节序
    skip_bytes: u64,                       // 需要跳过的字节数
    read_bytes: Option<u64>,               // 需要读取的字节数，None表示读取全部
    label: Option<u64>,                    // 地址标签的起始值
    input_strings: Vec<String>,            // 输入文件列表
    formats: Vec<ParsedFormatterItemInfo>, // 输出格式列表
    line_bytes: usize,                     // 每行显示的字节数
    output_duplicates: bool,               // 是否输出重复行
    radix: OdRadix,                        // 地址的显示进制
    strings_min_len: Option<usize>,        // <--- 新增: -S 字符串最小长度
}

impl OdSettings {
    fn new(matches: &ArgMatches, args: &[String]) -> CTResult<Self> {
        let byte_order = if let Some(s) = matches.get_one::<String>(od_options::OD_ENDIAN) {
            match s.as_str() {
                "little" => ByteOrder::Little,
                "big" => ByteOrder::Big,
                _ => {
                    return Err(CtSimpleError::new(
                        1,
                        format!("Invalid argument --endian={s}"),
                    ));
                }
            }
        } else {
            ByteOrder::Native
        };

        let mut skip_bytes = match matches.get_one::<String>(od_options::OD_SKIP_BYTES) {
            None => 0,
            Some(s) => od_parse_number_of_bytes(s).map_err(|e| {
                CtSimpleError::new(1, od_format_error_message(&e, s, od_options::OD_SKIP_BYTES))
            })?,
        };

        let mut label: Option<u64> = None;

        let parsed_input = od_parse_inputs(matches)
            .map_err(|e| CtSimpleError::new(1, format!("Invalid inputs: {e}")))?;
        let input_strings = match parsed_input {
            CommandLineInputs::FileNames(v) => v,
            CommandLineInputs::FileAndOffset((f, s, l)) => {
                skip_bytes = s;
                label = l;
                vec![f]
            }
        };

        let formats = od_parse_format_flags(args).map_err(|e| CtSimpleError::new(1, e))?;

        let mut line_bytes = Self::parse_line_width(matches)?;
        let min_bytes = formats.iter().fold(1, |max, next| {
            cmp::max(max, next.formatter_item_info.byte_size)
        });

        if line_bytes == 0 || line_bytes % min_bytes != 0 {
            ct_show_warning!("invalid width {}; using {} instead", line_bytes, min_bytes);
            line_bytes = min_bytes;
        }

        let output_duplicates = matches.get_flag(od_options::OD_OUTPUT_DUPLICATES);
        let read_bytes = Self::parse_read_bytes(matches)?;
        let radix = Self::parse_radix(matches)?;

        // --- 解析 -S (--strings) 参数 ---
        let strings_min_len = matches
            .get_one::<String>(od_options::OD_STRINGS)
            .map(|s| {
                s.parse::<usize>().map_err(|_| {
                    CtSimpleError::new(1, format!("invalid -S argument {}", s.quote()))
                })
            })
            .transpose()?;

        Ok(Self {
            byte_order,
            skip_bytes,
            read_bytes,
            label,
            input_strings,
            formats,
            line_bytes,
            output_duplicates,
            radix,
            strings_min_len,
        })
    }

    fn parse_line_width(matches: &ArgMatches) -> CTResult<usize> {
        match matches.get_one::<String>(od_options::OD_WIDTH) {
            None => Ok(16),
            Some(s) => {
                if matches.value_source(od_options::OD_WIDTH) == Some(ValueSource::CommandLine) {
                    od_parse_number_of_bytes(s)
                        .map_err(|e| {
                            CtSimpleError::new(
                                1,
                                od_format_error_message(&e, s, od_options::OD_WIDTH),
                            )
                        })
                        .and_then(|n| {
                            usize::try_from(n)
                                .map_err(|_| CtSimpleError::new(1, format!("'{s}' is too large")))
                        })
                } else {
                    Ok(16)
                }
            }
        }
    }

    fn parse_read_bytes(matches: &ArgMatches) -> CTResult<Option<u64>> {
        match matches.get_one::<String>(od_options::OD_READ_BYTES) {
            None => Ok(None),
            Some(s) => od_parse_number_of_bytes(s).map(Some).map_err(|e| {
                CtSimpleError::new(1, od_format_error_message(&e, s, od_options::OD_READ_BYTES))
            }),
        }
    }

    fn parse_radix(matches: &ArgMatches) -> CTResult<OdRadix> {
        match matches.get_one::<String>(od_options::OD_ADDRESS_RADIX) {
            None => Ok(OdRadix::Octal),
            Some(s) => match s.as_bytes().first().copied() {
                Some(b'd') => Ok(OdRadix::Decimal),
                Some(b'x') => Ok(OdRadix::Hexadecimal),
                Some(b'o') => Ok(OdRadix::Octal),
                Some(b'n') => Ok(OdRadix::NoPrefix),
                _ => Err(CtSimpleError::new(
                    1,
                    format!(
                        "invalid output address radix '{s}'; it must be one character from [doxn]"
                    ),
                )),
            },
        }
    }
}

pub fn od_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let args = args.collect_ignore();

    let clap_matches = ct_app().try_get_matches_from(&args)?;

    let od_settings = OdSettings::new(&clap_matches, &args)?;

    let mut input_offset =
        OdInputOffset::new(od_settings.radix, od_settings.skip_bytes, od_settings.label);

    let mut input = od_open_input_peek_reader(
        &od_settings.input_strings,
        od_settings.skip_bytes,
        od_settings.read_bytes,
    );

    // 如果指定了 -S，进入专用的严格字符串扫描模式
    if let Some(min_len) = od_settings.strings_min_len {
        return odexec_strings(
            &mut input,
            od_settings.radix,
            od_settings.skip_bytes,
            od_settings.read_bytes, // 把 -N 的限制传进去用于精确判断
            min_len,
        );
    }

    let mut input_decoder = OdInputDecoder::new(
        &mut input,
        od_settings.line_bytes,
        OD_PEEK_BUFFER_SIZE,
        od_settings.byte_order,
    );

    let output_info = OutputInfo::new(
        od_settings.line_bytes,
        &od_settings.formats[..],
        od_settings.output_duplicates,
    );

    odexec(&mut input_offset, &mut input_decoder, &output_info)
}

// --- 专门用于处理 -S (--strings) 的执行循环 ---
fn odexec_strings<R: Read>(
    input: &mut R,
    radix: OdRadix,
    skip_bytes: u64,
    read_bytes: Option<u64>,
    min_len: usize,
) -> CTResult<()> {
    let mut buffer = Vec::new();
    let mut current_offset = skip_bytes;
    let mut string_start = current_offset;
    let mut chunk = [0u8; 8192];
    let mut bytes_read_total = 0u64;

    loop {
        let n = match input.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                ct_show_error!("read error: {}", e);
                return Err(1.into());
            }
        };

        for &b in &chunk[..n] {
            bytes_read_total += 1;
            // 筛选可打印的 ASCII 图形字符 (0x20 空格 到 0x7E 波浪号)
            if (0x20..=0x7E).contains(&b) {
                if buffer.is_empty() {
                    string_start = current_offset;
                }
                buffer.push(b);
            } else {
                // GNU 规范：字符串必须以 NUL (\0) 字节结尾才算合法！
                if buffer.len() >= min_len && b == 0 {
                    print_string_record(string_start, radix, &buffer)?;
                }
                buffer.clear();
            }
            current_offset += 1;
        }
    }

    // 文件流结尾时的特殊处理
    if buffer.len() >= min_len {
        if let Some(limit) = read_bytes {
            if bytes_read_total == limit {
                print_string_record(string_start, radix, &buffer)?;
            }
        }
    }

    Ok(())
}

fn print_string_record(offset: u64, radix: OdRadix, buffer: &[u8]) -> CTResult<()> {
    use std::io::Write as _;
    let mut stdout = std::io::stdout().lock();

    let address = match radix {
        OdRadix::Octal => format!("{:07o}", offset),
        OdRadix::Decimal => format!("{:07}", offset),
        OdRadix::Hexadecimal => format!("{:06x}", offset),
        OdRadix::NoPrefix => String::new(),
    };

    // 因为我们已经提前过滤了 0x20..=0x7E，它必定是合法的 UTF-8
    let s = unsafe { std::str::from_utf8_unchecked(buffer) };

    if address.is_empty() {
        writeln!(stdout, "{}", s).map_err(|e| CtSimpleError::new(1, e.to_string()))?;
    } else {
        writeln!(stdout, "{} {}", address, s).map_err(|e| CtSimpleError::new(1, e.to_string()))?;
    }
    Ok(())
}

pub fn ct_app() -> Command {
    let args = vec![
        Arg::new(od_options::OD_HELP)
            .long(od_options::OD_HELP)
            .help(t!("od.clap.od_help"))
            .action(ArgAction::Help),
        Arg::new(od_options::OD_ADDRESS_RADIX)
            .short('A')
            .long(od_options::OD_ADDRESS_RADIX)
            .help(t!("od.clap.od_address_radix"))
            .value_name("RADIX"),
        Arg::new(od_options::OD_SKIP_BYTES)
            .short('j')
            .long(od_options::OD_SKIP_BYTES)
            .help(t!("od.clap.od_skip_bytes"))
            .value_name("BYTES"),
        Arg::new(od_options::OD_READ_BYTES)
            .short('N')
            .long(od_options::OD_READ_BYTES)
            .help(t!("od.clap.od_read_bytes"))
            .value_name("BYTES"),
        Arg::new(od_options::OD_ENDIAN)
            .long(od_options::OD_ENDIAN)
            .help(t!("od.clap.od_endian"))
            .value_parser(["big", "little"])
            .value_name("big|little"),
        Arg::new(od_options::OD_STRINGS)
            .short('S')
            .long(od_options::OD_STRINGS)
            .help("output strings of at least BYTES graphic chars. 3 is assumed when BYTES is not specified.")
            .default_missing_value("3")
            .value_name("BYTES"),
        Arg::new("a").short('a').help(t!("od.clap.a")).action(ArgAction::SetTrue),
        Arg::new("b").short('b').help(t!("od.clap.b")).action(ArgAction::SetTrue),
        Arg::new("c").short('c').help(t!("od.clap.c")).action(ArgAction::SetTrue),
        Arg::new("d").short('d').help(t!("od.clap.d")).action(ArgAction::SetTrue),
        Arg::new("D").short('D').help(t!("od.clap.d")).action(ArgAction::SetTrue),
        Arg::new("o").short('o').help(t!("od.clap.o")).action(ArgAction::SetTrue),
        Arg::new("I").short('I').help(t!("od.clap.i")).action(ArgAction::SetTrue),
        Arg::new("L").short('L').help(t!("od.clap.l")).action(ArgAction::SetTrue),
        Arg::new("i").short('i').help(t!("od.clap.i")).action(ArgAction::SetTrue),
        Arg::new("l").short('l').help(t!("od.clap.l")).action(ArgAction::SetTrue),
        Arg::new("x").short('x').help(t!("od.clap.x")).action(ArgAction::SetTrue),
        Arg::new("h").short('h').help(t!("od.clap.h")).action(ArgAction::SetTrue),
        Arg::new("O").short('O').help(t!("od.clap.o")).action(ArgAction::SetTrue),
        Arg::new("s").short('s').help(t!("od.clap.s")).action(ArgAction::SetTrue),
        Arg::new("X").short('X').help(t!("od.clap.x")).action(ArgAction::SetTrue),
        Arg::new("H").short('H').help(t!("od.clap.h")).action(ArgAction::SetTrue),
        Arg::new("e").short('e').help(t!("od.clap.e")).action(ArgAction::SetTrue),
        Arg::new("f").short('f').help(t!("od.clap.f")).action(ArgAction::SetTrue),
        Arg::new("F").short('F').help(t!("od.clap.f")).action(ArgAction::SetTrue),
        Arg::new(od_options::OD_FORMAT)
            .short('t')
            .long("format")
            .help(t!("od.clap.od_format"))
            .action(ArgAction::Append)
            .num_args(1)
            .value_name("TYPE"),
        Arg::new(od_options::OD_OUTPUT_DUPLICATES)
            .short('v')
            .long(od_options::OD_OUTPUT_DUPLICATES)
            .help(t!("od.clap.od_output_duplicates"))
            .action(ArgAction::SetTrue),
        Arg::new(od_options::OD_WIDTH)
            .short('w')
            .long(od_options::OD_WIDTH)
            .help("output BYTES bytes per output line. 32 is implied when BYTES is not specified.")
            .default_missing_value("32")
            .value_name("BYTES")
            .num_args(..=1),
        Arg::new(od_options::OD_TRADITIONAL)
            .long(od_options::OD_TRADITIONAL)
            .help(t!("od.clap.od_traditional"))
            .action(ArgAction::SetTrue),
        Arg::new(od_options::OD_FILENAME)
            .hide(true)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::FilePath),
    ];

    Command::new(ctcore::ct_util_name())
        .version(crate_version!())
        .about(t!("od.about"))
        .override_usage(t!("od.usage"))
        .after_help(t!("od.after_help"))
        .trailing_var_arg(true)
        .dont_delimit_trailing_values(true)
        .infer_long_args(true)
        .args_override_self(true)
        .disable_help_flag(true)
        .args(args)
}

fn odexec<I>(
    input_offset: &mut OdInputOffset,
    input_decoder: &mut OdInputDecoder<I>,
    output_info: &OutputInfo,
) -> CTResult<()>
where
    I: PeekRead + HasError,
{
    let mut state = DuplicateState::new();

    loop {
        match od_process_next_line(input_offset, input_decoder, output_info, &mut state)? {
            LineProcessResult::EndOfFile => break,
            LineProcessResult::Continue => continue,
        }
    }

    if input_decoder.has_error() {
        Err(1.into())
    } else {
        Ok(())
    }
}

struct DuplicateState {
    is_duplicate: bool,
    previous_bytes: Vec<u8>,
}

impl DuplicateState {
    fn new() -> Self {
        Self {
            is_duplicate: false,
            previous_bytes: Vec::new(),
        }
    }
}

enum LineProcessResult {
    EndOfFile,
    Continue,
}

fn od_process_next_line<I: PeekRead + HasError>(
    input_offset: &mut OdInputOffset,
    input_decoder: &mut OdInputDecoder<I>,
    output_info: &OutputInfo,
    state: &mut DuplicateState,
) -> CTResult<LineProcessResult> {
    match input_decoder.od_peek_read() {
        Ok(mut memory_decoder) => {
            let length = memory_decoder.length();

            if length == 0 {
                if !input_decoder.has_error() {
                    input_offset.print_final_offset();
                }
                return Ok(LineProcessResult::EndOfFile);
            }

            od_handle_incomplete_line(&mut memory_decoder, length, output_info);
            // 向上抛出 I/O 错误
            od_process_line_content(
                input_offset,
                &mut memory_decoder,
                output_info,
                state,
                length,
            )?;

            Ok(LineProcessResult::Continue)
        }
        Err(e) => {
            ct_show_error!("{}", e);
            input_offset.print_final_offset();
            Err(1.into())
        }
    }
}

fn od_handle_incomplete_line(
    memory_decoder: &mut OdMemoryDecoder,
    length: usize,
    output_info: &OutputInfo,
) {
    if length != output_info.byte_size_line {
        let max_used = (length + output_info.byte_size_block).min(output_info.byte_size_line);
        memory_decoder.zero_out_buffer(length, max_used);
    }
}

fn od_process_line_content(
    input_offset: &mut OdInputOffset,
    memory_decoder: &mut OdMemoryDecoder,
    output_info: &OutputInfo,
    state: &mut DuplicateState,
    length: usize,
) -> CTResult<()> {
    use std::io::Write as _;
    let mut stdout = std::io::stdout().lock();

    if is_duplicate_line(memory_decoder, output_info, state, length) {
        if !state.is_duplicate {
            state.is_duplicate = true;
            writeln!(stdout, "*").map_err(|e| CtSimpleError::new(1, e.to_string()))?;
        }
    } else {
        state.is_duplicate = false;
        if length == output_info.byte_size_line {
            memory_decoder.clone_buffer(&mut state.previous_bytes);
        }
        od_print_bytes(
            &input_offset.format_byte_offset(),
            memory_decoder,
            output_info,
        )?;
    }
    input_offset.increase_position(length as u64);
    Ok(())
}

fn is_duplicate_line(
    memory_decoder: &OdMemoryDecoder,
    output_info: &OutputInfo,
    state: &DuplicateState,
    length: usize,
) -> bool {
    !output_info.output_duplicates
        && length == output_info.byte_size_line
        && memory_decoder.get_buffer(0) == &state.previous_bytes[..]
}

fn od_format_line(
    input_decoder: &OdMemoryDecoder,
    formatter: &SpacedFormatterItemInfo,
    output_info: &OutputInfo,
) -> String {
    let mut output_text = String::new();
    let mut byte_pos = 0;

    while byte_pos < input_decoder.length() {
        od_add_spacing(&mut output_text, formatter, byte_pos, output_info);
        od_format_bytes(&mut output_text, input_decoder, formatter, byte_pos);
        byte_pos += formatter.formatter_item_info.byte_size;
    }

    if formatter.add_ascii_dump {
        od_add_ascii_dump(&mut output_text, input_decoder, output_info);
    }

    output_text
}

fn od_add_spacing(
    output_text: &mut String,
    formatter: &SpacedFormatterItemInfo,
    byte_pos: usize,
    output_info: &OutputInfo,
) {
    write!(
        output_text,
        "{:>width$}",
        "",
        width = formatter.spacing[byte_pos % output_info.byte_size_block]
    )
    .unwrap();
}

fn od_format_bytes(
    output_text: &mut String,
    input_decoder: &OdMemoryDecoder,
    formatter: &SpacedFormatterItemInfo,
    byte_pos: usize,
) {
    match &formatter.formatter_item_info.formatter {
        OdFormatWriter::IntWriter(func) => {
            let value = input_decoder.read_uint(byte_pos, formatter.formatter_item_info.byte_size);
            output_text.push_str(&func(value));
        }
        OdFormatWriter::FloatWriter(func) => {
            let value = input_decoder.read_float(byte_pos, formatter.formatter_item_info.byte_size);
            output_text.push_str(&func(value));
        }
        OdFormatWriter::MultibyteWriter(func) => {
            output_text.push_str(&func(input_decoder.get_full_buffer(byte_pos)));
        }
    }
}

fn od_add_ascii_dump(
    output_text: &mut String,
    input_decoder: &OdMemoryDecoder,
    output_info: &OutputInfo,
) {
    let missing_spacing = output_info
        .print_width_line
        .saturating_sub(output_text.chars().count());
    write!(
        output_text,
        "{:>width$}  {}",
        "",
        format_ascii_dump(input_decoder.get_buffer(0)),
        width = missing_spacing
    )
    .unwrap();
}

fn od_print_formatted_line(prefix: &str, output_text: &str, is_first: bool) -> CTResult<()> {
    use std::io::Write as _;
    let mut stdout = std::io::stdout().lock();

    if is_first {
        write!(stdout, "{prefix}").map_err(|e| CtSimpleError::new(1, e.to_string()))?;
    } else {
        write!(stdout, "{:>width$}", "", width = prefix.chars().count())
            .map_err(|e| CtSimpleError::new(1, e.to_string()))?;
    }
    writeln!(stdout, "{output_text}").map_err(|e| CtSimpleError::new(1, e.to_string()))?;
    Ok(())
}

fn od_print_bytes(
    prefix: &str,
    input_decoder: &OdMemoryDecoder,
    output_info: &OutputInfo,
) -> CTResult<()> {
    let mut first = true;

    for formatter in output_info.spaced_formatters_iter() {
        let output_text = od_format_line(input_decoder, formatter, output_info);
        od_print_formatted_line(prefix, &output_text, first)?;
        first = false;
    }
    Ok(())
}

fn od_open_input_peek_reader(
    input_strings: &[String],
    skip_bytes: u64,
    read_bytes: Option<u64>,
) -> PeekReader<PartialReader<OdMultifileReader>> {
    let inputs = input_strings
        .iter()
        .map(|w| match w as &str {
            "-" => OdInputSource::Stdin,
            x => OdInputSource::FileName(x),
        })
        .collect::<Vec<_>>();

    let mf = OdMultifileReader::new(inputs);
    let pr = PartialReader::new(mf, skip_bytes, read_bytes);
    PeekReader::new(pr)
}

fn od_format_error_message(error: &ParseSizeError, s: &str, option: &str) -> String {
    match error {
        ParseSizeError::InvalidSuffix(_) => {
            format!("invalid suffix in --{} argument {}", option, s.quote())
        }
        ParseSizeError::ParseFailure(_) => format!("invalid --{} argument {}", option, s.quote()),
        ParseSizeError::SizeTooBig(_) => format!("--{} argument {} too large", option, s.quote()),
    }
}

#[derive(Default)]
pub struct Od;
impl Tool for Od {
    fn name(&self) -> &'static str {
        "od"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        od_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Od;

        // 测试 name 方法
        assert_eq!(tool.name(), "od");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("od"));

        // 测试 execute 方法
        let args = vec![OsString::from("od"), OsString::from("--version")];
        assert!(tool.execute(&args).is_err());
    }

    mod od_settings_tests {
        use super::*;

        #[test]
        fn test_new_with_defaults() {
            let matches = ct_app().try_get_matches_from(vec!["od"]).unwrap();
            let settings = OdSettings::new(&matches, &["od".to_string()]).unwrap();

            assert_eq!(settings.byte_order, ByteOrder::Native);
            assert_eq!(settings.skip_bytes, 0);
            assert_eq!(settings.read_bytes, None);
            assert_eq!(settings.line_bytes, 16);
            assert!(!settings.output_duplicates);
            assert_eq!(settings.radix, OdRadix::Octal);
        }

        #[test]
        fn test_new_with_custom_values() {
            let matches = ct_app()
                .try_get_matches_from(vec!["od", "--endian=little", "-j", "100"])
                .unwrap();
            let settings = OdSettings::new(&matches, &["od".to_string()]).unwrap();

            assert_eq!(settings.byte_order, ByteOrder::Little);
            assert_eq!(settings.skip_bytes, 100);
        }
    }

    mod odexec_tests {
        use super::*;
        #[test]
        fn test_odexec_empty_input() {
            let mut input_offset = OdInputOffset::new(OdRadix::Octal, 0, None);
            let mut input = od_open_input_peek_reader(&[], 0, None);
            let mut input_decoder =
                OdInputDecoder::new(&mut input, 16, OD_PEEK_BUFFER_SIZE, ByteOrder::Native);
            let output_info = OutputInfo::new(16, &[], false);

            assert!(odexec(&mut input_offset, &mut input_decoder, &output_info).is_ok());
        }
    }

    mod od_main_tests {
        use super::*;
        use std::ffi::OsString;
        use std::fs::File;
        use std::io::Write;

        fn setup_test_file(filename: &str, content: &[u8]) -> String {
            File::create(filename)
                .and_then(|mut file| file.write_all(content))
                .expect("Failed to create test file");
            filename.to_string()
        }

        fn cleanup_test_file(test_file: &str) {
            std::fs::remove_file(test_file).expect("Failed to remove test file");
        }

        #[test]
        fn test_od_main_default() {
            let test_file = setup_test_file("test_default.txt", b"Hello, World!");
            let result =
                od_main(vec![OsString::from("od"), OsString::from(&test_file)].into_iter());
            cleanup_test_file(&test_file);
            assert!(result.is_ok());
        }

        #[test]
        fn test_od_main_with_options() {
            let test_file =
                setup_test_file("test_options.txt", b"Hello, World! This is a test file.");
            let result = od_main(
                vec![
                    OsString::from("od"),
                    OsString::from("--endian=little"),
                    OsString::from("-j"),
                    OsString::from("5"),
                    OsString::from("-N"),
                    OsString::from("10"),
                    OsString::from(&test_file),
                ]
                .into_iter(),
            );
            cleanup_test_file(&test_file);
            assert!(result.is_ok());
        }

        #[test]
        fn test_od_main_with_format() {
            let test_file = setup_test_file("test_format.txt", b"Test data for format");
            let result = od_main(
                vec![
                    OsString::from("od"),
                    OsString::from("-t"),
                    OsString::from("x2"),
                    OsString::from("-w"),
                    OsString::from("32"),
                    OsString::from(&test_file),
                ]
                .into_iter(),
            );
            cleanup_test_file(&test_file);
            assert!(result.is_ok());
        }

        #[test]
        fn test_od_main_with_duplicate_output() {
            let test_file = setup_test_file("test_duplicate.txt", b"Duplicate test data");
            let result = od_main(
                vec![
                    OsString::from("od"),
                    OsString::from("-v"),
                    OsString::from(&test_file),
                ]
                .into_iter(),
            );
            cleanup_test_file(&test_file);
            assert!(result.is_ok());
        }

        #[test]
        fn test_od_main_with_address_radix() {
            let test_file = setup_test_file("test_radix.txt", b"Address radix test data");
            let result = od_main(
                vec![
                    OsString::from("od"),
                    OsString::from("-A"),
                    OsString::from("x"),
                    OsString::from(&test_file),
                ]
                .into_iter(),
            );
            cleanup_test_file(&test_file);
            assert!(result.is_ok());
        }
    }
}
