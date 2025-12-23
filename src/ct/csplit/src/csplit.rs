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
#![allow(rustdoc::private_intra_doc_links)]
#![allow(rustdoc::private_intra_doc_links)]

use std::cmp::Ordering;
use std::io::{self, BufReader};
use std::{
    fs::{remove_file, File},
    io::{BufRead, BufWriter, Write},
};

use clap::{crate_version, Arg, ArgAction, ArgMatches, Command};
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, FromIo};
use ctcore::{ct_crash_if_err, ct_format_usage, ct_help_about, ct_help_section, ct_help_usage};
use regex::Regex;

mod csplit_error;
mod patterns;
mod split_name;

use crate::csplit_error::CsplitError;
use crate::split_name::SplitName;

const CSPLIT_ABOUT: &str = ct_help_about!("csplit.md");
const AFTER_HELP: &str = ct_help_section!("after help", "csplit.md");
const CSPLIT_USAGE: &str = ct_help_usage!("csplit.md");

mod opt_flags {
    pub const SUFFIX_FORMAT: &str = "suffix-format";
    pub const SUPPRESS_MATCHED: &str = "suppress-matched";
    pub const DIGITS: &str = "digits";
    pub const PREFIX: &str = "prefix";
    pub const KEEP_FILES: &str = "keep-files";
    pub const QUIET: &str = "quiet";
    pub const ELIDE_EMPTY_FILES: &str = "elide-empty-files";
    pub const FILE: &str = "file";
    pub const PATTERN: &str = "pattern";
}

/// Command line options for csplit.
pub struct CsplitOptions {
    split_name: crate::SplitName,
    keep_files: bool,
    quiet: bool,
    elide_empty_files: bool,
    suppress_matched: bool,
}

impl CsplitOptions {
    fn new(matches: &ArgMatches) -> Self {
        let keep_files = matches.get_flag(opt_flags::KEEP_FILES);
        let quiet = matches.get_flag(opt_flags::QUIET);
        let elide_empty_files = matches.get_flag(opt_flags::ELIDE_EMPTY_FILES);
        let suppress_matched = matches.get_flag(opt_flags::SUPPRESS_MATCHED);

        Self {
            split_name: ct_crash_if_err!(
                1,
                SplitName::new(
                    matches.get_one::<String>(opt_flags::PREFIX).cloned(),
                    matches.get_one::<String>(opt_flags::SUFFIX_FORMAT).cloned(),
                    matches.get_one::<String>(opt_flags::DIGITS).cloned()
                )
            ),
            keep_files,
            quiet,
            elide_empty_files,
            suppress_matched,
        }
    }
}

/// Splits a file into severals according to the command line patterns.
///
/// # Errors
///
/// - [`io::Error`] if there is some problem reading/writing from/to a file.
/// - [`CsplitError::LineOutOfRange`] if the line number pattern is larger than the number of input
///   lines.
/// - [`CsplitError::LineOutOfRangeOnRepetition`], like previous but after applying the pattern
///   more than once.
/// - [`CsplitError::MatchNotFound`] if no line matched a regular expression.
/// - [`CsplitError::MatchNotFoundOnRepetition`], like previous but after applying the pattern
///   more than once.
/// 根据指定的模式对输入进行拆分。
///
/// # 参数
/// - `options`: 拆分选项，控制拆分的行为。
/// - `patterns`: 一个字符串向量，包含了一个或多个拆分模式。
/// - `input`: 一个实现了 `BufRead` 的类型，代表输入数据。
///
/// # 返回值
/// 返回一个 `Result<(), CsplitError>`，如果成功，则结果为 `Ok(())`，如果过程中发生错误，则返回相应的错误信息。
pub fn csplit<T>(
    csplit_opts: &CsplitOptions,
    csplit_patterns: Vec<String>,
    input_info: T,
) -> Result<(), CsplitError>
where
    T: BufRead,
{
    // 初始化输入迭代器和拆分写入器
    let mut input_iter = InputSplitter::new(input_info.lines().enumerate());
    let mut split_writer = SplitWriter::new(csplit_opts);

    // 将字符串模式转换为内部使用的拆分模式
    let patterns: Vec<patterns::CsplitPattern> = patterns::get_patterns(&csplit_patterns[..])?;

    // 执行拆分操作
    let result = do_csplit(&mut split_writer, patterns, &mut input_iter);

    // 处理剩余的输入行
    input_iter.csplit_rewind_buffer();
    if let Some((_, line)) = input_iter.next() {
        split_writer.new_writer()?;
        split_writer.writeln(&line?)?;
        for (_, line) in input_iter {
            split_writer.writeln(&line?)?;
        }
        split_writer.finish_split();
    }

    // 如果拆分过程中发生错误，并且设置为不保留文件，则删除所有拆分结果
    if result.is_err() && !csplit_opts.keep_files {
        split_writer.delete_all_splits()?;
    }

    result
}

fn do_csplit<I>(
    split_writer: &mut SplitWriter,
    csplit_patterns: Vec<patterns::CsplitPattern>,
    input_iter: &mut InputSplitter<I>,
) -> Result<(), CsplitError>
where
    I: Iterator<Item = (usize, io::Result<String>)>,
{
    // 遍历拆分模式并对输入进行拆分
    for p in csplit_patterns {
        let pattern_as_str = p.to_string();
        let is_skip = matches!(p, patterns::CsplitPattern::SkipToMatch(_, _, _));
        match p {
            patterns::CsplitPattern::UpToLine(n, ex) => {
                // 根据行数拆分
                let mut up_to_line = n;
                for (_, ith) in ex.iter() {
                    split_writer.new_writer()?;
                    match split_writer.do_to_line(&pattern_as_str, up_to_line, input_iter) {
                        // 如果在重复应用模式时超出行范围，则返回错误
                        Err(CsplitError::LineOutOfRange(_)) if ith != 1 => {
                            return Err(CsplitError::LineOutOfRangeOnRepetition(
                                pattern_as_str.to_string(),
                                ith - 1,
                            ));
                        }
                        Err(err) => return Err(err),

                        Ok(()) => (),
                    }
                    up_to_line += n;
                }
            }
            patterns::CsplitPattern::UpToMatch(regex, offset, ex)
            | patterns::CsplitPattern::SkipToMatch(regex, offset, ex) => {
                // 根据匹配模式拆分
                for (max, ith) in ex.iter() {
                    if is_skip {
                        // 在跳过模式下，将写入器设置为/dev/null，不进行实际写入
                        split_writer.as_dev_null();
                    } else {
                        split_writer.new_writer()?;
                    }
                    match (
                        split_writer.csplit_do_to_match(
                            &pattern_as_str,
                            &regex,
                            offset,
                            input_iter,
                        ),
                        max,
                    ) {
                        // 如果指定总是执行但未找到匹配，则视为成功
                        (Err(CsplitError::MatchNotFound(_)), None) => {
                            return Ok(());
                        }
                        // 如果在重复应用模式时未找到匹配，则返回错误
                        (Err(CsplitError::MatchNotFound(_)), Some(m)) if m != 1 && ith != 1 => {
                            return Err(CsplitError::MatchNotFoundOnRepetition(
                                pattern_as_str.to_string(),
                                ith - 1,
                            ));
                        }
                        (Err(err), _) => return Err(err),
                        // 继续拆分处理
                        (Ok(()), _) => (),
                    };
                }
            }
        };
    }
    Ok(())
}

/// Write a portion of the input file into a split which filename is based on an incrementing
/// counter.
struct SplitWriter<'a> {
    /// the options set through the command line
    options: &'a CsplitOptions,
    /// a split counter
    counter: usize,
    /// the writer to the current split
    current_writer: Option<BufWriter<File>>,
    /// the size in bytes of the current split
    size: usize,
    /// flag to indicate that no content should be written to a split
    dev_null: bool,
}

impl<'a> Drop for SplitWriter<'a> {
    fn drop(&mut self) {
        if self.options.elide_empty_files && self.size == 0 {
            let file_name = self.options.split_name.get(self.counter);
            remove_file(file_name).expect("Failed to elide split");
        }
    }
}

impl<'a> SplitWriter<'a> {
    fn new(options: &CsplitOptions) -> SplitWriter {
        SplitWriter {
            options,
            counter: 0,
            current_writer: None,
            size: 0,
            dev_null: false,
        }
    }

    /// Creates a new split and returns its filename.
    ///
    /// # Errors
    ///
    /// The creation of the split file may fail with some [`io::Error`].
    fn new_writer(&mut self) -> io::Result<()> {
        let file_name = self.options.split_name.get(self.counter);
        let file = File::create(file_name)?;
        self.current_writer = Some(BufWriter::new(file));
        self.counter += 1;
        self.size = 0;
        self.dev_null = false;
        Ok(())
    }

    /// The current split will not keep any of the read input lines.
    fn as_dev_null(&mut self) {
        self.dev_null = true;
    }

    /// Writes the line to the current split, appending a newline character.
    /// If [`self.dev_null`] is true, then the line is discarded.
    ///
    /// # Errors
    ///
    /// Some [`io::Error`] may occur when attempting to write the line.
    fn writeln(&mut self, line: &str) -> io::Result<()> {
        if !self.dev_null {
            match self.current_writer {
                Some(ref mut current_writer) => {
                    let bytes = line.as_bytes();
                    current_writer.write_all(bytes)?;
                    current_writer.write_all(b"\n")?;
                    self.size += bytes.len() + 1;
                }
                None => panic!("trying to write to a split that was not created"),
            }
        }
        Ok(())
    }

    /// Perform some operations after completing a split, i.e., either remove it
    /// if the [`opt_flags::ELIDE_EMPTY_FILES`] option is enabled, or print how much bytes were written
    /// to it if [`opt_flags::QUIET`] is disabled.
    ///
    /// # Errors
    ///
    /// Some [`io::Error`] if the split could not be removed in case it should be elided.
    fn finish_split(&mut self) {
        if !self.dev_null {
            if self.options.elide_empty_files && self.size == 0 {
                self.counter -= 1;
            } else if !self.options.quiet {
                println!("{}", self.size);
            }
        }
    }

    /// Removes all the split files that were created.
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] if there was a problem removing a split.
    fn delete_all_splits(&self) -> io::Result<()> {
        let mut ret = Ok(());
        for ith in 0..self.counter {
            let file_name = self.options.split_name.get(ith);
            if let Err(err) = remove_file(file_name) {
                ret = Err(err);
            }
        }
        ret
    }

    /// Split the input stream up to the line number `n`.
    ///
    /// If the line number `n` is smaller than the current position in the input, then an empty
    /// split is created.
    ///
    /// # Errors
    ///
    /// In addition to errors reading/writing from/to a file, if the line number
    /// `n` is greater than the total available lines, then a
    /// [`CsplitError::LineOutOfRange`] error is returned.
    fn do_to_line<I>(
        &mut self,
        pattern_as_str: &str,
        n: usize,
        input_iter: &mut InputSplitter<I>,
    ) -> Result<(), CsplitError>
    where
        I: Iterator<Item = (usize, io::Result<String>)>,
    {
        input_iter.csplit_rewind_buffer();
        input_iter.csplit_set_size_of_buffer(1);

        let mut result = Err(CsplitError::LineOutOfRange(pattern_as_str.to_string()));
        while let Some((ln, line)) = input_iter.next() {
            let l = line?;
            match n.cmp(&(&ln + 1)) {
                Ordering::Less => {
                    assert!(
                        input_iter.csplit_add_line_to_buffer(ln, l).is_none(),
                        "the buffer is big enough to contain 1 line"
                    );
                    result = Ok(());
                    break;
                }
                Ordering::Equal => {
                    assert!(
                        self.options.suppress_matched
                            || input_iter.csplit_add_line_to_buffer(ln, l).is_none(),
                        "the buffer is big enough to contain 1 line"
                    );
                    result = Ok(());
                    break;
                }
                Ordering::Greater => (),
            }
            self.writeln(&l)?;
        }
        self.finish_split();
        result
    }

    /// Read lines up to the line matching a [`Regex`]. With a non-zero offset,
    /// the block of relevant lines can be extended (if positive), or reduced
    /// (if negative).
    ///
    /// # Errors
    ///
    /// In addition to errors reading/writing from/to a file, the following errors may be returned:
    /// - if no line matched, an [`CsplitError::MatchNotFound`].
    /// - if there are not enough lines to accommodate the offset, an
    /// [`CsplitError::LineOutOfRange`].

    /**
     * 根据给定的正则表达式和偏移量，在输入流中查找匹配，并据此进行分割。
     *
     * @param pattern_as_str 搜索模式的字符串表示。
     * @param regex 用于匹配的正则表达式对象。
     * @param offset 匹配成功后，额外添加到当前分割中的行数。正值表示在匹配行之后添加，负值表示在匹配行之前添加。
     * @param input_splitter 输入迭代器，提供按行分割的输入流。
     * @return Result<(), CsplitError>，成功时返回空的Result，失败时返回包含错误信息的Result。
     */
    #[allow(clippy::cognitive_complexity)]
    fn csplit_do_to_match<I>(
        &mut self,
        pattern_as_str: &str,
        regex: &Regex,
        mut offset: i32,
        input_splitter: &mut InputSplitter<I>,
    ) -> Result<(), CsplitError>
    where
        I: Iterator<Item = (usize, io::Result<String>)>,
    {
        if offset >= 0 {
            // 处理正偏移量的情况：不需要保留之前的行，直接从当前行开始匹配。
            for line_string in input_splitter.csplit_drain_buffer() {
                self.writeln(&line_string)?;
            }
            // 设置缓冲区大小以保留匹配的行。
            input_splitter.csplit_set_size_of_buffer(1);

            while let Some((ln, line)) = input_splitter.next() {
                let l = line?;
                if regex.is_match(&l) {
                    match (self.options.suppress_matched, offset) {
                        // 不抑制匹配的行且没有偏移量，直接添加到下一个分割。
                        (false, 0) => {
                            assert!(
                                input_splitter.csplit_add_line_to_buffer(ln, l).is_none(),
                                "the buffer is big enough to contain 1 line"
                            );
                        }
                        // 有正偏移量，需要在当前分割中添加更多行。
                        (false, _) => self.writeln(&l)?,
                        _ => (),
                    };
                    offset -= 1;

                    // 根据偏移量添加额外的行。
                    while offset > 0 {
                        match input_splitter.next() {
                            Some((_, line)) => {
                                self.writeln(&line?)?;
                            }
                            None => {
                                self.finish_split();
                                return Err(CsplitError::LineOutOfRange(
                                    pattern_as_str.to_string(),
                                ));
                            }
                        };
                        offset -= 1;
                    }
                    self.finish_split();
                    return Ok(());
                }
                self.writeln(&l)?;
            }
        } else {
            // 处理负偏移量的情况：需要保留之前的行以满足偏移量要求。
            let f_usize = -offset as usize;
            input_splitter.csplit_set_size_of_buffer(f_usize);
            while let Some((ln, line)) = input_splitter.next() {
                let l = line?;
                if regex.is_match(&l) {
                    // 从缓冲区中删除超出偏移量的行。
                    for line in input_splitter.csplit_shrink_buffer_to_size() {
                        self.writeln(&line)?;
                    }
                    if !self.options.suppress_matched {
                        // 为匹配的行留出空间。
                        input_splitter.csplit_set_size_of_buffer(f_usize + 1);
                        assert!(
                            input_splitter.csplit_add_line_to_buffer(ln, l).is_none(),
                            "should be big enough to hold every lines"
                        );
                    }
                    self.finish_split();
                    if input_splitter.csplit_buffer_len() < f_usize {
                        return Err(CsplitError::LineOutOfRange(pattern_as_str.to_string()));
                    }
                    return Ok(());
                }
                if let Some(line) = input_splitter.csplit_add_line_to_buffer(ln, l) {
                    self.writeln(&line)?;
                }
            }
            // 未找到匹配，将缓冲区中的剩余行添加到当前分割。
            for line in input_splitter.csplit_drain_buffer() {
                self.writeln(&line)?;
            }
        }

        self.finish_split();
        // 如果未找到匹配，返回错误。
        Err(CsplitError::MatchNotFound(pattern_as_str.to_string()))
    }
}

/// An iterator which can output items from a buffer filled externally.
/// This is used to pass matching lines to the next split and to support patterns with a negative offset.
struct InputSplitter<I>
where
    I: Iterator<Item = (usize, io::Result<String>)>,
{
    iter: I,
    buffer: Vec<<I as Iterator>::Item>,
    /// the number of elements the buffer may hold
    size: usize,
    /// flag to indicate content off the buffer should be returned instead of off the wrapped
    /// iterator
    rewind: bool,
}

impl<I> InputSplitter<I>
where
    I: Iterator<Item = (usize, io::Result<String>)>,
{
    fn new(iter: I) -> Self {
        Self {
            iter,
            buffer: Vec::new(),
            rewind: false,
            size: 1,
        }
    }

    /// Rewind the iteration by outputting the buffer's content.
    fn csplit_rewind_buffer(&mut self) {
        self.rewind = true;
    }

    /// Shrink the buffer so that its length is equal to the set size, returning an iterator for
    /// the elements that were too much.
    fn csplit_shrink_buffer_to_size(&mut self) -> impl Iterator<Item = String> + '_ {
        let shrink_offset = if self.buffer.len() > self.size {
            self.buffer.len() - self.size
        } else {
            0
        };
        self.buffer
            .drain(..shrink_offset)
            .map(|(_, line)| line.unwrap())
    }

    /// Drain the content of the buffer.
    fn csplit_drain_buffer(&mut self) -> impl Iterator<Item = String> + '_ {
        self.buffer.drain(..).map(|(_, line)| line.unwrap())
    }

    /// Set the maximum number of lines to keep.
    fn csplit_set_size_of_buffer(&mut self, size: usize) {
        self.size = size;
    }

    /// Add a line to the buffer. If the buffer has [`self.size`] elements, then its head is removed and
    /// the new line is pushed to the buffer. The removed head is then available in the returned
    /// option.
    fn csplit_add_line_to_buffer(&mut self, ln: usize, line: String) -> Option<String> {
        if self.rewind {
            self.buffer.insert(0, (ln, Ok(line)));
            None
        } else if self.buffer.len() >= self.size {
            let (_, head_line) = self.buffer.remove(0);
            self.buffer.push((ln, Ok(line)));
            Some(head_line.unwrap())
        } else {
            self.buffer.push((ln, Ok(line)));
            None
        }
    }

    /// Returns the number of lines stored in the buffer
    fn csplit_buffer_len(&self) -> usize {
        self.buffer.len()
    }
}

impl<I> Iterator for InputSplitter<I>
where
    I: Iterator<Item = (usize, io::Result<String>)>,
{
    type Item = <I as Iterator>::Item;

    fn next(&mut self) -> Option<Self::Item> {
        if self.rewind {
            if !self.buffer.is_empty() {
                return Some(self.buffer.remove(0));
            }
            self.rewind = false;
        }
        self.iter.next()
    }
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    csplit_main(args).map(|_| ())
}

pub fn csplit_main(args: impl ctcore::Args) -> CTResult<i32> {
    // 使用 clap 库解析命令行参数
    let args_match = ct_app().try_get_matches_from(args)?;

    // 获取待拆分的文件名
    let filename = args_match.get_one::<String>(opt_flags::FILE).unwrap();

    // 获取拆分所用的模式
    let patterns: Vec<String> = args_match
        .get_many::<String>(opt_flags::PATTERN)
        .unwrap()
        .map(|s| s.to_string())
        .collect();
    let csplit_opts = CsplitOptions::new(&args_match);

    // 根据文件名是标准输入还是文件路径，来决定如何处理文件
    if filename == "-" {
        // 处理标准输入
        let stdin = io::stdin();
        csplit(&csplit_opts, patterns, stdin.lock()).map_err(|err| {
            eprintln!("Error: {}", err);
            1 // 错误时返回状态码 1
        })?;
    } else {
        // 打开指定路径的文件
        let file_name = File::open(filename)
            .map_err_context(|| format!("cannot access {}", filename.quote()))?;
        // 获取文件元数据，检查是否为普通文件
        let file_metadata = file_name
            .metadata()
            .map_err_context(|| format!("cannot access {}", filename.quote()))?;
        if !file_metadata.is_file() {
            // 如果不是普通文件，则返回错误
            return Err(CsplitError::NotRegularFile(filename.to_string()).into());
        }
        // 使用缓冲读取器读取文件，并进行拆分
        csplit(&csplit_opts, patterns, BufReader::new(file_name)).map_err(|err| {
            eprintln!("Error: {}", err);
            1 // 错误时返回状态码 2
        })?;
    }

    Ok(0) // 成功时返回状态码 0
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = CSPLIT_ABOUT;
    let usage_description = ct_format_usage(CSPLIT_USAGE);

    let args = csplit_args_init();

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(&args)
        .after_help(AFTER_HELP)
}

fn csplit_args_init() -> Vec<Arg> {
    let args = vec![
        Arg::new(opt_flags::SUFFIX_FORMAT)
            .short('b')
            .long(opt_flags::SUFFIX_FORMAT)
            .value_name("FORMAT")
            .help("use sprintf FORMAT instead of %02d"),
        Arg::new(opt_flags::PREFIX)
            .short('f')
            .long(opt_flags::PREFIX)
            .value_name("PREFIX")
            .help("use PREFIX instead of 'xx'"),
        Arg::new(opt_flags::KEEP_FILES)
            .short('k')
            .long(opt_flags::KEEP_FILES)
            .help("do not remove output files on errors")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::SUPPRESS_MATCHED)
            .long(opt_flags::SUPPRESS_MATCHED)
            .help("suppress the lines matching PATTERN")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::DIGITS)
            .short('n')
            .long(opt_flags::DIGITS)
            .value_name("DIGITS")
            .help("use specified number of digits instead of 2"),
        Arg::new(opt_flags::QUIET)
            .short('s')
            .long(opt_flags::QUIET)
            .visible_alias("silent")
            .help("do not print counts of output file sizes")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::ELIDE_EMPTY_FILES)
            .short('z')
            .long(opt_flags::ELIDE_EMPTY_FILES)
            .help("remove empty output files")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::FILE)
            .hide(true)
            .required(true)
            .value_hint(clap::ValueHint::FilePath),
        Arg::new(opt_flags::PATTERN)
            .hide(true)
            .action(clap::ArgAction::Append)
            .required(true),
    ];
    args
}

