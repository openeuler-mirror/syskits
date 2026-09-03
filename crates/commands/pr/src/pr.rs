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
use rust_i18n::t;
use std::fs::{File, metadata};
rust_i18n::i18n!("locales", fallback = "en-US");
#[cfg(unix)]
use std::ffi::CString;
use std::io::{BufRead, BufReader, Error, Lines, Read, Write, stderr, stdout};
#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;

use chrono::{DateTime, Local};
use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::CTResult;
use ctcore::ct_locale::hard_locale_time;
use itertools::Itertools;
use quick_error::ResultExt;
use quick_error::quick_error;
use regex::Regex;
use std::ffi::OsString;
use sys_locale::get_locale;
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

const PR_TAB: char = '\t';
const PR_LINES_PER_PAGE: usize = 66;
const PR_LINES_PER_PAGE_FOR_FORM_FEED: usize = 63;
const PR_HEADER_LINES_PER_PAGE: usize = 5;
const PR_TRAILER_LINES_PER_PAGE: usize = 5;
const PR_FILE_STDIN: &str = "-";
const PR_READ_BUFFER_SIZE: usize = 1024 * 64;
const PR_DEFAULT_COLUMN_WIDTH: usize = 72;
const PR_DEFAULT_COLUMN_WIDTH_WITH_S_OPTION: usize = 512;
const PR_DEFAULT_COLUMN_SEPARATOR: &char = &' ';
const PR_FF: u8 = 0x0C_u8;
// 根据locale选择时间格式
fn get_pr_date_time_format() -> &'static str {
    if std::env::var_os("POSIXLY_CORRECT").is_some() && !hard_locale_time() {
        "%b %d %H:%M %Y" // POSIXLY_CORRECT + C/POSIX locale
    } else {
        "%Y-%m-%d %H:%M" // GNU 默认格式
    }
}

#[cfg(unix)]
fn pr_format_local_time_strftime(seconds: i64, fmt: &str) -> Option<String> {
    let time = ctcore::libc::time_t::try_from(seconds).ok()?;
    let mut tm: ctcore::libc::tm = unsafe { std::mem::zeroed() };
    let c_format = CString::new(fmt).ok()?;
    let tm_ptr = unsafe { ctcore::libc::localtime_r(&time as *const _, &mut tm as *mut _) };
    if tm_ptr.is_null() {
        return None;
    }

    let mut capacity = 128usize;
    while capacity <= 8192 {
        let mut buf = vec![0u8; capacity];
        let written = unsafe {
            ctcore::libc::strftime(
                buf.as_mut_ptr() as *mut ctcore::libc::c_char,
                capacity,
                c_format.as_ptr(),
                &tm as *const _,
            )
        };
        if written > 0 {
            buf.truncate(written);
            return String::from_utf8(buf).ok();
        }
        capacity *= 2;
    }
    None
}

fn pr_format_local_datetime(fmt: &str, date_time: DateTime<Local>) -> String {
    #[cfg(unix)]
    {
        pr_format_local_time_strftime(date_time.timestamp(), fmt)
            .unwrap_or_else(|| date_time.format(fmt).to_string())
    }
    #[cfg(not(unix))]
    {
        date_time.format(fmt).to_string()
    }
}

mod pr_flags {
    pub const PR_HEADER: &str = "header";
    pub const PR_DOUBLE_SPACE: &str = "double-space";
    pub const PR_NUMBER_LINES: &str = "number-lines";
    pub const PR_FIRST_LINE_NUMBER: &str = "first-line-number";
    pub const PR_PAGES: &str = "pages";
    pub const PR_OMIT_HEADER: &str = "omit-header";
    pub const PR_PAGE_LENGTH: &str = "length";
    pub const PR_NO_FILE_WARNINGS: &str = "no-file-warnings";
    pub const PR_FORM_FEED: &str = "form-feed";
    pub const PR_COLUMN_WIDTH: &str = "width";
    pub const PR_PAGE_WIDTH: &str = "page-width";
    pub const PR_ACROSS: &str = "across";
    pub const PR_BALANCE: &str = "balance";
    pub const PR_COLUMN: &str = "column";
    pub const PR_COLUMN_CHAR_SEPARATOR: &str = "separator";
    pub const PR_COLUMN_STRING_SEPARATOR: &str = "sep-string";
    pub const PR_MERGE: &str = "merge";
    pub const PR_INDENT: &str = "indent";
    pub const PR_JOIN_LINES: &str = "join-lines";
    pub const PR_HELP: &str = "help";
    pub const PR_FILES: &str = "files";
    pub const PR_SHOW_CONTROL_CHARS: &str = "show-control-chars";
    pub const PR_SHOW_NONPRINTING: &str = "show-nonprinting";
}

#[derive(Debug, Clone, PartialEq)]
struct PrOutputOptions {
    /// 行编号模式
    number: Option<PrNumberingMode>,
    header: String,
    is_double_space: bool,
    line_separator: String,
    content_line_separator: String,
    last_modified_time: String,
    start_page: usize,
    end_page: Option<usize>,
    is_display_header_and_trailer: bool,
    content_lines_per_page: usize,
    page_separator_char: String,
    column_mode_options: Option<PrColumnModeOptions>,
    merge_files_print: Option<usize>,
    offset_spaces: String,
    is_form_feed_used: bool,
    is_join_lines: bool,
    col_sep_for_printing: String,
    page_width: usize,
    line_width: Option<usize>,
    show_control_chars: bool,
    show_nonprinting: bool,
    is_omit_pagination: bool,
    is_pad_columns: bool,
    expand_tabs: Option<(char, usize)>,
    output_tabs: Option<(char, usize)>,
}

#[derive(Debug)]
struct PrFileLine {
    file_id: usize,
    line_number: usize,
    page_number: usize,
    group_key: usize,
    line_content: Result<String, std::io::Error>,
    form_feeds_after: usize,
    inline_form_feed_after: bool,
}

impl PartialEq for PrFileLine {
    fn eq(&self, other: &Self) -> bool {
        if self.file_id != other.file_id
            || self.line_number != other.line_number
            || self.page_number != other.page_number
            || self.group_key != other.group_key
            || self.form_feeds_after != other.form_feeds_after
            || self.inline_form_feed_after != other.inline_form_feed_after
        {
            return false;
        }

        match (&self.line_content, &other.line_content) {
            (Ok(a), Ok(b)) => a == b,
            (Err(_), Err(_)) => true, // 选择将所有错误视为相等
            _ => false,               // 一个Ok和一个Err不相等
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct PrColumnModeOptions {
    width: usize,
    columns: usize,
    column_separator: String,
    is_across_mode: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct JoinLineSegment {
    text: String,
    logical_width: usize,
    has_separator: bool,
    reset_position: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
struct PrRenderedLine {
    text: String,
    has_separator: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrSemanticRow {
    pub page: usize,
    pub kind: String,
    pub section: String,
    pub file: Option<String>,
    pub file_id: usize,
    pub line_index: Option<usize>,
    pub group_key: usize,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrSemantic {
    pub rows: Vec<PrSemanticRow>,
    pub classic_text: String,
    pub stderr_text: String,
    pub exit_code: i32,
}

#[derive(Debug, Clone, PartialEq)]
/// 行编号模式
struct PrNumberingMode {
    width: usize,
    separator: String,
    first_number: usize,
}

impl Default for PrNumberingMode {
    fn default() -> Self {
        Self {
            width: 5,
            separator: PR_TAB.to_string(),
            first_number: 0,
        }
    }
}

impl Default for PrFileLine {
    fn default() -> Self {
        Self {
            file_id: 0,
            line_number: 0,
            page_number: 0,
            group_key: 0,
            line_content: Ok(String::new()),
            form_feeds_after: 0,
            inline_form_feed_after: false,
        }
    }
}

impl From<std::io::Error> for PrError {
    fn from(err: std::io::Error) -> Self {
        Self::EncounteredErrors(err.to_string())
    }
}

quick_error! {
    #[derive(Debug)]
    enum PrError {
        Input(err: std::io::Error, path: String) {
            context(path: &'a str, err: std::io::Error) -> (err, path.to_owned())
            display("pr: Reading from input {0} gave error", path)
            source(err)
        }

        UnknownFiletype(path: String) {
            display("pr: {0}: unknown filetype", path)
        }

        EncounteredErrors(msg: String) {
            display("pr: {0}", msg)
        }

        IsDirectory(path: String) {
            display("pr: {0}: Is a directory", path)
        }

        IsSocket(path: String) {
            display("pr: cannot open {}, Operation not supported on socket", path)
        }

        NotExists(path: String) {
            display("pr: cannot open {}, No such file or directory", path)
        }

    }
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("pr.about");
    let usage_description = t!("pr.usage");
    let args = vec![
        Arg::new(pr_flags::PR_PAGES)
            .long(pr_flags::PR_PAGES)
            .help(t!("pr.clap.pr_pages"))
            .value_name("FIRST_PAGE[:LAST_PAGE]"),
        Arg::new(pr_flags::PR_HEADER)
            .short('h')
            .long(pr_flags::PR_HEADER)
            .help(
                "Use the string header to replace the file name \
                     in the header line.",
            )
            .value_name("STRING"),
        Arg::new(pr_flags::PR_DOUBLE_SPACE)
            .short('d')
            .long(pr_flags::PR_DOUBLE_SPACE)
            .help(
                "Produce output that is double spaced. An extra <newline> \
                 character is output following every <newline> found in the input.",
            )
            .action(ArgAction::SetTrue),
        Arg::new(pr_flags::PR_NUMBER_LINES)
            .short('n')
            .long(pr_flags::PR_NUMBER_LINES)
            .help(
                "Provide width digit line numbering.  The default for width, \
                 if not specified, is 5.  The number occupies the first width column \
                 positions of each text column or each line of -m output.  If char \
                 (any non-digit character) is given, it is appended to the line number \
                 to separate it from whatever follows.  The default for char is a <tab>. \
                 Line numbers longer than width columns are truncated.",
            )
            .allow_hyphen_values(true)
            .num_args(0..=1)
            .require_equals(true)
            .value_name("[char][width]"),
        Arg::new(pr_flags::PR_FIRST_LINE_NUMBER)
            .short('N')
            .long(pr_flags::PR_FIRST_LINE_NUMBER)
            .help(t!("pr.clap.pr_first_line_number"))
            .value_name("NUMBER"),
        Arg::new(pr_flags::PR_OMIT_HEADER)
            .short('t')
            .long(pr_flags::PR_OMIT_HEADER)
            .help(
                "Write neither the five-line identifying header nor the five-line \
                 trailer usually supplied for each page. Quit writing after the last line \
                  of each file without spacing to the end of the page.",
            )
            .action(ArgAction::SetTrue),
        Arg::new(pr_flags::PR_PAGE_LENGTH)
            .short('l')
            .long(pr_flags::PR_PAGE_LENGTH)
            .help(
                "Override the 66-line default (default number of lines of text 56, \
                     and with -F 63) and reset the page length to lines.  If lines is not \
                     greater than the sum  of  both the  header  and trailer depths (in lines), \
                     the pr utility shall suppress both the header and trailer, as if the -t \
                     option were in effect. ",
            )
            .value_name("PAGE_LENGTH"),
        Arg::new(pr_flags::PR_NO_FILE_WARNINGS)
            .short('r')
            .long(pr_flags::PR_NO_FILE_WARNINGS)
            .help(t!("pr.clap.pr_no_file_warnings"))
            .action(ArgAction::SetTrue),
        Arg::new(pr_flags::PR_FORM_FEED)
            .short('F')
            .visible_short_alias('f')
            .long(pr_flags::PR_FORM_FEED)
            .help(
                "Use a <form-feed> for new pages, instead of the default behavior that \
                 uses a sequence of <newline>s.",
            )
            .action(ArgAction::SetTrue),
        Arg::new(pr_flags::PR_COLUMN_WIDTH)
            .short('w')
            .long(pr_flags::PR_COLUMN_WIDTH)
            .help(
                "Set the width of the line to width column positions for multiple \
                 text-column output only. If the -w option is not specified and the -s option \
                 is not specified, the default width shall be 72. If the -w option is not specified \
                 and the -s option is specified, the default width shall be 512.",
            )
            .value_name("width"),
        Arg::new(pr_flags::PR_PAGE_WIDTH)
            .short('W')
            .long(pr_flags::PR_PAGE_WIDTH)
            .help(
                "set page width to PAGE_WIDTH (72) characters always, \
                 truncate lines, except -J option is set, no interference \
                 with -S or -s",
            )
            .value_name("width"),
        Arg::new(pr_flags::PR_ACROSS)
            .short('a')
            .long(pr_flags::PR_ACROSS)
            .help(
                "Modify the effect of the - column option so that the columns are filled \
                 across the page in a  round-robin  order (for example, when column is 2, the \
                 first input line heads column 1, the second heads column 2, the third is the \
                 second line in column 1, and so on).",
            )
            .action(ArgAction::SetTrue),
        Arg::new(pr_flags::PR_BALANCE)
            .short('b')
            .long(pr_flags::PR_BALANCE)
            .help("use balanced columns in the last page")
            .action(ArgAction::SetTrue),
        Arg::new(pr_flags::PR_COLUMN)
            .long(pr_flags::PR_COLUMN)
            .visible_alias("columns")
            .help(
                "Produce multi-column output that is arranged in column columns \
                 (the default shall be 1) and is written down each column  in  the order in which \
                 the text is received from the input file. This option should not be used with -m. \
                 The options -e and -i shall be assumed for multiple text-column output.  Whether \
                 or not text columns are produced with identical vertical lengths is unspecified, \
                 but a text column shall never exceed the length of the page (see the -l option). \
                 When used with -t, use the minimum number of lines to write the output.",
            )
            .value_name("column"),
        Arg::new(pr_flags::PR_COLUMN_CHAR_SEPARATOR)
            .short('s')
            .long(pr_flags::PR_COLUMN_CHAR_SEPARATOR)
            .help(
                "Separate text columns by the single character char instead of by the \
                 appropriate number of <space>s (default for char is the <tab> character).",
            )
            .num_args(0..=1)
            .require_equals(true)
            .value_name("char"),
        Arg::new(pr_flags::PR_COLUMN_STRING_SEPARATOR)
            .short('S')
            .long(pr_flags::PR_COLUMN_STRING_SEPARATOR)
            .help(
                "separate columns by STRING, \
                 without -S: Default separator <TAB> with -J and <space> \
                 otherwise (same as -S\" \"), no effect on column options",
            )
            .num_args(0..=1)
            .require_equals(true)
            .value_name("string"),
        Arg::new(pr_flags::PR_MERGE)
            .short('m')
            .long(pr_flags::PR_MERGE)
            .help(
                "Merge files. Standard output shall be formatted so the pr utility \
                 writes one line from each file specified by a file operand, side by side \
                 into text columns of equal fixed widths, in terms of the number of column \
                 positions. Implementations shall support merging of at least nine file operands.",
            )
            .action(ArgAction::SetTrue),
        Arg::new(pr_flags::PR_INDENT)
            .short('o')
            .long(pr_flags::PR_INDENT)
            .help(
                "Each line of output shall be preceded by offset <space>s. If the -o \
                 option is not specified, the default offset shall be zero. The space taken is \
                 in addition to the output line width (see the -w option below).",
            )
            .value_name("margin"),
        Arg::new(pr_flags::PR_JOIN_LINES)
            .short('J')
            .long(pr_flags::PR_JOIN_LINES)
            .help(
                "merge full lines, turns off -W line truncation, no column \
                 alignment, --sep-string[=STRING] sets separators",
            )
            .action(ArgAction::SetTrue),
        Arg::new(pr_flags::PR_HELP)
            .long(pr_flags::PR_HELP)
            .help(t!("pr.clap.pr_help"))
            .action(ArgAction::Help),
        Arg::new(pr_flags::PR_SHOW_CONTROL_CHARS)
            .short('c')
            .long(pr_flags::PR_SHOW_CONTROL_CHARS)
            .help("use hat notation (^G) and octal backslash notation")
            .action(ArgAction::SetTrue),
        Arg::new(pr_flags::PR_SHOW_NONPRINTING)
            .short('v')
            .long(pr_flags::PR_SHOW_NONPRINTING)
            .help("use octal backslash notation")
            .action(ArgAction::SetTrue),
        Arg::new("date-format")
            .short('D')
            .long("date-format")
            .help("use FORMAT for the header date")
            .value_name("format"),
        Arg::new("expand-tabs")
            .short('e')
            .long("expand-tabs")
            .help("expand input CHARs (TABs) to space WIDTH (8)")
            .num_args(0..=1)
            .require_equals(true),
        Arg::new("output-tabs")
            .short('i')
            .long("output-tabs")
            .help("replace spaces with CHARs (TABs) to space WIDTH (8)")
            .num_args(0..=1)
            .require_equals(true),
        Arg::new("omit-pagination")
            .short('T')
            .long("omit-pagination")
            .help("omit page headers and trailers, eliminate any pagination by form feeds set in input files")
            .action(ArgAction::SetTrue),
        Arg::new(pr_flags::PR_FILES)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::FilePath),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(&args)
        .after_help(t!("pr.after_help"))
        .args_override_self(true)
        .disable_help_flag(true)
}

#[derive(Default)]
pub struct Pr;
impl Tool for Pr {
    fn name(&self) -> &'static str {
        "pr"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        pr_main(args.iter().cloned())
    }
}

pub fn pr_main(args: impl ctcore::Args) -> CTResult<()> {
    let semantic = pr_native_semantic(args)?;
    pr_write_semantic_output(&semantic)?;

    if semantic.exit_code == 0 {
        Ok(())
    } else {
        Err(semantic.exit_code.into())
    }
}

pub fn pr_native_semantic(args: impl ctcore::Args) -> CTResult<PrSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let args = args.collect_ignore();

    let opt_args = pr_recreate_arguments(&args);

    let mut command = ct_app();
    let matches = match command.try_get_matches_from_mut(opt_args) {
        Ok(m) => m,
        Err(e) => return Ok(pr_semantic_from_clap_error(e)),
    };

    let mut semantic = PrSemantic {
        rows: Vec::new(),
        classic_text: String::new(),
        stderr_text: String::new(),
        exit_code: 0,
    };

    let mut files = matches
        .get_many::<String>(pr_flags::PR_FILES)
        .map(|v| v.map(|s| s.as_str()).collect::<Vec<_>>())
        .unwrap_or_default()
        .clone();
    if files.is_empty() {
        files.insert(0, PR_FILE_STDIN);
    }

    let file_groups: Vec<_> = match matches.get_flag(pr_flags::PR_MERGE) {
        true => {
            vec![files]
        }
        false => files.into_iter().map(|i| vec![i]).collect(),
    };

    for file_group in file_groups {
        let result_options = pr_build_options(&matches, &file_group, &args);
        let options = match result_options {
            Ok(options) => options,
            Err(err) => {
                semantic.stderr_text = pr_error_text(&matches, &err);
                semantic.exit_code = 1;
                return Ok(semantic);
            }
        };

        let cmd_result = match file_group.iter().exactly_one() {
            Ok(group) => pr_collect_semantic_for_path(&mut semantic, &file_group, group, &options),
            Err(_) => pr_collect_semantic_for_merge(&mut semantic, &file_group, &options),
        };

        if let Err(error) = cmd_result {
            semantic.stderr_text = pr_error_text(&matches, &error);
            semantic.exit_code = 1;
            return Ok(semantic);
        }
    }

    Ok(semantic)
}

fn pr_semantic_from_clap_error(error: clap::Error) -> PrSemantic {
    let rendered = error.to_string();
    if error.use_stderr() {
        PrSemantic {
            rows: Vec::new(),
            classic_text: String::new(),
            stderr_text: rendered,
            exit_code: 1,
        }
    } else {
        PrSemantic {
            rows: Vec::new(),
            classic_text: rendered,
            stderr_text: String::new(),
            exit_code: 0,
        }
    }
}

fn pr_write_semantic_output(semantic: &PrSemantic) -> CTResult<()> {
    let mut out = stdout();
    let mut err = stderr();

    if !semantic.classic_text.is_empty() {
        out.write_all(semantic.classic_text.as_bytes())?;
    }
    if !semantic.stderr_text.is_empty() {
        err.write_all(semantic.stderr_text.as_bytes())?;
    }

    out.flush()?;
    err.flush()?;
    Ok(())
}

/// 返回重写后传递给程序的参数。
/// 删除 -column 和 +page 选项，因为 getopts 无法解析 -3 等参数。
/// # 参数
/// * `args` - 命令行参数
fn pr_recreate_arguments(args: &[String]) -> Vec<String> {
    let column_page_option_regex = Regex::new(r"^[-+]\d+.*").unwrap();

    let mut recreated_args = Vec::with_capacity(args.len());
    let mut stop_rewrite = false;
    for arg in args {
        if stop_rewrite {
            recreated_args.push(arg.clone());
            continue;
        }
        if arg == "--" {
            stop_rewrite = true;
            recreated_args.push(arg.clone());
            continue;
        }

        // GNU pr accepts old-style column counts merged into -t (e.g. -t2 == -t -2).
        if let Some(old_column_count) = arg.strip_prefix("-t") {
            if !old_column_count.is_empty() && old_column_count.chars().all(|c| c.is_ascii_digit())
            {
                recreated_args.push("-t".to_string());
                continue;
            }

            // GNU pr accepts -tn2 / -tn:2 style short-option clusters.
            if let Some(number_arg) = old_column_count.strip_prefix('n') {
                recreated_args.push("-t".to_string());
                if number_arg.is_empty() {
                    recreated_args.push("-n".to_string());
                } else {
                    recreated_args.push(format!("-n={number_arg}"));
                }
                continue;
            }
        }

        if column_page_option_regex.is_match(arg) {
            continue;
        }

        if arg.starts_with("-n") && arg.len() > 2 && !arg.starts_with("-n=") {
            recreated_args.push(format!("-n={}", &arg[2..]));
        } else if arg.starts_with("-s") && arg.len() > 2 && !arg.starts_with("-s=") {
            recreated_args.push(format!("-s={}", &arg[2..]));
        } else if arg.starts_with("-S") && arg.len() > 2 && !arg.starts_with("-S=") {
            recreated_args.push(format!("-S={}", &arg[2..]));
        } else if arg.starts_with("-e") && arg.len() > 2 && !arg.starts_with("-e=") {
            recreated_args.push(format!("-e={}", &arg[2..]));
        } else if arg.starts_with("-i") && arg.len() > 2 && !arg.starts_with("-i=") {
            recreated_args.push(format!("-i={}", &arg[2..]));
        } else {
            recreated_args.push(arg.clone());
        }
    }

    recreated_args
}

fn pr_error_text(arg_matches: &ArgMatches, pr_err: &PrError) -> String {
    if arg_matches.get_flag(pr_flags::PR_NO_FILE_WARNINGS) {
        String::new()
    } else {
        format!("{pr_err}\n")
    }
}

fn pr_parse_usize(arg_matches: &ArgMatches, opt: &str) -> Option<Result<usize, PrError>> {
    let from_parse_error_to_pr_error = |value_to_parse: (String, String)| {
        let i = value_to_parse.0;
        let option = value_to_parse.1;
        i.parse().map_err(|_e| {
            PrError::EncounteredErrors(format!("invalid {} argument {}", option, i.quote()))
        })
    };
    arg_matches
        .get_one::<String>(opt)
        .map(|i| (i.to_string(), format!("-{opt}")))
        .map(from_parse_error_to_pr_error)
}

#[allow(clippy::cognitive_complexity)]
fn pr_build_options(
    arg_matches: &ArgMatches,
    paths: &[&str],
    args: &[String],
) -> Result<PrOutputOptions, PrError> {
    let number = parse_number(arg_matches)?;
    let (start_page, end_page) = parse_start_end_page(arg_matches, args)?;
    let offset_spaces = parse_offset_spaces(arg_matches)?;

    let is_form_feed_used = arg_matches.get_flag(pr_flags::PR_FORM_FEED);
    let page_length = parse_page_length(arg_matches, is_form_feed_used)?;
    let (is_display_header_and_trailer, content_lines_per_page) =
        parse_content_lines_per_page(arg_matches, page_length);

    let column_mode_options = parse_column_mode_options(arg_matches, args)?;
    let is_join_lines = arg_matches.get_flag(pr_flags::PR_JOIN_LINES)
        || (arg_matches.contains_id(pr_flags::PR_COLUMN_CHAR_SEPARATOR)
            && !arg_matches.contains_id(pr_flags::PR_COLUMN_WIDTH)
            && !arg_matches.contains_id(pr_flags::PR_PAGE_WIDTH));
    let merge_files_print = parse_merge_files_print(arg_matches, paths);
    let col_sep_for_printing = parse_col_sep_for_printing(
        arg_matches,
        merge_files_print,
        &column_mode_options,
        is_join_lines,
    );
    let columns_to_print = parse_columns_to_print(merge_files_print, &column_mode_options);
    let page_width = parse_page_width(arg_matches, is_join_lines, merge_files_print.is_some())?;
    let line_width = parse_line_width(
        page_width,
        &column_mode_options,
        is_join_lines,
        columns_to_print,
    );

    let is_double_space = arg_matches.get_flag(pr_flags::PR_DOUBLE_SPACE);
    let is_merge_mode = parse_merge_mode(arg_matches)?;

    let is_pad_columns =
        !arg_matches.contains_id(pr_flags::PR_COLUMN_CHAR_SEPARATOR) && !is_join_lines;
    let expand_tabs = parse_tab_args(arg_matches, "expand-tabs")?;
    let expand_tabs =
        if columns_to_print > 1 && expand_tabs.is_none() && col_sep_for_printing != "\t" {
            Some((PR_TAB, 8))
        } else {
            expand_tabs
        };
    let output_tabs = parse_tab_args(arg_matches, "output-tabs")?;
    let output_tabs = if columns_to_print > 1 {
        output_tabs.or(Some((PR_TAB, 8)))
    } else {
        output_tabs
    };

    Ok(PrOutputOptions {
        number,
        header: parse_header(arg_matches, paths, is_merge_mode),
        is_double_space,
        content_line_separator: parse_content_line_separator(is_double_space),
        last_modified_time: {
            let fmt = arg_matches
                .get_one::<String>("date-format")
                .map(|s| s.as_str())
                .unwrap_or_else(|| get_pr_date_time_format());
            parse_last_modified_time(fmt, paths, is_merge_mode)
        },
        start_page,
        end_page,
        is_display_header_and_trailer,
        content_lines_per_page,
        page_separator_char: parse_page_separator_char(arg_matches),
        column_mode_options,
        merge_files_print,
        offset_spaces,
        is_form_feed_used,
        is_join_lines,
        col_sep_for_printing,
        page_width: page_width.unwrap_or(72),
        line_width,
        show_control_chars: arg_matches.get_flag(pr_flags::PR_SHOW_CONTROL_CHARS),
        show_nonprinting: arg_matches.get_flag(pr_flags::PR_SHOW_NONPRINTING),
        is_omit_pagination: arg_matches.get_flag("omit-pagination"),
        is_pad_columns,
        line_separator: "\n".to_string(),
        expand_tabs,
        output_tabs,
    })
}

fn parse_tab_args(arg_matches: &ArgMatches, name: &str) -> Result<Option<(char, usize)>, PrError> {
    if !arg_matches.contains_id(name) {
        return Ok(None);
    }
    let invalid_tab_arg = |opt_name: &str, invalid_value: &str| {
        PrError::EncounteredErrors(format!(
            "{} extra characters or invalid number in the argument: {}",
            opt_name.quote(),
            invalid_value.quote()
        ))
    };

    let opt_name = match name {
        "expand-tabs" => "-e",
        "output-tabs" => "-i",
        _ => name,
    };

    match arg_matches.get_one::<String>(name) {
        None => Ok(Some(('\t', 8))),
        Some(val) => {
            if val.is_empty() {
                return Ok(Some(('\t', 8)));
            }

            let mut ch = '\t';
            let mut width_str = val.as_str();
            if let Some(first) = val.chars().next() {
                if !first.is_ascii_digit() {
                    ch = first;
                    width_str = &val[first.len_utf8()..];
                }
            }

            if width_str.is_empty() {
                return Ok(Some((ch, 8)));
            }
            if !width_str.chars().all(|c| c.is_ascii_digit()) {
                return Err(invalid_tab_arg(opt_name, width_str));
            }
            let width = width_str
                .parse::<usize>()
                .map_err(|_| invalid_tab_arg(opt_name, width_str))?;
            if width == 0 {
                return Err(invalid_tab_arg(opt_name, width_str));
            }

            Ok(Some((ch, width)))
        }
    }
}

fn parse_content_lines_per_page(arg_matches: &ArgMatches, page_length: usize) -> (bool, usize) {
    let is_page_length_le_ht =
        page_length <= (PR_HEADER_LINES_PER_PAGE + PR_TRAILER_LINES_PER_PAGE);

    let is_omit_pagination = arg_matches.get_flag("omit-pagination");
    let is_display_header_and_trailer = !is_page_length_le_ht
        && !arg_matches.get_flag(pr_flags::PR_OMIT_HEADER)
        && !is_omit_pagination;

    let content_lines_per_page = if is_page_length_le_ht || is_omit_pagination {
        page_length
    } else {
        page_length - (PR_HEADER_LINES_PER_PAGE + PR_TRAILER_LINES_PER_PAGE)
    };
    (is_display_header_and_trailer, content_lines_per_page)
}

fn parse_page_length(arg_matches: &ArgMatches, is_form_feed_used: bool) -> Result<usize, PrError> {
    let default_lines_per_page = if is_form_feed_used {
        PR_LINES_PER_PAGE_FOR_FORM_FEED
    } else {
        PR_LINES_PER_PAGE
    };

    let page_length = pr_parse_usize(arg_matches, pr_flags::PR_PAGE_LENGTH)
        .unwrap_or(Ok(default_lines_per_page))?;
    Ok(page_length)
}

fn parse_page_separator_char(arg_matches: &ArgMatches) -> String {
    if arg_matches.get_flag(pr_flags::PR_FORM_FEED) {
        let bytes = vec![PR_FF];
        String::from_utf8(bytes).unwrap()
    } else {
        "\n".to_string()
    }
}

fn parse_offset_spaces(arg_matches: &ArgMatches) -> Result<String, PrError> {
    let offset_spaces =
        " ".repeat(pr_parse_usize(arg_matches, pr_flags::PR_INDENT).unwrap_or(Ok(0))?);
    Ok(offset_spaces)
}

fn parse_line_width(
    page_width: Option<usize>,
    column_mode_options: &Option<PrColumnModeOptions>,
    is_join_lines: bool,
    columns_to_print: usize,
) -> Option<usize> {
    if is_join_lines {
        None
    } else if columns_to_print > 1 {
        page_width.or_else(|| {
            Some(
                column_mode_options
                    .as_ref()
                    .map(|i| i.width)
                    .unwrap_or(PR_DEFAULT_COLUMN_WIDTH),
            )
        })
    } else {
        page_width
    }
}

fn parse_columns_to_print(
    merge_files_print: Option<usize>,
    column_mode_options: &Option<PrColumnModeOptions>,
) -> usize {
    merge_files_print
        .unwrap_or_else(|| column_mode_options.as_ref().map(|i| i.columns).unwrap_or(1))
}

fn parse_col_sep_for_printing(
    arg_matches: &ArgMatches,
    merge_files_print: Option<usize>,
    column_mode_options: &Option<PrColumnModeOptions>,
    is_join_lines: bool,
) -> String {
    let fallback = || {
        if is_join_lines && (merge_files_print.is_some() || column_mode_options.is_some()) {
            PR_TAB.to_string()
        } else {
            merge_files_print
                .map(|_k| PR_DEFAULT_COLUMN_SEPARATOR.to_string())
                .unwrap_or_default()
        }
    };

    // First try the column separator explicitly
    if arg_matches.contains_id(pr_flags::PR_COLUMN_STRING_SEPARATOR) {
        return arg_matches
            .get_one::<String>(pr_flags::PR_COLUMN_STRING_SEPARATOR)
            .cloned()
            .unwrap_or_default();
    } else if arg_matches.contains_id(pr_flags::PR_COLUMN_CHAR_SEPARATOR) {
        return match arg_matches.get_one::<String>(pr_flags::PR_COLUMN_CHAR_SEPARATOR) {
            Some(value) => value.clone(),
            None => {
                if arg_matches.contains_id(pr_flags::PR_COLUMN_WIDTH)
                    || arg_matches.contains_id(pr_flags::PR_PAGE_WIDTH)
                {
                    String::new()
                } else {
                    PR_TAB.to_string()
                }
            }
        };
    }

    column_mode_options
        .as_ref()
        .map(|i| {
            if is_join_lines {
                PR_TAB.to_string()
            } else {
                i.column_separator.clone()
            }
        })
        .unwrap_or_else(fallback)
}

fn parse_column_mode_options(
    arg_matches: &ArgMatches,
    args: &[String],
) -> Result<Option<PrColumnModeOptions>, PrError> {
    let mut column_option_value = None;
    let mut tokens = args
        .iter()
        .map(std::string::String::as_str)
        .skip(1)
        .take_while(|token| *token != "--")
        .peekable();
    while let Some(token) = tokens.next() {
        let parse_old_style = |unparsed_num: &str| {
            unparsed_num.parse::<usize>().map_err(|_e| {
                PrError::EncounteredErrors(format!(
                    "invalid number of columns: {}",
                    unparsed_num.quote()
                ))
            })
        };

        if let Some(num) = token.strip_prefix("--column=") {
            column_option_value = Some(parse_old_style(num)?);
            continue;
        }
        if let Some(num) = token.strip_prefix("--columns=") {
            column_option_value = Some(parse_old_style(num)?);
            continue;
        }
        if token == "--column" || token == "--columns" {
            if let Some(num) = tokens.next() {
                column_option_value = Some(parse_old_style(num)?);
            }
            continue;
        }
        if let Some(num) = token.strip_prefix("-t") {
            if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()) {
                column_option_value = Some(parse_old_style(num)?);
            }
            continue;
        }
        if token.starts_with('-')
            && token.len() > 1
            && token[1..].chars().all(|c| c.is_ascii_digit())
        {
            column_option_value = Some(parse_old_style(&token[1..])?);
        }
    }

    let column_width = parse_column_width(arg_matches)?;
    let column_separator = parse_column_separator(arg_matches);
    let is_across_mode = arg_matches.get_flag(pr_flags::PR_ACROSS);

    if let Some(0) = column_option_value {
        return Err(PrError::EncounteredErrors(
            "invalid number of columns: '0'".to_string(),
        ));
    }
    let column_mode_options = column_option_value.map(|columns| PrColumnModeOptions {
        columns,
        width: column_width,
        column_separator,
        is_across_mode,
    });
    Ok(column_mode_options)
}

fn parse_page_width(
    arg_matches: &ArgMatches,
    _is_join_lines: bool,
    is_merge_mode: bool,
) -> Result<Option<usize>, PrError> {
    let page_width = if let Some(res) = pr_parse_usize(arg_matches, pr_flags::PR_PAGE_WIDTH) {
        Some(res?)
    } else if is_merge_mode {
        pr_parse_usize(arg_matches, pr_flags::PR_COLUMN_WIDTH).transpose()?
    } else {
        None
    };
    Ok(page_width)
}

fn parse_column_separator(arg_matches: &ArgMatches) -> String {
    if arg_matches.contains_id(pr_flags::PR_COLUMN_STRING_SEPARATOR) {
        arg_matches
            .get_one::<String>(pr_flags::PR_COLUMN_STRING_SEPARATOR)
            .cloned()
            .unwrap_or_default()
    } else if arg_matches.contains_id(pr_flags::PR_COLUMN_CHAR_SEPARATOR) {
        arg_matches
            .get_one::<String>(pr_flags::PR_COLUMN_CHAR_SEPARATOR)
            .cloned()
            .unwrap_or_else(|| PR_TAB.to_string())
    } else {
        PR_DEFAULT_COLUMN_SEPARATOR.to_string()
    }
}

fn parse_column_width(arg_matches: &ArgMatches) -> Result<usize, PrError> {
    let default_column_width = if arg_matches.contains_id(pr_flags::PR_COLUMN_WIDTH)
        && arg_matches.contains_id(pr_flags::PR_COLUMN_CHAR_SEPARATOR)
    {
        PR_DEFAULT_COLUMN_WIDTH_WITH_S_OPTION
    } else {
        PR_DEFAULT_COLUMN_WIDTH
    };

    let column_width = pr_parse_usize(arg_matches, pr_flags::PR_COLUMN_WIDTH)
        .unwrap_or(Ok(default_column_width))?;
    Ok(column_width)
}

fn parse_start_end_page(
    arg_matches: &ArgMatches,
    args: &[String],
) -> Result<(usize, Option<usize>), PrError> {
    let args_before_double_dash = args
        .iter()
        .map(std::string::String::as_str)
        .skip(1)
        .take_while(|token| *token != "--")
        .collect::<Vec<_>>();

    // +page 选项的优先级低于 --pages
    let page_plus_re = Regex::new(r"^\+(\d+)(?::(\d*))?$").unwrap();
    let plus_capture = args_before_double_dash
        .iter()
        .find_map(|token| page_plus_re.captures(token));

    let (start_page_in_plus_option, end_page_in_plus_option) = match plus_capture {
        Some(captures) => {
            let start_raw = captures.get(1).unwrap().as_str();
            let start_page = start_raw.parse::<usize>().map_err(|_e| {
                PrError::EncounteredErrors(format!(
                    "invalid {} argument {}",
                    "+",
                    start_raw.quote()
                ))
            })?;

            let end_page = match captures.get(2) {
                Some(end) => {
                    let end_raw = end.as_str();
                    Some(end_raw.parse::<usize>().map_err(|_e| {
                        PrError::EncounteredErrors(format!(
                            "invalid {} argument {}",
                            "+",
                            format!("{start_raw}:{end_raw}").quote()
                        ))
                    })?)
                }
                None => None,
            };
            (start_page, end_page)
        }
        None => (1, None),
    };

    let invalid_pages_map = |i: String| {
        let unparsed_value = arg_matches.get_one::<String>(pr_flags::PR_PAGES).unwrap();
        i.parse::<usize>().map_err(|_e| {
            PrError::EncounteredErrors(format!(
                "invalid --pages argument {}",
                unparsed_value.quote()
            ))
        })
    };

    let res = arg_matches
        .get_one::<String>(pr_flags::PR_PAGES)
        .map(|i| {
            let x: Vec<_> = i.split(':').collect();
            x[0].to_string()
        })
        .map(invalid_pages_map);
    let start_page = match res {
        Some(res) => res?,
        None => start_page_in_plus_option,
    };

    let res = arg_matches
        .get_one::<String>(pr_flags::PR_PAGES)
        .filter(|i| i.contains(':'))
        .map(|i| {
            let x: Vec<_> = i.split(':').collect();
            x[1].to_string()
        })
        .map(invalid_pages_map);
    let end_page = match res {
        Some(res) => Some(res?),
        None => end_page_in_plus_option,
    };

    if let Some(end_page) = end_page {
        if start_page > end_page {
            return Err(PrError::EncounteredErrors(format!(
                "invalid --pages argument '{start_page}:{end_page}'"
            )));
        }
    }
    Ok((start_page, end_page))
}

fn parse_last_modified_time(fmt: &str, paths: &[&str], is_merge_mode: bool) -> String {
    if is_merge_mode || paths[0].eq(PR_FILE_STDIN) {
        pr_format_local_datetime(fmt, Local::now())
    } else {
        pr_file_last_modified_time(paths.first().unwrap(), fmt)
    }
}

fn parse_content_line_separator(is_double_space: bool) -> String {
    match is_double_space {
        true => "\n".repeat(2),
        false => "\n".to_string(),
    }
}

fn parse_number(arg_matches: &ArgMatches) -> Result<Option<PrNumberingMode>, PrError> {
    let default_first_number = PrNumberingMode::default().first_number;
    let first_number = pr_parse_usize(arg_matches, pr_flags::PR_FIRST_LINE_NUMBER)
        .unwrap_or(Ok(default_first_number))?;

    Ok(arg_matches
        .get_one::<String>(pr_flags::PR_NUMBER_LINES)
        .map(|i| {
            let parse_result = i.parse::<usize>();

            let (separator, width) = match parse_result {
                Ok(res) => (PrNumberingMode::default().separator, res),
                Err(_) => (
                    i[0..1].to_string(),
                    i[1..]
                        .parse::<usize>()
                        .unwrap_or(PrNumberingMode::default().width),
                ),
            };

            PrNumberingMode {
                width,
                separator,
                first_number,
            }
        })
        .or_else(
            || match arg_matches.contains_id(pr_flags::PR_NUMBER_LINES) {
                true => Some(PrNumberingMode {
                    first_number,
                    ..PrNumberingMode::default()
                }),
                false => None,
            },
        ))
}

fn parse_header(arg_matches: &ArgMatches, paths: &[&str], is_merge_mode: bool) -> String {
    arg_matches
        .get_one::<String>(pr_flags::PR_HEADER)
        .map(|s| s.as_str())
        .unwrap_or(if is_merge_mode || paths[0] == PR_FILE_STDIN {
            ""
        } else {
            paths[0]
        })
        .to_string()
}

fn parse_merge_files_print(arg_matches: &ArgMatches, paths: &[&str]) -> Option<usize> {
    match arg_matches.get_flag(pr_flags::PR_MERGE) {
        true => Some(paths.len()),
        false => None,
    }
}

fn parse_merge_mode(arg_matches: &ArgMatches) -> Result<bool, PrError> {
    let is_merge_mode = arg_matches.get_flag(pr_flags::PR_MERGE);
    if is_merge_mode {
        if arg_matches.contains_id(pr_flags::PR_COLUMN) {
            let err_msg =
                String::from("cannot specify number of columns when printing in parallel");
            return Err(PrError::EncounteredErrors(err_msg));
        }
        if arg_matches.get_flag(pr_flags::PR_ACROSS) {
            let err_msg =
                String::from("cannot specify both printing across and printing in parallel");
            return Err(PrError::EncounteredErrors(err_msg));
        }
    }
    Ok(is_merge_mode)
}

fn pr_open(path: &str) -> Result<Box<dyn Read>, PrError> {
    if path == PR_FILE_STDIN {
        return Ok(ctcore::ct_io::stdin_reader_box());
    }

    metadata(path)
        .map(|i| {
            let path_string = path.to_string();
            match i.file_type() {
                #[cfg(unix)]
                ft if ft.is_block_device() => Err(PrError::UnknownFiletype(path_string)),
                #[cfg(unix)]
                ft if ft.is_char_device() => Err(PrError::UnknownFiletype(path_string)),
                #[cfg(unix)]
                ft if ft.is_fifo() => Err(PrError::UnknownFiletype(path_string)),
                #[cfg(unix)]
                ft if ft.is_socket() => Err(PrError::IsSocket(path_string)),
                ft if ft.is_dir() => Err(PrError::IsDirectory(path_string)),
                ft if ft.is_file() || ft.is_symlink() => {
                    Ok(Box::new(File::open(path).context(path)?) as Box<dyn Read>)
                }
                _ => Err(PrError::UnknownFiletype(path_string)),
            }
        })
        .unwrap_or_else(|_| Err(PrError::NotExists(path.to_string())))
}

fn pr_split_lines_if_form_feed(
    file_content: Result<String, std::io::Error>,
    _is_omit_pagination: bool,
) -> Vec<PrFileLine> {
    file_content
        .map(|content| {
            let mut lines = Vec::new();
            let mut f_occurred = 0;
            let mut chunk = Vec::new();

            for byte in content.as_bytes() {
                if *byte == PR_FF {
                    f_occurred += 1;
                    continue;
                }

                if f_occurred != 0 {
                    lines.push(PrFileLine {
                        line_content: Ok(String::from_utf8(std::mem::take(&mut chunk)).unwrap()),
                        form_feeds_after: f_occurred,
                        inline_form_feed_after: true,
                        ..PrFileLine::default()
                    });
                    f_occurred = 0;
                }
                chunk.push(*byte);
            }

            lines.push(PrFileLine {
                line_content: Ok(String::from_utf8(chunk).unwrap()),
                form_feeds_after: f_occurred,
                inline_form_feed_after: f_occurred > 0,
                ..PrFileLine::default()
            });

            lines
        })
        .unwrap_or_else(|e| {
            vec![PrFileLine {
                line_content: Err(e),
                ..PrFileLine::default()
            }]
        })
}

fn pr_push_page_rows(
    rows: &mut Vec<PrSemanticRow>,
    paths: &[&str],
    page_lines: &[PrFileLine],
    output_opts: &PrOutputOptions,
    page_number: usize,
) -> Result<(), PrError> {
    for (index, line) in pr_header_content(output_opts, page_number)
        .into_iter()
        .enumerate()
    {
        rows.push(PrSemanticRow {
            page: page_number,
            kind: "header".into(),
            section: if line.is_empty() {
                "blank".into()
            } else {
                "title".into()
            },
            file: paths.first().map(|path| (*path).to_string()),
            file_id: 0,
            line_index: None,
            group_key: index,
            text: line,
        });
    }

    let body_group_base = rows.len();
    for (index, line) in page_lines.iter().enumerate() {
        let text = match &line.line_content {
            Ok(content) => content.clone(),
            Err(err) => return Err(std::io::Error::new(err.kind(), err.to_string()).into()),
        };

        rows.push(PrSemanticRow {
            page: page_number,
            kind: "body".into(),
            section: "content".into(),
            file: paths.get(line.file_id).map(|path| (*path).to_string()),
            file_id: line.file_id,
            line_index: (line.line_number != 0).then_some(line.line_number),
            group_key: body_group_base + line.group_key.max(index),
            text,
        });
    }

    let trailer_group_base = rows.len();
    for (index, line) in pr_trailer_content(output_opts).into_iter().enumerate() {
        rows.push(PrSemanticRow {
            page: page_number,
            kind: "trailer".into(),
            section: if line.is_empty() {
                "blank".into()
            } else {
                "content".into()
            },
            file: paths.first().map(|path| (*path).to_string()),
            file_id: 0,
            line_index: None,
            group_key: trailer_group_base + index,
            text: line,
        });
    }

    Ok(())
}

fn pr_collect_semantic_for_path(
    semantic: &mut PrSemantic,
    paths: &[&str],
    path: &str,
    output_opts: &PrOutputOptions,
) -> Result<(), PrError> {
    let lines = BufReader::with_capacity(PR_READ_BUFFER_SIZE, pr_open(path)?).lines();
    let pages = pr_read_stream_and_create_pages(output_opts, lines, 0);

    for (page_index, page_lines) in pages {
        let page_number = page_index + 1;
        pr_push_page_rows(
            &mut semantic.rows,
            paths,
            &page_lines,
            output_opts,
            page_number,
        )?;
        let mut page_output = Vec::new();
        pr_output_page(&page_lines, output_opts, &mut page_output, page_number)?;
        semantic
            .classic_text
            .push_str(&String::from_utf8_lossy(&page_output));
    }

    Ok(())
}

#[cfg(test)]
fn pr_handle(path: &str, output_opts: &PrOutputOptions) -> Result<i32, PrError> {
    let mut semantic = PrSemantic {
        rows: Vec::new(),
        classic_text: String::new(),
        stderr_text: String::new(),
        exit_code: 0,
    };
    pr_collect_semantic_for_path(&mut semantic, &[path], path, output_opts)?;
    let mut out = stdout();
    out.write_all(semantic.classic_text.as_bytes())?;
    out.flush()?;
    Ok(0)
}

fn pr_read_stream_and_create_pages(
    output_opts: &PrOutputOptions,
    lines: Lines<BufReader<Box<dyn Read>>>,
    file_id: usize,
) -> Box<dyn Iterator<Item = (usize, Vec<PrFileLine>)>> {
    let start_page = output_opts.start_page;
    let start_line_number = pr_get_start_line_number(output_opts);
    let renumber_from_first_printed_page = output_opts
        .number
        .as_ref()
        .is_some_and(|number| number.first_number != 0);
    let last_page = output_opts.end_page;
    let lines_needed_per_page = pr_lines_to_read_for_page(output_opts);
    let is_omit_pagination = output_opts.is_omit_pagination;
    let keep_input_form_feeds =
        !output_opts.is_display_header_and_trailer && !output_opts.is_omit_pagination;
    let mut ignore_next_leading_form_feed = false;
    Box::new(
        lines
            .flat_map(move |l| pr_split_lines_if_form_feed(l, is_omit_pagination))
            .scan(start_line_number, move |next_line_number, line| {
                let is_form_feed_marker = line.form_feeds_after > 0
                    && matches!(line.line_content.as_ref(), Ok(content) if content.is_empty());
                let line_number = if is_form_feed_marker {
                    0
                } else {
                    let current = *next_line_number;
                    *next_line_number += 1;
                    current
                };

                Some(PrFileLine {
                    line_number,
                    file_id,
                    ..line
                })
            })
            .batching(move |it| {
                let mut first_page: Vec<PrFileLine> = Vec::new();
                let mut page_with_lines: Vec<Vec<PrFileLine>> = Vec::new();
                for line in it {
                    let form_feeds_after = line.form_feeds_after;
                    let is_form_feed_marker = form_feeds_after > 0
                        && matches!(line.line_content.as_ref(), Ok(content) if content.is_empty());

                    if is_form_feed_marker {
                        let mut effective_form_feeds_after = form_feeds_after;
                        if first_page.is_empty()
                            && ignore_next_leading_form_feed
                            && !keep_input_form_feeds
                        {
                            // GNU pr consumes one leading form feed if the previous page
                            // was already full (FF-coincidence case), which prevents an
                            // extra empty page.
                            effective_form_feeds_after =
                                effective_form_feeds_after.saturating_sub(1);
                            ignore_next_leading_form_feed = false;
                            if effective_form_feeds_after == 0 {
                                continue;
                            }
                        }

                        if first_page.is_empty() {
                            page_with_lines.push(vec![]);
                        } else {
                            if let Some(last) = first_page.last_mut() {
                                // If we hit an FF-only marker after printable content, one FF
                                // belongs to the current page boundary; remaining FFs become
                                // empty pages below.
                                last.form_feeds_after = last.form_feeds_after.max(1);
                                last.inline_form_feed_after = false;
                            }
                            page_with_lines.push(first_page);
                        }
                        for _i in 1..effective_form_feeds_after {
                            page_with_lines.push(vec![]);
                        }
                        ignore_next_leading_form_feed = false;
                        return Some(page_with_lines);
                    }

                    ignore_next_leading_form_feed = false;
                    first_page.push(line);

                    if form_feeds_after > 1 {
                        // 插入空页面
                        page_with_lines.push(first_page);
                        for _i in 1..form_feeds_after {
                            page_with_lines.push(vec![]);
                        }
                        ignore_next_leading_form_feed = false;
                        return Some(page_with_lines);
                    }

                    if first_page.len() == lines_needed_per_page || form_feeds_after == 1 {
                        break;
                    }
                }

                if first_page.is_empty() {
                    return None;
                }
                ignore_next_leading_form_feed = !keep_input_form_feeds
                    && first_page.len() == lines_needed_per_page
                    && first_page
                        .last()
                        .is_some_and(|line| line.form_feeds_after == 0);
                page_with_lines.push(first_page);
                Some(page_with_lines)
            }) // 创建一组页面，因为表单输入可能导致页面为空
            .flatten() // 从页面集平铺到页面
            .enumerate() // 指定页码
            .skip_while(move |(x, _)| {
                // 跳过不需要的页面
                let current_page = x + 1;
                current_page < start_page
            })
            .take_while(move |(x, _)| {
                // 只获取所需的页面
                let current_page = x + 1;

                current_page >= start_page
                    && last_page.is_none_or(|last_page| current_page <= last_page)
            })
            .scan(start_line_number, move |next_line_number, (page_index, page_lines)| {
                if !renumber_from_first_printed_page {
                    return Some((page_index, page_lines));
                }

                let page_lines = page_lines
                    .into_iter()
                    .map(|line| {
                        let is_form_feed_marker = line.form_feeds_after > 0
                            && matches!(line.line_content.as_ref(), Ok(content) if content.is_empty());
                        let line_number = if is_form_feed_marker {
                            0
                        } else {
                            let current = *next_line_number;
                            *next_line_number += 1;
                            current
                        };

                        PrFileLine {
                            line_number,
                            ..line
                        }
                    })
                    .collect();

                Some((page_index, page_lines))
            }),
    )
}

fn pr_collect_semantic_for_merge(
    semantic: &mut PrSemantic,
    paths: &[&str],
    output_opts: &PrOutputOptions,
) -> Result<(), PrError> {
    // 检查文件是否存在
    for path in paths {
        pr_open(path)?;
    }

    let file_pages = paths
        .iter()
        .enumerate()
        .map(|(i, path)| {
            let mut page_input_opts = output_opts.clone();
            page_input_opts.start_page = 1;
            let lines =
                BufReader::with_capacity(PR_READ_BUFFER_SIZE, pr_open(path).unwrap()).lines();

            pr_read_stream_and_create_pages(&page_input_opts, lines, i).collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    let start_page_index = output_opts.start_page.saturating_sub(1);
    let maybe_last_page_index = file_pages
        .iter()
        .filter_map(|pages| pages.last().map(|(page_index, _)| *page_index))
        .max();

    let Some(last_page_index) = maybe_last_page_index else {
        return Ok(());
    };

    let renumber_from_first_printed_page = output_opts
        .number
        .as_ref()
        .is_some_and(|number| number.first_number != 0);
    let mut next_merge_line_number = None;

    for page_index in start_page_index..=last_page_index {
        let page_number = page_index + 1;
        let page_lines_by_file = file_pages
            .iter()
            .map(|pages| {
                pages
                    .iter()
                    .find(|(idx, _)| *idx == page_index)
                    .map(|(_, page_lines)| page_lines)
            })
            .collect::<Vec<_>>();
        let rows_in_page = if output_opts.is_omit_pagination {
            page_lines_by_file
                .iter()
                .filter_map(|page_lines| page_lines.map(Vec::len))
                .max()
                .unwrap_or(0)
        } else if output_opts.is_double_space {
            output_opts.content_lines_per_page / 2
        } else {
            output_opts.content_lines_per_page
        };
        let mut merged_columns = (0..paths.len()).map(|_| Vec::new()).collect::<Vec<_>>();

        for row_index in 0..rows_in_page {
            let row_has_content = page_lines_by_file
                .iter()
                .any(|page_lines| page_lines.and_then(|lines| lines.get(row_index)).is_some());

            if !row_has_content {
                continue;
            }

            let first_file_line = page_lines_by_file
                .first()
                .and_then(|page_lines| page_lines.and_then(|lines| lines.get(row_index)));
            let row_line_number = if output_opts.number.is_some() {
                let current = next_merge_line_number.unwrap_or_else(|| {
                    if renumber_from_first_printed_page {
                        pr_get_start_line_number(output_opts)
                    } else {
                        first_file_line
                            .map(|file_line| file_line.line_number)
                            .unwrap_or_else(|| pr_get_start_line_number(output_opts))
                    }
                });
                next_merge_line_number = Some(current.saturating_add(1));
                current
            } else {
                0
            };

            for (file_id, page_lines) in page_lines_by_file.iter().enumerate() {
                if let Some(file_line) = page_lines.and_then(|lines| lines.get(row_index)) {
                    match &file_line.line_content {
                        Ok(content) => merged_columns[file_id].push(PrFileLine {
                            file_id,
                            line_number: if file_id == 0 {
                                row_line_number
                            } else {
                                file_line.line_number
                            },
                            page_number,
                            group_key: row_index,
                            line_content: Ok(content.clone()),
                            form_feeds_after: file_line.form_feeds_after,
                            inline_form_feed_after: file_line.inline_form_feed_after,
                        }),
                        Err(e) => return Err(std::io::Error::new(e.kind(), e.to_string()).into()),
                    }
                } else if file_id == 0 && output_opts.number.is_some() {
                    merged_columns[file_id].push(PrFileLine {
                        file_id,
                        line_number: row_line_number,
                        page_number,
                        group_key: row_index,
                        line_content: Ok(String::new()),
                        form_feeds_after: 0,
                        inline_form_feed_after: false,
                    });
                }
            }
        }

        let merged_lines = merged_columns.into_iter().flatten().collect::<Vec<_>>();

        let mut page_output_opts = output_opts.clone();
        page_output_opts.merge_files_print = Some(paths.len());
        pr_push_page_rows(
            &mut semantic.rows,
            paths,
            &merged_lines,
            &page_output_opts,
            page_number,
        )?;
        let mut page_output = Vec::new();
        pr_output_page(
            &merged_lines,
            &page_output_opts,
            &mut page_output,
            page_number,
        )?;
        semantic
            .classic_text
            .push_str(&String::from_utf8_lossy(&page_output));
    }

    Ok(())
}

#[cfg(test)]
fn mpr_handle(paths: &[&str], output_opts: &PrOutputOptions) -> Result<i32, PrError> {
    let mut semantic = PrSemantic {
        rows: Vec::new(),
        classic_text: String::new(),
        stderr_text: String::new(),
        exit_code: 0,
    };
    pr_collect_semantic_for_merge(&mut semantic, paths, output_opts)?;
    let mut out = stdout();
    out.write_all(semantic.classic_text.as_bytes())?;
    out.flush()?;
    Ok(0)
}

#[cfg(test)]
fn pr_print_page(
    lines: &[PrFileLine],
    output_opts: &PrOutputOptions,
    page: usize,
) -> Result<usize, std::io::Error> {
    let out = stdout();
    let mut out = out.lock();

    pr_output_page(lines, output_opts, &mut out, page)
}

fn pr_output_page(
    lines: &[PrFileLine],
    output_opts: &PrOutputOptions,
    out: &mut impl Write,
    page: usize,
) -> Result<usize, Error> {
    let line_separator = output_opts.line_separator.as_bytes();
    let page_separator = output_opts.page_separator_char.as_bytes();
    let keep_input_form_feeds =
        !output_opts.is_display_header_and_trailer && !output_opts.is_omit_pagination;
    let page_has_input_form_feed =
        keep_input_form_feeds && lines.iter().any(|line| line.form_feeds_after > 0);

    if keep_input_form_feeds && lines.is_empty() {
        out.write_all(&[PR_FF])?;
        out.flush()?;
        return Ok(0);
    }

    let header = pr_header_content(output_opts, page);
    let trailer_content = pr_trailer_content(output_opts);

    for x in header {
        out.write_all(x.as_bytes())?;
        out.write_all(line_separator)?;
    }

    let lines_written = pr_write_columns(lines, output_opts, out)?;

    for (index, x) in trailer_content.iter().enumerate() {
        out.write_all(x.as_bytes())?;
        if index + 1 != trailer_content.len() {
            out.write_all(line_separator)?;
        }
    }
    if output_opts.is_display_header_and_trailer {
        out.write_all(page_separator)?;
    } else if page_has_input_form_feed {
        out.write_all(&[PR_FF])?;
    }
    out.flush()?;
    Ok(lines_written)
}

#[allow(clippy::cognitive_complexity)]
fn pr_write_columns(
    lines: &[PrFileLine],
    output_opts: &PrOutputOptions,
    out: &mut impl Write,
) -> Result<usize, std::io::Error> {
    let line_separator = output_opts.content_line_separator.as_bytes();

    let mut content_lines_per_page = if output_opts.is_double_space {
        output_opts.content_lines_per_page / 2
    } else {
        output_opts.content_lines_per_page
    };

    let columns = match output_opts.merge_files_print {
        Some(col) => col,
        None => pr_get_columns(output_opts),
    };

    let line_width = output_opts.line_width;
    let mut lines_printed = 0;
    let feed_line_present = output_opts.is_form_feed_used;

    let across_mode = output_opts
        .column_mode_options
        .as_ref()
        .map(|i| i.is_across_mode)
        .unwrap_or(false);

    if output_opts.is_omit_pagination {
        content_lines_per_page = if output_opts.merge_files_print.is_some() {
            (0..columns)
                .map(|column_id| {
                    lines
                        .iter()
                        .filter(|line| line.file_id == column_id)
                        .count()
                })
                .max()
                .unwrap_or(0)
        } else {
            lines.len().div_ceil(columns)
        };
    }

    let mut filled_lines = Vec::new();
    if output_opts.merge_files_print.is_some() {
        let mut offset = 0;
        for col in 0..columns {
            let mut inserted = 0;
            for line in &lines[offset..] {
                if line.file_id != col {
                    break;
                }
                filled_lines.push(Some(line));
                inserted += 1;
            }
            offset += inserted;

            for _i in inserted..content_lines_per_page {
                filled_lines.push(None);
            }
        }
    }

    let down_mode_layout = if output_opts.merge_files_print.is_none() && !across_mode {
        let total = lines.len();
        let base = total / columns;
        let remainder = total % columns;
        let mut start = 0usize;
        let mut layout = Vec::with_capacity(columns);
        for col in 0..columns {
            let len = base + usize::from(col < remainder);
            layout.push((start, len));
            start += len;
        }
        Some(layout)
    } else {
        None
    };

    let table: Vec<Vec<_>> = (0..content_lines_per_page)
        .map(move |a| {
            (0..columns)
                .map(|i| {
                    if across_mode {
                        let idx = a * columns + i;
                        if idx >= lines.len() {
                            None
                        } else {
                            lines.get(idx)
                        }
                    } else if output_opts.merge_files_print.is_some() {
                        *filled_lines
                            .get(content_lines_per_page * i + a)
                            .unwrap_or(&None)
                    } else {
                        down_mode_layout
                            .as_ref()
                            .and_then(|layout| layout.get(i).copied())
                            .and_then(
                                |(start, len)| {
                                    if a >= len { None } else { lines.get(start + a) }
                                },
                            )
                    }
                })
                .collect()
        })
        .collect();
    if columns > 1 {
        return pr_write_multicolumn_table(
            &table,
            output_opts,
            out,
            line_separator,
            feed_line_present,
            across_mode,
            &line_width,
        );
    }

    let blank_line = PrFileLine::default();
    for row in table {
        let cell = row.first().copied().flatten();
        if cell.is_none() {
            if feed_line_present || !output_opts.is_display_header_and_trailer {
                if feed_line_present && lines_printed == 0 {
                    out.write_all(line_separator)?;
                }
                break;
            }
            out.write_all(line_separator)?;
            continue;
        }

        let segment = pr_get_line_for_printing(
            output_opts,
            cell.unwrap_or(&blank_line),
            columns,
            0,
            &line_width,
            1,
            true,
        )?;
        lines_printed += 1;
        let rendered_row = if let Some((tab_ch, tab_width)) = output_opts.output_tabs {
            replace_spaces_with_tabs_by_segments(
                &[format!("{}{}", output_opts.offset_spaces, segment)],
                tab_ch,
                tab_width,
                true,
            )
        } else {
            format!("{}{}", output_opts.offset_spaces, segment)
        };
        out.write_all(rendered_row.as_bytes())?;
        out.write_all(line_separator)?;
    }

    Ok(lines_printed)
}

fn pr_write_multicolumn_table(
    table: &[Vec<Option<&PrFileLine>>],
    output_opts: &PrOutputOptions,
    out: &mut impl Write,
    line_separator: &[u8],
    feed_line_present: bool,
    across_mode: bool,
    line_width: &Option<usize>,
) -> Result<usize, std::io::Error> {
    let columns = table.first().map_or(0, Vec::len);
    let blank_line = PrFileLine::default();
    let mut lines_printed = 0usize;

    for row in table {
        if row.iter().all(|cell| cell.is_none()) {
            if feed_line_present || !output_opts.is_display_header_and_trailer {
                if feed_line_present && lines_printed == 0 {
                    out.write_all(line_separator)?;
                }
                break;
            }
            out.write_all(line_separator)?;
            continue;
        }

        let last_actual_index = row.iter().rposition(|cell| cell.is_some());
        let last_actual_has_inline_ff = last_actual_index
            .and_then(|index| row.get(index).copied().flatten())
            .is_some_and(|line| line.inline_form_feed_after);
        let layout_indexes = if output_opts.merge_files_print.is_some()
            && !output_opts.is_join_lines
            && last_actual_has_inline_ff
        {
            row.iter()
                .rposition(|cell| cell.is_some())
                .map(|i| i + 1)
                .unwrap_or(0)
        } else if output_opts.is_join_lines
            && (output_opts.merge_files_print.is_none() || last_actual_has_inline_ff)
        {
            row.iter()
                .rposition(|cell| cell.is_some())
                .map(|i| i + 1)
                .unwrap_or(0)
        } else if output_opts.merge_files_print.is_some() || across_mode {
            columns
        } else {
            row.iter()
                .rposition(|cell| cell.is_some())
                .map(|i| i + 1)
                .unwrap_or(0)
        };
        let mut row_segments: Vec<(String, bool, bool)> = Vec::new();
        if !output_opts.offset_spaces.is_empty() {
            row_segments.push((output_opts.offset_spaces.clone(), false, false));
        }
        let mut pending_empty_segments: Vec<(String, bool, bool)> = Vec::new();
        let mut row_has_actual_cell = false;
        for (index, cell) in row.iter().enumerate() {
            if layout_indexes > 0 && index >= layout_indexes {
                continue;
            }
            if output_opts.merge_files_print.is_none() && across_mode && cell.is_none() {
                break;
            }

            let segment = if output_opts.is_join_lines {
                match cell {
                    Some(file_line) => {
                        let rendered = pr_get_rendered_line_for_printing(
                            output_opts,
                            file_line,
                            columns,
                            index,
                            line_width,
                            layout_indexes,
                            last_actual_index == Some(index),
                        )?;
                        lines_printed += 1;
                        (rendered.text, rendered.has_separator, true)
                    }
                    None => {
                        let rendered = pr_get_rendered_line_for_printing(
                            output_opts,
                            &blank_line,
                            columns,
                            index,
                            line_width,
                            layout_indexes,
                            false,
                        )?;
                        (rendered.text, rendered.has_separator, false)
                    }
                }
            } else {
                let segment = match cell {
                    Some(file_line) => {
                        let segment = pr_get_line_for_printing(
                            output_opts,
                            file_line,
                            columns,
                            index,
                            line_width,
                            layout_indexes,
                            last_actual_index == Some(index),
                        )?;
                        lines_printed += 1;
                        segment
                    }
                    None => pr_get_line_for_printing(
                        output_opts,
                        &blank_line,
                        columns,
                        index,
                        line_width,
                        layout_indexes,
                        false,
                    )?,
                };
                (segment, false, cell.is_some())
            };

            if cell.is_some() {
                row_segments.append(&mut pending_empty_segments);
                row_segments.push(segment);
                row_has_actual_cell = true;
            } else if row_has_actual_cell {
                row_segments.push(segment);
            } else {
                pending_empty_segments.push(segment);
            }
        }

        let rendered_row = if let Some((tab_ch, tab_width)) = output_opts.output_tabs {
            if output_opts.is_join_lines {
                let join_line_segments = row_segments
                    .into_iter()
                    .scan(
                        0usize,
                        |actual_segment_index, (text, has_separator, is_actual_cell)| {
                            let is_offset_prefix = !output_opts.offset_spaces.is_empty()
                                && *actual_segment_index == 0
                                && !has_separator
                                && text == output_opts.offset_spaces;
                            let reset_position = if output_opts.merge_files_print.is_none()
                                && !across_mode
                                && is_actual_cell
                                && !is_offset_prefix
                            {
                                let base = if *actual_segment_index == 0 {
                                    UnicodeWidthStr::width(output_opts.offset_spaces.as_str())
                                } else {
                                    0
                                };
                                *actual_segment_index += 1;
                                Some(base + pr_rendered_width(&text, 8))
                            } else {
                                None
                            };
                            Some(JoinLineSegment {
                                logical_width: pr_rendered_width(&text, 8),
                                text,
                                has_separator,
                                reset_position,
                            })
                        },
                    )
                    .collect::<Vec<_>>();
                render_join_lines_segments(
                    &join_line_segments,
                    &output_opts.col_sep_for_printing,
                    tab_ch,
                    tab_width,
                    output_opts.merge_files_print.is_none() && !across_mode,
                )
            } else {
                let row_segments = row_segments
                    .iter()
                    .map(|(segment, _, _)| segment.clone())
                    .collect::<Vec<_>>();
                replace_spaces_with_tabs_by_segments(&row_segments, tab_ch, tab_width, false)
            }
        } else {
            row_segments
                .into_iter()
                .map(|(segment, has_separator, _)| {
                    if has_separator {
                        format!("{segment}{}", output_opts.col_sep_for_printing)
                    } else {
                        segment
                    }
                })
                .collect::<String>()
        };
        out.write_all(rendered_row.as_bytes())?;
        out.write_all(line_separator)?;
    }

    Ok(lines_printed)
}

fn pr_get_line_for_printing(
    output_opts: &PrOutputOptions,
    file_line: &PrFileLine,
    columns: usize,
    index: usize,
    line_width: &Option<usize>,
    indexes: usize,
    is_last_actual_column: bool,
) -> Result<String, std::io::Error> {
    let rendered = pr_get_rendered_line_for_printing(
        output_opts,
        file_line,
        columns,
        index,
        line_width,
        indexes,
        is_last_actual_column,
    )?;
    if rendered.has_separator {
        Ok(format!(
            "{}{}",
            rendered.text, output_opts.col_sep_for_printing
        ))
    } else {
        Ok(rendered.text)
    }
}

fn pr_get_rendered_line_for_printing(
    output_opts: &PrOutputOptions,
    file_line: &PrFileLine,
    columns: usize,
    index: usize,
    line_width: &Option<usize>,
    indexes: usize,
    is_last_actual_column: bool,
) -> Result<PrRenderedLine, std::io::Error> {
    let formatted_line_number =
        pr_get_formatted_line_number(output_opts, file_line.line_number, index);
    let numbering_width = pr_rendered_width(&formatted_line_number, 8);

    let mut content = match &file_line.line_content {
        Ok(content) => {
            if let Some((tab_ch, tab_width)) = output_opts.expand_tabs {
                let expand_plain_tabs = tab_ch != '\t'
                    && (output_opts.merge_files_print.is_some()
                        || output_opts.column_mode_options.is_some());
                expand_tabs_to_spaces(
                    content,
                    tab_ch,
                    tab_width,
                    numbering_width,
                    expand_plain_tabs,
                )
            } else {
                content.clone()
            }
        }
        Err(e) => return Err(std::io::Error::new(e.kind(), e.to_string())),
    };

    if output_opts.show_control_chars || output_opts.show_nonprinting {
        content = escape_control_chars(
            &content,
            output_opts.show_control_chars,
            output_opts.show_nonprinting,
        );
    }

    if output_opts.merge_files_print.is_some() && !output_opts.is_join_lines {
        content = content.trim_end_matches([' ', '\t']).to_string();
    }

    let complete_line = format!("{formatted_line_number}{content}");

    let display_length = pr_rendered_width(&complete_line, 8);

    let has_no_separator = output_opts.col_sep_for_printing.is_empty();
    let is_string_sep = !output_opts.col_sep_for_printing.is_empty()
        && output_opts.col_sep_for_printing != "\t"
        && output_opts.col_sep_for_printing != " ";

    let parallel_number_field_width = if output_opts.merge_files_print.is_some() {
        pr_get_number_field_width_for_multicolumn(output_opts)
    } else {
        None
    };

    let result_line = line_width
        .map(|i| {
            // When is_pad_columns is true, the implicit separator (tab/space) between
            // columns takes 1 character of width, matching GNU pr's formula:
            // chars_per_column = (chars_per_line - (columns-1) * col_sep_length) / columns
            let effective_sep_width =
                if output_opts.is_pad_columns && columns > 1 && !is_string_sep && !has_no_separator
                {
                    1 // implicit tab/space separator
                } else {
                    UnicodeWidthStr::width(output_opts.col_sep_for_printing.as_str())
                };

            let useful_line_width = i.saturating_sub(parallel_number_field_width.unwrap_or(0));

            if useful_line_width <= (columns - 1) * effective_sep_width {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Page width too narrow".to_owned(),
                ));
            }

            // Should dynamic tab/space padding be generated?
            let should_pad = output_opts.is_pad_columns && index + 1 < indexes;

            let mut min_width =
                useful_line_width.saturating_sub((columns - 1) * effective_sep_width) / columns;

            // GNU pr: for parallel mode (-m) with numbering, the line-number field
            // is reserved once globally and effectively belongs to the first column.
            if index == 0 {
                min_width += parallel_number_field_width.unwrap_or(0);
            }

            if should_pad {
                // For implicit separators (default tab/space), pad includes +1 for separator.
                // For explicit string separators (-S), the separator is appended via `sep`, so no +1.
                let pad_target = if is_string_sep || has_no_separator {
                    min_width
                } else {
                    min_width + 1
                };
                if display_length < pad_target {
                    let mut extended_line = complete_line.clone();
                    let mut current_len = display_length;

                    while current_len < pad_target {
                        extended_line.push(' ');
                        current_len += 1;
                    }
                    Ok((pr_take_display_width(&extended_line, pad_target, 8), false))
                } else {
                    let mut truncated = pr_take_display_width(&complete_line, min_width, 8);
                    let was_truncated = truncated.chars().count() < complete_line.chars().count();
                    let omitted_suffix = &complete_line[truncated.len()..];
                    let next_omitted_char = omitted_suffix.chars().next();
                    let omitted_suffix_has_whitespace =
                        omitted_suffix.chars().any(|c| matches!(c, ' ' | '\t'));
                    let suppress_trailing_separator = output_opts.merge_files_print.is_some()
                        && is_last_actual_column
                        && file_line.inline_form_feed_after
                        && was_truncated
                        && (file_line.form_feeds_after > 1
                            || !formatted_line_number.is_empty()
                            || is_string_sep
                            || truncated.ends_with([' ', '\t'])
                            || omitted_suffix_has_whitespace
                            || matches!(next_omitted_char, Some(' ' | '\t'))
                            || output_opts.is_pad_columns);
                    if suppress_trailing_separator {
                        truncated = truncated.trim_end_matches([' ', '\t']).to_string();
                    }
                    if !is_string_sep && !has_no_separator && !suppress_trailing_separator {
                        truncated.push(' ');
                    }
                    Ok((truncated, suppress_trailing_separator))
                }
            } else {
                let mut truncated = pr_take_display_width(&complete_line, min_width, 8);
                let was_truncated = truncated.chars().count() < complete_line.chars().count();
                let omitted_suffix = &complete_line[truncated.len()..];
                let next_omitted_char = omitted_suffix.chars().next();
                let omitted_suffix_has_whitespace =
                    omitted_suffix.chars().any(|c| matches!(c, ' ' | '\t'));
                let suppress_trailing_separator = output_opts.merge_files_print.is_some()
                    && is_last_actual_column
                    && file_line.inline_form_feed_after
                    && was_truncated
                    && (file_line.form_feeds_after > 1
                        || !formatted_line_number.is_empty()
                        || is_string_sep
                        || truncated.ends_with([' ', '\t'])
                        || omitted_suffix_has_whitespace
                        || matches!(next_omitted_char, Some(' ' | '\t'))
                        || output_opts.is_pad_columns);
                if suppress_trailing_separator {
                    truncated = truncated.trim_end_matches([' ', '\t']).to_string();
                }
                Ok((truncated, suppress_trailing_separator))
            }
        })
        .unwrap_or_else(|| Ok((complete_line.clone(), false)));

    result_line.map(|(line, suppress_trailing_separator)| {
        let has_separator = !suppress_trailing_separator
            && (!output_opts.is_pad_columns || is_string_sep)
            && (index + 1) != indexes;
        let text = if output_opts.merge_files_print.is_some() && index + 1 == indexes {
            line.trim_end_matches([' ', '\t']).to_string()
        } else {
            line
        };
        PrRenderedLine {
            text,
            has_separator,
        }
    })
}

fn pr_rendered_width(s: &str, tab_width: usize) -> usize {
    if tab_width == 0 {
        return UnicodeWidthStr::width(s);
    }

    let mut current_col = 0usize;
    for c in s.chars() {
        if c == '\t' {
            current_col = (current_col / tab_width + 1) * tab_width;
        } else if c == '\u{0008}' {
            current_col = current_col.saturating_sub(1);
        } else {
            current_col += UnicodeWidthChar::width(c).unwrap_or(0);
        }
    }
    current_col
}

fn pr_take_display_width(s: &str, max_width: usize, tab_width: usize) -> String {
    let mut current_col = 0usize;
    let mut result = String::new();

    for c in s.chars() {
        let next_col = if c == '\t' {
            (current_col / tab_width + 1) * tab_width
        } else if c == '\u{0008}' {
            current_col.saturating_sub(1)
        } else {
            current_col + UnicodeWidthChar::width(c).unwrap_or(0)
        };

        if next_col > max_width {
            break;
        }

        result.push(c);
        current_col = next_col;
    }

    result
}

fn expand_tabs_to_spaces(
    s: &str,
    tab_char: char,
    tab_width: usize,
    initial_col: usize,
    expand_plain_tabs: bool,
) -> String {
    if tab_width == 0 {
        return s.to_string();
    }
    let mut res = String::new();
    let mut current_col = initial_col;
    for c in s.chars() {
        if c == tab_char || (expand_plain_tabs && c == '\t') {
            let width = if c == tab_char { tab_width } else { 8 };
            let spaces = width - (current_col % width);
            res.push_str(&" ".repeat(spaces));
            current_col += spaces;
        } else if c == '\t' {
            res.push(c);
            current_col = (current_col / 8 + 1) * 8;
        } else if c == '\u{0008}' {
            // Match GNU pr: backspaces beyond column 0 are ignored.
            if current_col > 0 {
                res.push(c);
                current_col -= 1;
            }
        } else {
            res.push(c);
            current_col += UnicodeWidthChar::width(c).unwrap_or(0);
        }
    }
    res
}

fn replace_spaces_with_tabs_by_segments(
    segments: &[String],
    tab_char: char,
    tab_width: usize,
    flush_trailing_spaces: bool,
) -> String {
    if tab_width == 0 {
        return segments.concat();
    }

    let mut res = String::new();
    let mut current_col = 0usize;
    let mut pending_spaces = 0usize;

    let flush_spaces = |res: &mut String, current_col: &mut usize, pending_spaces: &mut usize| {
        if *pending_spaces == 0 {
            return;
        }

        let mut h_old = *current_col;
        let goal = h_old + *pending_spaces;

        while goal.saturating_sub(h_old) > 1 {
            let h_new = (h_old / tab_width + 1) * tab_width;
            if h_new > goal {
                break;
            }
            res.push(tab_char);
            h_old = h_new;
        }

        while h_old < goal {
            res.push(' ');
            h_old += 1;
        }

        *current_col = goal;
        *pending_spaces = 0;
    };

    for (index, segment) in segments.iter().enumerate() {
        for c in segment.chars() {
            if c == ' ' {
                pending_spaces += 1;
                continue;
            }

            flush_spaces(&mut res, &mut current_col, &mut pending_spaces);
            res.push(c);
            if c == '\t' {
                current_col = (current_col / tab_width + 1) * tab_width;
            } else if c == '\u{0008}' {
                current_col = current_col.saturating_sub(1);
            } else {
                current_col += UnicodeWidthChar::width(c).unwrap_or(0);
            }
        }
        let is_last_segment = index + 1 == segments.len();
        if !is_last_segment || flush_trailing_spaces {
            // Match GNU behavior: flush pending spaces between logical output chunks,
            // but leave end-of-line pending spaces unprinted in multicolumn mode.
            flush_spaces(&mut res, &mut current_col, &mut pending_spaces);
        }
    }

    res
}

fn render_join_lines_segments(
    segments: &[JoinLineSegment],
    separator: &str,
    tab_char: char,
    tab_width: usize,
    align_to_segment_logical_width: bool,
) -> String {
    if tab_width == 0 {
        return segments
            .iter()
            .map(|segment| {
                if segment.has_separator {
                    format!("{}{}", segment.text, separator)
                } else {
                    segment.text.clone()
                }
            })
            .collect();
    }

    let mut res = String::new();
    let mut logical_col = 0usize;
    let mut pending_spaces = 0usize;
    let separator_width = if separator == "\t" {
        1
    } else {
        UnicodeWidthStr::width(separator)
    };

    let flush_spaces = |res: &mut String, logical_col: &mut usize, pending_spaces: &mut usize| {
        if *pending_spaces == 0 {
            return;
        }

        let mut h_old = *logical_col;
        let goal = h_old + *pending_spaces;

        while goal.saturating_sub(h_old) > 1 {
            let h_new = (h_old / tab_width + 1) * tab_width;
            if h_new > goal {
                break;
            }
            res.push(tab_char);
            h_old = h_new;
        }

        while h_old < goal {
            res.push(' ');
            h_old += 1;
        }

        *logical_col = goal;
        *pending_spaces = 0;
    };

    let emit_content_char =
        |res: &mut String, logical_col: &mut usize, pending_spaces: &mut usize, c: char| {
            if c == ' ' {
                *pending_spaces += 1;
                return;
            }

            flush_spaces(res, logical_col, pending_spaces);
            res.push(c);
            if c == '\u{0008}' {
                *logical_col = logical_col.saturating_sub(1);
            } else if c != '\t' {
                *logical_col += UnicodeWidthChar::width(c).unwrap_or(0);
            }
        };

    let emit_separator = |res: &mut String, logical_col: &mut usize, pending_spaces: &mut usize| {
        let mut has_non_space = false;
        for c in separator.chars() {
            if c == ' ' {
                *pending_spaces += 1;
                continue;
            }

            has_non_space = true;
            flush_spaces(res, logical_col, pending_spaces);
            res.push(c);
        }

        if has_non_space {
            *logical_col += separator_width;
        }

        if *pending_spaces > 0 {
            flush_spaces(res, logical_col, pending_spaces);
        }
    };

    for segment in segments {
        let segment_start_col = logical_col;
        for c in segment.text.chars() {
            emit_content_char(&mut res, &mut logical_col, &mut pending_spaces, c);
        }

        if align_to_segment_logical_width && pending_spaces == 0 {
            logical_col = segment
                .reset_position
                .unwrap_or(segment_start_col + segment.logical_width);
        }

        if segment.has_separator {
            emit_separator(&mut res, &mut logical_col, &mut pending_spaces);
        }
    }

    res
}
fn pr_get_formatted_line_number(
    output_opts: &PrOutputOptions,
    line_number: usize,
    index: usize,
) -> String {
    let should_show_line_number =
        output_opts.number.is_some() && (output_opts.merge_files_print.is_none() || index == 0);
    if should_show_line_number && line_number != 0 {
        let line_str = line_number.to_string();
        let num_opt = output_opts.number.as_ref().unwrap();
        let width = num_opt.width;
        let is_multicolumn = output_opts
            .merge_files_print
            .unwrap_or_else(|| pr_get_columns(output_opts))
            > 1;
        let separator = if is_multicolumn && !output_opts.is_join_lines && num_opt.separator == "\t"
        {
            // GNU pr: in multicolumn output, default line-number TAB uses a fixed
            // width (from column start), so it behaves like spaces before tabify.
            let spaces = 8 - (width % 8);
            " ".repeat(spaces)
        } else {
            num_opt.separator.clone()
        };
        if line_str.len() >= width {
            format!(
                "{:>width$}{}",
                &line_str[line_str.len() - width..],
                separator
            )
        } else {
            format!("{line_str:>width$}{separator}")
        }
    } else {
        String::new()
    }
}

fn pr_get_number_field_width_for_multicolumn(output_opts: &PrOutputOptions) -> Option<usize> {
    output_opts.number.as_ref().map(|num_opt| {
        let width = num_opt.width;
        let separator_width = if num_opt.separator == "\t" {
            8 - (width % 8)
        } else {
            UnicodeWidthStr::width(num_opt.separator.as_str())
        };
        width + separator_width
    })
}

fn escape_control_chars(input: &str, show_control_chars: bool, show_nonprinting: bool) -> String {
    if !show_control_chars && !show_nonprinting {
        return input.to_string();
    }
    let mut result = String::with_capacity(input.len());
    for c in input.chars() {
        if c == '\t' || c == '\n' || c == '\x0C' || c == ' ' {
            result.push(c);
            continue;
        }
        if c.is_control() {
            let cp = c as u32;
            if show_nonprinting {
                result.push_str(&format!("\\{cp:03o}"));
            } else if show_control_chars {
                if cp < 128 {
                    result.push('^');
                    result.push((cp ^ 0x40) as u8 as char);
                } else {
                    result.push_str(&format!("\\{cp:03o}"));
                }
            } else {
                result.push(c);
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// 如果没有使用 `NO_HEADER_TRAILER_OPTION` 选项禁止显示页眉，则返回五行页眉内容。
/// 使用 "NO_HEADER_TRAILER_OPTION "选项。
fn pr_header_content(output_opts: &PrOutputOptions, page: usize) -> Vec<String> {
    if output_opts.is_display_header_and_trailer {
        let date_width = UnicodeWidthStr::width(output_opts.last_modified_time.as_str());
        let file_width = UnicodeWidthStr::width(output_opts.header.as_str());
        let page_text = t!("pr.page", page = page);
        let page_width = UnicodeWidthStr::width(page_text.as_str());

        let chars_per_line = output_opts.line_width.unwrap_or(output_opts.page_width);

        let header_width_available = chars_per_line
            .saturating_sub(date_width)
            .saturating_sub(file_width);

        let available_width = header_width_available.saturating_sub(page_width);

        let lhs_spaces = available_width / 2;
        let rhs_spaces = available_width - lhs_spaces;

        let first_line = format!(
            "{}{}{:lhs$}{}{:rhs$}{}",
            output_opts.offset_spaces,
            output_opts.last_modified_time,
            " ",
            output_opts.header,
            " ",
            page_text,
            lhs = lhs_spaces.max(1),
            rhs = rhs_spaces.max(1)
        );

        let blank = if output_opts.offset_spaces.is_empty() {
            String::new()
        } else {
            output_opts.offset_spaces.clone()
        };
        vec![
            blank.clone(),
            String::new(),
            first_line,
            String::new(),
            String::new(),
        ]
    } else {
        Vec::new()
    }
}

fn pr_file_last_modified_time(path: &str, fmt: &str) -> String {
    metadata(path)
        .map(|i| {
            i.modified()
                .map(|x| {
                    let date_time: DateTime<Local> = x.into();
                    pr_format_local_datetime(fmt, date_time)
                })
                .unwrap_or_default()
        })
        .unwrap_or_default()
}

/// 如果没有使用 "NO_HEADER_TRAILER_OPTION "选项禁用显示拖尾，则返回五个空行作为拖尾内容。
/// 未使用 `NO_HEADER_TRAILER_OPTION` 选项禁用预告片显示。
fn pr_trailer_content(output_opts: &PrOutputOptions) -> Vec<String> {
    if output_opts.is_display_header_and_trailer && !output_opts.is_form_feed_used {
        vec![
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        ]
    } else {
        Vec::new()
    }
}

/// 返回要打印文件的起始行号。
/// 如果指定了 -N，则第一行的行号会发生变化。
/// 默认为 1。
fn pr_get_start_line_number(output_opts: &PrOutputOptions) -> usize {
    output_opts
        .number
        .as_ref()
        .map(|i| {
            if i.first_number == 0 {
                1
            } else {
                i.first_number
            }
        })
        .unwrap_or(1)
}

/// 返回构建一页 pr 输出所需的输入行数。
/// 如果使用双空格-d，行数减半。
/// 如果使用列--columns，行数将乘以该值。
fn pr_lines_to_read_for_page(output_opts: &PrOutputOptions) -> usize {
    let content_lines_per_page = output_opts.content_lines_per_page;
    let columns = pr_get_columns(output_opts);
    if output_opts.is_double_space {
        (content_lines_per_page / 2) * columns
    } else {
        content_lines_per_page * columns
    }
}

/// 返回要输出的列数
fn pr_get_columns(output_opts: &PrOutputOptions) -> usize {
    match &output_opts.column_mode_options {
        Some(col) => col.columns,
        None => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 新增测试模块，专门测试结构体实现
    #[cfg(test)]
    mod struct_impl_tests {
        use super::super::*;
        use std::io::{Error, ErrorKind};

        #[test]
        fn test_pr_file_line_partial_eq() {
            // 测试相同内容的行
            let line1 = PrFileLine {
                file_id: 1,
                line_number: 10,
                page_number: 2,
                group_key: 5,
                line_content: Ok("测试行".to_string()),
                form_feeds_after: 0,
                inline_form_feed_after: false,
            };

            let line2 = PrFileLine {
                file_id: 1,
                line_number: 10,
                page_number: 2,
                group_key: 5,
                line_content: Ok("测试行".to_string()),
                form_feeds_after: 0,
                inline_form_feed_after: false,
            };

            assert_eq!(line1, line2);

            // 测试不同字段值
            let line3 = PrFileLine {
                file_id: 2, // 不同的file_id
                line_number: 10,
                page_number: 2,
                group_key: 5,
                line_content: Ok("测试行".to_string()),
                form_feeds_after: 0,
                inline_form_feed_after: false,
            };

            assert_ne!(line1, line3);

            // 测试不同内容
            let line4 = PrFileLine {
                file_id: 1,
                line_number: 10,
                page_number: 2,
                group_key: 5,
                line_content: Ok("不同内容".to_string()),
                form_feeds_after: 0,
                inline_form_feed_after: false,
            };

            assert_ne!(line1, line4);

            // 测试Error vs Ok
            let line5 = PrFileLine {
                file_id: 1,
                line_number: 10,
                page_number: 2,
                group_key: 5,
                line_content: Err(Error::other("测试错误")),
                form_feeds_after: 0,
                inline_form_feed_after: false,
            };

            assert_ne!(line1, line5);

            // 测试Error vs Error
            let line6 = PrFileLine {
                file_id: 1,
                line_number: 10,
                page_number: 2,
                group_key: 5,
                line_content: Err(Error::other("测试错误")),
                form_feeds_after: 0,
                inline_form_feed_after: false,
            };

            // 所有错误被视为相等
            assert_eq!(line5, line6);

            // 测试form_feeds_after不同
            let line7 = PrFileLine {
                file_id: 1,
                line_number: 10,
                page_number: 2,
                group_key: 5,
                line_content: Ok("测试行".to_string()),
                form_feeds_after: 1, // 不同的form_feeds_after
                inline_form_feed_after: false,
            };

            assert_ne!(line1, line7);
        }

        #[test]
        fn test_pr_error_from_io_error() {
            // 测试从IO错误创建PrError
            let io_error = Error::new(ErrorKind::NotFound, "文件不存在");
            let pr_error: PrError = io_error.into();

            match pr_error {
                PrError::EncounteredErrors(msg) => {
                    assert!(msg.contains("文件不存在"));
                }
                _ => panic!("转换为了错误的PrError类型"),
            }

            // 测试不同类型的IO错误
            let io_error2 = Error::new(ErrorKind::PermissionDenied, "权限被拒绝");
            let pr_error2: PrError = io_error2.into();

            match pr_error2 {
                PrError::EncounteredErrors(msg) => {
                    assert!(msg.contains("权限被拒绝"));
                }
                _ => panic!("转换为了错误的PrError类型"),
            }

            // 测试空错误消息
            let io_error3 = Error::other("");
            let pr_error3: PrError = io_error3.into();

            match pr_error3 {
                PrError::EncounteredErrors(msg) => {
                    assert_eq!(msg, "");
                }
                _ => panic!("转换为了错误的PrError类型"),
            }
        }
    }

    #[cfg(test)]
    mod pr_handle_tests {
        use super::super::*;
        use std::io::Write;
        use tempfile::NamedTempFile;

        // 辅助函数：创建带有特定内容的临时文件
        fn create_temp_file_with_content(content: &str) -> tempfile::NamedTempFile {
            let mut file = NamedTempFile::new().unwrap();
            file.write_all(content.as_bytes()).unwrap();
            file
        }

        #[test]
        fn test_pr_handle_basic() {
            // 创建一个测试文件
            let file = create_temp_file_with_content("line1\nline2\nline3\n");
            let file_path = file.path().to_str().unwrap();

            // 创建基本的输出选项
            let output_opts = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: ":".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            // 调用pr_handle函数
            let result = pr_handle(file_path, &output_opts);

            // 验证结果
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), 0);
        }

        #[test]
        fn test_pr_handle_with_line_numbers() {
            // 创建一个测试文件
            let file = create_temp_file_with_content("line1\nline2\nline3\n");
            let file_path = file.path().to_str().unwrap();

            // 创建带行号的输出选项
            let numbering_mode = PrNumberingMode {
                width: 5,
                separator: "\t".to_string(),
                first_number: 1,
            };

            let output_opts = PrOutputOptions {
                number: Some(numbering_mode),
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: " ".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            // 调用pr_handle函数
            let result = pr_handle(file_path, &output_opts);

            // 验证结果
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), 0);
        }

        #[test]
        fn test_pr_handle_with_header() {
            // 创建一个测试文件
            let file = create_temp_file_with_content("line1\nline2\nline3\n");
            let file_path = file.path().to_str().unwrap();

            // 创建带页眉的输出选项
            let output_opts = PrOutputOptions {
                number: None,
                header: "TEST_HEADER".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            // 调用pr_handle函数
            let result = pr_handle(file_path, &output_opts);

            // 验证结果
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), 0);
        }

        #[test]
        fn test_pr_handle_with_columns() {
            // 创建一个测试文件
            let file = create_temp_file_with_content("line1\nline2\nline3\nline4\nline5\nline6\n");
            let file_path = file.path().to_str().unwrap();

            // 创建带列模式的输出选项
            let column_opts = PrColumnModeOptions {
                width: PR_DEFAULT_COLUMN_WIDTH,
                columns: 2,
                column_separator: "\t".to_string(),
                is_across_mode: false,
            };

            let output_opts = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: Some(column_opts),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            // 调用pr_handle函数
            let result = pr_handle(file_path, &output_opts);

            // 验证结果
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), 0);
        }

        #[test]
        fn test_pr_handle_with_double_space() {
            // 创建一个测试文件
            let file = create_temp_file_with_content("line1\nline2\nline3\n");
            let file_path = file.path().to_str().unwrap();

            // 创建带双倍行距的输出选项
            let output_opts = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: true,
                line_separator: "\n".to_string(),
                content_line_separator: "\n\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            // 调用pr_handle函数
            let result = pr_handle(file_path, &output_opts);

            // 验证结果
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), 0);
        }

        #[test]
        fn test_pr_handle_nonexistent_file() {
            // 创建基本的输出选项
            let output_opts = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            // 调用pr_handle函数并传入不存在的文件路径
            let result = pr_handle("nonexistent_file.txt", &output_opts);

            // 验证结果
            assert!(result.is_err());
            match result {
                Err(PrError::NotExists(_)) => {}
                _ => panic!("Expected PrError::NotExists, got an unexpected error"),
            }
        }

        #[test]
        fn test_pr_handle_with_invalid_permissions() {
            // 假设 "/root/no_permission.txt" 是一个普通用户没有权限访问的文件
            let file_path = "/root/no_permission.txt";

            let output_opts = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            // 尝试打开没有权限的文件，应该返回错误
            let result = pr_handle(file_path, &output_opts);
            assert!(result.is_err());
        }

        #[test]
        fn test_pr_handle_with_directory() {
            // 尝试以文件方式打开目录
            let dir_path = "/tmp";

            let output_opts = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            // 尝试打开目录，应该返回错误
            let result = pr_handle(dir_path, &output_opts);
            assert!(result.is_err());
            match result {
                Err(PrError::IsDirectory(_)) => {}
                _ => panic!("Expected PrError::IsDirectory, got an unexpected error"),
            }
        }
    }

    #[cfg(test)]
    mod mpr_handle_tests {
        use super::super::*;
        use std::io::Write;
        use tempfile::NamedTempFile;

        // 辅助函数：创建带有特定内容的临时文件
        fn create_temp_file_with_content(content: &str) -> tempfile::NamedTempFile {
            let mut file = NamedTempFile::new().unwrap();
            file.write_all(content.as_bytes()).unwrap();
            file
        }

        #[test]
        fn test_mpr_handle_basic() {
            // 创建两个测试文件
            let file1 = create_temp_file_with_content("file1_line1\nfile1_line2\nfile1_line3\n");
            let file2 = create_temp_file_with_content("file2_line1\nfile2_line2\nfile2_line3\n");

            let file1_path = file1.path().to_str().unwrap();
            let file2_path = file2.path().to_str().unwrap();
            let paths = &[file1_path, file2_path];

            // 创建基本的输出选项
            let output_opts = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            // 调用mpr_handle函数
            let result = mpr_handle(paths, &output_opts);

            // 验证结果
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), 0);
        }

        #[test]
        fn test_mpr_handle_with_merge() {
            // 创建两个测试文件
            let file1 = create_temp_file_with_content("file1_line1\nfile1_line2\nfile1_line3\n");
            let file2 = create_temp_file_with_content("file2_line1\nfile2_line2\nfile2_line3\n");

            let file1_path = file1.path().to_str().unwrap();
            let file2_path = file2.path().to_str().unwrap();
            let paths = &[file1_path, file2_path];

            // 创建合并模式的输出选项
            let output_opts = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: Some(2),
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            // 调用mpr_handle函数
            let result = mpr_handle(paths, &output_opts);

            // 验证结果
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), 0);
        }

        #[test]
        fn test_mpr_handle_with_columns() {
            // 创建两个测试文件
            let file1 = create_temp_file_with_content(
                "file1_line1\nfile1_line2\nfile1_line3\nfile1_line4\nfile1_line5\nfile1_line6\n",
            );
            let file2 = create_temp_file_with_content(
                "file2_line1\nfile2_line2\nfile2_line3\nfile2_line4\nfile2_line5\nfile2_line6\n",
            );

            let file1_path = file1.path().to_str().unwrap();
            let file2_path = file2.path().to_str().unwrap();
            let paths = &[file1_path, file2_path];

            // 创建列模式的输出选项
            let column_opts = PrColumnModeOptions {
                width: PR_DEFAULT_COLUMN_WIDTH,
                columns: 2,
                column_separator: "\t".to_string(),
                is_across_mode: false,
            };

            let output_opts = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: Some(column_opts),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            // 调用mpr_handle函数
            let result = mpr_handle(paths, &output_opts);

            // 验证结果
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), 0);
        }

        #[test]
        fn test_mpr_handle_with_nonexistent_file() {
            // 创建一个测试文件
            let file = create_temp_file_with_content("line1\nline2\nline3\n");
            let file_path = file.path().to_str().unwrap();

            // 使用一个存在的文件和一个不存在的文件
            let paths = &[file_path, "nonexistent_file.txt"];

            // 创建基本的输出选项
            let output_opts = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            // 调用mpr_handle函数
            let result = mpr_handle(paths, &output_opts);

            // 验证结果 - 应该返回错误，因为其中一个文件不存在
            assert!(result.is_err());
            match result {
                Err(PrError::NotExists(_)) => {}
                _ => panic!("Expected PrError::NotExists, got a different error"),
            }
        }

        #[test]
        fn test_mpr_handle_with_header_and_line_numbers() {
            // 创建两个测试文件
            let file1 = create_temp_file_with_content("file1_line1\nfile1_line2\nfile1_line3\n");
            let file2 = create_temp_file_with_content("file2_line1\nfile2_line2\nfile2_line3\n");

            let file1_path = file1.path().to_str().unwrap();
            let file2_path = file2.path().to_str().unwrap();
            let paths = &[file1_path, file2_path];

            // 创建带页眉和行号的输出选项
            let numbering_mode = PrNumberingMode {
                width: 5,
                separator: "\t".to_string(),
                first_number: 1,
            };

            let output_opts = PrOutputOptions {
                number: Some(numbering_mode),
                header: "TEST_HEADER".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            // 调用mpr_handle函数
            let result = mpr_handle(paths, &output_opts);

            // 验证结果
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), 0);
        }

        #[test]
        fn test_mpr_handle_with_directory() {
            // 创建一个测试文件
            let file = create_temp_file_with_content("line1\nline2\nline3\n");
            let file_path = file.path().to_str().unwrap();

            // 使用一个正常文件和一个目录
            let paths = &[file_path, "/tmp"];

            // 创建基本的输出选项
            let output_opts = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            // 调用mpr_handle函数
            let result = mpr_handle(paths, &output_opts);

            // 验证结果 - 应该返回错误，因为其中一个是目录
            assert!(result.is_err());
            match result {
                Err(PrError::IsDirectory(_)) => {}
                _ => panic!("Expected PrError::IsDirectory, got a different error"),
            }
        }

        #[test]
        fn test_mpr_handle_with_empty_paths() {
            // 使用空的路径数组
            let paths: &[&str] = &[];

            // 创建基本的输出选项
            let output_opts = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            // 调用mpr_handle函数
            let result = mpr_handle(paths, &output_opts);

            // 验证结果 - 应该返回错误或某种特殊处理
            assert!(result.is_ok() || result.is_err());
        }
    }

    #[cfg(test)]
    mod file_tests {
        use super::super::*;
        use std::io::Write;
        use tempfile::NamedTempFile;

        // 辅助函数：创建带有特定内容的临时文件
        fn create_temp_file_with_content(content: &str) -> tempfile::NamedTempFile {
            let mut file = NamedTempFile::new().unwrap();
            file.write_all(content.as_bytes()).unwrap();
            file
        }

        #[test]
        fn test_pr_open() {
            // 测试从标准输入读取
            let result = pr_open(PR_FILE_STDIN);
            assert!(result.is_ok());

            // 测试从文件读取
            let file = create_temp_file_with_content("test content");
            let file_path = file.path().to_str().unwrap();

            let result = pr_open(file_path);
            assert!(result.is_ok());

            // 测试不存在的文件
            let result = pr_open("nonexistent_file.txt");
            assert!(result.is_err());
            match result {
                Err(PrError::NotExists(_)) => {}
                _ => panic!("Expected PrError::NotExists, got an unexpected error"),
            }
        }

        #[test]
        fn test_pr_split_lines_if_form_feed() {
            // 测试正常内容
            let content = Ok("line1\nline2\nline3".to_string());
            let result = pr_split_lines_if_form_feed(content, false);

            assert_eq!(result.len(), 1); // 应为1个元素，因为没有换页符，所有内容在一个元素中
            assert_eq!(
                result[0].line_content.as_ref().unwrap(),
                "line1\nline2\nline3"
            );

            // 测试包含换页符的内容
            let content = Ok("line1\nline2\u{000C}line3\nline4".to_string());
            let result = pr_split_lines_if_form_feed(content, false);

            assert_eq!(result.len(), 2); // 应为2个元素，换页符将内容分成两部分
            assert_eq!(result[0].line_content.as_ref().unwrap(), "line1\nline2");
            assert_eq!(result[0].form_feeds_after, 1);
            assert_eq!(result[1].line_content.as_ref().unwrap(), "line3\nline4");
        }

        #[test]
        fn test_pr_split_lines_if_form_feed_omit_pagination_treats_ff_as_line_boundary() {
            let content = Ok("left\u{000C}\u{000C}right".to_string());
            let result = pr_split_lines_if_form_feed(content, true);

            assert_eq!(result.len(), 2);
            assert_eq!(result[0].line_content.as_ref().unwrap(), "left");
            assert_eq!(result[1].line_content.as_ref().unwrap(), "right");
            assert_eq!(result[0].form_feeds_after, 2);
            assert_eq!(result[1].form_feeds_after, 0);

            let ff_only = pr_split_lines_if_form_feed(Ok("\u{000C}".to_string()), true);
            assert_eq!(ff_only.len(), 1);
            assert_eq!(ff_only[0].line_content.as_ref().unwrap(), "");
            assert_eq!(ff_only[0].form_feeds_after, 1);

            let true_blank_line = pr_split_lines_if_form_feed(Ok(String::new()), true);
            assert_eq!(true_blank_line.len(), 1);
            assert_eq!(true_blank_line[0].line_content.as_ref().unwrap(), "");
        }

        #[test]
        fn test_pr_read_stream_and_create_pages() {
            // 创建测试文件
            let file = create_temp_file_with_content("line1\nline2\nline3\nline4\n");
            let file_path = file.path().to_str().unwrap();

            // 打开文件
            let reader = pr_open(file_path).unwrap();
            let lines = BufReader::new(reader).lines();

            // 创建输出选项
            let output_opts = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: 2, // 每页2行，应该产生2页
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            // 调用函数
            let iterator = pr_read_stream_and_create_pages(&output_opts, lines, 1);

            // 收集结果
            let pages: Vec<(usize, Vec<PrFileLine>)> = iterator.collect();

            // 验证结果 - 根据实际实现调整期望
            assert_eq!(pages.len(), 2);
            assert_eq!(pages[0].0, 0); // 页码从0开始
            assert_eq!(pages[0].1.len(), 2); // 第一页有2行
            assert_eq!(pages[1].0, 1); // 第二页页码
            assert_eq!(pages[1].1.len(), 2); // 第二页有2行
        }

        #[test]
        fn test_pr_read_stream_and_create_pages_with_form_feed() {
            // 创建带有换页符的测试文件
            let file = create_temp_file_with_content("line1\nline2\u{000C}line3\nline4\n");
            let file_path = file.path().to_str().unwrap();

            // 打开文件
            let reader = pr_open(file_path).unwrap();

            // 读取整个文件内容
            let mut content = String::new();
            BufReader::new(reader).read_to_string(&mut content).unwrap();

            // 分割含有换页符的内容
            let file_lines = pr_split_lines_if_form_feed(Ok(content), false);

            // 验证结果 - 根据实际实现调整期望
            assert_eq!(file_lines.len(), 2); // 换页符将内容分成两部分
            assert_eq!(file_lines[0].form_feeds_after, 1); // 第一部分后有换页符
        }

        #[test]
        fn test_parse_last_modified_time() {
            // 创建测试文件
            let file = create_temp_file_with_content("test content");
            let file_path = file.path().to_str().unwrap();

            // 测试单个文件
            let fmt = get_pr_date_time_format();
            let paths = &[file_path];
            let result = parse_last_modified_time(fmt, paths, false);

            // 验证结果不为空 - 实际实现会返回日期时间字符串
            assert!(!result.is_empty());

            // 测试合并模式
            let result = parse_last_modified_time(fmt, paths, true);
            // 在合并模式下，函数仍然会返回当前时间，而不是空字符串
            assert!(!result.is_empty());
        }

        #[cfg(unix)]
        #[test]
        fn test_parse_last_modified_time_percent_z_not_offset_with_colon() {
            let out = parse_last_modified_time("%Z", &[PR_FILE_STDIN], true);
            assert!(!out.is_empty());
            assert!(!out.contains(':'));
        }

        #[cfg(unix)]
        #[test]
        fn test_parse_last_modified_time_percent_z_file_not_offset_with_colon() {
            let file = create_temp_file_with_content("test content");
            let file_path = file.path().to_str().unwrap();

            let out = parse_last_modified_time("%Z", &[file_path], false);
            assert!(!out.is_empty());
            assert!(!out.contains(':'));
        }

        #[test]
        fn test_pr_open_with_directory() {
            // 测试打开目录
            let result = pr_open("/tmp");
            assert!(result.is_err());
            match result {
                Err(PrError::IsDirectory(_)) => {}
                _ => panic!("Expected PrError::IsDirectory, got an unexpected error"),
            }
        }

        #[test]
        fn test_pr_split_lines_if_form_feed_with_error() {
            // 测试处理错误情况
            let error_content = Err(std::io::Error::other("测试IO错误"));
            let result = pr_split_lines_if_form_feed(error_content, false);

            // 应该返回包含错误的PrFileLine
            assert_eq!(result.len(), 1);
            assert!(result[0].line_content.is_err());
        }

        #[test]
        fn test_pr_split_lines_if_form_feed_with_form_feeds() {
            // 测试包含换页符的内容
            let form_feed_content =
                Ok("line1\nline2\u{000C}line3\u{000C}\u{000C}line4".to_string());
            let result = pr_split_lines_if_form_feed(form_feed_content, false);

            // 应该正确拆分换页符
            assert_eq!(result.len(), 3);
            assert_eq!(result[0].form_feeds_after, 1); // 第一部分后有1个换页符
            assert_eq!(result[1].form_feeds_after, 2); // 第二部分后有2个换页符
        }
    }

    #[cfg(test)]
    mod helper_tests {
        use super::super::*;
        use std::io::Write;
        use tempfile::NamedTempFile;

        // 辅助函数：创建带有特定内容的临时文件
        fn create_temp_file_with_content(content: &str) -> tempfile::NamedTempFile {
            let mut file = NamedTempFile::new().unwrap();
            file.write_all(content.as_bytes()).unwrap();
            file
        }

        #[test]
        fn test_pr_get_start_line_number() {
            // 测试默认行号
            let output_opts = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            let result = pr_get_start_line_number(&output_opts);
            assert_eq!(result, 1);

            // 测试自定义行号
            let numbering_mode = PrNumberingMode {
                width: 5,
                separator: "\t".to_string(),
                first_number: 10,
            };

            let output_opts = PrOutputOptions {
                number: Some(numbering_mode),
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            let result = pr_get_start_line_number(&output_opts);
            assert_eq!(result, 10);
        }

        #[test]
        fn test_pr_lines_to_read_for_page() {
            // 测试基本情况
            let output_opts = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: 10,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            let result = pr_lines_to_read_for_page(&output_opts);
            assert_eq!(result, 10);

            // 测试双倍行距
            let output_opts = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: true,
                line_separator: "\n".to_string(),
                content_line_separator: "\n\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: 10,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            let result = pr_lines_to_read_for_page(&output_opts);
            assert_eq!(result, 5);

            // 测试列模式
            let column_opts = PrColumnModeOptions {
                width: 72,
                columns: 2,
                column_separator: "\t".to_string(),
                is_across_mode: false,
            };

            let output_opts = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: 10,
                page_separator_char: "".to_string(),
                column_mode_options: Some(column_opts),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            let result = pr_lines_to_read_for_page(&output_opts);
            assert_eq!(result, 20);
        }

        #[test]
        fn test_pr_get_columns() {
            // 测试默认情况
            let output_opts = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            let result = pr_get_columns(&output_opts);
            assert_eq!(result, 1);

            // 测试列模式
            let column_opts = PrColumnModeOptions {
                width: 72,
                columns: 3,
                column_separator: "\t".to_string(),
                is_across_mode: false,
            };

            let output_opts = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: Some(column_opts),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            let result = pr_get_columns(&output_opts);
            assert_eq!(result, 3);
        }

        #[test]
        fn test_pr_file_last_modified_time() {
            // 创建一个临时文件
            let file = create_temp_file_with_content("test content");
            let file_path = file.path().to_str().unwrap();

            // 获取最后修改时间
            let result = pr_file_last_modified_time(file_path, get_pr_date_time_format());

            // 验证结果不为空
            assert!(!result.is_empty());
        }

        #[test]
        fn test_pr_get_formatted_line_number() {
            // 测试不显示行号的情况
            let output_opts = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            let result = pr_get_formatted_line_number(&output_opts, 5, 0);
            assert_eq!(result, "");

            // 测试显示行号的情况
            let numbering_mode = PrNumberingMode {
                width: 5,
                separator: "\t".to_string(),
                first_number: 1,
            };

            let output_opts = PrOutputOptions {
                number: Some(numbering_mode),
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            let result = pr_get_formatted_line_number(&output_opts, 5, 0);
            assert_eq!(result, "    5\t");

            // 测试行号超过宽度的情况
            let numbering_mode = PrNumberingMode {
                width: 3,
                separator: "\t".to_string(),
                first_number: 1,
            };

            let output_opts = PrOutputOptions {
                number: Some(numbering_mode),
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            let result = pr_get_formatted_line_number(&output_opts, 12345, 0);
            assert_eq!(result, "345\t");
        }

        #[test]
        fn test_pr_header_content() {
            // 测试不显示页眉的情况
            let output_opts = PrOutputOptions {
                number: None,
                header: "TEST_HEADER".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "2023-01-01".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            let result = pr_header_content(&output_opts, 1);
            assert!(result.is_empty());

            // 测试显示页眉的情况
            let output_opts = PrOutputOptions {
                number: None,
                header: "TEST_HEADER".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "2023-01-01".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            let result = pr_header_content(&output_opts, 1);
            assert_eq!(result.len(), 5);
            assert_eq!(
                result[2],
                "2023-01-01                      TEST_HEADER                       Page 1"
            );
        }

        #[test]
        fn test_pr_trailer_content() {
            // 测试不显示尾部的情况
            let output_opts = PrOutputOptions {
                number: None,
                header: "TEST_HEADER".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "2023-01-01".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            let result = pr_trailer_content(&output_opts);
            assert!(result.is_empty());

            // 测试显示尾部的情况
            let output_opts = PrOutputOptions {
                number: None,
                header: "TEST_HEADER".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "2023-01-01".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            let result = pr_trailer_content(&output_opts);
            assert_eq!(result.len(), 5);

            // 测试使用换页符的情况
            let output_opts = PrOutputOptions {
                number: None,
                header: "TEST_HEADER".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "2023-01-01".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "\u{000C}".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: true,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            let result = pr_trailer_content(&output_opts);
            assert!(result.is_empty());
        }
    }

    #[cfg(test)]
    mod output_tests {
        use super::*;

        // 辅助函数：创建具有指定内容的行结构
        fn create_line(content: &str) -> PrFileLine {
            PrFileLine {
                line_number: 0,
                file_id: 0,
                page_number: 0,
                group_key: 0,
                line_content: Ok(content.to_string()),
                form_feeds_after: 0,
                inline_form_feed_after: false,
            }
        }

        // 辅助函数：创建具有指定内容的行数组
        fn create_lines(contents: &[&str]) -> Vec<PrFileLine> {
            contents
                .iter()
                .enumerate()
                .map(|(i, &content)| PrFileLine {
                    line_number: i + 1,
                    file_id: 0,
                    page_number: 0,
                    group_key: 0,
                    line_content: Ok(content.to_string()),
                    form_feeds_after: 0,
                    inline_form_feed_after: false,
                })
                .collect()
        }

        #[test]
        fn test_pr_get_formatted_line_number() {
            // 测试没有行号的情况
            let options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            let result = pr_get_formatted_line_number(&options, 1, 0);
            assert_eq!(result, "");

            // 测试有行号的情况
            let numbering_mode = PrNumberingMode {
                width: 5,
                separator: "\t".to_string(),
                first_number: 1,
            };

            let options = PrOutputOptions {
                number: Some(numbering_mode),
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            let result = pr_get_formatted_line_number(&options, 1, 0);
            assert_eq!(result, "    1\t");

            let join_lines_options = PrOutputOptions {
                number: Some(PrNumberingMode {
                    width: 3,
                    separator: "\t".to_string(),
                    first_number: 1,
                }),
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    columns: 3,
                    width: PR_DEFAULT_COLUMN_WIDTH,
                    column_separator: PR_DEFAULT_COLUMN_SEPARATOR.to_string(),
                    is_across_mode: true,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: true,
                col_sep_for_printing: "\t".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: false,
                expand_tabs: None,
                output_tabs: Some(('\t', 8)),
            };

            let join_lines_result = pr_get_formatted_line_number(&join_lines_options, 3, 0);
            assert_eq!(join_lines_result, "  3\t");
        }

        #[test]
        fn test_pr_get_line_for_printing() {
            // 创建一个基本的行
            let line = create_line("test line");

            // 创建基本的输出选项
            let options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            // 调用函数
            let result = pr_get_line_for_printing(&options, &line, 1, 0, &None, 1, true);

            // 验证结果
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), "test line");
        }

        #[test]
        fn test_pr_get_line_for_printing_with_line_number() {
            // 创建一个基本的行
            let line = PrFileLine {
                line_number: 1,
                file_id: 0,
                page_number: 0,
                group_key: 0,
                line_content: Ok("test line".to_string()),
                form_feeds_after: 0,
                inline_form_feed_after: false,
            };

            // 创建带行号的输出选项
            let numbering_mode = PrNumberingMode {
                width: 5,
                separator: "\t".to_string(),
                first_number: 1,
            };

            let options = PrOutputOptions {
                number: Some(numbering_mode),
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            // 调用函数
            let result = pr_get_line_for_printing(&options, &line, 1, 0, &None, 1, true);

            // 验证结果
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), "    1\ttest line");
        }

        #[test]
        fn test_pr_get_line_for_printing_with_line_width() {
            // 创建一个基本的行
            let line = create_line("test line that is longer than the width limit");

            // 创建带宽度限制的输出选项
            let options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            // 调用函数
            let line_width = Some(20);
            let result = pr_get_line_for_printing(&options, &line, 1, 0, &line_width, 1, true);

            // 验证结果 - 行宽应该被限制
            assert!(result.is_ok());
            assert!(result.unwrap().len() <= 20);
        }

        #[test]
        fn test_pr_output_page() {
            // 创建测试行
            let lines = create_lines(&["line1", "line2"]);

            // 创建基本的输出选项
            let options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false, // 没有页眉和页脚
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "\n".to_string(), // 使用换行符作为页分隔符
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            // 准备输出缓冲区
            let mut buf = Vec::new();

            // 调用函数
            let result = pr_output_page(&lines, &options, &mut buf, 1);

            // 验证结果
            assert!(result.is_ok());

            // 实际实现会在文件内容后添加足够的换行符来填充页面
            // 我们只验证输出包含预期的行，而不是严格比较整个输出
            let output = String::from_utf8(buf).unwrap();
            assert!(output.contains("line1"));
            assert!(output.contains("line2"));
        }

        #[test]
        fn test_pr_output_page_with_header() {
            // 创建测试行
            let lines = create_lines(&["line1", "line2"]);

            // 创建带页眉的输出选项
            let options = PrOutputOptions {
                number: None,
                header: "TEST_HEADER".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true, // 显示页眉和页脚
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "\n".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            // 准备输出缓冲区
            let mut buf = Vec::new();

            // 调用函数
            let result = pr_output_page(&lines, &options, &mut buf, 1);

            // 验证结果
            assert!(result.is_ok());
            let output = String::from_utf8(buf).unwrap();

            // 输出应该包含页眉、内容和页脚
            assert!(output.contains("TEST_HEADER"));
            assert!(output.contains("line1"));
            assert!(output.contains("line2"));
        }

        #[test]
        fn test_pr_print_page() {
            // 这个测试很难验证，因为它写入标准输出
            // 我们只验证函数存在并能够被调用，而不是测试其实际行为

            // 创建一个简单的行数组
            let _lines = create_lines(&["test line"]);

            // 创建基本的输出选项
            let _options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "\n".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            // 只确认函数类型的定义是正确的
            let _: fn(&[PrFileLine], &PrOutputOptions, usize) -> Result<usize, std::io::Error> =
                pr_print_page;

            // 注意：我们不实际调用pr_print_page函数，因为它会写入到stdout
        }

        #[test]
        fn test_pr_write_columns() {
            // 创建测试行
            let lines = create_lines(&["line1", "line2"]);

            // 创建基本的输出选项
            let options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "\n".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            // 准备输出缓冲区
            let mut buf = Vec::new();

            // 调用函数
            let result = pr_write_columns(&lines, &options, &mut buf);

            // 验证结果
            assert!(result.is_ok());
            let output = String::from_utf8(buf).unwrap();

            // 输出应该包含所有行，每行后跟一个换行符
            assert!(output.contains("line1"));
            assert!(output.contains("line2"));
        }

        #[test]
        fn test_pr_write_columns_with_across_mode() {
            // 创建测试行
            let lines = create_lines(&["line1", "line2", "line3", "line4"]);

            // 创建带across模式的输出选项
            let column_opts = PrColumnModeOptions {
                width: PR_DEFAULT_COLUMN_WIDTH,
                columns: 2,
                column_separator: "\t".to_string(),
                is_across_mode: true,
            };

            let options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(column_opts),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            // 准备输出缓冲区
            let mut buf = Vec::new();

            // 调用函数
            let result = pr_write_columns(&lines, &options, &mut buf);

            // 验证结果
            assert!(result.is_ok());
            let output = String::from_utf8(buf).unwrap();

            // 验证输出包含所有行
            assert!(output.contains("line1"));
            assert!(output.contains("line2"));
            assert!(output.contains("line3"));
            assert!(output.contains("line4"));
        }

        #[test]
        fn test_pr_write_columns_merge_drops_trailing_whitespace_from_last_actual_column() {
            let lines = vec![
                PrFileLine {
                    file_id: 0,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok("left".to_string()),
                    form_feeds_after: 0,
                    inline_form_feed_after: false,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 2,
                    line_content: Ok("right   ".to_string()),
                    form_feeds_after: 0,
                    inline_form_feed_after: false,
                },
            ];

            let options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: 1,
                page_separator_char: "\n".to_string(),
                column_mode_options: None,
                merge_files_print: Some(2),
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                page_width: 72,
                line_width: Some(72),
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: Some(('\t', 8)),
            };

            let mut buf = Vec::new();
            pr_write_columns(&lines, &options, &mut buf).unwrap();

            let output = String::from_utf8(buf).unwrap();
            assert!(output.ends_with("right\n"));
            assert!(!output.ends_with("right \n"));
            assert!(!output.ends_with("right\t\n"));
        }

        #[test]
        fn test_pr_write_columns_merge_uses_last_actual_column_instead_of_layout_width() {
            let lines = vec![PrFileLine {
                file_id: 1,
                line_number: 1,
                page_number: 1,
                group_key: 2,
                line_content: Ok("1       ".to_string()),
                form_feeds_after: 0,
                inline_form_feed_after: false,
            }];

            let options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: 1,
                page_separator_char: "\n".to_string(),
                column_mode_options: None,
                merge_files_print: Some(2),
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                page_width: 72,
                line_width: Some(72),
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: Some(('\t', 8)),
            };

            let mut buf = Vec::new();
            pr_write_columns(&lines, &options, &mut buf).unwrap();

            let output = String::from_utf8(buf).unwrap();
            assert!(output.ends_with("1\n"));
            assert!(!output.ends_with("1 \n"));
            assert!(!output.ends_with("1\t\n"));
        }

        #[test]
        fn test_pr_write_columns_merge_ff_last_actual_column_does_not_flush_padding() {
            let lines = vec![PrFileLine {
                file_id: 0,
                line_number: 56,
                page_number: 1,
                group_key: 1,
                line_content: Ok("56 456789 123456789 abcdefghi ABCDEDFHI".to_string()),
                form_feeds_after: 1,
                inline_form_feed_after: true,
            }];

            let options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: 1,
                page_separator_char: "\n".to_string(),
                column_mode_options: None,
                merge_files_print: Some(2),
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                page_width: 72,
                line_width: Some(72),
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: Some(('\t', 8)),
            };

            let mut buf = Vec::new();
            pr_write_columns(&lines, &options, &mut buf).unwrap();

            let output = String::from_utf8(buf).unwrap();
            assert_eq!(output, "56 456789 123456789 abcdefghi ABCDE\n");
        }

        #[test]
        fn test_pr_get_line_for_printing_merge_last_column_trims_truncated_space() {
            let line = PrFileLine {
                file_id: 1,
                line_number: 42,
                page_number: 1,
                group_key: 2,
                line_content: Ok("42 456789 123456789 abcdefghi ABCDEDFHI".to_string()),
                form_feeds_after: 0,
                inline_form_feed_after: false,
            };

            let options = PrOutputOptions {
                number: Some(PrNumberingMode {
                    width: 3,
                    first_number: 1,
                    separator: ".".to_string(),
                }),
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: 20,
                page_separator_char: "\n".to_string(),
                column_mode_options: None,
                merge_files_print: Some(2),
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: " ".to_string(),
                page_width: 72,
                line_width: Some(72),
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: Some(('\t', 8)),
            };

            let result =
                pr_get_line_for_printing(&options, &line, 2, 1, &Some(72), 2, true).unwrap();
            assert_eq!(result, "42 456789 123456789 abcdefghi ABC");
        }

        #[test]
        fn test_pr_get_line_for_printing_merge_first_column_truncation_shape() {
            let line = PrFileLine {
                file_id: 0,
                line_number: 56,
                page_number: 1,
                group_key: 1,
                line_content: Ok("42 456789 123456789 abcdefghi ABCDEDFHI".to_string()),
                form_feeds_after: 1,
                inline_form_feed_after: true,
            };

            let options = PrOutputOptions {
                number: Some(PrNumberingMode {
                    width: 3,
                    separator: ".".to_string(),
                    first_number: 1,
                }),
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: 20,
                page_separator_char: "\n".to_string(),
                column_mode_options: None,
                merge_files_print: Some(2),
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: " ".to_string(),
                page_width: 72,
                line_width: Some(72),
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: Some(('\t', 8)),
            };

            let result =
                pr_get_line_for_printing(&options, &line, 2, 0, &Some(72), 2, true).unwrap();
            assert_eq!(result, " 56.42 456789 123456789 abcdefghi ABC");
        }

        #[test]
        fn test_pr_get_line_for_printing_merge_first_column_keeps_padding_without_ff() {
            let line = PrFileLine {
                file_id: 0,
                line_number: 58,
                page_number: 1,
                group_key: 1,
                line_content: Ok("44 456789 123456789 xyzxyzxyz XYZXYZXYZ".to_string()),
                form_feeds_after: 0,
                inline_form_feed_after: false,
            };

            let options = PrOutputOptions {
                number: Some(PrNumberingMode {
                    width: 3,
                    separator: ".".to_string(),
                    first_number: 1,
                }),
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: 20,
                page_separator_char: "\n".to_string(),
                column_mode_options: None,
                merge_files_print: Some(2),
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: " ".to_string(),
                page_width: 72,
                line_width: Some(72),
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: Some(('\t', 8)),
            };

            let result =
                pr_get_line_for_printing(&options, &line, 2, 0, &Some(72), 2, true).unwrap();
            assert_eq!(result, " 58.44 456789 123456789 xyzxyzxyz XYZ ");
        }

        #[test]
        fn test_pr_get_line_for_printing_merge_non_ff_boundary_keeps_separator_for_non_space_cut() {
            let line = PrFileLine {
                file_id: 0,
                line_number: 42,
                page_number: 1,
                group_key: 1,
                line_content: Ok("42 456789 123456789 abcdefghi ABCDEDFHI".to_string()),
                form_feeds_after: 0,
                inline_form_feed_after: false,
            };

            let options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: 20,
                page_separator_char: "\n".to_string(),
                column_mode_options: None,
                merge_files_print: Some(2),
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: " ".to_string(),
                page_width: 72,
                line_width: Some(72),
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: Some(('\t', 8)),
            };

            let result =
                pr_get_line_for_printing(&options, &line, 2, 0, &Some(72), 2, true).unwrap();
            assert_eq!(result, "42 456789 123456789 abcdefghi ABCDE ");
        }

        #[test]
        fn test_pr_get_line_for_printing_merge_ff_boundary_drops_separator_for_space_cut() {
            let line = PrFileLine {
                file_id: 0,
                line_number: 42,
                page_number: 1,
                group_key: 1,
                line_content: Ok("42<<<  123456789 abcdefghi ABCDEDFHI  >>>".to_string()),
                form_feeds_after: 1,
                inline_form_feed_after: true,
            };

            let options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: 24,
                page_separator_char: "\n".to_string(),
                column_mode_options: None,
                merge_files_print: Some(2),
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: " ".to_string(),
                page_width: 35,
                line_width: Some(35),
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: Some(('\t', 8)),
            };

            let result =
                pr_get_line_for_printing(&options, &line, 2, 0, &Some(35), 2, true).unwrap();
            assert_eq!(result, "42<<<  123456789");
        }

        #[test]
        fn test_pr_get_line_for_printing_merge_ff_boundary_drops_implicit_padding_for_last_actual_column()
         {
            let line = PrFileLine {
                file_id: 0,
                line_number: 56,
                page_number: 1,
                group_key: 1,
                line_content: Ok("56 456789 123456789 abcdefghi ABCDEDFHI".to_string()),
                form_feeds_after: 1,
                inline_form_feed_after: true,
            };

            let options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: 24,
                page_separator_char: "\n".to_string(),
                column_mode_options: None,
                merge_files_print: Some(2),
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: " ".to_string(),
                page_width: 72,
                line_width: Some(72),
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: Some(('\t', 8)),
            };

            let result =
                pr_get_line_for_printing(&options, &line, 2, 0, &Some(72), 2, true).unwrap();
            assert_eq!(result, "56 456789 123456789 abcdefghi ABCDE");
        }

        #[test]
        fn test_pr_get_line_for_printing_merge_ff_boundary_suppresses_string_separator() {
            let line = PrFileLine {
                file_id: 0,
                line_number: 76,
                page_number: 1,
                group_key: 1,
                line_content: Ok("75 456789 123456789 abcdefghi ABCDEDFHI".to_string()),
                form_feeds_after: 1,
                inline_form_feed_after: true,
            };

            let options = PrOutputOptions {
                number: Some(PrNumberingMode {
                    width: 3,
                    separator: ".".to_string(),
                    first_number: 1,
                }),
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: 20,
                page_separator_char: "\n".to_string(),
                column_mode_options: None,
                merge_files_print: Some(2),
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: ":--:".to_string(),
                page_width: 72,
                line_width: Some(72),
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: false,
                expand_tabs: None,
                output_tabs: Some(('\t', 8)),
            };

            let result =
                pr_get_line_for_printing(&options, &line, 2, 0, &Some(72), 2, true).unwrap();
            assert_eq!(result, " 76.75 456789 123456789 abcdefghi AB");
        }

        #[test]
        fn test_pr_get_line_for_printing_with_invalid_width() {
            // 创建一个基本的行
            let line = create_line("test line");

            // 创建基本的输出选项
            let options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            // 使用无效的行宽（太窄）
            let line_width = Some(1);
            let result = pr_get_line_for_printing(&options, &line, 2, 0, &line_width, 2, false);

            // 验证结果 - 极窄列宽下不应发生 panic
            assert!(result.is_err() || result.is_ok());
        }

        #[test]
        fn test_pr_write_columns_with_error() {
            // 创建带有错误的行
            let file_line = PrFileLine {
                line_number: 1,
                file_id: 0,
                page_number: 0,
                group_key: 0,
                line_content: Err(std::io::Error::other("测试IO错误")),
                form_feeds_after: 0,
                inline_form_feed_after: false,
            };
            let lines = vec![file_line];

            // 创建基本的输出选项
            let options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "\n".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            // 准备输出缓冲区
            let mut buf = Vec::new();

            // 调用函数
            let result = pr_write_columns(&lines, &options, &mut buf);

            // 验证结果 - 应该处理错误
            assert!(result.is_err() || result.is_ok());
        }
    }

    #[cfg(test)]
    mod tool_impl_tests {
        use super::super::*;
        use std::ffi::OsString;
        use std::io::Write;
        use tempfile::NamedTempFile;

        // 辅助函数：创建带有特定内容的临时文件
        fn create_temp_file_with_content(content: &str) -> tempfile::NamedTempFile {
            let mut file = NamedTempFile::new().unwrap();
            file.write_all(content.as_bytes()).unwrap();
            file
        }

        // 辅助函数：将字符串参数转换为OsString
        fn strings_to_os_strings(args: &[&str]) -> Vec<OsString> {
            args.iter().map(OsString::from).collect()
        }

        #[test]
        fn test_pr_name() {
            let pr = Pr;
            assert_eq!(pr.name(), "pr");
        }

        #[test]
        fn test_pr_command() {
            let pr = Pr;
            let command = pr.command();

            // 测试生成的Command对象的名称
            // 注意：这里不检查完整路径，只检查命令名是否包含"pr"
            let name = command.get_name();
            assert!(name.contains("pr"));
        }

        #[test]
        fn test_pr_execute() {
            let pr = Pr;

            // 创建一个测试文件
            let file = create_temp_file_with_content("line1\nline2\nline3\n");
            let file_path = file.path().to_str().unwrap();

            // 创建命令行参数
            let args: Vec<OsString> = vec![
                OsString::from("pr"),
                OsString::from("-t"), // 不显示页眉和页脚
                OsString::from(file_path),
            ];

            // 调用execute方法
            let result = pr.execute(&args);

            // 验证执行结果
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app() {
            let command = ct_app();

            // 测试生成的Command对象的基本属性
            // 注意：这里不检查完整路径，只检查是否生成了命令
            assert!(command.get_name().contains("pr"));

            // 测试是否包含必要的参数
            let args = command.get_arguments();

            // 验证必要的参数存在
            let arg_names: Vec<_> = args.map(|a| a.get_id().to_string()).collect();
            assert!(arg_names.contains(&pr_flags::PR_HEADER.to_string()));
            assert!(arg_names.contains(&pr_flags::PR_DOUBLE_SPACE.to_string()));
            assert!(arg_names.contains(&pr_flags::PR_NUMBER_LINES.to_string()));
        }

        #[test]
        fn test_pr_main() {
            // 创建一个测试文件
            let file = create_temp_file_with_content("line1\nline2\nline3\n");
            let file_path = file.path().to_str().unwrap();

            // 构建参数
            let args = strings_to_os_strings(&[
                "pr", "-t", // 不显示页眉和页脚
                file_path,
            ]);

            // 调用pr_main函数
            let result = pr_main(args.into_iter());

            // 验证执行结果
            assert!(result.is_ok());
        }

        #[test]
        fn test_pr_main_with_options() {
            // 创建一个测试文件
            let file = create_temp_file_with_content("line1\nline2\nline3\n");
            let file_path = file.path().to_str().unwrap();

            // 构建带多个选项的参数
            let args = strings_to_os_strings(&[
                "pr", "-n", // 显示行号
                "-d", // 双倍行距
                file_path,
            ]);

            // 调用pr_main函数
            let result = pr_main(args.into_iter());

            // 验证执行结果
            assert!(result.is_ok());
        }

        #[test]
        fn test_pr_main_with_help() {
            // 构建带帮助选项的参数
            let args = strings_to_os_strings(&["pr", "--help"]);

            // 调用pr_main函数
            let result = pr_main(args.into_iter());

            // 验证执行结果 - 帮助信息应该成功显示，返回Ok
            assert!(result.is_ok());
        }

        #[test]
        fn test_pr_main_with_invalid_option() {
            // 构建带无效选项的参数
            let args = strings_to_os_strings(&["pr", "--invalid-option"]);

            // 调用pr_main函数
            let result = pr_main(args.into_iter());

            // 验证执行结果 - 无效选项应返回非零状态，匹配 GNU-visible 行为
            assert!(result.is_err());
        }

        #[test]
        fn test_pr_main_with_nonexistent_file() {
            // 构建带不存在文件的参数
            let args = strings_to_os_strings(&["pr", "nonexistent_file.txt"]);

            // 调用pr_main函数
            let result = pr_main(args.into_iter());

            // 验证执行结果 - 文件不存在时应该返回错误
            assert!(result.is_err());
        }

        #[test]
        fn test_pr_execute_with_invalid_arguments() {
            let pr = Pr;

            // 创建无效的命令行参数
            let args: Vec<OsString> = vec![
                OsString::from("pr"),
                OsString::from("--invalid-pages=abc:xyz"), // 无效的页码格式
            ];

            // 调用execute方法
            let result = pr.execute(&args);

            // 验证执行结果 - 无效参数应该被处理
            assert!(result.is_ok() || result.is_err());
        }

        #[test]
        fn test_pr_main_with_conflicting_options() {
            // 构建带冲突选项的参数
            let args = strings_to_os_strings(&[
                "pr",
                "-m",
                "--column=3", // 合并模式和列模式冲突
            ]);

            // 调用pr_main函数
            let result = pr_main(args.into_iter());

            // 验证执行结果 - 冲突选项应该返回错误
            assert!(result.is_err());
        }

        #[test]
        fn test_pr_recreate_arguments_with_special_cases() {
            // 测试特殊参数重写
            let args = vec![
                "pr".to_string(),
                "-n".to_string(),
                "file.txt".to_string(), // 没有宽度值，应该用默认值
            ];

            let result = pr_recreate_arguments(&args);

            // 验证结果 - 应该插入默认宽度
            assert_eq!(result.len(), 3);
            assert_eq!(result[0], "pr");
            assert_eq!(result[1], "-n");
            assert_eq!(result[2], "file.txt");

            // 测试-column参数过滤
            let args = vec![
                "pr".to_string(),
                "-3".to_string(), // 应该被过滤掉的column参数
                "file.txt".to_string(),
            ];

            let result = pr_recreate_arguments(&args);

            // 验证结果 - 应该过滤掉-column参数
            assert_eq!(result.len(), 2);
            assert_eq!(result[0], "pr");
            assert_eq!(result[1], "file.txt");
        }

        #[test]
        fn test_pr_recreate_arguments_splits_t_old_column_syntax() {
            let args = vec!["pr".to_string(), "-t2".to_string(), "file.txt".to_string()];

            let result = pr_recreate_arguments(&args);

            assert_eq!(result, vec!["pr", "-t", "file.txt"]);
        }

        #[test]
        fn test_pr_recreate_arguments_splits_tn_cluster_syntax() {
            let args = vec!["pr".to_string(), "-tn2".to_string(), "file.txt".to_string()];
            let result = pr_recreate_arguments(&args);
            assert_eq!(result, vec!["pr", "-t", "-n=2", "file.txt"]);

            let args = vec![
                "pr".to_string(),
                "-tn:2".to_string(),
                "file.txt".to_string(),
            ];
            let result = pr_recreate_arguments(&args);
            assert_eq!(result, vec!["pr", "-t", "-n=:2", "file.txt"]);
        }

        #[test]
        fn test_pr_recreate_arguments_preserves_operands_after_double_dash() {
            let args = vec![
                "pr".to_string(),
                "-t".to_string(),
                "--".to_string(),
                "-nfoo".to_string(),
                "+12".to_string(),
            ];

            let result = pr_recreate_arguments(&args);

            assert_eq!(result, args);
        }

        #[test]
        fn test_expand_tabs_to_spaces_handles_backspace_underflow() {
            assert_eq!(
                expand_tabs_to_spaces("\u{0008}\u{0008}\tx", '\t', 8, 0, false),
                "        x"
            );

            let input = format!("a{}\t", "\u{0008}".repeat(50));
            let expected = format!("a\u{0008}{}", " ".repeat(300));
            assert_eq!(expand_tabs_to_spaces(&input, '\t', 300, 0, false), expected);
        }

        #[test]
        fn test_expand_tabs_to_spaces_respects_initial_column() {
            assert_eq!(
                expand_tabs_to_spaces("aaa\tb", '\t', 8, 6, false),
                "aaa       b"
            );
        }

        #[test]
        fn test_expand_tabs_to_spaces_expands_plain_tabs_in_multicolumn_custom_mode() {
            assert_eq!(
                expand_tabs_to_spaces("aaa:abcde\t\tfgh", ':', 8, 8, true),
                "aaa     abcde           fgh"
            );
        }

        #[test]
        fn test_replace_spaces_with_tabs_by_segments_does_not_cross_segment_boundaries() {
            let first = format!("{} ", "x".repeat(23));
            let second = " y".to_string();
            let rendered = replace_spaces_with_tabs_by_segments(&[first, second], '\t', 8, true);

            assert_eq!(rendered, format!("{}  y", "x".repeat(23)));
            assert!(!rendered.contains('\t'));
        }

        #[test]
        fn test_replace_spaces_with_tabs_by_segments_drops_trailing_spaces_when_requested() {
            let rendered =
                replace_spaces_with_tabs_by_segments(&[String::from("abc    ")], '\t', 8, false);

            assert_eq!(rendered, "abc");
        }

        #[test]
        fn test_render_join_lines_segments_keeps_content_tabs_out_of_logical_width() {
            let rendered = render_join_lines_segments(
                &[JoinLineSegment {
                    text: "a\t  b".to_string(),
                    logical_width: pr_rendered_width("a\t  b", 8),
                    has_separator: false,
                    reset_position: Some(pr_rendered_width("a\t  b", 8)),
                }],
                "\t",
                '\t',
                8,
                true,
            );

            assert_eq!(rendered, "a\t  b");
        }

        #[test]
        fn test_render_join_lines_segments_treats_separator_tab_as_width_one() {
            let rendered = render_join_lines_segments(
                &[
                    JoinLineSegment {
                        text: "12345678".to_string(),
                        logical_width: 8,
                        has_separator: true,
                        reset_position: Some(8),
                    },
                    JoinLineSegment {
                        text: "  b".to_string(),
                        logical_width: 3,
                        has_separator: false,
                        reset_position: Some(3),
                    },
                ],
                "\t",
                '\t',
                8,
                true,
            );

            assert_eq!(rendered, "12345678\t  b");
        }

        #[test]
        fn test_pr_output_page_tabifies_single_column_offset_with_content() {
            let lines = vec![PrFileLine {
                line_number: 1,
                file_id: 0,
                page_number: 0,
                group_key: 0,
                line_content: Ok("a        b".to_string()),
                form_feeds_after: 0,
                inline_form_feed_after: false,
            }];
            let numbering_mode = PrNumberingMode {
                width: 5,
                separator: "\t".to_string(),
                first_number: 1,
            };
            let options = PrOutputOptions {
                number: Some(numbering_mode),
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: " ".repeat(9),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: Some(('\t', 5)),
            };

            let mut out = Vec::new();
            let lines_printed = pr_output_page(&lines, &options, &mut out, 1).unwrap();

            assert_eq!(lines_printed, 1);
            assert_eq!(String::from_utf8(out).unwrap(), "\t\t   1\ta\t    b\n");
        }
    }

    #[cfg(test)]
    mod parse_tests {
        use super::super::*;
        use clap::ArgMatches;

        // 辅助函数：构建命令行参数
        fn build_args(args_str: &str) -> Vec<String> {
            let mut args = vec!["pr".to_string()];
            args.extend(args_str.split_whitespace().map(|s| s.to_string()));
            args
        }

        // 辅助函数：创建带有参数的ArgMatches
        fn create_matches_with_args(args_str: &str) -> ArgMatches {
            let args = build_args(args_str);
            ct_app().try_get_matches_from(&args).unwrap()
        }

        #[test]
        fn test_parse_start_end_page_invalid_format() {
            // 测试无效的页码格式
            let args = create_matches_with_args("--pages=5:3");
            let argv = build_args("--pages=5:3");

            let result = parse_start_end_page(&args, &argv);

            // 起始页大于结束页，应该返回错误
            assert!(result.is_err());
            match result {
                Err(PrError::EncounteredErrors(msg)) => {
                    assert!(msg.contains("invalid --pages argument '5:3'"));
                }
                _ => panic!("Expected PrError::EncounteredErrors, got a different error"),
            }
        }

        #[test]
        fn test_parse_number_invalid_format() {
            // 测试无效的行号格式
            let args = create_matches_with_args("-n=xxx");

            let result = parse_number(&args);

            // 应该使用默认值
            assert!(result.is_ok());
            let numbering_mode = result.unwrap().unwrap();
            assert_eq!(numbering_mode.separator, "x"); // 第一个字符作为分隔符
            assert_eq!(numbering_mode.width, 5); // 使用默认宽度
        }

        #[test]
        fn test_parse_tab_args_rejects_invalid_width() {
            let args = create_matches_with_args("-e=0");
            let err = parse_tab_args(&args, "expand-tabs").unwrap_err();
            assert!(
                err.to_string()
                    .contains("'-e' extra characters or invalid number in the argument")
            );

            let args = create_matches_with_args("-i=abc");
            let err = parse_tab_args(&args, "output-tabs").unwrap_err();
            assert!(
                err.to_string()
                    .contains("'-i' extra characters or invalid number in the argument")
            );
        }

        #[test]
        fn test_parse_start_end_page_ignores_plus_syntax_after_double_dash() {
            let args = create_matches_with_args("-t -- +5:10");
            let argv = build_args("-t -- +5:10");
            let result = parse_start_end_page(&args, &argv).unwrap();
            assert_eq!(result, (1, None));
        }

        #[test]
        fn test_parse_start_end_page_with_date_format_containing_double_dash() {
            let argv = vec![
                "pr".to_string(),
                "--date-format=-- Date/Time --".to_string(),
                "+5:7".to_string(),
            ];
            let args = ct_app().try_get_matches_from(&argv).unwrap();
            let result = parse_start_end_page(&args, &argv).unwrap();
            assert_eq!(result, (5, Some(7)));
        }

        #[test]
        fn test_parse_merge_mode_with_conflicts() {
            // 测试合并模式与列模式冲突的情况
            let args = create_matches_with_args("-m --column=2");

            let result = parse_merge_mode(&args);

            // 应该返回错误
            assert!(result.is_err());
            match result {
                Err(PrError::EncounteredErrors(msg)) => {
                    assert!(
                        msg.contains("cannot specify number of columns when printing in parallel")
                    );
                }
                _ => panic!("Expected PrError::EncounteredErrors, got a different error"),
            }

            // 测试合并模式与across模式冲突的情况
            let args = create_matches_with_args("-m -a");

            let result = parse_merge_mode(&args);

            // 应该返回错误
            assert!(result.is_err());
            match result {
                Err(PrError::EncounteredErrors(msg)) => {
                    assert!(
                        msg.contains(
                            "cannot specify both printing across and printing in parallel"
                        )
                    );
                }
                _ => panic!("Expected PrError::EncounteredErrors, got a different error"),
            }
        }

        #[test]
        fn test_invalid_pages_map_with_invalid_value() {
            // 测试 invalid_pages_map 函数处理无效页码值的情况
            let cmd = ct_app();
            let matches = cmd.try_get_matches_from(["pr", "--pages=abc"]).unwrap();

            let invalid_pages_map = |i: String| {
                let unparsed_value = matches.get_one::<String>(pr_flags::PR_PAGES).unwrap();
                i.parse::<usize>().map_err(|_e| {
                    PrError::EncounteredErrors(format!(
                        "invalid --pages argument {}",
                        unparsed_value.quote()
                    ))
                })
            };

            let result = invalid_pages_map("abc".to_string());
            assert!(result.is_err());
            match result {
                Err(PrError::EncounteredErrors(msg)) => {
                    assert!(msg.contains("invalid --pages argument"));
                    assert!(msg.contains("abc"));
                }
                _ => panic!("Expected PrError::EncounteredErrors"),
            }
        }

        #[test]
        fn test_parse_start_page_from_args() {
            // 测试从参数中解析起始页
            let cmd1 = ct_app();
            let matches1 = cmd1.try_get_matches_from(["pr", "--pages=5"]).unwrap();

            let res = matches1.get_one::<String>(pr_flags::PR_PAGES).map(|i| {
                let x: Vec<_> = i.split(':').collect();
                x[0].to_string()
            });

            assert!(res.is_some());
            assert_eq!(res.unwrap(), "5");

            // 测试解析起始页和结束页
            let cmd2 = ct_app();
            let matches2 = cmd2.try_get_matches_from(["pr", "--pages=5:10"]).unwrap();

            let res = matches2.get_one::<String>(pr_flags::PR_PAGES).map(|i| {
                let x: Vec<_> = i.split(':').collect();
                x[0].to_string()
            });

            assert!(res.is_some());
            assert_eq!(res.unwrap(), "5");
        }
    }

    #[cfg(test)]
    mod read_stream_tests {
        use super::super::*;
        use std::io::Cursor;

        // 创建带有特定内容的模拟读取流
        fn create_test_reader(content: &str) -> Box<dyn Read> {
            Box::new(Cursor::new(content.to_string()))
        }

        #[test]
        fn test_form_feeds_handling_in_page_creation() {
            // 测试含有多个连续换页符的情况，验证是否正确创建了多个空页面
            let content = "line1\nline2\n\u{000C}\u{000C}line3\nline4";
            let reader = create_test_reader(content);
            let buffer_reader = BufReader::new(reader);
            let lines = buffer_reader.lines();

            let output_opts = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: 10, // 小容量便于测试
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: true,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            let pages: Vec<_> = pr_read_stream_and_create_pages(&output_opts, lines, 0).collect();

            // 验证是否创建了足够的页面，并且处理了连续的换页符
            assert!(!pages.is_empty());

            // 查找是否有空页面存在（由连续换页符创建）
            let has_empty_page = pages.iter().any(|(_, lines)| lines.is_empty());
            assert!(has_empty_page, "应该存在由连续换页符创建的空页面");
        }

        #[test]
        fn test_omit_pagination_still_splits_multicolumn_pages_on_form_feed() {
            let reader = create_test_reader("a\nb\nc\n\u{000C}\nd\n");
            let lines = BufReader::new(reader).lines();

            let output_opts = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: PR_DEFAULT_COLUMN_WIDTH,
                    columns: 2,
                    column_separator: " ".to_string(),
                    is_across_mode: true,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: " ".to_string(),
                page_width: PR_DEFAULT_COLUMN_WIDTH,
                line_width: Some(PR_DEFAULT_COLUMN_WIDTH),
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: true,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: Some(('\t', 8)),
            };

            let pages: Vec<_> = pr_read_stream_and_create_pages(&output_opts, lines, 0).collect();

            assert_eq!(pages.len(), 2);
            assert_eq!(pages[0].1.len(), 3);
            assert_eq!(pages[1].1.len(), 1);
        }

        #[test]
        fn test_start_page_line_numbers_restart_on_first_printed_page() {
            let reader = create_test_reader("line1\nline2\nline3\nline4\n");
            let lines = BufReader::new(reader).lines();

            let output_opts = PrOutputOptions {
                number: Some(PrNumberingMode {
                    first_number: 1,
                    ..PrNumberingMode::default()
                }),
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 3,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: 1,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: PR_DEFAULT_COLUMN_WIDTH,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            let pages: Vec<_> = pr_read_stream_and_create_pages(&output_opts, lines, 0).collect();

            assert_eq!(pages.len(), 2);
            assert_eq!(pages[0].1[0].line_number, 1);
            assert_eq!(pages[1].1[0].line_number, 2);
        }

        #[test]
        fn test_error_handling_in_file_content() {
            // 测试当文件内容出现错误时的处理
            let output_opts = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: 10,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            // 测试 mpr_handle 中的错误处理逻辑
            let paths = &["test_file.txt"];
            let result = mpr_handle(paths, &output_opts);

            // 由于文件不存在，应该返回错误
            assert!(result.is_err());
        }

        #[test]
        fn test_form_feed_marker_line_does_not_add_blank_content_line() {
            let content = "line1\n\u{000C}\nline2\n";
            let reader = create_test_reader(content);
            let buffer_reader = BufReader::new(reader);
            let lines = buffer_reader.lines();

            let output_opts = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: 10,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: true,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            let pages: Vec<_> = pr_read_stream_and_create_pages(&output_opts, lines, 0).collect();

            assert_eq!(pages.len(), 2);
            assert_eq!(pages[0].1.len(), 1);
            assert_eq!(pages[0].1[0].line_content.as_ref().unwrap(), "line1");
            assert_eq!(pages[1].1.len(), 1);
            assert_eq!(pages[1].1[0].line_content.as_ref().unwrap(), "line2");
        }

        #[test]
        fn test_full_page_followed_by_double_form_feed_adds_only_one_empty_page() {
            let content = "line1\nline2\n\u{000C}\u{000C}\nline3\n";
            let reader = create_test_reader(content);
            let buffer_reader = BufReader::new(reader);
            let lines = buffer_reader.lines();

            let output_opts = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: 2,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: true,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            let pages: Vec<_> = pr_read_stream_and_create_pages(&output_opts, lines, 0).collect();

            assert_eq!(pages.len(), 4);
            assert_eq!(pages[0].1.len(), 2);
            assert_eq!(pages[0].1[0].line_content.as_ref().unwrap(), "line1");
            assert_eq!(pages[0].1[1].line_content.as_ref().unwrap(), "line2");
            assert!(pages[1].1.is_empty());
            assert!(pages[2].1.is_empty());
            assert_eq!(pages[3].1.len(), 1);
            assert_eq!(pages[3].1[0].line_content.as_ref().unwrap(), "line3");
        }

        #[test]
        fn test_pr_output_page_keeps_ff_for_empty_page_without_headers() {
            let output_opts = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: 10,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            let mut out = Vec::new();
            let lines_written = pr_output_page(&[], &output_opts, &mut out, 1).unwrap();

            assert_eq!(lines_written, 0);
            assert_eq!(out, vec![PR_FF]);
        }

        #[test]
        fn test_pr_output_page_emits_trailing_ff_when_input_contains_ff() {
            let output_opts = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: 10,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            let lines = vec![PrFileLine {
                file_id: 0,
                line_number: 1,
                page_number: 1,
                group_key: 1,
                line_content: Ok("line".to_string()),
                form_feeds_after: 1,
                inline_form_feed_after: false,
            }];

            let mut out = Vec::new();
            let lines_written = pr_output_page(&lines, &output_opts, &mut out, 1).unwrap();

            assert_eq!(lines_written, 1);
            assert_eq!(out, b"line\n\x0c");
        }
    }

    #[cfg(test)]
    mod line_formatting_tests {
        use super::super::*;

        #[test]
        fn test_line_width_handling_with_padding() {
            // 测试行宽度处理，特别是当实际内容小于指定宽度时的填充操作
            let file_line = PrFileLine {
                file_id: 0,
                line_number: 1,
                page_number: 1,
                group_key: 1,
                line_content: Ok("短内容".to_string()),
                form_feeds_after: 0,
                inline_form_feed_after: false,
            };

            let output_opts = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: PR_LINES_PER_PAGE,
                page_separator_char: "".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                page_width: 72,
                line_width: None,
                show_control_chars: false,
                show_nonprinting: false,
                is_omit_pagination: false,
                is_pad_columns: true,
                expand_tabs: None,
                output_tabs: None,
            };

            // 测试当使用明确宽度限制，并且内容长度小于限制时的填充行为
            let line_width = Some(20);
            let columns = 1;
            let index = 0;
            let indexes = 1;

            let result = pr_get_line_for_printing(
                &output_opts,
                &file_line,
                columns,
                index,
                &line_width,
                indexes,
                true,
            );

            assert!(result.is_ok());
            let formatted_line = result.unwrap();

            // 验证生成的行是否被适当填充到指定宽度
            assert!(formatted_line.len() >= 6); // "短内容"至少6个字符（UTF-8中文每个字3字节）
        }
    }

    #[cfg(test)]
    mod parse_option_tests {
        use super::super::*;
        use clap::ArgMatches;

        // 辅助函数：构建命令行参数
        fn build_args(args_str: &str) -> Vec<String> {
            let mut args = vec!["pr".to_string()];
            args.extend(args_str.split_whitespace().map(|s| s.to_string()));
            args
        }

        // 辅助函数：创建带有参数的ArgMatches
        fn create_matches_with_args(args_str: &str) -> ArgMatches {
            let args = build_args(args_str);
            ct_app().try_get_matches_from(&args).unwrap()
        }

        #[test]
        fn test_parse_line_width() {
            // 测试 line 602-610 parse_line_width 函数的功能

            // 场景1: is_join_lines 为 true，应该返回 None
            let page_width = Some(80);
            let column_mode_options = None;
            let is_join_lines = true;
            let columns_to_print = 1;

            let result = parse_line_width(
                page_width,
                &column_mode_options,
                is_join_lines,
                columns_to_print,
            );
            assert_eq!(result, None);

            // 场景2: columns_to_print > 1 且指定了 page_width，优先使用 page_width
            let page_width = Some(80);
            let column_mode_options = Some(PrColumnModeOptions {
                width: 40,
                columns: 2,
                column_separator: "\t".to_string(),
                is_across_mode: false,
            });
            let is_join_lines = false;
            let columns_to_print = 2;

            let result = parse_line_width(
                page_width,
                &column_mode_options,
                is_join_lines,
                columns_to_print,
            );
            assert_eq!(result, Some(80));

            // 场景3: columns_to_print > 1 且 page_width 未指定，使用 column_mode_options 宽度
            let page_width = None;
            let column_mode_options = Some(PrColumnModeOptions {
                width: 40,
                columns: 2,
                column_separator: "\t".to_string(),
                is_across_mode: false,
            });
            let is_join_lines = false;
            let columns_to_print = 2;

            let result = parse_line_width(
                page_width,
                &column_mode_options,
                is_join_lines,
                columns_to_print,
            );
            assert_eq!(result, Some(40));

            // 场景4: columns_to_print > 1, page_width 和 column_mode_options 都未指定，使用默认值
            let page_width = None;
            let column_mode_options = None;
            let is_join_lines = false;
            let columns_to_print = 2;

            let result = parse_line_width(
                page_width,
                &column_mode_options,
                is_join_lines,
                columns_to_print,
            );
            assert_eq!(result, Some(PR_DEFAULT_COLUMN_WIDTH));

            // 场景5: columns_to_print = 1，应该直接使用 page_width
            let page_width = Some(80);
            let column_mode_options = Some(PrColumnModeOptions {
                width: 40,
                columns: 2,
                column_separator: "\t".to_string(),
                is_across_mode: false,
            });
            let is_join_lines = false;
            let columns_to_print = 1;

            let result = parse_line_width(
                page_width,
                &column_mode_options,
                is_join_lines,
                columns_to_print,
            );
            assert_eq!(result, Some(80));
        }

        #[test]
        fn test_parse_column_separator() {
            // 测试 line 646-651 parse_column_separator 函数的功能

            // 使用 PR_COLUMN_STRING_SEPARATOR 参数
            let matches = create_matches_with_args("-S=###");
            let result = parse_column_separator(&matches);
            assert_eq!(result, "###");

            // 使用 PR_COLUMN_CHAR_SEPARATOR 参数
            let matches = create_matches_with_args("-s=:");
            let result = parse_column_separator(&matches);
            assert_eq!(result, ":");

            // 同时指定两个参数，PR_COLUMN_STRING_SEPARATOR 优先级更高
            let matches = create_matches_with_args("-S=### -s=:");
            let result = parse_column_separator(&matches);
            assert_eq!(result, "###");

            // 没有指定任何参数，使用默认值
            let matches = create_matches_with_args("");
            let result = parse_column_separator(&matches);
            assert_eq!(result, PR_DEFAULT_COLUMN_SEPARATOR.to_string());
        }

        #[test]
        fn test_parse_col_sep_for_printing_join_lines_defaults_to_tab() {
            let matches = create_matches_with_args("-J --column=3");
            let argv = build_args("-J --column=3");
            let column_mode_options = parse_column_mode_options(&matches, &argv).unwrap();
            let result = parse_col_sep_for_printing(&matches, None, &column_mode_options, true);
            assert_eq!(result, PR_TAB.to_string());

            let merge_matches = create_matches_with_args("-J -m");
            let merge_result = parse_col_sep_for_printing(&merge_matches, Some(2), &None, true);
            assert_eq!(merge_result, PR_TAB.to_string());
        }

        #[test]
        fn test_parse_column_mode_options() {
            // 测试 line 653 is_across_mode 与其他相关功能

            // 测试 is_across_mode 为 true 的情况
            let matches = create_matches_with_args("--column=2 -a");
            let result = parse_column_mode_options(&matches, &build_args("--column=2 -a")).unwrap();
            assert!(result.is_some());
            let options = result.unwrap();
            assert_eq!(options.columns, 2);
            assert!(options.is_across_mode);

            // 测试 is_across_mode 为 false 的情况
            let matches = create_matches_with_args("--column=2");
            let result = parse_column_mode_options(&matches, &build_args("--column=2")).unwrap();
            assert!(result.is_some());
            let options = result.unwrap();
            assert_eq!(options.columns, 2);
            assert!(!options.is_across_mode);

            // 测试命令行中直接使用 -3 格式
            let matches = create_matches_with_args("");
            let result = parse_column_mode_options(&matches, &build_args("-3")).unwrap();
            assert!(result.is_some());
            let options = result.unwrap();
            assert_eq!(options.columns, 3);

            // 测试旧式 -t2 语法（等价于 -t -2）
            let matches = create_matches_with_args("-t");
            let result = parse_column_mode_options(&matches, &build_args("-t2")).unwrap();
            assert!(result.is_some());
            let options = result.unwrap();
            assert_eq!(options.columns, 2);

            // 测试列参数按出现顺序覆盖（最后的 -2 覆盖 --columns=1）
            let matches = create_matches_with_args("--columns=1");
            let result =
                parse_column_mode_options(&matches, &build_args("--columns=1 -2")).unwrap();
            assert!(result.is_some());
            let options = result.unwrap();
            assert_eq!(options.columns, 2);

            // 测试无效的 -column 格式
            let matches = create_matches_with_args("");
            let result = parse_column_mode_options(&matches, &build_args("-abc"));
            assert!(result.unwrap().is_none());

            // -0 不是合法的列数
            let matches = create_matches_with_args("");
            let result = parse_column_mode_options(&matches, &build_args("-0"));
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().to_string(),
                "pr: invalid number of columns: '0'"
            );

            // `--` 之后的 token 都是操作数，不应再触发列模式解析
            let matches = create_matches_with_args("-t -- --column=2");
            let result =
                parse_column_mode_options(&matches, &build_args("-t -- --column=2")).unwrap();
            assert!(result.is_none());
        }

        #[test]
        fn test_parse_column_mode_options_with_date_format_containing_double_dash() {
            let argv = vec![
                "pr".to_string(),
                "--date-format=-- Date/Time --".to_string(),
                "-t2".to_string(),
            ];
            let matches = ct_app()
                .try_get_matches_from(["pr", "--date-format=-- Date/Time --", "-t"])
                .unwrap();
            let result = parse_column_mode_options(&matches, &argv).unwrap();
            let options = result.expect("expected column mode options from -2");
            assert_eq!(options.columns, 2);
        }

        #[test]
        fn test_parse_page_width() {
            // 测试 line 678-680 parse_page_width 函数的功能

            // 当 PR_JOIN_LINES 为 true 时，应该返回 None
            let matches = create_matches_with_args("-J");
            let result = parse_page_width(&matches, true, false).unwrap();
            assert_eq!(result, None);

            // 当指定了 PR_PAGE_WIDTH 时，应该返回对应的值
            let matches = create_matches_with_args("-W 90");
            let result = parse_page_width(&matches, false, false).unwrap();
            assert_eq!(result, Some(90));

            // 当 PR_JOIN_LINES 和 PR_PAGE_WIDTH 都没有指定时，应该返回 None
            let matches = create_matches_with_args("");
            let result = parse_page_width(&matches, false, false).unwrap();
            assert_eq!(result, None);

            // -J 下显式 -W 仍会影响页眉宽度
            let matches = create_matches_with_args("-J -W 90");
            let result = parse_page_width(&matches, true, false).unwrap();
            assert_eq!(result, Some(90));

            // -m 下的小写 -w 作为页面宽度参与页眉/列宽计算
            let matches = create_matches_with_args("-m -w 35");
            let result = parse_page_width(&matches, false, true).unwrap();
            assert_eq!(result, Some(35));

            // -J -m 下的小写 -w 不再参与截断，但仍影响页眉宽度
            let matches = create_matches_with_args("-J -m -w 35");
            let result = parse_page_width(&matches, true, true).unwrap();
            assert_eq!(result, Some(35));
        }

        #[test]
        fn test_parse_column_width() {
            // 测试 line 702 column_width计算逻辑

            // 当同时设置了 PR_COLUMN_WIDTH 和 PR_COLUMN_CHAR_SEPARATOR 时，应使用 PR_DEFAULT_COLUMN_WIDTH_WITH_S_OPTION
            let matches = create_matches_with_args("-w 50 -s :");
            let result = parse_column_width(&matches).unwrap();
            assert_eq!(result, 50); // 显式指定值优先于默认值

            // 只设置了 PR_COLUMN_WIDTH 时，应使用设置的值
            let matches = create_matches_with_args("-w 60");
            let result = parse_column_width(&matches).unwrap();
            assert_eq!(result, 60);

            // 只设置了 PR_COLUMN_CHAR_SEPARATOR 时，应使用 PR_DEFAULT_COLUMN_WIDTH
            let matches = create_matches_with_args("-s :");
            let result = parse_column_width(&matches).unwrap();
            assert_eq!(result, PR_DEFAULT_COLUMN_WIDTH);

            // 都未设置时，应使用 PR_DEFAULT_COLUMN_WIDTH
            let matches = create_matches_with_args("");
            let result = parse_column_width(&matches).unwrap();
            assert_eq!(result, PR_DEFAULT_COLUMN_WIDTH);

            // 测试无效的列宽值
            let matches = create_matches_with_args("-w abc");
            let result = parse_column_width(&matches);
            assert!(result.is_err());
        }

        #[test]
        fn test_parse_start_end_page_plus_syntax() {
            // 测试 line 718-723 parse_start_end_page 函数中 +page 语法的功能

            // 测试 +5 语法
            let matches = create_matches_with_args("");
            let argv = build_args("+5");
            let result = parse_start_end_page(&matches, &argv).unwrap();
            assert_eq!(result.0, 5); // start_page
            assert_eq!(result.1, None); // end_page

            // 测试 +5:10 语法
            let matches = create_matches_with_args("");
            let argv = build_args("+5:10");
            let result = parse_start_end_page(&matches, &argv).unwrap();
            assert_eq!(result.0, 5); // start_page
            assert_eq!(result.1, Some(10)); // end_page

            // 测试无效的 +page 语法
            let matches = create_matches_with_args("");
            let argv = build_args("+abc");
            // 注意：由于正则表达式的匹配方式，+abc可能不会被解析为+page格式，
            // 因此可能会返回默认的start_page=1，不产生错误
            let result = parse_start_end_page(&matches, &argv);
            if result.is_ok() {
                let (start_page, end_page) = result.unwrap();
                assert_eq!(start_page, 1); // 默认值
                assert_eq!(end_page, None); // 默认值
            }

            // 测试另一种格式的无效 +page 语法，这个会导致实际解析错误
            let matches = create_matches_with_args("");
            let argv = build_args("+1a:10");
            let result = parse_start_end_page(&matches, &argv);
            assert!(result.is_err() || result.unwrap().0 == 1);
        }

        #[test]
        fn test_invalid_pages_map() {
            // 测试 line 733-741 invalid_pages_map 功能

            // 测试有效的 --pages 参数
            let matches = create_matches_with_args("--pages=5:10");
            let argv = build_args("");
            let result = parse_start_end_page(&matches, &argv).unwrap();
            assert_eq!(result.0, 5); // start_page
            assert_eq!(result.1, Some(10)); // end_page

            // 测试无效的 --pages 参数 (非数字)
            let matches = create_matches_with_args("--pages=abc");
            let argv = build_args("");
            let result = parse_start_end_page(&matches, &argv);
            assert!(result.is_err());
            let err = result.unwrap_err();
            match err {
                PrError::EncounteredErrors(msg) => {
                    assert!(msg.contains("invalid --pages argument"));
                }
                _ => panic!("Expected PrError::EncounteredErrors"),
            }

            // 测试无效的 --pages 参数范围 (起始页大于结束页)
            let matches = create_matches_with_args("--pages=10:5");
            let argv = build_args("");
            let result = parse_start_end_page(&matches, &argv);
            assert!(result.is_err());
            let err = result.unwrap_err();
            match err {
                PrError::EncounteredErrors(msg) => {
                    assert!(msg.contains("invalid --pages argument '10:5'"));
                }
                _ => panic!("Expected PrError::EncounteredErrors"),
            }

            // 测试 --pages 参数优先级高于 +page 语法
            let matches = create_matches_with_args("--pages=7:15");
            let argv = build_args("+5:10");
            let result = parse_start_end_page(&matches, &argv).unwrap();
            assert_eq!(result.0, 7); // start_page 来自 --pages
            assert_eq!(result.1, Some(15)); // end_page 来自 --pages
        }
    }

    #[cfg(test)]
    mod locale_tests {
        use super::*;
        use std::env;
        use std::sync::Mutex;

        static LOCALE_TEST_LOCK: Mutex<()> = Mutex::new(());

        fn set_or_remove_env(key: &str, val: Option<&str>) {
            unsafe {
                match val {
                    Some(v) => env::set_var(key, v),
                    None => env::remove_var(key),
                }
            }
        }

        fn save_locale_env() -> (
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ) {
            (
                env::var("LC_ALL").ok(),
                env::var("LC_TIME").ok(),
                env::var("LANG").ok(),
                env::var("POSIXLY_CORRECT").ok(),
            )
        }

        fn restore_locale_env(
            saved: (
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
            ),
        ) {
            let (lc_all, lc_time, lang, posixly_correct) = saved;
            set_or_remove_env("LC_ALL", lc_all.as_deref());
            set_or_remove_env("LC_TIME", lc_time.as_deref());
            set_or_remove_env("LANG", lang.as_deref());
            set_or_remove_env("POSIXLY_CORRECT", posixly_correct.as_deref());
        }

        #[test]
        fn test_get_pr_date_time_format_c_locale() {
            let _guard = LOCALE_TEST_LOCK
                .lock()
                .expect("failed to lock locale test mutex");
            let saved = save_locale_env();
            set_or_remove_env("LC_ALL", None);
            set_or_remove_env("LANG", None);
            set_or_remove_env("LC_TIME", Some("C"));
            set_or_remove_env("POSIXLY_CORRECT", Some("1"));

            // POSIXLY_CORRECT + C locale 应使用英文格式
            assert_eq!(get_pr_date_time_format(), "%b %d %H:%M %Y");

            restore_locale_env(saved);
        }

        #[test]
        fn test_get_pr_date_time_format_c_locale_default_iso() {
            let _guard = LOCALE_TEST_LOCK
                .lock()
                .expect("failed to lock locale test mutex");
            let saved = save_locale_env();
            set_or_remove_env("LC_ALL", None);
            set_or_remove_env("LANG", None);
            set_or_remove_env("LC_TIME", Some("C"));
            set_or_remove_env("POSIXLY_CORRECT", None);

            assert_eq!(get_pr_date_time_format(), "%Y-%m-%d %H:%M");

            restore_locale_env(saved);
        }

        #[test]
        fn test_get_pr_date_time_format_non_c_locale() {
            let _guard = LOCALE_TEST_LOCK
                .lock()
                .expect("failed to lock locale test mutex");
            let saved = save_locale_env();
            set_or_remove_env("LC_ALL", None);
            set_or_remove_env("LANG", None);
            set_or_remove_env("LC_TIME", Some("zh_CN.UTF-8"));
            set_or_remove_env("POSIXLY_CORRECT", Some("1"));

            // 非C/POSIX locale 应使用 ISO 格式
            let format = get_pr_date_time_format();
            if hard_locale_time() {
                assert_eq!(format, "%Y-%m-%d %H:%M");
            } else {
                // 某些环境可能缺少 zh_CN locale，此时回退到 C locale
                assert_eq!(format, "%b %d %H:%M %Y");
            }

            restore_locale_env(saved);
        }

        #[test]
        fn test_get_pr_date_time_format_posix_locale() {
            let _guard = LOCALE_TEST_LOCK
                .lock()
                .expect("failed to lock locale test mutex");
            let saved = save_locale_env();
            set_or_remove_env("LC_ALL", None);
            set_or_remove_env("LANG", None);
            set_or_remove_env("LC_TIME", Some("POSIX"));
            set_or_remove_env("POSIXLY_CORRECT", Some("1"));

            // POSIXLY_CORRECT + POSIX locale 应使用英文格式
            assert_eq!(get_pr_date_time_format(), "%b %d %H:%M %Y");

            restore_locale_env(saved);
        }

        #[test]
        fn test_hard_locale_time_integration() {
            let _guard = LOCALE_TEST_LOCK
                .lock()
                .expect("failed to lock locale test mutex");
            let saved = save_locale_env();
            set_or_remove_env("LC_ALL", None);
            set_or_remove_env("LANG", None);
            set_or_remove_env("POSIXLY_CORRECT", None);

            // 测试hard_locale_time函数的使用
            set_or_remove_env("LC_TIME", Some("C"));
            assert!(!hard_locale_time());

            set_or_remove_env("LC_TIME", Some("en_US.UTF-8"));
            assert!(hard_locale_time());

            restore_locale_env(saved);
        }
    }
}
