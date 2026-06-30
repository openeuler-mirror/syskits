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
use clap::{Arg, ArgAction, Command, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTError, CTResult, CtSimpleError, FromIo};
use regex::Regex;
use std::cmp;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::error::Error;
use std::ffi::OsString;
use std::fmt::{Display, Formatter, Write as FmtWrite};
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read, Write, stdin, stdout};
use std::num::ParseIntError;
use sys_locale::get_locale;

const REGEX_CHARCLASS: &str = "^-]\\";
const PTX_HELP_TEXT: &str = "Usage: ptx [OPTION]... [INPUT]...   (without -G)\n  or:  ptx -G [OPTION]... [INPUT [OUTPUT]]\nOutput a permuted index, including context, of the words in the input files.\n\nWith no FILE, or when FILE is -, read standard input.\n\nMandatory arguments to long options are mandatory for short options too.\n  -A, --auto-reference           output automatically generated references\n  -G, --traditional              behave more like System V 'ptx'\n  -F, --flag-truncation=STRING   use STRING for flagging line truncations.\n                                 The default is '/'\n  -M, --macro-name=STRING        macro name to use instead of 'xx'\n  -O, --format=roff              generate output as roff directives\n  -R, --right-side-refs          put references at right, not counted in -w\n  -S, --sentence-regexp=REGEXP   for end of lines or end of sentences\n  -T, --format=tex               generate output as TeX directives\n  -W, --word-regexp=REGEXP       use REGEXP to match each keyword\n  -b, --break-file=FILE          word break characters in this FILE\n  -f, --ignore-case              fold lower case to upper case for sorting\n  -g, --gap-size=NUMBER          gap size in columns between output fields\n  -i, --ignore-file=FILE         read ignore word list from FILE\n  -o, --only-file=FILE           read only word list from this FILE\n  -r, --references               first field of each line is a reference\n  -t, --typeset-mode               - not implemented -\n  -w, --width=NUMBER             output width in columns, reference excluded\n      --help        display this help and exit\n      --version     output version information and exit\n\nGNU coreutils online help: <https://www.gnu.org/software/coreutils/>\nReport any translation bugs to <https://translationproject.org/team/>\nFull documentation <https://www.gnu.org/software/coreutils/ptx>\nor available locally via: info '(coreutils) ptx invocation'\n";
const PTX_VERSION_TEXT: &str = "ptx (GNU coreutils) 9.4\nCopyright (C) 2023 Free Software Foundation, Inc.\nLicense GPLv3+: GNU GPL version 3 or later <https://gnu.org/licenses/gpl.html>.\nThis is free software: you are free to change and redistribute it.\nThere is NO WARRANTY, to the extent permitted by law.\n\nWritten by F. Pinard.\n";

#[derive(Debug)]
enum OutFormat {
    Dumb,
    Roff,
    Tex,
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
            context_regex: "\\w+".to_owned(),
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
                    "\\w+".to_owned()
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
            word_regex: r"\w+".to_string(),
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
    /// 所在文件名
    filename: String,
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
    /// 文件的所有行
    lines: Vec<String>,
    /// 每行的字符数组表示，用于快速索引
    chars_lines: Vec<Vec<char>>,
    /// 在所有文件中的行偏移量
    offset: usize,
}

type FileMap = HashMap<String, FileContent>;

/// 从输入文件读取内容并构建文件映射
///
/// # 参数
/// * `input_files` - 输入文件路径列表
/// * `config` - PTX 配置，控制是否启用 GNU 扩展
///
/// # 返回值
/// 返回一个 HashMap，键为文件名，值为文件内容和偏移量
fn ptx_read_input(input_files: &[String], config: &PtxConfig) -> std::io::Result<FileMap> {
    // 初始化文件映射
    let mut file_map: FileMap = HashMap::new();
    let mut files = Vec::new();

    // 确定要处理的文件列表
    if input_files.is_empty() {
        files.push("-"); // 标准输入
    } else if config.is_gnu_ext {
        files.extend(input_files.iter().map(|s| s.as_str())); // GNU 模式：处理所有文件
    } else {
        files.push(&input_files[0]); // 传统模式：只处理第一个文件
    }

    let mut offset: usize = 0;
    for filename in files {
        // 创建文件或标准输入的读取器
        let reader: BufReader<Box<dyn Read>> = BufReader::new(if filename == "-" {
            Box::new(stdin())
        } else {
            Box::new(File::open(filename)?)
        });

        // 读取所有行并转换为字符向量
        let lines: Vec<String> = if config.is_gnu_ext && !config.is_input_ref {
            let all_lines = reader.lines().collect::<std::io::Result<Vec<String>>>()?;
            vec![all_lines.join(" ")]
        } else {
            reader.lines().collect::<std::io::Result<Vec<String>>>()?
        };
        let chars_lines: Vec<Vec<char>> = lines.iter().map(|x| x.chars().collect()).collect();

        let size = lines.len();
        file_map.insert(
            filename.to_owned(),
            FileContent {
                lines,
                chars_lines,
                offset,
            },
        );
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
    // 编译正则表达式
    let reg = compile_regex_lossy(&filter.word_regex);
    let ref_reg = compile_regex_lossy(&config.context_regex);
    let mut word_set: BTreeSet<WordRef> = BTreeSet::new();

    // 遍历每个文件的每一行
    for (file, lines) in file_map {
        let mut count: usize = 0;
        let offs = lines.offset;
        for line in &lines.lines {
            // 获取引用范围（如果启用了输入引用）
            let (ref_beg, ref_end) = match ref_reg.find(line) {
                Some(x) => (x.start(), x.end()),
                None => (0, 0),
            };

            // 查找所有匹配的单词
            for mat in reg.find_iter(line) {
                let (beg, end) = (mat.start(), mat.end());
                // 跳过作为引用的单词
                if config.is_input_ref && ((beg, end) == (ref_beg, ref_end)) {
                    continue;
                }

                let mut word = line[beg..end].to_owned();
                // 应用过滤规则
                if filter.is_only_specified && !filter.only_set.contains(&word) {
                    continue;
                }
                if filter.is_ignore_specified && filter.ignore_set.contains(&word) {
                    continue;
                }

                // 处理大小写
                if config.is_ignore_case {
                    word = word.to_lowercase();
                }

                // 创建并添加单词引用
                word_set.insert(WordRef {
                    word,
                    filename: file.clone(),
                    global_line_nr: offs + count,
                    local_line_nr: count,
                    position: beg,
                    position_end: end,
                });
            }
            count += 1;
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
    line: &str,
    context_reg: &Regex,
) -> String {
    if config.is_auto_ref {
        // 自动引用：文件名:行号
        format!(
            "{}:{}",
            word_ref.filename.maybe_quote(),
            word_ref.local_line_nr + 1
        )
    } else if config.is_input_ref {
        // 输入引用：使用正则表达式匹配的内容
        match context_reg.find(line) {
            Some(x) => line[x.start()..x.end()].to_string(),
            None => String::new(),
        }
    } else {
        String::new()
    }
}

fn assert_str_integrity(s: &[char], beg: usize, end: usize) {
    assert!(beg <= end);
    assert!(end <= s.len());
}

/// 向左调整位置以避免在单词中间截断
///
/// # 参数
/// * `text` - 要处理的字符数组
/// * `begin` - 起始位置
/// * `end` - 结束位置
///
/// # 返回值
/// 返回调整后的起始位置，确保不会在单词中间截断
fn trim_broken_word_left(text: &[char], begin: usize, end: usize) -> usize {
    // 处理边界情况
    if begin == end || begin == 0 || text[begin].is_whitespace() || text[begin - 1].is_whitespace()
    {
        return begin;
    }

    // 如果起始位置在单词中间，向左移动到单词开始或空格
    let mut pos = begin;
    while pos < end && !text[pos].is_whitespace() {
        pos += 1;
    }
    pos
}

fn trim_broken_word_right(s: &[char], beg: usize, end: usize) -> usize {
    assert_str_integrity(s, beg, end);
    if beg == end || end == s.len() || s[end - 1].is_whitespace() || s[end].is_whitespace() {
        return end;
    }
    let mut e = end;
    while beg < e && !s[e - 1].is_whitespace() {
        e -= 1;
    }
    e
}

fn trim_idx(s: &[char], beg: usize, end: usize) -> (usize, usize) {
    assert_str_integrity(s, beg, end);
    let mut b = beg;
    let mut e = end;
    while b < e && s[b].is_whitespace() {
        b += 1;
    }
    while b < e && s[e - 1].is_whitespace() {
        e -= 1;
    }
    (b, e)
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
fn ptx_get_output_chunks(
    all_before: &[char],
    keyword: &str,
    all_after: &[char],
    config: &PtxConfig,
) -> (String, String, String, String) {
    // 1. 计算基础尺寸
    let half_line_size = config.line_width / 2;
    let trunc_len = if config.trunc_str.is_empty() {
        0
    } else {
        config.trunc_str.len()
    };

    // 2. 计算最大允许尺寸（对齐 coreutils fix_output_parameters 逻辑）
    let mut max_before_size = half_line_size.saturating_sub(config.gap_size);
    let mut max_keyafter_size = half_line_size;
    if trunc_len > 0 {
        if config.is_gnu_ext {
            max_before_size = max_before_size.saturating_sub(2 * trunc_len);
            max_keyafter_size = max_keyafter_size.saturating_sub(2 * trunc_len);
        } else {
            max_keyafter_size = max_keyafter_size.saturating_sub(2 * trunc_len + 1);
        }
    }
    let max_after_size = max_keyafter_size.saturating_sub(keyword.len());

    // 3. 预分配字符串缓冲区
    let mut head = String::with_capacity(half_line_size);
    let mut before = String::with_capacity(half_line_size);
    let mut after = String::with_capacity(half_line_size);
    let mut tail = String::with_capacity(half_line_size);

    // 4. 处理 before 块
    // 4.1 找到 before 块的结束位置（去除尾部空白）
    let (_, before_end) = trim_idx(all_before, 0, all_before.len());
    // 4.2 计算 before 块的起始位置
    let before_beg = cmp::max(before_end as isize - max_before_size as isize, 0) as usize;
    // 4.3 避免在单词中间截断
    let before_beg = trim_broken_word_left(all_before, before_beg, before_end);
    // 4.4 去除首尾空白
    let (before_beg, before_end) = trim_idx(all_before, before_beg, before_end);
    // 4.5 提取 before 文本
    let before_str: String = all_before[before_beg..before_end].iter().collect();
    before.push_str(&before_str);
    if before.is_empty() && !all_before.is_empty() && all_before.iter().all(|c| c.is_whitespace()) {
        // Keep one spacer for whitespace-only left context, matching coreutils alignment.
        before.push(' ');
    }

    // 5. 处理 after 块
    // 5.1 计算 after 块的结束位置
    let after_end = cmp::min(max_after_size, all_after.len());
    // 5.2 避免在单词中间截断
    let after_end = trim_broken_word_right(all_after, 0, after_end);
    // 5.3 去除首尾空白
    let (_, after_end) = trim_idx(all_after, 0, after_end);
    // 5.4 提取 after 文本
    let after_str: String = all_after[0..after_end].iter().collect();
    after.push_str(&after_str);

    // 6. 处理 tail 块
    // 6.1 计算 tail 块的最大尺寸
    let max_tail_size = cmp::max(
        max_before_size as isize - before.len() as isize - config.gap_size as isize,
        0,
    ) as usize;
    // 6.2 找到 tail 块的起始位置
    let (tail_beg, _) = trim_idx(all_after, after_end, all_after.len());
    // 6.3 计算 tail 块的结束位置
    let tail_end = cmp::min(all_after.len(), tail_beg + max_tail_size);
    let tail_end = trim_broken_word_right(all_after, tail_beg, tail_end);
    // 6.4 去除首尾空白
    let (tail_beg, tail_end) = trim_idx(all_after, tail_beg, tail_end);
    // 6.5 提取 tail 文本
    let tail_str: String = all_after[tail_beg..tail_end].iter().collect();
    tail.push_str(&tail_str);

    // 7. 处理 head 块
    // 7.1 计算 head 块的最大尺寸
    let max_head_size = cmp::max(
        max_after_size as isize - after.len() as isize - config.gap_size as isize,
        0,
    ) as usize;
    // 7.2 找到 head 块的结束位置
    let (_, head_end) = trim_idx(all_before, 0, before_beg);
    // 7.3 计算 head 块的起始位置
    let head_beg = cmp::max(head_end as isize - max_head_size as isize, 0) as usize;
    let head_beg = trim_broken_word_left(all_before, head_beg, head_end);
    // 7.4 去除首尾空白
    let (head_beg, head_end) = trim_idx(all_before, head_beg, head_end);
    // 7.5 提取 head 文本
    let head_str: String = all_before[head_beg..head_end].iter().collect();
    head.push_str(&head_str);

    // 8. 添加截断标记
    // 8.1 处理右侧截断
    if after_end != all_after.len() && tail_beg == tail_end {
        after.push_str(&config.trunc_str);
    } else if after_end != all_after.len() && tail_end != all_after.len() {
        tail.push_str(&config.trunc_str);
    }
    // 8.2 处理左侧截断
    // Only mark left truncation when we actually dropped non-whitespace context.
    let left_truncated_non_ws = before_beg != 0
        && all_before
            .iter()
            .take(before_beg)
            .any(|c| !c.is_whitespace());
    if left_truncated_non_ws && head_beg == head_end {
        before = format!("{}{}", config.trunc_str, before);
    } else if left_truncated_non_ws && head_beg != 0 {
        head = format!("{}{}", config.trunc_str, head);
    }

    (tail, before, after, head)
}

fn tex_mapper(x: char) -> String {
    match x {
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
    context_reg: &Regex,
    reference: &str,
) -> String {
    let mut output = String::with_capacity(line.len() * 2);

    // 获取关键词前后的文本范围
    let before_start = context_base_start(config, line, chars_line, context_reg);
    let (context_left, context_right) = context_bounds(
        config,
        line,
        context_reg,
        word_ref.position,
        word_ref.position_end,
        before_start,
    );
    let before_chars_trim_idx = (context_left, word_ref.position);
    let after_chars_trim_idx = (word_ref.position_end, context_right);

    // 提取关键词和上下文
    let keyword = &line[word_ref.position..word_ref.position_end];
    let all_before = &chars_line[before_chars_trim_idx.0..before_chars_trim_idx.1];
    let all_after = &chars_line[after_chars_trim_idx.0..after_chars_trim_idx.1];

    // 获取格式化后的文本块
    let (tail, before, after, head) = ptx_get_output_chunks(all_before, keyword, all_after, config);

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
    s.replace('\"', "\"\"")
}

/// 格式化输出为 Roff 格式
fn ptx_format_roff_line(
    config: &PtxConfig,
    word_ref: &WordRef,
    line: &str,
    chars_line: &[char],
    context_reg: &Regex,
    reference: &str,
) -> String {
    let mut output = String::with_capacity(line.len() * 2);
    write!(output, ".{}", config.macro_name).unwrap();

    // 获取关键词前后的文本范围
    let before_start = context_base_start(config, line, chars_line, context_reg);
    let (context_left, context_right) = context_bounds(
        config,
        line,
        context_reg,
        word_ref.position,
        word_ref.position_end,
        before_start,
    );
    let before_chars_trim_idx = (context_left, word_ref.position);
    let after_chars_trim_idx = (word_ref.position_end, context_right);

    // 提取关键词和上下文
    let keyword = &line[word_ref.position..word_ref.position_end];
    let all_before = &chars_line[before_chars_trim_idx.0..before_chars_trim_idx.1];
    let all_after = &chars_line[after_chars_trim_idx.0..after_chars_trim_idx.1];

    // 获取格式化后的文本块
    let (tail, before, after, head) = ptx_get_output_chunks(all_before, keyword, all_after, config);

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

fn context_base_start(
    config: &PtxConfig,
    line: &str,
    chars_line: &[char],
    context_reg: &Regex,
) -> usize {
    if config.is_input_ref {
        match context_reg.find(line) {
            Some(m) => {
                let mut idx = m.end();
                while idx < chars_line.len() && chars_line[idx].is_whitespace() {
                    idx += 1;
                }
                idx
            }
            None => 0,
        }
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
    context_reg: &Regex,
    reference: &str,
    reference_max_width: usize,
) -> String {
    let mut output = String::with_capacity(line.len() * 2);
    let before_start = context_base_start(config, line, chars_line, context_reg);
    let (context_left, context_right) = context_bounds(
        config,
        line,
        context_reg,
        word_ref.position,
        word_ref.position_end,
        before_start,
    );
    let before_chars_trim_idx = (context_left, word_ref.position);
    let after_chars_trim_idx = (word_ref.position_end, context_right);
    let keyword = &line[word_ref.position..word_ref.position_end];
    let all_before = &chars_line[before_chars_trim_idx.0..before_chars_trim_idx.1];
    let all_after = &chars_line[after_chars_trim_idx.0..after_chars_trim_idx.1];
    let (tail, before, after, head) = ptx_get_output_chunks(all_before, keyword, all_after, config);
    let keyafter = format!("{keyword}{after}");
    let gap_size = config.gap_size;
    let mut effective_line_width = config.line_width;
    if (config.is_auto_ref || config.is_input_ref) && !config.is_right_ref {
        effective_line_width = effective_line_width.saturating_sub(reference_max_width + gap_size);
    }
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
    if !tail.is_empty() {
        output.push_str(&tail);
        let pad = half_line_width
            .saturating_sub(gap_size)
            .saturating_sub(before_len)
            .saturating_sub(tail_len);
        output.push_str(&" ".repeat(pad));
    } else {
        let before_space_adjust = if config.is_gnu_ext
            && before.is_empty()
            && word_ref.position > before_start
            && chars_line[word_ref.position - 1].is_whitespace()
            && half_line_width <= gap_size + config.trunc_str.len() * 2
        {
            1
        } else {
            0
        };
        let trunc_only_adjust = if config.is_gnu_ext
            && before_is_only_trunc
            && word_ref.position > before_start
            && chars_line[word_ref.position - 1].is_whitespace()
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

    output.push_str(&before);
    output.push_str(&" ".repeat(gap_size));
    output.push_str(&keyafter);

    let keyafter_len = str_cols(&keyafter);
    let head_len = str_cols(&head);
    if !head.is_empty() {
        let pad = half_line_width
            .saturating_sub(keyafter_len)
            .saturating_sub(head_len);
        output.push_str(&" ".repeat(pad));
        output.push_str(&head);
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
            let file_map_value = settings
                .file_map
                .get(&word_ref.filename)
                .expect("Missing file in file map");
            let reference = ptx_get_reference(
                &settings.config,
                word_ref,
                &file_map_value.lines[word_ref.local_line_nr],
                &context_reg,
            );
            reference_max_width = reference_max_width.max(str_cols(&reference));
        }
    }

    for word_ref in &settings.words {
        let file_map_value = settings
            .file_map
            .get(&word_ref.filename)
            .expect("Missing file in file map");

        let reference = ptx_get_reference(
            &settings.config,
            word_ref,
            &file_map_value.lines[word_ref.local_line_nr],
            &context_reg,
        );

        let output_line = match settings.config.format {
            OutFormat::Tex => ptx_format_tex_line(
                &settings.config,
                word_ref,
                &file_map_value.lines[word_ref.local_line_nr],
                &file_map_value.chars_lines[word_ref.local_line_nr],
                &context_reg,
                &reference,
            ),
            OutFormat::Roff => ptx_format_roff_line(
                &settings.config,
                word_ref,
                &file_map_value.lines[word_ref.local_line_nr],
                &file_map_value.chars_lines[word_ref.local_line_nr],
                &context_reg,
                &reference,
            ),
            OutFormat::Dumb => ptx_format_dumb_line(
                &settings.config,
                word_ref,
                &file_map_value.lines[word_ref.local_line_nr],
                &file_map_value.chars_lines[word_ref.local_line_nr],
                &context_reg,
                &reference,
                reference_max_width,
            ),
        };

        writeln!(writer, "{output_line}").map_err_context(String::new)?;
    }
    Ok(())
}

pub fn ptx_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let args: Vec<OsString> = args.collect();
    if args.len() == 2 {
        if let Some(arg1) = args[1].to_str() {
            if arg1 == "--help" {
                print!("{PTX_HELP_TEXT}");
                return Ok(());
            }
            if arg1 == "--version" || arg1 == "-V" {
                print!("{PTX_VERSION_TEXT}");
                return Ok(());
            }
        }
    }
    let settings = PtxSettings::new(args.into_iter())?;
    ptx_exec(&settings)
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
    /// 从命令行参数创建 PTX 设置
    ///
    /// # 参数
    /// * `args` - 命令行参数
    ///
    /// # 返回值
    /// 成功返回 PTX 设置及其所需的数据结构，失败返回错误
    fn new(args: impl ctcore::Args) -> CTResult<Self> {
        // 解析命令行参数
        let matches = ct_app().try_get_matches_from(args)?;

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
            .hide(true)
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
        .infer_long_args(true)
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
            assert_eq!(filter.word_regex, "\\w+");
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
            let word1 = WordRef {
                word: "test".to_string(),
                global_line_nr: 1,
                local_line_nr: 1,
                position: 0,
                position_end: 4,
                filename: "test.txt".to_string(),
            };

            let word2 = WordRef {
                word: "test".to_string(),
                global_line_nr: 2,
                local_line_nr: 1,
                position: 0,
                position_end: 4,
                filename: "test.txt".to_string(),
            };

            assert!(word1 < word2);
        }
    }

    mod string_manipulation_tests {
        use super::*;

        #[test]
        fn test_trim_broken_word_right() {
            let s: Vec<char> = "hello world".chars().collect();
            assert_eq!(trim_broken_word_right(&s, 0, 7), 6); // "hello"
            assert_eq!(trim_broken_word_right(&s, 6, 11), 11); // "world"
        }

        #[test]
        fn test_trim_idx() {
            let s: Vec<char> = "  hello  ".chars().collect();
            assert_eq!(trim_idx(&s, 0, 8), (2, 7));
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

            let word_ref = WordRef {
                word: "test".to_string(),
                global_line_nr: 1,
                local_line_nr: 1,
                position: 6,
                position_end: 10,
                filename: "test.txt".to_string(),
            };

            let line = "hello test world";
            let chars_line: Vec<char> = line.chars().collect();
            let reference = "1";
            let context_reg = compile_regex_lossy(&config.context_regex);

            let result = ptx_format_roff_line(
                &config,
                &word_ref,
                line,
                &chars_line,
                &context_reg,
                reference,
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
            let word_ref = WordRef {
                word: "bar".to_string(),
                global_line_nr: 0,
                local_line_nr: 0,
                position: 4,
                position_end: 7,
                filename: "test.txt".to_string(),
            };
            let context_reg = compile_regex_lossy(&config.context_regex);
            let got =
                ptx_format_dumb_line(&config, &word_ref, line, &chars_line, &context_reg, "", 0);
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
            let word_ref = WordRef {
                word: "beta".to_string(),
                global_line_nr: 0,
                local_line_nr: 0,
                position: 7,
                position_end: 11,
                filename: "test.txt".to_string(),
            };
            let context_reg = compile_regex_lossy(&config.context_regex);
            let got =
                ptx_format_dumb_line(&config, &word_ref, line, &chars_line, &context_reg, "", 0);
            assert_eq!(got, "                                        beta!");
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
                    let mut map = FileMap::new();
                    map.insert(
                        "test.txt".to_string(),
                        FileContent {
                            lines: vec!["hello test world".to_string()],
                            chars_lines: vec!["hello test world".chars().collect()],
                            offset: 0,
                        },
                    );
                    map
                },
                words: {
                    let mut set = BTreeSet::new();
                    set.insert(WordRef {
                        word: "test".to_string(),
                        global_line_nr: 1,
                        local_line_nr: 0,
                        position: 6,
                        position_end: 10,
                        filename: "test.txt".to_string(),
                    });
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
                file_map: {
                    let mut map = FileMap::new();
                    map.insert(
                        "test.txt".to_string(),
                        FileContent {
                            lines: vec!["test".to_string()],
                            chars_lines: vec!["test".chars().collect()],
                            offset: 0,
                        },
                    );
                    map
                },
                words: {
                    let mut set = BTreeSet::new();
                    set.insert(WordRef {
                        word: "test".to_string(),
                        global_line_nr: 1,
                        local_line_nr: 0,
                        position: 0,
                        position_end: 4,
                        filename: "test.txt".to_string(),
                    });
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

            let (tail, before_out, after_out, head) =
                ptx_get_output_chunks(before, keyword, after, &config);

            assert_eq!(tail, "");
            assert_eq!(before_out, "hello");
            assert_eq!(after_out, " /");
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

            let (_tail, before_out, after_out, _head) =
                ptx_get_output_chunks(before, keyword, after, &config);

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

            let (tail, before_out, after_out, head) =
                ptx_get_output_chunks(before, keyword, after, &config);

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

            let (tail, before_out, after_out, head) =
                ptx_get_output_chunks(before, keyword, after, &config);

            // 验证空白字符被正确处理
            assert_eq!(tail, "");
            assert_eq!(before_out, " ");
            assert_eq!(after_out, "   ");
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
            let content = result.get(file.path().to_str().unwrap()).unwrap();
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

            let mut file_map = FileMap::new();
            file_map.insert(
                "test.txt".to_string(),
                FileContent {
                    lines: vec!["hello world".to_string()],
                    chars_lines: vec!["hello world".chars().collect()],
                    offset: 0,
                },
            );

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

            let mut file_map = FileMap::new();
            file_map.insert(
                "test.txt".to_string(),
                FileContent {
                    lines: vec!["Hello WORLD".to_string()],
                    chars_lines: vec!["Hello WORLD".chars().collect()],
                    offset: 0,
                },
            );

            let word_set = ptx_create_word_set(&config, &filter, &file_map);

            assert!(word_set.iter().any(|w| w.word == "hello"));
            assert!(word_set.iter().any(|w| w.word == "world"));
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

            let word_ref = WordRef {
                word: "test".to_string(),
                filename: "test.txt".to_string(),
                local_line_nr: 0,
                global_line_nr: 1,
                position: 0,
                position_end: 4,
            };

            let context_reg = Regex::new(&config.context_regex).unwrap();
            let reference = ptx_get_reference(&config, &word_ref, "test line", &context_reg);

            assert_eq!(reference, "test.txt:1");
        }

        #[test]
        fn test_ptx_get_reference_input_ref() {
            let config = PtxConfig {
                is_auto_ref: false,
                is_input_ref: true,
                context_regex: r"\d+".to_string(),
                ..Default::default()
            };

            let word_ref = WordRef::default();
            let context_reg = Regex::new(&config.context_regex).unwrap();
            let reference = ptx_get_reference(&config, &word_ref, "word 123 text", &context_reg);

            assert_eq!(reference, "123");
        }
    }

    mod text_manipulation_tests {
        use super::*;

        #[test]
        fn test_trim_broken_word_left() {
            let text: Vec<char> = "one two three".chars().collect();

            // 测试在单词中间的情况
            assert_eq!(trim_broken_word_left(&text, 2, text.len()), 3); // "one"的末尾

            // 测试在空格处的情况
            assert_eq!(trim_broken_word_left(&text, 4, text.len()), 4); // 空格位置

            // 测试在开头的情况
            assert_eq!(trim_broken_word_left(&text, 0, text.len()), 0);

            // 测试空字符串
            let empty: Vec<char> = vec![];
            assert_eq!(trim_broken_word_left(&empty, 0, 0), 0);
        }

        #[test]
        fn test_trim_broken_word_left_with_multiple_spaces() {
            let text: Vec<char> = "one   two".chars().collect();
            assert_eq!(trim_broken_word_left(&text, 5, text.len()), 5);
        }
    }
}
