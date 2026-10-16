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

//! 对每个指定的文件设置自动换行（折行），并将重新排版后的结果输出到标准输出。
//!
//! 算法逻辑和模式说明
//!
//! 支持的三种计数模式
//!
//! 1. 列模式（Column Mode，默认模式）
//! - 计算方式：按显示列数计算，考虑字符的实际显示宽度
//! - 中文字符处理：中文字符通常占2列宽度
//! - Emoji处理：emoji通常占1-2列宽度（根据具体字符而定）
//! - 特殊字符处理：
//!   - `\t`（制表符）：计算到下一个8的倍数位置
//!   - `\b`（退格符）：减少1列位置
//!   - `\r`（回车符）：重置到行首（位置0）
//!   - `\n`（换行符）：特殊处理，直接输出当前行并重置状态
//! - 换行时机：当下一个字符会导致显示列数超过指定宽度时换行
//! - 处理方式：逐行读取和处理
//!
//! 2. 字节模式（Byte Mode，-b选项）
//! - 计算方式：按当前locale编码后的字节长度计算
//! - 关键特性：完全按照GNU fold.c的multibyte_text函数实现
//! - 换行符处理：C/POSIX单字节locale下结束当前行；多字节locale下按普通字节计数
//! - UTF-8中文字符：一个中文字符通常占3个字节
//! - UTF-8 Emoji字符：通常占4个字节
//! - ASCII字符：每个字符占1个字节
//! - 换行时机：当column + 字符字节长度 > 宽度时换行
//! - 处理方式：字符流处理，使用固定大小的buffer增量读取
//! - Rescan机制：模拟GNU fold的goto rescan逻辑，超过宽度时先换行再重新处理当前字符
//!
//! 3. 字符模式（Character Mode，-c选项）
//! - 计算方式：按Unicode字符数计算，每个字符计为1个单位
//! - 关键特性：完全按照GNU fold.c的character_mode实现
//! - 中文字符处理：每个中文字符计为1个字符（而非2列或3字节）
//! - 特殊字符处理：所有字符（包括制表符、退格符、换行符）都简单计为1，不进行特殊扩展或处理
//! - 换行符处理：不进行特殊处理，换行符被当作普通的1字符处理
//! - Emoji处理：每个emoji字符计为1个字符
//! - 处理方式：使用字符流处理（类似字节模式），而非逐行处理
//! - Rescan机制：模拟GNU fold的goto rescan逻辑，超过宽度时先换行再重新处理当前字符
//!
//! 空格分割选项（-s，--spaces）
//! - 功能：在空白字符处优先分割，避免在单词中间断行
//! - 实现机制：
//!   - 记录最后一个空白字符的位置（`last_blank_pos`）
//!   - 当超过宽度限制时，优先在空白字符处分割
//!   - 重新计算剩余部分的列/字节/字符数
//! - 适用范围：所有三种模式都支持空格分割
//!
//! 核心算法实现（完全按照GNU fold.c的架构）
//!
//! 统一文本折叠算法（fold_file_multibyte_unified）
//! - 适用于：所有三种模式（字节模式、字符模式、列模式）
//! - 完全模拟GNU fold.c的fold_multibyte_text函数
//!
//! 三种模式的差异：
//! - **字节模式**：increment = mblength（字符的UTF-8字节长度）
//! - **字符模式**：increment = 1（每个字符固定为1）
//! - **列模式**：increment = 复杂的列宽度计算
//!   * 制表符：计算到下一个8的倍数位置
//!   * 退格符：向后退一个位置（如果可能）
//!   * 回车符：重置到行首
//!   * 换行符：特殊处理，直接输出并重置状态
//!   * 其他字符：使用Unicode显示宽度
//!
//! 算法步骤：
//! 1. 增量读取文件到buffer，保留可能不完整的多字节序列
//! 2. 对每个字符：
//!    a. 获取字符及其UTF-8字节长度（mblength）
//!    b. 列模式下换行符的特殊处理：直接输出并重置状态
//!    c. 根据模式计算increment
//!    d. 列模式下特殊字符（\r, \b）的位置更新
//!    e. 进入rescan循环：
//!    如果 column + increment > width：
//!    * 如果启用-s且有空白位置：在空白处分割
//!    * 否则如果column != 0：输出当前行，重置状态，继续rescan
//!      否则：添加字符到输出，column += increment
//!      f. 记录空白字符位置（用于-s选项）
//! 3. 输出剩余内容

extern crate rust_i18n;
use clap::error::ErrorKind;
use clap::{Arg, ArgAction, Command, crate_version};
use ctcore::Tool;
use ctcore::ct_error::{CTResult, CtSimpleError, set_ct_exit_code, strip_errno};
use rust_i18n::t;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::Path;
use sys_locale::get_locale;

const FOLD_TAB_WIDTH: usize = 8; // 制表符宽度
const FOLD_READ_BUFFER_SIZE: usize = 8 * 1024;
// GNU uses MB_LEN_MAX and falls back to 16 when the platform value is unusable.
const FOLD_MB_LEN_MAX: usize = 16;

unsafe extern "C" {
    fn mbrtowc(
        wide_char: *mut libc::wchar_t,
        bytes: *const libc::c_char,
        byte_count: usize,
        state: *mut libc::mbstate_t,
    ) -> usize;
    fn iswblank(wide_char: libc::c_uint) -> libc::c_int;
    fn wcwidth(wide_char: libc::wchar_t) -> libc::c_int;
}

#[cfg(target_env = "gnu")]
unsafe extern "C" {
    fn __ctype_get_mb_cur_max() -> usize;
}

// 支持的语言环境
rust_i18n::i18n!("locales", fallback = "en");

// 各种参数选项的常量定义
mod fold_flags {
    pub const FOLD_BYTES: &str = "bytes";
    pub const FOLD_CHARACTERS: &str = "characters";
    pub const FOLD_SPACES: &str = "spaces";
    pub const FOLD_WIDTH: &str = "width";
    pub const FOLD_FILE: &str = "file";
}

/// 包含折叠操作相关标志的结构体
struct FoldFlags {
    bytes: bool,
    characters: bool,
    spaces: bool,
    width: usize,
    files: Vec<OsString>,
}

/// 字符计数模式
#[derive(Debug, Clone, Copy, PartialEq)]
enum CountMode {
    Columns,    // 默认模式：按显示列数计算（中文字符占2列）
    Bytes,      // -b模式：按字节数计算
    Characters, // -c模式：按Unicode字符数计算
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoldMode {
    Columns,
    Bytes,
    Characters,
}

#[cfg(not(target_env = "gnu"))]
fn fold_locale_is_utf8() -> bool {
    std::env::var("LC_ALL")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::env::var("LC_CTYPE")
                .ok()
                .filter(|value| !value.is_empty())
        })
        .or_else(|| std::env::var("LANG").ok().filter(|value| !value.is_empty()))
        .is_some_and(|locale| {
            let locale = locale.to_ascii_lowercase();
            locale.contains("utf-8") || locale.contains("utf8")
        })
}

fn fold_locale_is_single_byte() -> bool {
    unsafe {
        libc::setlocale(libc::LC_ALL, c"".as_ptr());
    }

    #[cfg(target_env = "gnu")]
    unsafe {
        __ctype_get_mb_cur_max() <= 1
    }

    #[cfg(not(target_env = "gnu"))]
    {
        !fold_locale_is_utf8()
    }
}

#[derive(Debug, Clone, Copy)]
struct FoldDecodedChar {
    wide_char: libc::wchar_t,
    byte_len: usize,
}

fn fold_decode_next(
    bytes: &[u8],
    state: &mut libc::mbstate_t,
    single_byte_locale: bool,
    conversion_failed: &mut bool,
) -> FoldDecodedChar {
    if single_byte_locale {
        return FoldDecodedChar {
            wide_char: libc::wchar_t::from(bytes[0]),
            byte_len: 1,
        };
    }

    let saved_state = *state;
    let mut wide_char = 0;
    let byte_len = unsafe {
        mbrtowc(
            &mut wide_char,
            bytes.as_ptr().cast::<libc::c_char>(),
            bytes.len(),
            state,
        )
    };

    match byte_len {
        value if value == usize::MAX || value == usize::MAX - 1 => {
            *conversion_failed = true;
            *state = saved_state;
            FoldDecodedChar {
                wide_char: libc::wchar_t::from(bytes[0]),
                byte_len: 1,
            }
        }
        0 => FoldDecodedChar {
            wide_char: 0,
            byte_len: 1,
        },
        byte_len => FoldDecodedChar {
            wide_char,
            byte_len,
        },
    }
}

fn fold_character_width(wide_char: libc::wchar_t) -> usize {
    let width = unsafe { wcwidth(wide_char) };
    usize::try_from(width).unwrap_or(0)
}

fn fold_is_blank(wide_char: libc::wchar_t) -> bool {
    unsafe { iswblank(wide_char as libc::c_uint) != 0 }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldRow {
    pub row_index: usize,
    pub line: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldSemantic {
    pub mode: FoldMode,
    pub width: usize,
    pub break_spaces: bool,
    pub rows: Vec<FoldRow>,
    pub classic_text: String,
    pub classic_bytes: Vec<u8>,
    pub stderr_text: String,
    pub exit_code: i32,
}

struct FoldRunOutcome {
    stderr_text: String,
    exit_code: i32,
}

struct FoldFileOutcome {
    read_error: Option<std::io::Error>,
}

fn fold_mode(flags: &FoldFlags) -> FoldMode {
    if flags.bytes {
        FoldMode::Bytes
    } else if flags.characters {
        FoldMode::Characters
    } else {
        FoldMode::Columns
    }
}

fn fold_rows_from_output(output: &str) -> Vec<FoldRow> {
    output
        .split_terminator('\n')
        .enumerate()
        .map(|(index, line)| FoldRow {
            row_index: index + 1,
            line: line.to_string(),
        })
        .collect()
}

/// 通用的折叠文件函数 - 完全按照GNU fold.c的架构实现
fn fold_file_generic<T: Read, W: Write>(
    writer: &mut W,
    file: BufReader<T>,
    is_spaces: bool,
    width: usize,
    mode: CountMode,
) -> CTResult<FoldFileOutcome> {
    fold_file_multibyte_unified(
        writer,
        file,
        is_spaces,
        width,
        mode,
        fold_locale_is_single_byte(),
    )
}

/// 统一的文本折叠函数 - 完全按照GNU fold.c的fold_multibyte_text函数实现
/// 处理所有三种模式：字节模式、字符模式和列模式
fn fold_file_multibyte_unified<T: Read, W: Write>(
    writer: &mut W,
    mut file: BufReader<T>,
    is_spaces: bool,
    width: usize,
    mode: CountMode,
    single_byte_locale: bool,
) -> CTResult<FoldFileOutcome> {
    let mut buffer = Vec::with_capacity(FOLD_READ_BUFFER_SIZE + FOLD_MB_LEN_MAX);
    let mut buffer_pos = 0;
    let mut eof = false;
    let mut read_error = None;
    let mut column = 0;
    let mut output = Vec::new();
    let mut last_blank_pos = None;
    let mut last_blank_column = 0;
    let mut last_blank_increment = 0;
    let mut is_blank_seen = false;
    let mut is_bs_following_last_blank = false;
    let mut bs_following_last_blank_num = 0;
    let mut is_cr_after_last_blank = false;
    let mut conversion_failed = false;
    let mut conversion_state = unsafe { std::mem::zeroed::<libc::mbstate_t>() };
    let uses_column_rules =
        mode == CountMode::Columns || (single_byte_locale && mode == CountMode::Characters);
    let decode_lookahead = if single_byte_locale {
        1
    } else {
        FOLD_MB_LEN_MAX
    };
    loop {
        if buffer.len() - buffer_pos < decode_lookahead && !eof {
            if buffer_pos != 0 {
                let remaining = buffer.len() - buffer_pos;
                buffer.copy_within(buffer_pos.., 0);
                buffer.truncate(remaining);
                buffer_pos = 0;
            }

            while buffer.len() < decode_lookahead && !eof {
                let old_len = buffer.len();
                buffer.resize(old_len + FOLD_READ_BUFFER_SIZE, 0);
                let bytes_read = loop {
                    match file.read(&mut buffer[old_len..]) {
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(error) => {
                            read_error = Some(error);
                            eof = true;
                            break 0;
                        }
                        Ok(bytes_read) => break bytes_read,
                    }
                };
                buffer.truncate(old_len + bytes_read);
                if bytes_read == 0 {
                    eof = true;
                }
            }
        }

        if buffer_pos == buffer.len() {
            break;
        }

        let start_pos = buffer_pos;
        let decoded = fold_decode_next(
            &buffer[buffer_pos..],
            &mut conversion_state,
            single_byte_locale,
            &mut conversion_failed,
        );
        let wide_char = decoded.wide_char;
        let mblength = decoded.byte_len;
        let convfail = conversion_failed;

        // GNU fold_text handles newlines before counting in single-byte locales,
        // while fold_multibyte_text only does so in column mode.
        if wide_char == libc::wchar_t::from(b'\n')
            && (single_byte_locale || (mode == CountMode::Columns && !convfail))
        {
            writer.write_all(&output)?;
            writer.write_all(b"\n")?;
            column = 0;
            output.clear();
            last_blank_pos = None;
            last_blank_column = 0;
            last_blank_increment = 0;
            is_blank_seen = false;
            is_bs_following_last_blank = false;
            bs_following_last_blank_num = 0;
            is_cr_after_last_blank = false;
            buffer_pos += mblength;
            continue;
        }

        // rescan循环
        loop {
            let increment = if mode == CountMode::Bytes {
                mblength
            } else if !uses_column_rules || convfail {
                1
            } else {
                match wide_char {
                    value if value == libc::wchar_t::from(b'\r') => column,
                    value if value == libc::wchar_t::from(b'\t') => {
                        FOLD_TAB_WIDTH - column % FOLD_TAB_WIDTH
                    }
                    value if value == libc::wchar_t::from(b'\x08') => usize::from(column > 0),
                    _ if single_byte_locale => 1,
                    _ => fold_character_width(wide_char),
                }
            };

            // 列模式下特殊字符的位置更新需要特殊处理
            if uses_column_rules && !convfail {
                match wide_char {
                    value if value == libc::wchar_t::from(b'\r') => {
                        column = 0;
                        output.extend_from_slice(&buffer[start_pos..start_pos + mblength]);
                        if is_blank_seen && !convfail {
                            is_cr_after_last_blank = true;
                        }
                        is_bs_following_last_blank = false;
                        break;
                    }
                    value if value == libc::wchar_t::from(b'\x08') => {
                        if column > 0 {
                            column = column.saturating_sub(1);
                        }
                        output.extend_from_slice(&buffer[start_pos..start_pos + mblength]);
                        if is_bs_following_last_blank && !convfail {
                            bs_following_last_blank_num += 1;
                        } else {
                            is_bs_following_last_blank = false;
                        }
                        break;
                    }
                    _ => {}
                }
            }

            // 检查是否会超过宽度
            if column + increment > width {
                // 处理spaces选项
                if is_spaces && last_blank_pos.is_some() {
                    let blank_pos = last_blank_pos.unwrap();

                    writer.write_all(&output[..blank_pos])?;
                    writer.write_all(b"\n")?;

                    let remaining = output[blank_pos..].to_vec();
                    output = remaining;

                    let column_adjustment = if is_cr_after_last_blank {
                        last_blank_increment
                    } else {
                        bs_following_last_blank_num
                    };
                    // GNU performs this expression with size_t arithmetic.
                    column = column
                        .wrapping_sub(last_blank_column)
                        .wrapping_add(column_adjustment);
                    last_blank_pos = None;
                    last_blank_column = 0;
                    last_blank_increment = 0;
                    is_blank_seen = false;
                    is_bs_following_last_blank = false;
                    bs_following_last_blank_num = 0;
                    is_cr_after_last_blank = false;

                    continue;
                }

                if column != 0 {
                    // 输出当前行（不包括当前字符），重置，然后rescan当前字符
                    writer.write_all(&output)?;
                    writer.write_all(b"\n")?;
                    column = 0;
                    output.clear();
                    last_blank_pos = None;
                    last_blank_column = 0;
                    last_blank_increment = 0;
                    is_blank_seen = false;
                    is_bs_following_last_blank = false;
                    bs_following_last_blank_num = 0;
                    is_cr_after_last_blank = false;

                    // 重新处理当前字符（继续rescan循环）
                    continue;
                }
            }

            // 字符可以放入当前行（或column == 0且字符太大的特殊情况）
            output.extend_from_slice(&buffer[start_pos..start_pos + mblength]);
            column += increment;

            if is_blank_seen && !convfail && wide_char == libc::wchar_t::from(b'\r') {
                is_cr_after_last_blank = true;
            }

            if is_bs_following_last_blank && !convfail && wide_char == libc::wchar_t::from(b'\x08')
            {
                bs_following_last_blank_num += 1;
            } else {
                is_bs_following_last_blank = false;
            }

            // 记录空格位置
            if is_spaces && !convfail && fold_is_blank(wide_char) {
                last_blank_pos = Some(output.len());
                last_blank_column = column;
                last_blank_increment = increment;
                is_blank_seen = true;
                is_bs_following_last_blank = true;
                bs_following_last_blank_num = 0;
                is_cr_after_last_blank = false;
            }

            break; // 退出rescan循环
        }

        buffer_pos += mblength;
    }

    // 输出剩余内容
    if !output.is_empty() {
        writer.write_all(&output)?;
    }

    Ok(FoldFileOutcome { read_error })
}

/// 主折叠函数，用于处理命令行参数并输出结果
///
/// # Parameters
///
/// - `writer`: 一个实现了Write trait的可变引用，用于输出结果
/// - `args`: 命令行参数的切片
///
/// # Returns
///
/// 返回一个Result，表示操作成功或失败
pub fn fold_main<W: Write>(writer: &mut W, args: impl ctcore::Args) -> CTResult<()> {
    // 设置语言
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let (matches, obs_width) = match fold_arg_matches(args) {
        Ok(m) => m,
        Err(e) => {
            if e.kind() == ErrorKind::ArgumentConflict {
                set_ct_exit_code(2); // 检查是否是参数冲突错误，如果是则返回退出码2
                return Ok(());
            }
            return Err(e.into());
        }
    };

    let flags = fold_flags_from_matches(&matches, obs_width)?;
    fold(writer, &flags)
}

fn fold_flags_from_matches(
    matches: &clap::ArgMatches,
    obs_width: Option<String>,
) -> CTResult<FoldFlags> {
    let width_arg = match matches.get_one::<String>(fold_flags::FOLD_WIDTH) {
        Some(v) => Some(v.as_str()),
        None => obs_width.as_deref(),
    };
    let width = parse_fold_width(width_arg)?;

    Ok(FoldFlags {
        bytes: matches.get_flag(fold_flags::FOLD_BYTES),
        characters: matches.get_flag(fold_flags::FOLD_CHARACTERS),
        spaces: matches.get_flag(fold_flags::FOLD_SPACES),
        width,
        files: match matches.get_many::<OsString>(fold_flags::FOLD_FILE) {
            Some(v) => v.cloned().collect(),
            None => vec![OsString::from("-")],
        },
    })
}

fn fold_arg_matches(
    args: impl ctcore::Args,
) -> Result<(clap::ArgMatches, Option<String>), clap::Error> {
    let os_args: Vec<OsString> = args.collect();
    let (args, obs_width) = handle_obsolete(&os_args);
    let matches = ct_app().try_get_matches_from(args)?;
    Ok((matches, obs_width))
}

fn parse_fold_width(width: Option<&str>) -> CTResult<usize> {
    let Some(width) = width else {
        return Ok(80);
    };

    match width.parse::<usize>() {
        Ok(0) => Err(CtSimpleError::new(
            1,
            format!("invalid number of columns: '{width}': Numerical result out of range"),
        )),
        Ok(width) => Ok(width),
        Err(_) if !width.is_empty() && width.chars().all(|c| c.is_ascii_digit()) => {
            Err(CtSimpleError::new(
                1,
                format!(
                    "invalid number of columns: '{width}': Value too large for defined data type"
                ),
            ))
        }
        Err(_) => Err(CtSimpleError::new(
            1,
            format!("invalid number of columns: '{width}'"),
        )),
    }
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("fold.about");
    let usage_description = t!("fold.usage");
    let args = vec![
        Arg::new(fold_flags::FOLD_BYTES)
            .long(fold_flags::FOLD_BYTES)
            .short('b')
            .help("count bytes rather than columns")
            .action(ArgAction::SetTrue)
            .conflicts_with(fold_flags::FOLD_CHARACTERS),
        Arg::new(fold_flags::FOLD_CHARACTERS)
            .long(fold_flags::FOLD_CHARACTERS)
            .short('c')
            .help(t!("fold.clap.fold_characters"))
            .action(ArgAction::SetTrue)
            .conflicts_with(fold_flags::FOLD_BYTES),
        Arg::new(fold_flags::FOLD_SPACES)
            .long(fold_flags::FOLD_SPACES)
            .short('s')
            .help(t!("fold.clap.fold_spaces"))
            .action(ArgAction::SetTrue),
        Arg::new(fold_flags::FOLD_WIDTH)
            .long(fold_flags::FOLD_WIDTH)
            .short('w')
            .help(t!("fold.clap.fold_width"))
            .value_name("WIDTH")
            .value_parser(clap::builder::StringValueParser::new())
            .allow_hyphen_values(true)
            .overrides_with(fold_flags::FOLD_WIDTH),
        Arg::new(fold_flags::FOLD_FILE)
            .hide(true)
            .action(ArgAction::Append)
            .value_parser(clap::builder::OsStringValueParser::new())
            .value_hint(clap::ValueHint::FilePath),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(&args)
}

/// 处理过时的参数。
///
/// 该函数检查命令行参数列表，查找以单个连字符（-）开头且后跟数字的参数。
/// 如果找到这样的参数，则将其改写为等价的 -wWIDTH，并保留原始顺序。
///
/// # 参数
///
/// - `args`: 命令行参数列表。
///
/// # 返回值
///
/// - 一个包含处理后参数的向量。
/// - 一个可选的字符串，表示最后一个过时参数的值。
fn handle_obsolete(args: &[OsString]) -> (Vec<OsString>, Option<String>) {
    let mut result = Vec::with_capacity(args.len());
    let mut obsolete = None;
    let mut after_end_of_options = false;

    for arg in args {
        if after_end_of_options {
            result.push(arg.clone());
            continue;
        }

        if arg == OsStr::new("--") {
            after_end_of_options = true;
            result.push(arg.clone());
            continue;
        }

        if let Some(width) = obsolete_width_arg(arg) {
            result.push(OsString::from(format!("-w{width}")));
            obsolete = Some(width.to_owned());
        } else {
            result.push(arg.clone());
        }
    }

    (result, obsolete)
}

fn obsolete_width_arg(arg: &OsStr) -> Option<&str> {
    let arg = arg.to_str()?;
    if arg.starts_with("--") {
        return None;
    }

    let width = arg.strip_prefix('-')?;
    if width.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        Some(width)
    } else {
        None
    }
}

/// 对文件内容进行折叠处理
///
/// 该函数根据提供的折叠标志（`FoldFlags`）对指定的文件进行处理。
/// 如果文件名是`-`，则从标准输入读取内容；否则，从指定的文件中读取内容。
/// 然后根据`bytes`标志决定是按字节还是按列进行折叠，并根据`spaces`标志决定是否在空格处进行换行。
///
/// # 参数
///
/// - `fold_flags`: 包含折叠标志的结构体。
///
/// # 返回值
///
/// - 如果折叠成功，返回`Ok(())`；如果发生错误，返回`Err`。
fn fold<W: Write>(writer: &mut W, fold_flags: &FoldFlags) -> CTResult<()> {
    let outcome = fold_with_writer(writer, fold_flags)?;
    if !outcome.stderr_text.is_empty() {
        eprint!("{}", outcome.stderr_text);
    }
    if outcome.exit_code != 0 {
        set_ct_exit_code(outcome.exit_code);
    }
    Ok(())
}

fn fold_with_writer<W: Write>(writer: &mut W, fold_flags: &FoldFlags) -> CTResult<FoldRunOutcome> {
    let mut stderr_text = String::new();
    let mut exit_code = 0;

    for filename in &fold_flags.files {
        let input: Box<dyn Read> = if filename == OsStr::new("-") {
            // 如果文件名是`-`，则从标准输入读取内容
            ctcore::ct_io::stdin_reader_box()
        } else {
            // 否则，从指定的文件中读取内容
            match File::open(Path::new(filename)) {
                Ok(f) => Box::new(f) as Box<dyn Read>,
                Err(e) => {
                    let error_msg = match e.kind() {
                        std::io::ErrorKind::NotFound => "No such file or directory".to_string(),
                        std::io::ErrorKind::PermissionDenied => "Permission denied".to_string(),
                        _ => e.to_string(),
                    };
                    let filename = filename.to_string_lossy();
                    stderr_text.push_str(&format!("fold: {filename}: {error_msg}\n"));
                    continue;
                }
            }
        };
        let buffer = BufReader::new(input);

        let spaces = fold_flags.spaces;
        let width = fold_flags.width;

        // 确定计数模式
        let mode = if fold_flags.bytes {
            CountMode::Bytes
        } else if fold_flags.characters {
            CountMode::Characters
        } else {
            CountMode::Columns
        };

        let outcome = fold_file_generic(writer, buffer, spaces, width, mode)?;
        if let Some(error) = outcome.read_error {
            let filename = filename.to_string_lossy();
            stderr_text.push_str(&format!("fold: {filename}: {}\n", strip_errno(&error)));
            exit_code = 1;
        }
    }
    Ok(FoldRunOutcome {
        stderr_text,
        exit_code,
    })
}

pub fn fold_native_semantic(args: impl ctcore::Args) -> CTResult<FoldSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let os_args: Vec<OsString> = args.collect();
    let (args, obs_width) = handle_obsolete(&os_args);
    let matches = match ct_app().try_get_matches_from(args) {
        Ok(m) => m,
        Err(e) => {
            if e.kind() == ErrorKind::ArgumentConflict {
                return Ok(FoldSemantic {
                    mode: FoldMode::Columns,
                    width: obs_width
                        .and_then(|value| value.parse::<usize>().ok())
                        .unwrap_or(80),
                    break_spaces: false,
                    rows: Vec::new(),
                    classic_text: String::new(),
                    classic_bytes: Vec::new(),
                    stderr_text: String::new(),
                    exit_code: 2,
                });
            }
            return Err(e.into());
        }
    };

    let flags = fold_flags_from_matches(&matches, obs_width)?;
    let mut classic_output = Vec::new();
    let outcome = fold_with_writer(&mut classic_output, &flags)?;
    let classic_text = String::from_utf8_lossy(&classic_output).into_owned();

    Ok(FoldSemantic {
        mode: fold_mode(&flags),
        width: flags.width,
        break_spaces: flags.spaces,
        rows: fold_rows_from_output(&classic_text),
        classic_text,
        classic_bytes: classic_output,
        stderr_text: outcome.stderr_text,
        exit_code: outcome.exit_code,
    })
}

#[derive(Default)]
pub struct Fold;
impl Tool for Fold {
    fn name(&self) -> &'static str {
        "fold"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        // 调用原有的 fold_main 函数，传入 stdout 作为 writer
        let mut stdout = std::io::stdout();
        fold_main(&mut stdout, args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    // 新增：测试 Tool trait 的基本实现
    #[test]
    fn test_tool_implementation() {
        let tool = Fold;

        // 测试 name 方法
        assert_eq!(tool.name(), "fold");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("fold"));

        // 测试 execute 方法
        let args = vec![OsString::from("fold"), OsString::from("--version")];
        assert!(tool.execute(&args).is_err());
    }

    #[cfg(test)]
    mod fold_main_tests {
        use super::*;
        use std::ffi::OsString;
        use std::fs::File;
        use std::sync::Mutex;
        use tempfile::tempdir;

        static EXIT_CODE_TEST_LOCK: Mutex<()> = Mutex::new(());

        #[test]
        fn test_ctmain_version() {
            let mut writer = Vec::new();
            let args = [
                OsString::from(ctcore::ct_util_name()),
                OsString::from("--version"),
            ];
            let result = fold_main(&mut writer, args.iter().cloned());
            match result {
                Err(output) => {
                    assert_eq!(output.code(), 0);
                }
                Ok(output) => {
                    println!("{output:?}");
                }
            }
        }

        #[test]
        fn test_ctmain_v() {
            let mut writer = Vec::new();
            let args = [ctcore::ct_util_name(), "-V"];
            let result = fold_main(&mut writer, args.iter().map(OsString::from));
            match result {
                Err(output) => {
                    assert_eq!(output.code(), 0);
                }
                Ok(output) => {
                    println!("{output:?}");
                }
            }
        }

        #[test]
        fn test_ctmain_help() {
            let mut writer = Vec::new();
            let args = [ctcore::ct_util_name(), "--help"];
            let result = fold_main(&mut writer, args.iter().map(OsString::from));
            match result {
                Err(output) => {
                    assert_eq!(output.code(), 0);
                }
                Ok(output) => {
                    println!("{output:?}");
                }
            }
        }

        #[test]
        fn test_ctmain_h() {
            let mut writer = Vec::new();
            let args = [ctcore::ct_util_name(), "-h"];
            let result = fold_main(&mut writer, args.iter().map(OsString::from));
            match result {
                Err(output) => {
                    assert_eq!(output.code(), 0);
                }
                Ok(output) => {
                    println!("{output:?}");
                }
            }
        }

        #[test]
        fn test_ct_main_long_option_b_short() {
            let mut writer = Vec::new();
            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let temp_file_path = temp_dir.path().join("fold_temp_file1.txt");
            let mut temp_file =
                File::create(&temp_file_path).expect("Failed to create temporary file");
            temp_file
                .write_all(b"aaaaaaaaaaaaaaaaaaaaaaaaa\n")
                .expect("Failed to write to temporary file");
            let binding = temp_file_path.to_string_lossy().into_owned();
            let args = [ctcore::ct_util_name(), "-b", &binding];
            let result = fold_main(&mut writer, args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_long_option_b_long() {
            let mut writer = Vec::new();

            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let temp_file_path = temp_dir.path().join("fold_temp_file1.txt");
            let mut temp_file =
                File::create(&temp_file_path).expect("Failed to create temporary file");
            temp_file
                .write_all(b"File 1\n")
                .expect("Failed to write to temporary file");
            let binding = temp_file_path.to_string_lossy().into_owned();

            let args = [ctcore::ct_util_name(), "--bytes", &binding];
            let result = fold_main(&mut writer, args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_long_option_s_short() {
            let mut writer = Vec::new();

            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let temp_file_path = temp_dir.path().join("fold_temp_file1.txt");
            let mut temp_file =
                File::create(&temp_file_path).expect("Failed to create temporary file");
            temp_file
                .write_all(b"File 1\n")
                .expect("Failed to write to temporary file");
            let binding = temp_file_path.to_string_lossy().into_owned();

            let args = [ctcore::ct_util_name(), "-s", &binding];
            let result = fold_main(&mut writer, args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_long_option_s_long() {
            let mut writer = Vec::new();

            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let temp_file_path = temp_dir.path().join("fold_temp_file1.txt");
            let mut temp_file =
                File::create(&temp_file_path).expect("Failed to create temporary file");
            temp_file
                .write_all(b"File 1\n")
                .expect("Failed to write to temporary file");
            let binding = temp_file_path.to_string_lossy().into_owned();

            let args = [ctcore::ct_util_name(), "--spaces", &binding];
            let result = fold_main(&mut writer, args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_long_option_w_short() {
            let mut writer = Vec::new();

            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let temp_file_path = temp_dir.path().join("fold_temp_file1.txt");
            let mut temp_file =
                File::create(&temp_file_path).expect("Failed to create temporary file");
            temp_file
                .write_all(b"File 1\n")
                .expect("Failed to write to temporary file");
            let binding = temp_file_path.to_string_lossy().into_owned();

            let args = [ctcore::ct_util_name(), "-w", "10", &binding];
            let result = fold_main(&mut writer, args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_long_option_w_long() {
            // 使用临时文件而不是标准输入来避免阻塞
            let content = "test content for width option";
            let temp_file = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(temp_file.path(), content).unwrap();

            let args = vec![
                OsString::from("fold"),
                OsString::from("--width"),
                OsString::from("10"),
                temp_file.path().as_os_str().to_os_string(),
            ];

            let mut writer = Vec::new();
            let result = fold_main(&mut writer, args.into_iter());

            assert!(result.is_ok());
            let output = String::from_utf8(writer).unwrap();
            assert!(output.contains("test"));
        }

        #[test]
        fn test_ct_main_obsolete_width_after_w_overrides() {
            let temp_file = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(temp_file.path(), "123456789\n").unwrap();

            let args = vec![
                OsString::from("fold"),
                OsString::from("-w8"),
                OsString::from("-3"),
                temp_file.path().as_os_str().to_os_string(),
            ];

            let mut writer = Vec::new();
            let result = fold_main(&mut writer, args.into_iter());

            assert!(result.is_ok());
            assert_eq!(String::from_utf8(writer).unwrap(), "123\n456\n789\n");
        }

        #[test]
        fn test_ct_main_w_after_obsolete_width_overrides() {
            let temp_file = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(temp_file.path(), "123456789\n").unwrap();

            let args = vec![
                OsString::from("fold"),
                OsString::from("-3"),
                OsString::from("-w8"),
                temp_file.path().as_os_str().to_os_string(),
            ];

            let mut writer = Vec::new();
            let result = fold_main(&mut writer, args.into_iter());

            assert!(result.is_ok());
            assert_eq!(String::from_utf8(writer).unwrap(), "12345678\n9\n");
        }

        #[test]
        fn test_ct_main_long_option_c_short() {
            // 使用临时文件而不是标准输入来避免阻塞
            let content = "测试字符abc";
            let temp_file = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(temp_file.path(), content).unwrap();

            let args = vec![
                OsString::from("fold"),
                OsString::from("-c"),
                temp_file.path().as_os_str().to_os_string(),
            ];

            let mut writer = Vec::new();
            let result = fold_main(&mut writer, args.into_iter());

            assert!(result.is_ok());
            let output = String::from_utf8(writer).unwrap();
            assert!(output.contains("测试"));
        }

        #[test]
        fn test_ct_main_long_option_c_long() {
            // 使用临时文件而不是标准输入来避免阻塞
            let content = "测试字符abc";
            let temp_file = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(temp_file.path(), content).unwrap();

            let args = vec![
                OsString::from("fold"),
                OsString::from("--characters"),
                temp_file.path().as_os_str().to_os_string(),
            ];

            let mut writer = Vec::new();
            let result = fold_main(&mut writer, args.into_iter());

            assert!(result.is_ok());
            let output = String::from_utf8(writer).unwrap();
            assert!(output.contains("测试"));
        }

        #[test]
        fn test_ct_main_mutually_exclusive_b_c_exit_code() {
            // 测试互斥选项-b和-c返回退出码2
            use ctcore::ct_error::{get_ct_exit_code, set_ct_exit_code};
            let _guard = EXIT_CODE_TEST_LOCK
                .lock()
                .expect("failed to lock exit code test mutex");

            // 重置退出码
            set_ct_exit_code(0);

            let mut writer = Vec::new();
            let args = [ctcore::ct_util_name(), "-b", "-c", "/etc/passwd"];
            let result = fold_main(&mut writer, args.iter().map(OsString::from));

            // 应该返回Ok()，因为我们设置了exit_code并返回Ok
            assert!(result.is_ok());
            // 检查退出码应该是2
            assert_eq!(get_ct_exit_code(), 2);

            // 重置退出码
            set_ct_exit_code(0);
        }

        #[test]
        fn test_ct_main_mutually_exclusive_c_b_exit_code() {
            // 测试互斥选项-c和-b返回退出码2（顺序相反）
            use ctcore::ct_error::{get_ct_exit_code, set_ct_exit_code};
            let _guard = EXIT_CODE_TEST_LOCK
                .lock()
                .expect("failed to lock exit code test mutex");

            // 重置退出码
            set_ct_exit_code(0);

            let mut writer = Vec::new();
            let args = [ctcore::ct_util_name(), "-c", "-b", "/etc/passwd"];
            let result = fold_main(&mut writer, args.iter().map(OsString::from));

            // 应该返回Ok()，因为我们设置了exit_code并返回Ok
            assert!(result.is_ok());
            // 检查退出码应该是2
            assert_eq!(get_ct_exit_code(), 2);

            // 重置退出码
            set_ct_exit_code(0);
        }

        #[test]
        fn test_ct_main_characters_functionality() {
            if fold_locale_is_single_byte() {
                return;
            }

            // 测试-c选项的字符计数功能
            let mut writer = Vec::new();

            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let temp_file_path = temp_dir.path().join("fold_temp_utf8.txt");
            let mut temp_file =
                File::create(&temp_file_path).expect("Failed to create temporary file");
            // 写入包含UTF-8字符的内容，确保字符计数正确
            temp_file
                .write_all("这是中文字符测试abcd".as_bytes())
                .expect("Failed to write to temporary file");
            let binding = temp_file_path.to_string_lossy().into_owned();

            // 测试字符模式 -c，设置宽度为5字符
            let args = [ctcore::ct_util_name(), "-c", "-w", "5", &binding];
            let result = fold_main(&mut writer, args.iter().map(OsString::from));
            assert!(result.is_ok());

            let output = String::from_utf8(writer).unwrap();
            // 验证输出包含折行
            assert!(!output.is_empty());
            // 验证输出包含中文字符
            assert!(output.contains("这是中文"));
        }
    }

    #[cfg(test)]
    mod ct_app_tests {
        use clap::error::ErrorKind;

        use super::*;

        // fold 接口测试: fold [OPTION]... [FILE]...
        //
        // Options:
        //   -b, --bytes          count bytes rather than columns
        //   -s, --spaces         break lines at word boundaries rather than a hard cut-off
        //   -w, --width <WIDTH>  set WIDTH as the maximum line width rather than 80
        //   -h, --help           Print help
        //   -V, --version        Print version

        #[test]
        fn test_ct_app_execution_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];

            // Assuming `command` has a method to retrieve the executable name, replace it with the actual one
            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_other_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];
            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
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
        fn test_ct_app_long_option_b_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-b"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
            assert!(executable.unwrap().contains_id(fold_flags::FOLD_BYTES));
        }

        #[test]
        fn test_ct_app_long_option_b_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--bytes"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
            assert!(executable.unwrap().contains_id(fold_flags::FOLD_BYTES));
        }

        #[test]
        fn test_ct_app_long_option_s_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-s"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
            assert!(executable.unwrap().contains_id(fold_flags::FOLD_SPACES));
        }

        #[test]
        fn test_ct_app_long_option_s_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--spaces"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
            assert!(executable.unwrap().contains_id(fold_flags::FOLD_SPACES));
        }

        #[test]
        fn test_ct_app_long_option_w_short_err() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-w"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_err());
        }

        #[test]
        fn test_ct_app_long_option_w_long_err() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--width"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_err());
        }

        #[test]
        fn test_ct_app_long_option_w_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-w", "10"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
            assert!(executable.unwrap().contains_id(fold_flags::FOLD_WIDTH));
        }

        #[test]
        fn test_ct_app_long_option_w_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--width", "10"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
            assert!(executable.unwrap().contains_id(fold_flags::FOLD_WIDTH));
        }

        #[test]
        fn test_ct_app_long_option_c_short() {
            let command = ct_app();
            let args = vec!["fold", "-c"];
            let matches = command.try_get_matches_from(args).unwrap();
            assert!(matches.get_flag("characters"));
        }

        #[test]
        fn test_ct_app_long_option_c_long() {
            let command = ct_app();
            let args = vec!["fold", "--characters"];
            let matches = command.try_get_matches_from(args).unwrap();
            assert!(matches.get_flag("characters"));
        }

        #[test]
        fn test_ct_app_combined_options_c_s() {
            let command = ct_app();
            let args = vec!["fold", "-c", "-s", "-w", "20"];
            let matches = command.try_get_matches_from(args).unwrap();
            assert!(matches.get_flag("characters"));
            assert!(matches.get_flag("spaces"));
            assert_eq!(matches.get_one::<String>("width").unwrap(), "20");
        }

        #[test]
        fn test_ct_app_mutually_exclusive_b_c() {
            // 测试-b和-c选项互斥，应该返回错误
            let command = ct_app();
            let args = vec!["fold", "-b", "-c"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            if let Err(e) = result {
                assert_eq!(e.kind(), clap::error::ErrorKind::ArgumentConflict);
            }
        }

        #[test]
        fn test_ct_app_mutually_exclusive_c_b() {
            // 测试-c和-b选项互斥，应该返回错误（顺序相反）
            let command = ct_app();
            let args = vec!["fold", "-c", "-b"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            if let Err(e) = result {
                assert_eq!(e.kind(), clap::error::ErrorKind::ArgumentConflict);
            }
        }
    }

    #[cfg(test)]
    mod handle_obsolete_tests {
        /*
        分支 1：参数以单个连字符（-）开头并后跟一个数字。
            测试用例 1：参数列表中包含一个过时参数（例如，"-1"）。
            测试用例 2：参数列表中包含多个过时参数，确保只处理第一个（例如，"-1", "-2"）。
        分支 2：参数不以单个连字符（-）开头或不后跟数字。
            测试用例 3：参数列表中不包含过时参数（例如，"foo", "bar"）。
            测试用例 4：参数列表中包含以连字符开头但不后跟数字的参数（例如，"-foo"）
        */
        use super::*;

        fn os_args(args: &[&str]) -> Vec<OsString> {
            args.iter().map(OsString::from).collect()
        }

        #[test]
        fn handle_obsolete_with_obsolete_parameter_removes_and_returns_value() {
            let args = os_args(&["foo", "-1", "bar"]);
            let (result, obsolete) = handle_obsolete(&args);
            assert_eq!(result, os_args(&["foo", "-w1", "bar"]));
            assert_eq!(obsolete, Some("1".to_string()));
        }

        #[test]
        fn handle_obsolete_with_multiple_obsolete_parameters_removes_first_and_returns_value() {
            let args = os_args(&["foo", "-1", "-2", "bar"]);
            let (result, obsolete) = handle_obsolete(&args);
            assert_eq!(result, os_args(&["foo", "-w1", "-w2", "bar"]));
            assert_eq!(obsolete, Some("2".to_string()));
        }

        #[test]
        fn handle_obsolete_without_obsolete_parameters_returns_original_list_and_none() {
            let args = os_args(&["foo", "bar"]);
            let (result, obsolete) = handle_obsolete(&args);
            assert_eq!(result, os_args(&["foo", "bar"]));
            assert_eq!(obsolete, None);
        }

        #[test]
        fn handle_obsolete_with_non_numeric_parameter_returns_original_list_and_none() {
            let args = os_args(&["foo", "-foo", "bar"]);
            let (result, obsolete) = handle_obsolete(&args);
            assert_eq!(result, os_args(&["foo", "-foo", "bar"]));
            assert_eq!(obsolete, None);
        }

        #[test]
        fn handle_obsolete_with_empty_args_returns_empty_list_and_none() {
            let args: Vec<OsString> = vec![];
            let (result, obsolete) = handle_obsolete(&args);
            assert_eq!(result, Vec::<OsString>::new());
            assert_eq!(obsolete, None);
        }

        #[test]
        fn handle_obsolete_with_only_obsolete_parameters_removes_all_and_returns_first_value() {
            let args = os_args(&["-1", "-2", "-3"]);
            let (result, obsolete) = handle_obsolete(&args);
            assert_eq!(result, os_args(&["-w1", "-w2", "-w3"]));
            assert_eq!(obsolete, Some("3".to_string()));
        }

        #[test]
        fn handle_obsolete_with_obsolete_parameter_at_beginning_removes_and_returns_value() {
            let args = os_args(&["-1", "foo", "bar"]);
            let (result, obsolete) = handle_obsolete(&args);
            assert_eq!(result, os_args(&["-w1", "foo", "bar"]));
            assert_eq!(obsolete, Some("1".to_string()));
        }

        #[test]
        fn handle_obsolete_with_obsolete_parameter_in_middle_removes_and_returns_value() {
            let args = os_args(&["foo", "-2", "bar"]);
            let (result, obsolete) = handle_obsolete(&args);
            assert_eq!(result, os_args(&["foo", "-w2", "bar"]));
            assert_eq!(obsolete, Some("2".to_string()));
        }

        #[test]
        fn handle_obsolete_with_obsolete_parameter_at_end_removes_and_returns_value() {
            let args = os_args(&["foo", "bar", "-3"]);
            let (result, obsolete) = handle_obsolete(&args);
            assert_eq!(result, os_args(&["foo", "bar", "-w3"]));
            assert_eq!(obsolete, Some("3".to_string()));
        }

        #[test]
        fn handle_obsolete_with_double_dash_parameter_does_not_remove() {
            let args = os_args(&["foo", "--2", "bar"]);
            let (result, obsolete) = handle_obsolete(&args);
            assert_eq!(result, os_args(&["foo", "--2", "bar"]));
            assert_eq!(obsolete, None);
        }

        #[test]
        fn handle_obsolete_after_end_of_options_stays_file_operand() {
            let args = os_args(&["fold", "--", "-3"]);
            let (result, obsolete) = handle_obsolete(&args);
            assert_eq!(result, os_args(&["fold", "--", "-3"]));
            assert_eq!(obsolete, None);
        }

        #[test]
        fn handle_obsolete_with_negative_number_parameter_removes_and_returns_value() {
            let args = os_args(&["foo", "-123", "bar"]);
            let (result, obsolete) = handle_obsolete(&args);
            assert_eq!(result, os_args(&["foo", "-w123", "bar"]));
            assert_eq!(obsolete, Some("123".to_string()));
        }

        #[test]
        fn handle_obsolete_with_leading_or_trailing_spaces_does_not_remove() {
            let args = os_args(&["foo", " -1 ", "bar"]);
            let (result, obsolete) = handle_obsolete(&args);
            assert_eq!(result, os_args(&["foo", " -1 ", "bar"]));
            assert_eq!(obsolete, None);
        }
    }

    #[cfg(test)]
    mod fold_tests {
        use super::*;
        use ctcore::ct_error::CTResult;
        use std::io::{self, BufReader, BufWriter, Read, Write};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use tempfile::NamedTempFile;

        struct StreamingProbeReader {
            input: Vec<u8>,
            offset: usize,
            bytes_written: Arc<AtomicUsize>,
        }

        impl Read for StreamingProbeReader {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                if self.offset < self.input.len() {
                    let bytes_read = buffer.len().min(self.input.len() - self.offset);
                    buffer[..bytes_read]
                        .copy_from_slice(&self.input[self.offset..self.offset + bytes_read]);
                    self.offset += bytes_read;
                    return Ok(bytes_read);
                }

                if self.bytes_written.load(Ordering::SeqCst) == 0 {
                    return Err(io::Error::other(
                        "fold requested EOF before producing output",
                    ));
                }

                Ok(0)
            }
        }

        struct StreamingProbeWriter {
            output: Vec<u8>,
            bytes_written: Arc<AtomicUsize>,
        }

        impl Write for StreamingProbeWriter {
            fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
                self.output.extend_from_slice(buffer);
                self.bytes_written.fetch_add(buffer.len(), Ordering::SeqCst);
                Ok(buffer.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        #[derive(Default)]
        struct ErrorAfterDataReader {
            returned_data: bool,
        }

        impl Read for ErrorAfterDataReader {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                if self.returned_data {
                    Err(io::Error::other("simulated read error"))
                } else {
                    buffer[..3].copy_from_slice(b"abc");
                    self.returned_data = true;
                    Ok(3)
                }
            }
        }

        /// 写入临时文件的辅助函数
        fn write_temp_file(content: &str) -> NamedTempFile {
            let mut temp_file = NamedTempFile::new().expect("Failed to create temp file");
            write!(temp_file, "{content}").expect("Failed to write to temp file");
            temp_file
        }

        #[test]
        fn fold_native_semantic_preserves_non_utf8_output() {
            let mut temp_file = NamedTempFile::new().expect("create temp file");
            temp_file.write_all(b"\xff").expect("write temp file");
            let path = temp_file.path().as_os_str().to_os_string();

            let semantic = fold_native_semantic([OsString::from("fold"), path].into_iter())
                .expect("fold semantic");

            assert_eq!(semantic.classic_bytes, b"\xff");
            assert_eq!(semantic.classic_text, "\u{fffd}");
        }

        #[cfg(unix)]
        #[test]
        fn test_fold_opens_non_utf8_filename() -> CTResult<()> {
            use std::os::unix::ffi::{OsStrExt, OsStringExt};

            let temp_dir = tempfile::tempdir().expect("create temp dir");
            let mut path_bytes = temp_dir.path().as_os_str().as_bytes().to_vec();
            path_bytes.push(b'/');
            path_bytes.push(0xff);
            let path = OsString::from_vec(path_bytes);

            std::fs::write(Path::new(&path), b"invalid-name-content\n")
                .expect("write non-UTF-8 filename");

            let mut writer = Vec::new();
            fold_main(&mut writer, [OsString::from("fold"), path].into_iter())?;

            assert_eq!(writer, b"invalid-name-content\n");
            Ok(())
        }

        fn fold_bytes(content: &str, width: usize, single_byte_locale: bool) -> String {
            let mut writer = Vec::new();
            fold_file_multibyte_unified(
                &mut writer,
                BufReader::new(content.as_bytes()),
                false,
                width,
                CountMode::Bytes,
                single_byte_locale,
            )
            .expect("fold byte input");
            String::from_utf8(writer).expect("fold output is UTF-8")
        }

        #[test]
        fn fold_streams_output_before_reaching_eof() -> CTResult<()> {
            let bytes_written = Arc::new(AtomicUsize::new(0));
            let reader = StreamingProbeReader {
                input: b"x\n".to_vec(),
                offset: 0,
                bytes_written: Arc::clone(&bytes_written),
            };
            let mut writer = StreamingProbeWriter {
                output: Vec::new(),
                bytes_written,
            };

            fold_file_multibyte_unified(
                &mut writer,
                BufReader::new(reader),
                false,
                1,
                CountMode::Columns,
                true,
            )?;

            assert_eq!(writer.output, b"x\n");
            Ok(())
        }

        #[test]
        fn fold_preserves_multibyte_character_across_read_boundary() -> CTResult<()> {
            if fold_locale_is_single_byte() {
                return Ok(());
            }

            let mut input = vec![b'a'; FOLD_READ_BUFFER_SIZE - 1];
            input.extend_from_slice("界b".as_bytes());

            let mut writer = Vec::new();
            fold_file_multibyte_unified(
                &mut writer,
                BufReader::new(input.as_slice()),
                false,
                FOLD_READ_BUFFER_SIZE,
                CountMode::Bytes,
                false,
            )?;

            let mut expected = vec![b'a'; FOLD_READ_BUFFER_SIZE - 1];
            expected.push(b'\n');
            expected.extend_from_slice("界b".as_bytes());
            assert_eq!(writer, expected);
            Ok(())
        }

        #[test]
        fn fold_flushes_buffered_output_before_reporting_read_error() -> CTResult<()> {
            let mut writer = Vec::new();
            let outcome = fold_file_multibyte_unified(
                &mut writer,
                BufReader::new(ErrorAfterDataReader::default()),
                false,
                2,
                CountMode::Columns,
                true,
            )?;

            assert_eq!(writer, b"ab\nc");
            assert_eq!(
                outcome.read_error.expect("read should fail").to_string(),
                "simulated read error"
            );
            Ok(())
        }

        #[test]
        fn test_fold_spaces_breaks_after_multibyte_blank() -> CTResult<()> {
            if fold_locale_is_single_byte() {
                return Ok(());
            }

            let mut writer = Vec::new();
            fold_file_multibyte_unified(
                &mut writer,
                BufReader::new("a\u{3000}bcde".as_bytes()),
                true,
                4,
                CountMode::Columns,
                false,
            )?;

            assert_eq!(writer, "a\u{3000}\nbcde".as_bytes());
            Ok(())
        }

        #[test]
        fn test_fold_nonexistent_file_with_width() {
            let fold_flags = FoldFlags {
                bytes: false,
                characters: false,
                spaces: false,
                width: 20,
                files: vec![OsString::from("nonexistent_file.txt")],
            };

            let mut writer = Vec::new();
            let result = fold(&mut writer, &fold_flags);

            // 验证执行成功（因为错误被处理了）
            assert!(result.is_ok());

            // 验证输出为空（因为文件不存在）
            assert_eq!(String::from_utf8(writer).unwrap(), "");
        }

        #[test]
        fn test_fold_existing_file_with_width() -> CTResult<()> {
            // 创建一个临时文件并写入测试内容
            let content = "This is a test file with content that should be folded at width 20.\nThis is another line that should also be folded properly.";
            let temp_file = write_temp_file(content);

            let fold_flags = FoldFlags {
                bytes: false,
                characters: false,
                spaces: false,
                width: 20,
                files: vec![temp_file.path().as_os_str().to_os_string()],
            };

            let mut writer = Vec::new();
            fold(&mut writer, &fold_flags)?;

            let output = String::from_utf8(writer).unwrap();
            println!("output: {output}");
            // 验证输出是否按照20个字符宽度正确折行
            assert!(output.contains("This is a test file"));
            assert!(output.contains("with content that sh"));
            assert!(output.contains("ould be folded at wi"));
            assert!(output.contains("dth 20."));
            assert!(output.contains("This is another line"));
            assert!(output.contains(" that should also be"));
            assert!(output.contains(" folded properly."));

            Ok(())
        }

        #[test]
        fn test_fold_single_file_bytewise_no_spaces() -> CTResult<()> {
            // 创建一个临时文件，并写入内容
            let content = "Hello Rust World!";
            let temp_file = write_temp_file(content);

            // 构造 FoldFlags：开启 bytes 模式，不在空格处折行，设定行宽 5
            let fold_flags = FoldFlags {
                bytes: true,
                characters: false,
                spaces: false,
                width: 5,
                files: vec![temp_file.path().as_os_str().to_os_string()],
            };

            // 创建一个内存 writer
            let mut writer = Vec::new();

            // 调用 fold 函数
            fold(&mut writer, &fold_flags)?;

            // 读取输出并断言
            let output = String::from_utf8(writer).unwrap();
            assert_eq!(output, "Hello\n Rust\n Worl\nd!");

            Ok(())
        }

        #[test]
        fn test_fold_single_file_bytewise_spaces() -> CTResult<()> {
            // 在空白处断行，当 `spaces=true` 时，折叠时优先寻找空格位置
            let content = "Hello Rust World!";
            let temp_file = write_temp_file(content);

            // 设定行宽较小，观察空格折行效果
            let fold_flags = FoldFlags {
                bytes: true,
                characters: false,
                spaces: true,
                width: 6,
                files: vec![temp_file.path().as_os_str().to_os_string()],
            };

            let mut writer = Vec::new();
            fold(&mut writer, &fold_flags)?;

            let output = String::from_utf8(writer).unwrap();
            // 断行逻辑示例（以 6 为上限，优先在空格处分割）：
            // "Hello " -> (空格前就能折行)
            // "Rust "  -> ...
            // "World!"
            assert_eq!(output, "Hello \nRust \nWorld!");

            Ok(())
        }

        #[test]
        fn test_fold_single_file_columnwise_spaces() -> CTResult<()> {
            // 不开启 bytewise，而是使用默认列宽模式
            // （列宽模式会把制表符视为多列、退格符减少列数等，但此处仅测试普通字符+空格折行）
            let content = "Hello Rust World!";
            let temp_file = write_temp_file(content);

            let fold_flags = FoldFlags {
                bytes: false,
                characters: false,
                spaces: true,
                width: 6,
                files: vec![temp_file.path().as_os_str().to_os_string()],
            };

            let mut writer = Vec::new();
            fold(&mut writer, &fold_flags)?;

            let output = String::from_utf8(writer).unwrap();
            // 在列模式下，遇到 6 列后会折行。由于 spaces=true，会在最后一次空格处断开
            // 可能结果与 bytewise 类似，但内部对制表符等有不同处理方式
            assert_eq!(output, "Hello \nRust \nWorld!");

            Ok(())
        }

        #[test]
        fn test_fold_single_file_columnwise_no_spaces() -> CTResult<()> {
            let content = "Hello Rust World!";
            let temp_file = write_temp_file(content);

            let fold_flags = FoldFlags {
                bytes: false,
                characters: false,
                spaces: false,
                width: 6,
                files: vec![temp_file.path().as_os_str().to_os_string()],
            };

            let mut writer = Vec::new();
            fold(&mut writer, &fold_flags)?;

            let output = String::from_utf8(writer).unwrap();
            // 不在空格处优先断行，直接按列宽截断
            // "Hello " -> 6列
            // "Rust W" -> 6列
            // "orld!"
            // 注意列宽模式下，对 \t 等可能处理不同，但此例中不存在 \t
            assert_eq!(output, "Hello \nRust W\norld!");

            Ok(())
        }

        #[test]
        fn test_fold_multiple_files() -> CTResult<()> {
            let content1 = "File1 content.\nNext line in file1.";
            let content2 = "File2 content.\nNext line in file2.";
            let temp_file1 = write_temp_file(content1);
            let temp_file2 = write_temp_file(content2);

            let fold_flags = FoldFlags {
                bytes: false,
                characters: false,
                spaces: false,
                width: 10,
                files: vec![
                    temp_file1.path().as_os_str().to_os_string(),
                    temp_file2.path().as_os_str().to_os_string(),
                ],
            };

            let mut writer = Vec::new();
            fold(&mut writer, &fold_flags)?;

            let output = String::from_utf8(writer).unwrap();
            // 简单断言一下，里面应包含来自 file1 与 file2 的折行结果
            assert!(output.contains("File1"));
            assert!(output.contains("File2"));

            Ok(())
        }

        #[test]
        fn test_fold_file_not_found() {
            // 指定一个不存在的文件路径
            let fold_flags = FoldFlags {
                bytes: false,
                characters: false,
                spaces: false,
                width: 10,
                files: vec![OsString::from("this_file_does_not_exist.xyz")],
            };

            let mut writer = BufWriter::new(Vec::new());
            // fold 应该返回错误
            let result = fold(&mut writer, &fold_flags);
            assert!(result.is_ok());
        }

        #[test]
        fn fold_files_file_not_found_returns_ok() {
            let mut output = Vec::new();
            let fold_flags = FoldFlags {
                files: vec![OsString::from("nonexistent.txt")],
                bytes: false,
                characters: false,
                spaces: true,
                width: 80,
            };

            // 尝试读取不存在的文件
            let result = fold(&mut output, &fold_flags);

            // 验证错误
            assert!(result.is_ok());
        }

        #[test]
        fn test_fold_single_file_characterwise_no_spaces() -> CTResult<()> {
            if fold_locale_is_single_byte() {
                return Ok(());
            }

            // 测试字符计数模式：中文字符每个算作1个字符
            let content = "这是测试中文字符串abc";
            let temp_file = write_temp_file(content);

            let fold_flags = FoldFlags {
                bytes: false,
                characters: true,
                spaces: false,
                width: 8,
                files: vec![temp_file.path().as_os_str().to_os_string()],
            };

            let mut writer = Vec::new();
            fold(&mut writer, &fold_flags)?;

            let output = String::from_utf8(writer).unwrap();
            // 按字符计数：前8个字符是"这是测试中文字符"，剩下"串abc"
            assert_eq!(output, "这是测试中文字符\n串abc");

            Ok(())
        }

        #[test]
        fn test_fold_single_file_characterwise_spaces() -> CTResult<()> {
            if fold_locale_is_single_byte() {
                return Ok(());
            }

            // 测试字符计数模式配合空格断行
            let content = "这是 测试 中文字符 串";
            let temp_file = write_temp_file(content);

            let fold_flags = FoldFlags {
                bytes: false,
                characters: true,
                spaces: true,
                width: 6,
                files: vec![temp_file.path().as_os_str().to_os_string()],
            };

            let mut writer = Vec::new();
            fold(&mut writer, &fold_flags)?;

            let output = String::from_utf8(writer).unwrap();
            // 按字符计数且在空格处断行：
            // "这是 测试 " 中，"这是 " 3个字符， "测试 " 3个字符，"中文字符 " 5个字符，"串" 1个字符
            // 实际会是"这是 测试 "，"中文字符 串"
            assert_eq!(output, "这是 测试 \n中文字符 串");

            Ok(())
        }

        #[test]
        fn test_fold_bytewise_utf8_character_boundaries() -> CTResult<()> {
            if fold_locale_is_single_byte() {
                return Ok(());
            }

            // 测试字节模式正确处理UTF-8字符边界
            let content = "中文测试abc";
            let temp_file = write_temp_file(content);

            let fold_flags = FoldFlags {
                bytes: true,
                characters: false,
                spaces: false,
                width: 10, // 10字节：每个中文字符3字节，"中文测"=9字节，"试"会在下一行
                files: vec![temp_file.path().as_os_str().to_os_string()],
            };

            let mut writer = Vec::new();
            fold(&mut writer, &fold_flags)?;

            let output = String::from_utf8(writer).unwrap();
            // 字节模式但保证UTF-8字符完整性
            assert_eq!(output, "中文测\n试abc");

            Ok(())
        }

        #[test]
        fn test_fold_mixed_ascii_utf8_characters() -> CTResult<()> {
            if fold_locale_is_single_byte() {
                return Ok(());
            }

            // 测试混合ASCII和UTF-8字符的字符计数模式
            let content = "Hello世界123测试";
            let temp_file = write_temp_file(content);

            let fold_flags = FoldFlags {
                bytes: false,
                characters: true,
                spaces: false,
                width: 8,
                files: vec![temp_file.path().as_os_str().to_os_string()],
            };

            let mut writer = Vec::new();
            fold(&mut writer, &fold_flags)?;

            let output = String::from_utf8(writer).unwrap();
            // 按字符计数：前8个字符"Hello世界1"，剩下"23测试"
            assert_eq!(output, "Hello世界1\n23测试");

            Ok(())
        }

        #[test]
        fn test_fold_characterwise_vs_bytewise_difference() -> CTResult<()> {
            if fold_locale_is_single_byte() {
                return Ok(());
            }

            // 演示字符模式和字节模式的区别
            let content = "测试中文abc";
            let temp_file1 = write_temp_file(content);
            let temp_file2 = write_temp_file(content);

            // 字符模式
            let char_flags = FoldFlags {
                bytes: false,
                characters: true,
                spaces: false,
                width: 6,
                files: vec![temp_file1.path().as_os_str().to_os_string()],
            };

            let mut char_writer = Vec::new();
            fold(&mut char_writer, &char_flags)?;
            let char_output = String::from_utf8(char_writer).unwrap();

            // 字节模式
            let byte_flags = FoldFlags {
                bytes: true,
                characters: false,
                spaces: false,
                width: 6,
                files: vec![temp_file2.path().as_os_str().to_os_string()],
            };

            let mut byte_writer = Vec::new();
            fold(&mut byte_writer, &byte_flags)?;
            let byte_output = String::from_utf8(byte_writer).unwrap();

            // 字符模式：6个字符 "测试中文ab"，剩下"c"
            assert_eq!(char_output, "测试中文ab\nc");

            // 字节模式：6字节只能容纳2个中文字符"测试"(6字节)，剩下"中文abc"
            assert_eq!(byte_output, "测试\n中文\nabc");

            Ok(())
        }

        #[test]
        fn test_fold_characters_vs_bytes_vs_columns() -> CTResult<()> {
            if fold_locale_is_single_byte() {
                return Ok(());
            }

            // 综合测试三种模式的区别：字符模式、字节模式、列模式
            let content = "中文测试abc";
            let temp_file = write_temp_file(content);

            // 字符模式
            let fold_flags_chars = FoldFlags {
                bytes: false,
                characters: true,
                spaces: false,
                width: 5,
                files: vec![temp_file.path().as_os_str().to_os_string()],
            };

            // 字节模式
            let fold_flags_bytes = FoldFlags {
                bytes: true,
                characters: false,
                spaces: false,
                width: 5,
                files: vec![temp_file.path().as_os_str().to_os_string()],
            };

            // 列模式（默认）
            let fold_flags_cols = FoldFlags {
                bytes: false,
                characters: false,
                spaces: false,
                width: 5,
                files: vec![temp_file.path().as_os_str().to_os_string()],
            };

            let mut writer_chars = Vec::new();
            let mut writer_bytes = Vec::new();
            let mut writer_cols = Vec::new();

            fold(&mut writer_chars, &fold_flags_chars)?;
            fold(&mut writer_bytes, &fold_flags_bytes)?;
            fold(&mut writer_cols, &fold_flags_cols)?;

            let output_chars = String::from_utf8(writer_chars).unwrap();
            let output_bytes = String::from_utf8(writer_bytes).unwrap();
            let output_cols = String::from_utf8(writer_cols).unwrap();

            // 验证三种输出都不为空
            assert!(!output_chars.is_empty());
            assert!(!output_bytes.is_empty());
            assert!(!output_cols.is_empty());

            // 验证字符模式：按字符计数，宽度5，"中文测试a"是5个字符，应该在第一行
            assert!(output_chars.contains("中文测试a"));

            // 验证字节模式：由于UTF-8边界保护，应该包含中文字符
            assert!(output_bytes.contains("中") && output_bytes.contains("文"));
            assert!(output_bytes.contains("测") && output_bytes.contains("试"));

            // 验证列模式：按显示列数计算，中文字符占2列，"中文"=4列，"测试a"=5列
            assert!(output_cols.contains("中文"));
            assert!(output_cols.contains("测试a"));

            Ok(())
        }

        #[test]
        fn test_fold_characters_exact_width() -> CTResult<()> {
            // 测试字符模式的精确宽度控制
            let content = "12345678901234567890"; // 20个ASCII字符
            let temp_file = write_temp_file(content);

            let fold_flags = FoldFlags {
                bytes: false,
                characters: true,
                spaces: false,
                width: 10,
                files: vec![temp_file.path().as_os_str().to_os_string()],
            };

            let mut writer = Vec::new();
            fold(&mut writer, &fold_flags)?;

            let output = String::from_utf8(writer).unwrap();
            let lines: Vec<&str> = output.lines().collect();

            // 应该有2行，每行10个字符
            assert_eq!(lines.len(), 2);
            assert_eq!(lines[0], "1234567890");
            assert_eq!(lines[1], "1234567890");

            Ok(())
        }

        #[test]
        fn test_fold_characters_with_emoji() -> CTResult<()> {
            if fold_locale_is_single_byte() {
                return Ok(());
            }

            // 测试字符模式对emoji的处理
            let content = "😀😃😄😁😆😅"; // 6个emoji字符
            let temp_file = write_temp_file(content);

            let fold_flags = FoldFlags {
                bytes: false,
                characters: true,
                spaces: false,
                width: 3,
                files: vec![temp_file.path().as_os_str().to_os_string()],
            };

            let mut writer = Vec::new();
            fold(&mut writer, &fold_flags)?;

            let output = String::from_utf8(writer).unwrap();
            let lines: Vec<&str> = output.lines().collect();

            // 应该有2行，每行3个emoji字符
            assert_eq!(lines.len(), 2);
            assert_eq!(lines[0].chars().count(), 3);
            assert_eq!(lines[1].chars().count(), 3);

            Ok(())
        }

        #[test]
        fn test_fold_byte_mode_newline_handling() {
            let content = "a\nb";

            assert_eq!(fold_bytes(content, 1, true), "a\nb");
            assert_eq!(fold_bytes(content, 1, false), "a\n\n\nb");
        }

        #[test]
        fn test_fold_byte_mode_ascii_complex() {
            let content = "12345678\n123456789";

            assert_eq!(fold_bytes(content, 8, true), "12345678\n12345678\n9");
            assert_eq!(fold_bytes(content, 8, false), "12345678\n\n1234567\n89");
        }

        #[test]
        fn test_fold_byte_mode_chinese_newline_in_multibyte_locale() {
            if fold_locale_is_single_byte() {
                return;
            }

            let content = "容\n第二行";

            assert_eq!(fold_bytes(content, 8, false), "容\n第\n二行");
        }

        #[test]
        fn test_fold_byte_mode_consecutive_newlines() {
            let content = "a\n\nb";

            assert_eq!(fold_bytes(content, 1, true), "a\n\nb");
            assert_eq!(fold_bytes(content, 1, false), "a\n\n\n\n\nb");
        }

        #[test]
        fn test_fold_byte_mode_rescan_mechanism() {
            let content = "abc\ndef";

            assert_eq!(fold_bytes(content, 2, true), "ab\nc\nde\nf");
            assert_eq!(fold_bytes(content, 2, false), "ab\nc\n\nde\nf");
        }

        #[test]
        fn test_fold_byte_vs_character_mode_difference() -> CTResult<()> {
            if fold_locale_is_single_byte() {
                return Ok(());
            }

            // 测试字节模式与字符模式在中文处理上的差异
            let content = "测试abc";
            let temp_file_bytes = write_temp_file(content);
            let temp_file_chars = write_temp_file(content);

            // 字节模式测试
            let fold_flags_bytes = FoldFlags {
                bytes: true,
                characters: false,
                spaces: false,
                width: 6,
                files: vec![temp_file_bytes.path().as_os_str().to_os_string()],
            };

            let mut writer_bytes = Vec::new();
            fold(&mut writer_bytes, &fold_flags_bytes)?;
            let output_bytes = String::from_utf8(writer_bytes).unwrap();

            // 字符模式测试
            let fold_flags_chars = FoldFlags {
                bytes: false,
                characters: true,
                spaces: false,
                width: 6,
                files: vec![temp_file_chars.path().as_os_str().to_os_string()],
            };

            let mut writer_chars = Vec::new();
            fold(&mut writer_chars, &fold_flags_chars)?;
            let output_chars = String::from_utf8(writer_chars).unwrap();

            // 验证两种模式的不同输出
            assert_eq!(output_bytes, "测试\nabc"); // 字节模式：测(3) + 试(3) = 6字节
            assert_eq!(output_chars, "测试abc"); // 字符模式：5个字符，未超过6的限制

            Ok(())
        }
    }

    #[cfg(test)]
    mod fold_file_tests {
        /*
        空输入：测试输入为空的情况。
        单行输入：测试单行输入，确保它被正确格式化。
        多行输入：测试多行输入，确保每行都被正确处理。
        换行符处理：测试包含换行符的输入，确保它们被正确处理。
        制表符处理：测试包含制表符的输入，确保它们被正确转换。
        退格符处理：测试包含退格符的输入，确保它们被正确处理。
        空格保留：测试 spaces 标志设置为 true 和 false 的情况。
        行宽处理：测试不同宽度的输入，确保行被正确换行
        */
        use std::io::BufReader;

        use super::*;

        #[test]
        fn fold_file_empty_input_no_output() {
            let input = "";
            let reader = BufReader::new(input.as_bytes());
            let mut writer = Vec::new();

            fold_file_generic(&mut writer, reader, false, 10, CountMode::Columns).unwrap();
            let output_str = String::from_utf8(writer).unwrap();
            assert_eq!(output_str, "");
        }

        #[test]
        fn fold_file_single_line_input_no_wrap() {
            let input = "This is a single line.";

            let reader = BufReader::new(input.as_bytes());
            let mut writer = Vec::new();

            fold_file_generic(&mut writer, reader, false, 22, CountMode::Columns).unwrap();
            let output_str = String::from_utf8(writer).unwrap();
            assert_eq!(output_str, "This is a single line.");
        }

        #[test]
        fn fold_file_multiple_lines_wrap_correctly() {
            let input = "This is line one.\nThis is line two.\n";

            let reader = BufReader::new(input.as_bytes());
            let mut writer = Vec::new();

            fold_file_generic(&mut writer, reader, false, 17, CountMode::Columns).unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            assert_eq!(output_str, "This is line one.\nThis is line two.\n");
        }

        #[test]
        fn fold_file_newline_handling_correct_output() {
            let input = "Line one\nLine two\n";
            let reader = BufReader::new(input.as_bytes());
            let mut writer = Vec::new();

            fold_file_generic(&mut writer, reader, false, 10, CountMode::Columns).unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            assert_eq!(output_str, "Line one\nLine two\n");
        }

        #[test]
        fn fold_file_tab_handling_correct_output() {
            let input = "Line\twith\ttabs";

            let reader = BufReader::new(input.as_bytes());
            let mut writer = Vec::new();

            fold_file_generic(&mut writer, reader, false, 20, CountMode::Columns).unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            assert_eq!(output_str, "Line\twith\ttabs");
        }

        #[test]
        fn fold_file_backspace_handling_correct_output() {
            let input = "Line\\bBackspace";

            let reader = BufReader::new(input.as_bytes());
            let mut writer = Vec::new();

            fold_file_generic(&mut writer, reader, false, 10, CountMode::Columns).unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            assert_eq!(output_str, "Line\\bBack\nspace");
        }

        #[test]
        fn fold_file_space_handling_with_spaces() {
            let input = "Line with spaces";

            let reader = BufReader::new(input.as_bytes());
            let mut writer = Vec::new();

            fold_file_generic(&mut writer, reader, true, 10, CountMode::Columns).unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            assert_eq!(output_str, "Line with \nspaces");
        }

        #[test]
        fn fold_file_line_width_handling_correct_wrap() {
            let input = "This line is too long and should be wrapped.";

            let reader = BufReader::new(input.as_bytes());
            let mut writer = Vec::new();

            fold_file_generic(&mut writer, reader, false, 10, CountMode::Columns).unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            assert_eq!(
                output_str,
                "This line \nis too lon\ng and shou\nld be wrap\nped."
            );
        }
    }
}
