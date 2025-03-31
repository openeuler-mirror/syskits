/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
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

use std::cmp;
use std::fmt::Write;

use crate::byteorder_io::ByteOrder;
use crate::formatteriteminfo::OdFormatWriter;
use crate::inputdecoder::{OdInputDecoder, OdMemoryDecoder};
use crate::inputoffset::{OdInputOffset, OdRadix};
use crate::multifilereader::{HasError, OdInputSource, OdMultifileReader};
use crate::output_info::{OutputInfo, SpacedFormatterItemInfo};
use crate::parse_formats::{ParsedFormatterItemInfo, od_parse_format_flags};
use crate::parse_inputs::{CommandLineInputs, od_parse_inputs};
use crate::parse_nrofbytes::od_parse_number_of_bytes;
use crate::partialreader::PartialReader;
use crate::peekreader::{PeekRead, PeekReader};
use crate::prn_format::format_ascii_dump;
use clap::ArgAction;
use clap::{Arg, ArgMatches, Command, crate_version, parser::ValueSource};
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CtSimpleError};
use ctcore::ct_parse_size::ParseSizeError;
use ctcore::{
    ct_format_usage, ct_help_about, ct_help_section, ct_help_usage, ct_show_error, ct_show_warning,
};

const OD_PEEK_BUFFER_SIZE: usize = 4; // utf-8 can be 4 bytes

const OD_ABOUT: &str = ct_help_about!("od.md");
const OD_USAGE: &str = ct_help_usage!("od.md");
const OD_AFTER_HELP: &str = ct_help_section!("after help", "od.md");

pub(crate) mod od_options {
    pub const OD_HELP: &str = "help";
    pub const OD_ADDRESS_RADIX: &str = "address-radix";
    pub const OD_SKIP_BYTES: &str = "skip-bytes";
    pub const OD_READ_BYTES: &str = "read-bytes";
    pub const OD_ENDIAN: &str = "endian";
    pub const OD_STRINGS: &str = "strings";
    pub const OD_FORMAT: &str = "ct_format";
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
}

impl OdSettings {
    /// 从命令行参数创建新的 OdSettings 实例
    ///
    /// # Arguments
    /// * `matches` - 命令行参数匹配结果
    /// * `args` - 原始命令行参数
    ///
    /// # Returns
    /// * `CTResult<Self>` - 成功则返回 OdSettings 实例，失败则返回错误
    fn new(matches: &ArgMatches, args: &[String]) -> CTResult<Self> {
        // 解析字节序参数
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

        // 解析跳过字节数
        let mut skip_bytes = match matches.get_one::<String>(od_options::OD_SKIP_BYTES) {
            None => 0,
            Some(s) => od_parse_number_of_bytes(s).map_err(|e| {
                CtSimpleError::new(1, od_format_error_message(&e, s, od_options::OD_SKIP_BYTES))
            })?,
        };

        let mut label: Option<u64> = None;

        // 解析输入文件
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

        // 解析格式化参数
        let formats = od_parse_format_flags(args).map_err(|e| CtSimpleError::new(1, e))?;

        // 解析并验证行宽度
        let mut line_bytes = Self::parse_line_width(matches)?;
        let min_bytes = formats.iter().fold(1, |max, next| {
            cmp::max(max, next.formatter_item_info.byte_size)
        });

        if line_bytes == 0 || line_bytes % min_bytes != 0 {
            ct_show_warning!("invalid width {}; using {} instead", line_bytes, min_bytes);
            line_bytes = min_bytes;
        }

        // 解析其他选项
        let output_duplicates = matches.get_flag(od_options::OD_OUTPUT_DUPLICATES);
        let read_bytes = Self::parse_read_bytes(matches)?;
        let radix = Self::parse_radix(matches)?;

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
        })
    }

    // 辅助方法，解析行宽度
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

    // 辅助方法，解析读取字节数
    fn parse_read_bytes(matches: &ArgMatches) -> CTResult<Option<u64>> {
        match matches.get_one::<String>(od_options::OD_READ_BYTES) {
            None => Ok(None),
            Some(s) => od_parse_number_of_bytes(s).map(Some).map_err(|e| {
                CtSimpleError::new(1, od_format_error_message(&e, s, od_options::OD_READ_BYTES))
            }),
        }
    }

    // 辅助方法，解析基数
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
                    "Radix must be one of [d, o, n, x]".to_string(),
                )),
            },
        }
    }
}

/// parses and validates command line parameters, prepares data structures,
/// opens the input and calls `odfunc` to process the input.
#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    od_main(args)
}
pub fn od_main(args: impl ctcore::Args) -> CTResult<()> {
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

pub fn ct_app() -> Command {
    let args = vec![
        Arg::new(od_options::OD_HELP)
            .long(od_options::OD_HELP)
            .help("Print help information.")
            .action(ArgAction::Help),
        Arg::new(od_options::OD_ADDRESS_RADIX)
            .short('A')
            .long(od_options::OD_ADDRESS_RADIX)
            .help("Select the base in which file offsets are printed.")
            .value_name("RADIX"),
        Arg::new(od_options::OD_SKIP_BYTES)
            .short('j')
            .long(od_options::OD_SKIP_BYTES)
            .help("Skip bytes input bytes before formatting and writing.")
            .value_name("BYTES"),
        Arg::new(od_options::OD_READ_BYTES)
            .short('N')
            .long(od_options::OD_READ_BYTES)
            .help("limit dump to BYTES input bytes")
            .value_name("BYTES"),
        Arg::new(od_options::OD_ENDIAN)
            .long(od_options::OD_ENDIAN)
            .help("byte order to use for multi-byte formats")
            .value_parser(["big", "little"])
            .value_name("big|little"),
        Arg::new(od_options::OD_STRINGS)
            .short('S')
            .long(od_options::OD_STRINGS)
            .help(
                "NotImplemented: output strings of at least BYTES graphic chars. 3 is assumed when \
                    BYTES is not specified.",
            )
            .default_missing_value("3")
            .value_name("BYTES"),
        Arg::new("a")
            .short('a')
            .help("named characters, ignoring high-order bit")
            .action(ArgAction::SetTrue),
        Arg::new("b")
            .short('b')
            .help("octal bytes")
            .action(ArgAction::SetTrue),
        Arg::new("c")
            .short('c')
            .help("ASCII characters or backslash escapes")
            .action(ArgAction::SetTrue),
        Arg::new("d")
            .short('d')
            .help("unsigned decimal 2-byte units")
            .action(ArgAction::SetTrue),
        Arg::new("D")
            .short('D')
            .help("unsigned decimal 4-byte units")
            .action(ArgAction::SetTrue),
        Arg::new("o")
            .short('o')
            .help("octal 2-byte units")
            .action(ArgAction::SetTrue),
        Arg::new("I")
            .short('I')
            .help("decimal 8-byte units")
            .action(ArgAction::SetTrue),
        Arg::new("L")
            .short('L')
            .help("decimal 8-byte units")
            .action(ArgAction::SetTrue),
        Arg::new("i")
            .short('i')
            .help("decimal 4-byte units")
            .action(ArgAction::SetTrue),
        Arg::new("l")
            .short('l')
            .help("decimal 8-byte units")
            .action(ArgAction::SetTrue),
        Arg::new("x")
            .short('x')
            .help("hexadecimal 2-byte units")
            .action(ArgAction::SetTrue),
        Arg::new("h")
            .short('h')
            .help("hexadecimal 2-byte units")
            .action(ArgAction::SetTrue),
        Arg::new("O")
            .short('O')
            .help("octal 4-byte units")
            .action(ArgAction::SetTrue),
        Arg::new("s")
            .short('s')
            .help("decimal 2-byte units")
            .action(ArgAction::SetTrue),
        Arg::new("X")
            .short('X')
            .help("hexadecimal 4-byte units")
            .action(ArgAction::SetTrue),
        Arg::new("H")
            .short('H')
            .help("hexadecimal 4-byte units")
            .action(ArgAction::SetTrue),
        Arg::new("e")
            .short('e')
            .help("floating point double precision (64-bit) units")
            .action(ArgAction::SetTrue),
        Arg::new("f")
            .short('f')
            .help("floating point double precision (32-bit) units")
            .action(ArgAction::SetTrue),
        Arg::new("F")
            .short('F')
            .help("floating point double precision (64-bit) units")
            .action(ArgAction::SetTrue),
        Arg::new(od_options::OD_FORMAT)
            .short('t')
            .long("ct_format")
            .help("select output ct_format or formats")
            .action(ArgAction::Append)
            .num_args(1)
            .value_name("TYPE"),
        Arg::new(od_options::OD_OUTPUT_DUPLICATES)
            .short('v')
            .long(od_options::OD_OUTPUT_DUPLICATES)
            .help("do not use * to mark line suppression")
            .action(ArgAction::SetTrue),
        Arg::new(od_options::OD_WIDTH)
            .short('w')
            .long(od_options::OD_WIDTH)
            .help(
                "output BYTES bytes per output line. 32 is implied when BYTES is not \
                    specified.",
            )
            .default_missing_value("32")
            .value_name("BYTES")
            .num_args(..=1),
        Arg::new(od_options::OD_TRADITIONAL)
            .long(od_options::OD_TRADITIONAL)
            .help("compatibility mode with one input, offset and label.")
            .action(ArgAction::SetTrue),
        Arg::new(od_options::OD_FILENAME)
            .hide(true)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::FilePath),
    ];

    Command::new(ctcore::ct_util_name())
        .version(crate_version!())
        .about(OD_ABOUT)
        .override_usage(ct_format_usage(OD_USAGE))
        .after_help(OD_AFTER_HELP)
        .trailing_var_arg(true)
        .dont_delimit_trailing_values(true)
        .infer_long_args(true)
        .args_override_self(true)
        .disable_help_flag(true)
        .args(args)
}

/// 执行OD命令的主要处理函数
///
/// # Arguments
/// * `input_offset` - 输入偏移量管理器
/// * `input_decoder` - 输入解码器
/// * `output_info` - 输出格式信息
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

/// 用于跟踪重复行状态的结构体
struct DuplicateState {
    is_duplicate: bool,      // 标记当前是否处于重复行状态
    previous_bytes: Vec<u8>, // 存储上一行的内容用于比较
}

impl DuplicateState {
    /// 创建新的重复行状态跟踪器
    fn new() -> Self {
        Self {
            is_duplicate: false,
            previous_bytes: Vec::new(),
        }
    }
}

/// 行处理的结果枚举
enum LineProcessResult {
    EndOfFile, // 到达文件末尾
    Continue,  // 继续处理下一行
}

/// 处理输入流的下一行数据
///
/// # Arguments
/// * `input_offset` - 输入偏移量管理器
/// * `input_decoder` - 输入解码器
/// * `output_info` - 输出格式信息
/// * `state` - 重复行状态跟踪器
fn od_process_next_line<I: PeekRead + HasError>(
    input_offset: &mut OdInputOffset,
    input_decoder: &mut OdInputDecoder<I>,
    output_info: &OutputInfo,
    state: &mut DuplicateState,
) -> CTResult<LineProcessResult> {
    // 尝试读取下一行数据
    match input_decoder.od_peek_read() {
        Ok(mut memory_decoder) => {
            // 获取实际读取的字节数
            let length = memory_decoder.length();

            // 如果没有读到数据，说明到达文件末尾
            if length == 0 {
                input_offset.print_final_offset();
                return Ok(LineProcessResult::EndOfFile);
            }

            od_handle_incomplete_line(&mut memory_decoder, length, output_info);
            od_process_line_content(
                input_offset,
                &mut memory_decoder,
                output_info,
                state,
                length,
            );

            Ok(LineProcessResult::Continue)
        }
        // 处理读取错误
        Err(e) => {
            ct_show_error!("{}", e);
            input_offset.print_final_offset();
            Err(1.into())
        }
    }
}

/// 处理不完整的行数据
///
/// # Arguments
/// * `memory_decoder` - 内存解码器
/// * `length` - 实际读取的字节数
/// * `output_info` - 输出格式信息
fn od_handle_incomplete_line(
    memory_decoder: &mut OdMemoryDecoder,
    length: usize,
    output_info: &OutputInfo,
) {
    // 处理最后一行不完整的情况
    if length != output_info.byte_size_line {
        // 计算需要填充零的范围
        let max_used = (length + output_info.byte_size_block).min(output_info.byte_size_line);
        // 将未填满的部分用零填充
        memory_decoder.zero_out_buffer(length, max_used);
    }
}

/// 处理行内容
///
/// # Arguments
/// * `input_offset` - 输入偏移量管理器
/// * `memory_decoder` - 内存解码器
/// * `output_info` - 输出格式信息
/// * `state` - 重复行状态跟踪器
/// * `length` - 当前行的字节数
fn od_process_line_content(
    input_offset: &mut OdInputOffset,
    memory_decoder: &mut OdMemoryDecoder,
    output_info: &OutputInfo,
    state: &mut DuplicateState,
    length: usize,
) {
    // 检查是否是重复行且不需要输出重复内容
    if is_duplicate_line(memory_decoder, output_info, state, length) {
        // 第一次遇到重复行时打印 *
        if !state.is_duplicate {
            state.is_duplicate = true;
            println!("*");
        }
    } else {
        // 不是重复行，重置标记
        state.is_duplicate = false;
        // 如果是完整行，保存内容用于后续比较
        if length == output_info.byte_size_line {
            memory_decoder.clone_buffer(&mut state.previous_bytes);
        }
        // 打印当前行
        od_print_bytes(
            &input_offset.format_byte_offset(),
            memory_decoder,
            output_info,
        );
    }
    // 更新偏移量
    input_offset.increase_position(length as u64);
}

/// 检查是否是重复行
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

/// 格式化一行数据
fn od_format_line(
    input_decoder: &OdMemoryDecoder,
    formatter: &SpacedFormatterItemInfo,
    output_info: &OutputInfo,
) -> String {
    let mut output_text = String::new();
    let mut byte_pos = 0;

    // 处理当前行的所有字节
    while byte_pos < input_decoder.length() {
        od_add_spacing(&mut output_text, formatter, byte_pos, output_info);
        od_format_bytes(&mut output_text, input_decoder, formatter, byte_pos);
        byte_pos += formatter.formatter_item_info.byte_size;
    }

    // 添加ASCII转储（如果需要）
    if formatter.add_ascii_dump {
        od_add_ascii_dump(&mut output_text, input_decoder, output_info);
    }

    output_text
}

/// 添加格式化所需的空格
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

/// 格式化字节数据
fn od_format_bytes(
    output_text: &mut String,
    input_decoder: &OdMemoryDecoder,
    formatter: &SpacedFormatterItemInfo,
    byte_pos: usize,
) {
    // 根据不同的格式化器类型处理数据
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

/// 添加ASCII转储
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

/// 打印格式化后的行
fn od_print_formatted_line(prefix: &str, output_text: &str, is_first: bool) {
    // 只在第一个格式时打印地址
    if is_first {
        print!("{prefix}");
    } else {
        print!("{:>width$}", "", width = prefix.chars().count());
    }
    println!("{output_text}");
}

/// 打印一行数据
///
/// # Arguments
/// * `prefix` - 行前缀（地址）
/// * `input_decoder` - 输入解码器
/// * `output_info` - 输出格式信息
fn od_print_bytes(prefix: &str, input_decoder: &OdMemoryDecoder, output_info: &OutputInfo) {
    let mut first = true;

    for formatter in output_info.spaced_formatters_iter() {
        let output_text = od_format_line(input_decoder, formatter, output_info);
        od_print_formatted_line(prefix, &output_text, first);
        first = false;
    }
}

/// 创建输入流读取器
///
/// # Arguments
/// * `input_strings` - 输入文件列表
/// * `skip_bytes` - 需要跳过的字节数
/// * `read_bytes` - 需要读取的字节数
fn od_open_input_peek_reader(
    input_strings: &[String],
    skip_bytes: u64,
    read_bytes: Option<u64>,
) -> PeekReader<PartialReader<OdMultifileReader>> {
    // 将输入字符串转换为输入源
    let inputs = input_strings
        .iter()
        .map(|w| match w as &str {
            "-" => OdInputSource::Stdin,
            x => OdInputSource::FileName(x),
        })
        .collect::<Vec<_>>();

    // 创建多文件读取器
    let mf = OdMultifileReader::new(inputs);
    // 创建部分读取器（处理跳过和限制读取）
    let pr = PartialReader::new(mf, skip_bytes, read_bytes);
    // 创建支持预读的读取器
    PeekReader::new(pr)
}

fn od_format_error_message(error: &ParseSizeError, s: &str, option: &str) -> String {
    // NOTE:
    // GNU's od echos affected flag, -N or --read-bytes (-j or --skip-bytes, etc.), depending user's selection
    match error {
        ParseSizeError::InvalidSuffix(_) => {
            format!("invalid suffix in --{} argument {}", option, s.quote())
        }
        ParseSizeError::ParseFailure(_) => format!("invalid --{} argument {}", option, s.quote()),
        ParseSizeError::SizeTooBig(_) => format!("--{} argument {} too large", option, s.quote()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
