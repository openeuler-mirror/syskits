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

use clap::builder::ValueParser;
use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use std::borrow::{Borrow, Cow};
use std::cmp::max;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Write};
use std::iter;
use std::path::{Path, PathBuf};
use thiserror::Error;
use unicode_width::UnicodeWidthChar;

use ctcore::ct_error::{CTError, CTResult, FromIo};
use ctcore::ct_quoting_style::{CtQuotingStyle, escape_name};
use ctcore::ct_show;
use ctcore::{ct_format_usage, ct_help_about, ct_help_usage};

use crate::count_fast::{count_bytes_chars_lines_from_stream, count_bytes_handle};
use crate::countable::WcWordCountable;
use crate::read_utf8::{ReadBufDecoder, ReadBufDecoderError};
use crate::word_count::WcWordCount;
use ctcore::Tool;

mod count_fast;
mod countable;
mod read_utf8;
mod utf8;
mod word_count;

/// The minimum character width for formatting counts when reading from stdin.
const WC_MINIMUM_WIDTH: usize = 7;

#[derive(Debug, PartialEq)]
struct WcSettings<'a> {
    is_show_bytes: bool,
    is_show_chars: bool,
    is_show_lines: bool,
    is_show_words: bool,
    is_show_max_line_length: bool,
    files0_from: Option<WcInput<'a>>,
    total_when: WcTotalWhen,
}

impl Default for WcSettings<'_> {
    fn default() -> Self {
        // 如果未指定 -c、-m、-l、-w 或 -L 默认值。
        Self {
            is_show_bytes: true,
            is_show_chars: false,
            is_show_lines: true,
            is_show_words: true,
            is_show_max_line_length: false,
            files0_from: None,
            total_when: WcTotalWhen::default(),
        }
    }
}

impl<'a> WcSettings<'a> {
    fn new(matches: &'a ArgMatches) -> Self {
        let files0_from = matches
            .get_one::<OsString>(wc_flags::WC_FILES0_FROM)
            .map(Into::into);

        let total_when = matches
            .get_one::<String>(wc_flags::WC_TOTAL)
            .map(Into::into)
            .unwrap_or_default();

        let settings = Self {
            is_show_bytes: matches.get_flag(wc_flags::WC_BYTES),
            is_show_chars: matches.get_flag(wc_flags::WC_CHAR),
            is_show_lines: matches.get_flag(wc_flags::WC_LINES),
            is_show_words: matches.get_flag(wc_flags::WC_WORDS),
            is_show_max_line_length: matches.get_flag(wc_flags::WC_MAX_LINE_LENGTH),
            files0_from,
            total_when,
        };

        match settings.number_enabled() > 0 {
            true => settings,
            _ => Self {
                files0_from: settings.files0_from,
                total_when,
                ..Default::default()
            },
        }
    }

    fn number_enabled(&self) -> u32 {
        [
            self.is_show_bytes,
            self.is_show_chars,
            self.is_show_lines,
            self.is_show_max_line_length,
            self.is_show_words,
        ]
        .into_iter()
        .map(Into::<u32>::into)
        .sum()
    }
}

const WC_ABOUT: &str = ct_help_about!("wc.md");
const WC_USAGE: &str = ct_help_usage!("wc.md");

mod wc_flags {
    pub static WC_BYTES: &str = "bytes";
    pub static WC_CHAR: &str = "chars";
    pub static WC_FILES0_FROM: &str = "files0-from";
    pub static WC_LINES: &str = "lines";
    pub static WC_MAX_LINE_LENGTH: &str = "max-line-length";
    pub static WC_TOTAL: &str = "total";
    pub static WC_WORDS: &str = "words";
}

static WC_ARG_FILES: &str = "files";
static WC_STDIN_REPR: &str = "-";

static WC_QS_ESCAPE: &CtQuotingStyle = &CtQuotingStyle::Shell {
    escape: true,
    always_quote: false,
    show_control: false,
};
static WC_QS_QUOTE_ESCAPE: &CtQuotingStyle = &CtQuotingStyle::Shell {
    escape: true,
    always_quote: true,
    show_control: false,
};

/// Supported inputs.
#[derive(Debug)]
enum WcInputs<'a> {
    /// 默认为标准输入，即无参数。
    Stdin,
    /// 文件；"-"表示 stdin，可能是多次！
    Paths(Vec<WcInput<'a>>),
    /// --files0-from; "-" 是指 stdin.
    Files0From(WcInput<'a>),
}

impl<'a> WcInputs<'a> {
    fn new(matches: &'a ArgMatches) -> CTResult<Self> {
        let arg_files = matches.get_many::<OsString>(WC_ARG_FILES);
        let files0_from = matches.get_one::<OsString>(wc_flags::WC_FILES0_FROM);

        match (arg_files, files0_from) {
            (None, None) => Ok(Self::Stdin),
            (Some(files), None) => Ok(Self::Paths(files.map(Into::into).collect())),
            (None, Some(path)) => {
                // 如果路径是文件，且文件不太大，我们会提前加载它。
                // 文件中的每个路径都将检查其长度，以 希望能更好地对齐输出列。
                let input = WcInput::from(path);
                match input.try_as_files0()? {
                    Some(paths) => Ok(Self::Paths(paths)),
                    None => Ok(Self::Files0From(input)),
                }
            }
            (Some(mut files), Some(_)) => {
                Err(WcError::disabled_files(files.next().unwrap()).into())
            }
        }
    }

    // 创建一个迭代器，生成从命令行参数中提取的值。
    // 如果 --files0-from 中指定的文件无法打开，则返回错误信息。
    fn try_iter(
        &'a self,
        settings: &'a WcSettings<'a>,
    ) -> CTResult<impl Iterator<Item = InputIterItem<'a>>> {
        let base: Box<dyn Iterator<Item = _>> = match self {
            Self::Stdin => Box::new(iter::once(Ok(WcInput::Stdin(StdinKind::Implicit)))),
            Self::Paths(inputs) => Box::new(inputs.iter().map(|i| Ok(i.as_borrowed()))),
            Self::Files0From(input) => match input {
                WcInput::Path(path) => Box::new(files0_iter_file(path)?),
                WcInput::Stdin(_) => Box::new(files0_iter_stdin()),
            },
        };

        // 必须跟踪每个生成项目的基于 1 的指数，以便报告错误。
        let mut with_idx = base.enumerate().map(|(i, v)| (i + 1, v));
        let files0_from_path = settings.files0_from.as_ref().map(WcInput::as_borrowed);

        let iter = iter::from_fn(move || {
            let (idx, next) = with_idx.next()?;
            match next {
                // filter zero length file names...
                Ok(WcInput::Path(p)) if p.as_os_str().is_empty() => Some(Err({
                    let maybe_ctx = files0_from_path.as_ref().map(|p| (p, idx));
                    WcError::zero_length(maybe_ctx).into()
                })),
                _ => Some(next),
            }
        });
        Ok(iter)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum StdinKind {
    /// 在命令行中用"-"指定 (STDIN_REPR)
    Explicit,
    ///  没有任何论据
    Implicit,
}

/// --files0-from.代表单个输入，可通过以下方式计算或处理其他文件名
#[derive(Debug, PartialEq)]
enum WcInput<'a> {
    Path(Cow<'a, Path>),
    Stdin(StdinKind),
}

impl From<PathBuf> for WcInput<'_> {
    fn from(p: PathBuf) -> Self {
        match p.as_os_str() == WC_STDIN_REPR {
            true => Self::Stdin(StdinKind::Explicit),
            _ => Self::Path(Cow::Owned(p)),
        }
    }
}

impl<'a, T: AsRef<Path> + ?Sized> From<&'a T> for WcInput<'a> {
    fn from(p: &'a T) -> Self {
        let p = p.as_ref();

        match p.as_os_str() == WC_STDIN_REPR {
            true => Self::Stdin(StdinKind::Explicit),
            _ => Self::Path(Cow::Borrowed(p)),
        }
    }
}

impl<'a> WcInput<'a> {
    /// 转化 Path(Cow::Owned(_)) to Path(Cow::Borrowed(_)).
    fn as_borrowed(&'a self) -> Self {
        match self {
            Self::Path(p) => Self::Path(Cow::Borrowed(p.borrow())),
            Self::Stdin(k) => Self::Stdin(*k),
        }
    }

    /// 将输入内容转换为显示在统计信息中的标题。
    fn to_title(&self) -> Option<Cow<str>> {
        match self {
            Self::Path(path) => Some(match path.to_str() {
                Some(s) if !s.contains('\n') => Cow::Borrowed(s),
                _ => Cow::Owned(escape_name(path.as_os_str(), WC_QS_ESCAPE)),
            }),
            Self::Stdin(StdinKind::Explicit) => Some(Cow::Borrowed(WC_STDIN_REPR)),
            Self::Stdin(StdinKind::Implicit) => None,
        }
    }

    /// 将输入转换为错误显示的形式
    fn path_display(&self) -> String {
        match self {
            Self::Path(path) => escape_name(path.as_os_str(), WC_QS_ESCAPE),
            Self::Stdin(_) => String::from("standard input"),
        }
    }

    /// 当给定 --files0-from 时，我们可以给定一个路径或 stdin。二者都可以是流或普通文件。
    /// 如果给定的文件小于 10MB，它将被消耗并转化为一个 Input::Paths 的 Vec，
    /// 扫描该 Vec 可以确定最终打印的列的宽度。
    fn try_as_files0(&self) -> CTResult<Option<Vec<WcInput<'static>>>> {
        match self {
            Self::Path(path) => match fs::metadata(path) {
                Ok(meta) if meta.is_file() && meta.len() <= (10 << 20) => Ok(Some(
                    files0_iter_file(path)?.collect::<Result<Vec<_>, _>>()?,
                )),
                _ => Ok(None),
            },
            Self::Stdin(_) => {
                if is_stdin_small_file() {
                    Ok(Some(files0_iter_stdin().collect::<Result<Vec<_>, _>>()?))
                } else {
                    Ok(None)
                }
            }
        }
    }
}

#[cfg(unix)]
fn is_stdin_small_file() -> bool {
    use std::os::unix::io::{AsRawFd, FromRawFd};
    // 安全性：我们将依靠 Rust 为 stdin 提供一个有效的 RawFd，我们可以尝试用它打开文件，但只是为了获取 .metadata()。
    // 如果出现意外情况，ManuallyDrop 将确保我们不会对 FD 做任何其他操作。
    let f = std::mem::ManuallyDrop::new(unsafe { File::from_raw_fd(io::stdin().as_raw_fd()) });
    matches!(f.metadata(),
     Ok(meta) if meta.is_file() && meta.len() <= (10 << 20))
}

#[cfg(not(unix))]
// windows 会将管道传输的 stdin 显示为 "普通文件"，其长度等于检查时缓冲的字节数。
// 为了安全起见，我们绝不能假定它是一个文件。
fn is_stdin_small_file() -> bool {
    false
}

/// 何时显示 "total" 行
#[derive(Clone, Copy, Default, PartialEq, Debug)]
enum WcTotalWhen {
    #[default]
    Auto,
    Always,
    Only,
    Never,
}

impl<T: AsRef<str>> From<T> for WcTotalWhen {
    fn from(s: T) -> Self {
        match s.as_ref() {
            "auto" => WcTotalWhen::Auto,
            "always" => WcTotalWhen::Always,
            "only" => WcTotalWhen::Only,
            "never" => WcTotalWhen::Never,
            _ => unreachable!("Should have been caught by clap"),
        }
    }
}

impl WcTotalWhen {
    fn is_total_row_visible(&self, num_inputs: usize) -> bool {
        match self {
            WcTotalWhen::Auto => num_inputs > 1,
            WcTotalWhen::Always | WcTotalWhen::Only => true,
            WcTotalWhen::Never => false,
        }
    }
}

#[allow(clippy::enum_variant_names)]
#[derive(Debug, Error)]
enum WcError {
    #[error("extra operand '{extra}'\nfile operands cannot be combined with --files0-from")]
    CtFilesDisabled { extra: Cow<'static, str> },
    #[error("when reading file names from stdin, no file name of '-' allowed")]
    CtStdinReprNotAllowed,
    #[error("invalid zero-length file name")]
    CtZeroLengthFileName,
    #[error("{path}:{idx}: invalid zero-length file name")]
    CtZeroLengthFileNameCtx { path: Cow<'static, str>, idx: usize },
}

impl WcError {
    fn zero_length(ctx: Option<(&WcInput, usize)>) -> Self {
        if let Some((input, idx)) = ctx {
            let path = match input {
                WcInput::Stdin(_) => WC_STDIN_REPR.into(),
                WcInput::Path(path) => escape_name(path.as_os_str(), WC_QS_ESCAPE).into(),
            };
            Self::CtZeroLengthFileNameCtx { path, idx }
        } else {
            Self::CtZeroLengthFileName
        }
    }
    fn disabled_files(first_extra: &OsString) -> Self {
        let extra = first_extra.to_string_lossy().into_owned().into();
        Self::CtFilesDisabled { extra }
    }
}

impl CTError for WcError {
    fn usage(&self) -> bool {
        matches!(self, Self::CtFilesDisabled { .. })
    }
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    wc_main(args)
}

pub fn wc_main(args: impl ctcore::Args) -> CTResult<()> {
    let matches = ct_app().try_get_matches_from(args)?;

    let settings = WcSettings::new(&matches);
    let inputs = WcInputs::new(&matches)?;

    wc(&inputs, &settings)
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = WC_ABOUT;
    let usage_description = ct_format_usage(WC_USAGE);
    let args = vec![
        Arg::new(wc_flags::WC_BYTES)
            .short('c')
            .long(wc_flags::WC_BYTES)
            .help("print the byte counts")
            .action(ArgAction::SetTrue),
        Arg::new(wc_flags::WC_CHAR)
            .short('m')
            .long(wc_flags::WC_CHAR)
            .help("print the character counts")
            .action(ArgAction::SetTrue),
        Arg::new(wc_flags::WC_FILES0_FROM)
            .long(wc_flags::WC_FILES0_FROM)
            .value_name("F")
            .help(concat!(
                "read input from the files specified by\n",
                "  NUL-terminated names in file F;\n",
                "  If F is - then read names from standard input"
            ))
            .value_parser(ValueParser::os_string())
            .value_hint(clap::ValueHint::FilePath),
        Arg::new(wc_flags::WC_LINES)
            .short('l')
            .long(wc_flags::WC_LINES)
            .help("print the newline counts")
            .action(ArgAction::SetTrue),
        Arg::new(wc_flags::WC_MAX_LINE_LENGTH)
            .short('L')
            .long(wc_flags::WC_MAX_LINE_LENGTH)
            .help("print the length of the longest line")
            .action(ArgAction::SetTrue),
        Arg::new(wc_flags::WC_TOTAL)
            .long(wc_flags::WC_TOTAL)
            .value_parser(["auto", "always", "only", "never"])
            .value_name("WHEN")
            .hide_possible_values(true)
            .help(concat!(
                "when to print a line with total counts;\n",
                "  WHEN can be: auto, always, only, never"
            )),
        Arg::new(wc_flags::WC_WORDS)
            .short('w')
            .long(wc_flags::WC_WORDS)
            .help("print the word counts")
            .action(ArgAction::SetTrue),
        Arg::new(WC_ARG_FILES)
            .action(ArgAction::Append)
            .value_parser(ValueParser::os_string())
            .value_hint(clap::ValueHint::FilePath),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args_override_self(true)
        .args(&args)
}

fn word_count_from_reader<T: WcWordCountable>(
    mut reader: T,
    settings: &WcSettings,
) -> (WcWordCount, Option<io::Error>) {
    match (
        settings.is_show_bytes,
        settings.is_show_chars,
        settings.is_show_lines,
        settings.is_show_max_line_length,
        settings.is_show_words,
    ) {
        // 专业化扫描循环，提高性能。
        (false, false, false, false, false) => unreachable!(),

        // 显示bytes
        (true, false, false, false, false) => {
            // 仅 显示bytes 为真时的快速路径。
            let (bytes, error) = count_bytes_handle(&mut reader);
            (
                WcWordCount {
                    bytes,
                    ..WcWordCount::default()
                },
                error,
            )
        }

        // 无需进行 Unicode 解码即可计算的快速路径。
        // 显示lines
        (false, false, true, false, false) => {
            count_bytes_chars_lines_from_stream::<_, false, false, true>(&mut reader)
        }
        // 显示chars
        (false, true, false, false, false) => {
            count_bytes_chars_lines_from_stream::<_, false, true, false>(&mut reader)
        }
        // 显示lines, 显示chars
        (false, true, true, false, false) => {
            count_bytes_chars_lines_from_stream::<_, false, true, true>(&mut reader)
        }
        // 显示bytes, 显示lines
        (true, false, true, false, false) => {
            count_bytes_chars_lines_from_stream::<_, true, false, true>(&mut reader)
        }
        // 显示bytes, 显示chars
        (true, true, false, false, false) => {
            count_bytes_chars_lines_from_stream::<_, true, true, false>(&mut reader)
        }
        // 显示bytes, 显示chars, 显示lines
        (true, true, true, false, false) => {
            count_bytes_chars_lines_from_stream::<_, true, true, true>(&mut reader)
        }
        // 显示words
        (_, false, false, false, true) => {
            word_count_from_specialized_reader::<_, false, false, false, true>(reader)
        }
        // 显示max_line_length
        (_, false, false, true, false) => {
            word_count_from_specialized_reader::<_, false, false, true, false>(reader)
        }
        // 显示max_line_length, 显示words
        (_, false, false, true, true) => {
            word_count_from_specialized_reader::<_, false, false, true, true>(reader)
        }
        // 显示lines, 显示words
        (_, false, true, false, true) => {
            word_count_from_specialized_reader::<_, false, true, false, true>(reader)
        }
        // 显示lines, 显示max_line_length
        (_, false, true, true, false) => {
            word_count_from_specialized_reader::<_, false, true, true, false>(reader)
        }
        // 显示lines, 显示max_line_length, 显示words
        (_, false, true, true, true) => {
            word_count_from_specialized_reader::<_, false, true, true, true>(reader)
        }
        // 显示chars, 显示words
        (_, true, false, false, true) => {
            word_count_from_specialized_reader::<_, true, false, false, true>(reader)
        }
        // 显示chars, 显示max_line_length
        (_, true, false, true, false) => {
            word_count_from_specialized_reader::<_, true, false, true, false>(reader)
        }
        // 显示chars, 显示max_line_length, 显示words
        (_, true, false, true, true) => {
            word_count_from_specialized_reader::<_, true, false, true, true>(reader)
        }
        // 显示chars, 显示lines, 显示words
        (_, true, true, false, true) => {
            word_count_from_specialized_reader::<_, true, true, false, true>(reader)
        }
        // 显示chars, 显示lines, 显示max_line_length
        (_, true, true, true, false) => {
            word_count_from_specialized_reader::<_, true, true, true, false>(reader)
        }
        // 显示chars, 显示lines, 显示max_line_length, 显示words
        (_, true, true, true, true) => {
            word_count_from_specialized_reader::<_, true, true, true, true>(reader)
        }
    }
}

fn process_chunk<
    const SHOW_CHARS: bool,
    const SHOW_LINES: bool,
    const SHOW_MAX_LINE_LENGTH: bool,
    const SHOW_WORDS: bool,
>(
    total: &mut WcWordCount,
    text: &str,
    current_len: &mut usize,
    in_word: &mut bool,
) {
    for ch in text.chars() {
        if SHOW_WORDS {
            if ch.is_whitespace() {
                *in_word = false;
            } else if ch.is_ascii_control() {
                // 这些字符算作字符，但不影响单词状态
            } else if !(*in_word) {
                *in_word = true;
                total.words += 1;
            }
        }
        if SHOW_MAX_LINE_LENGTH {
            match ch {
                '\n' | '\r' | '\x0c' => {
                    total.max_line_length = max(*current_len, total.max_line_length);
                    *current_len = 0;
                }
                '\t' => {
                    *current_len -= *current_len % 8;
                    *current_len += 8;
                }
                _ => {
                    *current_len += ch.width().unwrap_or(0);
                }
            }
        }
        if SHOW_LINES && ch == '\n' {
            total.lines += 1;
        }
        if SHOW_CHARS {
            total.chars += 1;
        }
    }
    total.bytes += text.len();

    total.max_line_length = max(*current_len, total.max_line_length);
}

fn handle_error(error: ReadBufDecoderError<'_>, total: &mut WcWordCount) -> Option<io::Error> {
    if let ReadBufDecoderError::InvalidByteSequence(bytes) = error {
        total.bytes += bytes.len();
    } else if let ReadBufDecoderError::Io(e) = error {
        return Some(e);
    }
    None
}

fn word_count_from_specialized_reader<
    T: WcWordCountable,
    const SHOW_CHARS: bool,
    const SHOW_LINES: bool,
    const SHOW_MAX_LINE_LENGTH: bool,
    const SHOW_WORDS: bool,
>(
    reader: T,
) -> (WcWordCount, Option<io::Error>) {
    let mut total = WcWordCount::default();
    let mut reader = ReadBufDecoder::new(reader.buffered());
    let mut in_word = false;
    let mut current_len = 0;
    while let Some(chunk) = reader.next_strict() {
        if let Ok(text) = chunk {
            process_chunk::<SHOW_CHARS, SHOW_LINES, SHOW_MAX_LINE_LENGTH, SHOW_WORDS>(
                &mut total,
                text,
                &mut current_len,
                &mut in_word,
            );
        } else if let Some(e) = handle_error(chunk.unwrap_err(), &mut total) {
            return (total, Some(e));
        }
    }

    (total, None)
}

enum CountResult {
    /// 没有出错。
    Success(WcWordCount),
    /// 成功打开，但无法阅读。
    Interrupted(WcWordCount, io::Error),
    /// 甚至都没来得及打开。
    Failure(io::Error),
}

/// 如果打开文件失败，我们只会显示错误。如果读取文件失败，我们会显示成功读取的文件数量。
/// 因此，读取实现总是返回总数，有时也会返回(WordCount, Option<io::Error>).
fn word_count_from_input(input: &WcInput<'_>, settings: &WcSettings) -> CountResult {
    let (total, maybe_err) = match input {
        WcInput::Stdin(_) => word_count_from_reader(io::stdin().lock(), settings),
        WcInput::Path(path) => match File::open(path) {
            Ok(f) => word_count_from_reader(f, settings),
            Err(err) => return CountResult::Failure(err),
        },
    };

    if let Some(err) = maybe_err {
        CountResult::Interrupted(total, err)
    } else {
        CountResult::Success(total)
    }
}

/// 计算在所有输入中表示所有计数所需的位数。
/// 对于 [`WcInputs::Stdin`]，将返回 [`WC_MINIMUM_WIDTH`]，除非只有一个计数器数字需要打印，否则将返回 1。
/// 对于 [`WcInputs::Files0From`]，将返回 [`WC_MINIMUM_WIDTH`]。
/// 一个[`WcInputs::Paths`]可能包含零个或多个"-"条目，每个"-"条目代表从 "stdin`"读取数据。
/// 任何此类条目的存在都会导致此函数返回至少为 [`WC_MINIMUM_WIDTH`] 的宽度。
/// 如果[`WcInputs::Paths`]只包含一个路径，并且只需要打印一个数字，那么此函数将被优化为返回 1，而无需调用任何函数来获取文件元数据。
/// 如果无法从任何 [`WcInput::Path`] 输入中读取文件元数据，则该输入不会影响数字宽度的计算。
/// 否则，将对文件元数据中的文件大小进行求和，并返回总大小的位数。
fn compute_number_width(inputs: &WcInputs, settings: &WcSettings) -> usize {
    match inputs {
        WcInputs::Stdin if settings.number_enabled() == 1 => 1,
        WcInputs::Stdin => WC_MINIMUM_WIDTH,
        WcInputs::Files0From(_) => 1,
        WcInputs::Paths(inputs) => {
            if settings.number_enabled() == 1 && inputs.len() == 1 {
                return 1;
            }

            let mut minimum_width = 1;
            let mut total: u64 = 0;
            for input in inputs {
                if let WcInput::Stdin(_) = input {
                    minimum_width = WC_MINIMUM_WIDTH;
                } else if let WcInput::Path(path) = input {
                    if let Ok(meta) = fs::metadata(path) {
                        if meta.is_file() {
                            total += meta.len();
                        } else {
                            minimum_width = WC_MINIMUM_WIDTH;
                        }
                    }
                }
            }

            if total == 0 {
                minimum_width
            } else {
                let ilog = 1 + total.ilog10();
                let total_width = match ilog.try_into() {
                    Ok(width) => width,
                    Err(_) => panic!("ilog of a u64 should fit into a usize"),
                };
                max(total_width, minimum_width)
            }
        }
    }
}

type InputIterItem<'a> = Result<WcInput<'a>, Box<dyn CTError>>;

/// 与 `--files0-from=-` 一起使用时，会对 files0_iter 的结果进行过滤，将"-"转换为相应的错误。
fn files0_iter_stdin<'a>() -> impl Iterator<Item = InputIterItem<'a>> {
    let files_iter = files0_iter(io::stdin().lock(), WC_STDIN_REPR.into());
    let mut result: Vec<Result<WcInput<'a>, Box<dyn CTError>>> = vec![];

    for i in files_iter {
        let mapped = match i {
            Ok(WcInput::Stdin(_)) => Err(WcError::CtStdinReprNotAllowed.into()),
            _ => i,
        };
        result.push(mapped);
    }

    result.into_iter()
}

fn files0_iter_file<'a>(path: &Path) -> CTResult<impl Iterator<Item = InputIterItem<'a>>> {
    let f = File::open(path);
    if let Ok(f) = f {
        Ok(files0_iter(f, path.into()))
    } else {
        let e = f.unwrap_err();
        Err(e.map_err_context(|| {
            format!(
                "cannot open {} for reading",
                escape_name(path.as_os_str(), WC_QS_QUOTE_ESCAPE)
            )
        }))
    }
}

fn files0_iter<'a>(
    r: impl io::Read + 'static,
    err_path: OsString,
) -> impl Iterator<Item = InputIterItem<'a>> {
    use std::io::BufRead;
    let mut i = Some(io::BufReader::new(r).split(b'\0').map(move |res| {
        if let Ok(p) = res {
            if p == WC_STDIN_REPR.as_bytes() {
                Ok(WcInput::Stdin(StdinKind::Explicit))
            } else {
                #[cfg(unix)]
                {
                    use std::os::unix::ffi::OsStringExt;
                    Ok(WcInput::Path(PathBuf::from(OsString::from_vec(p)).into()))
                }

                #[cfg(not(unix))]
                {
                    let s = String::from_utf8(p)
                        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
                    Ok(WcInput::Path(PathBuf::from(s).into()))
                }
            }
        } else {
            let e = res.unwrap_err();
            Err(e
                .map_err_context(|| format!("{}: read error", escape_name(&err_path, WC_QS_ESCAPE)))
                as Box<dyn CTError>)
        }
    }));

    // Loop until there is an error; yield that error and then nothing else.
    std::iter::from_fn(move || {
        let next = i.as_mut().and_then(Iterator::next);
        if matches!(next, Some(Err(_)) | None) {
            i = None;
        }
        next
    })
}

fn wc(inputs: &WcInputs, settings: &WcSettings) -> CTResult<()> {
    let mut total_word_count = WcWordCount::default();
    let mut num_inputs: usize = 0;

    let (number_width, are_stats_visible) = if settings.total_when == WcTotalWhen::Only {
        (1, false)
    } else {
        (compute_number_width(inputs, settings), true)
    };

    for maybe_input in inputs.try_iter(settings)? {
        num_inputs += 1;

        let input = if let Ok(val) = maybe_input {
            val
        } else {
            if let Err(err) = maybe_input {
                ct_show!(err);
            }
            continue;
        };

        let mut word_count = WcWordCount::default();
        let word_cnt = word_count_from_input(&input, settings);
        if let CountResult::Success(word_count_tmp) = word_cnt {
            word_count = word_count_tmp;
        } else if let CountResult::Interrupted(word_count_tmp, err) = word_cnt {
            ct_show!(err.map_err_context(|| input.path_display()));
            word_count = word_count_tmp;
        } else if let CountResult::Failure(err) = word_cnt {
            ct_show!(err.map_err_context(|| input.path_display()));
            continue;
        }

        total_word_count += word_count;

        if are_stats_visible {
            let maybe_title = input.to_title();
            let maybe_title_str = maybe_title.as_deref();
            let _ =
                print_stats(settings, &word_count, maybe_title_str, number_width).map_err(|err| {
                    let title = maybe_title_str.unwrap_or("<stdin>");
                    ct_show!(
                        err.map_err_context(|| format!("failed to print result for {}", title))
                    )
                });
        }
    }

    if settings.total_when.is_total_row_visible(num_inputs) {
        let title = are_stats_visible.then_some("total");
        print_stats(settings, &total_word_count, title, number_width).unwrap_or_else(|err| {
            ct_show!(err.map_err_context(|| "failed to print total".into()));
        });
    }

    // 虽然这似乎是返回 `Ok` ，但退出代码可能已被设置为一个非零值(调用`record_error!()`)。
    Ok(())
}

fn print_stats(
    settings: &WcSettings,
    result: &WcWordCount,
    title: Option<&str>,
    number_width: usize,
) -> io::Result<()> {
    let mut stdout = io::stdout().lock();

    let maybe_cols = &[
        (settings.is_show_lines, result.lines),
        (settings.is_show_words, result.words),
        (settings.is_show_chars, result.chars),
        (settings.is_show_bytes, result.bytes),
        (settings.is_show_max_line_length, result.max_line_length),
    ];

    let mut space = "";
    for (_, num) in maybe_cols.iter().filter(|(show, _)| *show) {
        write!(stdout, "{space}{num:number_width$}")?;
        space = " ";
    }

    if let Some(title) = title {
        writeln!(stdout, "{space}{title}")
    } else {
        writeln!(stdout)
    }
}

#[derive(Default)]
pub struct Wc;
impl Tool for Wc {
    fn name(&self) -> &'static str {
        "wc"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        wc_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::ffi::OsString;
    use std::io::Write;
    use std::path::PathBuf;

    use clap::ArgMatches;
    use clap::error::ErrorKind;
    use tempfile::NamedTempFile;
    use tempfile::tempfile;

    use super::*;

    #[test]
    fn test_tool_implementation() {
        let tool = Wc;

        // Test name method
        assert_eq!(tool.name(), "wc");

        // Test command method
        let command = tool.command();
        assert!(command.get_name().contains("wc"));

        // Test execute method with help flag (should work)
        let args = vec![OsString::from("wc"), OsString::from("--help")];
        let result = tool.execute(&args);
        assert!(result.is_err());
    }

    // ----------------- 测试 wc 函数 -----------------
    #[cfg(test)]
    mod tests {
        use std::io::Write;

        use super::*;

        fn setup_settings(bytes: bool, words: bool, lines: bool) -> WcSettings<'static> {
            WcSettings {
                is_show_bytes: bytes,
                is_show_chars: false,
                is_show_lines: lines,
                is_show_words: words,
                is_show_max_line_length: false,
                files0_from: None,
                total_when: WcTotalWhen::Auto,
            }
        }

        // 创建文件并写入内容
        fn base_create_file_with_content(filename: &str, content: &str) -> std::io::Result<()> {
            let mut file = File::create(filename)?;
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
            Ok(())
        }

        // 删除指定文件
        fn base_delete_file(filename: &str) -> std::io::Result<()> {
            fs::remove_file(filename)?;
            Ok(())
        }

        #[test]
        fn test_wc_with_file_input() {
            let file_name = "test_file";
            let content = "hello world\nhello rust\n";
            let mut path_buf = PathBuf::new();

            path_buf.push(file_name);
            // 创建文件并写入内容
            match base_create_file_with_content(file_name, content) {
                Ok(_) => println!("File '{}' created successfully.", file_name),
                Err(e) => eprintln!("Error creating file: {}", e),
            }

            let inputs = WcInputs::Paths(vec![WcInput::Path(Cow::Owned(path_buf))]);
            let settings = setup_settings(true, true, true);

            // 删除文件
            match base_delete_file(file_name) {
                Ok(_) => println!("File '{}' deleted successfully.", file_name),
                Err(e) => eprintln!("Error deleting file: {}", e),
            }

            assert!(
                wc(&inputs, &settings).is_ok(),
                "wc should handle file input correctly"
            );
        }

        #[test]
        fn test_wc_with_files0_from_exsit_file() {
            let file_name = "test_file0";
            let content = "file1.txt\0file2.txt\0";
            let mut path_buf = PathBuf::new();
            let file_name1 = "file1.txt";
            let content1 = "hello file1\n";
            let file_name2 = "file2.txt";
            let content2 = "hello file2\n";
            path_buf.push(file_name);
            // 创建文件并写入内容
            match base_create_file_with_content(file_name, content) {
                Ok(_) => println!("File '{}' created successfully.", file_name),
                Err(e) => eprintln!("Error creating file: {}", e),
            }

            match base_create_file_with_content(file_name1, content1) {
                Ok(_) => println!("File '{}' created successfully.", file_name1),
                Err(e) => eprintln!("Error creating file: {}", e),
            }

            match base_create_file_with_content(file_name2, content2) {
                Ok(_) => println!("File '{}' created successfully.", file_name2),
                Err(e) => eprintln!("Error creating file: {}", e),
            }

            let inputs = WcInputs::Files0From(WcInput::Path(Cow::Owned(path_buf)));
            let settings = setup_settings(false, false, true);

            assert!(
                wc(&inputs, &settings).is_ok(),
                "wc should handle --files0-from input correctly"
            );

            // 删除文件
            match base_delete_file(file_name1) {
                Ok(_) => println!("File '{}' deleted successfully.", file_name1),
                Err(e) => eprintln!("Error deleting file: {}", e),
            }

            match base_delete_file(file_name2) {
                Ok(_) => println!("File '{}' deleted successfully.", file_name2),
                Err(e) => eprintln!("Error deleting file: {}", e),
            }

            match base_delete_file(file_name) {
                Ok(_) => println!("File '{}' deleted successfully.", file_name),
                Err(e) => eprintln!("Error deleting file: {}", e),
            }
        }

        #[test]
        fn test_wc_with_files0_from() {
            let file_name = "test_file1";
            let content = "file3.txt\0file4.txt\0";
            let mut path_buf = PathBuf::new();

            path_buf.push(file_name);
            // 创建文件并写入内容
            match base_create_file_with_content(file_name, content) {
                Ok(_) => println!("File '{}' created successfully.", file_name),
                Err(e) => eprintln!("Error creating file: {}", e),
            }

            let inputs = WcInputs::Files0From(WcInput::Path(Cow::Owned(path_buf)));
            let settings = setup_settings(false, false, true);

            assert!(
                wc(&inputs, &settings).is_ok(),
                "wc should handle --files0-from input correctly"
            );

            // 删除文件
            match base_delete_file(file_name) {
                Ok(_) => println!("File '{}' deleted successfully.", file_name),
                Err(e) => eprintln!("Error deleting file: {}", e),
            }
        }
    }

    // ----------------- 测试setting 函数 -----------------
    /// Helper function to get ArgMatches from command line arguments
    /// 测试 setting 和  input 函数使用
    fn get_matches_from_args(args: &[&str]) -> ArgMatches {
        ct_app().try_get_matches_from(args).unwrap()
    }

    #[test]
    fn test_number_enabled_none() {
        let matches = get_matches_from_args(&["test"]);
        let settings = WcSettings::new(&matches);
        assert_eq!(settings.number_enabled(), 3);
    }

    #[test]
    fn test_number_enabled_all() {
        let matches = get_matches_from_args(&[
            "test",
            "--bytes",
            "--chars",
            "--lines",
            "--words",
            "--max-line-length",
        ]);
        let settings = WcSettings::new(&matches);
        assert_eq!(settings.number_enabled(), 5);
    }

    #[test]
    fn test_number_enabled_some() {
        let matches = get_matches_from_args(&["test", "--bytes", "--words"]);
        let settings = WcSettings::new(&matches);
        assert_eq!(settings.number_enabled(), 2);
    }

    #[test]
    fn test_settings_new_defaults() {
        let matches = get_matches_from_args(&["test"]);
        let settings = WcSettings::new(&matches);
        // Since no flags are set, check default values are as expected
        assert!(settings.is_show_bytes);
        assert!(!settings.is_show_chars);
        assert!(settings.is_show_lines);
        assert!(settings.is_show_words);
        assert!(!settings.is_show_max_line_length);
        assert!(settings.files0_from.is_none());
        assert_eq!(settings.total_when, WcTotalWhen::Auto);
    }

    #[test]
    fn test_settings_new_specific() {
        let matches = get_matches_from_args(&["test", "--bytes", "--lines", "--total=always"]);
        let settings = WcSettings::new(&matches);
        assert!(settings.is_show_bytes);
        assert!(!settings.is_show_chars);
        assert!(settings.is_show_lines);
        assert!(!settings.is_show_words);
        assert!(!settings.is_show_max_line_length);
        assert_eq!(settings.total_when, WcTotalWhen::Always);
    }

    #[test]
    fn test_settings_with_files0_and_files_argument() {
        let matches = get_matches_from_args(&["test", "--files0-from=file0.txt", "file1.txt"]);
        let settings = WcSettings::new(&matches);
        // Assume settings should not have initialized properly or should reflect an error state
        assert_eq!(
            settings.files0_from.is_some(),
            true,
            "files0-from should not accept other file arguments"
        );
    }

    #[test]
    fn test_all_flags_enabled() {
        let matches = get_matches_from_args(&[
            "test",
            "--bytes",
            "--chars",
            "--lines",
            "--words",
            "--max-line-length",
        ]);
        let settings = WcSettings::new(&matches);
        let expected_settings = WcSettings {
            is_show_bytes: true,
            is_show_chars: true,
            is_show_lines: true,
            is_show_words: true,
            is_show_max_line_length: true,
            files0_from: None,
            total_when: WcTotalWhen::Auto,
        };
        assert_eq!(
            settings, expected_settings,
            "All flags enabled settings do not match expected"
        );
    }

    #[test]
    fn test_files0_from_with_path() {
        let matches = get_matches_from_args(&["test", "--files0-from=path/to/file"]);
        let settings = WcSettings::new(&matches);
        assert!(
            settings.files0_from.is_some(),
            "files0-from setting should be set with a path"
        );
    }

    #[test]
    fn test_auto_total_with_multiple_files() {
        let matches = get_matches_from_args(&["test", "--total=auto", "file1.txt", "file2.txt"]);
        let settings = WcSettings::new(&matches);
        assert!(
            settings.total_when.is_total_row_visible(2),
            "Auto total should be visible with multiple files"
        );
    }

    #[test]
    fn test_no_flags_enabled() {
        let matches = get_matches_from_args(&["test"]);
        let settings = WcSettings::new(&matches);
        let expected_settings = WcSettings::default();
        assert_eq!(
            settings, expected_settings,
            "No flags enabled settings do not match expected default settings"
        );
    }

    #[test]
    fn test_invalid_file_argument() {
        let result =
            ct_app().try_get_matches_from(&["test", "--files0-from", "--some-invalid-file"]);
        assert!(result.is_err(), "Should error with invalid file argument");
    }

    #[test]
    fn test_char_flag_alone() {
        let matches = get_matches_from_args(&["test", "--chars"]);
        let settings = WcSettings::new(&matches);
        assert!(settings.is_show_chars, "Char flag should be enabled");
        assert!(
            !settings.is_show_bytes,
            "Bytes should not be enabled when only char flag is set"
        );
    }

    #[test]
    fn test_combining_files0_from_with_stdin() {
        let matches = get_matches_from_args(&["test", "--files0-from=-"]);
        let settings = WcSettings::new(&matches);
        assert_eq!(
            settings.files0_from,
            Some(WcInput::Stdin(StdinKind::Explicit)),
            "files0-from should accept stdin as '-'"
        );
    }

    #[test]
    fn test_total_never_with_no_files() {
        let matches = get_matches_from_args(&["test", "--total=never"]);
        let settings = WcSettings::new(&matches);
        assert!(
            !settings.total_when.is_total_row_visible(0),
            "Total should never be visible when total is set to never"
        );
    }

    #[test]
    fn test_mixed_flags_with_files() {
        let matches =
            get_matches_from_args(&["test", "--bytes", "--words", "file1.txt", "file2.txt"]);
        let settings = WcSettings::new(&matches);
        assert!(
            settings.is_show_bytes && settings.is_show_words,
            "Bytes and Words flags should be enabled"
        );
        assert!(!settings.is_show_chars, "Chars should not be enabled");
        assert!(!settings.is_show_lines, "Lines should not be enabled");
    }

    #[test]
    fn test_default_settings() {
        let matches = get_matches_from_args(&["test"]);
        let settings = WcSettings::new(&matches);
        let expected_settings = WcSettings {
            is_show_bytes: true,
            is_show_chars: false,
            is_show_lines: true,
            is_show_words: true,
            is_show_max_line_length: false,
            files0_from: None,
            total_when: WcTotalWhen::Auto,
        };
        assert_eq!(
            settings, expected_settings,
            "Default settings do not match expected"
        );
    }

    #[test]
    fn test_specific_settings() {
        let matches = get_matches_from_args(&["test", "--bytes", "--total=always"]);
        let settings = WcSettings::new(&matches);
        let expected_settings = WcSettings {
            is_show_bytes: true,
            is_show_chars: false,
            is_show_lines: false,
            is_show_words: false,
            is_show_max_line_length: false,
            files0_from: None,
            total_when: WcTotalWhen::Always,
        };
        assert_eq!(
            settings, expected_settings,
            "Settings with specific flags do not match expected"
        );
    }

    #[test]
    fn test_settings_never_show_total() {
        let matches = get_matches_from_args(&["test", "--total=never", "file1.txt", "file2.txt"]);
        let settings = WcSettings::new(&matches);
        // Total should never be visible
        assert!(
            !settings.total_when.is_total_row_visible(1),
            "Never should never display total"
        );
        assert!(
            !settings.total_when.is_total_row_visible(2),
            "Never should never display total even with multiple files"
        );
    }

    #[test]
    fn test_invalid_command_argument() {
        let result = ct_app().try_get_matches_from(&["test", "--nonsensical"]);
        assert!(result.is_err(), "Should error on nonsensical argument");
    }

    /// Helper function to setup ArgMatches from given arguments
    fn setup_arg_matches(args: Vec<&str>) -> ArgMatches {
        let app = ct_app(); // Assuming ct_app() returns a clap::Command configured for the application
        let os_args: Vec<OsString> = args.iter().map(|&arg| OsString::from(arg)).collect();
        app.try_get_matches_from(os_args).unwrap()
    }

    #[test]
    fn test_inputs_new_stdin_implicit() {
        let matches = setup_arg_matches(vec!["program"]);
        let inputs = WcInputs::new(&matches).unwrap();
        match inputs {
            WcInputs::Stdin => (), // This is the expected outcome
            _ => panic!("Expected Inputs::Stdin for implicit standard input"),
        }
    }

    #[test]
    fn test_inputs_new_paths() {
        let matches = setup_arg_matches(vec!["program", "file1.txt", "file2.txt"]);
        let inputs = WcInputs::new(&matches).unwrap();
        match inputs {
            WcInputs::Paths(paths) => {
                assert_eq!(paths.len(), 2);
                assert_eq!(paths[0].to_title().unwrap(), "file1.txt");
                assert_eq!(paths[1].to_title().unwrap(), "file2.txt");
            }
            _ => panic!("Expected Inputs::Paths for file arguments"),
        }
    }

    #[test]
    fn test_inputs_new_files0_from() {
        let matches = setup_arg_matches(vec!["program", "--files0-from=file0.txt"]);
        let inputs = WcInputs::new(&matches).unwrap();
        match inputs {
            WcInputs::Files0From(input) => {
                assert_eq!(input.to_title().unwrap(), "file0.txt");
            }
            _ => panic!("Expected Inputs::Files0From for --files0-from argument"),
        }
    }

    #[test]
    fn test_inputs_new_error_on_conflict() {
        let matches = setup_arg_matches(vec!["program", "--files0-from=file0.txt", "file1.txt"]);
        assert!(
            WcInputs::new(&matches).is_err(),
            "Expected error when combining --files0-from with direct file paths"
        );
    }

    #[test]
    fn test_as_borrowed() {
        let owned_path = PathBuf::from("/tmp/example.txt");
        let input_owned = WcInput::Path(Cow::Owned(owned_path.clone()));
        let input_borrowed = input_owned.as_borrowed();

        match input_borrowed {
            WcInput::Path(Cow::Borrowed(path)) => assert_eq!(path, &owned_path),
            _ => panic!("Expected borrowed path"),
        }
    }

    #[test]
    fn test_to_title() {
        let input_path = WcInput::Path(Cow::Owned(PathBuf::from("/tmp/example.txt")));
        let input_stdin_explicit = WcInput::Stdin(StdinKind::Explicit);
        let input_stdin_implicit = WcInput::Stdin(StdinKind::Implicit);

        assert_eq!(
            input_path.to_title(),
            Some(Cow::Borrowed("/tmp/example.txt"))
        );
        assert_eq!(input_stdin_explicit.to_title(), Some(Cow::Borrowed("-")));
        assert!(input_stdin_implicit.to_title().is_none());
    }

    #[test]
    fn test_path_display() {
        let input_path = WcInput::Path(Cow::Owned(PathBuf::from("/tmp/example.txt")));
        let input_stdin = WcInput::Stdin(StdinKind::Explicit);

        assert_eq!(input_path.path_display(), "/tmp/example.txt");
        assert_eq!(input_stdin.path_display(), "standard input");
    }

    #[test]
    fn test_try_as_files0() {
        use std::fs::File;
        use std::io::Write;

        // Setup a temporary file with file names separated by '\0'
        let temp_file_path = "/tmp/files0_test.txt";
        let mut temp_file = File::create(temp_file_path).unwrap();
        writeln!(temp_file, "file1.txt\0file2.txt").unwrap();

        let input = WcInput::Path(Cow::Owned(PathBuf::from(temp_file_path)));

        if let Ok(Some(paths)) = input.try_as_files0() {
            assert_eq!(paths.len(), 2);
            match &paths[0] {
                WcInput::Path(cow) => assert_eq!(cow.as_ref(), Path::new("file1.txt")),
                _ => panic!("Expected path input"),
            }
            match &paths[1] {
                WcInput::Path(cow) => assert_eq!(cow.as_ref(), Path::new("file2.txt\n")),
                _ => panic!("Expected path input"),
            }
        } else {
            panic!("Expected some paths");
        }

        // Cleanup
        std::fs::remove_file(temp_file_path).unwrap();
    }

    #[test]
    fn test_process_chunk_with_empty_string() {
        let mut total = WcWordCount::default();
        let mut current_len = 0;
        let mut in_word = false;
        let text = "";

        process_chunk::<true, true, false, true>(&mut total, text, &mut current_len, &mut in_word);
        assert_eq!(total.words, 0);
        assert_eq!(total.lines, 0);
    }

    #[test]
    fn test_process_chunk_with_long_line() {
        let mut total = WcWordCount::default();
        let long_line = "a".repeat(10000);
        let mut current_len = 0;
        let mut in_word = false;

        process_chunk::<true, true, false, true>(
            &mut total,
            &long_line,
            &mut current_len,
            &mut in_word,
        );
        assert_eq!(total.words, 1, "Should count one long word");
    }

    #[test]
    fn test_handle_io_error() {
        let mut total = WcWordCount::default();
        let error = ReadBufDecoderError::Io(io::Error::new(io::ErrorKind::Other, "test error"));

        let result = handle_error(error, &mut total);
        assert!(result.is_some(), "Should return an io::Error");
    }

    #[test]
    #[cfg(unix)]
    fn test_is_stdin_small_file() {
        use std::os::unix::io::AsRawFd;

        let mut temp_file = tempfile::tempfile().unwrap();
        writeln!(temp_file, "Hello, world!").unwrap();
        let fd = temp_file.as_raw_fd();

        // Duplicate the fd to stdin
        unsafe {
            libc::dup2(fd, libc::STDIN_FILENO);
        }

        assert!(is_stdin_small_file(), "Should recognize small stdin file");

        // Reset stdin to normal
        unsafe {
            libc::dup2(libc::STDIN_FILENO, fd);
        }
    }

    #[test]
    fn test_process_chunk() {
        let mut word_count = WcWordCount::default();
        let mut current_len = 0;
        let mut in_word = false;
        let text = "hello world\nhello rust";

        process_chunk::<true, true, false, true>(
            &mut word_count,
            text,
            &mut current_len,
            &mut in_word,
        );

        assert_eq!(word_count.words, 4);
        assert_eq!(word_count.lines, 1);
        assert_eq!(word_count.chars, text.chars().count());
    }

    #[test]
    fn test_files0_iter_file() {
        let mut temp_file = tempfile::NamedTempFile::new().unwrap();
        write!(temp_file.as_file_mut(), "file1.txt\0file2.txt\0").unwrap();

        let iter = files0_iter_file(temp_file.path()).unwrap();
        let inputs: Vec<_> = iter.collect();

        assert_eq!(inputs.len(), 2);
        assert!(matches!(inputs[0], Ok(WcInput::Path(_))));
        assert!(matches!(inputs[1], Ok(WcInput::Path(_))));
    }

    #[test]
    fn test_word_count_from_reader_with_file() {
        // 创建一个临时文件
        let mut file = tempfile().expect("failed to create temp file");
        writeln!(file, "hello world\nhello rust").expect("failed to write to temp file");
        let settings = WcSettings::default(); // 假设这里的 Settings 可以像这样构建

        // 使用文件进行测试
        let (count, _) = word_count_from_reader(file, &settings);
        assert_eq!(count.words, 0);
        assert_eq!(count.lines, 0);
    }

    #[test]
    fn test_handle_error() {
        let mut total = WcWordCount::default();
        let error = ReadBufDecoderError::InvalidByteSequence(b"abc");

        let result = handle_error(error, &mut total);
        assert_eq!(total.bytes, 3);
        assert!(result.is_none(), "Should not return an io::Error");
    }

    #[test]
    fn test_files0_iter_file2() {
        let mut temp_file = NamedTempFile::new().unwrap();
        write!(temp_file.as_file_mut(), "file1.txt\0file2.txt\0").unwrap();

        let iter = files0_iter_file(temp_file.path()).unwrap();
        let inputs: Vec<_> = iter.collect();

        assert_eq!(inputs.len(), 2);
    }

    // wc 接口: wc [OPTION]... [FILE]...
    //   or:  wc [OPTION]... --files0-from=F
    //
    //   -c, --bytes            print the byte counts
    //   -m, --chars            print the character counts
    //   -l, --lines            print the newline counts
    //       --files0-from=F    read input from the files specified by
    //                            NUL-terminated names in file F;
    //                            If F is - then read names from standard input
    //   -L, --max-line-length  print the maximum display width
    //   -w, --words            print the word counts
    //       --help     display this help and exit
    //       --version  output version information and exit
    #[test]
    fn test_ct_app_execution_version() {
        let command = ct_app();

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), "--version"];

        // Assuming `command` has a method to retrieve the executable name, replace it with the actual one
        let executable = command.try_get_matches_from(args);

        assert!(executable.is_err());
        assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
    }

    #[test]
    fn test_ct_app_execution_other_version() {
        let command = ct_app();

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), "-V"];

        // Assuming `command` has a method to retrieve the executable name, replace it with the actual one
        let executable = command.try_get_matches_from(args);

        assert!(executable.is_err());
        assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
    }

    #[test]
    fn test_ct_app_execution_help() {
        let command = ct_app();

        // 测试用例2：验证 --help 参数是否正确处理
        let help_args = vec![ctcore::ct_util_name(), "--help"];
        let result = command.try_get_matches_from(help_args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
    }

    #[test]
    fn test_ct_app_execution_unsupport_help() {
        let command = ct_app();

        // 测试用例2：验证 --help 参数是否正确处理
        let help_args = vec![ctcore::ct_util_name(), "-H"];
        let result = command.try_get_matches_from(help_args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn test_ct_app_invalid_argument() {
        let command = ct_app();

        // 测试用例3：验证当提供未知参数时是否正确报错
        let invalid_args = vec![ctcore::ct_util_name(), "--invalid-argument"];
        let result = command.try_get_matches_from(invalid_args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn test_ct_app_support_missing_argument() {
        let command = ct_app();

        // 测试用例4：验证当缺少必需的参数时是否正确报错
        let missing_args = vec![ctcore::ct_util_name()]; // 缺少任何参数
        let result = command.try_get_matches_from(missing_args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_bytes() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--bytes"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_chars() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--chars"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_files0_from() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--files0-from", "filename"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_lines() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--lines"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_max_line_length() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--max-line-length"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_total() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--total", "auto"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_words() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--words"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_c() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-c"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_m() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-m"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_l() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-l"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_uppercase_l() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-L"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_w() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-w"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }
}
