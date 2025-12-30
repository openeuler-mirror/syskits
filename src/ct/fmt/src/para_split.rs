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

use std::io::{BufRead, Lines};
use std::iter::Peekable;
use std::slice::Iter;

use unicode_width::UnicodeWidthChar;

use crate::FmtConfigs;
use crate::FmtFileOrStdReader;

fn fmt_char_width(c: char) -> usize {
    match (c as usize) < 0xA0 {
        true => {
            // 如果是 ASCII 码，调用时宽度正好为 1（包括控制字符）
            // 调用控制字符的宽度为 1 与 OpenBSD fmt 一致
            1
        }
        false => {
            // 否则，获取 unicode 宽度
            // 注意，实际上我们不应该在这里得到 None，因为只有 c < 0xA0
            // 可以返回 None，但为了安全和面向未来，我们这样做
            UnicodeWidthChar::width(c).unwrap_or(1)
        }
    }
}

// 带有 PSKIP、缺少 PREFIX 或完全空白的行为 NoFormatLines（无格式行），否则为 FormatLines（格式行
#[derive(Debug, Clone)]
pub enum FmtLine {
    FormatLine(FmtFileLine),
    NoFormatLine(String, bool),
}

impl FmtLine {
    // 当我们知道它是一个格式行时，就像在段落流迭代器中一样
    fn get_format_line(self) -> FmtFileLine {
        match self {
            FmtLine::FormatLine(fl) => fl,
            FmtLine::NoFormatLine(..) => panic!("Found NoFormatLine when expecting FormatLine"),
        }
    }

    // 当我们知道它是 NoFormatLine 时，就像在 ParagraphStream 迭代器中一样
    fn get_no_format_line(self) -> (String, bool) {
        match self {
            FmtLine::NoFormatLine(s, b) => (s, b),
            FmtLine::FormatLine(..) => panic!("Found FormatLine when expecting NoFormatLine"),
        }
    }
}

/// 必须考虑每一行的前缀，以确定是否将其与下一行合并
#[derive(Debug, Clone)]
pub struct FmtFileLine {
    line: String,
    /// 缩进的终点，总是文本的起点
    indent_end: usize,
    /// PREFIX 缩进的末尾，即前缀前的空格
    prefix_indent_end: usize,
    /// 显示缩进的长度，同时考虑制表符
    indent_len: usize,
    /// 考虑到制表符的 PREFIX 缩进长度
    prefix_len: usize,
}

/// 从文件中产生行数据流的迭代器
pub struct FmtFileLines<'a> {
    opts: &'a FmtConfigs,
    lines: Lines<&'a mut FmtFileOrStdReader>,
}

impl<'a> FmtFileLines<'a> {
    fn new<'b>(opts: &'b FmtConfigs, lines: Lines<&'b mut FmtFileOrStdReader>) -> FmtFileLines<'b> {
        FmtFileLines { opts, lines }
    }

    /// 如果该行应被格式化，则返回 true
    fn match_prefix(&self, line: &str) -> (bool, usize) {
        if let Some(prefix) = &self.opts.prefix_option {
            FmtFileLines::match_prefix_generic(prefix, line, self.opts.is_xprefix)
        } else {
            (true, 0)
        }
    }

    /// 如果该行应被格式化，则返回 true
    fn match_anti_prefix(&self, line: &str) -> bool {
        if let Some(anti_prefix) = &self.opts.anti_prefix_option {
            match FmtFileLines::match_prefix_generic(anti_prefix, line, self.opts.is_xanti_prefix) {
                (true, _) => false,
                (_, _) => true,
            }
        } else {
            true
        }
    }

    fn match_prefix_generic(pfx: &str, line: &str, exact: bool) -> (bool, usize) {
        if line.starts_with(pfx) {
            (true, 0)
        } else {
            if !exact {
                // 我们采用这种方式，而不是字节索引，以支持 unicode 白区字符
                for (i, char) in line.char_indices() {
                    if line[i..].starts_with(pfx) {
                        return (true, i);
                    } else if !char.is_whitespace() {
                        break;
                    }
                }
            }
            (false, 0)
        }
    }

    fn compute_indent(&self, string: &str, prefix_end: usize) -> (usize, usize, usize) {
        let mut prefix_len = 0;
        let mut indent_len = 0;
        let mut indent_end = 0;
        for (os, c) in string.char_indices() {
            if os == prefix_end {
                // 我们找到了前缀的末尾，因此这里打印的就是前缀的长度
                prefix_len = indent_len;
            }

            if (os >= prefix_end) && !c.is_whitespace() {
                // 发现前缀后第一个非空格，这是 indent_end
                indent_end = os;
                break;
            } else if c == '\t' {
                // 计算制表符长度
                indent_len = (indent_len / self.opts.tab_width + 1) * self.opts.tab_width;
            } else {
                // 非制表符
                indent_len += fmt_char_width(c);
            }
        }
        (indent_end, prefix_len, indent_len)
    }
}

impl<'a> Iterator for FmtFileLines<'a> {
    type Item = FmtLine;

    fn next(&mut self) -> Option<FmtLine> {
        let n = self.lines.next()?.ok()?;

        // 如果这一行完全是空白、
        // 发送一个空行
        // Err(true) 表示这是一个换行符、
        // 在检测邮件头时必须知道这一点
        if n.chars().all(char::is_whitespace) {
            return Some(FmtLine::NoFormatLine(String::new(), true));
        }

        let (is_p_match, p_offset) = self.match_prefix(&n[..]);

        // 如果该行与前缀不匹配、
        // 未处理该行，并再次迭代
        if !is_p_match {
            return Some(FmtLine::NoFormatLine(n, false));
        }

        // 如果行符合前缀，但后面是空白、 不允许通过它合并行（也就是说，把它当作空行来处理，
        // 但由于它不是真正的空行，我们将不允许在下面一行中使用邮件标题）
        if is_p_match
            && n[p_offset + self.opts.prefix_option.as_ref().map_or(0, |s| s.len())..]
                .chars()
                .all(char::is_whitespace)
        {
            return Some(FmtLine::NoFormatLine(n, false));
        }

        // 如果该行匹配反前缀，则跳过
        //（注意，如果要处理，match_anti_prefix 的定义为 TRUE）
        if !self.match_anti_prefix(&n[..]) {
            return Some(FmtLine::NoFormatLine(n, false));
        }

        // 计算出缩进点、前缀和前缀缩进结束点
        let prefix_end = p_offset + self.opts.prefix_option.as_ref().map_or(0, |s| s.len());
        let (indent_end, prefix_len, indent_len) = self.compute_indent(&n[..], prefix_end);

        Some(FmtLine::FormatLine(FmtFileLine {
            line: n,
            indent_end,
            prefix_indent_end: p_offset,
            indent_len,
            prefix_len,
        }))
    }
}

/// 段落：需要格式化的文件行集合 加上关于段落缩进的信息
///
/// 我们只保留文件行中的字符串，其他信息只是为了帮助我们决定如何将文件行合并到段落中。
#[derive(Debug)]
pub struct FmtParagraph {
    /// 文件的行数
    pub lines: Vec<String>,
    /// 表示初始值的字符串，即第一行的缩进值
    pub init_str: String,
    /// 考虑到 TABWIDTH，初始字符串的可打印长度
    pub init_len: usize,
    /// 初始字符串在第一行结束的字节位置 String
    pub init_end: usize,
    /// 表示缩进的字符串
    pub indent_str: String,
    /// length of above
    pub indent_len: usize,
    /// 缩进结束的字节位置（在冠模式和标记模式下，仅适用于第二行及以后的行）
    pub indent_end: usize,
    /// 我们需要知道这是否是邮件头，因为在这种情况下，我们会以不同的方式分词
    pub mail_header: bool,
}

/// 一个迭代器，在给定一组选项的情况下，从行列流中产生段落流。
pub struct FmtParagraphStream<'a> {
    lines: Peekable<FmtFileLines<'a>>,
    next_mail: bool,
    opts: &'a FmtConfigs,
}

impl<'a> FmtParagraphStream<'a> {
    pub fn new<'b>(
        opts: &'b FmtConfigs,
        reader: &'b mut FmtFileOrStdReader,
    ) -> FmtParagraphStream<'b> {
        let lines = FmtFileLines::new(opts, reader.lines()).peekable();
        // 在文件开头，我们可能会发现邮件头
        FmtParagraphStream {
            lines,
            next_mail: true,
            opts,
        }
    }

    /// 检测 RFC822 邮件头
    fn is_mail_header(file_line: &FmtFileLine) -> bool {
        // 邮件标题以 "发件人"（信封发件人行）开头 或以一串可打印的 ASCII 字符（33 至 126，含括在内，冒号除外）开头，后接冒号。
        match file_line.indent_end > 0 {
            true => false,
            false => {
                let l_slice = &file_line.line[..];
                if l_slice.starts_with("From ") {
                    true
                } else {
                    let colon_posn = if let Some(n) = l_slice.find(':') {
                        n
                    } else {
                        return false;
                    };

                    // 标头字段长度必须不为零
                    if colon_posn == 0 {
                        return false;
                    }

                    l_slice[..colon_posn]
                        .chars()
                        .all(|x| !matches!(x as usize, y if !(33..=126).contains(&y)))
                }
            }
        }
    }
}

impl<'a> Iterator for FmtParagraphStream<'a> {
    type Item = Result<FmtParagraph, String>;

    #[allow(clippy::cognitive_complexity)]
    fn next(&mut self) -> Option<Result<FmtParagraph, String>> {
        // 在 Err 中返回 NoFormatLine；应立即输出
        let is_no_format = match self.lines.peek()? {
            FmtLine::FormatLine(_) => false,
            FmtLine::NoFormatLine(_, _) => true,
        };

        // 发现 NoFormatLine，立即将其转出
        if is_no_format {
            let (s, is_nm) = self.lines.next().unwrap().get_no_format_line();
            self.next_mail = is_nm;
            return Some(Err(s));
        }

        // 发现一个格式行，现在建立一个段落
        let mut init_str = String::new();
        let mut init_end = 0;
        let mut init_len = 0;
        let mut indent_str = String::new();
        let mut indent_end = 0;
        let mut indent_len = 0;
        let mut prefix_len = 0;
        let mut prefix_indent_end = 0;
        let mut p_lines = Vec::new();

        let mut is_in_mail = false;
        let mut is_second_done = false; // 当我们使用冠模式或标记模式时
        loop {
            // peek ahead
            // 在调用 self.lines.next() 之前，需要明确地强制 fl 退出作用域
            let Some(FmtLine::FormatLine(fl)) = self.lines.peek() else {
                break;
            };

            if p_lines.is_empty() {
                // 第一次通过循环，进行设置
                // 检测邮件头
                if self.opts.is_mail && self.next_mail && FmtParagraphStream::is_mail_header(fl) {
                    is_in_mail = true;
                    // 不能有任何缩进或前缀缩进，否则is_mail_header就会失败，因为在有效的邮件头字段中，
                    // 冒号前不能有任何空白。就会失败，因为在一个有效的邮件头字段中，冒号前不能有任何空白。
                    indent_str.push_str("  ");
                    indent_len = 2;
                } else {
                    if self.opts.is_crown || self.opts.is_tagged {
                        init_str.push_str(&fl.line[..fl.indent_end]);
                        init_len = fl.indent_len;
                        init_end = fl.indent_end;
                    } else {
                        is_second_done = true;
                    }

                    // 这些内容将在冠模式或标记模式的第二行被覆盖，但我们不能保证一定能写到第二行，
                    // 例如，如果下一行是 NoFormatLine 或 None。因此，我们在第一次运行时设置了合理的默认值
                    indent_str.push_str(&fl.line[..fl.indent_end]);
                    indent_len = fl.indent_len;
                    indent_end = fl.indent_end;

                    // 保存这些内容，以检查是否有匹配的行
                    prefix_len = fl.prefix_len;
                    prefix_indent_end = fl.prefix_indent_end;

                    // 在标记模式下，默认情况下增加 4 个额外的缩进空格（gnu fmt 的行为与此不同：它似乎会找到最靠近
                    // indent_end 的一列，且该列能被 3 整除。也许更好的默认值是 1 TABWIDTH？不过这也太大了。
                    if self.opts.is_tagged {
                        indent_str.push_str("    ");
                        indent_len += 4;
                    }
                }
            } else if is_in_mail {
                // 邮件头后面的行必须以空格开头
                if fl.indent_end == 0
                    || (self.opts.prefix_option.is_some() && fl.prefix_indent_end == 0)
                {
                    break; // 这一行不以空格开头
                }
            } else if !is_second_done {
                // 现在我们有足够的信息来处理冠状边缘和标记模式

                // 在冠状边缘和标记模式下，我们都要求前缀_len 相同
                if prefix_len != fl.prefix_len || prefix_indent_end != fl.prefix_indent_end {
                    break;
                }

                // 在标记模式下，后面几行的缩进必须*different*。
                if self.opts.is_tagged
                    && indent_len - 4 == fl.indent_len
                    && indent_end == fl.indent_end
                {
                    break;
                }

                // 这是同一段落的一部分，从这一行获取缩进信息
                indent_str.clear();
                indent_str.push_str(&fl.line[..fl.indent_end]);
                indent_len = fl.indent_len;
                indent_end = fl.indent_end;

                is_second_done = true;
            } else {
                // 检测不匹配
                if indent_end != fl.indent_end
                    || prefix_indent_end != fl.prefix_indent_end
                    || indent_len != fl.indent_len
                    || prefix_len != fl.prefix_len
                {
                    break;
                }
            }

            p_lines.push(self.lines.next().unwrap().get_format_line().line);

            // 当我们处于纯分割模式时，我们从不连接行，所以到此为止
            if self.opts.is_split_only {
                break;
            }
        }

        // 如果这是一个邮件头，那么下一行可以被检测为邮件头。否则就不能检测。
        // 注意 next_mail 在 ParagraphStream 实例化时为 true，并在空白 NoFormatLine 后设置为 true。
        self.next_mail = is_in_mail;

        Some(Ok(FmtParagraph {
            lines: p_lines,
            init_str,
            init_len,
            init_end,
            indent_str,
            indent_len,
            indent_end,
            mail_header: is_in_mail,
        }))
    }
}

pub struct FmtParaWords<'a> {
    opts: &'a FmtConfigs,
    para: &'a FmtParagraph,
    words: Vec<FmtWordInfo<'a>>,
}

impl<'a> FmtParaWords<'a> {
    pub fn new(fmt_opts: &'a FmtConfigs, para: &'a FmtParagraph) -> Self {
        let mut pw = FmtParaWords {
            opts: fmt_opts,
            para,
            words: Vec::new(),
        };
        pw.create_words();
        pw
    }

    fn create_words(&mut self) {
        if self.para.mail_header {
            // 邮件标题没有额外的间距；邮件标题的每一行都有 1 个安全的空格，因为第一行保证没有任何空格。
            self.words.extend(
                self.para
                    .lines
                    .iter()
                    .flat_map(|x| x.split_whitespace())
                    .map(|x| FmtWordInfo {
                        word: x,
                        word_start: 0,
                        word_nchars: x.len(), // 确定邮件头；只允许使用 ASCII 编码（unicode 已转义）
                        before_tab: None,
                        after_tab: 0,
                        is_sentence_start: false,
                        is_ends_punct: false,
                        is_new_line: false,
                    }),
            );
        } else {
            // 第一行
            self.words
                .extend(if self.opts.is_crown || self.opts.is_tagged {
                    // 冠模式和标记模式的第一行是 "init"，因此从这里开始切分
                    FmtWordSplit::new(self.opts, &self.para.lines[0][self.para.init_end..])
                } else {
                    // 否则，我们从缩进处开始切分
                    FmtWordSplit::new(self.opts, &self.para.lines[0][self.para.indent_end..])
                });

            if self.para.lines.len() > 1 {
                let indent_end = self.para.indent_end;
                let opts = self.opts;
                self.words.extend(
                    self.para
                        .lines
                        .iter()
                        .skip(1)
                        .flat_map(|x| FmtWordSplit::new(opts, &x[indent_end..])),
                );
            }
        }
    }

    pub fn words(&'a self) -> Iter<'a, FmtWordInfo<'a>> {
        self.words.iter()
    }
}

struct FmtWordSplit<'a> {
    opts: &'a FmtConfigs,
    string: &'a str,
    length: usize,
    position: usize,
    is_prev_punct: bool,
}

impl<'a> FmtWordSplit<'a> {
    fn analyze_tabs(&self, string: &str) -> (Option<usize>, usize, Option<usize>) {
        // 给定一个字符串，确定（制表符前的长度）和（第一个制表符后的打印长度）
        // 如果没有制表符，则 beforetab =-1，aftertab 为打印长度
        let mut before_tab = None;
        let mut after_tab = 0;
        let mut word_start = None;
        for (os, c) in string.char_indices() {
            if !c.is_whitespace() {
                word_start = Some(os);
                break;
            } else if c == '\t' {
                if before_tab.is_none() {
                    before_tab = Some(after_tab);
                    after_tab = 0;
                } else {
                    after_tab = (after_tab / self.opts.tab_width + 1) * self.opts.tab_width;
                }
            } else {
                after_tab += 1;
            }
        }
        (before_tab, after_tab, word_start)
    }
}

impl<'a> FmtWordSplit<'a> {
    fn new<'b>(fmt_opts: &'b FmtConfigs, string: &'b str) -> FmtWordSplit<'b> {
        // 分词 *must* 以非空格字符开始
        let trim_string = string.trim_start();
        FmtWordSplit {
            opts: fmt_opts,
            string: trim_string,
            length: string.len(),
            position: 0,
            is_prev_punct: false,
        }
    }

    fn is_punctuation(c: char) -> bool {
        matches!(c, '!' | '.' | '?')
    }
}

#[derive(PartialEq, Debug)]
pub struct FmtWordInfo<'a> {
    pub word: &'a str,
    pub word_start: usize,
    pub word_nchars: usize,
    pub before_tab: Option<usize>,
    pub after_tab: usize,
    pub is_sentence_start: bool,
    pub is_ends_punct: bool,
    pub is_new_line: bool,
}

// 返回 (&str，is_start_of_sentence)
impl<'a> Iterator for FmtWordSplit<'a> {
    type Item = FmtWordInfo<'a>;

    fn next(&mut self) -> Option<FmtWordInfo<'a>> {
        if self.position >= self.length {
            return None;
        }

        let old_position = self.position;
        let is_new_line = old_position == 0;

        // 查找下一个单词的开头，并记录是否找到制表符
        let (before_tab, after_tab, word_start) =
            match self.analyze_tabs(&self.string[old_position..]) {
                (b, a, Some(s)) => (b, a, s + old_position),
                (_, _, None) => {
                    self.position = self.length;
                    return None;
                }
            };

        // 查找下一个空白字符的起始位置 注意，这样做保留了 self.position 指向空白字符或字符串末尾的不变性
        let mut word_n_chars = 0;
        self.position = match self.string[word_start..].find(|x: char| {
            if x.is_whitespace() {
                true
            } else {
                word_n_chars += fmt_char_width(x);
                false
            }
        }) {
            None => self.length,
            Some(s) => s + word_start,
        };

        let word_start_relative = word_start - old_position;
        // 如果上一句是标点符号，而这一句有 >2 个空白或一个制表符，则是一个新句子。
        let is_start_of_sentence =
            self.is_prev_punct && (before_tab.is_some() || word_start_relative > 1);

        // 现在记录该词是否以标点符号结尾
        self.is_prev_punct = if let Some(ch) = self.string[..self.position].chars().next_back() {
            FmtWordSplit::is_punctuation(ch)
        } else {
            panic!("fatal: expected word not to be empty")
        };

        let (word, word_start_relative, before_tab, after_tab) = match self.opts.is_uniform {
            true => (&self.string[word_start..self.position], 0, None, 0),
            false => (
                &self.string[old_position..self.position],
                word_start_relative,
                before_tab,
                after_tab,
            ),
        };

        Some(FmtWordInfo {
            word,
            word_start: word_start_relative,
            word_nchars: word_n_chars,
            before_tab,
            after_tab,
            is_sentence_start: is_start_of_sentence,
            is_ends_punct: self.is_prev_punct,
            is_new_line,
        })
    }
}

