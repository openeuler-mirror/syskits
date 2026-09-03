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

// spell-checker:ignore (ToDOs) corasick memchr Roff trunc oset iset CHARCLASS

//! PTX (Permuted Index) 实现
//!
//! 该模块实现了类似于 GNU PTX 的排列索引功能。它可以从输入文件中提取关键词，
//! 并生成一个排序的索引，每个关键词都显示在其上下文中。
//!
//! 主要功能:
//! - 从文件或标准输入读取文本
//! - 提取和过滤关键词
//! - 生成格式化的输出(支持 Roff 和 TeX 格式)
//! - 提供引用和上下文显示

extern crate rust_i18n;
use clap::{Arg, ArgAction, Command, crate_version, error::ErrorKind};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTError, CTResult, CtSimpleError, FromIo};
use regex::Regex;
use std::collections::{BTreeSet, HashSet};
use std::error::Error;
use std::ffi::OsString;
use std::fmt::{Display, Formatter, Write as FmtWrite};
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read, Write, stdout};
use std::num::ParseIntError;
use std::process::{Command as ProcessCommand, Stdio};
use sys_locale::get_locale;

const REGEX_CHARCLASS: &str = "^-]\\";
const GNU_DEFAULT_CONTEXT_REGEX: &str = r#"(?m)[.?!][\]\"')}]*($|\t|  )[ \t\n]*"#;

#[derive(Debug)]
enum OutFormat {
    Dumb,
    Roff,
    Tex,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtxSemanticRow {
    pub row_index: usize,
    pub keyword: String,
    pub before: String,
    pub after: String,
    pub head: String,
    pub tail: String,
    pub reference: String,
    pub file: String,
    pub line_index: usize,
    pub global_line_index: usize,
    pub rendered_text: String,
    pub format: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtxSemantic {
    pub rows: Vec<PtxSemanticRow>,
    pub classic_text: String,
    pub stderr_text: String,
    pub exit_code: i32,
}

struct DirectPtxInvocation {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exit_code: i32,
}

#[derive(Debug)]
struct PtxConfig {
    /// 是否启用 GNU 扩展
    is_gnu_ext: bool,
    /// 是否忽略大小写
    is_ignore_case: bool,
    /// 是否自动生成引用
    is_auto_ref: bool,
    /// 是否使用输入引用
    is_input_ref: bool,
    /// 引用是否在右侧显示
    is_right_ref: bool,
    /// 输出格式(Roff 或 TeX)
    format: OutFormat,
    /// 行宽度
    line_width: usize,
    /// 间隔大小
    gap_size: usize,
    /// 截断标记字符串
    trunc_str: String,
    /// 宏名称
    macro_name: String,
    /// 上下文正则表达式
    context_regex: String,
}

impl Default for PtxConfig {
    fn default() -> Self {
        Self {
            format: OutFormat::Dumb,
            is_gnu_ext: true,
            is_auto_ref: false,
            is_input_ref: false,
            is_right_ref: false,
            is_ignore_case: false,
            macro_name: "xx".to_owned(),
            trunc_str: "/".to_owned(),
            context_regex: GNU_DEFAULT_CONTEXT_REGEX.to_owned(),
            line_width: 72,
            gap_size: 3,
        }
    }
}

fn read_word_filter_file(
    matches: &clap::ArgMatches,
    option: &str,
) -> std::io::Result<HashSet<String>> {
    let filename = matches
        .get_one::<String>(option)
        .expect("parsing options failed!")
        .to_string();
    let file = File::open(filename)?;
    let reader = BufReader::new(file);
    let mut words: HashSet<String> = HashSet::new();
    for word in reader.lines() {
        words.insert(word?);
    }
    Ok(words)
}

/// reads contents of file as unique set of characters to be used with the break-file option
fn read_char_filter_file(
    matches: &clap::ArgMatches,
    option: &str,
) -> std::io::Result<HashSet<char>> {
    let filename = matches
        .get_one::<String>(option)
        .expect("parsing options failed!");
    let mut reader = File::open(filename)?;
    let mut buffer = String::new();
    reader.read_to_string(&mut buffer)?;
    Ok(buffer.chars().collect())
}

#[derive(Debug)]
struct WordFilter {
    /// 是否只包含指定的单词
    is_only_specified: bool,
    /// 是否忽略指定的单词
    is_ignore_specified: bool,
    /// 要包含的单词集合
    only_set: HashSet<String>,
    /// 要忽略的单词集合
    ignore_set: HashSet<String>,
    /// 用于匹配单词的正则表达式
    word_regex: String,
}

impl WordFilter {
    #[allow(clippy::cognitive_complexity)]
    fn new(matches: &clap::ArgMatches, config: &PtxConfig) -> CTResult<Self> {
        let (o, oset): (bool, HashSet<String>) = if matches.contains_id(ptx_options::PTX_ONLY_FILE)
        {
            let words = read_word_filter_file(matches, ptx_options::PTX_ONLY_FILE)
                .map_err_context(String::new)?;
            (true, words)
        } else {
            (false, HashSet::new())
        };
        let (i, iset): (bool, HashSet<String>) =
            if matches.contains_id(ptx_options::PTX_IGNORE_FILE) {
                let words = read_word_filter_file(matches, ptx_options::PTX_IGNORE_FILE)
                    .map_err_context(String::new)?;
                (true, words)
            } else {
                (false, HashSet::new())
            };
        let break_set: Option<HashSet<char>> = if matches.contains_id(ptx_options::PTX_BREAK_FILE)
            && !matches.contains_id(ptx_options::PTX_WORD_REGEXP)
        {
            let chars = read_char_filter_file(matches, ptx_options::PTX_BREAK_FILE)
                .map_err_context(String::new)?;
            let mut hs: HashSet<char> = if config.is_gnu_ext {
                HashSet::new() // really only chars found in file
            } else {
                // GNU off means at least these are considered
                [' ', '\t', '\n'].iter().cloned().collect()
            };
            hs.extend(chars);
            Some(hs)
        } else {
            // if -W takes precedence or default
            None
        };
        // Ignore empty string regex from cmd-line-args
        let arg_reg: Option<String> = if matches.contains_id(ptx_options::PTX_WORD_REGEXP) {
            match matches.get_one::<String>(ptx_options::PTX_WORD_REGEXP) {
                Some(v) => {
                    if v.is_empty() {
                        None
                    } else {
                        Some(v.to_string())
                    }
                }
                None => None,
            }
        } else {
            None
        };
        let reg = match arg_reg {
            Some(arg_reg) => arg_reg,
            None => {
                if break_set.is_some() {
                    format!(
                        "[^{}]+",
                        break_set
                            .unwrap()
                            .into_iter()
                            .map(|c| if REGEX_CHARCLASS.contains(c) {
                                format!("\\{c}")
                            } else {
                                c.to_string()
                            })
                            .collect::<String>()
                    )
                } else if config.is_gnu_ext {
                    "[A-Za-z]+".to_owned()
                } else {
                    "[^ \t\n]+".to_owned()
                }
            }
        };
        Ok(Self {
            is_only_specified: o,
            is_ignore_specified: i,
            only_set: oset,
            ignore_set: iset,
            word_regex: reg,
        })
    }
}

impl Default for WordFilter {
    fn default() -> Self {
        Self {
            is_only_specified: false,
            is_ignore_specified: false,
            only_set: HashSet::new(),
            ignore_set: HashSet::new(),
            word_regex: "[A-Za-z]+".to_string(),
        }
    }
}

/// 单词引用
///
/// 记录单词在文本中的位置和上下文信息
#[derive(Debug, PartialOrd, PartialEq, Eq, Ord, Default)]
struct WordRef {
    /// 单词本身
    word: String,
    /// 在所有文件中的行号
    global_line_nr: usize,
    /// 在当前文件中的行号
    local_line_nr: usize,
    /// 单词在行中的起始位置
    position: usize,
    /// 单词在行中的结束位置
    position_end: usize,
    /// 单词在完整文件文本中的起始位置
    global_position: usize,
    /// 单词在完整文件文本中的结束位置
    global_position_end: usize,
    /// 当前上下文在完整文件文本中的起始位置
    context_start: usize,
    /// 当前上下文在完整文件文本中的结束位置
    context_end: usize,
    /// 单词在完整文件字符数组中的起始位置
    global_char_position: usize,
    /// 单词在完整文件字符数组中的结束位置
    global_char_position_end: usize,
    /// 当前上下文在完整文件字符数组中的起始位置
    context_char_start: usize,
    /// 当前上下文在完整文件字符数组中的结束位置
    context_char_end: usize,
    file_index: usize,
}

#[derive(Debug)]
enum PtxError {
    ParseError(ParseIntError),
}

impl Error for PtxError {}
impl CTError for PtxError {}

impl Display for PtxError {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        match self {
            Self::ParseError(e) => e.fmt(f),
        }
    }
}

fn get_config(matches: &clap::ArgMatches) -> CTResult<PtxConfig> {
    let mut config = PtxConfig::default();
    let err_msg = "parsing options failed";
    if matches.get_flag(ptx_options::PTX_TRADITIONAL) {
        config.is_gnu_ext = false;
        config.format = OutFormat::Roff;
        "[^ \t\n]+".clone_into(&mut config.context_regex);
    }
    if let Some(reg) = matches.get_one::<String>(ptx_options::PTX_SENTENCE_REGEXP) {
        config.context_regex = reg.to_string();
        // Note: Zero-length regex check is deferred to actual usage time
        // to match GNU ptx behavior (only errors when processing non-empty content)
    }
    config.is_auto_ref = matches.get_flag(ptx_options::PTX_AUTO_REFERENCE);
    config.is_input_ref = matches.get_flag(ptx_options::PTX_REFERENCES);
    if config.is_input_ref && !matches.contains_id(ptx_options::PTX_SENTENCE_REGEXP) {
        config.context_regex = "\n".to_string();
    }
    config.is_right_ref = matches.get_flag(ptx_options::PTX_RIGHT_SIDE_REFS);
    config.is_ignore_case = matches.get_flag(ptx_options::PTX_IGNORE_CASE);
    if matches.contains_id(ptx_options::PTX_MACRO_NAME) {
        config.macro_name = matches
            .get_one::<String>(ptx_options::PTX_MACRO_NAME)
            .expect(err_msg)
            .to_string();
    }
    if matches.contains_id(ptx_options::PTX_FLAG_TRUNCATION) {
        config.trunc_str = matches
            .get_one::<String>(ptx_options::PTX_FLAG_TRUNCATION)
            .expect(err_msg)
            .to_string();
    }
    if matches.contains_id(ptx_options::PTX_WIDTH) {
        let width: usize = matches
            .get_one::<String>(ptx_options::PTX_WIDTH)
            .expect(err_msg)
            .parse()
            .map_err(PtxError::ParseError)?;
        if width == 0 {
            return Err(CtSimpleError::new(1, "invalid line width: '0'"));
        }
        config.line_width = width;
    }
    if matches.contains_id(ptx_options::PTX_GAP_SIZE) {
        let gap: usize = matches
            .get_one::<String>(ptx_options::PTX_GAP_SIZE)
            .expect(err_msg)
            .parse()
            .map_err(PtxError::ParseError)?;
        if gap == 0 {
            return Err(CtSimpleError::new(1, "invalid gap width: '0'"));
        }
        config.gap_size = gap;
    }
    if let Some(fmt) = matches.get_one::<String>(ptx_options::PTX_FORMAT) {
        config.format = match fmt.as_str() {
            "roff" => OutFormat::Roff,
            "tex" => OutFormat::Tex,
            _ => config.format,
        };
    }
    if matches.get_flag(ptx_options::PTX_FORMAT_ROFF) {
        config.format = OutFormat::Roff;
    }
    if matches.get_flag(ptx_options::PTX_FORMAT_TEX) {
        config.format = OutFormat::Tex;
    }
    Ok(config)
}

fn regex_matches_zero_len(pattern: &str) -> bool {
    Regex::new(pattern)
        .ok()
        .and_then(|re| re.find(""))
        .is_some_and(|m| m.start() == m.end())
}

fn compile_regex_lossy(pattern: &str) -> Regex {
    if let Ok(re) = Regex::new(pattern) {
        return re;
    }

    if pattern.ends_with('\\') {
        let mut fixed = pattern.to_owned();
        fixed.push('\\');
        if let Ok(re) = Regex::new(&fixed) {
            return re;
        }
    }

    Regex::new(r"$^").expect("fallback regex must be valid")
}

/// 文件内容
///
/// 存储文件的行内容和字符级表示
#[derive(Debug)]
struct FileContent {
    /// 文件名 (从 Map 键移入内部)
    filename: String,
    /// 文件完整文本，物理行之间保留 '\n'，用于 GNU 默认跨行上下文处理。
    text: String,
    chars_text: Vec<char>,
    byte_to_char: Vec<usize>,
    /// 每个物理行在完整文本中的起始字节偏移。
    line_starts: Vec<usize>,
    /// 文件的所有行
    lines: Vec<String>,
    /// 每行的字符数组表示，用于快速索引
    chars_lines: Vec<Vec<char>>,
    /// 在所有文件中的行偏移量
    offset: usize,
}

type FileMap = Vec<FileContent>;

fn build_byte_to_char_map(text: &str) -> Vec<usize> {
    let mut map = vec![0; text.len() + 1];
    let mut char_index = 0usize;
    for (byte_index, ch) in text.char_indices() {
        for slot in &mut map[byte_index..byte_index + ch.len_utf8()] {
            *slot = char_index;
        }
        char_index += 1;
        map[byte_index + ch.len_utf8()] = char_index;
    }
    map[text.len()] = char_index;
    map
}

fn line_index_for_offset(line_starts: &[usize], offset: usize) -> usize {
    match line_starts.binary_search(&offset) {
        Ok(index) => index,
        Err(0) => 0,
        Err(index) => index - 1,
    }
}

fn next_context_end(context_reg: &Regex, text: &str, start: usize) -> usize {
    match context_reg.find_at(text, start) {
        Some(m) if m.end() > start => m.end(),
        _ => text.len(),
    }
}

fn trim_context_end(text: &str, start: usize, end: usize) -> usize {
    let mut trimmed = end;
    while trimmed > start {
        let prefix = &text[start..trimmed];
        match prefix.chars().next_back() {
            Some(ch) if ch.is_whitespace() => trimmed -= ch.len_utf8(),
            _ => break,
        }
    }
    trimmed
}

fn ptx_input_reference_span(line: &str) -> Option<(usize, usize)> {
    let Some(first) = line.chars().next() else {
        return None;
    };
    if first.is_whitespace() {
        return None;
    }

    let end = line
        .char_indices()
        .find_map(|(idx, ch)| ch.is_whitespace().then_some(idx))
        .unwrap_or(line.len());

    Some((0, end))
}

fn ptx_input_reference_text(line: &str) -> &str {
    match ptx_input_reference_span(line) {
        Some((start, end)) => &line[start..end],
        None => "",
    }
}

fn ptx_input_reference_content_start(line: &str) -> usize {
    let Some((_, end)) = ptx_input_reference_span(line) else {
        return 0;
    };

    let mut start = end;
    for (idx, ch) in line[end..].char_indices() {
        if !ch.is_whitespace() {
            start = end + idx;
            break;
        }
        start = end + idx + ch.len_utf8();
    }

    start
}

/// 从输入文件读取内容并构建文件映射
///
/// # 参数
/// * `input_files` - 输入文件路径列表
/// * `config` - PTX 配置，控制是否启用 GNU 扩展
///
/// # 返回值
/// 返回一个 HashMap，键为文件名，值为文件内容和偏移量
fn ptx_read_input(input_files: &[String], config: &PtxConfig) -> std::io::Result<FileMap> {
    // 初始化文件数组
    let mut file_map: FileMap = Vec::new();
    let mut files = Vec::new();

    if input_files.is_empty() {
        files.push("-");
    } else if config.is_gnu_ext {
        files.extend(input_files.iter().map(|s| s.as_str()));
    } else {
        files.push(&input_files[0]);
    }

    let mut offset: usize = 0;
    for filename in files {
        let reader: BufReader<Box<dyn Read>> = BufReader::new(if filename == "-" {
            ctcore::ct_io::stdin_reader_box()
        } else {
            Box::new(File::open(filename)?)
        });

        let lines: Vec<String> = reader.lines().collect::<std::io::Result<Vec<String>>>()?;
        let mut text = String::new();
        let mut line_starts = Vec::with_capacity(lines.len());
        for (index, line) in lines.iter().enumerate() {
            if index > 0 {
                text.push('\n');
            }
            line_starts.push(text.len());
            text.push_str(line);
        }
        let chars_text: Vec<char> = text.chars().collect();
        let byte_to_char = build_byte_to_char_map(&text);
        let chars_lines: Vec<Vec<char>> = lines.iter().map(|x| x.chars().collect()).collect();

        let size = lines.len();
        // 直接 Push 到数组尾部
        file_map.push(FileContent {
            filename: filename.to_owned(),
            text,
            chars_text,
            byte_to_char,
            line_starts,
            lines,
            chars_lines,
            offset,
        });
        offset += size;
    }
    Ok(file_map)
}

/// 从文件内容中提取单词并创建单词引用集合
///
/// # 参数
/// * `config` - PTX 配置，控制大小写敏感性和引用处理
/// * `filter` - 单词过滤器，定义单词匹配和过滤规则
/// * `file_map` - 文件内容映射
///
/// # 返回值
/// 返回一个有序集合，包含所有匹配的单词引用
fn ptx_create_word_set(
    config: &PtxConfig,
    filter: &WordFilter,
    file_map: &FileMap,
) -> BTreeSet<WordRef> {
    let reg = compile_regex_lossy(&filter.word_regex);
    let ref_reg = compile_regex_lossy(&config.context_regex);
    let mut word_set: BTreeSet<WordRef> = BTreeSet::new();

    for (file_idx, content) in file_map.iter().enumerate() {
        let mut context_start = 0usize;
        while context_start < content.text.len() {
            let context_end_raw = next_context_end(&ref_reg, &content.text, context_start);
            let context_end = trim_context_end(&content.text, context_start, context_end_raw);
            let context_text = &content.text[context_start..context_end];

            for mat in reg.find_iter(context_text) {
                let (global_beg, global_end) =
                    (context_start + mat.start(), context_start + mat.end());
                let local_line_nr = line_index_for_offset(&content.line_starts, global_beg);
                let line_start = content.line_starts[local_line_nr];
                let line = &content.lines[local_line_nr];
                if config.is_input_ref {
                    let reference_content_start = ptx_input_reference_content_start(line);
                    if global_beg < line_start + reference_content_start {
                        continue;
                    }
                }

                let mut word = content.text[global_beg..global_end].to_owned();
                if filter.is_only_specified && !filter.only_set.contains(&word) {
                    continue;
                }
                if filter.is_ignore_specified && filter.ignore_set.contains(&word) {
                    continue;
                }
                if config.is_ignore_case {
                    word = word.to_lowercase();
                }

                let context_start = if config.is_input_ref {
                    context_start.max(line_start + ptx_input_reference_content_start(line))
                } else {
                    context_start
                };
                let global_char_position = content.byte_to_char[global_beg];
                let global_char_position_end = content.byte_to_char[global_end];
                let context_char_start = content.byte_to_char[context_start];
                let context_char_end = content.byte_to_char[context_end];
                word_set.insert(WordRef {
                    word,
                    file_index: file_idx,
                    global_line_nr: content.offset + local_line_nr,
                    local_line_nr,
                    position: global_beg - line_start,
                    position_end: global_end - line_start,
                    global_position: global_beg,
                    global_position_end: global_end,
                    context_start,
                    context_end,
                    global_char_position,
                    global_char_position_end,
                    context_char_start,
                    context_char_end,
                });
            }

            if context_end_raw <= context_start {
                break;
            }
            context_start = context_end_raw;
        }
    }
    word_set
}

/// 获取单词的引用字符串
///
/// # 参数
/// * `config` - PTX 配置，控制引用生成方式
/// * `word_ref` - 单词引用信息
/// * `line` - 包含单词的行文本
/// * `context_reg` - 上下文正则表达式
///
/// # 返回值
/// 返回生成的引用字符串
fn ptx_get_reference(
    config: &PtxConfig,
    word_ref: &WordRef,
    file_name: &str,
    line: &str,
    _context_reg: &Regex,
) -> String {
    if config.is_auto_ref {
        format!("{}:{}", file_name.maybe_quote(), word_ref.local_line_nr + 1)
    } else if config.is_input_ref {
        ptx_input_reference_text(line).to_string()
    } else {
        String::new()
    }
}

fn run_ptx_direct_process(
    argv: &[OsString],
    stdin_bytes: Option<&[u8]>,
) -> CTResult<DirectPtxInvocation> {
    let current_exe = std::env::current_exe()?;
    let mut command = ProcessCommand::new(current_exe);
    command
        .args(argv)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if stdin_bytes.is_some() {
        command.stdin(Stdio::piped());
    } else {
        command.stdin(Stdio::inherit());
    }

    let mut child = command.spawn()?;
    let mut stdin_write_error = None;
    if let Some(bytes) = stdin_bytes
        && let Some(mut stdin) = child.stdin.take()
        && let Err(err) = stdin.write_all(bytes)
        && err.kind() != std::io::ErrorKind::BrokenPipe
    {
        stdin_write_error = Some(err);
    }

    let output = child.wait_with_output()?;
    if let Some(err) = stdin_write_error {
        return Err(err.into());
    }

    Ok(DirectPtxInvocation {
        stdout: output.stdout,
        stderr: output.stderr,
        exit_code: output.status.code().unwrap_or(1),
    })
}

fn ptx_format_name(format: &OutFormat) -> &'static str {
    match format {
        OutFormat::Dumb => "dumb",
        OutFormat::Roff => "roff",
        OutFormat::Tex => "tex",
    }
}

#[derive(Debug, Clone, Default)]
struct PtxOutputFields {
    tail: String,
    before: String,
    keyafter: String,
    head: String,
    keyword_len: usize,
    tail_truncation: bool,
    before_truncation: bool,
    keyafter_truncation: bool,
    head_truncation: bool,
}

fn ptx_chars_to_string(chars: &[char], start: usize, end: usize) -> String {
    if start >= end || start >= chars.len() {
        return String::new();
    }
    chars[start..end.min(chars.len())].iter().collect()
}

fn ptx_skip_white(chars: &[char], mut cursor: usize, limit: usize) -> usize {
    while cursor < limit && chars[cursor].is_whitespace() {
        cursor += 1;
    }
    cursor
}

fn ptx_skip_white_backwards(chars: &[char], mut cursor: usize, start: usize) -> usize {
    while cursor > start && chars[cursor - 1].is_whitespace() {
        cursor -= 1;
    }
    cursor
}

fn ptx_is_default_word_char(config: &PtxConfig, c: char) -> bool {
    if config.is_gnu_ext {
        c.is_ascii_alphabetic()
    } else {
        !c.is_whitespace()
    }
}

fn ptx_skip_something(chars: &[char], cursor: usize, limit: usize, config: &PtxConfig) -> usize {
    if cursor >= limit {
        return cursor;
    }

    let mut next = cursor;
    if ptx_is_default_word_char(config, chars[next]) {
        while next < limit && ptx_is_default_word_char(config, chars[next]) {
            next += 1;
        }
    } else {
        next += 1;
    }
    next
}

fn ptx_field_dimensions(config: &PtxConfig, line_width: usize) -> (usize, usize, usize, usize) {
    let half_line_width = line_width / 2;
    let trunc_len = if config.trunc_str.is_empty() {
        0
    } else {
        config.trunc_str.chars().count()
    };

    let mut before_max_width = half_line_width.saturating_sub(config.gap_size);
    let mut keyafter_max_width = half_line_width;
    if trunc_len > 0 {
        if config.is_gnu_ext {
            before_max_width = before_max_width.saturating_sub(2 * trunc_len);
            keyafter_max_width = keyafter_max_width.saturating_sub(2 * trunc_len);
        } else {
            keyafter_max_width = keyafter_max_width.saturating_sub(2 * trunc_len + 1);
        }
    }

    (
        half_line_width,
        before_max_width,
        keyafter_max_width,
        trunc_len,
    )
}

fn ptx_content_maximum_word_length(content: &FileContent, config: &PtxConfig) -> usize {
    ptx_maximum_word_length_in_chars(&content.chars_text, config)
}

fn ptx_content_maximum_word_length_bytes(content: &FileContent, config: &PtxConfig) -> usize {
    ptx_maximum_word_length_in_bytes(content.text.as_bytes(), config)
}

fn ptx_maximum_word_length_in_chars(chars: &[char], config: &PtxConfig) -> usize {
    let mut max_len = 0usize;
    let mut cursor = 0usize;
    while cursor < chars.len() {
        if ptx_is_default_word_char(config, chars[cursor]) {
            let start = cursor;
            while cursor < chars.len() && ptx_is_default_word_char(config, chars[cursor]) {
                cursor += 1;
            }
            max_len = max_len.max(cursor - start);
        } else {
            cursor += 1;
        }
    }
    max_len
}

fn ptx_maximum_word_length_in_bytes(bytes: &[u8], config: &PtxConfig) -> usize {
    let mut max_len = 0usize;
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        if ptx_is_default_word_byte(config, bytes[cursor]) {
            let start = cursor;
            while cursor < bytes.len() && ptx_is_default_word_byte(config, bytes[cursor]) {
                cursor += 1;
            }
            max_len = max_len.max(cursor - start);
        } else {
            cursor += 1;
        }
    }
    max_len
}

fn ptx_define_output_fields_for_width(
    all_before: &[char],
    keyword: &str,
    all_after: &[char],
    config: &PtxConfig,
    line_width: usize,
    maximum_word_length: usize,
) -> PtxOutputFields {
    let (half_line_width, before_max_width, keyafter_max_width, _) =
        ptx_field_dimensions(config, line_width);
    let truncation_enabled = !config.trunc_str.is_empty();

    let mut chars =
        Vec::with_capacity(all_before.len() + keyword.chars().count() + all_after.len());
    chars.extend_from_slice(all_before);
    let keyword_chars: Vec<char> = keyword.chars().collect();
    chars.extend(keyword_chars.iter());
    chars.extend_from_slice(all_after);

    let context_start = 0usize;
    let context_end = chars.len();
    let key_start = all_before.len();
    let key_end = key_start + keyword_chars.len();
    let left_context_start = context_start;
    let right_context_end = context_end;
    let left_field_start = if key_start.saturating_sub(left_context_start)
        > half_line_width.saturating_add(maximum_word_length)
    {
        let jump = half_line_width.saturating_add(maximum_word_length);
        let mut start = key_start.saturating_sub(jump);
        start = ptx_skip_something(&chars, start, key_start, config);
        start
    } else {
        left_context_start
    };

    let keyafter_start = key_start;
    let mut keyafter_end = key_end;
    let mut cursor = keyafter_end;
    let keyafter_limit = keyafter_start.saturating_add(keyafter_max_width);
    while cursor < right_context_end && cursor <= keyafter_limit {
        keyafter_end = cursor;
        cursor = ptx_skip_something(&chars, cursor, right_context_end, config);
    }
    if cursor <= keyafter_limit {
        keyafter_end = cursor;
    }
    let mut keyafter_truncation = truncation_enabled && keyafter_end < right_context_end;
    keyafter_end = ptx_skip_white_backwards(&chars, keyafter_end, keyafter_start);

    let mut before_start = left_field_start;
    let mut before_end = keyafter_start;
    before_end = ptx_skip_white_backwards(&chars, before_end, before_start);
    while before_start.saturating_add(before_max_width) < before_end {
        let next = ptx_skip_something(&chars, before_start, before_end, config);
        if next <= before_start {
            break;
        }
        before_start = next;
    }

    let mut before_truncation = if truncation_enabled {
        ptx_skip_white_backwards(&chars, before_start, context_start) > left_context_start
    } else {
        false
    };
    before_start = ptx_skip_white(&chars, before_start, context_end);

    let before_len = before_end.saturating_sub(before_start);
    let tail_max_width = before_max_width
        .saturating_sub(before_len)
        .saturating_sub(config.gap_size);
    let (tail_start, tail_end, tail_truncation) = if tail_max_width > 0 {
        let tail_start = ptx_skip_white(&chars, keyafter_end, context_end);
        let mut tail_end = tail_start;
        let mut cursor = tail_end;
        let tail_limit = tail_start.saturating_add(tail_max_width);
        while cursor < right_context_end && cursor < tail_limit {
            tail_end = cursor;
            cursor = ptx_skip_something(&chars, cursor, right_context_end, config);
        }
        if cursor < tail_limit {
            tail_end = cursor;
        }

        let mut tail_truncation = false;
        if tail_end > tail_start {
            keyafter_truncation = false;
            tail_truncation = truncation_enabled && tail_end < right_context_end;
        }
        tail_end = ptx_skip_white_backwards(&chars, tail_end, tail_start);
        (tail_start, tail_end, tail_truncation)
    } else {
        (0, 0, false)
    };

    let keyafter_len = keyafter_end.saturating_sub(keyafter_start);
    let head_max_width = keyafter_max_width
        .saturating_sub(keyafter_len)
        .saturating_sub(config.gap_size);
    let (head_start, head_end, head_truncation) = if head_max_width > 0 {
        let head_end = ptx_skip_white_backwards(&chars, before_start, context_start);
        let mut head_start = left_field_start;
        while head_start.saturating_add(head_max_width) < head_end {
            let next = ptx_skip_something(&chars, head_start, head_end, config);
            if next <= head_start {
                break;
            }
            head_start = next;
        }

        let mut head_truncation = false;
        if head_end > head_start {
            before_truncation = false;
            head_truncation = truncation_enabled && head_start > left_context_start;
        }
        head_start = ptx_skip_white(&chars, head_start, head_end);
        (head_start, head_end, head_truncation)
    } else {
        (0, 0, false)
    };

    PtxOutputFields {
        tail: ptx_chars_to_string(&chars, tail_start, tail_end),
        before: ptx_chars_to_string(&chars, before_start, before_end),
        keyafter: ptx_chars_to_string(&chars, keyafter_start, keyafter_end),
        head: ptx_chars_to_string(&chars, head_start, head_end),
        keyword_len: keyword_chars.len(),
        tail_truncation,
        before_truncation,
        keyafter_truncation,
        head_truncation,
    }
}

#[derive(Debug, Clone, Default)]
struct PtxOutputFieldsBytes {
    tail: Vec<u8>,
    before: Vec<u8>,
    keyafter: Vec<u8>,
    head: Vec<u8>,
    tail_truncation: bool,
    before_truncation: bool,
    keyafter_truncation: bool,
    head_truncation: bool,
}

fn ptx_is_default_word_byte(config: &PtxConfig, byte: u8) -> bool {
    if config.is_gnu_ext {
        byte.is_ascii_alphabetic()
    } else {
        !byte.is_ascii_whitespace()
    }
}

fn ptx_skip_white_bytes(bytes: &[u8], mut cursor: usize, limit: usize) -> usize {
    while cursor < limit && bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    cursor
}

fn ptx_skip_white_backwards_bytes(bytes: &[u8], mut cursor: usize, start: usize) -> usize {
    while cursor > start && bytes[cursor - 1].is_ascii_whitespace() {
        cursor -= 1;
    }
    cursor
}

fn ptx_skip_something_bytes(
    bytes: &[u8],
    cursor: usize,
    limit: usize,
    config: &PtxConfig,
) -> usize {
    if cursor >= limit {
        return cursor;
    }

    let mut next = cursor;
    if ptx_is_default_word_byte(config, bytes[next]) {
        while next < limit && ptx_is_default_word_byte(config, bytes[next]) {
            next += 1;
        }
    } else {
        next += 1;
    }
    next
}

fn ptx_bytes_to_vec(bytes: &[u8], start: usize, end: usize) -> Vec<u8> {
    if start >= end || start >= bytes.len() {
        return Vec::new();
    }
    bytes[start..end.min(bytes.len())].to_vec()
}

fn ptx_define_output_fields_bytes_for_width(
    all_before: &[u8],
    keyword: &[u8],
    all_after: &[u8],
    config: &PtxConfig,
    line_width: usize,
    maximum_word_length: usize,
) -> PtxOutputFieldsBytes {
    let (half_line_width, before_max_width, keyafter_max_width, _) =
        ptx_field_dimensions(config, line_width);
    let truncation_enabled = !config.trunc_str.is_empty();

    let mut bytes = Vec::with_capacity(all_before.len() + keyword.len() + all_after.len());
    bytes.extend_from_slice(all_before);
    bytes.extend_from_slice(keyword);
    bytes.extend_from_slice(all_after);

    let context_start = 0usize;
    let context_end = bytes.len();
    let key_start = all_before.len();
    let key_end = key_start + keyword.len();
    let left_context_start = context_start;
    let right_context_end = context_end;
    let left_field_start = if key_start.saturating_sub(left_context_start)
        > half_line_width.saturating_add(maximum_word_length)
    {
        let jump = half_line_width.saturating_add(maximum_word_length);
        let mut start = key_start.saturating_sub(jump);
        start = ptx_skip_something_bytes(&bytes, start, key_start, config);
        start
    } else {
        left_context_start
    };

    let keyafter_start = key_start;
    let mut keyafter_end = key_end;
    let mut cursor = keyafter_end;
    let keyafter_limit = keyafter_start.saturating_add(keyafter_max_width);
    while cursor < right_context_end && cursor <= keyafter_limit {
        keyafter_end = cursor;
        cursor = ptx_skip_something_bytes(&bytes, cursor, right_context_end, config);
    }
    if cursor <= keyafter_limit {
        keyafter_end = cursor;
    }
    let mut keyafter_truncation = truncation_enabled && keyafter_end < right_context_end;
    keyafter_end = ptx_skip_white_backwards_bytes(&bytes, keyafter_end, keyafter_start);

    let mut before_start = left_field_start;
    let mut before_end = keyafter_start;
    before_end = ptx_skip_white_backwards_bytes(&bytes, before_end, before_start);
    while before_start.saturating_add(before_max_width) < before_end {
        let next = ptx_skip_something_bytes(&bytes, before_start, before_end, config);
        if next <= before_start {
            break;
        }
        before_start = next;
    }

    let mut before_truncation = if truncation_enabled {
        ptx_skip_white_backwards_bytes(&bytes, before_start, context_start) > left_context_start
    } else {
        false
    };
    before_start = ptx_skip_white_bytes(&bytes, before_start, context_end);

    let before_len = before_end.saturating_sub(before_start);
    let tail_max_width = before_max_width
        .saturating_sub(before_len)
        .saturating_sub(config.gap_size);
    let (tail_start, tail_end, tail_truncation) = if tail_max_width > 0 {
        let tail_start = ptx_skip_white_bytes(&bytes, keyafter_end, context_end);
        let mut tail_end = tail_start;
        let mut cursor = tail_end;
        let tail_limit = tail_start.saturating_add(tail_max_width);
        while cursor < right_context_end && cursor < tail_limit {
            tail_end = cursor;
            cursor = ptx_skip_something_bytes(&bytes, cursor, right_context_end, config);
        }
        if cursor < tail_limit {
            tail_end = cursor;
        }

        let mut tail_truncation = false;
        if tail_end > tail_start {
            keyafter_truncation = false;
            tail_truncation = truncation_enabled && tail_end < right_context_end;
        }
        tail_end = ptx_skip_white_backwards_bytes(&bytes, tail_end, tail_start);
        (tail_start, tail_end, tail_truncation)
    } else {
        (0, 0, false)
    };

    let keyafter_len = keyafter_end.saturating_sub(keyafter_start);
    let head_max_width = keyafter_max_width
        .saturating_sub(keyafter_len)
        .saturating_sub(config.gap_size);
    let (head_start, head_end, head_truncation) = if head_max_width > 0 {
        let head_end = ptx_skip_white_backwards_bytes(&bytes, before_start, context_start);
        let mut head_start = left_field_start;
        while head_start.saturating_add(head_max_width) < head_end {
            let next = ptx_skip_something_bytes(&bytes, head_start, head_end, config);
            if next <= head_start {
                break;
            }
            head_start = next;
        }

        let mut head_truncation = false;
        if head_end > head_start {
            before_truncation = false;
            head_truncation = truncation_enabled && head_start > left_context_start;
        }
        head_start = ptx_skip_white_bytes(&bytes, head_start, head_end);
        (head_start, head_end, head_truncation)
    } else {
        (0, 0, false)
    };

    PtxOutputFieldsBytes {
        tail: ptx_bytes_to_vec(&bytes, tail_start, tail_end),
        before: ptx_bytes_to_vec(&bytes, before_start, before_end),
        keyafter: ptx_bytes_to_vec(&bytes, keyafter_start, keyafter_end),
        head: ptx_bytes_to_vec(&bytes, head_start, head_end),
        tail_truncation,
        before_truncation,
        keyafter_truncation,
        head_truncation,
    }
}

/// 获取格式化的输出文本块
///
/// 该函数基于 GNU ptx 源码实现，将输入文本分割成四个部分：
/// - tail: 右侧上下文的尾部
/// - before: 关键词前的文本
/// - after: 关键词后的文本
/// - head: 左侧上下文的头部
///
/// 每个部分的大小受以下因素限制：
/// - line_width: 总行宽度
/// - gap_size: 部分之间的间隔大小
/// - trunc_str: 截断标记字符串
///
/// # 参数
/// * `all_before` - 关键词前的所有字符
/// * `keyword` - 关键词字符串
/// * `all_after` - 关键词后的所有字符
/// * `config` - PTX 配置参数
///
/// # 返回值
/// 返回一个元组 (tail, before, after, head)，每个部分都是格式化后的字符串
fn ptx_get_output_chunks_for_width_with_max(
    all_before: &[char],
    keyword: &str,
    all_after: &[char],
    config: &PtxConfig,
    line_width: usize,
    maximum_word_length: usize,
) -> (String, String, String, String) {
    let fields = ptx_define_output_fields_for_width(
        all_before,
        keyword,
        all_after,
        config,
        line_width,
        maximum_word_length,
    );
    let mut tail = fields.tail;
    if fields.tail_truncation {
        tail.push_str(&config.trunc_str);
    }
    let before = if fields.before_truncation {
        format!("{}{}", config.trunc_str, fields.before)
    } else {
        fields.before
    };
    let mut after: String = fields.keyafter.chars().skip(fields.keyword_len).collect();
    if fields.keyafter_truncation {
        after.push_str(&config.trunc_str);
    }
    let head = if fields.head_truncation {
        format!("{}{}", config.trunc_str, fields.head)
    } else {
        fields.head
    };
    (tail, before, after, head)
}

fn tex_mapper(x: char) -> String {
    match x {
        c if c.is_whitespace() => " ".to_string(),
        '\\' => "\\backslash{}".to_owned(),
        '$' | '%' | '#' | '&' | '_' => format!("\\{x}"),
        '}' | '{' => format!("$\\{x}$"),
        _ => x.to_string(),
    }
}

/// Escape special characters for TeX.
fn format_tex_field(s: &str) -> String {
    let mapped_chunks: Vec<String> = s.chars().map(tex_mapper).collect();
    mapped_chunks.join("")
}

/// 格式化输出为 TeX 格式
fn ptx_format_tex_line(
    config: &PtxConfig,
    word_ref: &WordRef,
    line: &str,
    chars_line: &[char],
    text: &str,
    chars_text: &[char],
    context_reg: &Regex,
    reference: &str,
    maximum_word_length: usize,
) -> String {
    let mut output = String::with_capacity(line.len() * 2);

    let (keyword, all_before, all_after, _) = ptx_context_slices(
        config,
        word_ref,
        text,
        chars_text,
        line,
        chars_line,
        context_reg,
    );

    // 获取格式化后的文本块
    let (tail, before, after, head) = ptx_get_output_chunks_for_width_with_max(
        all_before,
        keyword,
        all_after,
        config,
        config.line_width,
        maximum_word_length,
    );

    // 转义特殊字符并构建输出
    write!(
        output,
        "\\{} {{{}}}{{{}}}{{{}}}{{{}}}{{{}}}",
        config.macro_name,
        format_tex_field(&tail),
        format_tex_field(&before),
        format_tex_field(keyword),
        format_tex_field(&after),
        format_tex_field(&head),
    )
    .unwrap();

    // 添加引用信息
    if config.is_auto_ref || config.is_input_ref {
        write!(output, "{{{}}}", format_tex_field(reference)).unwrap();
    }

    output
}

fn ptx_format_roff_field(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_whitespace() {
                " ".to_string()
            } else if c == '"' {
                "\"\"".to_string()
            } else {
                c.to_string()
            }
        })
        .collect::<String>()
}

fn ptx_context_slices<'a>(
    config: &PtxConfig,
    word_ref: &WordRef,
    text: &'a str,
    chars_text: &'a [char],
    line: &'a str,
    chars_line: &'a [char],
    context_reg: &Regex,
) -> (&'a str, &'a [char], &'a [char], &'a [char]) {
    if word_ref.context_end > word_ref.context_start
        && word_ref.global_position_end <= text.len()
        && word_ref.context_end <= text.len()
    {
        let keyword = &text[word_ref.global_position..word_ref.global_position_end];
        let all_before = &chars_text[word_ref.context_char_start..word_ref.global_char_position];
        let all_after = &chars_text[word_ref.global_char_position_end..word_ref.context_char_end];
        return (keyword, all_before, all_after, chars_text);
    }

    let before_start = context_base_start(config, line, chars_line, context_reg);
    let (context_left, context_right) = context_bounds(
        config,
        line,
        context_reg,
        word_ref.position,
        word_ref.position_end,
        before_start,
    );
    let keyword = &line[word_ref.position..word_ref.position_end];
    (
        keyword,
        &chars_line[context_left..word_ref.position],
        &chars_line[word_ref.position_end..context_right],
        chars_line,
    )
}

/// 格式化输出为 Roff 格式
fn ptx_format_roff_line(
    config: &PtxConfig,
    word_ref: &WordRef,
    line: &str,
    chars_line: &[char],
    text: &str,
    chars_text: &[char],
    context_reg: &Regex,
    reference: &str,
    maximum_word_length: usize,
) -> String {
    let mut output = String::with_capacity(line.len() * 2);
    write!(output, ".{}", config.macro_name).unwrap();

    let (keyword, all_before, all_after, _) = ptx_context_slices(
        config,
        word_ref,
        text,
        chars_text,
        line,
        chars_line,
        context_reg,
    );

    // 获取格式化后的文本块
    let (tail, before, after, head) = ptx_get_output_chunks_for_width_with_max(
        all_before,
        keyword,
        all_after,
        config,
        config.line_width,
        maximum_word_length,
    );

    // 转义特殊字符并构建输出
    write!(
        output,
        " \"{}\" \"{}\" \"{}{}\" \"{}\"",
        ptx_format_roff_field(&tail),
        ptx_format_roff_field(&before),
        ptx_format_roff_field(keyword),
        ptx_format_roff_field(&after),
        ptx_format_roff_field(&head)
    )
    .unwrap();

    // 添加引用信息
    if config.is_auto_ref || config.is_input_ref {
        write!(output, " \"{}\"", ptx_format_roff_field(reference)).unwrap();
    }

    output
}

fn str_cols(s: &str) -> usize {
    s.chars().count()
}

fn ptx_display_field(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect()
}

fn context_base_start(
    config: &PtxConfig,
    line: &str,
    _chars_line: &[char],
    _context_reg: &Regex,
) -> usize {
    if config.is_input_ref {
        ptx_input_reference_content_start(line)
    } else {
        0
    }
}

fn context_bounds(
    config: &PtxConfig,
    line: &str,
    context_reg: &Regex,
    keyword_beg: usize,
    keyword_end: usize,
    base_start: usize,
) -> (usize, usize) {
    if !config.is_gnu_ext || config.context_regex == "\\w+" {
        return (base_start, line.len());
    }

    let mut left = base_start;
    for m in context_reg.find_iter(line) {
        if m.end() <= keyword_beg {
            left = m.end();
        } else {
            break;
        }
    }
    left = left.max(base_start);

    let mut right = line.len();
    for m in context_reg.find_iter(line) {
        if m.start() >= keyword_end {
            right = m.end();
            break;
        }
    }

    (left, right)
}

fn ptx_format_dumb_line(
    config: &PtxConfig,
    word_ref: &WordRef,
    line: &str,
    chars_line: &[char],
    text: &str,
    chars_text: &[char],
    context_reg: &Regex,
    reference: &str,
    reference_max_width: usize,
    maximum_word_length: usize,
) -> String {
    let mut output = String::with_capacity(line.len() * 2);
    let before_start = context_base_start(config, line, chars_line, context_reg);
    let (keyword, all_before, all_after, _) = ptx_context_slices(
        config,
        word_ref,
        text,
        chars_text,
        line,
        chars_line,
        context_reg,
    );
    let gap_size = config.gap_size;
    let mut effective_line_width = config.line_width;
    if (config.is_auto_ref || config.is_input_ref) && !config.is_right_ref {
        effective_line_width = effective_line_width.saturating_sub(reference_max_width + gap_size);
    }
    let (tail, before, after, head) = ptx_get_output_chunks_for_width_with_max(
        all_before,
        keyword,
        all_after,
        config,
        effective_line_width,
        maximum_word_length,
    );
    let keyafter = format!("{keyword}{after}");
    let half_line_width = effective_line_width / 2;

    let reference_len = str_cols(reference);
    if !config.is_right_ref {
        if config.is_auto_ref {
            output.push_str(reference);
            output.push(':');
            let pad = reference_max_width
                .saturating_add(gap_size)
                .saturating_sub(reference_len.saturating_add(1));
            output.push_str(&" ".repeat(pad));
        } else {
            output.push_str(reference);
            let pad = reference_max_width
                .saturating_add(gap_size)
                .saturating_sub(reference_len);
            output.push_str(&" ".repeat(pad));
        }
    }

    let before_len = str_cols(&before);
    let tail_len = str_cols(&tail);
    let before_is_only_trunc = !config.trunc_str.is_empty() && before == config.trunc_str;
    let previous_char_is_whitespace = all_before.last().is_some_and(|c| c.is_whitespace());
    if !tail.is_empty() {
        output.push_str(&ptx_display_field(&tail));
        let pad = half_line_width
            .saturating_sub(gap_size)
            .saturating_sub(before_len)
            .saturating_sub(tail_len);
        output.push_str(&" ".repeat(pad));
    } else {
        let before_space_adjust = if config.is_gnu_ext
            && before.is_empty()
            && word_ref.position > before_start
            && previous_char_is_whitespace
            && half_line_width <= gap_size + config.trunc_str.len() * 2
        {
            1
        } else {
            0
        };
        let trunc_only_adjust = if config.is_gnu_ext
            && before_is_only_trunc
            && word_ref.position > before_start
            && previous_char_is_whitespace
            && half_line_width <= gap_size + config.trunc_str.len() * 2
        {
            1
        } else {
            0
        };
        let whitespace_before_adjust =
            if config.is_gnu_ext && !before.is_empty() && before.chars().all(char::is_whitespace) {
                1
            } else {
                0
            };
        let pad = half_line_width
            .saturating_sub(gap_size)
            .saturating_sub(before_len)
            .saturating_add(before_space_adjust)
            .saturating_add(trunc_only_adjust)
            .saturating_add(whitespace_before_adjust);
        output.push_str(&" ".repeat(pad));
    }

    output.push_str(&ptx_display_field(&before));
    output.push_str(&" ".repeat(gap_size));
    output.push_str(&ptx_display_field(&keyafter));

    let keyafter_len = str_cols(&keyafter);
    let head_len = str_cols(&head);
    if !head.is_empty() {
        let pad = half_line_width
            .saturating_sub(keyafter_len)
            .saturating_sub(head_len);
        output.push_str(&" ".repeat(pad));
        output.push_str(&ptx_display_field(&head));
    } else if (config.is_auto_ref || config.is_input_ref) && config.is_right_ref {
        let pad = half_line_width.saturating_sub(keyafter_len);
        output.push_str(&" ".repeat(pad));
    }

    if (config.is_auto_ref || config.is_input_ref) && config.is_right_ref {
        output.push_str(&" ".repeat(gap_size));
        output.push_str(reference);
    }

    output
}

fn ptx_format_roff_field_bytes(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for &b in s {
        if b.is_ascii_whitespace() {
            out.push(b' ');
        } else if b == b'"' {
            out.push(b'"');
            out.push(b'"');
        } else {
            out.push(b);
        }
    }
    out
}

fn ptx_display_field_bytes(s: &[u8]) -> Vec<u8> {
    s.iter()
        .map(|&b| if b.is_ascii_whitespace() { b' ' } else { b })
        .collect()
}

fn ptx_format_dumb_line_bytes(
    config: &PtxConfig,
    word_ref: &WordRef,
    content: &FileContent,
    reference: &str,
    reference_max_width: usize,
    maximum_word_length: usize,
) -> Vec<u8> {
    let bytes_text = content.text.as_bytes();
    let (keyword, all_before, all_after) = if word_ref.context_end > word_ref.context_start {
        (
            &bytes_text[word_ref.global_position..word_ref.global_position_end],
            &bytes_text[word_ref.context_start..word_ref.global_position],
            &bytes_text[word_ref.global_position_end..word_ref.context_end],
        )
    } else {
        let line = content.lines[word_ref.local_line_nr].as_bytes();
        (
            &line[word_ref.position..word_ref.position_end],
            &line[..word_ref.position],
            &line[word_ref.position_end..],
        )
    };
    let gap_size = config.gap_size;
    let mut effective_line_width = config.line_width;
    if (config.is_auto_ref || config.is_input_ref) && !config.is_right_ref {
        effective_line_width = effective_line_width.saturating_sub(reference_max_width + gap_size);
    }
    let fields = ptx_define_output_fields_bytes_for_width(
        all_before,
        keyword,
        all_after,
        config,
        effective_line_width,
        maximum_word_length,
    );

    let mut output = Vec::new();
    if !config.is_right_ref {
        let reference_bytes = reference.as_bytes();
        if config.is_auto_ref {
            output.extend_from_slice(reference_bytes);
            output.push(b':');
            let pad = reference_max_width
                .saturating_add(gap_size)
                .saturating_sub(reference.chars().count().saturating_add(1));
            output.extend(std::iter::repeat(b' ').take(pad));
        } else {
            output.extend_from_slice(reference_bytes);
            let pad = reference_max_width
                .saturating_add(gap_size)
                .saturating_sub(reference.chars().count());
            output.extend(std::iter::repeat(b' ').take(pad));
        }
    }

    let half_line_width = effective_line_width / 2;
    let trunc_len = config.trunc_str.as_bytes().len();

    if !fields.tail.is_empty() {
        output.extend_from_slice(&ptx_display_field_bytes(&fields.tail));
        if fields.tail_truncation {
            output.extend_from_slice(config.trunc_str.as_bytes());
        }
        let pad = half_line_width
            .saturating_sub(gap_size)
            .saturating_sub(fields.before.len())
            .saturating_sub(if fields.before_truncation {
                trunc_len
            } else {
                0
            })
            .saturating_sub(fields.tail.len())
            .saturating_sub(if fields.tail_truncation { trunc_len } else { 0 });
        output.extend(std::iter::repeat(b' ').take(pad));
    } else {
        let previous_byte_is_whitespace = all_before
            .last()
            .is_some_and(|byte| byte.is_ascii_whitespace());
        let trunc_only_adjust = if config.is_gnu_ext
            && fields.before_truncation
            && fields.before.is_empty()
            && word_ref.position > 0
            && previous_byte_is_whitespace
            && half_line_width <= gap_size + trunc_len * 2
        {
            1
        } else {
            0
        };
        let whitespace_before_adjust = if config.is_gnu_ext
            && !fields.before.is_empty()
            && fields.before.iter().all(|byte| byte.is_ascii_whitespace())
        {
            1
        } else {
            0
        };
        let pad = half_line_width
            .saturating_sub(gap_size)
            .saturating_sub(fields.before.len())
            .saturating_sub(if fields.before_truncation {
                trunc_len
            } else {
                0
            })
            .saturating_add(trunc_only_adjust)
            .saturating_add(whitespace_before_adjust);
        output.extend(std::iter::repeat(b' ').take(pad));
    }

    if fields.before_truncation {
        output.extend_from_slice(config.trunc_str.as_bytes());
    }
    output.extend_from_slice(&ptx_display_field_bytes(&fields.before));
    output.extend(std::iter::repeat(b' ').take(gap_size));
    output.extend_from_slice(&ptx_display_field_bytes(&fields.keyafter));
    if fields.keyafter_truncation {
        output.extend_from_slice(config.trunc_str.as_bytes());
    }

    if !fields.head.is_empty() {
        let pad = half_line_width
            .saturating_sub(fields.keyafter.len())
            .saturating_sub(if fields.keyafter_truncation {
                trunc_len
            } else {
                0
            })
            .saturating_sub(fields.head.len())
            .saturating_sub(if fields.head_truncation { trunc_len } else { 0 });
        output.extend(std::iter::repeat(b' ').take(pad));
        if fields.head_truncation {
            output.extend_from_slice(config.trunc_str.as_bytes());
        }
        output.extend_from_slice(&ptx_display_field_bytes(&fields.head));
    } else if (config.is_auto_ref || config.is_input_ref) && config.is_right_ref {
        let pad = half_line_width
            .saturating_sub(fields.keyafter.len())
            .saturating_sub(if fields.keyafter_truncation {
                trunc_len
            } else {
                0
            });
        output.extend(std::iter::repeat(b' ').take(pad));
    }

    if (config.is_auto_ref || config.is_input_ref) && config.is_right_ref {
        output.extend(std::iter::repeat(b' ').take(gap_size));
        output.extend_from_slice(reference.as_bytes());
    }

    output
}

fn ptx_format_roff_line_bytes(
    config: &PtxConfig,
    word_ref: &WordRef,
    content: &FileContent,
    reference: &str,
    maximum_word_length: usize,
) -> Vec<u8> {
    let bytes_text = content.text.as_bytes();
    let (keyword, all_before, all_after) = if word_ref.context_end > word_ref.context_start {
        (
            &bytes_text[word_ref.global_position..word_ref.global_position_end],
            &bytes_text[word_ref.context_start..word_ref.global_position],
            &bytes_text[word_ref.global_position_end..word_ref.context_end],
        )
    } else {
        let line = content.lines[word_ref.local_line_nr].as_bytes();
        (
            &line[word_ref.position..word_ref.position_end],
            &line[..word_ref.position],
            &line[word_ref.position_end..],
        )
    };
    let fields = ptx_define_output_fields_bytes_for_width(
        all_before,
        keyword,
        all_after,
        config,
        config.line_width,
        maximum_word_length,
    );

    let mut output = Vec::new();
    output.push(b'.');
    output.extend_from_slice(config.macro_name.as_bytes());
    output.extend_from_slice(b" \"");
    output.extend_from_slice(&ptx_format_roff_field_bytes(&fields.tail));
    if fields.tail_truncation {
        output.extend_from_slice(config.trunc_str.as_bytes());
    }
    output.extend_from_slice(b"\" \"");
    if fields.before_truncation {
        output.extend_from_slice(config.trunc_str.as_bytes());
    }
    output.extend_from_slice(&ptx_format_roff_field_bytes(&fields.before));
    output.extend_from_slice(b"\" \"");
    output.extend_from_slice(&ptx_format_roff_field_bytes(&fields.keyafter));
    if fields.keyafter_truncation {
        output.extend_from_slice(config.trunc_str.as_bytes());
    }
    output.extend_from_slice(b"\" \"");
    if fields.head_truncation {
        output.extend_from_slice(config.trunc_str.as_bytes());
    }
    output.extend_from_slice(&ptx_format_roff_field_bytes(&fields.head));
    output.push(b'"');
    if config.is_auto_ref || config.is_input_ref {
        output.extend_from_slice(b" \"");
        output.extend_from_slice(&ptx_format_roff_field_bytes(reference.as_bytes()));
        output.push(b'"');
    }
    output
}

/// 执行 PTX 命令的核心逻辑
fn ptx_exec(settings: &PtxSettings) -> CTResult<()> {
    let mut writer: BufWriter<Box<dyn Write>> =
        BufWriter::new(if settings.output_filename == "-" {
            Box::new(stdout())
        } else {
            let file = File::create(&settings.output_filename).map_err_context(String::new)?;
            Box::new(file)
        });

    let context_reg = compile_regex_lossy(&settings.config.context_regex);

    // Check for zero-length regex match only when there are words to process
    // This matches GNU ptx behavior (only errors when processing non-empty content)
    if !settings.words.is_empty() && regex_matches_zero_len(&settings.config.context_regex) {
        return Err(CtSimpleError::new(
            1,
            format!(
                "error: regular expression has a match of length zero: '{}'",
                settings.config.context_regex
            ),
        ));
    }

    let mut reference_max_width = 0usize;
    if settings.config.is_auto_ref || settings.config.is_input_ref || !settings.config.is_right_ref
    {
        for word_ref in &settings.words {
            // 通过索引直接获取文件内容
            let content = &settings.file_map[word_ref.file_index];
            let reference = ptx_get_reference(
                &settings.config,
                word_ref,
                &content.filename, // 传入提取到的文件名
                &content.lines[word_ref.local_line_nr],
                &context_reg,
            );
            reference_max_width = reference_max_width.max(str_cols(&reference));
        }
    }

    for word_ref in &settings.words {
        // 通过索引直接获取文件内容
        let content = &settings.file_map[word_ref.file_index];
        let maximum_word_length = ptx_content_maximum_word_length_bytes(content, &settings.config);

        let reference = ptx_get_reference(
            &settings.config,
            word_ref,
            &content.filename, // 传入提取到的文件名
            &content.lines[word_ref.local_line_nr],
            &context_reg,
        );

        let output_line = match settings.config.format {
            OutFormat::Tex => ptx_format_tex_line(
                &settings.config,
                word_ref,
                &content.lines[word_ref.local_line_nr],
                &content.chars_lines[word_ref.local_line_nr],
                &content.text,
                &content.chars_text,
                &context_reg,
                &reference,
                maximum_word_length,
            )
            .into_bytes(),
            OutFormat::Roff => ptx_format_roff_line_bytes(
                &settings.config,
                word_ref,
                content,
                &reference,
                maximum_word_length,
            ),
            OutFormat::Dumb => ptx_format_dumb_line_bytes(
                &settings.config,
                word_ref,
                content,
                &reference,
                reference_max_width,
                maximum_word_length,
            ),
        };

        writer
            .write_all(&output_line)
            .map_err_context(String::new)?;
        writer.write_all(b"\n").map_err_context(String::new)?;
    }
    Ok(())
}

fn ptx_reference_max_width(settings: &PtxSettings, context_reg: &Regex) -> usize {
    let mut reference_max_width = 0usize;
    if settings.config.is_auto_ref || settings.config.is_input_ref || !settings.config.is_right_ref
    {
        for word_ref in &settings.words {
            let content = &settings.file_map[word_ref.file_index];
            let reference = ptx_get_reference(
                &settings.config,
                word_ref,
                &content.filename,
                &content.lines[word_ref.local_line_nr],
                context_reg,
            );
            reference_max_width = reference_max_width.max(str_cols(&reference));
        }
    }
    reference_max_width
}

fn ptx_render_row(
    settings: &PtxSettings,
    word_ref: &WordRef,
    context_reg: &Regex,
    reference_max_width: usize,
) -> PtxSemanticRow {
    let content = &settings.file_map[word_ref.file_index];
    let maximum_word_length = ptx_content_maximum_word_length(content, &settings.config);
    let line = &content.lines[word_ref.local_line_nr];
    let chars_line = &content.chars_lines[word_ref.local_line_nr];
    let reference = ptx_get_reference(
        &settings.config,
        word_ref,
        &content.filename,
        line,
        context_reg,
    );
    let (keyword, all_before, all_after, _) = ptx_context_slices(
        &settings.config,
        word_ref,
        &content.text,
        &content.chars_text,
        line,
        chars_line,
        context_reg,
    );
    let mut effective_line_width = settings.config.line_width;
    if (settings.config.is_auto_ref || settings.config.is_input_ref)
        && !settings.config.is_right_ref
    {
        effective_line_width =
            effective_line_width.saturating_sub(reference_max_width + settings.config.gap_size);
    }
    let (tail, before, after, head) = ptx_get_output_chunks_for_width_with_max(
        all_before,
        keyword,
        all_after,
        &settings.config,
        effective_line_width,
        maximum_word_length,
    );
    let rendered_text = match settings.config.format {
        OutFormat::Tex => ptx_format_tex_line(
            &settings.config,
            word_ref,
            line,
            chars_line,
            &content.text,
            &content.chars_text,
            context_reg,
            &reference,
            maximum_word_length,
        ),
        OutFormat::Roff => ptx_format_roff_line(
            &settings.config,
            word_ref,
            line,
            chars_line,
            &content.text,
            &content.chars_text,
            context_reg,
            &reference,
            maximum_word_length,
        ),
        OutFormat::Dumb => ptx_format_dumb_line(
            &settings.config,
            word_ref,
            line,
            chars_line,
            &content.text,
            &content.chars_text,
            context_reg,
            &reference,
            reference_max_width,
            maximum_word_length,
        ),
    };

    PtxSemanticRow {
        row_index: 0,
        keyword: keyword.to_string(),
        before,
        after,
        head,
        tail,
        reference,
        file: content.filename.clone(),
        line_index: word_ref.local_line_nr + 1,
        global_line_index: word_ref.global_line_nr + 1,
        rendered_text,
        format: ptx_format_name(&settings.config.format).to_string(),
    }
}

fn ptx_collect_semantic_rows(settings: &PtxSettings) -> Vec<PtxSemanticRow> {
    let context_reg = compile_regex_lossy(&settings.config.context_regex);
    let reference_max_width = ptx_reference_max_width(settings, &context_reg);
    let mut rows: Vec<PtxSemanticRow> = settings
        .words
        .iter()
        .map(|word_ref| ptx_render_row(settings, word_ref, &context_reg, reference_max_width))
        .collect();

    for (index, row) in rows.iter_mut().enumerate() {
        row.row_index = index + 1;
    }

    rows
}

fn ptx_exec_to_writer(settings: &PtxSettings, writer: &mut impl Write) -> CTResult<()> {
    let context_reg = compile_regex_lossy(&settings.config.context_regex);

    if !settings.words.is_empty() && regex_matches_zero_len(&settings.config.context_regex) {
        return Err(CtSimpleError::new(
            1,
            format!(
                "error: regular expression has a match of length zero: '{}'",
                settings.config.context_regex
            ),
        ));
    }

    let mut reference_max_width = 0usize;
    if settings.config.is_auto_ref || settings.config.is_input_ref || !settings.config.is_right_ref
    {
        for word_ref in &settings.words {
            let file_map_value = &settings.file_map[word_ref.file_index];
            let reference = ptx_get_reference(
                &settings.config,
                word_ref,
                &file_map_value.filename,
                &file_map_value.lines[word_ref.local_line_nr],
                &context_reg,
            );
            reference_max_width = reference_max_width.max(str_cols(&reference));
        }
    }

    for word_ref in &settings.words {
        let file_map_value = &settings.file_map[word_ref.file_index];
        let maximum_word_length =
            ptx_content_maximum_word_length_bytes(file_map_value, &settings.config);

        let reference = ptx_get_reference(
            &settings.config,
            word_ref,
            &file_map_value.filename,
            &file_map_value.lines[word_ref.local_line_nr],
            &context_reg,
        );

        let output_line = match settings.config.format {
            OutFormat::Tex => ptx_format_tex_line(
                &settings.config,
                word_ref,
                &file_map_value.lines[word_ref.local_line_nr],
                &file_map_value.chars_lines[word_ref.local_line_nr],
                &file_map_value.text,
                &file_map_value.chars_text,
                &context_reg,
                &reference,
                maximum_word_length,
            )
            .into_bytes(),
            OutFormat::Roff => ptx_format_roff_line_bytes(
                &settings.config,
                word_ref,
                file_map_value,
                &reference,
                maximum_word_length,
            ),
            OutFormat::Dumb => ptx_format_dumb_line_bytes(
                &settings.config,
                word_ref,
                file_map_value,
                &reference,
                reference_max_width,
                maximum_word_length,
            ),
        };

        writer
            .write_all(&output_line)
            .map_err_context(String::new)?;
        writer.write_all(b"\n").map_err_context(String::new)?;
    }
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PtxCoreOutput {
    pub bytes: Vec<u8>,
}

pub fn ptx_main(args: impl ctcore::Args) -> CTResult<()> {
    let mut out = stdout().lock();
    ptx_main_with_writer(args, &mut out)
}

pub fn ptx_core_output(args: impl ctcore::Args) -> CTResult<PtxCoreOutput> {
    let mut out = Vec::new();
    ptx_main_with_writer(args, &mut out)?;
    Ok(PtxCoreOutput { bytes: out })
}

fn ptx_render_help_text() -> String {
    let mut command = ct_app();
    command.render_help().to_string()
}

fn ptx_render_version_text() -> String {
    ct_app().render_version()
}

pub fn ptx_main_with_writer<W: Write>(args: impl ctcore::Args, out: &mut W) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let args: Vec<OsString> = args.collect();
    let matches = match ct_app().try_get_matches_from(args) {
        Ok(matches) => matches,
        Err(err) => {
            return match err.kind() {
                ErrorKind::DisplayHelp => {
                    out.write_all(ptx_render_help_text().as_bytes())?;
                    Ok(())
                }
                ErrorKind::DisplayVersion => {
                    out.write_all(ptx_render_version_text().as_bytes())?;
                    Ok(())
                }
                _ => Err(err.into()),
            };
        }
    };
    let settings = PtxSettings::from_matches(matches)?;
    if settings.output_filename != "-" {
        ptx_exec(&settings)?;
        return Ok(());
    }

    ptx_exec_to_writer(&settings, out)
}

pub fn ptx_core_output_from_args(args: Vec<OsString>) -> CTResult<PtxCoreOutput> {
    ptx_core_output(args.into_iter())
}

pub fn ptx_native_semantic(args: impl ctcore::Args) -> CTResult<PtxSemantic> {
    ptx_native_semantic_with_stdin(args, None)
}

pub fn ptx_native_semantic_with_stdin(
    args: impl ctcore::Args,
    stdin_bytes: Option<Vec<u8>>,
) -> CTResult<PtxSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);

    let argv: Vec<OsString> = args.collect();
    let direct = run_ptx_direct_process(&argv, stdin_bytes.as_deref())?;
    let classic_text = String::from_utf8_lossy(&direct.stdout).into_owned();
    let stderr_text = String::from_utf8_lossy(&direct.stderr).into_owned();

    let rows = if direct.exit_code == 0 {
        ptx_collect_semantic_rows_from_argv(argv, stdin_bytes)?
    } else {
        Vec::new()
    };

    Ok(PtxSemantic {
        rows,
        classic_text,
        stderr_text,
        exit_code: direct.exit_code,
    })
}

fn ptx_collect_semantic_rows_from_argv(
    argv: Vec<OsString>,
    stdin_bytes: Option<Vec<u8>>,
) -> CTResult<Vec<PtxSemanticRow>> {
    let matches = match ct_app().try_get_matches_from(argv) {
        Ok(matches) => matches,
        Err(_) => return Ok(Vec::new()),
    };

    if let Some(bytes) = stdin_bytes {
        return ctcore::ct_io::with_injected_stdin(bytes, move || {
            ptx_collect_semantic_rows_from_matches(matches)
        });
    }

    ptx_collect_semantic_rows_from_matches(matches)
}

fn ptx_collect_semantic_rows_from_matches(
    matches: clap::ArgMatches,
) -> CTResult<Vec<PtxSemanticRow>> {
    let settings = match PtxSettings::from_matches(matches) {
        Ok(settings) => settings,
        Err(_) => return Ok(Vec::new()),
    };

    Ok(ptx_semantic_rows_for_settings(&settings))
}

fn ptx_semantic_from_clap_error(error: clap::Error) -> PtxSemantic {
    match error.kind() {
        ErrorKind::DisplayHelp => PtxSemantic {
            rows: Vec::new(),
            classic_text: ptx_render_help_text(),
            stderr_text: String::new(),
            exit_code: 0,
        },
        ErrorKind::DisplayVersion => PtxSemantic {
            rows: Vec::new(),
            classic_text: ptx_render_version_text(),
            stderr_text: String::new(),
            exit_code: 0,
        },
        _ => PtxSemantic {
            rows: Vec::new(),
            classic_text: String::new(),
            stderr_text: error.to_string(),
            exit_code: 1,
        },
    }
}

fn ptx_render_error_text(err: &dyn CTError) -> String {
    let mut stderr = format!("ptx: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'ptx --help' for more information.\n");
    }
    stderr
}

fn ptx_zero_length_regex_error(pattern: &str) -> Box<dyn CTError> {
    CtSimpleError::new(
        1,
        format!("error: regular expression has a match of length zero: '{pattern}'"),
    )
}

fn ptx_semantic_rows_for_settings(settings: &PtxSettings) -> Vec<PtxSemanticRow> {
    if !settings.words.is_empty() && regex_matches_zero_len(&settings.config.context_regex) {
        Vec::new()
    } else {
        ptx_collect_semantic_rows(settings)
    }
}

pub fn ptx_native_semantic_rows_only(args: impl ctcore::Args) -> CTResult<PtxSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);

    let argv: Vec<OsString> = args.collect();
    let matches = match ct_app().try_get_matches_from(argv) {
        Ok(matches) => matches,
        Err(error) => return Ok(ptx_semantic_from_clap_error(error)),
    };

    let settings = match PtxSettings::from_matches(matches) {
        Ok(settings) => settings,
        Err(error) => {
            return Ok(PtxSemantic {
                rows: Vec::new(),
                classic_text: String::new(),
                stderr_text: ptx_render_error_text(error.as_ref()),
                exit_code: error.code(),
            });
        }
    };

    if !settings.words.is_empty() && regex_matches_zero_len(&settings.config.context_regex) {
        let error = ptx_zero_length_regex_error(&settings.config.context_regex);
        return Ok(PtxSemantic {
            rows: Vec::new(),
            classic_text: String::new(),
            stderr_text: ptx_render_error_text(error.as_ref()),
            exit_code: error.code(),
        });
    }

    Ok(PtxSemantic {
        rows: ptx_semantic_rows_for_settings(&settings),
        classic_text: String::new(),
        stderr_text: String::new(),
        exit_code: 0,
    })
}

mod ptx_options {
    pub static PTX_FILE: &str = "file";
    pub static PTX_AUTO_REFERENCE: &str = "auto-reference";
    pub static PTX_TRADITIONAL: &str = "traditional";
    pub static PTX_FLAG_TRUNCATION: &str = "flag-truncation";
    pub static PTX_MACRO_NAME: &str = "macro-name";
    pub static PTX_FORMAT: &str = "format";
    pub static PTX_FORMAT_ROFF: &str = "format-roff";
    pub static PTX_RIGHT_SIDE_REFS: &str = "right-side-refs";
    pub static PTX_SENTENCE_REGEXP: &str = "sentence-regexp";
    pub static PTX_FORMAT_TEX: &str = "format-tex";
    pub static PTX_WORD_REGEXP: &str = "word-regexp";
    pub static PTX_BREAK_FILE: &str = "break-file";
    pub static PTX_IGNORE_CASE: &str = "ignore-case";
    pub static PTX_GAP_SIZE: &str = "gap-size";
    pub static PTX_IGNORE_FILE: &str = "ignore-file";
    pub static PTX_ONLY_FILE: &str = "only-file";
    pub static PTX_REFERENCES: &str = "references";
    pub static PTX_TYPESET_MODE: &str = "typeset-mode";
    pub static PTX_WIDTH: &str = "width";
}

/// PTX 命令的运行配置
#[derive(Debug)]
struct PtxSettings {
    /// 基础配置选项
    config: PtxConfig,
    /// 文件内容映射
    file_map: FileMap,
    /// 单词引用集合
    words: BTreeSet<WordRef>,
    /// 输出文件名
    output_filename: String,
}

impl PtxSettings {
    fn from_matches(matches: clap::ArgMatches) -> CTResult<Self> {
        // 获取输入文件列表
        let mut input_files: Vec<String> = match &matches.get_many::<String>(ptx_options::PTX_FILE)
        {
            Some(v) => v.clone().cloned().collect(),
            None => vec!["-".to_string()],
        };

        // 获取配置
        let config = get_config(&matches)?;

        // 创建单词过滤器
        let word_filter = WordFilter::new(&matches, &config)?;

        // 读取输入文件
        let file_map = ptx_read_input(&input_files, &config).map_err_context(String::new)?;

        // 创建单词集合
        let word_set = ptx_create_word_set(&config, &word_filter, &file_map);

        // 确定输出文件名
        let output_file = if !config.is_gnu_ext && input_files.len() == 2 {
            input_files.pop().unwrap()
        } else {
            "-".to_string()
        };

        // 创建设置
        let settings = Self {
            config,
            file_map,
            words: word_set,
            output_filename: output_file,
        };

        Ok(settings)
    }
}

impl Default for PtxSettings {
    fn default() -> Self {
        Self {
            config: PtxConfig::default(),
            file_map: FileMap::new(),
            words: BTreeSet::new(),
            output_filename: "-".to_string(),
        }
    }
}

pub fn ct_app() -> Command {
    let args = vec![
        Arg::new(ptx_options::PTX_FILE)
            .hide(true)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::FilePath),
        Arg::new(ptx_options::PTX_AUTO_REFERENCE)
            .short('A')
            .long(ptx_options::PTX_AUTO_REFERENCE)
            .help(t!("ptx.clap.ptx_auto_reference"))
            .action(ArgAction::SetTrue),
        Arg::new(ptx_options::PTX_TRADITIONAL)
            .short('G')
            .long(ptx_options::PTX_TRADITIONAL)
            .help(t!("ptx.clap.ptx_traditional"))
            .action(ArgAction::SetTrue),
        Arg::new(ptx_options::PTX_FLAG_TRUNCATION)
            .short('F')
            .long(ptx_options::PTX_FLAG_TRUNCATION)
            .help(t!("ptx.clap.ptx_flag_truncation"))
            .value_name("STRING"),
        Arg::new(ptx_options::PTX_MACRO_NAME)
            .short('M')
            .long(ptx_options::PTX_MACRO_NAME)
            .help(t!("ptx.clap.ptx_macro_name"))
            .value_name("STRING"),
        Arg::new(ptx_options::PTX_FORMAT_ROFF)
            .short('O')
            .help(t!("ptx.clap.ptx_format_roff"))
            .action(ArgAction::SetTrue),
        Arg::new(ptx_options::PTX_FORMAT)
            .long(ptx_options::PTX_FORMAT)
            .value_name("FORMAT")
            .value_parser(["roff", "tex"]),
        Arg::new(ptx_options::PTX_RIGHT_SIDE_REFS)
            .short('R')
            .long(ptx_options::PTX_RIGHT_SIDE_REFS)
            .help(t!("ptx.clap.ptx_right_side_refs"))
            .action(ArgAction::SetTrue),
        Arg::new(ptx_options::PTX_SENTENCE_REGEXP)
            .short('S')
            .long(ptx_options::PTX_SENTENCE_REGEXP)
            .help(t!("ptx.clap.ptx_sentence_regexp"))
            .value_name("REGEXP"),
        Arg::new(ptx_options::PTX_FORMAT_TEX)
            .short('T')
            .help(t!("ptx.clap.ptx_format_tex"))
            .action(ArgAction::SetTrue),
        Arg::new(ptx_options::PTX_WORD_REGEXP)
            .short('W')
            .long(ptx_options::PTX_WORD_REGEXP)
            .help(t!("ptx.clap.ptx_word_regexp"))
            .value_name("REGEXP"),
        Arg::new(ptx_options::PTX_BREAK_FILE)
            .short('b')
            .long(ptx_options::PTX_BREAK_FILE)
            .help(t!("ptx.clap.ptx_break_file"))
            .value_name("FILE")
            .value_hint(clap::ValueHint::FilePath),
        Arg::new(ptx_options::PTX_IGNORE_CASE)
            .short('f')
            .long(ptx_options::PTX_IGNORE_CASE)
            .help(t!("ptx.clap.ptx_ignore_case"))
            .action(ArgAction::SetTrue),
        Arg::new(ptx_options::PTX_GAP_SIZE)
            .short('g')
            .long(ptx_options::PTX_GAP_SIZE)
            .help(t!("ptx.clap.ptx_gap_size"))
            .value_name("NUMBER"),
        Arg::new(ptx_options::PTX_IGNORE_FILE)
            .short('i')
            .long(ptx_options::PTX_IGNORE_FILE)
            .help(t!("ptx.clap.ptx_ignore_file"))
            .value_name("FILE")
            .value_hint(clap::ValueHint::FilePath),
        Arg::new(ptx_options::PTX_ONLY_FILE)
            .short('o')
            .long(ptx_options::PTX_ONLY_FILE)
            .help(t!("ptx.clap.ptx_only_file"))
            .value_name("FILE")
            .value_hint(clap::ValueHint::FilePath),
        Arg::new(ptx_options::PTX_REFERENCES)
            .short('r')
            .long(ptx_options::PTX_REFERENCES)
            .help(t!("ptx.clap.ptx_references"))
            .action(ArgAction::SetTrue),
        Arg::new(ptx_options::PTX_TYPESET_MODE)
            .short('t')
            .long(ptx_options::PTX_TYPESET_MODE)
            .help("not implemented.")
            .action(ArgAction::SetTrue),
        Arg::new(ptx_options::PTX_WIDTH)
            .short('w')
            .long(ptx_options::PTX_WIDTH)
            .help(t!("ptx.clap.ptx_width"))
            .value_name("NUMBER"),
    ];

    Command::new(ctcore::ct_util_name())
        .about(t!("ptx.about"))
        .version(crate_version!())
        .override_usage(t!("ptx.usage"))
        .disable_help_flag(true)
        .disable_version_flag(true)
        .infer_long_args(true)
        .arg(
            Arg::new("help")
                .long("help")
                .help("display this help and exit")
                .action(ArgAction::Help),
        )
        .arg(
            Arg::new("version")
                .long("version")
                .help("output version information and exit")
                .action(ArgAction::Version),
        )
        .args(args)
}

#[derive(Default)]
pub struct Ptx;
impl Tool for Ptx {
    fn name(&self) -> &'static str {
        "ptx"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        ptx_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn test_word_ref(
        word: &str,
        local_line_nr: usize,
        position: usize,
        position_end: usize,
    ) -> WordRef {
        WordRef {
            word: word.to_string(),
            global_line_nr: local_line_nr,
            local_line_nr,
            position,
            position_end,
            global_position: position,
            global_position_end: position_end,
            context_start: 0,
            context_end: 0,
            global_char_position: position,
            global_char_position_end: position_end,
            context_char_start: 0,
            context_char_end: 0,
            file_index: 0,
        }
    }

    fn test_file_content(filename: &str, lines: Vec<String>, offset: usize) -> FileContent {
        let mut text = String::new();
        let mut line_starts = Vec::with_capacity(lines.len());
        for (index, line) in lines.iter().enumerate() {
            if index > 0 {
                text.push('\n');
            }
            line_starts.push(text.len());
            text.push_str(line);
        }
        FileContent {
            filename: filename.to_string(),
            chars_text: text.chars().collect(),
            byte_to_char: build_byte_to_char_map(&text),
            text,
            line_starts,
            chars_lines: lines.iter().map(|line| line.chars().collect()).collect(),
            lines,
            offset,
        }
    }

    #[test]
    fn test_tool_implementation() {
        let tool = Ptx;

        // 测试 name 方法
        assert_eq!(tool.name(), "ptx");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("ptx"));

        // 测试 execute 方法
        let args = vec![
            OsString::from("ptx"),
            OsString::from("--definitely-invalid-flag"),
        ];
        assert!(tool.execute(&args).is_err());
    }

    #[test]
    fn test_version_uses_syskits_package_version() {
        let mut out = Vec::new();
        ptx_main_with_writer(
            [OsString::from("ptx"), OsString::from("--version")].into_iter(),
            &mut out,
        )
        .unwrap();

        let text = String::from_utf8(out).unwrap();
        assert!(text.ends_with(&format!(" {}\n", crate_version!())));
        assert!(!text.contains("GNU coreutils"));
    }

    #[test]
    fn test_help_uses_syskits_command_definition() {
        let mut out = Vec::new();
        ptx_main_with_writer(
            [OsString::from("ptx"), OsString::from("--help")].into_iter(),
            &mut out,
        )
        .unwrap();

        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("Usage:"));
        assert!(text.contains("--auto-reference"));
        assert!(!text.contains("GNU coreutils"));
        assert!(!text.contains("translationproject.org"));

        let options = text
            .split_once("Options:")
            .map(|(_, options)| options)
            .expect("help should include options");
        let option_lines: Vec<&str> = options
            .lines()
            .filter(|line| line.trim_start().starts_with('-'))
            .collect();
        assert!(option_lines.len() > 1);
        for window in option_lines.windows(2) {
            let first = options.find(window[0]).unwrap();
            let second = options.find(window[1]).unwrap();
            assert!(
                !options[first..second].contains("\n\n"),
                "options should be rendered in compact help format"
            );
        }
    }

    #[test]
    fn test_cli_dumb_formatter_w10_two_tokens_alignment() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), b"foo bar\n").unwrap();

        let mut out = Vec::new();
        ptx_main_with_writer(
            [
                OsString::from("ptx"),
                OsString::from("-w10"),
                OsString::from(temp.path()),
            ]
            .into_iter(),
            &mut out,
        )
        .unwrap();

        assert_eq!(out, b"     /   bar\n        foo/\n");
    }

    mod config_tests {
        use super::*;

        #[test]
        fn test_get_config_default() {
            let matches = ct_app().try_get_matches_from(vec!["ptx"]).unwrap();
            let config = get_config(&matches).unwrap();
            assert!(config.is_gnu_ext);
            assert!(matches!(config.format, OutFormat::Dumb));
        }

        #[test]
        fn test_get_config_sentence_regexp_supported() {
            let matches = ct_app()
                .try_get_matches_from(vec!["ptx", "-G", "-S", "[A-Z].*"])
                .unwrap();
            let config = get_config(&matches).unwrap();
            assert_eq!(config.context_regex, "[A-Z].*");
        }

        #[test]
        fn test_get_config_sentence_regexp_zero_len_accepted() {
            // Zero-length regex check is now deferred to execution time
            // to match GNU ptx behavior (only errors when processing non-empty content)
            let matches = ct_app()
                .try_get_matches_from(vec!["ptx", "-S", "^"])
                .unwrap();
            let config = get_config(&matches).unwrap();
            assert_eq!(config.context_regex, "^");
        }

        #[test]
        fn test_get_config_traditional() {
            let matches = ct_app().try_get_matches_from(vec!["ptx", "-G"]).unwrap();
            let config = get_config(&matches).unwrap();
            assert!(!config.is_gnu_ext);
            assert!(matches!(config.format, OutFormat::Roff));
            assert_eq!(config.context_regex, "[^ \t\n]+");
        }

        #[test]
        fn test_get_config_with_options() {
            let matches = ct_app()
                .try_get_matches_from(vec![
                    "ptx", "-G", "-w", "80", "-g", "4", "-M", "test", "-F", "*", "-O",
                ])
                .unwrap();
            let config = get_config(&matches).unwrap();
            assert_eq!(config.line_width, 80);
            assert_eq!(config.gap_size, 4);
            assert_eq!(config.macro_name, "test");
            assert_eq!(config.trunc_str, "*");
            assert!(matches!(config.format, OutFormat::Roff));
        }
    }

    mod filter_tests {
        use super::*;

        fn create_temp_file_with_content(content: &str) -> NamedTempFile {
            let mut file = NamedTempFile::new().unwrap();
            write!(file, "{content}").unwrap();
            file
        }

        #[test]
        fn test_read_word_filter_file() {
            let file = create_temp_file_with_content("word1\nword2\nword3");
            let matches = ct_app()
                .try_get_matches_from(vec!["ptx", "-o", file.path().to_str().unwrap()])
                .unwrap();

            let words = read_word_filter_file(&matches, ptx_options::PTX_ONLY_FILE).unwrap();
            assert_eq!(words.len(), 3);
            assert!(words.contains("word1"));
            assert!(words.contains("word2"));
            assert!(words.contains("word3"));
        }

        #[test]
        fn test_read_char_filter_file() {
            let file = create_temp_file_with_content("abc");
            let matches = ct_app()
                .try_get_matches_from(vec!["ptx", "-b", file.path().to_str().unwrap()])
                .unwrap();

            let chars = read_char_filter_file(&matches, ptx_options::PTX_BREAK_FILE).unwrap();
            assert_eq!(chars.len(), 3);
            assert!(chars.contains(&'a'));
            assert!(chars.contains(&'b'));
            assert!(chars.contains(&'c'));
        }

        #[test]
        fn test_word_filter_new() {
            let config = PtxConfig::default();
            let matches = ct_app().try_get_matches_from(vec!["ptx"]).unwrap();

            let filter = WordFilter::new(&matches, &config).unwrap();
            assert!(!filter.is_only_specified);
            assert!(!filter.is_ignore_specified);
            assert_eq!(filter.word_regex, "[A-Za-z]+");
        }

        #[test]
        fn test_word_filter_break_file_generates_regex() {
            let breaker = create_temp_file_with_content("/");
            let matches = ct_app()
                .try_get_matches_from(vec!["ptx", "-G", "-b", breaker.path().to_str().unwrap()])
                .unwrap();
            let config = PtxConfig::default();
            let filter = WordFilter::new(&matches, &config).unwrap();
            assert_eq!(filter.word_regex, "[^/]+");
        }

        #[test]
        fn test_word_filter_custom_word_regex() {
            let matches = ct_app()
                .try_get_matches_from(vec!["ptx", "-G", "-W", "[A-Z]+"])
                .unwrap();
            let config = PtxConfig::default();
            let filter = WordFilter::new(&matches, &config).unwrap();
            assert_eq!(filter.word_regex, "[A-Z]+");
        }
    }

    mod word_ref_tests {
        use super::*;

        #[test]
        fn test_word_ref_ordering() {
            let word1 = test_word_ref("test", 1, 0, 4);
            let word2 = WordRef {
                global_line_nr: 2,
                ..test_word_ref("test", 1, 0, 4)
            };

            assert!(word1 < word2);
        }
    }

    mod formatting_tests {
        use super::*;

        #[test]
        fn test_format_roff_line() {
            let config = PtxConfig {
                format: OutFormat::Roff,
                macro_name: "xx".to_string(),
                ..Default::default()
            };

            let word_ref = test_word_ref("test", 1, 6, 10);

            let line = "hello test world";
            let chars_line: Vec<char> = line.chars().collect();
            let maximum_word_length = ptx_maximum_word_length_in_chars(&chars_line, &config);
            let reference = "1";
            let context_reg = compile_regex_lossy(&config.context_regex);

            let result = ptx_format_roff_line(
                &config,
                &word_ref,
                line,
                &chars_line,
                line,
                &chars_line,
                &context_reg,
                reference,
                maximum_word_length,
            );
            assert!(result.starts_with(".xx"));
            assert!(result.contains("test"));
        }

        #[test]
        fn test_format_dumb_line_w10_two_tokens_alignment() {
            let config = PtxConfig {
                line_width: 10,
                gap_size: 3,
                trunc_str: "/".to_string(),
                ..Default::default()
            };
            let line = "foo bar";
            let chars_line: Vec<char> = line.chars().collect();
            let maximum_word_length = ptx_maximum_word_length_in_chars(&chars_line, &config);
            let word_ref = test_word_ref("bar", 0, 4, 7);
            let context_reg = compile_regex_lossy(&config.context_regex);
            let got = ptx_format_dumb_line(
                &config,
                &word_ref,
                line,
                &chars_line,
                line,
                &chars_line,
                &context_reg,
                "",
                0,
                maximum_word_length,
            );
            assert_eq!(got, "     /   bar");
        }

        #[test]
        fn test_format_dumb_line_sentence_regex_alignment() {
            let config = PtxConfig {
                context_regex: "[.!?]".to_string(),
                ..Default::default()
            };
            let line = "alpha. beta! gamma?";
            let chars_line: Vec<char> = line.chars().collect();
            let maximum_word_length = ptx_maximum_word_length_in_chars(&chars_line, &config);
            let word_ref = test_word_ref("beta", 0, 7, 11);
            let context_reg = compile_regex_lossy(&config.context_regex);
            let got = ptx_format_dumb_line(
                &config,
                &word_ref,
                line,
                &chars_line,
                line,
                &chars_line,
                &context_reg,
                "",
                0,
                maximum_word_length,
            );
            assert_eq!(got, "                                       beta!");
        }
    }

    mod execution_tests {
        use super::*;
        use tempfile::NamedTempFile;

        #[test]
        fn test_ptx_exec() {
            // 创建测试配置
            let settings = PtxSettings {
                config: PtxConfig {
                    format: OutFormat::Roff,
                    is_gnu_ext: false,
                    ..Default::default()
                },
                file_map: {
                    vec![test_file_content(
                        "test.txt",
                        vec!["hello test world".to_string()],
                        0,
                    )]
                },
                words: {
                    let mut set = BTreeSet::new();
                    set.insert(test_word_ref("test", 0, 6, 10));
                    set
                },
                output_filename: NamedTempFile::new()
                    .unwrap()
                    .path()
                    .to_str()
                    .unwrap()
                    .to_string(),
            };

            let result = ptx_exec(&settings);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ptx_exec_dumb_format() {
            let settings = PtxSettings {
                config: PtxConfig {
                    format: OutFormat::Dumb,
                    ..Default::default()
                },
                file_map: { vec![test_file_content("test.txt", vec!["test".to_string()], 0)] },
                words: {
                    let mut set = BTreeSet::new();
                    set.insert(test_word_ref("test", 0, 0, 4));
                    set
                },
                output_filename: "-".to_string(),
            };

            let result = ptx_exec(&settings);
            assert!(result.is_ok());
        }
    }

    mod output_chunk_tests {
        use super::*;

        #[test]
        fn test_get_output_chunks_basic() {
            let config = PtxConfig {
                line_width: 20,
                gap_size: 3,
                trunc_str: "/".to_string(),
                ..Default::default()
            };

            let before = &['h', 'e', 'l', 'l', 'o', ' '];
            let keyword = "test";
            let after = &[' ', 'w', 'o', 'r', 'l', 'd'];
            let max_word_len = 5;

            let (tail, before_out, after_out, head) = ptx_get_output_chunks_for_width_with_max(
                before,
                keyword,
                after,
                &config,
                config.line_width,
                max_word_len,
            );

            assert_eq!(tail, "");
            assert_eq!(before_out, "hello");
            assert_eq!(after_out, "/");
            assert_eq!(head, "");
        }

        #[test]
        fn test_get_output_chunks_long_line() {
            let config = PtxConfig {
                line_width: 5, // 设置更小的宽度以确保触发截断
                gap_size: 2,
                trunc_str: "*".to_string(),
                ..Default::default()
            };

            // 使用更长的文本
            let before = &[
                'v', 'e', 'r', 'y', ' ', 'l', 'o', 'n', 'g', ' ', 't', 'e', 'x', 't', ' ',
            ];
            let keyword = "test";
            let after = &[
                ' ', 'h', 'e', 'r', 'e', ' ', 'a', 'n', 'd', ' ', 't', 'h', 'e', 'r', 'e',
            ];
            let max_word_len = 5;

            let (_tail, before_out, after_out, _head) = ptx_get_output_chunks_for_width_with_max(
                before,
                keyword,
                after,
                &config,
                config.line_width,
                max_word_len,
            );

            // 验证长文本被适当截断
            assert!(!before_out.is_empty());
            assert!(!after_out.is_empty());
            assert!(before_out.len() + after_out.len() <= config.line_width);
            assert!(before_out.contains('*') || after_out.contains('*')); // 修改断言检查实际输出部分
        }

        #[test]
        fn test_get_output_chunks_empty_context() {
            let config = PtxConfig::default();

            let before = &[];
            let keyword = "test";
            let after = &[];
            let max_word_len = 4;

            let (tail, before_out, after_out, head) = ptx_get_output_chunks_for_width_with_max(
                before,
                keyword,
                after,
                &config,
                config.line_width,
                max_word_len,
            );

            assert_eq!(tail, "");
            assert_eq!(before_out, "");
            assert_eq!(after_out, "");
            assert_eq!(head, "");
        }

        #[test]
        fn test_get_output_chunks_whitespace() {
            let config = PtxConfig {
                trunc_str: "/".to_string(),
                ..Default::default()
            };

            let before = &[' ', ' ', ' '];
            let keyword = "test";
            let after = &[' ', ' ', ' '];
            let max_word_len = 4;

            let (tail, before_out, after_out, head) = ptx_get_output_chunks_for_width_with_max(
                before,
                keyword,
                after,
                &config,
                config.line_width,
                max_word_len,
            );

            // 验证空白字符被正确处理
            assert_eq!(tail, "");
            assert_eq!(before_out, "");
            assert_eq!(after_out, "");
            assert_eq!(head, "");
        }
    }

    mod input_processing_tests {
        use super::*;
        use tempfile::NamedTempFile;

        #[test]
        fn test_ptx_read_input() {
            // 创建测试文件
            let mut file = NamedTempFile::new().unwrap();
            writeln!(file, "line one\nline two").unwrap();

            let config = PtxConfig {
                is_gnu_ext: false,
                ..Default::default()
            };

            let input_files = vec![file.path().to_str().unwrap().to_string()];
            let result = ptx_read_input(&input_files, &config).unwrap();

            assert_eq!(result.len(), 1);
            let content = &result[0];
            assert_eq!(content.lines, vec!["line one", "line two"]);
            assert_eq!(content.offset, 0);
        }

        #[test]
        fn test_ptx_read_input_multiple_files() {
            let mut file1 = NamedTempFile::new().unwrap();
            let mut file2 = NamedTempFile::new().unwrap();
            writeln!(file1, "file1").unwrap();
            writeln!(file2, "file2").unwrap();

            let config = PtxConfig {
                is_gnu_ext: true, // 允许多文件
                ..Default::default()
            };

            let input_files = vec![
                file1.path().to_str().unwrap().to_string(),
                file2.path().to_str().unwrap().to_string(),
            ];
            let result = ptx_read_input(&input_files, &config).unwrap();

            assert_eq!(result.len(), 2);
        }
    }

    mod word_set_tests {
        use super::*;

        #[test]
        fn test_ptx_create_word_set() {
            let config = PtxConfig {
                is_ignore_case: false,
                is_input_ref: false,
                ..Default::default()
            };

            let filter = WordFilter {
                is_only_specified: false,
                is_ignore_specified: false,
                only_set: HashSet::new(),
                ignore_set: HashSet::new(),
                word_regex: r"\w+".to_string(),
            };

            let file_map = vec![test_file_content(
                "test.txt",
                vec!["hello world".to_string()],
                0,
            )];

            let word_set = ptx_create_word_set(&config, &filter, &file_map);

            assert_eq!(word_set.len(), 2); // "hello" 和 "world"
            assert!(word_set.iter().any(|w| w.word == "hello"));
            assert!(word_set.iter().any(|w| w.word == "world"));
        }

        #[test]
        fn test_ptx_create_word_set_with_ignore_case() {
            let config = PtxConfig {
                is_ignore_case: true,
                ..Default::default()
            };

            let filter = WordFilter {
                word_regex: r"\w+".to_string(),
                ..Default::default()
            };

            let file_map = vec![test_file_content(
                "test.txt",
                vec!["Hello WORLD".to_string()],
                0,
            )];

            let word_set = ptx_create_word_set(&config, &filter, &file_map);

            assert!(word_set.iter().any(|w| w.word == "hello"));
            assert!(word_set.iter().any(|w| w.word == "world"));
        }

        #[test]
        fn test_ptx_create_word_set_skips_input_reference_field() {
            let config = PtxConfig {
                is_input_ref: true,
                context_regex: "\n".to_string(),
                ..Default::default()
            };
            let filter = WordFilter {
                word_regex: r"[A-Za-z]+".to_string(),
                ..Default::default()
            };
            let file_map = vec![test_file_content(
                "test.txt",
                vec![
                    "openssl,https://githubs.com/openssl/openssl.git".to_string(),
                    "ref hello world".to_string(),
                ],
                0,
            )];

            let words: Vec<String> = ptx_create_word_set(&config, &filter, &file_map)
                .into_iter()
                .map(|word_ref| word_ref.word)
                .collect();

            assert!(!words.iter().any(|word| word == "openssl"));
            assert!(!words.iter().any(|word| word == "ref"));
            assert!(words.iter().any(|word| word == "hello"));
            assert!(words.iter().any(|word| word == "world"));
        }
    }

    mod reference_tests {
        use super::*;

        #[test]
        fn test_ptx_get_reference_auto_ref() {
            let config = PtxConfig {
                is_auto_ref: true,
                is_input_ref: false,
                ..Default::default()
            };

            let word_ref = test_word_ref("test", 0, 0, 4);

            let context_reg = Regex::new(&config.context_regex).unwrap();
            let reference =
                ptx_get_reference(&config, &word_ref, "test.txt", "test line", &context_reg);

            assert_eq!(reference, "test.txt:1");
        }

        #[test]
        fn test_ptx_get_reference_input_ref() {
            let config = PtxConfig {
                is_auto_ref: false,
                is_input_ref: true,
                ..Default::default()
            };

            let word_ref = WordRef::default();
            let context_reg = Regex::new(&config.context_regex).unwrap();
            let reference = ptx_get_reference(
                &config,
                &word_ref,
                "test.txt",
                "123 word text",
                &context_reg,
            );

            assert_eq!(reference, "123");
        }
    }
}
