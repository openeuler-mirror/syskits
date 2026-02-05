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
use bstr::io::BufReadExt;
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use self::searcher::Searcher;
use clap::{Arg, ArgAction, ArgMatches, Command, builder::ValueParser, crate_version};
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CtSimpleError, FromIo, set_ct_exit_code};
use ctcore::ct_line_ending::CtLineEnding;
use ctcore::ct_ranges::CtRange;
use ctcore::{ct_show_error, ct_show_if_err};
use matcher::{ExactMatcher, Matcher, WhitespaceMatcher};
use std::ffi::OsString;
use std::fs::File;
use std::io::{BufReader, BufWriter, IsTerminal, Read, Write, stdin, stdout};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use sys_locale::get_locale;
use unicode_segmentation::UnicodeSegmentation;

mod matcher;
mod searcher;

#[derive(PartialEq, Debug)]
struct CutOptions<'a> {
    out_delimiter: Option<&'a [u8]>,
    line_ending: CtLineEnding,
    field_opts: Option<CutFieldOptions<'a>>,
    no_split_multibyte: bool,
}

#[derive(PartialEq, Debug)]
enum CutDelimiter<'a> {
    Whitespace,
    Slice(&'a [u8]),
}
#[derive(PartialEq, Debug)]
struct CutFieldOptions<'a> {
    delimiter: CutDelimiter<'a>,
    only_delimited: bool,
}

#[derive(Debug, PartialEq)]
enum CutMode<'a> {
    Bytes(Vec<CtRange>, CutOptions<'a>),
    Characters(Vec<CtRange>, CutOptions<'a>),
    Fields(Vec<CtRange>, CutOptions<'a>),
}

impl Default for CutDelimiter<'_> {
    fn default() -> Self {
        Self::Slice(b"\t")
    }
}

impl<'a> From<&'a OsString> for CutDelimiter<'a> {
    fn from(s: &'a OsString) -> Self {
        Self::Slice(os_string_as_bytes(s).unwrap())
    }
}

mod opt_flags {
    pub const BYTES: &str = "bytes";
    pub const CHARACTERS: &str = "characters";
    pub const DELIMITER: &str = "delimiter";
    pub const FIELDS: &str = "fields";
    pub const ZERO_TERMINATED: &str = "zero-terminated";
    pub const ONLY_DELIMITED: &str = "only-delimited";
    pub const OUTPUT_DELIMITER: &str = "output-delimiter";
    pub const WHITESPACE_DELIMITED: &str = "whitespace-delimited";
    pub const COMPLEMENT: &str = "complement";
    pub const FILE: &str = "file";
    pub const NO_SPLIT_MULTIBYTE: &str = "no-split-multibyte";
}

// 创建一个stdout的writer，如果stdout是终端，则直接返回stdout，否则返回一个包裹了stdout的BufWriter。
//
// - 返回值: 包装了stdout的Box<dyn Write>类型，用于后续的写入操作。
fn cut_stdout_writer() -> Box<dyn Write> {
    if std::io::stdout().is_terminal() {
        Box::new(stdout())
    } else {
        Box::new(BufWriter::new(stdout())) as Box<dyn Write>
    }
}

// 将给定的字符列表转换为CtRange的集合。
// 如果`complement`标志为true，则计算给定范围的补集。
//

// 解析字节/字符位置范围，使用GNU兼容的错误消息
// 对于字符和字节模式，不自动合并相邻范围以保持与GNU cut兼容的output-delimiter行为
fn parse_position_ranges(list: &str, complement: bool) -> Result<Vec<CtRange>, String> {
    let mut ct_ranges = Vec::new();

    for item in list.split(&[',', ' ']) {
        let range_item = std::str::FromStr::from_str(item)
            .map_err(|e| format!("range {} was invalid: {}", item.quote(), e))?;
        ct_ranges.push(range_item);
    }

    // 对于字符和字节模式，仅排序不合并，以保持与GNU cut的output-delimiter兼容性
    ct_ranges.sort();

    let result = if complement {
        Ok(ctcore::ct_ranges::complement(&ct_ranges))
    } else {
        Ok(ct_ranges)
    };

    // 转换错误消息以匹配GNU格式
    result.map_err(|err: String| {
        if err.contains("fields and positions are numbered from 1") {
            "byte/character positions are numbered from 1\nTry 'cut --help' for more information."
                .to_string()
        } else {
            err
        }
    })
}

// 解析字段范围，使用GNU兼容的错误消息
fn parse_field_ranges(list: &str, complement: bool) -> Result<Vec<CtRange>, String> {
    let result = if complement {
        CtRange::from_list(list).map(|r| ctcore::ct_ranges::complement(&r))
    } else {
        CtRange::from_list(list)
    };

    // 转换错误消息以匹配GNU格式
    result.map_err(|err| {
        if err.contains("fields and positions are numbered from 1") {
            "fields are numbered from 1\nTry 'cut --help' for more information.".to_string()
        } else {
            err
        }
    })
}

/**
 * 从给定的读取器中按照指定的范围切割字节，并输出到标准输出。
 *
 * # 参数
 * - `reader`：实现了Read接口的读取器，代表要进行切割的输入数据源。
 * - `ranges`：一个包含切割范围的切片，使用CtRange结构体表示起始和结束位置。
 * - `opts`：包含切割操作选项的结构体引用，如行结束符、输出分隔符等。
 *
 * # 返回值
 * - `CTResult<()>`：成功时返回`Ok(())`，错误时返回包含错误信息的`Err`。
 */
fn cut_bytes<R: Read>(reader: R, ranges: &[CtRange], opts: &CutOptions) -> CTResult<()> {
    // 将行结束符选项转换为字节
    let newline_char = opts.line_ending.into();
    // 使用缓冲读取器包装输入读取器
    let mut buf_in = BufReader::new(reader);
    // 创建一个用于写入标准输出的缓冲写入器
    let mut out = cut_stdout_writer();
    // 获取输出字段分隔符，默认为制表符
    let out_delim = opts.out_delimiter.unwrap_or(b"\t");

    // 遍历输入数据，按指定范围切割并输出
    let result = buf_in.for_byte_record(newline_char, |line| {
        // 智能合并重叠范围，但保持相邻范围分离
        let mut merged_segments = Vec::new();
        let mut current_start = None;
        let mut current_end = None;

        for &CtRange { low, high } in ranges {
            // 范围超出当前行时跳出循环
            if low > line.len() {
                continue;
            }

            let start = low - 1; // 转换为0索引
            let end = high.min(line.len());

            match (current_start, current_end) {
                (None, None) => {
                    current_start = Some(start);
                    current_end = Some(end);
                }
                (Some(cur_start), Some(cur_end)) => {
                    if start < cur_end {
                        // 真正重叠，合并
                        current_end = Some(cur_end.max(end));
                    } else {
                        // 分离或相邻，保存当前段并开始新段
                        merged_segments.push((cur_start, cur_end));
                        current_start = Some(start);
                        current_end = Some(end);
                    }
                }
                _ => unreachable!(),
            }

            // 不在这里输出，等待后面统一输出
        }

        // 添加最后一个段
        if let (Some(start), Some(end)) = (current_start, current_end) {
            merged_segments.push((start, end));
        }

        // 输出各段，在分离的段之间添加分隔符
        for (i, &(start, end)) in merged_segments.iter().enumerate() {
            if i > 0 && opts.out_delimiter.is_some() {
                out.write_all(out_delim)?;
            }

            let mut segment_start = start;
            let mut segment_end = end;

            // 如果启用了no_split_multibyte选项，调整范围以避免分割多字节字符
            if opts.no_split_multibyte {
                // 尝试将字节切片转换为UTF-8字符串来检查字符边界
                if let Ok(line_str) = std::str::from_utf8(line) {
                    // 调整起始位置 - 向后移动到有效的UTF-8字符边界
                    while segment_start > 0 && !line_str.is_char_boundary(segment_start) {
                        segment_start -= 1;
                    }

                    // 调整结束位置 - 向前移动到有效的UTF-8字符边界
                    while segment_end < line.len() && !line_str.is_char_boundary(segment_end) {
                        segment_end += 1;
                    }
                }
                // 如果输入不是有效的UTF-8，就保持原有的字节范围
            }

            // 将指定范围的字节写入输出
            out.write_all(&line[segment_start..segment_end])?;
        }
        // 在每行末尾写入行结束符
        out.write_all(&[newline_char])?;
        Ok(true)
    });

    // 如果处理过程中发生错误，则返回错误信息
    if let Err(e) = result {
        return Err(CtSimpleError::new(1, e.to_string()));
    }

    // 成功完成操作，返回Ok(())
    Ok(())
}

/**
 * 从给定的读取器中读取数据，并根据匹配器和指定的范围切割字段。
 * 只有在指定的范围内且根据是否仅限定于分隔符区间的字段才会被输出。
 *
 * # 参数
 * - `reader`：实现了Read的读取器，用于读取数据。
 * - `matcher`：匹配器，用于识别字段分隔符。
 * - `ranges`：包含要提取字段的起始和结束索引的向量。
 * - `only_delimited`：一个布尔值，指定是否仅输出分隔符限定的字段。
 * - `newline_char`：表示换行符的字节。
 * - `out_delim`：用于输出字段之间分隔符的字节序列。
 *
 * # 返回值
 * - `CTResult<()>`：操作成功返回`Ok(())`，失败则返回包含错误信息的`Err`。
 */
fn cut_fields_explicit_out_delim<R: Read, M: Matcher>(
    reader: R,
    matcher: &M,
    ranges: &[CtRange],
    only_delimited: bool,
    newline_char: u8,
    out_delim: &[u8],
) -> CTResult<()> {
    let mut buffer_in = BufReader::new(reader); // 创建一个缓冲读取器
    let mut out_writer = cut_stdout_writer(); // 准备输出

    // 遍历读取器中的每行数据
    let result = buffer_in.for_byte_record_with_terminator(newline_char, |line| {
        let mut fields_pos = 1;
        let mut low_idx = 0;
        let mut delim_search = Searcher::new(matcher, line).peekable();
        let mut print_delim = false;

        // 处理行中没有分隔符的情况
        if delim_search.peek().is_none() {
            if !only_delimited {
                out_writer.write_all(line)?;
                if line[line.len() - 1] != newline_char {
                    out_writer.write_all(&[newline_char])?;
                }
            }

            return Ok(true);
        }

        // 遍历指定的字段范围，提取并输出字段
        for &CtRange { low, high } in ranges {
            if low - fields_pos > 0 {
                // 跳过当前范围之前不感兴趣的字段
                low_idx = match delim_search.nth(low - fields_pos - 1) {
                    Some((_, last)) => last,
                    None => break,
                };
            }

            for _ in 0..=high - low {
                if print_delim {
                    out_writer.write_all(out_delim)?;
                } else {
                    print_delim = true;
                }

                match delim_search.next() {
                    // 输出字段内容
                    Some((first, last)) => {
                        let segment = &line[low_idx..first];

                        out_writer.write_all(segment)?;

                        low_idx = last;
                        fields_pos = high + 1;
                    }
                    None => {
                        // 处理行的最后一个字段
                        let segment = &line[low_idx..];

                        out_writer.write_all(segment)?;

                        if line[line.len() - 1] == newline_char {
                            return Ok(true);
                        }
                        break;
                    }
                }
            }
        }

        out_writer.write_all(&[newline_char])?;
        Ok(true)
    });

    // 处理遍历过程中的任何错误
    if let Err(e) = result {
        return Err(CtSimpleError::new(1, e.to_string()));
    }

    Ok(())
}

/**
 * 从输入流中读取数据，并根据指定的匹配器和范围切割字段。
 * 输出的字段分隔符与输入相同。
 *
 * # 参数
 * - `reader`：实现了Read接口的输入流，用于读取数据。
 * - `matcher`：匹配器的引用，用于识别字段分隔符。
 * - `ranges`：一个包含切割点位置的数组，用于指定要切割的字段范围。
 * - `only_delimited`：一个布尔值，指示是否只处理被分隔符包围的字段。
 * - `newline_char`：表示换行符的字节，用于处理行结束。
 *
 * # 返回值
 * - `CTResult<()>`：成功时返回`Ok(())`，错误时返回包含错误信息的`Err`。
 */
fn cut_fields_implicit_out_delim<R: Read, M: Matcher>(
    reader: R,
    matcher: &M,
    ranges: &[CtRange],
    only_delimited: bool,
    newline_char: u8,
) -> CTResult<()> {
    // 创建一个缓冲读取器以提高读取效率
    let mut buffer_in = BufReader::new(reader);
    // 准备输出，使用cut_stdout_writer()创建一个写入器
    let mut out = cut_stdout_writer();

    // 循环处理输入流中的每一行
    let result = buffer_in.for_byte_record_with_terminator(newline_char, |line| {
        // 初始化字段位置、上一个分隔符的索引、分隔符搜索器和是否需要打印分隔符的标志
        let mut fields_pos = 1;
        let mut low_idx = 0;
        let mut delim_search = Searcher::new(matcher, line).peekable();
        let mut print_delim = false;

        // 如果当前行没有分隔符，则直接输出整个行，并返回
        if delim_search.peek().is_none() {
            if !only_delimited {
                out.write_all(line)?;
                if line[line.len() - 1] != newline_char {
                    out.write_all(&[newline_char])?;
                }
            }
            // let location = std::panic::Location::caller(); // 获取当前函数调用的位置信息
            // println!("--------------cut_fields_implicit_out_delim--------: {}", location.line()); // 打印行号

            return Ok(true);
        }

        // 遍历指定的切割范围，切割并输出字段
        for &CtRange { low, high } in ranges {
            if low - fields_pos > 0 {
                // 在当前字段与下一个字段之间寻找分隔符，并更新low_idx
                if let Some((first, last)) = delim_search.nth(low - fields_pos - 1) {
                    low_idx = if print_delim { first } else { last }
                } else {
                    break;
                }
            }

            // 在当前字段内寻找下一个分隔符，并输出该字段
            match delim_search.nth(high - low) {
                Some((first, _)) => {
                    let segment = &line[low_idx..first];
                    // let location = std::panic::Location::caller(); // 获取当前函数调用的位置信息
                    // println!("--------------cut_fields_implicit_out_delim--------: {}", location.line()); // 打印行号

                    out.write_all(segment)?;

                    print_delim = true;
                    low_idx = first;
                    fields_pos = high + 1;
                }
                None => {
                    // 如果在字段内找不到下一个分隔符，则输出剩余部分，并结束处理当前行
                    let segment = &line[low_idx..line.len()];
                    // let location = std::panic::Location::caller(); // 获取当前函数调用的位置信息
                    // println!("--------------cut_fields_implicit_out_delim--------: {}, segment{:?}", location.line(),segment); // 打印行号

                    out.write_all(segment)?;

                    if line[line.len() - 1] == newline_char {
                        return Ok(true);
                    }
                    break;
                }
            }
        }

        // println!("--------------cut_fields_implicit_out_delim-----------: {}, low_idx: {}", fields_pos, low_idx);

        // 在行尾添加换行符，准备处理下一行
        out.write_all(&[newline_char])?;
        Ok(true)
    });

    // 处理可能发生的错误
    if let Err(e) = result {
        return Err(CtSimpleError::new(1, e.to_string()));
    }

    Ok(())
}

/**
 * 从给定的读取器中按照指定的字段范围和选项切割字段。
 *
 * @param reader 一个实现了Read接口的读取器，通常是一个文件或标准输入。
 * @param ranges 指定要切割的字段范围，由[CtRange]数组表示。
 * @param opts 包含切割操作的选项，如行结束符、字段选项等。
 * @return 返回一个[CTResult]，成功时为()，失败时为错误信息。
 */
fn cut_fields<R: Read>(reader: R, ranges: &[CtRange], opts: &CutOptions) -> CTResult<()> {
    let newline_char = opts.line_ending.into(); // 将行结束符选项转换为具体的字符
    let field_opts = opts.field_opts.as_ref().unwrap(); // 获取字段选项，此处unwrap安全，因为field_opts在cut_fields调用时总是Some

    // 根据字段分隔符类型进行不同的切割逻辑
    match field_opts.delimiter {
        CutDelimiter::Slice(delim) => {
            // 使用精确匹配器，用于按照指定字符切割
            let matcher = ExactMatcher::new(delim);
            // 根据是否指定了输出字段分隔符，选择不同的切割函数
            match opts.out_delimiter {
                Some(out_delim) => cut_fields_explicit_out_delim(
                    reader,
                    &matcher,
                    ranges,
                    field_opts.only_delimited,
                    newline_char,
                    out_delim,
                ),
                None => cut_fields_implicit_out_delim(
                    reader,
                    &matcher,
                    ranges,
                    field_opts.only_delimited,
                    newline_char,
                ),
            }
        }
        CutDelimiter::Whitespace => {
            // 使用空白符匹配器，用于按照空白字符切割
            let matcher = WhitespaceMatcher {};
            // 由于空白符切割默认没有输出分隔符，这里直接调用相应的函数
            cut_fields_explicit_out_delim(
                reader,
                &matcher,
                ranges,
                field_opts.only_delimited,
                newline_char,
                opts.out_delimiter.unwrap_or(b"\t"), // 若未指定输出分隔符，则默认使用制表符
            )
        }
    }
}

/**
 * 对给定的文件或标准输入进行切割操作。
 *
 * 根据提供的 `CutMode` 模式，对文件内容进行字节、字符或字段的切割，并输出结果。
 * 如果文件名列表为空，则读取标准输入。支持以"-"作为文件名来表示标准输入。
 *
 * @param mut filenames 要处理的文件名的向量。可以包含"-"来表示标准输入。
 * @param mode 切割操作的模式，包含具体的切割规则。
 */
fn cut_files(mut filenames: Vec<String>, mode: &CutMode) {
    let mut stdin_read = false; // 标记是否已从标准输入读取数据

    // 如果没有指定文件名，则默认读取标准输入
    if filenames.is_empty() {
        filenames.push("-".to_owned());
    }

    for filename in &filenames {
        if filename == "-" {
            // 仅处理一次标准输入
            if stdin_read {
                continue;
            }

            // 根据模式对标准输入进行切割
            ct_show_if_err!(match mode {
                CutMode::Bytes(ranges, opts) => cut_bytes(stdin(), ranges, opts),
                CutMode::Characters(ranges, opts) => cut_characters(stdin(), ranges, opts),
                CutMode::Fields(ranges, opts) => cut_fields(stdin(), ranges, opts),
            });

            stdin_read = true; // 标记已处理标准输入
        } else {
            let path = Path::new(&filename[..]);

            // 如果指定的路径是目录，则报错并跳过该文件
            if path.is_dir() {
                ct_show_error!("{}: Is a directory", filename.maybe_quote());
                set_ct_exit_code(1);
                continue;
            }

            // 尝试打开文件，并根据模式对文件内容进行切割
            ct_show_if_err!(
                File::open(path)
                    .map_err_context(|| filename.maybe_quote().to_string())
                    .and_then(|file| {
                        match &mode {
                            CutMode::Bytes(ranges, opts) => cut_bytes(file, ranges, opts),
                            CutMode::Fields(ranges, opts) => cut_fields(file, ranges, opts),
                            CutMode::Characters(ranges, opts) => cut_characters(file, ranges, opts),
                        }
                    })
            );
        }
    }
}

/**
 * 处理分隔符值的辅助函数（可能是非UTF-8的）
 * 该函数仅在UNIX目标上将OsString转换为 &[u8]
 * 在非UNIX（例如Windows）上，如果分隔符值不是UTF-8，则会返回错误
 *
 * @param os_string 一个OsString类型的引用，代表可能是非UTF-8的分隔符值。
 * @return CTResult<&[u8]>。成功时返回一个字节切片的引用，失败时返回错误。
 */
fn os_string_as_bytes(os_string: &OsString) -> CTResult<&[u8]> {
    // 当构建目标为unix时的代码路径，直接将OsString转换为字节切片
    #[cfg(unix)]
    let bytes = os_string.as_bytes();

    // 当构建目标非unix时的代码路径，尝试将OsString转换为UTF-8字符串，
    // 如果失败，则返回一个自定义的错误
    #[cfg(not(unix))]
    let bytes = os_string
        .to_str()
        .ok_or_else(|| {
            // 创建一个CTsageError错误，指示检测到了无效的UTF-8
            ctcore::ct_error::CTsageError::new(
                1,
                "invalid UTF-8 was detected in one or more arguments",
            )
        })?
        .as_bytes();

    // 返回转换后的字节切片
    Ok(bytes)
}

/**
 * 根据命令行匹配结果获取分隔符和输出分隔符。
 *
 * 此函数处理 `-d`/`--delimiter` 和 `--output-delimiter` 选项，允许分隔符既不是 UTF-8 也不是 ASCII，
 * 以与 GNU 行为保持一致。
 *
 * @param matches 命令行参数的匹配结果，来自 Clap 解析器。
 * @param delimiter_is_equal 指示是否因为使用了 `-d=` 而使得分隔符被设置为 `=`。
 * @return CTResult<(CutDelimiter, Option<&[u8]>)> 一个结果类型，包含分隔符和可选的输出分隔符，
 *         分别表示为 `CutDelimiter` 枚举和字节切片的选项。
 */
fn cut_get_delimiters(
    args_match: &ArgMatches,
    delimiter_is_equal: bool,
) -> CTResult<(CutDelimiter, Option<&[u8]>)> {
    // 检查是否指定了以空格分隔的选项
    let is_whitespace_delimited = args_match.get_flag(opt_flags::WHITESPACE_DELIMITED);
    // 尝试获取分隔符选项
    let delim_options = args_match.get_one::<OsString>(opt_flags::DELIMITER);
    // 根据是否指定了分隔符和是否为空白分隔进行处理
    let delim = match delim_options {
        Some(_) if is_whitespace_delimited => {
            // 如果同时指定了 --delimiter 和 -w，返回错误
            return Err(CtSimpleError::new(
                1,
                "invalid input: Only one of --delimiter (-d) or -w option can be specified",
            ));
        }
        Some(os_string) => {
            // 处理分隔符为 "=" 的特殊情况
            if delimiter_is_equal {
                CutDelimiter::Slice(b"=")
            } else if os_string == "''" || os_string.is_empty() {
                // 将空字符串或"''"视为空分隔符
                CutDelimiter::Slice(b"\0")
            } else {
                // 处理分隔符选项，允许单个字符，包括非 UTF-8 和非 ASCII 字符
                let bytes = os_string_as_bytes(os_string)?;
                if os_string.to_str().is_some_and(|s| s.chars().count() > 1)
                    || os_string.to_str().is_none() && bytes.len() > 1
                {
                    // 如果分隔符不是单个字符，返回错误
                    return Err(CtSimpleError::new(
                        1,
                        "the delimiter must be a single character",
                    ));
                } else {
                    CutDelimiter::from(os_string)
                }
            }
        }
        None => match is_whitespace_delimited {
            true => CutDelimiter::Whitespace,
            false => CutDelimiter::default(),
        },
    };
    // 获取输出分隔符，如果有指定
    let out_delim = args_match
        .get_one::<OsString>(opt_flags::OUTPUT_DELIMITER)
        .map(|os_string| {
            // 空字符串或"''"被视为空分隔符
            if os_string.is_empty() || os_string == "''" {
                b"\0"
            } else {
                // 转换输出分隔符为字节序列
                os_string_as_bytes(os_string).unwrap()
            }
        });
    // 返回分隔符和输出分隔符
    Ok((delim, out_delim))
}

/**
 * 构建并返回一个代表`cut`命令的`Command`实例。
 *
 * 该函数配置了命令行解析所需的全部参数，包括剪切模式相关的参数（如按字节、字符或字段剪切），
 * 以及输出选项（如分隔符的指定和反转筛选等）。
 *
 * # 返回值
 *
 * 返回一个配置好的`Command`实例，可用于进一步的设置或执行。
 */
pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("cut.about");
    let usage_description = t!("cut.usage");

    let args = cut_args_init();

    // 创建新的命令实例并设置基本属性，如程序名、版本、用法信息等。
    Command::new(utility_name)
        .version(command_version)
        .override_usage(usage_description)
        .about(application_info)
        .after_help(t!("cut.after_help"))
        .infer_long_args(true)
        // 允许某些参数（如`-d`和`--output-delimiter`）互相覆盖，但对剪切模式相关的参数
        // 使用`ArgAction::Append`来计数和处理，确保只能指定其中之一。
        .args_override_self(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .args(&args)
}

fn cut_args_init() -> Vec<Arg> {
    let args = vec![
        Arg::new("help")
            .short('h')
            .long("help")
            .help(t!("cut.clap.help"))
            .action(ArgAction::Help),
        Arg::new("version")
            .short('V')
            .long("version")
            .help(t!("cut.clap.version"))
            .action(ArgAction::Version),
        Arg::new(opt_flags::BYTES)
            .short('b')
            .long(opt_flags::BYTES)
            .help(t!("cut.clap.bytes"))
            .allow_hyphen_values(true)
            .value_name("LIST")
            .action(ArgAction::Append),
        Arg::new(opt_flags::CHARACTERS)
            .short('c')
            .long(opt_flags::CHARACTERS)
            .help(t!("cut.clap.characters"))
            .allow_hyphen_values(true)
            .value_name("LIST")
            .action(ArgAction::Append),
        Arg::new(opt_flags::DELIMITER)
            .short('d')
            .long(opt_flags::DELIMITER)
            .value_parser(ValueParser::os_string())
            .help("specify the delimiter character that separates fields in the input source. Defaults to Tab.")
            .value_name("DELIM"),
        Arg::new(opt_flags::WHITESPACE_DELIMITED)
            .short('w')
            .help(t!("cut.clap.whitespace_delimited"))
            .value_name("WHITESPACE")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::FIELDS)
            .short('f')
            .long(opt_flags::FIELDS)
            .help(t!("cut.clap.fields"))
            .allow_hyphen_values(true)
            .value_name("LIST")
            .action(ArgAction::Append),

        // 配置反转筛选、仅打印包含分隔符的行、输出分隔符替换等选项。

        Arg::new(opt_flags::COMPLEMENT)
            .long(opt_flags::COMPLEMENT)
            .help(t!("cut.clap.complement"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::ONLY_DELIMITED)
            .short('s')
            .long(opt_flags::ONLY_DELIMITED)
            .help(t!("cut.clap.only_delimited"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::OUTPUT_DELIMITER)
            .long(opt_flags::OUTPUT_DELIMITER)
            .value_parser(ValueParser::os_string())
            .help("in field mode, replace the delimiter in output lines with this option's argument")
            .value_name("NEW_DELIM"),


        // 配置以零终止符为基础的剪切选项。

        Arg::new(opt_flags::ZERO_TERMINATED)
            .short('z')
            .long(opt_flags::ZERO_TERMINATED)
            .help(t!("cut.clap.zero_terminated"))
            .action(ArgAction::SetTrue),


        // 隐藏并添加一个文件参数，用于读取输入。

        Arg::new(opt_flags::FILE)
            .hide(true)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::FilePath),

        // 添加 -n 参数，与 -b 一起使用时不分割多字节字符
        Arg::new(opt_flags::NO_SPLIT_MULTIBYTE)
            .short('n')
            .help("with -b: don't split multibyte characters")
            .action(ArgAction::SetTrue),
    ];
    args
}

#[derive(Default)]
pub struct Cut;
impl Tool for Cut {
    fn name(&self) -> &'static str {
        "cut"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        // 直接调用原有的 cut_main 函数
        cut_main(args.iter().cloned())
    }
}

/**
 * 主执行函数，用于处理文件或标准输入的切割操作。
 *
 * @param args 命令行参数的引用
 * @return `CTResult<()>`，成功执行返回 `Ok(())`，错误时返回包含错误信息的 `Err`。
 */
pub fn cut_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    // 将传入的参数转换为 OsString 的 Vec。
    let args = args.collect::<Vec<OsString>>();

    // 检查是否使用了等号形式的分隔符参数。
    let delimiter_is_equal = args.contains(&OsString::from("-d=")); // 特殊情况处理
    // 使用 clap 库解析命令行参数。
    let args_match = ct_app().try_get_matches_from(args)?;

    // 获取命令行指定的额外选项。
    let is_complement = args_match.get_flag(opt_flags::COMPLEMENT);
    let is_only_delimited = args_match.get_flag(opt_flags::ONLY_DELIMITED);

    // 解析输入的分隔符与输出分隔符。
    let (delimiter, out_delimiter) = cut_get_delimiters(&args_match, delimiter_is_equal)?;

    // 从命令行参数中解析以 0 结尾的标志。
    let line_ending = CtLineEnding::from_zero_flag(args_match.get_flag(opt_flags::ZERO_TERMINATED));

    // 计算参与切割的模式参数（-b, -c, -f）的数量，用于确定切割模式并处理错误情况。
    let mode_args_count = [
        args_match.indices_of(opt_flags::BYTES),
        args_match.indices_of(opt_flags::CHARACTERS),
        args_match.indices_of(opt_flags::FIELDS),
    ]
    .into_iter()
    .map(|indices| indices.unwrap_or_default().count())
    .sum();

    let cut_mode = cut_mode_parse(
        &args_match,
        is_complement,
        is_only_delimited,
        delimiter,
        out_delimiter,
        line_ending,
        mode_args_count,
    );

    let mode_parse = cut_mode_param_parse(&args_match, cut_mode);

    // 获取输入文件列表。
    let files: Vec<String> = args_match
        .get_many::<String>(opt_flags::FILE)
        .unwrap_or_default()
        .cloned()
        .collect();

    cut_files_by_mode(mode_parse, files)
}

fn cut_files_by_mode(cut_mode: Result<CutMode, String>, files: Vec<String>) -> CTResult<()> {
    // 根据解析的切割模式处理文件。
    match cut_mode {
        Ok(mode) => {
            cut_files(files, &mode);
            Ok(())
        }
        Err(e) => Err(CtSimpleError::new(1, e)),
    }
}

// 检查是否有不兼容的参数组合。
fn cut_mode_param_parse<'a>(
    args_match: &'a ArgMatches,
    mode_parse: Result<CutMode<'a>, String>,
) -> Result<CutMode<'a>, String> {
    match mode_parse {
        Err(_) => mode_parse,
        Ok(mode) => match mode {
            CutMode::Bytes(_, _) | CutMode::Characters(_, _)
            if args_match.contains_id(opt_flags::DELIMITER) =>
                {
                    Err("invalid input: The '--delimiter' ('-d') option only usable if printing a sequence of fields".into())
                }
            CutMode::Bytes(_, _) | CutMode::Characters(_, _)
            if args_match.get_flag(opt_flags::WHITESPACE_DELIMITED) =>
                {
                    Err("invalid input: The '-w' option only usable if printing a sequence of fields".into())
                }
            CutMode::Bytes(_, _) | CutMode::Characters(_, _)
            if args_match.get_flag(opt_flags::ONLY_DELIMITED) =>
                {
                    Err("suppressing non-delimited lines makes sense\n\tonly when operating on fields\nTry 'cut --help' for more information.".into())
                }
            _ => Ok(mode),
        },
    }
}

// 根据提供的参数解析切割模式。
fn cut_mode_parse<'a>(
    args_match: &'a ArgMatches,
    is_complement: bool,
    is_only_delimited: bool,
    delimiter: CutDelimiter<'a>,
    out_delimiter: Option<&'a [u8]>,
    line_ending: CtLineEnding,
    mode_args_count: usize,
) -> Result<CutMode<'a>, String> {
    let no_split_multibyte = args_match.get_flag(opt_flags::NO_SPLIT_MULTIBYTE);
    match (
        mode_args_count,
        args_match.get_one::<String>(opt_flags::BYTES),
        args_match.get_one::<String>(opt_flags::CHARACTERS),
        args_match.get_one::<String>(opt_flags::FIELDS),
    ) {
        (1, Some(byte_ranges), None, None) => parse_position_ranges(byte_ranges, is_complement).map(|ranges| {
            CutMode::Bytes(
                ranges,
                CutOptions {
                    out_delimiter,
                    line_ending,
                    field_opts: None,
                    no_split_multibyte,
                },
            )
        }),
        (1, None, Some(char_ranges), None) => parse_position_ranges(char_ranges, is_complement).map(|ranges| {
            CutMode::Characters(
                ranges,
                CutOptions {
                    out_delimiter,
                    line_ending,
                    field_opts: None,
                    no_split_multibyte,
                },
            )
        }),
        (1, None, None, Some(field_ranges)) => parse_field_ranges(field_ranges, is_complement).map(|ranges| {
            CutMode::Fields(
                ranges,
                CutOptions {
                    out_delimiter,
                    line_ending,
                    field_opts: Some(CutFieldOptions {
                        only_delimited: is_only_delimited,
                        delimiter,
                    }),
                    no_split_multibyte,
                },
            )
        }),
        (2.., _, _, _) => Err(
            "invalid usage: expects no more than one of --fields (-f), --chars (-c) or --bytes (-b)".into()
        ),
        _ => Err("invalid usage: expects one of --fields (-f), --chars (-c) or --bytes (-b)".into()),
    }
}

/// 从输入流中按字符位置切割数据
fn cut_characters<R: Read>(reader: R, ranges: &[CtRange], opts: &CutOptions) -> CTResult<()> {
    let mut buf_in = BufReader::new(reader);
    let mut out = cut_stdout_writer();
    let mut input_buffer = [0; 1024 * 31]; // 使用固定大小的缓冲区

    while let Ok(n) = buf_in.read(&mut input_buffer) {
        if n == 0 {
            break;
        }

        let mut position = 0;
        while position < n {
            // 获取当前位置到缓冲区末尾的切片
            let current_slice = &input_buffer[position..n];
            // 查找下一个换行符
            let next_newline = current_slice.iter().position(|&b| b == b'\n');
            // 计算当前行的长度
            let line_length = next_newline.unwrap_or(current_slice.len());
            let line = &current_slice[..line_length];
            // 使用 from_utf8_lossy 处理当前行
            let line_str = String::from_utf8_lossy(line);
            // 使用 graphemes 处理 Unicode 字符
            let graphemes: Vec<&str> = line_str.graphemes(true).collect();
            // 处理每个范围 - 修复output-delimiter逻辑以与GNU coreutils兼容
            // 需要智能合并重叠范围，但保持相邻范围分离
            let mut merged_segments = Vec::new();
            let mut current_start = None;
            let mut current_end = None;

            for &CtRange { low, high } in ranges {
                let start = low.saturating_sub(1);
                let end = high.min(graphemes.len());

                if start >= graphemes.len() {
                    continue;
                }

                match (current_start, current_end) {
                    (None, None) => {
                        // 第一个范围
                        current_start = Some(start);
                        current_end = Some(end);
                    }
                    (Some(cur_start), Some(cur_end)) => {
                        if start < cur_end {
                            // 真正重叠，合并
                            current_end = Some(cur_end.max(end));
                        } else {
                            // 分离或相邻，保存当前段并开始新段
                            merged_segments.push((cur_start, cur_end));
                            current_start = Some(start);
                            current_end = Some(end);
                        }
                    }
                    _ => unreachable!(),
                }
                // 不在这里输出，等待后面统一输出
            }

            // 添加最后一个段
            if let (Some(start), Some(end)) = (current_start, current_end) {
                merged_segments.push((start, end));
            }

            // 输出各段，在分离的段之间添加分隔符
            for (i, &(start, end)) in merged_segments.iter().enumerate() {
                if i > 0 && opts.out_delimiter.is_some() {
                    out.write_all(opts.out_delimiter.unwrap_or(b"\t"))?;
                }

                let selected = graphemes[start..end].join("");
                out.write_all(selected.as_bytes())?;
            }
            // 写入换行符（如果原本有换行符）
            if next_newline.is_some() {
                out.write_all(b"\n")?;
            }
            // 更新位置
            position += line_length + next_newline.map_or(0, |_| 1);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctcore::Tool;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Cut;

        // 测试 name 方法
        assert_eq!(tool.name(), "cut");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("cut"));

        // 测试 execute 方法
        let args = vec![OsString::from("cut"), OsString::from("--help")];
        let result = tool.execute(&args);
        assert!(result.is_err()); // cut命令需要必要参数，所以测试不带参数的情况应该返回错误
    }

    mod tests_cut_app {
        use crate::ct_app;
        use clap::error::ErrorKind;
        use std::fs;
        use std::fs::File;
        use tempfile::Builder;

        use std::io::Write;

        #[test]
        fn test_cut_app_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_cut_app_v() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_cut_app_help() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_cut_app_h() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-h"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_cut_app_b_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "-b", "3-8"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_cut_app_bytes_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "3-8"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_cut_app_c_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "-c", "3-8"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_cut_app_characters_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--characters", "3-8"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_cut_app_d_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "-f", "1", "-d", "o"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_cut_app_delimiter_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "-f",
                "1",
                "--delimiter",
                "o",
            ];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_cut_app_f_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "-f", "3-8"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_cut_app_fields_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "--fields", "3-8"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_cut_app_f_complement_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "-f",
                "1",
                "-d",
                "o",
                "--complement",
            ];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_cut_app_fields_complement_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--fields",
                "1",
                "-d",
                "o",
                "--complement",
            ];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_cut_app_fields_only_delimited_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--fields",
                "1",
                "--only-delimited",
            ];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_cut_app_fields_output_delimiter_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--fields",
                "2",
                "--output-delimiter='R'",
            ];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_cut_app_fields_z_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "-z", "-c", "1"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_cut_app_fields_zero_terminated_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "abc\0def\0ghi.\0\
                   \x0a\x256\x0789.\0\
                   ";

            file.write_all(content.as_bytes()).unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "--zero-terminated",
                "-f",
                "1",
            ];
            let result = ct_app().try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_cut_app_no_split_multibyte_short() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_app_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "测试文本\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = vec![ctcore::ct_util_name(), filename1, "-n", "-b", "1-3"];
            let result = ct_app().try_get_matches_from(args);

            assert!(result.is_ok());
        }
    }
    mod tests_cut_main {
        use crate::cut_main;
        use std::fs;
        use std::fs::File;
        use tempfile::Builder;

        use std::ffi::OsString;
        use std::io::Write;

        #[test]
        fn test_cut_main_version() {
            let args = [ctcore::ct_util_name(), "--version"];

            let result = cut_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_cut_main_v() {
            let args = [ctcore::ct_util_name(), "-V"];
            let result = cut_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_cut_main_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = cut_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_cut_main_h() {
            let args = [ctcore::ct_util_name(), "-h"];
            let result = cut_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_cut_main_b_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), filename1, "-b", "3-8"];
            let result = cut_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_cut_main_bytes_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), filename1, "--bytes", "3-8"];
            let result = cut_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_cut_main_c_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), filename1, "-c", "3-8"];
            let result = cut_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_cut_main_characters_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), filename1, "--characters", "3-8"];
            let result = cut_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_cut_main_d_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), filename1, "-f", "1", "-d", "o"];
            let result = cut_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_cut_main_delimiter_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                filename1,
                "-f",
                "1",
                "--delimiter",
                "o",
            ];
            let result = cut_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_cut_main_f_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), filename1, "-f", "3-8"];
            let result = cut_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_cut_main_fields_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), filename1, "--fields", "3-8"];
            let result = cut_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_cut_main_f_complement_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                filename1,
                "-f",
                "1",
                "-d",
                "o",
                "--complement",
            ];
            let result = cut_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_cut_main_fields_complement_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                filename1,
                "--fields",
                "1",
                "-d",
                "o",
                "--complement",
            ];
            let result = cut_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_cut_main_fields_only_delimited_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                filename1,
                "--fields",
                "1",
                "--only-delimited",
            ];
            let result = cut_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_cut_main_fields_output_delimiter_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                filename1,
                "--fields",
                "2",
                "--output-delimiter='R'",
            ];
            let result = cut_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_cut_main_fields_z_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), filename1, "-z", "-c", "1"];
            let result = cut_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_cut_main_fields_zero_terminated_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "abc\0def\0ghi.\0\
                   \x0a\x256\x0789.\0\
                   ";

            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                filename1,
                "--zero-terminated",
                "-f",
                "1",
            ];
            let result = cut_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_cut_main_no_split_multibyte_short() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "测试文本\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), filename1, "-n", "-b", "1-3"];
            let result = cut_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }
    }

    mod tests_cut_functions {
        use crate::{
            CutDelimiter, ct_app, cut_get_delimiters, cut_mode_param_parse, cut_mode_parse,
            opt_flags,
        };
        use ctcore::ct_line_ending::CtLineEnding;
        use std::fs;
        use std::fs::File;
        use std::io::Write;
        use tempfile::Builder;

        #[test]
        fn test_cut_get_delimiters() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "abc     def     ghi.\n\
                   012     456     789.\n\
                   ";

            file.write_all(content.as_bytes()).unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                filename1,
                "-w",
                "--only-delimited",
                "-f",
                "1",
            ];

            let matches = ct_app().try_get_matches_from(args).unwrap();

            let result = cut_get_delimiters(&matches, false);
            assert!(result.is_ok());
            let (delim, out_delim) = result.unwrap();
            assert_eq!(delim, CutDelimiter::Whitespace);
            assert_eq!(out_delim, None);
        }

        #[test]
        fn test_cut_mode_parse_bytes() {
            let args = vec![
                ctcore::ct_util_name(),
                "file",
                "-w",
                "--only-delimited",
                "-b",
                "1",
            ];

            let matches = ct_app().try_get_matches_from(args).unwrap();

            // 获取命令行指定的额外选项。
            let is_complement = matches.get_flag(opt_flags::COMPLEMENT);
            let is_only_delimited = matches.get_flag(opt_flags::ONLY_DELIMITED);

            // 解析输入的分隔符与输出分隔符。
            let (delimiter, out_delimiter) = cut_get_delimiters(&matches, false).unwrap();

            // 从命令行参数中解析以 0 结尾的标志。
            let line_ending =
                CtLineEnding::from_zero_flag(matches.get_flag(opt_flags::ZERO_TERMINATED));

            // 计算参与切割的模式参数（-b, -c, -f）的数量，用于确定切割模式并处理错误情况。
            let mode_args_count = [
                matches.indices_of(opt_flags::BYTES),
                matches.indices_of(opt_flags::CHARACTERS),
                matches.indices_of(opt_flags::FIELDS),
            ]
            .into_iter()
            .map(|indices| indices.unwrap_or_default().count())
            .sum();

            let mode_parse = cut_mode_parse(
                &matches,
                is_complement,
                is_only_delimited,
                delimiter,
                out_delimiter,
                line_ending,
                mode_args_count,
            );

            assert!(mode_parse.is_ok());
        }

        #[test]
        fn test_cut_mode_parse_characters() {
            let args = vec![
                ctcore::ct_util_name(),
                "file",
                "-w",
                "--only-delimited",
                "-c",
                "1",
            ];

            let matches = ct_app().try_get_matches_from(args).unwrap();

            // 获取命令行指定的额外选项。
            let is_complement = matches.get_flag(opt_flags::COMPLEMENT);
            let is_only_delimited = matches.get_flag(opt_flags::ONLY_DELIMITED);

            // 解析输入的分隔符与输出分隔符。
            let (delimiter, out_delimiter) = cut_get_delimiters(&matches, false).unwrap();

            // 从命令行参数中解析以 0 结尾的标志。
            let line_ending =
                CtLineEnding::from_zero_flag(matches.get_flag(opt_flags::ZERO_TERMINATED));

            // 计算参与切割的模式参数（-b, -c, -f）的数量，用于确定切割模式并处理错误情况。
            let mode_args_count = [
                matches.indices_of(opt_flags::BYTES),
                matches.indices_of(opt_flags::CHARACTERS),
                matches.indices_of(opt_flags::FIELDS),
            ]
            .into_iter()
            .map(|indices| indices.unwrap_or_default().count())
            .sum();

            let mode_parse = cut_mode_parse(
                &matches,
                is_complement,
                is_only_delimited,
                delimiter,
                out_delimiter,
                line_ending,
                mode_args_count,
            );

            assert!(mode_parse.is_ok());
        }

        #[test]
        fn test_cut_mode_parse_fields() {
            let args = vec![
                ctcore::ct_util_name(),
                "file",
                "-w",
                "--only-delimited",
                "-f",
                "1",
            ];

            let matches = ct_app().try_get_matches_from(args).unwrap();

            // 获取命令行指定的额外选项。
            let is_complement = matches.get_flag(opt_flags::COMPLEMENT);
            let is_only_delimited = matches.get_flag(opt_flags::ONLY_DELIMITED);

            // 解析输入的分隔符与输出分隔符。
            let (delimiter, out_delimiter) = cut_get_delimiters(&matches, false).unwrap();

            // 从命令行参数中解析以 0 结尾的标志。
            let line_ending =
                CtLineEnding::from_zero_flag(matches.get_flag(opt_flags::ZERO_TERMINATED));

            // 计算参与切割的模式参数（-b, -c, -f）的数量，用于确定切割模式并处理错误情况。
            let mode_args_count = [
                matches.indices_of(opt_flags::BYTES),
                matches.indices_of(opt_flags::CHARACTERS),
                matches.indices_of(opt_flags::FIELDS),
            ]
            .into_iter()
            .map(|indices| indices.unwrap_or_default().count())
            .sum();

            let mode_parse = cut_mode_parse(
                &matches,
                is_complement,
                is_only_delimited,
                delimiter,
                out_delimiter,
                line_ending,
                mode_args_count,
            );

            assert!(mode_parse.is_ok());
        }

        #[test]
        fn test_cut_mode_param_parse_fields() {
            let args = vec![
                ctcore::ct_util_name(),
                "file",
                "-w",
                "--only-delimited",
                "-f",
                "1",
            ];

            let matches = ct_app().try_get_matches_from(args).unwrap();

            // 获取命令行指定的额外选项。
            let is_complement = matches.get_flag(opt_flags::COMPLEMENT);
            let is_only_delimited = matches.get_flag(opt_flags::ONLY_DELIMITED);

            // 解析输入的分隔符与输出分隔符。
            let (delimiter, out_delimiter) = cut_get_delimiters(&matches, false).unwrap();

            // 从命令行参数中解析以 0 结尾的标志。
            let line_ending =
                CtLineEnding::from_zero_flag(matches.get_flag(opt_flags::ZERO_TERMINATED));

            // 计算参与切割的模式参数（-b, -c, -f）的数量，用于确定切割模式并处理错误情况。
            let mode_args_count = [
                matches.indices_of(opt_flags::BYTES),
                matches.indices_of(opt_flags::CHARACTERS),
                matches.indices_of(opt_flags::FIELDS),
            ]
            .into_iter()
            .map(|indices| indices.unwrap_or_default().count())
            .sum();

            let mode_parse = cut_mode_parse(
                &matches,
                is_complement,
                is_only_delimited,
                delimiter,
                out_delimiter,
                line_ending,
                mode_args_count,
            );

            let mode_parse = cut_mode_param_parse(&matches, mode_parse);

            assert!(mode_parse.is_ok());
        }

        #[test]
        fn test_basic_functionality() {
            // 简单测试cut功能的基本行为
            use ctcore::Tool;

            let cut = super::Cut;
            assert_eq!(cut.name(), "cut");

            // 测试一个简单的命令行解析
            let args = vec![
                std::ffi::OsString::from("cut"),
                std::ffi::OsString::from("-c1-5"),
            ];
            // 如果命令解析成功就通过测试
            let cmd = cut.command();
            let matches = cmd.try_get_matches_from(&args);
            assert!(matches.is_ok());
        }

        #[test]
        fn test_parse_position_ranges_basic() {
            // 测试基本的范围解析功能
            let ranges = super::parse_position_ranges("1-2,3-4", false).unwrap();
            assert_eq!(ranges.len(), 2);
            assert_eq!(ranges[0].low, 1);
            assert_eq!(ranges[0].high, 2);
            assert_eq!(ranges[1].low, 3);
            assert_eq!(ranges[1].high, 4);
        }

        #[test]
        fn test_parse_position_ranges_overlapping() {
            // 测试重叠范围解析 - 应该保持原始范围不合并
            let ranges = super::parse_position_ranges("1-3,2-4", false).unwrap();
            assert_eq!(ranges.len(), 2);
            assert_eq!(ranges[0].low, 1);
            assert_eq!(ranges[0].high, 3);
            assert_eq!(ranges[1].low, 2);
            assert_eq!(ranges[1].high, 4);
        }

        #[test]
        fn test_parse_position_ranges_sorting() {
            // 测试范围排序
            let ranges = super::parse_position_ranges("3-4,1-2", false).unwrap();
            assert_eq!(ranges.len(), 2);
            assert_eq!(ranges[0].low, 1);
            assert_eq!(ranges[0].high, 2);
            assert_eq!(ranges[1].low, 3);
            assert_eq!(ranges[1].high, 4);
        }

        #[test]
        fn test_parse_position_ranges_complement() {
            // 测试complement功能
            let ranges = super::parse_position_ranges("2-3", true).unwrap();
            assert!(!ranges.is_empty());
            // complement会生成补集范围，第一个范围应该是1-1
            assert_eq!(ranges[0].low, 1);
            assert_eq!(ranges[0].high, 1);
        }

        #[test]
        fn test_parse_position_ranges_invalid() {
            // 测试无效范围 - 位置从1开始编号
            let result = super::parse_position_ranges("0-2", false);
            assert!(result.is_err());
            let error_msg = result.unwrap_err();
            assert!(
                error_msg.contains("fields and positions are numbered from 1")
                    || error_msg.contains("byte/character positions are numbered from 1")
            );
        }

        #[test]
        fn test_parse_position_ranges_single_position() {
            // 测试单个位置
            let ranges = super::parse_position_ranges("5", false).unwrap();
            assert_eq!(ranges.len(), 1);
            assert_eq!(ranges[0].low, 5);
            assert_eq!(ranges[0].high, 5);
        }

        #[test]
        fn test_parse_position_ranges_unbounded() {
            // 测试无上界范围
            let ranges = super::parse_position_ranges("3-", false).unwrap();
            assert_eq!(ranges.len(), 1);
            assert_eq!(ranges[0].low, 3);
            // 无上界范围的高值可能是usize::MAX或usize::MAX-1，取决于实现
            assert!(ranges[0].high >= usize::MAX - 1);
        }
    }
}
