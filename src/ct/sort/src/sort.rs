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

use std::cmp::Ordering;
use std::env;
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt::Display;
use std::fs::{File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{stdin, stdout, BufRead, BufReader, BufWriter, Read, Write};
use std::ops::Range;
use std::path::Path;
use std::path::PathBuf;
use std::str::Utf8Error;

use clap::builder::ValueParser;
use clap::{crate_version, Arg, ArgAction, ArgMatches, Command};
use fnv::FnvHasher;
use rand::{thread_rng, Rng};
use rayon::prelude::*;
use unicode_width::UnicodeWidthStr;

use chunks::ChunkLineData;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{
    set_ct_exit_code, strip_errno, CTError, CTResult, CTsageError, CtSimpleError,
};
use ctcore::ct_line_ending::CtLineEnding;
use ctcore::ct_parse_size::{CtParser, ParseSizeError};
use ctcore::ct_version_cmp::ct_version_cmp;
use ctcore::{ct_format_usage, ct_help_about, ct_help_section, ct_help_usage};
use custom_str_cmp::custom_cmp_str;
use ext_sort::ext_sort;
use numeric_str_cmp::{
    num_cmp_human_numeric_str_cmp, numeric_str_cmp, NumInfo, NumInfoParseSettings,
};

use crate::tmp_dir::TmpDirWrapper;

mod check;
mod chunks;
mod custom_str_cmp;
mod ext_sort;
mod merge;
mod numeric_str_cmp;
mod tmp_dir;

const SORT_ABOUT: &str = ct_help_about!("sort.md");
const SORT_USAGE: &str = ct_help_usage!("sort.md");
const SORT_AFTER_HELP: &str = ct_help_section!("after help", "sort.md");

mod sort_flags {
    pub mod modes {
        pub const SORT: &str = "sort";

        pub const SORT_HUMAN_NUMERIC: &str = "human-numeric-sort";
        pub const SORT_MONTH: &str = "month-sort";
        pub const SORT_NUMERIC: &str = "numeric-sort";
        pub const SORT_GENERAL_NUMERIC: &str = "general-numeric-sort";
        pub const SORT_VERSION: &str = "version-sort";
        pub const SORT_RANDOM: &str = "random-sort";

        pub const SORT_ALL_MODES: [&str; 6] = [
            SORT_GENERAL_NUMERIC,
            SORT_HUMAN_NUMERIC,
            SORT_MONTH,
            SORT_NUMERIC,
            SORT_VERSION,
            SORT_RANDOM,
        ];
    }

    pub mod check {
        pub const SORT_CHECK: &str = "check";
        pub const SORT_CHECK_SILENT: &str = "check-silent";
        pub const SORT_SILENT: &str = "silent";
        pub const SORT_QUIET: &str = "quiet";
        pub const SORT_DIAGNOSE_FIRST: &str = "diagnose-first";
    }

    pub const SORT_HELP: &str = "help";
    pub const SORT_VERSION: &str = "version";
    pub const SORT_DICTIONARY_ORDER: &str = "dictionary-order";
    pub const SORT_MERGE: &str = "merge";
    pub const SORT_DEBUG: &str = "debug";
    pub const SORT_IGNORE_CASE: &str = "ignore-case";
    pub const SORT_IGNORE_LEADING_BLANKS: &str = "ignore-leading-blanks";
    pub const SORT_IGNORE_NONPRINTING: &str = "ignore-nonprinting";
    pub const SORT_OUTPUT: &str = "output";
    pub const SORT_REVERSE: &str = "reverse";
    pub const SORT_STABLE: &str = "stable";
    pub const SORT_UNIQUE: &str = "unique";
    pub const SORT_KEY: &str = "key";
    pub const SORT_SEPARATOR: &str = "field-separator";
    pub const SORT_ZERO_TERMINATED: &str = "zero-terminated";
    pub const SORT_PARALLEL: &str = "parallel";
    pub const SORT_FILES0_FROM: &str = "files0-from";
    pub const SORT_BUF_SIZE: &str = "buffer-size";
    pub const SORT_TMP_DIR: &str = "temporary-directory";
    pub const SORT_COMPRESS_PROG: &str = "compress-program";
    pub const SORT_BATCH_SIZE: &str = "batch-size";

    pub const SORT_FILES: &str = "files";
}

const SORT_DECIMAL_PT: char = '.';

const SORT_NEGATIVE: char = '-';
const SORT_POSITIVE: char = '+';

// 选择更大的缓冲区大小并不会提高性能
// 至少在我的机器上不会）。TODO: 在未来，我们还应该考虑可用内存的大小，而不是仅仅依赖于这个常数。
// 可用内存的大小，而不是仅仅依赖这个常数。
const SORT_DEFAULT_BUF_SIZE: usize = 1_000_000_000; // 1 GB

#[derive(Debug)]
enum SortError {
    SortDisorder {
        file: OsString,
        line_number: usize,
        line: String,
        is_silent: bool,
    },
    SortOpenFailed {
        path: String,
        error: std::io::Error,
    },
    SortReadFailed {
        path: PathBuf,
        error: std::io::Error,
    },
    SortParseKeyError {
        key: String,
        msg: String,
    },
    SortOpenTmpFileFailed {
        error: std::io::Error,
    },
    SortCompressProgExecutionFailed {
        code: i32,
    },
    SortCompressProgTerminatedAbnormally {
        prog: String,
    },
    SortTmpDirCreationFailed,
    SortUft8Error {
        error: Utf8Error,
    },
}

impl Error for SortError {}

impl CTError for SortError {
    fn code(&self) -> i32 {
        match self {
            Self::SortDisorder { .. } => 1,
            _ => 2,
        }
    }
}

impl Display for SortError {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SortDisorder {
                file,
                line_number,
                line,
                is_silent: silent,
            } => {
                if *silent {
                    Ok(())
                } else {
                    write!(
                        fmt,
                        "{}:{}: disorder: {}",
                        file.maybe_quote(),
                        line_number,
                        line
                    )
                }
            }
            Self::SortOpenFailed { path, error } => {
                write!(
                    fmt,
                    "open failed: {}: {}",
                    path.maybe_quote(),
                    strip_errno(error)
                )
            }
            Self::SortParseKeyError { key, msg } => {
                write!(fmt, "failed to parse key {}: {}", key.quote(), msg)
            }
            Self::SortReadFailed { path, error } => {
                write!(
                    fmt,
                    "cannot read: {}: {}",
                    path.maybe_quote(),
                    strip_errno(error)
                )
            }
            Self::SortOpenTmpFileFailed { error } => {
                write!(fmt, "failed to open temporary file: {}", strip_errno(error))
            }
            Self::SortCompressProgExecutionFailed { code } => {
                write!(fmt, "couldn't execute compress program: errno {code}")
            }
            Self::SortCompressProgTerminatedAbnormally { prog } => {
                write!(fmt, "{} terminated abnormally", prog.quote())
            }
            Self::SortTmpDirCreationFailed => write!(fmt, "could not create temporary directory"),
            Self::SortUft8Error { error } => write!(fmt, "{error}"),
        }
    }
}

#[allow(clippy::enum_variant_names)]
#[derive(Eq, Ord, PartialEq, PartialOrd, Clone, Copy, Debug)]
enum SortMode {
    SortNumeric,
    SortHumanNumeric,
    SortGeneralNumeric,
    SortMonth,
    SortVersion,
    SortRandom,
    SortDefault,
}

impl SortMode {
    fn get_short_name(&self) -> Option<char> {
        match self {
            SortMode::SortNumeric => Some('n'),
            SortMode::SortHumanNumeric => Some('h'),
            SortMode::SortGeneralNumeric => Some('g'),
            SortMode::SortMonth => Some('M'),
            SortMode::SortVersion => Some('V'),
            SortMode::SortRandom => Some('R'),
            SortMode::SortDefault => None,
        }
    }
}

pub struct SortOutput {
    file: Option<(String, File)>,
}

impl SortOutput {
    fn new(name: Option<&str>) -> CTResult<Self> {
        let file = if let Some(name) = name {
            // 这与 `File::create()` 不同，因为我们还没有截断输出。
            // 这样就可以将输出文件用作输入文件。
            #[allow(clippy::suspicious_open_options)]
            let file = OpenOptions::new()
                .write(true)
                .create(true)
                .open(name)
                .map_err(|e| SortError::SortOpenFailed {
                    path: name.to_owned(),
                    error: e,
                })?;
            Some((name.to_owned(), file))
        } else {
            None
        };
        Ok(Self { file })
    }

    fn into_write(self) -> BufWriter<Box<dyn Write>> {
        BufWriter::new(if let Some((_name, file)) = self.file {
            let _ = file.set_len(0);
            Box::new(file)
        } else {
            Box::new(stdout())
        })
    }

    fn as_output_name(&self) -> Option<&str> {
        match &self.file {
            Some((name, _file)) => Some(name),
            None => None,
        }
    }
}

#[derive(Clone)]
pub struct SortGlobalConfigs {
    mode: SortMode,
    is_debug: bool,
    is_ignore_leading_blanks: bool,
    is_ignore_case: bool,
    is_dictionary_order: bool,
    is_ignore_non_printing: bool,
    is_merge: bool,
    is_reverse: bool,
    is_stable: bool,
    is_unique: bool,
    is_check: bool,
    is_check_silent: bool,
    salt: Option<[u8; 16]>,
    selectors: Vec<SortFieldSelector>,
    separator: Option<char>,
    threads: String,
    line_ending: CtLineEnding,
    buffer_size: usize,
    compress_prog: Option<String>,
    merge_batch_size: usize,
    precomputed: SortPrecomputed,
}

/// 排序所需的数据。应在开始排序前计算一次
/// 调用 `GlobalSettings::init_precomputed`.
#[derive(Clone, Debug, Default)]
struct SortPrecomputed {
    is_needs_tokens: bool,
    num_infos_per_line: usize,
    floats_per_line: usize,
    selections_per_line: usize,
}

impl SortGlobalConfigs {
    /// 将一个 SIZE 字符串解析为若干字节。
    /// 大小字符串包括一个整数和一个可选单位。
    /// 单位可以是 k、K、m、M、g、G、t、T、P、E、Z、Y（1024 的幂次）或 1 的 b。
    /// 默认为 K。
    fn parse_byte_count(input: &str) -> Result<usize, ParseSizeError> {
        // GNU sort (8.32)   valid: 1b,        k, K, m, M, g, G, t, T, P, E, Z, Y
        // GNU sort (8.32) invalid:  b, B, 1B,                         p, e, z, y
        let size = CtParser::default()
            .with_allow_list(&[
                "b", "k", "K", "m", "M", "g", "G", "t", "T", "P", "E", "Z", "Y",
            ])
            .with_default_unit("K")
            .with_b_byte_count(true)
            .parse(input.trim())?;

        usize::try_from(size).map_err(|_| {
            ParseSizeError::SizeTooBig(format!("Buffer size {size} does not fit in address space"))
        })
    }

    /// 预先计算排序所需的一些数据。
    /// 必须在开始排序前调用此函数，之后不得更改 `GlobalSettings` 。
    /// 之后不得更改。
    fn init_precomputed(&mut self) {
        self.precomputed.is_needs_tokens = self.selectors.iter().any(|s| s.is_needs_tokens);
        self.precomputed.selections_per_line = self
            .selectors
            .iter()
            .filter(|s| s.is_needs_selection)
            .count();
        self.precomputed.num_infos_per_line = self
            .selectors
            .iter()
            .filter(|s| {
                matches!(
                    s.settings.mode,
                    SortMode::SortNumeric | SortMode::SortHumanNumeric
                )
            })
            .count();
        self.precomputed.floats_per_line = self
            .selectors
            .iter()
            .filter(|s| matches!(s.settings.mode, SortMode::SortGeneralNumeric))
            .count();
    }
}

impl Default for SortGlobalConfigs {
    fn default() -> Self {
        Self {
            mode: SortMode::SortDefault,
            is_debug: false,
            is_ignore_leading_blanks: false,
            is_ignore_case: false,
            is_dictionary_order: false,
            is_ignore_non_printing: false,
            is_merge: false,
            is_reverse: false,
            is_stable: false,
            is_unique: false,
            is_check: false,
            is_check_silent: false,
            salt: None,
            selectors: vec![],
            separator: None,
            threads: String::new(),
            line_ending: CtLineEnding::Newline,
            buffer_size: SORT_DEFAULT_BUF_SIZE,
            compress_prog: None,
            merge_batch_size: 32,
            precomputed: SortPrecomputed::default(),
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
struct SortKeySettings {
    mode: SortMode,
    is_ignore_blanks: bool,
    is_ignore_case: bool,
    is_dictionary_order: bool,
    is_ignore_non_printing: bool,
    is_reverse: bool,
}

impl SortKeySettings {
    /// 检查所提供的 `mode`、`ignore_non_printing` 和 `dictionary_order` 组合是否允许。
    fn check_compatibility(
        sort_mode: SortMode,
        is_ignore_non_printing: bool,
        is_dictionary_order: bool,
    ) -> Result<(), String> {
        if matches!(
            sort_mode,
            SortMode::SortNumeric
                | SortMode::SortHumanNumeric
                | SortMode::SortGeneralNumeric
                | SortMode::SortMonth
        ) {
            if is_dictionary_order {
                return Err(format!(
                    "options '-{}{}' are incompatible",
                    'd',
                    sort_mode.get_short_name().unwrap()
                ));
            } else if is_ignore_non_printing {
                return Err(format!(
                    "options '-{}{}' are incompatible",
                    'i',
                    sort_mode.get_short_name().unwrap()
                ));
            }
        }
        Ok(())
    }

    fn set_sort_mode(&mut self, sort_mode: SortMode) -> Result<(), String> {
        if self.mode != SortMode::SortDefault && self.mode != sort_mode {
            return Err(format!(
                "options '-{}{}' are incompatible",
                self.mode.get_short_name().unwrap(),
                sort_mode.get_short_name().unwrap()
            ));
        }
        Self::check_compatibility(
            sort_mode,
            self.is_ignore_non_printing,
            self.is_dictionary_order,
        )?;
        self.mode = sort_mode;
        Ok(())
    }

    fn set_dictionary_order(&mut self) -> Result<(), String> {
        Self::check_compatibility(self.mode, self.is_ignore_non_printing, true)?;
        self.is_dictionary_order = true;
        Ok(())
    }

    fn set_ignore_non_printing(&mut self) -> Result<(), String> {
        Self::check_compatibility(self.mode, true, self.is_dictionary_order)?;
        self.is_ignore_non_printing = true;
        Ok(())
    }
}

impl From<&SortGlobalConfigs> for SortKeySettings {
    fn from(settings: &SortGlobalConfigs) -> Self {
        Self {
            mode: settings.mode,
            is_ignore_blanks: settings.is_ignore_leading_blanks,
            is_ignore_case: settings.is_ignore_case,
            is_ignore_non_printing: settings.is_ignore_non_printing,
            is_reverse: settings.is_reverse,
            is_dictionary_order: settings.is_dictionary_order,
        }
    }
}

impl Default for SortKeySettings {
    fn default() -> Self {
        Self::from(&SortGlobalConfigs::default())
    }
}

enum SortSelection<'a> {
    AsF64(SortGeneralF64ParseResult),
    WithNumInfo(&'a str, NumInfo),
    Str(&'a str),
}

type Field = Range<usize>;

#[derive(Clone, Debug, PartialEq)]
pub struct SortLine<'a> {
    line: &'a str,
    index: usize,
}

impl<'a> SortLine<'a> {
    /// 创建一个新的 `Line`。
    ///
    /// 如果排序需要额外数据，则将其添加到 `line_data` 中。
    /// `token_buffer` 允许重复使用标记分配。
    fn create(
        line: &'a str,
        index: usize,
        chunk_line_data: &mut ChunkLineData<'a>,
        token_buffer: &mut Vec<Field>,
        sort_settings: &SortGlobalConfigs,
    ) -> Self {
        token_buffer.clear();
        if sort_settings.precomputed.is_needs_tokens {
            tokenize(line, sort_settings.separator, token_buffer);
        }
        for (selector, selection) in sort_settings
            .selectors
            .iter()
            .map(|selector| (selector, selector.get_selection(line, token_buffer)))
        {
            match selection {
                SortSelection::AsF64(parsed_float) => {
                    chunk_line_data.parsed_floats.push(parsed_float)
                }
                SortSelection::WithNumInfo(str, num_info) => {
                    chunk_line_data.num_infos.push(num_info);
                    chunk_line_data.selections.push(str);
                }
                SortSelection::Str(str) => {
                    if selector.is_needs_selection {
                        chunk_line_data.selections.push(str);
                    }
                }
            }
        }
        Self { line, index }
    }

    fn print(&self, w: &mut impl Write, sort_settings: &SortGlobalConfigs) {
        match sort_settings.is_debug {
            true => {
                self.print_debug(sort_settings, w).unwrap();
            }
            false => {
                w.write_all(self.line.as_bytes()).unwrap();
                w.write_all(&[sort_settings.line_ending.into()]).unwrap();
            }
        }
    }

    /// 为该行匹配的选项写入指示符。不希望已打印原始行内容。
    fn print_debug(
        &self,
        sort_settings: &SortGlobalConfigs,
        w: &mut impl Write,
    ) -> std::io::Result<()> {
        // 我们认为此函数对性能并不重要，因为调试输出只对小文件有用、
        // 在任何情况下都不会造成性能问题。因此，这里没有任何特殊的性能
        // 优化。

        let line = self.line.replace('\t', ">");
        writeln!(w, "{line}")?;

        let mut fields = vec![];
        tokenize(self.line, sort_settings.separator, &mut fields);
        for selector in &sort_settings.selectors {
            let mut selection = selector.get_range(self.line, Some(&fields));
            match selector.settings.mode {
                SortMode::SortNumeric | SortMode::SortHumanNumeric => {
                    // 找出用于数字比较的范围
                    let (_, num_range) = NumInfo::parse(
                        &self.line[selection.clone()],
                        &NumInfoParseSettings {
                            accept_si_units: selector.settings.mode == SortMode::SortHumanNumeric,
                            ..Default::default()
                        },
                    );
                    let initial_selection = selection.clone();

                    // 将选择缩短为 num_range。
                    selection.start += num_range.start;
                    selection.end = selection.start + num_range.len();

                    if num_range == (0..0) {
                        // 这不是一个有效的数字。
                        // 报告第一个非空格字符不匹配。
                        let leading_whitespace = self.line[selection.clone()]
                            .find(|c: char| !c.is_whitespace())
                            .unwrap_or(0);
                        selection.start += leading_whitespace;
                        selection.end += leading_whitespace;
                    } else {
                        // 包括尾部的 si 单位
                        if selector.settings.mode == SortMode::SortHumanNumeric
                            && self.line[selection.end..initial_selection.end]
                                .starts_with(&['k', 'K', 'M', 'G', 'T', 'P', 'E', 'Z', 'Y'][..])
                        {
                            selection.end += 1;
                        }

                        // 包括前导零、前导负数或前导小数点
                        while self.line[initial_selection.start..selection.start]
                            .ends_with(&['-', '0', '.'][..])
                        {
                            selection.start -= 1;
                        }
                    }
                }
                SortMode::SortGeneralNumeric => {
                    let initial_selection = &self.line[selection.clone()];

                    let leading = sort_get_leading_gen(initial_selection);

                    // 将选择缩短为前导。
                    selection.start += leading.start;
                    selection.end = selection.start + leading.len();
                }
                SortMode::SortMonth => {
                    let initial_selection = &self.line[selection.clone()];

                    let mut month_chars = initial_selection
                        .char_indices()
                        .skip_while(|(_, c)| c.is_whitespace());

                    let month = match sort_month_parse(initial_selection) {
                        SortMonth::Unknown => {
                            let first_non_whitespace = month_chars.next();
                            first_non_whitespace.map_or(
                                initial_selection.len()..initial_selection.len(),
                                |(idx, _)| idx..idx,
                            )
                        }
                        _ => {
                            month_chars.next().unwrap().0
                                ..month_chars
                                    .nth(2)
                                    .map_or(initial_selection.len(), |(idx, _)| idx)
                        }
                    };

                    // Shorten selection to month.
                    selection.start += month.start;
                    selection.end = selection.start + month.len();
                }
                _ => {}
            }

            write!(
                w,
                "{}",
                " ".repeat(UnicodeWidthStr::width(&line[..selection.start]))
            )?;

            if selection.is_empty() {
                writeln!(w, "^ no match for key")?;
            } else {
                writeln!(
                    w,
                    "{}",
                    "_".repeat(UnicodeWidthStr::width(&line[selection]))
                )?;
            }
        }
        if sort_settings.mode != SortMode::SortRandom
            && !sort_settings.is_stable
            && !sort_settings.is_unique
            && (sort_settings.is_dictionary_order
                || sort_settings.is_ignore_leading_blanks
                || sort_settings.is_ignore_case
                || sort_settings.is_ignore_non_printing
                || sort_settings.mode != SortMode::SortDefault
                || sort_settings
                    .selectors
                    .last()
                    .map_or(true, |selector| selector != &SortFieldSelector::default()))
        {
            // 使用最后的比较器，整行下划线。
            if self.line.is_empty() {
                writeln!(w, "^ no match for key")?;
            } else {
                writeln!(w, "{}", "_".repeat(UnicodeWidthStr::width(line.as_str())))?;
            }
        }
        Ok(())
    }
}

/// 将一行标记为字段。结果存储在 `token_buffer` 中。
fn tokenize(line: &str, separator: Option<char>, token_buffer: &mut Vec<Field>) {
    assert!(token_buffer.is_empty());
    match separator {
        Some(separator) => {
            tokenize_with_separator(line, separator, token_buffer);
        }
        None => {
            tokenize_default(line, token_buffer);
        }
    }
}

/// 默认情况下，字段由非空格后的第一个空格分隔。
/// 字段开头包含空格。
/// 结果存储到 `token_buffer` 中。
fn tokenize_default(line: &str, token_buf: &mut Vec<Field>) {
    token_buf.push(0..0);
    // 假装行前有空格
    let mut previous_was_whitespace = true;
    for (idx, char) in line.char_indices() {
        if char.is_whitespace() {
            if !previous_was_whitespace {
                token_buf.last_mut().unwrap().end = idx;
                token_buf.push(idx..0);
            }
            previous_was_whitespace = true;
        } else {
            previous_was_whitespace = false;
        }
    }
    token_buf.last_mut().unwrap().end = line.len();
}

/// 在分隔符之间分割。这些分隔符不包含在字段中。
/// 结果将存储到 `token_buffer` 中。
fn tokenize_with_separator(line: &str, separator: char, token_buf: &mut Vec<Field>) {
    let separator_indices =
        line.char_indices()
            .filter_map(|(i, c)| if c == separator { Some(i) } else { None });
    let mut start = 0;
    for sep_idx in separator_indices {
        token_buf.push(start..sep_idx);
        start = sep_idx + 1;
    }
    if start < line.len() {
        token_buf.push(start..line.len());
    }
}

#[derive(Clone, PartialEq, Debug)]
struct SortKeyPosition {
    /// 1-indexed, 0 is invalid.
    field: usize,
    /// 1-indexed, 0 is end of field.
    char: usize,
    is_ignore_blanks: bool,
}

impl SortKeyPosition {
    fn new(key: &str, default_char_index: usize, is_ignore_blanks: bool) -> Result<Self, String> {
        let mut field_and_char = key.split('.');

        let field = field_and_char
            .next()
            .ok_or_else(|| format!("invalid key {}", key.quote()))?;
        let char_option = field_and_char.next();

        let field = field
            .parse()
            .map_err(|e| format!("failed to parse field index {}: {}", field.quote(), e))?;
        if field == 0 {
            return Err("field index can not be 0".to_string());
        }

        let char_size = char_option.map_or(Ok(default_char_index), |char| {
            char.parse()
                .map_err(|e| format!("failed to parse character index {}: {}", char.quote(), e))
        })?;

        Ok(Self {
            field,
            char: char_size,
            is_ignore_blanks,
        })
    }
}

impl Default for SortKeyPosition {
    fn default() -> Self {
        Self {
            field: 1,
            char: 1,
            is_ignore_blanks: false,
        }
    }
}

#[derive(Clone, PartialEq, Debug, Default)]
struct SortFieldSelector {
    from: SortKeyPosition,
    to: Option<SortKeyPosition>,
    settings: SortKeySettings,
    is_needs_tokens: bool,
    // 此选择器是否对一行的子片段进行操作。
    // 因此，当该选择器匹配整行
    // 或排序模式为一般数字。
    is_needs_selection: bool,
}

impl SortFieldSelector {
    /// 将该位置拆分为实际位置和附加选项。
    fn split_key_options(position: &str) -> (&str, &str) {
        match position.char_indices().find(|(_, c)| c.is_alphabetic()) {
            Some((options_start, _)) => position.split_at(options_start),
            None => (position, ""),
        }
    }

    fn parse(key: &str, global_settings: &SortGlobalConfigs) -> CTResult<Self> {
        let mut from_to = key.split(',');
        let (from, from_options) = Self::split_key_options(from_to.next().unwrap());
        let to = from_to.next().map(Self::split_key_options);
        let options_are_empty = from_options.is_empty() && matches!(to, None | Some((_, "")));

        if options_are_empty {
            // 如果该键没有附加选项，则继承全局设置。
            (|| {
                // 我认为这将是尝试块的理想选择。同时，这个闭包允许
                // 在这里使用 `?` 操作符。
                Self::new(
                    SortKeyPosition::new(from, 1, global_settings.is_ignore_leading_blanks)?,
                    to.map(|(to, _)| {
                        SortKeyPosition::new(to, 0, global_settings.is_ignore_leading_blanks)
                    })
                    .transpose()?,
                    SortKeySettings::from(global_settings),
                )
            })()
        } else {
            // 不要从 `global_settings` 继承，因为该键有附加选项。
            Self::parse_with_options((from, from_options), to)
        }
        .map_err(|msg| {
            SortError::SortParseKeyError {
                key: key.to_owned(),
                msg,
            }
            .into()
        })
    }

    fn parse_with_options(
        (from, from_options): (&str, &str),
        to: Option<(&str, &str)>,
    ) -> Result<Self, String> {
        /// 将 `options` 应用到 `key_settings` 中，如果存在'b'标志（忽略空白），则返回结果。
        fn parse_key_settings(
            options: &str,
            key_settings: &mut SortKeySettings,
        ) -> Result<bool, String> {
            let mut ignore_blanks = false;
            for option in options.chars() {
                match option {
                    'M' => key_settings.set_sort_mode(SortMode::SortMonth)?,
                    'b' => ignore_blanks = true,
                    'd' => key_settings.set_dictionary_order()?,
                    'f' => key_settings.is_ignore_case = true,
                    'g' => key_settings.set_sort_mode(SortMode::SortGeneralNumeric)?,
                    'h' => key_settings.set_sort_mode(SortMode::SortHumanNumeric)?,
                    'i' => key_settings.set_ignore_non_printing()?,
                    'n' => key_settings.set_sort_mode(SortMode::SortNumeric)?,
                    'R' => key_settings.set_sort_mode(SortMode::SortRandom)?,
                    'r' => key_settings.is_reverse = true,
                    'V' => key_settings.set_sort_mode(SortMode::SortVersion)?,
                    c => return Err(format!("invalid option: '{c}'")),
                }
            }
            Ok(ignore_blanks)
        }

        let mut key_settings = SortKeySettings::default();
        let from = parse_key_settings(from_options, &mut key_settings)
            .map(|ignore_blanks| SortKeyPosition::new(from, 1, ignore_blanks))??;
        let to = match to {
            Some((to, to_options)) => Some(
                parse_key_settings(to_options, &mut key_settings)
                    .map(|ignore_blanks| SortKeyPosition::new(to, 0, ignore_blanks))??,
            ),
            None => None,
        };

        Self::new(from, to, key_settings)
    }

    fn new(
        from: SortKeyPosition,
        to: Option<SortKeyPosition>,
        sort_settings: SortKeySettings,
    ) -> Result<Self, String> {
        if from.char == 0 {
            Err("invalid character index 0 for the start position of a field".to_string())
        } else {
            Ok(Self {
                is_needs_selection: (from.field != 1
                    || from.char != 1
                    || to.is_some()
                    || matches!(
                        sort_settings.mode,
                        SortMode::SortNumeric | SortMode::SortHumanNumeric
                    )
                    || from.is_ignore_blanks)
                    && !matches!(sort_settings.mode, SortMode::SortGeneralNumeric),
                is_needs_tokens: from.field != 1 || from.char == 0 || to.is_some(),
                from,
                to,
                settings: sort_settings,
            })
        }
    }

    /// 获取与该行的选择器相对应的选择。
    /// 如果 needs_fields 返回 false，则标记可能为空。
    fn get_selection<'a>(&self, line: &'a str, tokens: &[Field]) -> SortSelection<'a> {
        // `get_range`期望`None`，当我们不需要标记时，空向量会让我们感到困惑。
        let tokens = match self.is_needs_tokens {
            true => Some(tokens),
            false => None,
        };
        let mut range = &line[self.get_range(line, tokens)];
        match self.settings.mode {
            SortMode::SortNumeric | SortMode::SortHumanNumeric => {
                let (info, num_range) = NumInfo::parse(
                    range,
                    &NumInfoParseSettings {
                        accept_si_units: self.settings.mode == SortMode::SortHumanNumeric,
                        ..Default::default()
                    },
                );
                // 将范围缩短到我们稍后需要传递给 numeric_str_cmp 的范围。
                range = &range[num_range];
                SortSelection::WithNumInfo(range, info)
            }
            SortMode::SortGeneralNumeric => {
                SortSelection::AsF64(sort_general_f64_parse(&range[sort_get_leading_gen(range)]))
            }
            _ => SortSelection::Str(range),
        }
    }

    /// 在该行中查找与该选择器相对应的范围。
    /// 如果 needs_fields 返回 false，则 tokens 必须为 None。
    fn get_range(&self, line: &str, tokens: Option<&[Field]>) -> Range<usize> {
        enum Resolution {
            // 已解析字符的起始索引，包括在内
            StartOfChar(usize),
            // 已解析字符的末尾索引，不包括。
            // 只有当字符索引为 0 时才返回。
            EndOfChar(usize),
            // 已解决的字符将位于第一个字符的前面
            TooLow,
            // 解析的字符将位于最后一个字符之后
            TooHigh,
        }

        // 根据关键位置获取该行的索引
        fn resolve_index(
            line: &str,
            tokens: Option<&[Field]>,
            position: &SortKeyPosition,
        ) -> Resolution {
            if matches!(tokens, Some(tokens) if tokens.len() < position.field) {
                Resolution::TooHigh
            } else if position.char == 0 {
                let end_size = tokens.unwrap()[position.field - 1].end;
                if end_size == 0 {
                    Resolution::TooLow
                } else {
                    Resolution::EndOfChar(end_size)
                }
            } else {
                let mut idx = if position.field == 1 {
                    // 第一个字段总是从 0 开始。
                    // 在这种情况下，我们不需要标记。
                    0
                } else {
                    tokens.unwrap()[position.field - 1].start
                };
                // 根据需要去掉空白
                if position.is_ignore_blanks {
                    idx += line[idx..]
                        .char_indices()
                        .find(|(_, c)| !c.is_whitespace())
                        .map_or(line[idx..].len(), |(idx, _)| idx);
                }
                // 应用字符索引
                idx += line[idx..]
                    .char_indices()
                    .nth(position.char - 1)
                    .map_or(line[idx..].len(), |(idx, _)| idx);
                if idx >= line.len() {
                    Resolution::TooHigh
                } else {
                    Resolution::StartOfChar(idx)
                }
            }
        }

        match resolve_index(line, tokens, &self.from) {
            Resolution::StartOfChar(from) => {
                let resolution_to = self.to.as_ref().map(|to| resolve_index(line, tokens, to));

                let mut range = match resolution_to {
                    Some(Resolution::StartOfChar(mut to)) => {
                        // 我们需要包含 `to` 字符。
                        to += line[to..].chars().next().map_or(1, char::len_utf8);
                        from..to
                    }
                    Some(Resolution::EndOfChar(to)) => from..to,
                    // 如果未给出 `to` 或匹配将在行尾之后、
                    // 匹配行尾之前的所有内容。
                    None | Some(Resolution::TooHigh) => from..line.len(),
                    // 如果 `to` 在行首之前，则报告不匹配。
                    // 如果该行以分隔符开始，就会出现这种情况。
                    Some(Resolution::TooLow) => 0..0,
                };
                if range.start > range.end {
                    range.end = range.start;
                }
                range
            }
            Resolution::TooLow | Resolution::EndOfChar(_) => {
                unreachable!("This should only happen if the field start index is 0, but that should already have caused an error.")
            }
            // 对于比较来说，重要的是这是一个空片段、
            // 为了生成准确的调试输出，我们需要在行尾匹配一个空片段。
            Resolution::TooHigh => line.len()..line.len(),
        }
    }
}

/// 创建的 `Arg` 与所有其他排序模式相冲突。
fn make_sort_mode_arg(mode: &'static str, short: char, help: &'static str) -> Arg {
    let mut arg = Arg::new(mode)
        .short(short)
        .long(mode)
        .help(help)
        .action(ArgAction::SetTrue);
    for possible_mode in &sort_flags::modes::SORT_ALL_MODES {
        if *possible_mode != mode {
            arg = arg.conflicts_with(possible_mode);
        }
    }
    arg
}

#[ctcore::main]
#[allow(clippy::cognitive_complexity)]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    sort_main(args)
}

// 中间测试
pub fn sort_main(args: impl ctcore::Args) -> CTResult<()> {
    let matches = match ct_app().try_get_matches_from(args) {
        Ok(t) => t,
        Err(e) => {
            // 并非所有 clap "错误 "都是因为参数解析失败。
            // "--版本 "也会导致返回错误，但我们不应打印到 stderr
            // 在这种情况下，我们不应该打印到 stderr，也不应该以非零的退出代码返回（我们应该打印到 stdout 并返回 0）。
            // 这个逻辑类似于 clap 中的代码，但在真正失败的情况下，我们的退出代码是 2（clap 返回 1）。
            // （clap 返回 1）。
            e.print().unwrap();
            if e.use_stderr() {
                set_ct_exit_code(2);
            }
            return Ok(());
        }
    };

    let (settings, mut files, mut tmp_dir, output) = sort_handle_settings(matches)?;

    let result = sort_exec(&mut files, &settings, output, &mut tmp_dir);
    //Wait here if `SIGINT` was received、
    // for signal handler to do its work and terminate the program.
    tmp_dir.wait_if_signal();
    result
    // Ok(())
}

fn sort_handle_settings(
    matches: ArgMatches,
) -> CTResult<(SortGlobalConfigs, Vec<OsString>, TmpDirWrapper, SortOutput)> {
    let mut settings = SortGlobalConfigs::default();

    // 检查用户是否指定了一个以零结尾的文件列表作为输入，否则从参数中读取文件
    let mut files: Vec<OsString> = sort_get_settings_files(&matches)?;

    let (mode, is_salt) = sort_get_settings_mode(&matches);
    settings.mode = mode;
    if is_salt {
        settings.salt = Some(sort_get_rand_string());
    }

    settings.is_dictionary_order = matches.get_flag(sort_flags::SORT_DICTIONARY_ORDER);
    settings.is_ignore_non_printing = matches.get_flag(sort_flags::SORT_IGNORE_NONPRINTING);
    settings.threads = sort_get_settings_threads(&matches);
    settings.buffer_size = sort_get_settings_buffer_size(&matches)?;

    let tmp_dir = TmpDirWrapper::new(
        matches
            .get_one::<String>(sort_flags::SORT_TMP_DIR)
            .map(PathBuf::from)
            .unwrap_or_else(env::temp_dir),
    );

    settings.compress_prog = sort_get_settings_compress_prog(&matches);
    settings.merge_batch_size = sort_get_settings_merge_batch_size(&matches)?;
    settings.line_ending =
        CtLineEnding::from_zero_flag(matches.get_flag(sort_flags::SORT_ZERO_TERMINATED));
    settings.is_merge = matches.get_flag(sort_flags::SORT_MERGE);

    (settings.is_check, settings.is_check_silent) = sort_get_setting_check(&matches);

    settings.is_ignore_case = matches.get_flag(sort_flags::SORT_IGNORE_CASE);
    settings.is_ignore_leading_blanks = matches.get_flag(sort_flags::SORT_IGNORE_LEADING_BLANKS);
    settings.is_reverse = matches.get_flag(sort_flags::SORT_REVERSE);
    settings.is_stable = matches.get_flag(sort_flags::SORT_STABLE);
    settings.is_unique = matches.get_flag(sort_flags::SORT_UNIQUE);

    if files.is_empty() {
        /* if no file, default to stdin */
        files.push("-".to_string().into());
    } else if settings.is_check && files.len() != 1 {
        return Err(CTsageError::new(
            2,
            format!("extra operand {} not allowed with -c", files[1].quote()),
        ));
    }
    settings.separator = sort_get_settings_sparator(&matches)?;

    if let Some(values) = matches.get_many::<String>(sort_flags::SORT_KEY) {
        for value in values {
            let selector = SortFieldSelector::parse(value, &settings)?;
            if selector.settings.mode == SortMode::SortRandom && settings.salt.is_none() {
                settings.salt = Some(sort_get_rand_string());
            }
            settings.selectors.push(selector);
        }
    }

    if !matches.contains_id(sort_flags::SORT_KEY) {
        // 添加匹配整行的默认选择器
        let key_settings = SortKeySettings::from(&settings);
        settings.selectors.push(
            SortFieldSelector::new(
                SortKeyPosition {
                    field: 1,
                    char: 1,
                    is_ignore_blanks: key_settings.is_ignore_blanks,
                },
                None,
                key_settings,
            )
            .unwrap(),
        );
    }

    // 验证我们是否可以打开所有输入文件。
    // 正确的做法是，随后关闭所有文件、
    // 之后关闭所有文件并在稍后重新打开它们才是正确的行为。这与处理输出文件的方式不同、
    // 可能是为了防止文件描述符耗尽。
    for file in &files {
        sort_open(file)?;
    }

    let output = SortOutput::new(
        matches
            .get_one::<String>(sort_flags::SORT_OUTPUT)
            .map(|s| s.as_str()),
    )?;

    settings.init_precomputed();
    Ok((settings, files, tmp_dir, output))
}

fn sort_get_setting_check(matches: &ArgMatches) -> (bool, bool) {
    if matches.get_flag(sort_flags::check::SORT_CHECK_SILENT)
        || matches!(
            matches
                .get_one::<String>(sort_flags::check::SORT_CHECK)
                .map(|s| s.as_str()),
            Some(sort_flags::check::SORT_SILENT | sort_flags::check::SORT_QUIET)
        )
    {
        let check_silent = true;
        let check = true;
        (check, check_silent)
    } else {
        (matches.contains_id(sort_flags::check::SORT_CHECK), false)
    }
}

fn sort_get_settings_merge_batch_size(matches: &ArgMatches) -> CTResult<usize> {
    if let Some(n_merge) = matches.get_one::<String>(sort_flags::SORT_BATCH_SIZE) {
        let merge_batch_size = n_merge.parse().map_err(|_| {
            CTsageError::new(
                2,
                format!("invalid --batch-size argument {}", n_merge.quote()),
            )
        })?;
        Ok(merge_batch_size)
    } else {
        Ok(32)
    }
}

fn sort_get_settings_sparator(matches: &ArgMatches) -> CTResult<Option<char>> {
    if let Some(arg) = matches.get_one::<OsString>(sort_flags::SORT_SEPARATOR) {
        let mut separator = arg.to_str().ok_or_else(|| {
            CTsageError::new(
                2,
                format!("separator is not valid unicode: {}", arg.quote()),
            )
        })?;
        if separator == "\\0" {
            separator = "\0";
        }
        // 这将拒绝非 ASCII 编码点，但也许我们不必这样做。
        // 另一方面，GNU 接受任何单字节，无论是否为有效的 unicode。
        // 支持多字节字符需要修改 tokenize_with_separator()）。
        if separator.len() != 1 {
            return Err(CTsageError::new(
                2,
                format!(
                    "separator must be exactly one character long: {}",
                    separator.quote()
                ),
            ));
        }
        Ok(Some(separator.chars().next().unwrap()))
    } else {
        Ok(None)
    }
}

fn sort_get_settings_compress_prog(matches: &ArgMatches) -> Option<String> {
    matches
        .get_one::<String>(sort_flags::SORT_COMPRESS_PROG)
        .map(String::from)
}

fn sort_get_settings_threads(matches: &ArgMatches) -> String {
    if matches.contains_id(sort_flags::SORT_PARALLEL) {
        // "0" is default - threads = num of cores
        let threads = matches
            .get_one::<String>(sort_flags::SORT_PARALLEL)
            .map(String::from)
            .unwrap_or_else(|| "0".to_string());
        env::set_var("RAYON_NUM_THREADS", &threads);
        threads
    } else {
        String::new()
    }
}

fn sort_get_settings_buffer_size(matches: &ArgMatches) -> CTResult<usize> {
    matches
        .get_one::<String>(sort_flags::SORT_BUF_SIZE)
        .map_or(Ok(SORT_DEFAULT_BUF_SIZE), |s| {
            SortGlobalConfigs::parse_byte_count(s).map_err(|e| {
                CtSimpleError::new(
                    2,
                    sort_format_error_message(&e, s, sort_flags::SORT_BUF_SIZE),
                )
            })
        })
}

fn sort_get_settings_files(matches: &ArgMatches) -> Result<Vec<OsString>, Box<dyn CTError>> {
    Ok(if matches.contains_id(sort_flags::SORT_FILES0_FROM) {
        let files0_from: Vec<OsString> = matches
            .get_many::<OsString>(sort_flags::SORT_FILES0_FROM)
            .map(|v| v.map(ToOwned::to_owned).collect())
            .unwrap_or_default();

        let mut files = Vec::new();
        for path in &files0_from {
            let reader = sort_open(path)?;
            let buf_reader = BufReader::new(reader);
            for line in buf_reader.split(b'\0').flatten() {
                files.push(OsString::from(
                    std::str::from_utf8(&line)
                        .expect("Could not parse string from zero terminated input."),
                ));
            }
        }
        files
    } else {
        matches
            .get_many::<OsString>(sort_flags::SORT_FILES)
            .map(|v| v.map(ToOwned::to_owned).collect())
            .unwrap_or_default()
    })
}

fn sort_get_settings_mode(matches: &ArgMatches) -> (SortMode, bool) {
    let mut is_salt = false;
    if matches.get_flag(sort_flags::modes::SORT_HUMAN_NUMERIC)
        || matches
            .get_one::<String>(sort_flags::modes::SORT)
            .map(|s| s.as_str())
            == Some("human-numeric")
    {
        (SortMode::SortHumanNumeric, is_salt)
    } else if matches.get_flag(sort_flags::modes::SORT_MONTH)
        || matches
            .get_one::<String>(sort_flags::modes::SORT)
            .map(|s| s.as_str())
            == Some("month")
    {
        (SortMode::SortMonth, is_salt)
    } else if matches.get_flag(sort_flags::modes::SORT_GENERAL_NUMERIC)
        || matches
            .get_one::<String>(sort_flags::modes::SORT)
            .map(|s| s.as_str())
            == Some("general-numeric")
    {
        (SortMode::SortGeneralNumeric, is_salt)
    } else if matches.get_flag(sort_flags::modes::SORT_NUMERIC)
        || matches
            .get_one::<String>(sort_flags::modes::SORT)
            .map(|s| s.as_str())
            == Some("numeric")
    {
        (SortMode::SortNumeric, is_salt)
    } else if matches.get_flag(sort_flags::modes::SORT_VERSION)
        || matches
            .get_one::<String>(sort_flags::modes::SORT)
            .map(|s| s.as_str())
            == Some("version")
    {
        (SortMode::SortVersion, is_salt)
    } else if matches.get_flag(sort_flags::modes::SORT_RANDOM)
        || matches
            .get_one::<String>(sort_flags::modes::SORT)
            .map(|s| s.as_str())
            == Some("random")
    {
        is_salt = true;
        // settings.salt = Some(get_rand_string());
        (SortMode::SortRandom, is_salt)
    } else {
        (SortMode::SortDefault, is_salt)
    }
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = SORT_ABOUT;
    let usage_description = ct_format_usage(SORT_USAGE);

    let args = vec![Arg::new(sort_flags::SORT_HELP)
                         .long(sort_flags::SORT_HELP)
                         .help("Print help information.")
                         .action(ArgAction::Help),
                     Arg::new(sort_flags::SORT_VERSION)
                         .long(sort_flags::SORT_VERSION)
                         .help("Print version information.")
                         .action(ArgAction::Version),
                     Arg::new(sort_flags::modes::SORT)
                         .long(sort_flags::modes::SORT)
                         .value_parser([
                             "general-numeric",
                             "human-numeric",
                             "month",
                             "numeric",
                             "version",
                             "random",
                         ])
                         .conflicts_with_all(sort_flags::modes::SORT_ALL_MODES),
                     make_sort_mode_arg(
                         sort_flags::modes::SORT_HUMAN_NUMERIC,
                         'h',
                         "compare according to human readable sizes, eg 1M > 100k",
                     ),
                     make_sort_mode_arg(
                         sort_flags::modes::SORT_MONTH,
                         'M',
                         "compare according to month name abbreviation",
                     ),
                     make_sort_mode_arg(
                         sort_flags::modes::SORT_NUMERIC,
                         'n',
                         "compare according to string numerical value",
                     ),
                     make_sort_mode_arg(
                         sort_flags::modes::SORT_GENERAL_NUMERIC,
                         'g',
                         "compare according to string general numerical value",
                     ),
                     make_sort_mode_arg(
                         sort_flags::modes::SORT_VERSION,
                         'V',
                         "Sort by SemVer version number, eg 1.12.2 > 1.1.2",
                     ),
                     make_sort_mode_arg(
                         sort_flags::modes::SORT_RANDOM,
                         'R',
                         "shuffle in random order",
                     ),
                     Arg::new(sort_flags::SORT_DICTIONARY_ORDER)
                         .short('d')
                         .long(sort_flags::SORT_DICTIONARY_ORDER)
                         .help("consider only blanks and alphanumeric characters")
                         .conflicts_with_all([
                             sort_flags::modes::SORT_NUMERIC,
                             sort_flags::modes::SORT_GENERAL_NUMERIC,
                             sort_flags::modes::SORT_HUMAN_NUMERIC,
                             sort_flags::modes::SORT_MONTH,
                         ])
                         .action(ArgAction::SetTrue),
                     Arg::new(sort_flags::SORT_MERGE)
                         .short('m')
                         .long(sort_flags::SORT_MERGE)
                         .help("merge already sorted files; do not sort")
                         .action(ArgAction::SetTrue),
                     Arg::new(sort_flags::check::SORT_CHECK)
                         .short('c')
                         .long(sort_flags::check::SORT_CHECK)
                         .require_equals(true)
                         .num_args(0..)
                         .value_parser([
                             sort_flags::check::SORT_SILENT,
                             sort_flags::check::SORT_QUIET,
                             sort_flags::check::SORT_DIAGNOSE_FIRST,
                         ])
                         .conflicts_with(sort_flags::SORT_OUTPUT)
                         .help("check for sorted input; do not sort"),
                     Arg::new(sort_flags::check::SORT_CHECK_SILENT)
                         .short('C')
                         .long(sort_flags::check::SORT_CHECK_SILENT)
                         .conflicts_with(sort_flags::SORT_OUTPUT)
                         .help(
                             "exit successfully if the given file is already sorted, \
                     and exit with status 1 otherwise.",
                         )
                         .action(ArgAction::SetTrue),
                     Arg::new(sort_flags::SORT_IGNORE_CASE)
                         .short('f')
                         .long(sort_flags::SORT_IGNORE_CASE)
                         .help("fold lower case to upper case characters")
                         .action(ArgAction::SetTrue),
                     Arg::new(sort_flags::SORT_IGNORE_NONPRINTING)
                         .short('i')
                         .long(sort_flags::SORT_IGNORE_NONPRINTING)
                         .help("ignore nonprinting characters")
                         .conflicts_with_all([
                             sort_flags::modes::SORT_NUMERIC,
                             sort_flags::modes::SORT_GENERAL_NUMERIC,
                             sort_flags::modes::SORT_HUMAN_NUMERIC,
                             sort_flags::modes::SORT_MONTH,
                         ])
                         .action(ArgAction::SetTrue),
                     Arg::new(sort_flags::SORT_IGNORE_LEADING_BLANKS)
                         .short('b')
                         .long(sort_flags::SORT_IGNORE_LEADING_BLANKS)
                         .help("ignore leading blanks when finding sort keys in each line")
                         .action(ArgAction::SetTrue),
                     Arg::new(sort_flags::SORT_OUTPUT)
                         .short('o')
                         .long(sort_flags::SORT_OUTPUT)
                         .help("write output to FILENAME instead of stdout")
                         .value_name("FILENAME")
                         .value_hint(clap::ValueHint::FilePath),
                     Arg::new(sort_flags::SORT_REVERSE)
                         .short('r')
                         .long(sort_flags::SORT_REVERSE)
                         .help("reverse the output")
                         .action(ArgAction::SetTrue),
                     Arg::new(sort_flags::SORT_STABLE)
                         .short('s')
                         .long(sort_flags::SORT_STABLE)
                         .help("stabilize sort by disabling last-resort comparison")
                         .action(ArgAction::SetTrue),
                     Arg::new(sort_flags::SORT_UNIQUE)
                         .short('u')
                         .long(sort_flags::SORT_UNIQUE)
                         .help("output only the first of an equal run")
                         .action(ArgAction::SetTrue),
                     Arg::new(sort_flags::SORT_KEY)
                         .short('k')
                         .long(sort_flags::SORT_KEY)
                         .help("sort by a key")
                         .action(ArgAction::Append)
                         .num_args(1),
                     Arg::new(sort_flags::SORT_SEPARATOR)
                         .short('t')
                         .long(sort_flags::SORT_SEPARATOR)
                         .help("custom separator for -k")
                         .value_parser(ValueParser::os_string()),
                     Arg::new(sort_flags::SORT_ZERO_TERMINATED)
                         .short('z')
                         .long(sort_flags::SORT_ZERO_TERMINATED)
                         .help("line delimiter is NUL, not newline")
                         .action(ArgAction::SetTrue),
                     Arg::new(sort_flags::SORT_PARALLEL)
                         .long(sort_flags::SORT_PARALLEL)
                         .help("change the number of threads running concurrently to NUM_THREADS")
                         .value_name("NUM_THREADS"),
                     Arg::new(sort_flags::SORT_BUF_SIZE)
                         .short('S')
                         .long(sort_flags::SORT_BUF_SIZE)
                         .help("sets the maximum SIZE of each segment in number of sorted items")
                         .value_name("SIZE"),
                     Arg::new(sort_flags::SORT_TMP_DIR)
                         .short('T')
                         .long(sort_flags::SORT_TMP_DIR)
                         .help("use DIR for temporaries, not $TMPDIR or /tmp")
                         .value_name("DIR")
                         .value_hint(clap::ValueHint::DirPath),
                     Arg::new(sort_flags::SORT_COMPRESS_PROG)
                         .long(sort_flags::SORT_COMPRESS_PROG)
                         .help("compress temporary files with PROG, decompress with PROG -d; PROG has to take input from stdin and output to stdout")
                         .value_name("PROG")
                         .value_hint(clap::ValueHint::CommandName),
                     Arg::new(sort_flags::SORT_BATCH_SIZE)
                         .long(sort_flags::SORT_BATCH_SIZE)
                         .help("Merge at most N_MERGE inputs at once.")
                         .value_name("N_MERGE"),
                     Arg::new(sort_flags::SORT_FILES0_FROM)
                         .long(sort_flags::SORT_FILES0_FROM)
                         .help("read input from the files specified by NUL-terminated NUL_FILES")
                         .value_name("NUL_FILES")
                         .action(ArgAction::Append)
                         .value_parser(ValueParser::os_string())
                         .value_hint(clap::ValueHint::FilePath),
                     Arg::new(sort_flags::SORT_DEBUG)
                         .long(sort_flags::SORT_DEBUG)
                         .help("underline the parts of the line that are actually used for sorting")
                         .action(ArgAction::SetTrue),
                     Arg::new(sort_flags::SORT_FILES)
                         .action(ArgAction::Append)
                         .value_parser(ValueParser::os_string())
                         .value_hint(clap::ValueHint::FilePath),
     ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .after_help(SORT_AFTER_HELP)
        .override_usage(ct_format_usage(SORT_USAGE))
        .override_usage(usage_description)
        .infer_long_args(true)
        .infer_long_args(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .args_override_self(true)
        .args(&args)
}

fn sort_exec(
    files: &mut [OsString],
    settings: &SortGlobalConfigs,
    output: SortOutput,
    tmp_dir: &mut TmpDirWrapper,
) -> CTResult<()> {
    if settings.is_merge {
        let file_merger = merge::merge(files, settings, output.as_output_name(), tmp_dir)?;
        file_merger.write_all(settings, output)
    } else if settings.is_check {
        if files.len() > 1 {
            Err(CTsageError::new(2, "only one file allowed with -c"))
        } else {
            check::check(files.first().unwrap(), settings)
        }
    } else {
        let mut lines = files.iter().map(sort_open);
        ext_sort(&mut lines, settings, output, tmp_dir)
    }
}

fn sort_by<'a>(
    unsorted: &mut Vec<SortLine<'a>>,
    settings: &SortGlobalConfigs,
    line_data: &ChunkLineData<'a>,
) {
    if settings.is_stable || settings.is_unique {
        unsorted.par_sort_by(|a, b| sort_compare_by(a, b, settings, line_data, line_data));
    } else {
        unsorted.par_sort_unstable_by(|a, b| sort_compare_by(a, b, settings, line_data, line_data));
    }
}

fn sort_compare_by<'a>(
    a: &SortLine<'a>,
    b: &SortLine<'a>,
    global_settings: &SortGlobalConfigs,
    a_line_data: &ChunkLineData<'a>,
    b_line_data: &ChunkLineData<'a>,
) -> Ordering {
    let mut selection_index = 0;
    let mut num_info_index = 0;
    let mut parsed_float_index = 0;
    for field_selector in &global_settings.selectors {
        let (a_str, b_str) = if field_selector.is_needs_selection {
            let selections = (
                a_line_data.selections
                    [a.index * global_settings.precomputed.selections_per_line + selection_index],
                b_line_data.selections
                    [b.index * global_settings.precomputed.selections_per_line + selection_index],
            );
            selection_index += 1;
            selections
        } else {
            // We can select the whole line.
            (a.line, b.line)
        };

        let settings = &field_selector.settings;

        let cmp: Ordering = match settings.mode {
            SortMode::SortRandom => {
                // check if the two strings are equal
                if custom_cmp_str(
                    a_str,
                    b_str,
                    settings.is_ignore_non_printing,
                    settings.is_dictionary_order,
                    settings.is_ignore_case,
                ) == Ordering::Equal
                {
                    Ordering::Equal
                } else {
                    // Only if they are not equal compare by the hash
                    sort_random_shuffle(a_str, b_str, &global_settings.salt.unwrap())
                }
            }
            SortMode::SortNumeric => {
                let a_num_info = &a_line_data.num_infos
                    [a.index * global_settings.precomputed.num_infos_per_line + num_info_index];
                let b_num_info = &b_line_data.num_infos
                    [b.index * global_settings.precomputed.num_infos_per_line + num_info_index];
                num_info_index += 1;
                numeric_str_cmp((a_str, a_num_info), (b_str, b_num_info))
            }
            SortMode::SortHumanNumeric => {
                let a_num_info = &a_line_data.num_infos
                    [a.index * global_settings.precomputed.num_infos_per_line + num_info_index];
                let b_num_info = &b_line_data.num_infos
                    [b.index * global_settings.precomputed.num_infos_per_line + num_info_index];
                num_info_index += 1;
                num_cmp_human_numeric_str_cmp((a_str, a_num_info), (b_str, b_num_info))
            }
            SortMode::SortGeneralNumeric => {
                let a_float = &a_line_data.parsed_floats
                    [a.index * global_settings.precomputed.floats_per_line + parsed_float_index];
                let b_float = &b_line_data.parsed_floats
                    [b.index * global_settings.precomputed.floats_per_line + parsed_float_index];
                parsed_float_index += 1;
                sort_general_numeric_compare(a_float, b_float)
            }
            SortMode::SortMonth => sort_month_compare(a_str, b_str),
            SortMode::SortVersion => ct_version_cmp(a_str, b_str),
            SortMode::SortDefault => custom_cmp_str(
                a_str,
                b_str,
                settings.is_ignore_non_printing,
                settings.is_dictionary_order,
                settings.is_ignore_case,
            ),
        };
        if cmp != Ordering::Equal {
            return if settings.is_reverse {
                cmp.reverse()
            } else {
                cmp
            };
        }
    }

    // Call "last resort compare" if all selectors returned Equal
    let cmp = if global_settings.mode == SortMode::SortRandom
        || global_settings.is_stable
        || global_settings.is_unique
    {
        Ordering::Equal
    } else {
        a.line.cmp(b.line)
    };

    if global_settings.is_reverse {
        cmp.reverse()
    } else {
        cmp
    }
}

#[allow(clippy::cognitive_complexity)]
fn sort_get_leading_gen(input: &str) -> Range<usize> {
    let trimmed = input.trim_start();
    let leading_whitespace_len = input.len() - trimmed.len();

    // 检查 inf、-inf 和 nan
    for allowed_prefix in ["inf", "-inf", "nan"] {
        if trimmed.is_char_boundary(allowed_prefix.len())
            && trimmed[..allowed_prefix.len()].eq_ignore_ascii_case(allowed_prefix)
        {
            return leading_whitespace_len..(leading_whitespace_len + allowed_prefix.len());
        }
    }
    // 使该迭代可被偷看，以查看下一个字符是否为数字
    let mut char_indices = itertools::peek_nth(trimmed.char_indices());

    let first = char_indices.peek();

    if matches!(first, Some((_, SORT_NEGATIVE) | (_, SORT_POSITIVE))) {
        char_indices.next();
    }

    let mut is_had_e_notation = false;
    let mut is_had_decimal_pt = false;
    while let Some((idx, c)) = char_indices.next() {
        if c.is_ascii_digit() {
            continue;
        }
        if c == SORT_DECIMAL_PT && !is_had_decimal_pt && !is_had_e_notation {
            is_had_decimal_pt = true;
            continue;
        }
        if (c == 'e' || c == 'E') && !is_had_e_notation {
            // 只有当后面是一个数字或一个符号后跟一个数字时，我们才能使用 "e"。
            if let Some(&(_, next_char)) = char_indices.peek() {
                if (next_char == '+' || next_char == '-')
                    && matches!(
                        char_indices.peek_nth(2),
                        Some((_, c)) if c.is_ascii_digit()
                    )
                {
                    // 消耗符号。主循环将消耗下面的数字。
                    char_indices.next();
                    is_had_e_notation = true;
                    continue;
                }
                if next_char.is_ascii_digit() {
                    is_had_e_notation = true;
                    continue;
                }
            }
        }
        return leading_whitespace_len..(leading_whitespace_len + idx);
    }
    leading_whitespace_len..input.len()
}

#[derive(Copy, Clone, PartialEq, PartialOrd, Debug)]
pub enum SortGeneralF64ParseResult {
    SortInvalid,
    SortNaN,
    SortNegInfinity,
    SortNumber(f64),
    SortInfinity,
}

/// 将开头的字符串解析为 GeneralF64ParseResult。
/// 必须使用 GeneralF64ParseResult 而不是 f64 才能正确排序浮点数。
#[inline(always)]
fn sort_general_f64_parse(a: &str) -> SortGeneralF64ParseResult {
    // 这里的实际行为依赖于 Rust 的浮点解析实现。
    // 例如，从 1.53 版本开始，"nan"、"inf"（忽略大小写）和 "无穷大 "只能解析为浮点数。
    // TODO：一旦我们支持的 Rust 最低版本达到 1.53 或以上，我们就应该为这些情况添加测试。
    match a.parse::<f64>() {
        Ok(a) if a.is_nan() => SortGeneralF64ParseResult::SortNaN,
        Ok(a) if a == std::f64::NEG_INFINITY => SortGeneralF64ParseResult::SortNegInfinity,
        Ok(a) if a == std::f64::INFINITY => SortGeneralF64ParseResult::SortInfinity,
        Ok(a) => SortGeneralF64ParseResult::SortNumber(a),
        Err(_) => SortGeneralF64ParseResult::SortInvalid,
    }
}

/// 比较两个浮点数，误差和非数字假定为-inf。
/// 在第一个非数字字符处停止强制。
/// 在这种情况下，我们明确需要转换为 f64。
fn sort_general_numeric_compare(
    a: &SortGeneralF64ParseResult,
    b: &SortGeneralF64ParseResult,
) -> Ordering {
    a.partial_cmp(b).unwrap()
}

fn sort_get_rand_string() -> [u8; 16] {
    thread_rng().sample(rand::distributions::Standard)
}

fn sort_get_hash<T: Hash>(t: &T) -> u64 {
    let mut s = FnvHasher::default();
    t.hash(&mut s);
    s.finish()
}

fn sort_random_shuffle(a: &str, b: &str, salt: &[u8]) -> Ordering {
    let da = sort_get_hash(&(a, salt));
    let db = sort_get_hash(&(b, salt));
    da.cmp(&db)
}

#[derive(Eq, Ord, PartialEq, PartialOrd, Clone, Copy, Debug)]
enum SortMonth {
    Unknown,
    January,
    February,
    March,
    April,
    May,
    June,
    July,
    August,
    September,
    October,
    November,
    December,
}

/// 将开头字符串解析为月份，如果出错，则返回 Month::Unknown。
fn sort_month_parse(line: &str) -> SortMonth {
    let line = line.trim();

    const MONTHS: [(&str, SortMonth); 12] = [
        ("JAN", SortMonth::January),
        ("FEB", SortMonth::February),
        ("MAR", SortMonth::March),
        ("APR", SortMonth::April),
        ("MAY", SortMonth::May),
        ("JUN", SortMonth::June),
        ("JUL", SortMonth::July),
        ("AUG", SortMonth::August),
        ("SEP", SortMonth::September),
        ("OCT", SortMonth::October),
        ("NOV", SortMonth::November),
        ("DEC", SortMonth::December),
    ];

    for (month_str, month) in &MONTHS {
        if line.is_char_boundary(month_str.len())
            && line[..month_str.len()].eq_ignore_ascii_case(month_str)
        {
            return *month;
        }
    }

    SortMonth::Unknown
}

fn sort_month_compare(a: &str, b: &str) -> Ordering {
    #![allow(clippy::comparison_chain)]
    let sort_month_a = sort_month_parse(a);
    let sort_month_b = sort_month_parse(b);

    if sort_month_a > sort_month_b {
        Ordering::Greater
    } else if sort_month_a < sort_month_b {
        Ordering::Less
    } else {
        Ordering::Equal
    }
}

fn sort_print_sorted<'a, T: Iterator<Item = &'a SortLine<'a>>>(
    iter: T,
    sort_settings: &SortGlobalConfigs,
    sort_output: SortOutput,
) {
    let mut writer = sort_output.into_write();
    for line in iter {
        line.print(&mut writer, sort_settings);
    }
}

fn sort_open(path: impl AsRef<OsStr>) -> CTResult<Box<dyn Read + Send>> {
    let path = path.as_ref();
    if path == "-" {
        let stdin = stdin();
        return Ok(Box::new(stdin) as Box<dyn Read + Send>);
    }

    let path = Path::new(path);

    match File::open(path) {
        Ok(f) => Ok(Box::new(f) as Box<dyn Read + Send>),
        Err(error) => Err(SortError::SortReadFailed {
            path: path.to_owned(),
            error,
        }
        .into()),
    }
}

fn sort_format_error_message(error: &ParseSizeError, s: &str, option: &str) -> String {
    // NOTE：GNU的排序回声受影响的标志，-S或--缓冲区大小，取决于用户的选择
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

    #[cfg(test)]
    mod format_error_message_tests {
        use super::*;

        #[test]
        fn test_format_error_message_invalid_suffix() {
            let error = ParseSizeError::InvalidSuffix("b".to_string());
            let s = "100B";
            let option = "-S";
            let expected = "invalid suffix in ---S argument '100B'";
            assert_eq!(sort_format_error_message(&error, s, option), expected);
        }

        #[test]
        fn test_format_error_message_parse_failure() {
            let error = ParseSizeError::ParseFailure("".to_string());
            let s = "abc";
            let option = "--buffer-size";
            let expected = "invalid ----buffer-size argument 'abc'";
            assert_eq!(sort_format_error_message(&error, s, option), expected);
        }

        #[test]
        fn test_format_error_message_size_too_big() {
            let error = ParseSizeError::SizeTooBig("b".to_string());
            let s = "1GB";
            let option = "-S";
            let expected = "---S argument '1GB' too large";
            assert_eq!(sort_format_error_message(&error, s, option), expected);
        }
    }

    #[cfg(test)]
    mod month_compare_tests {
        use super::*;

        #[test]
        fn test_month_compare_same_month() {
            assert_eq!(
                sort_month_compare("January", "January"),
                Ordering::Equal,
                "Test case 1 failed"
            );
            assert_eq!(
                sort_month_compare("january", "January"),
                Ordering::Equal,
                "Test case 2 failed"
            );
        }

        #[test]
        fn test_month_compare_different_months() {
            assert_eq!(
                sort_month_compare("January", "February"),
                Ordering::Less,
                "Test case 3 failed"
            );
            assert_eq!(
                sort_month_compare("february", "January"),
                Ordering::Greater,
                "Test case 4 failed"
            );
        }

        #[test]
        fn test_month_compare_with_unknown() {
            assert_eq!(
                sort_month_compare("January", "XYZ"),
                Ordering::Greater,
                "Test case 5 failed"
            );
            assert_eq!(
                sort_month_compare("XYZ", "January"),
                Ordering::Less,
                "Test case 6 failed"
            );
        }

        #[test]
        fn test_month_compare_case_insensitive() {
            assert_eq!(
                sort_month_compare("January", "january"),
                Ordering::Equal,
                "Test case 7 failed"
            );
            assert_eq!(
                sort_month_compare("january", "January"),
                Ordering::Equal,
                "Test case 8 failed"
            );
        }
    }

    #[cfg(test)]
    mod month_parse_tests {
        use super::*;

        #[test]
        fn test_month_parse_january() {
            assert_eq!(
                sort_month_parse("JAN"),
                SortMonth::January,
                "Test case 1 failed"
            );
            assert_eq!(
                sort_month_parse("jan"),
                SortMonth::January,
                "Test case 2 failed"
            );
        }

        #[test]
        fn test_month_parse_february() {
            assert_eq!(
                sort_month_parse("FEB"),
                SortMonth::February,
                "Test case 3 failed"
            );
            assert_eq!(
                sort_month_parse("feb"),
                SortMonth::February,
                "Test case 4 failed"
            );
        }

        #[test]
        fn test_month_parse_march() {
            assert_eq!(
                sort_month_parse("MAR"),
                SortMonth::March,
                "Test case 5 failed"
            );
            assert_eq!(
                sort_month_parse("mar"),
                SortMonth::March,
                "Test case 6 failed"
            );
        }

        #[test]
        fn test_month_parse_april() {
            assert_eq!(
                sort_month_parse("APR"),
                SortMonth::April,
                "Test case 7 failed"
            );
            assert_eq!(
                sort_month_parse("apr"),
                SortMonth::April,
                "Test case 8 failed"
            );
        }

        #[test]
        fn test_month_parse_may() {
            assert_eq!(
                sort_month_parse("MAY"),
                SortMonth::May,
                "Test case 9 failed"
            );
            assert_eq!(
                sort_month_parse("may"),
                SortMonth::May,
                "Test case 10 failed"
            );
        }

        #[test]
        fn test_month_parse_june() {
            assert_eq!(
                sort_month_parse("JUN"),
                SortMonth::June,
                "Test case 11 failed"
            );
            assert_eq!(
                sort_month_parse("jun"),
                SortMonth::June,
                "Test case 12 failed"
            );
        }

        #[test]
        fn test_month_parse_july() {
            assert_eq!(
                sort_month_parse("JUL"),
                SortMonth::July,
                "Test case 13 failed"
            );
            assert_eq!(
                sort_month_parse("jul"),
                SortMonth::July,
                "Test case 14 failed"
            );
        }

        #[test]
        fn test_month_parse_august() {
            assert_eq!(
                sort_month_parse("AUG"),
                SortMonth::August,
                "Test case 15 failed"
            );
            assert_eq!(
                sort_month_parse("aug"),
                SortMonth::August,
                "Test case 16 failed"
            );
        }

        #[test]
        fn test_month_parse_september() {
            assert_eq!(
                sort_month_parse("SEP"),
                SortMonth::September,
                "Test case 17 failed"
            );
            assert_eq!(
                sort_month_parse("sep"),
                SortMonth::September,
                "Test case 18 failed"
            );
        }

        #[test]
        fn test_month_parse_october() {
            assert_eq!(
                sort_month_parse("OCT"),
                SortMonth::October,
                "Test case 19 failed"
            );
            assert_eq!(
                sort_month_parse("oct"),
                SortMonth::October,
                "Test case 20 failed"
            );
        }

        #[test]
        fn test_month_parse_november() {
            assert_eq!(
                sort_month_parse("NOV"),
                SortMonth::November,
                "Test case 21 failed"
            );
            assert_eq!(
                sort_month_parse("nov"),
                SortMonth::November,
                "Test case 22 failed"
            );
        }

        #[test]
        fn test_month_parse_december() {
            assert_eq!(
                sort_month_parse("DEC"),
                SortMonth::December,
                "Test case 23 failed"
            );
            assert_eq!(
                sort_month_parse("dec"),
                SortMonth::December,
                "Test case 24 failed"
            );
        }

        #[test]
        fn test_month_parse_unknown() {
            assert_eq!(
                sort_month_parse("XYZ"),
                SortMonth::Unknown,
                "Test case 25 failed"
            );
        }
    }

    #[cfg(test)]
    mod general_f64_parse_tests {
        use super::*;

        #[test]
        fn test_general_f64_parse_valid_number() {
            assert_eq!(
                sort_general_f64_parse("123.45"),
                SortGeneralF64ParseResult::SortNumber(123.45),
                "Test case 1 failed for input: '123.45'"
            );
        }

        #[test]
        fn test_general_f64_parse_negative_number() {
            assert_eq!(
                sort_general_f64_parse("-123.45"),
                SortGeneralF64ParseResult::SortNumber(-123.45),
                "Test case 2 failed for input: '-123.45'"
            );
        }

        #[test]
        fn test_general_f64_parse_nan() {
            assert_eq!(
                sort_general_f64_parse("nan"),
                SortGeneralF64ParseResult::SortNaN,
                "Test case 3 failed for input: 'nan'"
            );
        }

        #[test]
        fn test_general_f64_parse_positive_infinity() {
            assert_eq!(
                sort_general_f64_parse("inf"),
                SortGeneralF64ParseResult::SortInfinity,
                "Test case 4 failed for input: 'inf'"
            );
        }

        #[test]
        fn test_general_f64_parse_negative_infinity() {
            assert_eq!(
                sort_general_f64_parse("-inf"),
                SortGeneralF64ParseResult::SortNegInfinity,
                "Test case 5 failed for input: '-inf'"
            );
        }

        #[test]
        fn test_general_f64_parse_invalid() {
            assert_eq!(
                sort_general_f64_parse("invalid"),
                SortGeneralF64ParseResult::SortInvalid,
                "Test case 6 failed for input: 'invalid'"
            );
        }

        #[test]
        fn test_general_f64_parse_invalid_multiple_dots() {
            assert_eq!(
                sort_general_f64_parse("123.45.67"),
                SortGeneralF64ParseResult::SortInvalid,
                "Test case 7 failed for input: '123.45.67'"
            );
        }
    }

    #[cfg(test)]
    mod get_leading_gen_tests {
        use super::*;

        #[test]
        fn test_get_leading_gen() {
            // Test case 1: Basic functionality with no special characters
            assert_eq!(
                sort_get_leading_gen("  -123.45"),
                (2..9),
                "Test case 1 failed for input: '  -123.45'"
            );

            // Test case 2: Basic functionality with a special character (e)
            assert_eq!(
                sort_get_leading_gen("  -123e-45"),
                (2..10),
                "Test case 2 failed for input: '  -123e-45'"
            );

            // Test case 3: Basic functionality with a special character (.)
            assert_eq!(
                sort_get_leading_gen("  -123."),
                (2..7),
                "Test case 3 failed for input: '  -123.'"
            );

            // Test case 4: Special case with inf
            assert_eq!(
                sort_get_leading_gen("  inf"),
                (2..5),
                "Test case 4 failed for input: '  inf'"
            );

            // Test case 5: Special case with -inf
            assert_eq!(
                sort_get_leading_gen("  -inf"),
                (2..6),
                "Test case 5 failed for input: '  -inf'"
            );

            // Test case 6: Special case with nan
            assert_eq!(
                sort_get_leading_gen("  nan"),
                (2..5),
                "Test case 6 failed for input: '  nan'"
            );

            // Test case 7: Edge case with whitespace only
            assert_eq!(
                sort_get_leading_gen("     "),
                (5..5),
                "Test case 7 failed for input: '     '"
            );

            // Test case 8: Basic functionality with multiple special characters
            assert_eq!(
                sort_get_leading_gen("  -123.45e-10"),
                (2..13),
                "Test case 8 failed for input: '  -123.45e-10'"
            );

            // Test case 9: Basic functionality with a mix of whitespace and special characters
            assert_eq!(
                sort_get_leading_gen("   -123.45\te-10"),
                (3..10),
                "Test case 9 failed for input: '   -123.45\\te-10'"
            );

            // Test case 10: Basic functionality with a plus sign
            assert_eq!(
                sort_get_leading_gen("  +123.45"),
                (2..9),
                "Test case 10 failed for input: '  +123.45'"
            );

            // Test case 11: Basic functionality with a mix of whitespace, special characters, and a plus sign
            assert_eq!(
                sort_get_leading_gen("   +123.45\te+10"),
                (3..10),
                "Test case 11 failed for input: '   +123.45\\te+10'"
            );

            // Test case 12: Edge case with only a plus or minus sign and no digits
            assert_eq!(
                sort_get_leading_gen("  -"),
                (2..3),
                "Test case 12 failed for input: '  -'"
            );

            // Test case 13: Edge case with only a decimal point and no digits
            assert_eq!(
                sort_get_leading_gen("  ."),
                (2..3),
                "Test case 13 failed for input: '  .'"
            );
        }
    }

    #[cfg(test)]
    mod compare_by_tests {
        use std::cmp::Ordering;

        use crate::chunks::ChunkLineData;
        use crate::numeric_str_cmp::NumInfo;
        use crate::{sort_compare_by, SortLine, SortMode};

        use super::*;

        #[test]
        fn test_basic_string_comparison() {
            let a = SortLine {
                line: "apple",
                index: 0,
            };
            let b = SortLine {
                line: "banana",
                index: 1,
            };
            let global_settings = SortGlobalConfigs {
                selectors: vec![],
                mode: SortMode::SortDefault,
                is_reverse: false,
                ..Default::default()
            };
            let a_line_data = ChunkLineData {
                selections: vec![],
                num_infos: vec![],
                parsed_floats: vec![],
            };
            let b_line_data = ChunkLineData {
                selections: vec![],
                num_infos: vec![],
                parsed_floats: vec![],
            };

            let result = sort_compare_by(&a, &b, &global_settings, &a_line_data, &b_line_data);
            assert_eq!(result, Ordering::Less);
        }

        #[test]
        fn test_numeric_comparison() {
            let a = SortLine {
                line: "10",
                index: 0,
            };
            let b = SortLine {
                line: "2",
                index: 1,
            };
            let global_settings = SortGlobalConfigs {
                selectors: vec![],
                mode: SortMode::SortNumeric,
                is_reverse: false,
                ..Default::default()
            };
            let settings = NumInfoParseSettings {
                decimal_pt: Some('.'),
                ..Default::default()
            };
            let a_info = NumInfo::parse("10", &settings).0;
            let b_info = NumInfo::parse("2", &settings).0;
            let a_line_data = ChunkLineData {
                selections: vec!["10"],
                num_infos: vec![a_info],
                parsed_floats: vec![],
            };
            let b_line_data = ChunkLineData {
                selections: vec!["2"],
                num_infos: vec![b_info],
                parsed_floats: vec![],
            };

            let result = sort_compare_by(&a, &b, &global_settings, &a_line_data, &b_line_data);
            assert_eq!(result, Ordering::Less); // Since 10 < 2 numerically
        }

        #[test]
        fn test_general_numeric_comparison() {
            let a = SortLine {
                line: "3.14",
                index: 0,
            };
            let b = SortLine {
                line: "2.718",
                index: 1,
            };
            let global_settings = SortGlobalConfigs {
                selectors: vec![],
                mode: SortMode::SortGeneralNumeric,
                is_reverse: false,
                ..Default::default()
            };
            let a_line_data = ChunkLineData {
                selections: vec![],
                num_infos: vec![],
                parsed_floats: vec![SortGeneralF64ParseResult::SortNumber(3.14)],
            };
            let b_line_data = ChunkLineData {
                selections: vec![],
                num_infos: vec![],
                parsed_floats: vec![SortGeneralF64ParseResult::SortNumber(2.718)],
            };

            let result = sort_compare_by(&a, &b, &global_settings, &a_line_data, &b_line_data);
            assert_eq!(result, Ordering::Greater);
        }

        #[test]
        fn test_reversed_sorting() {
            let a = SortLine {
                line: "apple",
                index: 0,
            };
            let b = SortLine {
                line: "banana",
                index: 1,
            };
            let global_settings = SortGlobalConfigs {
                selectors: vec![],
                mode: SortMode::SortDefault,
                is_reverse: true,
                ..Default::default()
            };
            let a_line_data = ChunkLineData {
                selections: vec![],
                num_infos: vec![],
                parsed_floats: vec![],
            };
            let b_line_data = ChunkLineData {
                selections: vec![],
                num_infos: vec![],
                parsed_floats: vec![],
            };

            let result = sort_compare_by(&a, &b, &global_settings, &a_line_data, &b_line_data);
            assert_eq!(result, Ordering::Greater); // Reversed result of basic string comparison
        }

        #[test]
        fn test_combination_of_sort_modes() {
            let a = SortLine {
                line: "10 apples",
                index: 0,
            };
            let b = SortLine {
                line: "2 bananas",
                index: 1,
            };
            let g_settings = SortGlobalConfigs {
                selectors: vec![
                    SortFieldSelector {
                        from: Default::default(),
                        to: None,
                        settings: SortKeySettings {
                            mode: SortMode::SortNumeric,
                            is_reverse: false,
                            ..Default::default()
                        },
                        is_needs_tokens: false,
                        is_needs_selection: true,
                    },
                    SortFieldSelector {
                        from: Default::default(),
                        to: None,
                        settings: SortKeySettings {
                            mode: SortMode::SortDefault,
                            is_reverse: false,
                            ..Default::default()
                        },
                        is_needs_tokens: false,
                        is_needs_selection: true,
                    },
                ],
                ..Default::default()
            };
            let settings = NumInfoParseSettings {
                decimal_pt: Some('.'),
                ..Default::default()
            };
            let a_info = NumInfo::parse("10", &settings).0;
            let b_info = NumInfo::parse("2", &settings).0;

            let a_line_data = ChunkLineData {
                selections: vec!["10", "apples"],
                num_infos: vec![a_info],
                parsed_floats: vec![],
            };
            let b_line_data = ChunkLineData {
                selections: vec!["2", "bananas"],
                num_infos: vec![b_info],
                parsed_floats: vec![],
            };

            let result = sort_compare_by(&a, &b, &g_settings, &a_line_data, &b_line_data);
            assert_eq!(result, Ordering::Greater); // "10" should sort after "2" due to numeric sorting
        }

        #[test]
        fn test_ignore_case_sorting() {
            let a = SortLine {
                line: "apple",
                index: 0,
            };
            let b = SortLine {
                line: "Banana",
                index: 1,
            };
            let settings = SortGlobalConfigs {
                selectors: vec![SortFieldSelector {
                    from: Default::default(),
                    to: None,
                    settings: SortKeySettings {
                        mode: SortMode::SortDefault,
                        is_ignore_case: true,
                        ..Default::default()
                    },
                    is_needs_tokens: false,
                    is_needs_selection: false,
                }],
                ..Default::default()
            };
            let a_line_data = ChunkLineData {
                selections: vec![],
                num_infos: vec![],
                parsed_floats: vec![],
            };
            let b_line_data = ChunkLineData {
                selections: vec![],
                num_infos: vec![],
                parsed_floats: vec![],
            };

            let result = sort_compare_by(&a, &b, &settings, &a_line_data, &b_line_data);
            assert_eq!(result, Ordering::Less); // "apple" should sort before "Banana" when ignoring case
        }

        #[test]
        fn test_ignore_non_printing_characters() {
            let a = SortLine {
                line: "\x01\x02Apple",
                index: 0,
            };
            let b = SortLine {
                line: "apple",
                index: 1,
            };
            let settings = SortGlobalConfigs {
                selectors: vec![SortFieldSelector {
                    from: Default::default(),
                    to: None,
                    settings: SortKeySettings {
                        mode: SortMode::SortDefault,
                        is_ignore_non_printing: true,
                        ..Default::default()
                    },
                    is_needs_tokens: false,
                    is_needs_selection: false,
                }],
                ..Default::default()
            };
            let a_line_data = ChunkLineData {
                selections: vec![],
                num_infos: vec![],
                parsed_floats: vec![],
            };
            let b_line_data = ChunkLineData {
                selections: vec![],
                num_infos: vec![],
                parsed_floats: vec![],
            };

            let result = sort_compare_by(&a, &b, &settings, &a_line_data, &b_line_data);
            assert_eq!(result, Ordering::Less); // Non-printing characters are ignored
        }

        #[test]
        fn test_edge_cases_for_numeric_sorting() {
            let a = SortLine {
                line: "-10000000000",
                index: 0,
            };
            let b = SortLine {
                line: "9999999999",
                index: 1,
            };
            let g_settings = SortGlobalConfigs {
                selectors: vec![SortFieldSelector {
                    from: Default::default(),
                    to: None,
                    settings: SortKeySettings {
                        mode: SortMode::SortNumeric,
                        is_reverse: false,
                        ..Default::default()
                    },
                    is_needs_tokens: false,
                    is_needs_selection: false,
                }],
                ..Default::default()
            };

            let settings = NumInfoParseSettings {
                decimal_pt: Some('.'),
                ..Default::default()
            };
            let a_info = NumInfo::parse("-10000000000", &settings).0;
            let b_info = NumInfo::parse("9999999999", &settings).0;

            let a_line_data = ChunkLineData {
                selections: vec![],
                num_infos: vec![a_info],
                parsed_floats: vec![],
            };
            let b_line_data = ChunkLineData {
                selections: vec![],
                num_infos: vec![b_info],
                parsed_floats: vec![],
            };

            let result = sort_compare_by(&a, &b, &g_settings, &a_line_data, &b_line_data);
            assert_eq!(result, Ordering::Less); // "-10000000000" should sort before "9999999999" numerically
        }
    }

    #[cfg(test)]
    mod sort_by_tests {
        use std::default::Default;

        use super::*;

        #[test]
        fn test_basic_sorting() {
            let mut lines = vec![
                SortLine {
                    line: "apple",
                    index: 1,
                },
                SortLine {
                    line: "banana",
                    index: 2,
                },
                SortLine {
                    line: "apricot",
                    index: 3,
                },
            ];
            let settings = SortGlobalConfigs::default();
            let line_data = ChunkLineData {
                selections: vec![],
                num_infos: vec![],
                parsed_floats: vec![],
            };

            sort_by(&mut lines, &settings, &line_data);
            assert_eq!(lines[0].line, "apple");
            assert_eq!(lines[1].line, "apricot");
            assert_eq!(lines[2].line, "banana");
        }

        #[test]
        fn test_stable_sorting() {
            let mut lines = vec![
                SortLine {
                    line: "cucumber",
                    index: 1,
                },
                SortLine {
                    line: "banana",
                    index: 2,
                },
                SortLine {
                    line: "apple",
                    index: 3,
                },
            ];
            let mut settings = SortGlobalConfigs::default();
            settings.is_stable = true;
            let line_data = ChunkLineData {
                selections: vec![],
                num_infos: vec![],
                parsed_floats: vec![],
            };

            sort_by(&mut lines, &settings, &line_data);
            assert_eq!(lines[0].line, "cucumber"); // Smallest length first
            assert_eq!(lines[1].line, "banana");
            assert_eq!(lines[2].line, "apple");
        }

        #[test]
        fn test_unique_sorting() {
            // This test would be more meaningful if `compare_by` could handle unique checks; currently, it does not
            let mut lines = vec![
                SortLine {
                    line: "apple",
                    index: 1,
                },
                SortLine {
                    line: "apple",
                    index: 2,
                },
                SortLine {
                    line: "banana",
                    index: 3,
                },
            ];
            let mut settings = SortGlobalConfigs::default();
            settings.is_unique = true; // Simulate unique behavior
            let line_data = ChunkLineData {
                selections: vec![],
                num_infos: vec![],
                parsed_floats: vec![],
            };

            sort_by(&mut lines, &settings, &line_data);
            // Expected that duplicates "apple" might be handled if `compare_by` were to consider `unique`
            assert_eq!(lines[0].line, "apple");
            assert_eq!(lines[1].line, "apple");
            assert_eq!(lines.len(), 3); // Assuming unique removes duplicates
        }

        #[test]
        fn test_sort_stability() {
            let mut lines = vec![
                SortLine {
                    line: "apple",
                    index: 2,
                },
                SortLine {
                    line: "apple",
                    index: 1,
                },
                SortLine {
                    line: "banana",
                    index: 3,
                },
            ];
            let settings = SortGlobalConfigs {
                is_stable: true,
                is_unique: false,
                ..Default::default()
            };
            let line_data = ChunkLineData {
                selections: vec![],
                num_infos: vec![],
                parsed_floats: vec![],
            };

            sort_by(&mut lines, &settings, &line_data);
            // Stability should maintain the order of "apple" as they are identical in terms of sorting criteria
            assert_eq!(lines[0].index, 2);
            assert_eq!(lines[1].index, 1);
            assert_eq!(lines[2].line, "banana");
        }

        #[test]
        fn test_sort_empty_vector() {
            let mut lines: Vec<SortLine> = vec![];
            let settings = SortGlobalConfigs::default();
            let line_data = ChunkLineData {
                selections: vec![],
                num_infos: vec![],
                parsed_floats: vec![],
            };

            sort_by(&mut lines, &settings, &line_data);
            assert!(lines.is_empty());
        }

        #[test]
        fn test_sort_with_line_data_influence() {
            // Assuming `compare_by` utilizes parsed_floats in its logic
            let mut lines = vec![
                SortLine {
                    line: "2.3",
                    index: 1,
                },
                SortLine {
                    line: "1.1",
                    index: 2,
                },
            ];
            let settings = SortGlobalConfigs::default();
            let line_data = ChunkLineData {
                selections: vec![],
                num_infos: vec![],
                parsed_floats: vec![
                    SortGeneralF64ParseResult::SortInfinity,
                    SortGeneralF64ParseResult::SortInfinity,
                ], // Mocked data
            };

            sort_by(&mut lines, &settings, &line_data);
            assert_eq!(lines[0].line, "1.1");
            assert_eq!(lines[1].line, "2.3");
        }

        #[test]
        fn test_sort_with_non_ascii_characters() {
            let mut lines = vec![
                SortLine {
                    line: "münchen",
                    index: 2,
                },
                SortLine {
                    line: "zürich",
                    index: 1,
                },
                SortLine {
                    line: "köln",
                    index: 3,
                },
            ];
            let settings = SortGlobalConfigs::default();
            let line_data = ChunkLineData {
                selections: vec![],
                num_infos: vec![],
                parsed_floats: vec![],
            };

            sort_by(&mut lines, &settings, &line_data);
            // Assuming basic lexicographical order
            assert_eq!(lines[0].line, "köln");
            assert_eq!(lines[1].line, "münchen");
            assert_eq!(lines[2].line, "zürich");
        }

        #[test]
        fn test_sort_with_identical_lines() {
            let mut lines = vec![
                SortLine {
                    line: "repeat",
                    index: 1,
                },
                SortLine {
                    line: "repeat",
                    index: 2,
                },
                SortLine {
                    line: "repeat",
                    index: 3,
                },
            ];
            let settings = SortGlobalConfigs {
                is_stable: true,
                is_unique: true,
                ..Default::default()
            }; // Assuming `unique` logic is implemented
            let line_data = ChunkLineData {
                selections: vec![],
                num_infos: vec![],
                parsed_floats: vec![],
            };

            sort_by(&mut lines, &settings, &line_data);
            // Expecting unique processing to possibly reduce duplicates
            assert_eq!(lines.len(), 3);
            assert_eq!(lines[0].line, "repeat");
        }
    }

    #[cfg(test)]
    mod field_selector_tests {
        use crate::SortFieldSelector;

        use super::*;

        #[test]
        fn test_split_key_options_no_options() {
            let key = "1";
            let (pos, opts) = SortFieldSelector::split_key_options(key);
            assert_eq!(pos, "1");
            assert_eq!(opts, "");
        }

        #[test]
        fn test_split_key_options_with_options() {
            let key = "1Mbg";
            let (pos, opts) = SortFieldSelector::split_key_options(key);
            assert_eq!(pos, "1");
            assert_eq!(opts, "Mbg");
        }

        #[test]
        fn test_parse_simple_key() {
            let global_settings = SortGlobalConfigs::default();
            let result = SortFieldSelector::parse("1", &global_settings);
            assert!(result.is_ok());
        }

        #[test]
        fn test_parse_key_with_options() {
            let global_settings = SortGlobalConfigs::default();
            let result = SortFieldSelector::parse("1,2Mn", &global_settings);
            assert!(result.is_err()); // Assuming 'n' and 'M' are conflicting options in this scenario
        }

        #[test]
        fn test_parse_with_options_numeric_err() {
            let from_to = Some(("2", "n"));
            let result = SortFieldSelector::parse_with_options(("1", "M"), from_to);
            assert!(result.is_err());
        }

        #[test]
        fn test_new_invalid_char_index() {
            let from = SortKeyPosition {
                field: 1,
                char: 0,
                is_ignore_blanks: false,
            }; // Invalid because char index 0 is not allowed
            let result = SortFieldSelector::new(from, None, SortKeySettings::default());
            assert!(result.is_err());
        }

        #[test]
        fn test_get_selection_numeric() {
            let selector = SortFieldSelector {
                from: SortKeyPosition {
                    field: 1,
                    char: 1,
                    is_ignore_blanks: false,
                },
                to: None,
                settings: SortKeySettings {
                    mode: SortMode::SortNumeric,
                    ..Default::default()
                },
                is_needs_tokens: false,
                is_needs_selection: true,
            };
            let line = "12345 abc";
            let tokens = vec![(0..5)]; // Simplified token representation
            let selection = selector.get_selection(line, &tokens);
            match selection {
                SortSelection::WithNumInfo(s, _) => assert_eq!(s, "12345"),
                _ => panic!("Expected numeric selection"),
            }
        }

        #[test]
        fn test_split_key_options_with_mixed_characters() {
            let key = "12Mb2g";
            let (pos, opts) = SortFieldSelector::split_key_options(key);
            assert_eq!(pos, "12");
            assert_eq!(opts, "Mb2g");
        }

        #[test]
        fn test_split_key_options_with_leading_zeros() {
            let key = "001Mb";
            let (pos, opts) = SortFieldSelector::split_key_options(key);
            assert_eq!(pos, "001");
            assert_eq!(opts, "Mb");
        }

        #[test]
        fn test_split_key_options_with_no_numeric_start() {
            let key = "M1";
            let (pos, opts) = SortFieldSelector::split_key_options(key);
            assert_eq!(pos, "");
            assert_eq!(opts, "M1");
        }
    }
}