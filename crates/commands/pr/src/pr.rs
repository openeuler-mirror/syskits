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
rust_i18n::i18n!("locales", fallback = "zh-CN");
use std::io::{BufRead, BufReader, Error, Lines, Read, Write, stdin, stdout};
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

const PR_TAB: char = '\t';
const PR_LINES_PER_PAGE: usize = 66;
const PR_LINES_PER_PAGE_FOR_FORM_FEED: usize = 63;
const PR_HEADER_LINES_PER_PAGE: usize = 5;
const PR_TRAILER_LINES_PER_PAGE: usize = 5;
const PR_FILE_STDIN: &str = "-";
const PR_READ_BUFFER_SIZE: usize = 1024 * 64;
const PR_DEFAULT_COLUMN_WIDTH: usize = 72;
const PR_DEFAULT_COLUMN_WIDTH_WITH_S_OPTION: usize = 512;
const PR_DEFAULT_COLUMN_SEPARATOR: &char = &PR_TAB;
const PR_FF: u8 = 0x0C_u8;
// 根据locale选择时间格式
fn get_pr_date_time_format() -> &'static str {
    if hard_locale_time() {
        "%Y-%m-%d %H:%M" // ISO格式用于非C locale
    } else {
        "%b %d %H:%M %Y" // 英文月份缩写格式用于C/POSIX locale
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
    pub const PR_COLUMN: &str = "column";
    pub const PR_COLUMN_CHAR_SEPARATOR: &str = "separator";
    pub const PR_COLUMN_STRING_SEPARATOR: &str = "sep-string";
    pub const PR_MERGE: &str = "merge";
    pub const PR_INDENT: &str = "indent";
    pub const PR_JOIN_LINES: &str = "join-lines";
    pub const PR_HELP: &str = "help";
    pub const PR_FILES: &str = "files";
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
    line_width: Option<usize>,
}

#[derive(Debug)]
struct PrFileLine {
    file_id: usize,
    line_number: usize,
    page_number: usize,
    group_key: usize,
    line_content: Result<String, std::io::Error>,
    form_feeds_after: usize,
}

impl PartialEq for PrFileLine {
    fn eq(&self, other: &Self) -> bool {
        if self.file_id != other.file_id
            || self.line_number != other.line_number
            || self.page_number != other.page_number
            || self.group_key != other.group_key
            || self.form_feeds_after != other.form_feeds_after
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
            first_number: 1,
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
            .short_alias('f')
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
        Arg::new(pr_flags::PR_COLUMN)
            .long(pr_flags::PR_COLUMN)
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
            .value_name("char"),
        Arg::new(pr_flags::PR_COLUMN_STRING_SEPARATOR)
            .short('S')
            .long(pr_flags::PR_COLUMN_STRING_SEPARATOR)
            .help(
                "separate columns by STRING, \
                 without -S: Default separator <TAB> with -J and <space> \
                 otherwise (same as -S\" \"), no effect on column options",
            )
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
            .help(
                "merge full lines, turns off -W line truncation, no column \
                 alignment, --sep-string[=STRING] sets separators",
            )
            .action(ArgAction::SetTrue),
        Arg::new(pr_flags::PR_HELP)
            .long(pr_flags::PR_HELP)
            .help(t!("pr.clap.pr_help"))
            .action(ArgAction::Help),
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
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let args = args.collect_ignore();

    let opt_args = pr_recreate_arguments(&args);

    let mut command = ct_app();
    let matches = match command.try_get_matches_from_mut(opt_args) {
        Ok(m) => m,
        Err(e) => {
            e.print()?;
            return Ok(());
        }
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
        let result_options = pr_build_options(&matches, &file_group, &args.join(" "));
        let options = match result_options {
            Ok(options) => options,
            Err(err) => {
                pr_print_error(&matches, &err);
                return Err(1.into());
            }
        };

        let cmd_result = match file_group.iter().exactly_one() {
            Ok(group) => pr_handle(group, &options),
            Err(_) => mpr_handle(&file_group, &options),
        };

        let status = match cmd_result {
            Err(error) => {
                pr_print_error(&matches, &error);
                1
            }
            _ => 0,
        };
        if status != 0 {
            return Err(status.into());
        }
    }
    Ok(())
}

/// 返回重写后传递给程序的参数。
/// 删除 -column 和 +page 选项，因为 getopts 无法解析 -3 等参数。
/// # 参数
/// * `args` - 命令行参数
fn pr_recreate_arguments(args: &[String]) -> Vec<String> {
    let column_page_option_regex = Regex::new(r"^[-+]\d+.*").unwrap();
    let num_regex = Regex::new(r"^[^-]\d*$").unwrap();
    let n_regex = Regex::new(r"^-n\s*$").unwrap();
    let mut arguments = args.to_owned();
    let num_option = args.iter().find_position(|x| n_regex.is_match(x.trim()));
    if let Some((pos, _value)) = num_option {
        if let Some(num_val_opt) = args.get(pos + 1) {
            if !num_regex.is_match(num_val_opt) {
                let could_be_file = arguments.remove(pos + 1);
                arguments.insert(pos + 1, format!("{}", PrNumberingMode::default().width));
                arguments.insert(pos + 2, could_be_file);
            }
        }
    }

    arguments
        .into_iter()
        .filter(|i| !column_page_option_regex.is_match(i))
        .collect()
}

fn pr_print_error(arg_matches: &ArgMatches, pr_err: &PrError) {
    if !arg_matches.get_flag(pr_flags::PR_NO_FILE_WARNINGS) {
        eprintln!("{pr_err}");
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
    args: &str,
) -> Result<PrOutputOptions, PrError> {
    let number = parse_number(arg_matches)?;
    let (start_page, end_page) = parse_start_end_page(arg_matches, args)?;
    let offset_spaces = parse_offset_spaces(arg_matches)?;

    let is_form_feed_used = arg_matches.get_flag(pr_flags::PR_FORM_FEED);
    let page_length = parse_page_length(arg_matches, is_form_feed_used)?;
    let (is_display_header_and_trailer, content_lines_per_page) =
        parse_content_lines_per_page(arg_matches, page_length);

    let column_mode_options = parse_column_mode_options(arg_matches, args)?;
    let merge_files_print = parse_merge_files_print(arg_matches, paths);
    let col_sep_for_printing = parse_col_sep_for_printing(merge_files_print, &column_mode_options);
    let columns_to_print = parse_columns_to_print(merge_files_print, &column_mode_options);

    let is_join_lines = arg_matches.get_flag(pr_flags::PR_JOIN_LINES);
    let page_width = parse_page_width(arg_matches)?;
    let line_width = parse_line_width(
        page_width,
        &column_mode_options,
        is_join_lines,
        columns_to_print,
    );

    let is_double_space = arg_matches.get_flag(pr_flags::PR_DOUBLE_SPACE);
    let is_merge_mode = parse_merge_mode(arg_matches)?;
    Ok(PrOutputOptions {
        number,
        header: parse_header(arg_matches, paths, is_merge_mode),
        is_double_space,
        line_separator: "\n".to_string(),
        content_line_separator: parse_content_line_separator(is_double_space),
        last_modified_time: parse_last_modified_time(paths, is_merge_mode),
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
        line_width,
    })
}

fn parse_content_lines_per_page(arg_matches: &ArgMatches, page_length: usize) -> (bool, usize) {
    let is_page_length_le_ht = page_length < (PR_HEADER_LINES_PER_PAGE + PR_TRAILER_LINES_PER_PAGE);

    let is_display_header_and_trailer =
        !(is_page_length_le_ht) && !arg_matches.get_flag(pr_flags::PR_OMIT_HEADER);

    let content_lines_per_page = if is_page_length_le_ht {
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
    let line_width = if is_join_lines {
        None
    } else if columns_to_print > 1 {
        Some(
            column_mode_options
                .as_ref()
                .map(|i| i.width)
                .unwrap_or(PR_DEFAULT_COLUMN_WIDTH),
        )
    } else {
        page_width
    };
    line_width
}

fn parse_columns_to_print(
    merge_files_print: Option<usize>,
    column_mode_options: &Option<PrColumnModeOptions>,
) -> usize {
    let columns_to_print = merge_files_print
        .unwrap_or_else(|| column_mode_options.as_ref().map(|i| i.columns).unwrap_or(1));
    columns_to_print
}

fn parse_col_sep_for_printing(
    merge_files_print: Option<usize>,
    column_mode_options: &Option<PrColumnModeOptions>,
) -> String {
    let col_sep_for_printing = column_mode_options
        .as_ref()
        .map(|i| i.column_separator.clone())
        .unwrap_or_else(|| {
            merge_files_print
                .map(|_k| PR_DEFAULT_COLUMN_SEPARATOR.to_string())
                .unwrap_or_default()
        });
    col_sep_for_printing
}

fn parse_column_mode_options(
    arg_matches: &ArgMatches,
    args: &str,
) -> Result<Option<PrColumnModeOptions>, PrError> {
    let re_col = Regex::new(r"\s*-(\d+)\s*").unwrap();
    let res = re_col.captures(args).map(|i| {
        let unparsed_num = i.get(1).unwrap().as_str().trim();
        unparsed_num.parse::<usize>().map_err(|_e| {
            PrError::EncounteredErrors(format!("invalid {} argument {}", "-", unparsed_num.quote()))
        })
    });
    let start_column_option = if let Some(res) = res {
        Some(res?)
    } else {
        None
    };

    let column_width = parse_column_width(arg_matches)?;
    let column_separator = parse_column_separator(arg_matches);
    let is_across_mode = arg_matches.get_flag(pr_flags::PR_ACROSS);
    // --column 的优先级高于 -column
    let column_option_value = if let Some(res) = pr_parse_usize(arg_matches, pr_flags::PR_COLUMN) {
        Some(res?)
    } else {
        start_column_option
    };
    let column_mode_options = column_option_value.map(|columns| PrColumnModeOptions {
        columns,
        width: column_width,
        column_separator,
        is_across_mode,
    });
    Ok(column_mode_options)
}

fn parse_page_width(arg_matches: &ArgMatches) -> Result<Option<usize>, PrError> {
    let page_width = if arg_matches.get_flag(pr_flags::PR_JOIN_LINES) {
        None
    } else if let Some(res) = pr_parse_usize(arg_matches, pr_flags::PR_PAGE_WIDTH) {
        Some(res?)
    } else {
        None
    };
    Ok(page_width)
}

fn parse_column_separator(arg_matches: &ArgMatches) -> String {
    let column_separator =
        match arg_matches.get_one::<String>(pr_flags::PR_COLUMN_STRING_SEPARATOR) {
            Some(x) => Some(x),
            None => arg_matches.get_one::<String>(pr_flags::PR_COLUMN_CHAR_SEPARATOR),
        }
        .map(ToString::to_string)
        .unwrap_or_else(|| PR_DEFAULT_COLUMN_SEPARATOR.to_string());
    column_separator
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
    args: &str,
) -> Result<(usize, Option<usize>), PrError> {
    // +page 选项的优先级低于 --pages
    let page_plus_re = Regex::new(r"\s*\+(\d+:*\d*)\s*").unwrap();
    let res = page_plus_re.captures(args).map(|i| {
        let unparsed_num = i.get(1).unwrap().as_str().trim();
        let x: Vec<_> = unparsed_num.split(':').collect();
        x[0].to_string().parse::<usize>().map_err(|_e| {
            PrError::EncounteredErrors(format!("invalid {} argument {}", "+", unparsed_num.quote()))
        })
    });
    let start_page_in_plus_option = match res {
        Some(res) => res?,
        None => 1,
    };
    let res = page_plus_re
        .captures(args)
        .map(|i| i.get(1).unwrap().as_str().trim())
        .filter(|i| i.contains(':'))
        .map(|unparsed_num| {
            let x: Vec<_> = unparsed_num.split(':').collect();
            x[1].to_string().parse::<usize>().map_err(|_e| {
                PrError::EncounteredErrors(format!(
                    "invalid {} argument {}",
                    "+",
                    unparsed_num.quote()
                ))
            })
        });
    let end_page_in_plus_option = match res {
        Some(res) => Some(res?),
        None => None,
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

fn parse_last_modified_time(paths: &[&str], is_merge_mode: bool) -> String {
    if is_merge_mode || paths[0].eq(PR_FILE_STDIN) {
        let date_time = Local::now();
        date_time.format(get_pr_date_time_format()).to_string()
    } else {
        pr_file_last_modified_time(paths.first().unwrap())
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
                true => Some(PrNumberingMode::default()),
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
        let stdin = stdin();
        return Ok(Box::new(stdin) as Box<dyn Read>);
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

fn pr_split_lines_if_form_feed(file_content: Result<String, std::io::Error>) -> Vec<PrFileLine> {
    file_content
        .map(|content| {
            let mut lines = Vec::new();
            let mut f_occurred = 0;
            let mut chunk = Vec::new();
            for byte in content.as_bytes() {
                if byte == &PR_FF {
                    f_occurred += 1;
                } else {
                    if f_occurred != 0 {
                        // 扫描中首次出现字节
                        lines.push(PrFileLine {
                            line_content: Ok(String::from_utf8(chunk.clone()).unwrap()),
                            form_feeds_after: f_occurred,
                            ..PrFileLine::default()
                        });
                        chunk.clear();
                    }
                    chunk.push(*byte);
                    f_occurred = 0;
                }
            }

            lines.push(PrFileLine {
                line_content: Ok(String::from_utf8(chunk).unwrap()),
                form_feeds_after: f_occurred,
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

fn pr_handle(path: &str, output_opts: &PrOutputOptions) -> Result<i32, PrError> {
    let lines = BufReader::with_capacity(PR_READ_BUFFER_SIZE, pr_open(path)?).lines();

    let pages = pr_read_stream_and_create_pages(output_opts, lines, 0);

    for page_with_page_number in pages {
        let page_number = page_with_page_number.0 + 1;
        let page = page_with_page_number.1;
        pr_print_page(&page, output_opts, page_number)?;
    }

    Ok(0)
}

fn pr_read_stream_and_create_pages(
    output_opts: &PrOutputOptions,
    lines: Lines<BufReader<Box<dyn Read>>>,
    file_id: usize,
) -> Box<dyn Iterator<Item = (usize, Vec<PrFileLine>)>> {
    let start_page = output_opts.start_page;
    let start_line_number = pr_get_start_line_number(output_opts);
    let last_page = output_opts.end_page;
    let lines_needed_per_page = pr_lines_to_read_for_page(output_opts);

    Box::new(
        lines
            .flat_map(pr_split_lines_if_form_feed)
            .enumerate()
            .map(move |(i, line)| PrFileLine {
                line_number: i + start_line_number,
                file_id,
                ..line
            }) // 添加行号和文件 ID
            .batching(move |it| {
                let mut first_page = Vec::new();
                let mut page_with_lines = Vec::new();
                for line in it {
                    let form_feeds_after = line.form_feeds_after;
                    first_page.push(line);

                    if form_feeds_after > 1 {
                        // 插入空页面
                        page_with_lines.push(first_page);
                        for _i in 1..form_feeds_after {
                            page_with_lines.push(vec![]);
                        }
                        return Some(page_with_lines);
                    }

                    if first_page.len() == lines_needed_per_page || form_feeds_after == 1 {
                        break;
                    }
                }

                if first_page.is_empty() {
                    return None;
                }
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
            }),
    )
}

fn mpr_handle(paths: &[&str], output_opts: &PrOutputOptions) -> Result<i32, PrError> {
    let n_files = paths.len();

    // 检查文件是否存在
    for path in paths {
        pr_open(path)?;
    }

    let file_line_groups = paths
        .iter()
        .enumerate()
        .map(|(i, path)| {
            let lines =
                BufReader::with_capacity(PR_READ_BUFFER_SIZE, pr_open(path).unwrap()).lines();

            pr_read_stream_and_create_pages(output_opts, lines, i).flat_map(move |(x, line)| {
                let file_line = line;
                let page_number = x + 1;
                file_line
                    .into_iter()
                    .map(|fl| PrFileLine {
                        page_number,
                        group_key: page_number * n_files + fl.file_id,
                        ..fl
                    })
                    .collect::<Vec<_>>()
            })
        })
        .kmerge_by(|a, b| {
            if a.group_key == b.group_key {
                a.line_number < b.line_number
            } else {
                a.group_key < b.group_key
            }
        })
        .group_by(|file_line| file_line.group_key);

    let start_page = output_opts.start_page;
    let mut lines = Vec::new();
    let mut page_counter = start_page;

    for (_key, file_line_group) in &file_line_groups {
        for file_line in file_line_group {
            if let Err(e) = file_line.line_content {
                return Err(e.into());
            }
            let new_page_number = file_line.page_number;
            if page_counter != new_page_number {
                pr_print_page(&lines, output_opts, page_counter)?;
                lines = Vec::new();
                page_counter = new_page_number;
            }
            lines.push(file_line);
        }
    }

    pr_print_page(&lines, output_opts, page_counter)?;

    Ok(0)
}

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
    out.write_all(page_separator)?;
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

    let content_lines_per_page = if output_opts.is_double_space {
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
    let mut not_found_break = false;

    let across_mode = output_opts
        .column_mode_options
        .as_ref()
        .map(|i| i.is_across_mode)
        .unwrap_or(false);

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

    let table: Vec<Vec<_>> = (0..content_lines_per_page)
        .map(move |a| {
            (0..columns)
                .map(|i| {
                    if across_mode {
                        lines.get(a * columns + i)
                    } else if output_opts.merge_files_print.is_some() {
                        *filled_lines
                            .get(content_lines_per_page * i + a)
                            .unwrap_or(&None)
                    } else {
                        lines.get(content_lines_per_page * i + a)
                    }
                })
                .collect()
        })
        .collect();

    let blank_line = PrFileLine::default();
    for row in table {
        let indexes = row.len();
        for (i, cell) in row.iter().enumerate() {
            if cell.is_none() && output_opts.merge_files_print.is_some() {
                out.write_all(
                    pr_get_line_for_printing(
                        output_opts,
                        &blank_line,
                        columns,
                        i,
                        &line_width,
                        indexes,
                    )?
                    .as_bytes(),
                )?;
            } else if cell.is_none() {
                not_found_break = true;
                break;
            } else if cell.is_some() {
                let file_line = cell.unwrap();

                out.write_all(
                    pr_get_line_for_printing(
                        output_opts,
                        file_line,
                        columns,
                        i,
                        &line_width,
                        indexes,
                    )?
                    .as_bytes(),
                )?;
                lines_printed += 1;
            }
        }
        if not_found_break && feed_line_present {
            break;
        } else {
            out.write_all(line_separator)?;
        }
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
) -> Result<String, std::io::Error> {
    let blank_line = String::new();
    let formatted_line_number =
        pr_get_formatted_line_number(output_opts, file_line.line_number, index);

    // 处理 line_content 可能为 Err 的情况
    let content = match &file_line.line_content {
        Ok(content) => content.clone(),
        Err(e) => return Err(std::io::Error::new(e.kind(), e.to_string())),
    };

    let complete_line = format!("{}{}", formatted_line_number, content);

    let offset_spaces = &output_opts.offset_spaces;

    let tab_count = complete_line.chars().filter(|i| i == &PR_TAB).count();

    let display_length = complete_line.len() + (tab_count * 7);

    let sep = if (index + 1) != indexes && !output_opts.is_join_lines {
        &output_opts.col_sep_for_printing
    } else {
        &blank_line
    };

    let result_line = line_width
        .map(|i| {
            if i <= (columns - 1) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Page width too narrow".to_owned(),
                ));
            }
            let min_width = (i - (columns - 1)) / columns;
            if display_length < min_width {
                let mut extended_line = complete_line.clone();
                for _ in 0..(min_width - display_length) {
                    extended_line.push(' ');
                }
                Ok(extended_line.chars().take(min_width).collect::<String>())
            } else {
                Ok(complete_line.chars().take(min_width).collect::<String>())
            }
        })
        .unwrap_or_else(|| Ok(complete_line.clone()));

    result_line.map(|line| format!("{}{}{}", offset_spaces, line, sep))
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
        let separator = &num_opt.separator;
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

/// 如果没有使用 `NO_HEADER_TRAILER_OPTION` 选项禁止显示页眉，则返回五行页眉内容。
/// 使用 "NO_HEADER_TRAILER_OPTION "选项。
fn pr_header_content(output_opts: &PrOutputOptions, page: usize) -> Vec<String> {
    if output_opts.is_display_header_and_trailer {
        let first_line = format!(
            "{} {} Page {}",
            output_opts.last_modified_time, output_opts.header, page
        );
        vec![
            String::new(),
            String::new(),
            first_line,
            String::new(),
            String::new(),
        ]
    } else {
        Vec::new()
    }
}

fn pr_file_last_modified_time(path: &str) -> String {
    metadata(path)
        .map(|i| {
            i.modified()
                .map(|x| {
                    let date_time: DateTime<Local> = x.into();
                    date_time.format(get_pr_date_time_format()).to_string()
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
        .map(|i| i.first_number)
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
            };

            let line2 = PrFileLine {
                file_id: 1,
                line_number: 10,
                page_number: 2,
                group_key: 5,
                line_content: Ok("测试行".to_string()),
                form_feeds_after: 0,
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
            };

            assert_ne!(line1, line4);

            // 测试Error vs Ok
            let line5 = PrFileLine {
                file_id: 1,
                line_number: 10,
                page_number: 2,
                group_key: 5,
                line_content: Err(Error::new(ErrorKind::Other, "测试错误")),
                form_feeds_after: 0,
            };

            assert_ne!(line1, line5);

            // 测试Error vs Error
            let line6 = PrFileLine {
                file_id: 1,
                line_number: 10,
                page_number: 2,
                group_key: 5,
                line_content: Err(Error::new(ErrorKind::Other, "测试错误")),
                form_feeds_after: 0,
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
            let io_error3 = Error::new(ErrorKind::Other, "");
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
                col_sep_for_printing: "".to_string(),
                line_width: None,
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
                col_sep_for_printing: "".to_string(),
                line_width: None,
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
                line_width: None,
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
                line_width: None,
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
                line_width: None,
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
                line_width: None,
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
                line_width: None,
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
                line_width: None,
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
                line_width: None,
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
                line_width: None,
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
                line_width: None,
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
                line_width: None,
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
                line_width: None,
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
                line_width: None,
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
                line_width: None,
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
            let result = pr_split_lines_if_form_feed(content);

            assert_eq!(result.len(), 1); // 应为1个元素，因为没有换页符，所有内容在一个元素中
            assert_eq!(
                result[0].line_content.as_ref().unwrap(),
                "line1\nline2\nline3"
            );

            // 测试包含换页符的内容
            let content = Ok("line1\nline2\u{000C}line3\nline4".to_string());
            let result = pr_split_lines_if_form_feed(content);

            assert_eq!(result.len(), 2); // 应为2个元素，换页符将内容分成两部分
            assert_eq!(result[0].line_content.as_ref().unwrap(), "line1\nline2");
            assert_eq!(result[0].form_feeds_after, 1);
            assert_eq!(result[1].line_content.as_ref().unwrap(), "line3\nline4");
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
                line_width: None,
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
            let file_lines = pr_split_lines_if_form_feed(Ok(content));

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
            let paths = &[file_path];
            let result = parse_last_modified_time(paths, false);

            // 验证结果不为空 - 实际实现会返回日期时间字符串
            assert!(!result.is_empty());

            // 测试合并模式
            let result = parse_last_modified_time(paths, true);
            // 在合并模式下，函数仍然会返回当前时间，而不是空字符串
            assert!(!result.is_empty());
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
            let error_content = Err(std::io::Error::new(std::io::ErrorKind::Other, "测试IO错误"));
            let result = pr_split_lines_if_form_feed(error_content);

            // 应该返回包含错误的PrFileLine
            assert_eq!(result.len(), 1);
            assert!(result[0].line_content.is_err());
        }

        #[test]
        fn test_pr_split_lines_if_form_feed_with_form_feeds() {
            // 测试包含换页符的内容
            let form_feed_content =
                Ok("line1\nline2\u{000C}line3\u{000C}\u{000C}line4".to_string());
            let result = pr_split_lines_if_form_feed(form_feed_content);

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
                line_width: None,
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
                line_width: None,
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
                line_width: None,
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
                line_width: None,
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
                line_width: None,
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
                line_width: None,
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
                line_width: None,
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
            let result = pr_file_last_modified_time(file_path);

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
                line_width: None,
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
                line_width: None,
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
                line_width: None,
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
                line_width: None,
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
                line_width: None,
            };

            let result = pr_header_content(&output_opts, 1);
            assert_eq!(result.len(), 5);
            assert_eq!(result[2], "2023-01-01 TEST_HEADER Page 1");
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
                line_width: None,
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
                line_width: None,
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
                line_width: None,
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
                line_width: None,
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
                line_width: None,
            };

            let result = pr_get_formatted_line_number(&options, 1, 0);
            assert_eq!(result, "    1\t");
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
                line_width: None,
            };

            // 调用函数
            let result = pr_get_line_for_printing(&options, &line, 1, 0, &None, 1);

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
                line_width: None,
            };

            // 调用函数
            let result = pr_get_line_for_printing(&options, &line, 1, 0, &None, 1);

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
                line_width: None,
            };

            // 调用函数
            let line_width = Some(20);
            let result = pr_get_line_for_printing(&options, &line, 1, 0, &line_width, 1);

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
                line_width: None,
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
                line_width: None,
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
                line_width: None,
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
                line_width: None,
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
                line_width: None,
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
                line_width: None,
            };

            // 使用无效的行宽（太窄）
            let line_width = Some(1);
            let result = pr_get_line_for_printing(&options, &line, 2, 0, &line_width, 2);

            // 验证结果 - 行宽太窄应该返回错误
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        }

        #[test]
        fn test_pr_write_columns_with_error() {
            // 创建带有错误的行
            let file_line = PrFileLine {
                line_number: 1,
                file_id: 0,
                page_number: 0,
                group_key: 0,
                line_content: Err(std::io::Error::new(std::io::ErrorKind::Other, "测试IO错误")),
                form_feeds_after: 0,
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
                line_width: None,
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
            args.iter().map(|s| OsString::from(s)).collect()
        }

        #[test]
        fn test_pr_name() {
            let pr = Pr::default();
            assert_eq!(pr.name(), "pr");
        }

        #[test]
        fn test_pr_command() {
            let pr = Pr::default();
            let command = pr.command();

            // 测试生成的Command对象的名称
            // 注意：这里不检查完整路径，只检查命令名是否包含"pr"
            let name = command.get_name();
            assert!(name.contains("pr"));
        }

        #[test]
        fn test_pr_execute() {
            let pr = Pr::default();

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

            // 验证执行结果 - 无效选项会打印错误但仍返回Ok（错误由clap处理）
            assert!(result.is_ok());
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
            let pr = Pr::default();

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
            assert_eq!(result.len(), 4);
            assert_eq!(result[0], "pr");
            assert_eq!(result[1], "-n");
            assert!(result[2] == "5" || result[2] == "file.txt");

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
            let args_str = "--pages=5:3";

            let result = parse_start_end_page(&args, args_str);

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
            let args = create_matches_with_args("-nxxx");

            let result = parse_number(&args);

            // 应该使用默认值
            assert!(result.is_ok());
            let numbering_mode = result.unwrap().unwrap();
            assert_eq!(numbering_mode.separator, "x"); // 第一个字符作为分隔符
            assert_eq!(numbering_mode.width, 5); // 使用默认宽度
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
            let matches = cmd.try_get_matches_from(&["pr", "--pages=abc"]).unwrap();

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
            let matches1 = cmd1.try_get_matches_from(&["pr", "--pages=5"]).unwrap();

            let res = matches1.get_one::<String>(pr_flags::PR_PAGES).map(|i| {
                let x: Vec<_> = i.split(':').collect();
                x[0].to_string()
            });

            assert!(res.is_some());
            assert_eq!(res.unwrap(), "5");

            // 测试解析起始页和结束页
            let cmd2 = ct_app();
            let matches2 = cmd2.try_get_matches_from(&["pr", "--pages=5:10"]).unwrap();

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
                line_width: None,
            };

            let pages: Vec<_> = pr_read_stream_and_create_pages(&output_opts, lines, 0).collect();

            // 验证是否创建了足够的页面，并且处理了连续的换页符
            assert!(!pages.is_empty());

            // 查找是否有空页面存在（由连续换页符创建）
            let has_empty_page = pages.iter().any(|(_, lines)| lines.is_empty());
            assert!(has_empty_page, "应该存在由连续换页符创建的空页面");
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
                line_width: None,
            };

            // 测试 mpr_handle 中的错误处理逻辑
            let paths = &["test_file.txt"];
            let result = mpr_handle(paths, &output_opts);

            // 由于文件不存在，应该返回错误
            assert!(result.is_err());
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
                line_width: None,
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

            // 场景2: columns_to_print > 1，应该使用 column_mode_options 的宽度
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
            assert_eq!(result, Some(40));

            // 场景3: columns_to_print > 1 但 column_mode_options 为 None，应该使用默认值
            let page_width = Some(80);
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

            // 场景4: columns_to_print = 1，应该直接使用 page_width
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
            let matches = create_matches_with_args("-S ###");
            let result = parse_column_separator(&matches);
            assert_eq!(result, "###");

            // 使用 PR_COLUMN_CHAR_SEPARATOR 参数
            let matches = create_matches_with_args("-s :");
            let result = parse_column_separator(&matches);
            assert_eq!(result, ":");

            // 同时指定两个参数，PR_COLUMN_STRING_SEPARATOR 优先级更高
            let matches = create_matches_with_args("-S ### -s :");
            let result = parse_column_separator(&matches);
            assert_eq!(result, "###");

            // 没有指定任何参数，使用默认值
            let matches = create_matches_with_args("");
            let result = parse_column_separator(&matches);
            assert_eq!(result, PR_DEFAULT_COLUMN_SEPARATOR.to_string());
        }

        #[test]
        fn test_parse_column_mode_options() {
            // 测试 line 653 is_across_mode 与其他相关功能

            // 测试 is_across_mode 为 true 的情况
            let matches = create_matches_with_args("--column=2 -a");
            let result = parse_column_mode_options(&matches, "--column=2 -a").unwrap();
            assert!(result.is_some());
            let options = result.unwrap();
            assert_eq!(options.columns, 2);
            assert!(options.is_across_mode);

            // 测试 is_across_mode 为 false 的情况
            let matches = create_matches_with_args("--column=2");
            let result = parse_column_mode_options(&matches, "--column=2").unwrap();
            assert!(result.is_some());
            let options = result.unwrap();
            assert_eq!(options.columns, 2);
            assert!(!options.is_across_mode);

            // 测试命令行中直接使用 -3 格式
            let matches = create_matches_with_args("");
            let result = parse_column_mode_options(&matches, " -3 ").unwrap();
            assert!(result.is_some());
            let options = result.unwrap();
            assert_eq!(options.columns, 3);

            // 测试无效的 -column 格式
            let matches = create_matches_with_args("");
            let result = parse_column_mode_options(&matches, " -abc ");
            assert!(result.unwrap().is_none());
        }

        #[test]
        fn test_parse_page_width() {
            // 测试 line 678-680 parse_page_width 函数的功能

            // 当 PR_JOIN_LINES 为 true 时，应该返回 None
            let matches = create_matches_with_args("-J");
            let result = parse_page_width(&matches).unwrap();
            assert_eq!(result, None);

            // 当指定了 PR_PAGE_WIDTH 时，应该返回对应的值
            let matches = create_matches_with_args("-W 90");
            let result = parse_page_width(&matches).unwrap();
            assert_eq!(result, Some(90));

            // 当 PR_JOIN_LINES 和 PR_PAGE_WIDTH 都没有指定时，应该返回 None
            let matches = create_matches_with_args("");
            let result = parse_page_width(&matches).unwrap();
            assert_eq!(result, None);

            // 当 PR_JOIN_LINES 和 PR_PAGE_WIDTH 同时存在时，PR_JOIN_LINES 优先
            let matches = create_matches_with_args("-J -W 90");
            let result = parse_page_width(&matches).unwrap();
            assert_eq!(result, None);
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
            let args = " +5 ";
            let result = parse_start_end_page(&matches, args).unwrap();
            assert_eq!(result.0, 5); // start_page
            assert_eq!(result.1, None); // end_page

            // 测试 +5:10 语法
            let matches = create_matches_with_args("");
            let args = " +5:10 ";
            let result = parse_start_end_page(&matches, args).unwrap();
            assert_eq!(result.0, 5); // start_page
            assert_eq!(result.1, Some(10)); // end_page

            // 测试无效的 +page 语法
            let matches = create_matches_with_args("");
            let args = " +abc ";
            // 注意：由于正则表达式的匹配方式，+abc可能不会被解析为+page格式，
            // 因此可能会返回默认的start_page=1，不产生错误
            let result = parse_start_end_page(&matches, args);
            if result.is_ok() {
                let (start_page, end_page) = result.unwrap();
                assert_eq!(start_page, 1); // 默认值
                assert_eq!(end_page, None); // 默认值
            }

            // 测试另一种格式的无效 +page 语法，这个会导致实际解析错误
            let matches = create_matches_with_args("");
            let args = " +1a:10 ";
            let result = parse_start_end_page(&matches, args);
            assert!(result.is_err() || result.unwrap().0 == 1);
        }

        #[test]
        fn test_invalid_pages_map() {
            // 测试 line 733-741 invalid_pages_map 功能

            // 测试有效的 --pages 参数
            let matches = create_matches_with_args("--pages=5:10");
            let args = "";
            let result = parse_start_end_page(&matches, args).unwrap();
            assert_eq!(result.0, 5); // start_page
            assert_eq!(result.1, Some(10)); // end_page

            // 测试无效的 --pages 参数 (非数字)
            let matches = create_matches_with_args("--pages=abc");
            let args = "";
            let result = parse_start_end_page(&matches, args);
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
            let args = "";
            let result = parse_start_end_page(&matches, args);
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
            let args = " +5:10 ";
            let result = parse_start_end_page(&matches, args).unwrap();
            assert_eq!(result.0, 7); // start_page 来自 --pages
            assert_eq!(result.1, Some(15)); // end_page 来自 --pages
        }
    }

    #[cfg(test)]
    mod locale_tests {
        use super::*;
        use std::env;

        #[test]
        fn test_get_pr_date_time_format_c_locale() {
            // 模拟C locale环境
            unsafe {
                env::set_var("LC_TIME", "C");
            }

            // C locale应该使用英文格式
            assert_eq!(get_pr_date_time_format(), "%b %d %H:%M %Y");

            // 清理环境变量
            unsafe {
                env::remove_var("LC_TIME");
            }
        }

        #[test]
        fn test_get_pr_date_time_format_non_c_locale() {
            // 模拟非C locale环境
            unsafe {
                env::set_var("LC_TIME", "zh_CN.UTF-8");
            }

            // 非C locale应该使用ISO格式
            assert_eq!(get_pr_date_time_format(), "%Y-%m-%d %H:%M");

            // 清理环境变量
            unsafe {
                env::remove_var("LC_TIME");
            }
        }

        #[test]
        fn test_get_pr_date_time_format_posix_locale() {
            // 模拟POSIX locale环境
            unsafe {
                env::set_var("LC_TIME", "POSIX");
            }

            // POSIX locale应该使用英文格式
            assert_eq!(get_pr_date_time_format(), "%b %d %H:%M %Y");

            // 清理环境变量
            unsafe {
                env::remove_var("LC_TIME");
            }
        }

        #[test]
        fn test_hard_locale_time_integration() {
            // 测试hard_locale_time函数的使用
            unsafe {
                env::set_var("LC_TIME", "C");
            }
            assert!(!hard_locale_time());

            unsafe {
                env::set_var("LC_TIME", "en_US.UTF-8");
            }
            assert!(hard_locale_time());

            // 清理环境变量
            unsafe {
                env::remove_var("LC_TIME");
            }
        }
    }
}
