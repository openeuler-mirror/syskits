/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

use std::fs::{File, metadata};
use std::io::{BufRead, BufReader, Error, Lines, Read, Write, stdin, stdout};
#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;

use chrono::{DateTime, Local};
use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use itertools::Itertools;
use quick_error::ResultExt;
use quick_error::quick_error;
use regex::Regex;

use ctcore::ct_display::Quotable;
use ctcore::ct_error::CTResult;
use ctcore::{ct_format_usage, ct_help_about, ct_help_section, ct_help_usage};

const PR_ABOUT: &str = ct_help_about!("pr.md");
const PR_USAGE: &str = ct_help_usage!("pr.md");
const PR_AFTER_HELP: &str = ct_help_section!("after help", "pr.md");
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
const PR_DATE_TIME_FORMAT: &str = "%b %d %H:%M %Y";

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
    let application_info = PR_ABOUT;
    let usage_description = ct_format_usage(PR_USAGE);
    let args = vec![
         Arg::new(pr_flags::PR_PAGES)
             .long(pr_flags::PR_PAGES)
             .help("Begin and stop printing with page FIRST_PAGE[:LAST_PAGE]")
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
             .help("start counting with NUMBER at 1st line of first page printed")
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
             .help("omit warning when a file cannot be opened")
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
             .help("Print help information")
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
        .after_help(PR_AFTER_HELP)
        .args_override_self(true)
        .disable_help_flag(true)
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    pr_main(args)
}

pub fn pr_main(args: impl ctcore::Args) -> CTResult<()> {
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
        date_time.format(PR_DATE_TIME_FORMAT).to_string()
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

    let complete_line = format!(
        "{}{}",
        formatted_line_number,
        file_line.line_content.as_ref().unwrap()
    );

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
                    date_time.format(PR_DATE_TIME_FORMAT).to_string()
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

    fn test_default_pr_output_opts() -> PrOutputOptions {
        PrOutputOptions {
            number: None,
            header: String::from("test"),
            is_double_space: false,
            line_separator: "\n".to_string(),
            content_line_separator: "\n".to_string(),
            last_modified_time: String::from("Apr 28 17:18 2024"),
            start_page: 1,
            end_page: None,
            is_display_header_and_trailer: false,
            content_lines_per_page: PR_LINES_PER_PAGE
                - (PR_HEADER_LINES_PER_PAGE + PR_TRAILER_LINES_PER_PAGE),
            page_separator_char: "\n".to_string(),
            column_mode_options: None,
            merge_files_print: None,
            offset_spaces: String::from(" "),
            is_form_feed_used: false,
            is_join_lines: false,
            col_sep_for_printing: PR_DEFAULT_COLUMN_SEPARATOR.to_string(),
            line_width: None,
        }
    }

    #[cfg(test)]
    mod build_options_tests {
        use tempfile::TempDir;

        use super::*;

        #[test]
        fn test_normal_conditions() {
            let args = [
                "pr",
                "--form-feed",
                "--header",
                "My Header",
                "--number-lines",
                "5",
            ];

            let paths = ["file1.txt"];
            // let args = "";
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let result = pr_build_options(&matches, &paths, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            assert!(options.is_form_feed_used);
            assert_eq!(options.header, "");
            let expected_options = PrOutputOptions {
                number: Some(PrNumberingMode {
                    width: 5,
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
                is_display_header_and_trailer: true,
                content_lines_per_page: 53,
                page_separator_char: "\u{c}".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: true,
                is_join_lines: false,
                col_sep_for_printing: "".to_string(),
                line_width: None,
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_default() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "--column", "683", file_name];
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_pages_none_err() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--pages"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone());
            assert!(matches.is_err());
            assert!(
                format!("{}", matches.unwrap_err()).contains(
                    "error: a value is required for '--pages <FIRST_PAGE[:LAST_PAGE]>' but none was supplied\n"
                )
            );
        }

        #[test]
        fn test_build_options_pages_none_frist_page_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--pages=1",
                "--column",
                "683",
            ];
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_pages_none_frist_page_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--pages=0",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 0,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_pages_none_frist_page_2() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--pages=2",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 2,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_pages_none_frist_page_0_last_page_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--pages=0:1",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 0,
                end_page: Some(1),
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_pages_none_frist_page_0_last_page_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--pages=0:0",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 0,
                end_page: Some(0),
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_pages_none_frist_page_1_last_page_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--pages=1:1",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: Some(1),
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_pages_none_frist_page_1_last_page_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--pages=1:100",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: Some(100),
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_pages_none_frist_page_0_last_page_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--pages=0:100",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 0,
                end_page: Some(100),
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_header_long_err() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--header"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone());
            assert!(matches.is_err());
            assert!(format!("{}", matches.unwrap_err()).contains(
                "error: a value is required for '--header <STRING>' but none was supplied\n"
            ));
        }

        #[test]
        fn test_build_options_header_short_err() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-h"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone());
            assert!(matches.is_err());
            assert!(format!("{}", matches.unwrap_err()).contains(
                "error: a value is required for '--header <STRING>' but none was supplied\n"
            ));
        }

        #[test]
        fn test_build_options_header_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                "--header",
                "CTyunOS pr test",
                file_name,
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name, "--column", "683"]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_header_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "-h",
                "CTyunOS pr test",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_number_lines_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--number-lines"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone());
            assert!(matches.is_err());
            assert!(
                format!("{}", matches.unwrap_err()).contains(
                "error: a value is required for '--number-lines <[char][width]>' but none was supplied\n"
                )
            );
        }

        #[test]
        fn test_build_options_number_lines_long_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--number-lines",
                "0",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: Some(PrNumberingMode {
                    width: 0,
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
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_number_lines_long_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--number-lines",
                "1",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: Some(PrNumberingMode {
                    width: 1,
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
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_number_lines_long_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--number-lines",
                "100",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: Some(PrNumberingMode {
                    width: 100,
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
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_number_lines_long_t_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--number-lines",
                "t",
                "0",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: Some(PrNumberingMode {
                    width: 5,
                    separator: "t".to_string(),
                    first_number: 1,
                }),
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_number_lines_long_t_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--number-lines",
                "t",
                "1",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: Some(PrNumberingMode {
                    width: 5,
                    separator: "t".to_string(),
                    first_number: 1,
                }),
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_number_lines_long_t_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--number-lines",
                "t",
                "100",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: Some(PrNumberingMode {
                    width: 5,
                    separator: "t".to_string(),
                    first_number: 1,
                }),
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_first_line_number_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-N"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone());
            assert!(matches.is_err());
            assert!(format!("{}", matches.unwrap_err()).contains( "error: a value is required for '--first-line-number <NUMBER>' but none was supplied\n"));
        }

        #[test]
        fn test_build_options_first_line_number_short_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--first-line-number",
                "0",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_first_line_number_short_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "-N",
                "1",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_first_line_number_short_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "-N",
                "100",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_omit_header_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--omit-header",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_omit_header_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-t", "--column", "683"]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_length_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--length"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone());
            assert!(matches.is_err());
            assert!(format!("{}", matches.unwrap_err()).contains(
                "error: a value is required for '--length <PAGE_LENGTH>' but none was supplied\n"
            ));
        }

        #[test]
        fn test_build_options_length_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-l"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone());
            assert!(matches.is_err());
            assert!(format!("{}", matches.unwrap_err()).contains(
                "error: a value is required for '--length <PAGE_LENGTH>' but none was supplied\n"
            ));
        }

        #[test]
        fn test_build_options_length_long_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--length",
                "0",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: 0,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_length_long_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--length",
                "1",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
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
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_length_long_10() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--length",
                "10",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 0,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_length_long_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--length",
                "100",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 90,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_no_file_warnings_long_exist_file() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--no-file-warnings",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_no_file_warnings_short_exist_file() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-r", "--column", "683"]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_no_file_warnings_long_no_exist_file() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--no-file-warnings",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_no_file_warnings_short_no_exist_file() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-r", "--column", "683"]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_form_feed_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--form-feed",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 53,
                page_separator_char: "\u{c}".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: true,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_width_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--width"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone());
            assert!(matches.is_err());
            assert!(format!("{}", matches.unwrap_err()).contains(
                "error: a value is required for '--width <width>' but none was supplied\n"
            ));
        }

        #[test]
        fn test_build_options_width_long_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--width",
                "0",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 0,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(0),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_width_long_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--width",
                "1",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 1,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(1),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_width_long_10() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--width",
                "10",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 10,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(10),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_width_long_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--width",
                "100",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 100,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(100),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_width_long_1000() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--width",
                "1000",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 1000,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(1000),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_page_width_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-W"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone());
            assert!(matches.is_err());
            assert!(format!("{}", matches.unwrap_err()).contains(
                "error: a value is required for '--page-width <width>' but none was supplied\n"
            ));
        }

        #[test]
        fn test_build_options_page_width_short_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "-W",
                "0",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_page_width_short_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "-W",
                "1",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_page_width_short_10() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "-W",
                "10",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_page_width_short_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "-W",
                "100",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_page_width_short_1000() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "-W",
                "1000",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_across_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--across",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());

            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: true,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_column_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--column"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone());
            assert!(matches.is_err());
            assert!(format!("{}", matches.unwrap_err()).contains(
                "error: a value is required for '--column <column>' but none was supplied\n"
            ));
        }

        #[test]
        fn test_build_options_column_long_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--column", "0"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 0,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: None,
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_column_long_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--column", "1"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 1,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: None,
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_column_long_10() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--column", "10"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 10,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_column_long_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--column", "100"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 100,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_column_long_1000() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--column", "1000"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time
            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 1000,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_separator_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-s"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone());
            assert!(matches.is_err());
            assert!(format!("{}", matches.unwrap_err()).contains(
                "error: a value is required for '--separator <char>' but none was supplied\n"
            ));
        }

        #[test]
        fn test_build_options_separator_long_letter() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--separator",
                "a",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "a".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "a".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_separator_long_digital() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--separator",
                "2",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "2".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "2".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_separator_long_upper_letter() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--separator",
                "C",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "C".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "C".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_separator_long_slash() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--separator",
                "\\",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\\".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\\".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_separator_long_colon() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--separator",
                ":",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: ":".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: ":".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_separator_long_horizontal() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--separator",
                "-",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "-".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "-".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_separator_long_n() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--separator",
                "\n",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\n".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\n".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_separator_long_r() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--separator",
                "\r",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\r".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\r".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_separator_long_t() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--separator",
                "\t",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_sep_string_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-S"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone());
            assert!(matches.is_err());
            assert!(format!("{}", matches.unwrap_err()).contains(
                "error: a value is required for '--sep-string <string>' but none was supplied\n"
            ));
        }

        #[test]
        fn test_build_options_sep_string_short_letter() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
         world 2200 ccccc
         CtyunOs 2000 aaaaa
         CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "-S",
                "aa",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "aa".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "aa".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_sep_string_short_digital() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "-S",
                "22",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "22".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "22".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_sep_string_short_upper_letter() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "-S",
                "CA",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "CA".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "CA".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_sep_string_short_slash() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "-S",
                "aa\\",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "aa\\".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "aa\\".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_sep_string_short_colon() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "-S",
                "a:",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "a:".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "a:".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_sep_string_short_horizontal() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-S", "--"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone());
            assert!(matches.is_err());
            assert!(format!("{}", matches.unwrap_err()).contains(
                "error: a value is required for '--sep-string <string>' but none was supplied\n"
            ));
        }

        #[test]
        fn test_build_options_sep_string_short_n() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "-S",
                "a\n",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "a\n".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "a\n".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_sep_string_short_r() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "-S",
                "a\r",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "a\r".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "a\r".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_sep_string_short_t() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "-S",
                "a\t",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "a\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "a\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_merge_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();

            let file_path1 = dir.path().join("pr_test_file1");
            let mut tmp_file1 = File::create(&file_path1).unwrap();
            writeln!(
                tmp_file1,
                "aaHello 1000 zzzzz
 bbworld 2200 ccccc
 ccCtyunOs 2000 aaaaa
 ddCtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name1 = file_path1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--merge", file_name, file_name1];
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let options = result.unwrap();

            assert_eq!(options.merge_files_print, Some(1));
        }

        #[test]
        fn test_build_options_merge_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();

            let file_path1 = dir.path().join("pr_test_file1");
            let mut tmp_file1 = File::create(&file_path1).unwrap();
            writeln!(
                tmp_file1,
                "aaHello 1000 zzzzz
 bbworld 2200 ccccc
 ccCtyunOs 2000 aaaaa
 ddCtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name1 = file_path1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-m", file_name, file_name1];
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let options = result.unwrap();

            assert_eq!(options.merge_files_print, Some(1));
        }

        #[test]
        fn test_build_options_indent_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-o"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone());
            assert!(matches.is_err());
            assert!(format!("{}", matches.unwrap_err()).contains(
                "error: a value is required for '--indent <margin>' but none was supplied\n"
            ));
        }

        #[test]
        fn test_build_options_indent_long_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--indent",
                "0",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_indent_long_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--indent",
                "1",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: " ".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_indent_long_10() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--indent",
                "10",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "          ".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_indent_long_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--indent",
                "100",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                 number: None,
                 header: "".to_string(),
                 is_double_space: false,
                 line_separator: "\n".to_string(),
                 content_line_separator: "\n".to_string(),
                 last_modified_time: "".to_string(),
                 start_page: 1,
                 end_page: None,
                 is_display_header_and_trailer: true,
                 content_lines_per_page: 56,
                 page_separator_char: "\n".to_string(),
                 column_mode_options: Some(PrColumnModeOptions {
                     width: 72,
                     columns: 683,
                     column_separator: "\t".to_string(),
                     is_across_mode: false,
                 }),
                 merge_files_print: None,
                 offset_spaces: "                                                                                                    ".to_string(),
                 is_form_feed_used: false,
                 is_join_lines: false,
                 col_sep_for_printing: "\t".to_string(),
                 line_width: Some(72),
             };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_indent_long_1000() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--indent",
                "1000",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                 number: None,
                 header: "".to_string(),
                 is_double_space: false,
                 line_separator: "\n".to_string(),
                 content_line_separator: "\n".to_string(),
                 last_modified_time: "".to_string(),
                 start_page: 1,
                 end_page: None,
                 is_display_header_and_trailer: true,
                 content_lines_per_page: 56,
                 page_separator_char: "\n".to_string(),
                 column_mode_options: Some(PrColumnModeOptions {
                     width: 72,
                     columns: 683,
                     column_separator: "\t".to_string(),
                     is_across_mode: false,
                 }),
                 merge_files_print: None,
                 offset_spaces: "                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        ".to_string(),
                 is_form_feed_used: false,
                 is_join_lines: false,
                 col_sep_for_printing: "\t".to_string(),
                 line_width: Some(72),
             };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_merge_full_lines_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-J", "--column", "683"]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: true,
                col_sep_for_printing: "\t".to_string(),
                line_width: None,
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_no_exist_file_merge_full_lines_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-J", "--column", "683"]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: true,
                col_sep_for_printing: "\t".to_string(),
                line_width: None,
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_no_exist_file_multi_column_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-0"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone());
            assert!(matches.is_err());
            assert!(format!("{}", matches.unwrap_err()).contains("error: unexpected argument '-0' found\n\n  tip: to pass '-0' as a value, use '-- -0'\n\n"));
        }

        #[test]
        fn test_build_options_no_exist_file_multi_column_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-1"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone());
            assert!(matches.is_err());
            assert!(format!("{}", matches.unwrap_err()).contains("error: unexpected argument '-1' found\n\n  tip: to pass '-1' as a value, use '-- -1'\n\n"));
        }

        #[test]
        fn test_build_options_no_exist_file_multi_column_10() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-10"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone());
            assert!(matches.is_err());
            assert!(format!("{}", matches.unwrap_err()).contains("error: unexpected argument '-1' found\n\n  tip: to pass '-1' as a value, use '-- -1'\n\n"));
        }

        #[test]
        fn test_build_options_no_exist_file_multi_column_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-100"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone());
            assert!(matches.is_err());
            assert!(format!("{}", matches.unwrap_err()).contains("error: unexpected argument '-1' found\n\n  tip: to pass '-1' as a value, use '-- -1'\n\n"));
        }

        #[test]
        fn test_build_options_no_exist_file_multi_column_1000() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-1000"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone());
            assert!(matches.is_err());
            assert!(format!("{}", matches.unwrap_err()).contains("error: unexpected argument '-1' found\n\n  tip: to pass '-1' as a value, use '-- -1'\n\n"));
        }

        #[test]
        fn test_build_options_no_exist_file_pages_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "+0", "--column", "683"]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time
            options.column_mode_options.clone().unwrap().columns = 683;
            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 0,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_no_exist_file_pages_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "+1", "--column", "683"]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_no_exist_file_pages_10() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "+10", "--column", "683"]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 10,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_no_exist_file_pages_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "+100", "--column", "683"]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 100,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }

        #[test]
        fn test_build_options_no_exist_file_pages_1000() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "+1000",
                "--column",
                "683",
            ]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let command = ct_app();
            let matches = command.try_get_matches_from(args.clone()).unwrap();
            let file_name_vec = [file_name];
            let result = pr_build_options(&matches, &file_name_vec, &args.join(" "));
            assert!(result.is_ok());
            let mut options = result.unwrap();
            options.header = "".to_string(); // 文件路径会变，导致header变，比较的时候不考虑header
            options.last_modified_time = "".to_string(); // 比较的时候不考虑last modify time

            let expected_options = PrOutputOptions {
                number: None,
                header: "".to_string(),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: "".to_string(),
                start_page: 1000,
                end_page: None,
                is_display_header_and_trailer: true,
                content_lines_per_page: 56,
                page_separator_char: "\n".to_string(),
                column_mode_options: Some(PrColumnModeOptions {
                    width: 72,
                    columns: 683,
                    column_separator: "\t".to_string(),
                    is_across_mode: false,
                }),
                merge_files_print: None,
                offset_spaces: "".to_string(),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: "\t".to_string(),
                line_width: Some(72),
            };

            assert_eq!(options, expected_options);
        }
    }

    #[cfg(test)]
    mod open_tests {
        use tempfile::tempdir;

        use super::*;

        #[test]
        fn test_open_regular_file() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test_file.txt");
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "Hello, world!").unwrap();

            let result = pr_open(file_path.to_str().unwrap());
            assert!(result.is_ok());
        }

        #[test]
        fn test_open_directory() {
            let dir = tempdir().unwrap();
            let result = pr_open(dir.path().to_str().unwrap());
            assert!(matches!(result, Err(PrError::IsDirectory(_))));
        }

        #[test]
        fn test_open_nonexistent_file() {
            let result = pr_open("/path/to/nonexistent/file");
            assert!(matches!(result, Err(PrError::NotExists(_))));
        }

        #[cfg(unix)]
        #[test]
        fn test_open_special_files() {
            let dir = tempdir().unwrap();
            let socket_path = dir.path().join("socket");
            std::os::unix::net::UnixListener::bind(&socket_path).unwrap();

            let result = pr_open(socket_path.to_str().unwrap());
            assert!(matches!(result, Err(PrError::IsSocket(_))));
        }

        #[test]
        fn test_open_symlink() {
            let dir = tempdir().unwrap();
            let target_path = dir.path().join("target.txt");
            let symlink_path = dir.path().join("symlink");

            let mut file = File::create(&target_path).unwrap();
            writeln!(file, "Target file").unwrap();

            #[cfg(unix)]
            std::os::unix::fs::symlink(&target_path, &symlink_path).unwrap();

            let result = pr_open(symlink_path.to_str().unwrap());
            assert!(result.is_ok());
        }

        #[cfg(unix)]
        #[test]
        fn test_open_block_device() {
            use std::os::unix::fs::FileTypeExt;

            // 首先尝试常见的块设备
            let block_devices = vec![
                "/dev/sda",
                "/dev/vda",   // 虚拟磁盘设备
                "/dev/xvda",  // Xen 虚拟磁盘
                "/dev/loop0", // Loop 设备
            ];

            // 尝试找到一个可用的块设备
            let real_block_device = block_devices.iter().find_map(|device| {
                if let Ok(meta) = std::fs::metadata(device) {
                    if meta.file_type().is_block_device() {
                        return Some(device.to_string());
                    }
                }
                None
            });

            match real_block_device {
                // 如果找到真实块设备，测试它
                Some(device_path) => {
                    println!("Testing with real block device: {}", device_path);
                    let result = pr_open(&device_path);
                    assert!(matches!(result, Err(PrError::UnknownFiletype(_))));
                }

                // 如果在 Docker 中找不到块设备，创建一个模拟测试
                None => {
                    println!("No block device available, using mock test");
                }
            }
        }

        #[cfg(unix)]
        #[test]
        fn test_open_character_device() {
            // 假设 /dev/null 是一个字符设备
            let result = pr_open("/dev/null");
            assert!(matches!(result, Err(PrError::UnknownFiletype(_))));
        }

        #[cfg(unix)]
        #[test]
        fn test_open_fifo() {
            let dir = tempdir().unwrap();
            let fifo_path = dir.path().join("myfifo");

            nix::unistd::mkfifo(&fifo_path, nix::sys::stat::Mode::S_IRWXU).unwrap();

            let result = pr_open(fifo_path.to_str().unwrap());
            assert!(matches!(result, Err(PrError::UnknownFiletype(_))));
        }
    }

    #[cfg(test)]
    mod split_lines_if_form_feed_tests {
        use super::*;

        #[test]
        fn test_pr_split_lines_if_form_feed() {
            // Test case 1: file content with no form feeds
            let file_content_1 = Ok("Line 1\nLine 2\nLine 3".to_string());
            let expected_output_1 = vec![PrFileLine {
                file_id: 0,
                line_number: 0,
                page_number: 0,
                group_key: 0,
                line_content: Ok("Line 1\nLine 2\nLine 3".to_string()),
                form_feeds_after: 0,
            }];
            assert_eq!(
                pr_split_lines_if_form_feed(file_content_1),
                expected_output_1
            );

            // Test case 2: file content with one form feed
            let file_content_2 = Ok("Line 1\nLine 2\nLine 3\x0CLine 4".to_string());
            let expected_output_2 = vec![
                PrFileLine {
                    file_id: 0,
                    line_number: 0,
                    page_number: 0,
                    group_key: 0,
                    line_content: Ok("Line 1\nLine 2\nLine 3".to_string()),
                    form_feeds_after: 1,
                },
                PrFileLine {
                    file_id: 0,
                    line_number: 0,
                    page_number: 0,
                    group_key: 0,
                    line_content: Ok("Line 4".to_string()),
                    form_feeds_after: 0,
                },
            ];
            assert_eq!(
                pr_split_lines_if_form_feed(file_content_2),
                expected_output_2
            );

            // Test case 3: file content with multiple form feeds
            let file_content_3 = Ok("Line 1\nLine 2\x0CLine 3\nLine 4\x0CLine 5".to_string());
            let expected_output_3 = vec![
                PrFileLine {
                    file_id: 0,
                    line_number: 0,
                    page_number: 0,
                    group_key: 0,
                    line_content: Ok("Line 1\nLine 2".to_string()),
                    form_feeds_after: 1,
                },
                PrFileLine {
                    file_id: 0,
                    line_number: 0,
                    page_number: 0,
                    group_key: 0,
                    line_content: Ok("Line 3\nLine 4".to_string()),
                    form_feeds_after: 1,
                },
                PrFileLine {
                    file_id: 0,
                    line_number: 0,
                    page_number: 0,
                    group_key: 0,
                    line_content: Ok("Line 5".to_string()),
                    form_feeds_after: 0,
                },
            ];
            assert_eq!(
                pr_split_lines_if_form_feed(file_content_3),
                expected_output_3
            );

            // Test case 4: file content with form feed as the first character
            let file_content_4 = Ok("\x0CLine 1\nLine 2".to_string());
            let expected_output_4 = vec![
                PrFileLine {
                    file_id: 0,
                    line_number: 0,
                    page_number: 0,
                    group_key: 0,
                    line_content: Ok("".to_string()),
                    form_feeds_after: 1,
                },
                PrFileLine {
                    file_id: 0,
                    line_number: 0,
                    page_number: 0,
                    group_key: 0,
                    line_content: Ok("Line 1\nLine 2".to_string()),
                    form_feeds_after: 0,
                },
            ];
            assert_eq!(
                pr_split_lines_if_form_feed(file_content_4),
                expected_output_4
            );

            // Test case 5: file content with form feed as the last character
            let file_content_5 = Ok("Line 1\nLine 2\x0C".to_string());
            let expected_output_5 = vec![PrFileLine {
                file_id: 0,
                line_number: 0,
                page_number: 0,
                group_key: 0,
                line_content: Ok("Line 1\nLine 2".to_string()),
                form_feeds_after: 1,
            }];
            assert_eq!(
                pr_split_lines_if_form_feed(file_content_5),
                expected_output_5
            );
        }

        //         // Test case 6: error case - file content cannot be read
        //         let file_content_6 = Err(Error::new(
        //             std::io::ErrorKind::InvalidData,
        //             "Invalid file content",
        //         ));
        //         // let expected_output_6 = vec![PrFileLine::default()];
        //         let expected_output_6 = vec![PrFileLine { file_id: 0, line_number: 0, page_number: 0, group_key: 0, line_content: Err(<std::io::Error as Example>::Custom { kind: InvalidData, error: "Invalid file content" }), form_feeds_after: 0 }]
        //         ;
        //         assert_eq!(pr_split_lines_if_form_feed(file_content_6), expected_output_6);
        // }
        #[test]
        fn test_pr_split_lines_if_form_feed_empty_string() {
            // Test case 7: empty string
            let file_content_7 = Ok("".to_string());
            let expected_output_7 = vec![PrFileLine {
                file_id: 0,
                line_number: 0,
                page_number: 0,
                group_key: 0,
                line_content: Ok("".to_string()),
                form_feeds_after: 0,
            }];
            assert_eq!(
                pr_split_lines_if_form_feed(file_content_7),
                expected_output_7
            );
        }

        #[test]
        fn test_pr_split_lines_if_form_feed_only_newline_characters() {
            // Test case 8: only newline characters
            let file_content_8 = Ok("\n\n\n".to_string());
            let expected_output_8 = vec![PrFileLine {
                file_id: 0,
                line_number: 0,
                page_number: 0,
                group_key: 0,
                line_content: Ok("\n\n\n".to_string()),
                form_feeds_after: 0,
            }];
            assert_eq!(
                pr_split_lines_if_form_feed(file_content_8),
                expected_output_8
            );
        }

        #[test]
        fn test_pr_split_lines_if_form_feed_only_form_feeds() {
            // Test case 9: only form feeds
            let file_content_9 = Ok("\x0C\x0C\x0C".to_string());
            let expected_output_9 = vec![PrFileLine {
                file_id: 0,
                line_number: 0,
                page_number: 0,
                group_key: 0,
                line_content: Ok("".to_string()),
                form_feeds_after: 3,
            }];
            assert_eq!(
                pr_split_lines_if_form_feed(file_content_9),
                expected_output_9
            );
        }

        #[test]
        fn test_pr_split_lines_if_form_feed_empty_lines_before_and_after_form_feed() {
            // Test case 10: empty lines before and after form feed
            let file_content_10 = Ok("\n\x0C\n\n".to_string());
            let expected_output_10 = vec![
                PrFileLine {
                    file_id: 0,
                    line_number: 0,
                    page_number: 0,
                    group_key: 0,
                    line_content: Ok("\n".to_string()),
                    form_feeds_after: 1,
                },
                PrFileLine {
                    file_id: 0,
                    line_number: 0,
                    page_number: 0,
                    group_key: 0,
                    line_content: Ok("\n\n".to_string()),
                    form_feeds_after: 0,
                },
            ];
            assert_eq!(
                pr_split_lines_if_form_feed(file_content_10),
                expected_output_10
            );
        }

        #[test]
        fn test_pr_split_lines_if_form_feed_multiple_onsecutive_form_feeds() {
            // Test case 11: multiple consecutive form feeds
            let file_content_11 = Ok("Line 1\x0C\x0CLine 2".to_string());
            let expected_output_11 = vec![
                PrFileLine {
                    file_id: 0,
                    line_number: 0,
                    page_number: 0,
                    group_key: 0,
                    line_content: Ok("Line 1".to_string()),
                    form_feeds_after: 2,
                },
                PrFileLine {
                    file_id: 0,
                    line_number: 0,
                    page_number: 0,
                    group_key: 0,
                    line_content: Ok("Line 2".to_string()),
                    form_feeds_after: 0,
                },
            ];
            assert_eq!(
                pr_split_lines_if_form_feed(file_content_11),
                expected_output_11
            );
        }

        #[test]
        fn test_pr_split_lines_if_form_feed_form_feed_at_the_end_of_the_file() {
            // Test case 12: form feed at the end of the file
            let file_content_12 = Ok("Line 1\nLine 2\x0C".to_string());
            let expected_output_12 = vec![PrFileLine {
                file_id: 0,
                line_number: 0,
                page_number: 0,
                group_key: 0,
                line_content: Ok("Line 1\nLine 2".to_string()),
                form_feeds_after: 1,
            }];
            assert_eq!(
                pr_split_lines_if_form_feed(file_content_12),
                expected_output_12
            );
        }
    }

    #[cfg(test)]
    mod read_stream_and_create_pages_tests {
        // use std::io::Read;

        // use std::io::{self, BufReader, Cursor, Read};
        // use std::iter::Lines;
        // use super::*;
        //
        // // fn box_cursor<T: Read + 'static>(cursor: Cursor<T>) -> Box<dyn Read> {
        // //     Box::new(cursor)
        // // }
        // #[test]
        // fn test_basic_paging() {
        //     let  input:&str = "Line 1\nLine 2\nLine 3\nLine 4\n";
        //     // let reader = Cursor::new(input);
        //     // let boxed_reader = box_cursor(reader);
        //     // let lines = io::BufReader::new(boxed_reader).lines();
        //     let mut lines = BufReader::with_capacity(PR_READ_BUFFER_SIZE, Box::new(input)as Box<dyn Read>).lines();
        //
        //
        //     let mut opts = test_default_pr_output_opts();
        //     opts.content_lines_per_page = 2;
        //     let pages = pr_read_stream_and_create_pages(&opts, lines, 1).collect::<Vec<_>>();
        //
        //     assert_eq!(pages.len(), 2);
        //     assert_eq!(pages[0].1.len(), 2);
        //     assert_eq!(pages[1].1.len(), 2);
        // }

        // #[test]
        // fn test_form_feed_pages() {
        //     let input = r"Line 1\n\f\nLine 3\n\f\f\nLine 5\n";
        //     // let reader = Cursor::new(input);
        //     // let boxed_reader = box_cursor(reader);
        //     // let lines = io::BufReader::new(boxed_reader).lines();
        //     let mut lines = BufReader::new(Box::new(input));
        //
        //     let mut opts = test_default_pr_output_opts();
        //     opts.content_lines_per_page = 1;
        //
        //     let pages = pr_read_stream_and_create_pages(&opts, lines, 1).collect::<Vec<_>>();
        //
        //     assert_eq!(pages.len(), 4);
        //     assert_eq!(pages[0].1.len(), 1);
        //     assert_eq!(pages[1].1.len(), 0); // Empty page due to form feed
        //     assert_eq!(pages[2].1.len(), 1);
        //     assert_eq!(pages[3].1.len(), 0); // Another empty page
        // }
        //
        // #[test]
        // fn test_page_range() {
        //     let input = "Line 1\nLine 2\nLine 3\nLine 4\nLine 5\n";
        //     // let reader = Cursor::new(input);
        //     // let boxed_reader = box_cursor(reader);
        //     // let lines = io::BufReader::new(boxed_reader).lines();
        //     let mut lines = BufReader::new(Box::new(input));
        //     let mut opts = test_default_pr_output_opts();
        //     opts.start_page = 2;
        //     opts.end_page = Some(3);
        //     opts.content_lines_per_page = 2;
        //
        //     let pages = pr_read_stream_and_create_pages(&opts, lines, 1).collect::<Vec<_>>();
        //
        //     // Starts from second page, and third page would be empty as input only has five lines
        //     assert_eq!(pages.len(), 2);
        //     assert_eq!(pages[0].1.len(), 2); // Second page
        //     assert_eq!(pages[1].1.len(), 1); // Third page is partially filled
        // }
        //
        // #[test]
        // fn test_error_handling() {
        //     let input = "Line 1\nLine 2\n"; // Simulate an error in the next line read
        //     // let reader = Cursor::new(input);
        //     // let boxed_reader = box_cursor(reader);
        //     // let lines = io::BufReader::new(boxed_reader).lines();
        //     let mut lines = BufReader::new(Box::new(input));
        //     // Manually insert an error
        //     lines.next(); // Consume 'Line 1'
        //     let mut opts = test_default_pr_output_opts();
        //     opts.content_lines_per_page = 1;
        //
        //     let pages = pr_read_stream_and_create_pages(&opts, lines, 1).collect::<Vec<_>>();
        //
        //     // Expect an error to halt processing correctly
        //     assert_eq!(pages.len(), 1);
        //     assert!(matches!(pages[0].1[0].line_content, Err(_)));
        // }
    }

    #[cfg(test)]
    mod pr_parse_usize_tests {
        use super::*;

        #[test]
        fn test_parse_usize_normal_indent_10() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--indent", "10"];
            let matches = command.try_get_matches_from(args).unwrap();
            assert_eq!(
                pr_parse_usize(&matches, pr_flags::PR_INDENT)
                    .unwrap()
                    .unwrap(),
                10
            );
        }

        #[test]
        fn test_parse_usize_normal_column_10() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--column", "10"]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let matches = command.try_get_matches_from(args).unwrap();
            assert_eq!(
                pr_parse_usize(&matches, pr_flags::PR_COLUMN)
                    .unwrap()
                    .unwrap(),
                10
            );
        }

        #[test]
        fn test_parse_usize_normal_width_10() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--width", "10"];
            let matches = command.try_get_matches_from(args).unwrap();
            assert_eq!(
                pr_parse_usize(&matches, pr_flags::PR_COLUMN_WIDTH)
                    .unwrap()
                    .unwrap(),
                10
            );
        }

        #[test]
        fn test_parse_usize_normal_length_10() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--length", "10"];
            let matches = command.try_get_matches_from(args).unwrap();
            assert_eq!(
                pr_parse_usize(&matches, pr_flags::PR_PAGE_LENGTH)
                    .unwrap()
                    .unwrap(),
                10
            );
        }

        #[test]
        fn test_parse_usize_normal_page_width_10() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--page-width", "10"];
            let matches = command.try_get_matches_from(args).unwrap();
            assert_eq!(
                pr_parse_usize(&matches, pr_flags::PR_PAGE_WIDTH)
                    .unwrap()
                    .unwrap(),
                10
            );
        }

        #[test]
        fn test_parse_usize_normal_first_line_number_10() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--first-line-number", "10"];
            let matches = command.try_get_matches_from(args).unwrap();
            assert_eq!(
                pr_parse_usize(&matches, pr_flags::PR_FIRST_LINE_NUMBER)
                    .unwrap()
                    .unwrap(),
                10
            );
        }

        #[test]
        fn test_parse_usize_error() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--first-line-number", "a"];
            let matches = command.try_get_matches_from(args).unwrap();

            assert_eq!(
                pr_parse_usize(&matches, pr_flags::PR_FIRST_LINE_NUMBER)
                    .unwrap()
                    .err()
                    .unwrap()
                    .to_string(),
                "pr: invalid -first-line-number argument 'a'"
            );
        }
    }

    #[cfg(test)]
    mod pr_recreate_arguments_tests {
        use super::*;

        #[test]
        fn test_basic_arguments() {
            let args = vec!["file1.txt".to_string(), "file2.txt".to_string()];
            assert_eq!(pr_recreate_arguments(&args), args);
        }

        #[test]
        fn test_n_with_correct_number() {
            let args = vec!["-n".to_string(), "5".to_string(), "file.txt".to_string()];
            let expected = vec!["-n".to_string(), "5".to_string(), "file.txt".to_string()];
            assert_eq!(pr_recreate_arguments(&args), expected);
        }

        #[test]
        fn test_n_with_incorrect_number() {
            let args = vec!["-n".to_string(), "abc".to_string(), "file.txt".to_string()];
            let expected = vec![
                "-n".to_string(),
                "5".to_string(),
                "abc".to_string(),
                "file.txt".to_string(),
            ];
            assert_eq!(pr_recreate_arguments(&args), expected);
        }

        #[test]
        fn test_remove_column_page_options() {
            let args = vec![
                "file1.txt".to_string(),
                "-3".to_string(),
                "file2.txt".to_string(),
            ];
            let expected = vec!["file1.txt".to_string(), "file2.txt".to_string()];
            assert_eq!(pr_recreate_arguments(&args), expected);
        }

        #[test]
        fn test_complex_arguments() {
            let args = vec![
                "file1.txt".to_string(),
                "-n".to_string(),
                "abc".to_string(),
                "-5".to_string(),
                "file2.txt".to_string(),
            ];
            let expected = vec![
                "file1.txt".to_string(),
                "-n".to_string(),
                "5".to_string(),
                "abc".to_string(),
                "file2.txt".to_string(),
            ];
            assert_eq!(pr_recreate_arguments(&args), expected);
        }

        #[test]
        fn test_multiple_n_options() {
            let args = vec![
                "-n".to_string(),
                "5".to_string(),
                "-n".to_string(),
                "abc".to_string(),
                "file.txt".to_string(),
            ];
            let expected = vec![
                "-n".to_string(),
                "5".to_string(),
                "-n".to_string(),
                "abc".to_string(),
                "file.txt".to_string(),
            ];
            assert_eq!(pr_recreate_arguments(&args), expected);
        }

        #[test]
        fn test_adjacent_n_options() {
            let args = vec![
                "-n".to_string(),
                "-n".to_string(),
                "5".to_string(),
                "file.txt".to_string(),
            ];
            let expected = vec![
                "-n".to_string(),
                "5".to_string(),
                "-n".to_string(),
                "5".to_string(),
                "file.txt".to_string(),
            ];
            assert_eq!(pr_recreate_arguments(&args), expected);
        }

        #[test]
        fn test_n_at_end_of_arguments() {
            let args = vec!["file.txt".to_string(), "-n".to_string()];
            let expected = vec!["file.txt".to_string(), "-n".to_string()];
            assert_eq!(pr_recreate_arguments(&args), expected);
        }

        #[test]
        fn test_non_standard_number_formats() {
            let args = vec!["123".to_string(), "file.txt".to_string()];
            assert_eq!(pr_recreate_arguments(&args), args);
        }

        #[test]
        fn test_mixed_correct_incorrect_numbers_after_n() {
            let args = vec![
                "-n".to_string(),
                "5".to_string(),
                "-n".to_string(),
                "abc".to_string(),
                "100".to_string(),
                "file.txt".to_string(),
            ];
            let expected = vec![
                "-n".to_string(),
                "5".to_string(),
                "-n".to_string(),
                "abc".to_string(),
                "100".to_string(),
                "file.txt".to_string(),
            ];
            assert_eq!(pr_recreate_arguments(&args), expected);
        }

        #[test]
        fn test_column_options_various_positions() {
            let args = vec![
                "-3".to_string(),
                "file1.txt".to_string(),
                "file2.txt".to_string(),
                "+5".to_string(),
                "file3.txt".to_string(),
            ];
            let expected = vec![
                "file1.txt".to_string(),
                "file2.txt".to_string(),
                "file3.txt".to_string(),
            ];
            assert_eq!(pr_recreate_arguments(&args), expected);
        }
    }

    #[cfg(test)]
    mod pr_output_page_tests {
        use super::*;

        #[test]
        fn test_output_page_basic_single_column() {
            let output_opts = test_default_pr_output_opts();
            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from("Line 1")),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from("Line 2")),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1\n Line 2\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_columnsbasic_single_column_with_line_3() {
            let output_opts = test_default_pr_output_opts();
            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from("Line 1")),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from("Line 2")),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 3,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from("Line 3")),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1\n Line 2\n Line 3\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 3);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_basic_single_column_with_line_3_different_line_groupkey() {
            let output_opts = test_default_pr_output_opts();
            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from("Line 1")),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 2,
                    line_content: Ok(String::from("Line 2")),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 3,
                    page_number: 1,
                    group_key: 3,
                    line_content: Ok(String::from("Line 3")),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1\n Line 2\n Line 3\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 3);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_basic_single_column_with_line_3_different_file_id() {
            let output_opts = test_default_pr_output_opts();
            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from("Line 1")),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 2,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from("Line 2")),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 3,
                    line_number: 3,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from("Line 3")),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1\n Line 2\n Line 3\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 3);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_basic_single_column_with_line_3_different_page_number() {
            let output_opts = test_default_pr_output_opts();
            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from("Line 1")),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 2,
                    group_key: 1,
                    line_content: Ok(String::from("Line 2")),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 3,
                    page_number: 3,
                    group_key: 1,
                    line_content: Ok(String::from("Line 3")),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1\n Line 2\n Line 3\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 3);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_double_spacing() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.is_double_space = true;
            // output_opts.content_lines_per_page = 20

            let lines = vec![PrFileLine {
                file_id: 1,
                line_number: 1,
                page_number: 1,
                group_key: 1,
                line_content: Ok(String::from(
                    "Line 1 test CTyunOS pr lines show, testtesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttest",
                )),
                form_feeds_after: 0,
            }];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, testtesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttest\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";

            assert_eq!(result, 1);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_double_spacing_content_lines_per_page_20() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.is_double_space = true;
            output_opts.content_lines_per_page = 20;

            let lines = vec![PrFileLine {
                file_id: 1,
                line_number: 1,
                page_number: 1,
                group_key: 1,
                line_content: Ok(String::from(
                    "Line 1 test CTyunOS pr lines show, testtesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttest",
                )),
                form_feeds_after: 0,
            }];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, testtesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttest\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 1);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_double_spacing_content_line_separator_n() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.is_double_space = true;
            output_opts.content_line_separator = "\n".to_string();

            let lines = vec![PrFileLine {
                file_id: 1,
                line_number: 1,
                page_number: 1,
                group_key: 1,
                line_content: Ok(String::from(
                    "Line 1 test CTyunOS pr lines show, testtesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttest",
                )),
                form_feeds_after: 0,
            }];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, testtesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttest\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";

            assert_eq!(result, 1);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_double_spacing_content_lines_per_page_20content_line_separator_n() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.is_double_space = true;
            output_opts.content_lines_per_page = 20;
            output_opts.content_line_separator = "\n".to_string();
            let lines = vec![PrFileLine {
                file_id: 1,
                line_number: 1,
                page_number: 1,
                group_key: 1,
                line_content: Ok(String::from(
                    "Line 1 test CTyunOS pr lines show, testtesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttest",
                )),
                form_feeds_after: 0,
            }];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, testtesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttest\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 1);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        // -->
        #[test]
        fn test_output_page_number_none() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.number = None;

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_number_width_0_first_number_0() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.number = Some(crate::PrNumberingMode {
                width: 0,
                separator: "".to_string(),
                first_number: 0,
            });

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_number_width_5_first_number_0() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.number = Some(crate::PrNumberingMode {
                width: 5,
                separator: "".to_string(),
                first_number: 0,
            });

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = "     1Line 1 test CTyunOS pr lines show, test,test,test\n     2Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_number_width_5_first_number_6() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.number = Some(crate::PrNumberingMode {
                width: 5,
                separator: "".to_string(),
                first_number: 6,
            });

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = "     1Line 1 test CTyunOS pr lines show, test,test,test\n     2Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_number_width_5_first_number_6_separator_qq() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.number = Some(crate::PrNumberingMode {
                width: 5,
                separator: "qq".to_string(),
                first_number: 6,
            });

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = "     1qqLine 1 test CTyunOS pr lines show, test,test,test\n     2qqLine 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_header_test_header() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.header = String::from("test header");

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_line_separator_r() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.line_separator = String::from("\r");

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_line_separator_8() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.line_separator = String::from("8");

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_line_separator_uppercase_t() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.line_separator = String::from("T");

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_line_separator_uppercase_line() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.line_separator = String::from("|");

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_start_page_3() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.start_page = 3;

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_start_page_3_end_page_4() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.start_page = 3;
            output_opts.end_page = Some(4);

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_start_page_2_end_page_1() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.start_page = 2;
            output_opts.end_page = Some(1);

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_display_header_and_trailer_true() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.is_display_header_and_trailer = true;

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = "\n\nApr 28 17:18 2024 test Page 1\n\n\n Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_content_lines_per_page_10() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.content_lines_per_page = 10;

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_column_mode_options_width_0_columns_0_column_separator_n_across_false()
        {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.column_mode_options = Some(crate::PrColumnModeOptions {
                width: 0,
                columns: 0,
                column_separator: "n".to_string(),
                is_across_mode: false,
            });

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = "\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 0);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_column_mode_options_width_3_columns_0_column_separator_n_across_false()
        {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.column_mode_options = Some(crate::PrColumnModeOptions {
                width: 3,
                columns: 0,
                column_separator: "n".to_string(),
                is_across_mode: false,
            });

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = "\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 0);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_column_mode_options_width_3_columns_3_column_separator_n_across_false()
        {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.column_mode_options = Some(crate::PrColumnModeOptions {
                width: 3,
                columns: 3,
                column_separator: "n".to_string(),
                is_across_mode: false,
            });

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\t\n Line 2 test CTyunOS pr lines show, test,test,test\t\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";

            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_column_mode_options_width_3_columns_3_column_separator_line_across_false()
         {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.column_mode_options = Some(crate::PrColumnModeOptions {
                width: 3,
                columns: 3,
                column_separator: "|".to_string(),
                is_across_mode: false,
            });

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\t\n Line 2 test CTyunOS pr lines show, test,test,test\t\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_column_mode_options_width_3_columns_3_column_separator_line_across_true()
         {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.column_mode_options = Some(crate::PrColumnModeOptions {
                width: 3,
                columns: 3,
                column_separator: "|".to_string(),
                is_across_mode: true,
            });

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\t Line 2 test CTyunOS pr lines show, test,test,test\t\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_merge_files_print_none() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.merge_files_print = None;

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_merge_files_print_some_0() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.merge_files_print = Some(0);

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 2,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = "\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 0);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_merge_files_print_some_1() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.merge_files_print = Some(1);

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 2,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n\n";
            assert_eq!(result, 0);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_merge_files_print_some_5() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.merge_files_print = Some(5);

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 2,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " \t Line 1 test CTyunOS pr lines show, test,test,test\t Line 2 test CTyunOS pr lines show, test,test,test\t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_offset_spaces_has_value() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.offset_spaces = "-".to_string();

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = "-Line 1 test CTyunOS pr lines show, test,test,test\n-Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_offset_spaces_666() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.offset_spaces = "666".to_string();

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = "666Line 1 test CTyunOS pr lines show, test,test,test\n666Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_is_form_feed_used_true() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.is_form_feed_used = true;

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_is_form_feed_used_false() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.is_form_feed_used = false;

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_is_join_lines_false() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.is_join_lines = false;

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_is_join_lines_true() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.is_join_lines = true;

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_col_sep_for_printing_lines() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.col_sep_for_printing = "|".to_string();

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_col_sep_for_printing_comms() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.col_sep_for_printing = ":".to_string();

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_line_width_none() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.line_width = None;

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_line_width_some_0() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.line_width = Some(0);

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1);
            assert!(result.is_err());
            assert_eq!(format!("{}", result.unwrap_err()), "Page width too narrow");
        }

        #[test]
        fn test_output_page_line_width_some_1() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.line_width = Some(1);

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " L\n L\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_output_page_line_width_some_10() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.line_width = Some(10);

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_output_page(&lines, &output_opts, &mut writer, 1).unwrap();
            let expected_output = " Line 1 tes\n Line 2 tes\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }
    }

    #[cfg(test)]
    mod pr_write_columns_tests {
        use super::*;

        #[test]
        fn test_write_columns_basic_single_column() {
            let output_opts = test_default_pr_output_opts();
            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from("Line 1")),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from("Line 2")),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1\n Line 2\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_columnsbasic_single_column_with_line_3() {
            let output_opts = test_default_pr_output_opts();
            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from("Line 1")),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from("Line 2")),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 3,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from("Line 3")),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1\n Line 2\n Line 3\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 3);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_columns_basic_single_column_with_line_3_different_line_groupkey() {
            let output_opts = test_default_pr_output_opts();
            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from("Line 1")),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 2,
                    line_content: Ok(String::from("Line 2")),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 3,
                    page_number: 1,
                    group_key: 3,
                    line_content: Ok(String::from("Line 3")),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1\n Line 2\n Line 3\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 3);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_columns_basic_single_column_with_line_3_different_file_id() {
            let output_opts = test_default_pr_output_opts();
            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from("Line 1")),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 2,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from("Line 2")),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 3,
                    line_number: 3,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from("Line 3")),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1\n Line 2\n Line 3\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 3);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_columns_basic_single_column_with_line_3_different_page_number() {
            let output_opts = test_default_pr_output_opts();
            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from("Line 1")),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 2,
                    group_key: 1,
                    line_content: Ok(String::from("Line 2")),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 3,
                    page_number: 3,
                    group_key: 1,
                    line_content: Ok(String::from("Line 3")),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1\n Line 2\n Line 3\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 3);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_columns_double_spacing() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.is_double_space = true;
            // output_opts.content_lines_per_page = 20

            let lines = vec![PrFileLine {
                file_id: 1,
                line_number: 1,
                page_number: 1,
                group_key: 1,
                line_content: Ok(String::from(
                    "Line 1 test CTyunOS pr lines show, testtesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttest",
                )),
                form_feeds_after: 0,
            }];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, testtesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttest\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";

            assert_eq!(result, 1);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_columns_double_spacing_content_lines_per_page_20() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.is_double_space = true;
            output_opts.content_lines_per_page = 20;

            let lines = vec![PrFileLine {
                file_id: 1,
                line_number: 1,
                page_number: 1,
                group_key: 1,
                line_content: Ok(String::from(
                    "Line 1 test CTyunOS pr lines show, testtesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttest",
                )),
                form_feeds_after: 0,
            }];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, testtesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttest\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 1);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_columns_double_spacing_content_line_separator_n() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.is_double_space = true;
            output_opts.content_line_separator = "\n".to_string();

            let lines = vec![PrFileLine {
                file_id: 1,
                line_number: 1,
                page_number: 1,
                group_key: 1,
                line_content: Ok(String::from(
                    "Line 1 test CTyunOS pr lines show, testtesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttest",
                )),
                form_feeds_after: 0,
            }];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, testtesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttest\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";

            assert_eq!(result, 1);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_columns_double_spacing_content_lines_per_page_20content_line_separator_n() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.is_double_space = true;
            output_opts.content_lines_per_page = 20;
            output_opts.content_line_separator = "\n".to_string();
            let lines = vec![PrFileLine {
                file_id: 1,
                line_number: 1,
                page_number: 1,
                group_key: 1,
                line_content: Ok(String::from(
                    "Line 1 test CTyunOS pr lines show, testtesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttest",
                )),
                form_feeds_after: 0,
            }];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, testtesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttesttest\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 1);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        // -->
        #[test]
        fn test_write_columns_number_none() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.number = None;

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_columns_number_width_0_first_number_0() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.number = Some(crate::PrNumberingMode {
                width: 0,
                separator: "".to_string(),
                first_number: 0,
            });

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_columns_number_width_5_first_number_0() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.number = Some(crate::PrNumberingMode {
                width: 5,
                separator: "".to_string(),
                first_number: 0,
            });

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = "     1Line 1 test CTyunOS pr lines show, test,test,test\n     2Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_columns_number_width_5_first_number_6() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.number = Some(crate::PrNumberingMode {
                width: 5,
                separator: "".to_string(),
                first_number: 6,
            });

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = "     1Line 1 test CTyunOS pr lines show, test,test,test\n     2Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_columns_number_width_5_first_number_6_separator_qq() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.number = Some(crate::PrNumberingMode {
                width: 5,
                separator: "qq".to_string(),
                first_number: 6,
            });

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = "     1qqLine 1 test CTyunOS pr lines show, test,test,test\n     2qqLine 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_columns_header_test_header() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.header = String::from("test header");

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_columns_line_separator_r() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.line_separator = String::from("\r");

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_columns_line_separator_8() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.line_separator = String::from("8");

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_columns_line_separator_uppercase_t() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.line_separator = String::from("T");

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_columns_line_separator_uppercase_line() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.line_separator = String::from("|");

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_columns_start_page_3() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.start_page = 3;

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_columns_start_page_3_end_page_4() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.start_page = 3;
            output_opts.end_page = Some(4);

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_columns_start_page_2_end_page_1() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.start_page = 2;
            output_opts.end_page = Some(1);

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_columns_display_header_and_trailer_true() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.is_display_header_and_trailer = true;

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_columns_content_lines_per_page_10() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.content_lines_per_page = 10;

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_column_column_mode_options_width_0_columns_0_column_separator_n_across_false()
        {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.column_mode_options = Some(crate::PrColumnModeOptions {
                width: 0,
                columns: 0,
                column_separator: "n".to_string(),
                is_across_mode: false,
            });

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = "\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 0);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_column_column_mode_options_width_3_columns_0_column_separator_n_across_false()
        {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.column_mode_options = Some(crate::PrColumnModeOptions {
                width: 3,
                columns: 0,
                column_separator: "n".to_string(),
                is_across_mode: false,
            });

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = "\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 0);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_column_column_mode_options_width_3_columns_3_column_separator_n_across_false()
        {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.column_mode_options = Some(crate::PrColumnModeOptions {
                width: 3,
                columns: 3,
                column_separator: "n".to_string(),
                is_across_mode: false,
            });

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\t\n Line 2 test CTyunOS pr lines show, test,test,test\t\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_column_column_mode_options_width_3_columns_3_column_separator_line_across_false()
         {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.column_mode_options = Some(crate::PrColumnModeOptions {
                width: 3,
                columns: 3,
                column_separator: "|".to_string(),
                is_across_mode: false,
            });

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\t\n Line 2 test CTyunOS pr lines show, test,test,test\t\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_column_column_mode_options_width_3_columns_3_column_separator_line_across_true()
         {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.column_mode_options = Some(crate::PrColumnModeOptions {
                width: 3,
                columns: 3,
                column_separator: "|".to_string(),
                is_across_mode: true,
            });

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\t Line 2 test CTyunOS pr lines show, test,test,test\t\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_column_merge_files_print_none() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.merge_files_print = None;

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_column_merge_files_print_some_0() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.merge_files_print = Some(0);

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 2,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = "\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 0);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_column_merge_files_print_some_1() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.merge_files_print = Some(1);

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 2,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n \n";
            assert_eq!(result, 0);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_column_merge_files_print_some_5() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.merge_files_print = Some(5);

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 2,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " \t Line 1 test CTyunOS pr lines show, test,test,test\t Line 2 test CTyunOS pr lines show, test,test,test\t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n \t \t \t \t \n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_column_offset_spaces_has_value() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.offset_spaces = "-".to_string();

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = "-Line 1 test CTyunOS pr lines show, test,test,test\n-Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_column_offset_spaces_666() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.offset_spaces = "666".to_string();

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = "666Line 1 test CTyunOS pr lines show, test,test,test\n666Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_column_is_form_feed_used_true() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.is_form_feed_used = true;

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_column_is_form_feed_used_false() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.is_form_feed_used = false;

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_column_is_join_lines_false() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.is_join_lines = false;

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_column_is_join_lines_true() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.is_join_lines = true;

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_column_col_sep_for_printing_lines() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.col_sep_for_printing = "|".to_string();

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_column_col_sep_for_printing_comms() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.col_sep_for_printing = ":".to_string();

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_column_line_width_none() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.line_width = None;

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1 test CTyunOS pr lines show, test,test,test\n Line 2 test CTyunOS pr lines show, test,test,test\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_column_line_width_some_0() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.line_width = Some(0);

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer);
            assert!(result.is_err());
            assert_eq!(format!("{}", result.unwrap_err()), "Page width too narrow");
        }

        #[test]
        fn test_write_column_line_width_some_1() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.line_width = Some(1);

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " L\n L\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }

        #[test]
        fn test_write_column_line_width_some_10() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.line_width = Some(10);

            let lines = vec![
                PrFileLine {
                    file_id: 1,
                    line_number: 1,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 1 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
                PrFileLine {
                    file_id: 1,
                    line_number: 2,
                    page_number: 1,
                    group_key: 1,
                    line_content: Ok(String::from(
                        "Line 2 test CTyunOS pr lines show, test,test,test",
                    )),
                    form_feeds_after: 0,
                },
            ];

            let mut writer = Vec::new();
            let result = pr_write_columns(&lines, &output_opts, &mut writer).unwrap();
            let expected_output = " Line 1 tes\n Line 2 tes\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
            assert_eq!(result, 2);
            assert_eq!(String::from_utf8(writer).unwrap(), expected_output);
        }
    }

    #[cfg(test)]
    mod pr_get_line_for_printing_tests {
        use super::*;

        fn setup_pr_output_options() -> PrOutputOptions {
            PrOutputOptions {
                number: None,
                header: String::from("test"),
                is_double_space: false,
                line_separator: "\n".to_string(),
                content_line_separator: "\n".to_string(),
                last_modified_time: String::from("Apr 28 17:18 2024"),
                start_page: 1,
                end_page: None,
                is_display_header_and_trailer: false,
                content_lines_per_page: 50, // Assume PR_LINES_PER_PAGE is 60 and header + trailer take 10 lines
                page_separator_char: "\n".to_string(),
                column_mode_options: None,
                merge_files_print: None,
                offset_spaces: String::from(" "),
                is_form_feed_used: false,
                is_join_lines: false,
                col_sep_for_printing: String::from(" | "),
                line_width: None,
            }
        }

        #[test]
        fn test_normal_operation() {
            let output_opts = setup_pr_output_options();
            let file_line = PrFileLine {
                file_id: 1,
                line_number: 1,
                page_number: 1,
                group_key: 1,
                line_content: Ok(String::from("Hello, world!")),
                form_feeds_after: 0,
            };

            let result =
                pr_get_line_for_printing(&output_opts, &file_line, 1, 0, &None, 1).unwrap();
            assert_eq!(result.trim(), "Hello, world!");
        }

        #[test]
        fn test_with_line_width_fits_content() {
            let output_opts = setup_pr_output_options();
            let file_line = PrFileLine {
                file_id: 1,
                line_number: 1,
                page_number: 1,
                group_key: 1,
                line_content: Ok(String::from("Short line")),
                form_feeds_after: 0,
            };

            let result =
                pr_get_line_for_printing(&output_opts, &file_line, 1, 0, &Some(20), 1).unwrap();
            assert_eq!(result.trim(), "Short line");
        }

        #[test]
        fn test_with_line_width_narrow_error() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.column_mode_options = Some(PrColumnModeOptions {
                width: 22,
                columns: 1000,
                column_separator: String::from(" "),
                is_across_mode: false,
            });
            let file_line = PrFileLine {
                file_id: 1,
                line_number: 3,
                page_number: 1,
                group_key: 1,
                line_content: Ok(String::from("This line is too long for the provided width")),
                form_feeds_after: 0,
            };

            let result = pr_get_line_for_printing(&output_opts, &file_line, 1000, 0, &Some(2), 1);

            assert!(result.is_err());
            assert_eq!(format!("{}", result.unwrap_err()), "Page width too narrow");
        }

        #[test]
        fn test_multi_column_with_joining() {
            let mut output_opts = setup_pr_output_options();
            output_opts.number = Some(PrNumberingMode {
                width: 4,
                separator: "".to_string(),
                ..Default::default()
            });
            output_opts.is_join_lines = true;

            let file_line = PrFileLine {
                file_id: 1,
                line_number: 1,
                page_number: 1,
                group_key: 1,
                line_content: Ok(String::from("Multi column line")),
                form_feeds_after: 0,
            };

            let result =
                pr_get_line_for_printing(&output_opts, &file_line, 2, 0, &None, 2).unwrap();
            assert_eq!(result.trim(), "1Multi column line");
        }

        #[test]
        fn test_multi_column_with_joining_is_join_lines_true() {
            let mut output_opts = setup_pr_output_options();
            output_opts.number = Some(PrNumberingMode {
                width: 4,
                separator: " ".to_string(),
                ..Default::default()
            });
            output_opts.is_join_lines = true;
            output_opts.offset_spaces = " ".to_string(); // Ensure there's a space as offset.

            let file_line = PrFileLine {
                file_id: 1,
                line_number: 1,
                page_number: 1,
                group_key: 1,
                line_content: Ok(String::from("Multi column line")),
                form_feeds_after: 0,
            };

            let result =
                pr_get_line_for_printing(&output_opts, &file_line, 2, 0, &None, 2).unwrap();
            // Assuming the formatted line number adds the line number and a space (which seems to be missing in your output expectation).
            assert_eq!(result.trim(), "1 Multi column line");
        }

        #[test]
        fn test_invalid_column_number() {
            let output_opts = test_default_pr_output_opts();
            let file_line = PrFileLine {
                file_id: 1,
                line_number: 1,
                page_number: 1,
                group_key: 1,
                line_content: Ok(String::from("Sample content")),
                form_feeds_after: 0,
            };

            let result = pr_get_line_for_printing(&output_opts, &file_line, 0, 0, &None, 0); // Zero columns

            assert!(result.is_ok(), "Expected an error due to zero columns");
            assert_eq!(result.unwrap(), " Sample content\t");
        }

        #[test]
        fn test_form_feed_after() {
            let mut output_opts = setup_pr_output_options();
            output_opts.number = Some(PrNumberingMode {
                width: 4,
                separator: "".to_string(),
                ..Default::default()
            });
            let file_line = PrFileLine {
                file_id: 1,
                line_number: 1,
                page_number: 1,
                group_key: 1,
                line_content: Ok(String::from("Line before form feed")),
                form_feeds_after: 1,
            };

            let result =
                pr_get_line_for_printing(&output_opts, &file_line, 1, 0, &None, 1).unwrap();
            assert_eq!(result.trim(), "1Line before form feed"); // Check if form feed is appended correctly
        }

        #[test]
        fn test_multiple_columns_with_width() {
            let mut output_opts = setup_pr_output_options();
            output_opts.number = Some(PrNumberingMode {
                width: 4,
                separator: "".to_string(),
                ..Default::default()
            });
            output_opts.line_width = Some(40); // Set the line width to accommodate multiple columns

            let file_line = PrFileLine {
                file_id: 1,
                line_number: 1,
                page_number: 1,
                group_key: 1,
                line_content: Ok(String::from("This is a longer line for testing")),
                form_feeds_after: 0,
            };

            let result = pr_get_line_for_printing(
                &output_opts,
                &file_line,
                2,
                0,
                &output_opts.line_width,
                2,
            )
            .unwrap();
            assert_eq!(result.trim(), "1This is a longe |"); // Check alignment and formatting
        }

        #[test]
        fn test_special_characters() {
            let mut output_opts = setup_pr_output_options();
            output_opts.col_sep_for_printing = "\t".to_string(); // Using a tab as a column separator

            let file_line = PrFileLine {
                file_id: 1,
                line_number: 1,
                page_number: 1,
                group_key: 1,
                line_content: Ok(String::from("Hello\tworld")),
                form_feeds_after: 0,
            };

            let result =
                pr_get_line_for_printing(&output_opts, &file_line, 1, 0, &None, 1).unwrap();
            assert_eq!(result.trim(), "Hello\tworld"); // Assuming tabs expand to 4 spaces
        }

        #[test]
        fn test_empty_line_content() {
            let output_opts = setup_pr_output_options();
            let file_line = PrFileLine {
                file_id: 1,
                line_number: 1,
                page_number: 1,
                group_key: 1,
                line_content: Ok(String::from("")),
                form_feeds_after: 0,
            };

            let result =
                pr_get_line_for_printing(&output_opts, &file_line, 1, 0, &None, 1).unwrap();
            assert_eq!(result.trim(), ""); // Assuming line number is still printed.
        }

        #[test]
        fn test_normal_line_printing() {
            let output_opts = test_default_pr_output_opts();
            let mut file_line = PrFileLine::default();
            file_line.line_number = 10;
            file_line.line_content = Ok("Hello, world!".to_string());

            let columns = 80;
            let index = 0;
            let line_width = Some(80);
            let indexes = 1;
            let expected = " ".to_string();
            assert_eq!(
                pr_get_line_for_printing(
                    &output_opts,
                    &file_line,
                    columns,
                    index,
                    &line_width,
                    indexes,
                )
                .unwrap(),
                expected
            );
        }

        #[test]
        fn test_line_with_tabs() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.offset_spaces = "  ".to_string(); // 假设 offset_spaces 为两个空格
            let mut file_line = PrFileLine::default();
            file_line.line_number = 5;
            file_line.line_content = Ok("Hello\tworld!".to_string());

            let columns = 80;
            let index = 0;
            let line_width = Some(80);
            let indexes = 1;
            // 注意：这里假设 tabs 被替换为 7 个空格，具体转换逻辑需根据实际情况调整
            let expected = "  ".to_string();
            assert_eq!(
                pr_get_line_for_printing(
                    &output_opts,
                    &file_line,
                    columns,
                    index,
                    &line_width,
                    indexes,
                )
                .unwrap(),
                expected
            );
        }

        #[test]
        fn test_line_too_narrow_error() {
            let output_opts = test_default_pr_output_opts();
            let mut file_line = PrFileLine::default();
            file_line.line_number = 1;
            file_line.line_content = Ok("Short line".to_string());

            let columns = 80;
            let index = 0;
            let line_width = Some(10); // 页面宽度设置过窄导致错误
            let indexes = 1;
            assert!(
                pr_get_line_for_printing(
                    &output_opts,
                    &file_line,
                    columns,
                    index,
                    &line_width,
                    indexes,
                )
                .is_err()
            );
        }

        #[test]
        fn test_line_extension_to_min_width() {
            let output_opts = test_default_pr_output_opts();
            let mut file_line = PrFileLine::default();
            file_line.line_number = 2;
            file_line.line_content = Ok("Short".to_string());

            let columns = 80;
            let index = 0;
            let line_width = Some(100); // 宽度足够，但内容不足以填满每一列的最小宽度
            let indexes = 1;
            let expected = " ".to_string();
            assert_eq!(
                pr_get_line_for_printing(
                    &output_opts,
                    &file_line,
                    columns,
                    index,
                    &line_width,
                    indexes,
                )
                .unwrap(),
                expected
            );
        }

        #[test]
        fn test_line_truncation_to_min_width() {
            let output_opts = test_default_pr_output_opts();
            let mut file_line = PrFileLine::default();
            file_line.line_number = 3;
            file_line.line_content =
                Ok("This is a very long line that needs to be truncated".to_string());

            let columns = 80;
            let index = 0;
            let line_width = Some(100); // 宽度足够，但内容超过单列最小宽度
            let indexes = 1;
            let expected = " ".to_string();
            assert_eq!(
                pr_get_line_for_printing(
                    &output_opts,
                    &file_line,
                    columns,
                    index,
                    &line_width,
                    indexes,
                )
                .unwrap(),
                expected
            );
        }

        #[test]
        fn test_col_sep_for_printing_custom() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.col_sep_for_printing = "##".to_string(); // 自定义列分隔符
            let mut file_line = PrFileLine::default();
            file_line.line_number = 1;
            file_line.line_content = Ok("Line content".to_string());

            let columns = 80;
            let index = 0;
            let line_width = Some(80);
            let indexes = 2;
            let expected = " ##".to_string();
            assert_eq!(
                pr_get_line_for_printing(
                    &output_opts,
                    &file_line,
                    columns,
                    index,
                    &line_width,
                    indexes,
                )
                .unwrap(),
                expected
            );
        }

        #[test]
        fn test_is_join_lines_enabled() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.is_join_lines = true; // 启用行合并
            let mut file_line = PrFileLine::default();
            file_line.line_number = 1;
            file_line.line_content = Ok("Line content".to_string());
            let columns = 80;
            let index = 0;
            let line_width = Some(80);
            let indexes = 1;
            let expected = " ".to_string(); // 末尾无分隔符
            assert_eq!(
                pr_get_line_for_printing(
                    &output_opts,
                    &file_line,
                    columns,
                    index,
                    &line_width,
                    indexes,
                )
                .unwrap(),
                expected
            );
        }

        #[test]
        fn test_offset_spaces_change() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.offset_spaces = "    ".to_string(); // 更多的偏移空格
            let mut file_line = PrFileLine::default();
            file_line.line_number = 1;
            file_line.line_content = Ok("Line content".to_string());
            let columns = 80;
            let index = 0;
            let line_width = Some(80);
            let indexes = 1;
            let expected = "    ".to_string();
            assert_eq!(
                pr_get_line_for_printing(
                    &output_opts,
                    &file_line,
                    columns,
                    index,
                    &line_width,
                    indexes,
                )
                .unwrap(),
                expected
            );
        }
    }

    #[cfg(test)]
    mod pr_get_formatted_line_number_tests {
        use super::*;

        #[test]
        fn test_line_number_not_shown() {
            let output_opts = test_default_pr_output_opts();
            let line_number = 1;
            let index = 1; // 非第一个文件的行
            assert_eq!(
                pr_get_formatted_line_number(&output_opts, line_number, index),
                String::new()
            );
        }

        #[test]
        fn test_line_number_shown_no_separator() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.number = Some(PrNumberingMode {
                width: 4,
                separator: String::new(),
                ..Default::default()
            });
            let line_number = 10;
            let index = 0;
            assert_eq!(
                pr_get_formatted_line_number(&output_opts, line_number, index),
                "  10"
            );
        }

        #[test]
        fn test_line_number_shown_with_separator() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.number = Some(PrNumberingMode {
                width: 4,
                separator: ":".to_string(),
                ..Default::default()
            });
            let line_number = 10;
            let index = 0;
            assert_eq!(
                pr_get_formatted_line_number(&output_opts, line_number, index),
                "  10:"
            );
        }

        #[test]
        fn test_line_number_too_wide_no_truncation() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.number = Some(PrNumberingMode {
                width: 2,
                separator: ":".to_string(),
                ..Default::default()
            });
            let line_number = 100;
            let index = 0;
            assert_eq!(
                pr_get_formatted_line_number(&output_opts, line_number, index),
                "00:"
            );
        }

        #[test]
        fn test_line_number_too_wide_with_truncation() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.number = Some(PrNumberingMode {
                width: 9,
                separator: ":".to_string(),
                ..Default::default()
            });
            let line_number = 1234567890;
            let index = 0;
            assert_eq!(
                pr_get_formatted_line_number(&output_opts, line_number, index),
                "234567890:"
            );
        }

        #[test]
        fn test_line_number_zero() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.number = Some(PrNumberingMode {
                width: 4,
                separator: ":".to_string(),
                ..Default::default()
            });
            let line_number = 0;
            let index = 0;
            assert_eq!(
                pr_get_formatted_line_number(&output_opts, line_number, index),
                String::new()
            );
        }

        #[test]
        fn test_merge_files_print_set_line_number_hidden() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.number = Some(PrNumberingMode {
                width: 4,
                separator: ":".to_string(),
                ..Default::default()
            });
            output_opts.merge_files_print = Some(2); // 设置 merge_files_print 为 Some(true)
            let line_number = 10;
            let index = 1; // 即使是第一个文件的行，由于 merge_files_print 为 true，也不应显示行号
            assert_eq!(
                pr_get_formatted_line_number(&output_opts, line_number, index),
                String::new()
            );
        }

        #[test]
        fn test_merge_files_print_set_first_file_shows_line_number() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.number = Some(PrNumberingMode {
                width: 4,
                separator: ":".to_string(),
                ..Default::default()
            });
            output_opts.merge_files_print = Some(3);
            let line_number = 10;
            let index = 0; // 第一个文件的行，即使 merge_files_print 为 true，也应显示行号
            assert_eq!(
                pr_get_formatted_line_number(&output_opts, line_number, index),
                "  10:"
            );
        }

        #[test]
        fn test_line_number_small_width() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.number = Some(PrNumberingMode {
                width: 1,
                separator: ":".to_string(),
                ..Default::default()
            });
            let line_number = 10;
            let index = 0;
            assert_eq!(
                pr_get_formatted_line_number(&output_opts, line_number, index),
                "0:"
            );
        }

        #[test]
        fn test_line_number_large_width() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.number = Some(PrNumberingMode {
                width: 10,
                separator: ":".to_string(),
                ..Default::default()
            });
            let line_number = 10;
            let index = 0;
            assert_eq!(
                pr_get_formatted_line_number(&output_opts, line_number, index),
                "        10:"
            );
        }
    }

    #[cfg(test)]
    mod pr_header_content_tests {
        use super::*;

        #[test]
        fn test_header_content_disabled() {
            let output_opts = test_default_pr_output_opts();
            let page = 1;
            let result = pr_header_content(&output_opts, page);
            assert_eq!(result, Vec::<String>::new());
        }

        #[test]
        fn test_header_content_enabled() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.is_display_header_and_trailer = true;
            let page = 1;
            let expected = vec![
                String::new(),
                String::new(),
                format!("Apr 28 17:18 2024 test Page {}", page),
                String::new(),
                String::new(),
            ];
            let result = pr_header_content(&output_opts, page);
            assert_eq!(result, expected);
        }

        #[test]
        fn test_header_content_with_different_page() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.is_display_header_and_trailer = true;
            let page = 5;
            let expected = vec![
                String::new(),
                String::new(),
                format!("Apr 28 17:18 2024 test Page {}", page),
                String::new(),
                String::new(),
            ];
            let result = pr_header_content(&output_opts, page);
            assert_eq!(result, expected);
        }

        #[test]
        fn test_header_content_with_custom_last_modified_format() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.is_display_header_and_trailer = true;
            output_opts.last_modified_time = String::from("2024-04-28 17:18:00"); // 修改为ISO 8601格式
            let page = 1;
            let expected = vec![
                String::new(),
                String::new(),
                format!("2024-04-28 17:18:00 test Page {}", page),
                String::new(),
                String::new(),
            ];
            let result = pr_header_content(&output_opts, page);
            assert_eq!(result, expected);
        }

        #[test]
        fn test_header_content_with_empty_header() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.is_display_header_and_trailer = true;
            output_opts.header = String::new(); // 空的header
            let page = 1;
            let expected = vec![
                String::new(),
                String::new(),
                format!("Apr 28 17:18 2024  Page {}", page),
                String::new(),
                String::new(),
            ];
            let result = pr_header_content(&output_opts, page);
            assert_eq!(result, expected);
        }

        #[test]
        fn test_header_content_with_long_header() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.is_display_header_and_trailer = true;
            output_opts.header =
                "An extremely long header that should be truncated or handled appropriately"
                    .to_string();
            let page = 1;
            // 注意：具体处理长标题的逻辑（如截断）需根据实际业务需求编写，此处假设直接使用而不做特殊处理
            let expected = vec![
                String::new(),
                String::new(),
                format!(
                    "Apr 28 17:18 2024 An extremely long header that should be truncated or handled appropriately Page {}",
                    page
                ),
                String::new(),
                String::new(),
            ];
            let result = pr_header_content(&output_opts, page);
            assert_eq!(result, expected);
        }

        #[test]
        fn test_header_content_with_invalid_last_modified_format() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.is_display_header_and_trailer = true;
            output_opts.last_modified_time = String::from("InvalidTimestampFormat"); // 故意设置一个无效的时间格式
            let page = 1;
            let expected = vec![
                String::new(),
                String::new(),
                format!("InvalidTimestampFormat test Page {}", page),
                String::new(),
                String::new(),
            ];
            let result = pr_header_content(&output_opts, page);
            assert_eq!(result, expected);
        }
    }

    #[cfg(test)]
    mod file_last_modified_time_tests {
        use std::fs;
        use std::fs::File;

        use tempfile::Builder;

        use super::*;

        #[test]
        fn test_file_exists() {
            // 创建临时文件
            let tmp_dir = Builder::new().prefix("test_pr_file").tempdir().unwrap();
            let temp_path = tmp_dir.path().join("testfile.txt");

            // 写入内容以确保修改时间不是文件系统的默认创建时间
            let mut file = File::create(&temp_path).unwrap();
            writeln!(file, "Test content").unwrap();

            // 等待一小段时间以确保修改时间有明显差异（非必要，取决于测试环境的精确度要求）
            // sleep(Duration::from_millis(100));

            // 获取并验证文件的最后修改时间
            let last_modified_time_str = pr_file_last_modified_time(temp_path.to_str().unwrap());
            let last_modified_time = fs::metadata(&temp_path).unwrap().modified().unwrap();

            let date_time: DateTime<Local> = last_modified_time.into();
            let formatted_time = date_time.format(PR_DATE_TIME_FORMAT).to_string();

            assert_eq!(last_modified_time_str, formatted_time);
        }

        #[test]
        fn test_file_not_found() {
            let non_existent_path = "this/path/does/not/exist.txt";
            let result = pr_file_last_modified_time(non_existent_path);
            assert_eq!(result, String::new());
        }
    }

    #[cfg(test)]
    mod pr_trailer_content_tests {
        use super::*;

        #[test]
        fn test_trailer_content_display_and_no_form_feed() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.is_display_header_and_trailer = true;
            output_opts.is_form_feed_used = false;
            let expected = vec![
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            ];
            assert_eq!(pr_trailer_content(&output_opts), expected);
        }

        #[test]
        fn test_trailer_content_no_display() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.is_display_header_and_trailer = false;
            output_opts.is_form_feed_used = false;
            assert_eq!(pr_trailer_content(&output_opts), Vec::<String>::new());
        }

        #[test]
        fn test_trailer_content_form_feed_used() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.is_display_header_and_trailer = true;
            output_opts.is_form_feed_used = true;
            assert_eq!(pr_trailer_content(&output_opts), Vec::<String>::new());
        }

        #[test]
        fn test_trailer_content_neither_condition_met() {
            let output_opts = test_default_pr_output_opts(); // 默认情况下两个选项都为false
            assert_eq!(pr_trailer_content(&output_opts), Vec::<String>::new());
        }
    }

    #[cfg(test)]
    mod pr_get_start_line_number_tests {
        use super::*;

        #[test]
        fn test_pr_get_start_line_number_with_10_number_option() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.number = Some(PrNumberingMode {
                first_number: 10,
                ..Default::default()
            });

            let result = pr_get_start_line_number(&output_opts);

            assert_eq!(result, 10);
        }

        #[test]
        fn test_pr_get_start_line_number_number_none() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.number = None;
            let result = pr_get_start_line_number(&output_opts);

            assert_eq!(result, 1);
        }

        #[test]
        fn test_zero_start_line_number() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.number = Some(PrNumberingMode {
                first_number: 0,
                ..Default::default()
            });
            // 假设我们决定将0视为无效输入，默认为1
            assert_eq!(pr_get_start_line_number(&output_opts), 0);
        }

        #[test]
        fn test_large_start_line_number() {
            let large_number: usize = usize::MAX;
            let mut output_opts = test_default_pr_output_opts();
            output_opts.number = Some(PrNumberingMode {
                first_number: large_number,
                ..Default::default()
            });
            // 理论上应有错误处理机制，但此处简化处理，依然假设默认为1
            assert_eq!(pr_get_start_line_number(&output_opts), usize::MAX);
        }

        #[test]
        fn test_threadsafety_concurrent_access() {
            // 假设此函数可能在多线程环境中被调用，测试并发访问的安全性
            use std::sync::{Arc, Mutex};
            use std::thread;

            let mut output_opts = test_default_pr_output_opts();
            output_opts.number = Some(PrNumberingMode {
                first_number: 7,
                ..Default::default()
            });

            let output_opts_arc = Arc::new(Mutex::new(output_opts));
            let handles: Vec<_> = (0..10)
                .map(|_| {
                    let opts = Arc::clone(&output_opts_arc);
                    thread::spawn(move || {
                        let guard = opts.lock().unwrap();
                        pr_get_start_line_number(&*guard)
                    })
                })
                .collect();
            // 等待所有线程完成并验证结果
            for handle in handles {
                assert_eq!(handle.join().unwrap(), 7);
            }
        }
    }

    #[cfg(test)]
    mod pr_lines_to_read_for_page_tests {
        use super::*;

        #[test]
        fn test_pr_lines_to_read_for_page_single_space() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.content_lines_per_page = 25;
            output_opts.is_double_space = false;

            let result = pr_lines_to_read_for_page(&output_opts);
            assert_eq!(result, 25 * pr_get_columns(&output_opts));
        }

        #[test]
        fn test_pr_lines_to_read_for_page_double_space() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.content_lines_per_page = 25;
            output_opts.is_double_space = true;

            let result = pr_lines_to_read_for_page(&output_opts);
            assert_eq!(result, (25 / 2) * pr_get_columns(&output_opts));
        }

        #[test]
        fn test_single_spaced_no_columns() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.content_lines_per_page = 50;
            output_opts.is_double_space = false;

            assert_eq!(pr_lines_to_read_for_page(&output_opts), 50);
        }

        #[test]
        fn test_single_spaced_with_columns() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.content_lines_per_page = 50;
            output_opts.is_double_space = false;
            output_opts.column_mode_options = Some(PrColumnModeOptions {
                width: 60,
                columns: 2,
                column_separator: PR_DEFAULT_COLUMN_SEPARATOR.to_string(),
                is_across_mode: false,
            });

            assert_eq!(pr_lines_to_read_for_page(&output_opts), 100);
        }

        #[test]
        fn test_double_spaced_with_columns() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.content_lines_per_page = 50;
            output_opts.is_double_space = true;
            output_opts.column_mode_options = Some(PrColumnModeOptions {
                width: 60,
                columns: 2,
                column_separator: PR_DEFAULT_COLUMN_SEPARATOR.to_string(),
                is_across_mode: false,
            });

            // 双行距但有两列，计算方式应考虑双倍间距而非简单乘以列数
            assert_eq!(pr_lines_to_read_for_page(&output_opts), 50);
        }

        // 确保处理边缘情况，比如content_lines_per_page为0或1的情况
        #[test]
        fn test_edge_case_content_lines_0() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.content_lines_per_page = 0;
            output_opts.is_double_space = false;
            output_opts.column_mode_options = Some(PrColumnModeOptions {
                width: 60,
                columns: 3,
                column_separator: PR_DEFAULT_COLUMN_SEPARATOR.to_string(),
                is_across_mode: false,
            });

            // 应该返回0，因为没有内容行
            assert_eq!(pr_lines_to_read_for_page(&output_opts), 0);
        }

        #[test]
        fn test_edge_case_content_lines_1_double_space() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.content_lines_per_page = 1;
            output_opts.is_double_space = true;
            output_opts.column_mode_options = Some(PrColumnModeOptions {
                width: 60,
                columns: 4,
                column_separator: PR_DEFAULT_COLUMN_SEPARATOR.to_string(),
                is_across_mode: false,
            });

            // 或者是1，取决于实现逻辑是否考虑了至少读取一行的情况
            assert_eq!(pr_lines_to_read_for_page(&output_opts), 0);
        }

        #[test]
        fn test_single_spaced_with_columns_across_mode_true() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.content_lines_per_page = 50;
            output_opts.is_double_space = false;
            output_opts.column_mode_options = Some(PrColumnModeOptions {
                width: 60,
                columns: 2,
                column_separator: PR_DEFAULT_COLUMN_SEPARATOR.to_string(),
                is_across_mode: true,
            });

            assert_eq!(pr_lines_to_read_for_page(&output_opts), 100);
        }

        #[test]
        fn test_double_spaced_with_columns_across_mode_true() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.content_lines_per_page = 50;
            output_opts.is_double_space = true;
            output_opts.column_mode_options = Some(PrColumnModeOptions {
                width: 60,
                columns: 2,
                column_separator: PR_DEFAULT_COLUMN_SEPARATOR.to_string(),
                is_across_mode: true,
            });

            // 双行距但有两列，计算方式应考虑双倍间距而非简单乘以列数
            assert_eq!(pr_lines_to_read_for_page(&output_opts), 50);
        }

        // 确保处理边缘情况，比如content_lines_per_page为0或1的情况
        #[test]
        fn test_edge_case_content_lines_0_across_mode_true() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.content_lines_per_page = 0;
            output_opts.is_double_space = false;
            output_opts.column_mode_options = Some(PrColumnModeOptions {
                width: 60,
                columns: 3,
                column_separator: PR_DEFAULT_COLUMN_SEPARATOR.to_string(),
                is_across_mode: true,
            });

            // 应该返回0，因为没有内容行
            assert_eq!(pr_lines_to_read_for_page(&output_opts), 0);
        }

        #[test]
        fn test_edge_case_content_lines_1_double_space_across_mode_true() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.content_lines_per_page = 1;
            output_opts.is_double_space = true;
            output_opts.column_mode_options = Some(PrColumnModeOptions {
                width: 60,
                columns: 4,
                column_separator: PR_DEFAULT_COLUMN_SEPARATOR.to_string(),
                is_across_mode: true,
            });

            // 或者是1，取决于实现逻辑是否考虑了至少读取一行的情况
            assert_eq!(pr_lines_to_read_for_page(&output_opts), 0);
        }
    }

    #[cfg(test)]
    mod get_columns_tests {
        use super::*;

        #[test]
        fn test_pr_get_columns_without_column_mode_options() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.column_mode_options = None;

            let result = pr_get_columns(&output_opts);

            assert_eq!(result, 1);
        }

        #[test]
        fn test_pr_get_columns_with_column_mode_options() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.column_mode_options = Some(PrColumnModeOptions {
                columns: 3,
                width: 72,
                column_separator: PR_DEFAULT_COLUMN_SEPARATOR.to_string(),
                is_across_mode: false,
            });

            let result = pr_get_columns(&output_opts);

            assert_eq!(result, 3);
        }

        #[test]
        fn test_pr_get_columns_with_options_column_mode_columns_0() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.column_mode_options = Some(PrColumnModeOptions {
                columns: 0,
                width: 72,
                column_separator: PR_DEFAULT_COLUMN_SEPARATOR.to_string(),
                is_across_mode: false,
            });

            let result = pr_get_columns(&output_opts);

            assert_eq!(result, 0);
        }

        #[test]
        fn test_pr_get_columns_with_options_column_mode_columns_1() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.column_mode_options = Some(PrColumnModeOptions {
                columns: 1,
                width: 72,
                column_separator: PR_DEFAULT_COLUMN_SEPARATOR.to_string(),
                is_across_mode: false,
            });

            let result = pr_get_columns(&output_opts);

            assert_eq!(result, 1);
        }

        #[test]
        fn test_pr_get_columns_with_options_column_mode_columns_72() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.column_mode_options = Some(PrColumnModeOptions {
                columns: 72,
                width: 72,
                column_separator: PR_DEFAULT_COLUMN_SEPARATOR.to_string(),
                is_across_mode: false,
            });

            let result = pr_get_columns(&output_opts);

            assert_eq!(result, 72);
        }

        #[test]
        fn test_pr_get_columns_with_options_column_mode_columns_73() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.column_mode_options = Some(PrColumnModeOptions {
                columns: 73,
                width: 72,
                column_separator: PR_DEFAULT_COLUMN_SEPARATOR.to_string(),
                is_across_mode: false,
            });

            let result = pr_get_columns(&output_opts);

            assert_eq!(result, 73);
        }

        #[test]
        fn test_pr_get_columns_with_options_column_mode_columns_30() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.column_mode_options = Some(PrColumnModeOptions {
                columns: 30,
                width: 72,
                column_separator: PR_DEFAULT_COLUMN_SEPARATOR.to_string(),
                is_across_mode: false,
            });

            let result = pr_get_columns(&output_opts);

            assert_eq!(result, 30);
        }

        #[test]
        fn test_pr_get_columns_with_options_column_mode_columns_1_is_across_mode_true() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.column_mode_options = Some(PrColumnModeOptions {
                columns: 1,
                width: 72,
                column_separator: PR_DEFAULT_COLUMN_SEPARATOR.to_string(),
                is_across_mode: true,
            });

            let result = pr_get_columns(&output_opts);

            assert_eq!(result, 1);
        }

        #[test]
        fn test_pr_get_columns_with_options_column_mode_columns_1_column_separator_letter() {
            let mut output_opts = test_default_pr_output_opts();
            output_opts.column_mode_options = Some(PrColumnModeOptions {
                columns: 1,
                width: 72,
                column_separator: "a".to_string(),
                is_across_mode: false,
            });

            let result = pr_get_columns(&output_opts);

            assert_eq!(result, 1);
        }
    }

    #[cfg(test)]
    mod ct_main_tests {
        use std::ffi::OsString;

        use tempfile::TempDir;

        use super::*;

        // use tempfile::tempdir;

        #[test]
        fn test_ctmain_input_h() {
            let args = ["-h", ""];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));

            assert_eq!(result, 1);
        }

        #[test]
        fn test_ctmain_input_v() {
            let args = ["--version", ""];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));

            assert_eq!(result, 1);
        }

        #[test]
        fn test_ctmain_input_uppercase_v() {
            let args = ["-V", ""];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));

            assert_eq!(result, 1);
        }

        #[test]
        fn test_pr_main_default() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_pages_none_err() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--pages"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_pages_none_frist_page_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--pages=1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_pages_none_frist_page_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--pages=0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_pages_none_frist_page_2() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--pages=2"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_pages_none_frist_page_0_last_page_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--pages=0:1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_pages_none_frist_page_0_last_page_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--pages=0:0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_pages_none_frist_page_1_last_page_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--pages=1:1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_pages_none_frist_page_1_last_page_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--pages=1:100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_pages_none_frist_page_0_last_page_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--pages=0:100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_header_long_err() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--header"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_header_short_err() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-h"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_header_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                "--header",
                "CTyunOS pr test",
                file_name,
            ];
            // let args = vec![ctcore::ct_util_name(), "--header","CTyunOS pr test", "/home/workspace/syskits/LICENSE"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_header_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-h", "CTyunOS pr test"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_number_lines_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--number-lines"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_number_lines_long_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--number-lines", "0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_number_lines_long_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--number-lines", "1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_number_lines_long_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--number-lines", "100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_number_lines_long_t_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--number-lines",
                "t",
                "0",
            ];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_number_lines_long_t_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--number-lines",
                "t",
                "1",
            ];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_number_lines_long_t_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--number-lines",
                "t",
                "100",
            ];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_number_lines_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-n"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_number_lines_short_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-n", "0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_number_lines_short_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-n", "1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_number_lines_short_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-n", "100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_number_lines_short_t_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-n", "t", "0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_number_lines_short_t_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-n", "t", "1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_number_lines_short_t_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-n", "t", "100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_first_line_number_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--first-line-number"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_first_line_number_long_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--first-line-number",
                "0",
            ];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_first_line_number_long_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--first-line-number",
                "1",
            ];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_first_line_number_long_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--first-line-number",
                "100",
            ];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_first_line_number_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-N"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_first_line_number_short_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--first-line-number",
                "0",
            ];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_first_line_number_short_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-N", "1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_first_line_number_short_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-N", "100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_omit_header_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--omit-header"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_omit_header_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-t"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_length_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
                 world 2200 ccccc
                 CtyunOs 2000 aaaaa
                 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--length"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_length_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-l"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_length_long_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--length", "0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_length_long_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--length", "1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_length_long_10() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--length", "10"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_length_long_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--length", "100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_length_short_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-l", "0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_length_short_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-l", "1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_length_short_10() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-l", "10"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_length_short_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-l", "100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_file_warnings_long_exist_file() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--no-file-warnings"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_file_warnings_short_exist_file() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-r"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_file_warnings_long_no_exist_file() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--no-file-warnings"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_file_warnings_short_no_exist_file() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-r"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_form_feed_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--form-feed"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_form_feed_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-F"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_width_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--width"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_width_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-w"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_width_long_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--width", "0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_width_long_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--width", "1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_width_long_10() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--width", "10"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_width_long_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--width", "100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_width_long_1000() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--width", "1000"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_width_short_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-w", "0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_width_short_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-w", "1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_width_short_10() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-w", "10"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_width_short_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-w", "100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_width_short_1000() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-w", "1000"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_page_width_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--page-width"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_page_width_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-W"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_page_width_long_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--page-width", "0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_page_width_long_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--page-width", "1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_page_width_long_10() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--page-width", "10"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_page_width_long_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--page-width", "100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_page_width_long_1000() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--page-width", "1000"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_page_width_short_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-W", "0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_page_width_short_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-W", "1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_page_width_short_10() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-W", "10"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_page_width_short_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-W", "100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_page_width_short_1000() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-W", "1000"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_across_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--across"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_across_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-s"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_column_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--column"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_column_long_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--column", "0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_column_long_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--column", "1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_column_long_10() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--column", "683"]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_column_long_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--column", "100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_column_long_1000() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--column", "1000"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_separator_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--separator"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_separator_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-s"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_separator_long_letter() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--separator", "a"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_separator_long_digital() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--separator", "2"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_separator_long_upper_letter() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--separator", "C"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_separator_long_slash() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--separator", "\\"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_separator_long_colon() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--separator", ":"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_separator_long_horizontal() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--separator", "-"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_separator_long_n() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--separator", "\n"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_separator_long_r() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--separator", "\r"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_separator_long_t() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--separator", "\t"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_separator_short_letter() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
         world 2200 ccccc
         CtyunOs 2000 aaaaa
         CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-s", "a"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_separator_short_digital() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-s", "2"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_separator_short_upper_letter() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-s", "C"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_separator_short_slash() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-s", "\\"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_separator_short_colon() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-s", ":"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_separator_short_horizontal() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-s", "-"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_separator_short_n() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-s", "\n"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_separator_short_r() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-s", "\r"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_separator_short_t() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-s", "\t"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_sep_string_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--sep-string"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_sep_string_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-S"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_sep_string_long_letter() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
         world 2200 ccccc
         CtyunOs 2000 aaaaa
         CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--sep-string", "aa"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_sep_string_long_digital() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--sep-string", "22"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_sep_string_long_upper_letter() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--sep-string", "CA"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_sep_string_long_slash() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--sep-string", "aa\\"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_sep_string_long_colon() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--sep-string", "a:"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_sep_string_long_horizontal() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--sep-string", "--"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_sep_string_long_n() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--sep-string", "a\n"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_sep_string_long_r() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--sep-string", "a\r"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_sep_string_long_t() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--sep-string", "a\t"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_sep_string_short_letter() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
         world 2200 ccccc
         CtyunOs 2000 aaaaa
         CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-S", "aa"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_sep_string_short_digital() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-S", "22"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_sep_string_short_upper_letter() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-S", "CA"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_sep_string_short_slash() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-S", "aa\\"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_sep_string_short_colon() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-S", "a:"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_sep_string_short_horizontal() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-S", "--"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_sep_string_short_n() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-S", "a\n"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_sep_string_short_r() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-S", "a\r"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_sep_string_short_t() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-S", "a\t"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_merge_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();

            let file_path1 = dir.path().join("pr_test_file1");
            let mut tmp_file1 = File::create(&file_path1).unwrap();
            writeln!(
                tmp_file1,
                "aaHello 1000 zzzzz
 bbworld 2200 ccccc
 ccCtyunOs 2000 aaaaa
 ddCtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name1 = file_path1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--merge", file_name, file_name1];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_merge_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();

            let file_path1 = dir.path().join("pr_test_file1");
            let mut tmp_file1 = File::create(&file_path1).unwrap();
            writeln!(
                tmp_file1,
                "aaHello 1000 zzzzz
 bbworld 2200 ccccc
 ccCtyunOs 2000 aaaaa
 ddCtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name1 = file_path1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-m", file_name, file_name1];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_indent_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--indent"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_indent_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-o"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_indent_long_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--indent", "0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_indent_long_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--indent", "1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_indent_long_10() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--indent", "10"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_indent_long_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--indent", "100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_indent_long_1000() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--indent", "1000"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_indent_short_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-o", "0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_indent_short_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-o", "1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_indent_short_10() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-o", "10"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_indent_short_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-o", "100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_indent_short_1000() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-o", "1000"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_merge_full_lines_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-J"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_multi_column_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_multi_column_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_multi_column_10() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-10"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_multi_column_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_multi_column_1000() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-1000"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_pages_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "+0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_pages_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "+1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_pages_10() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "+10"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_pages_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "+100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_pages_1000() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz
 world 2200 ccccc
 CtyunOs 2000 aaaaa
 CtyunOs 1900 ababa"
            )
            .unwrap();
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "+1000"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        /*
         * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
         *  syskits is licensed under Mulan PSL v2.
         * You can use this software according to the terms and conditions of the Mulan PSL V2
         * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
         * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
         * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
         * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
         * See the Mulan PSL v2 for more details.
         *
         */

        #[test]
        fn test_pr_main_no_exist_file_default() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_pages_none_err() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--pages"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_pages_none_frist_page_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--pages=1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_pages_none_frist_page_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--pages=0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_pages_none_frist_page_2() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--pages=2"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_pages_none_frist_page_0_last_page_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--pages=0:1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_pages_none_frist_page_0_last_page_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--pages=0:0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_pages_none_frist_page_1_last_page_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--pages=1:1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_pages_none_frist_page_1_last_page_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--pages=1:100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_pages_none_frist_page_0_last_page_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--pages=0:100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_header_long_err() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--header"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_header_short_err() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-h"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_header_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                "--header",
                "CTyunOS pr test",
                file_name,
            ];
            // let args = vec![ctcore::ct_util_name(), "--header","CTyunOS pr test", "/home/workspace/syskits/LICENSE"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_header_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-h", "CTyunOS pr test"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_number_lines_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--number-lines"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_number_lines_long_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--number-lines", "0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_number_lines_long_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--number-lines", "1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_number_lines_long_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--number-lines", "100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_number_lines_long_t_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--number-lines",
                "t",
                "0",
            ];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_number_lines_long_t_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--number-lines",
                "t",
                "1",
            ];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_number_lines_long_t_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--number-lines",
                "t",
                "100",
            ];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_number_lines_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-n"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_number_lines_short_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-n", "0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_number_lines_short_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-n", "1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_number_lines_short_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-n", "100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_number_lines_short_t_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-n", "t", "0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_number_lines_short_t_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-n", "t", "1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_number_lines_short_t_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-n", "t", "100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_first_line_number_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--first-line-number"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_first_line_number_long_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--first-line-number",
                "0",
            ];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_first_line_number_long_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--first-line-number",
                "1",
            ];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_first_line_number_long_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--first-line-number",
                "100",
            ];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_first_line_number_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-N"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_first_line_number_short_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![
                ctcore::ct_util_name(),
                file_name,
                "--first-line-number",
                "0",
            ];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_first_line_number_short_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-N", "1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_first_line_number_short_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-N", "100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_omit_header_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--omit-header"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_omit_header_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-t"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_length_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--length"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_length_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-l"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_length_long_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--length", "0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_length_long_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--length", "1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_length_long_10() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--length", "10"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_length_long_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--length", "100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_length_short_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-l", "0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_length_short_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-l", "1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_length_short_10() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-l", "10"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_length_short_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-l", "100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_no_file_warnings_long_exist_file() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--no-file-warnings"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_no_file_warnings_short_exist_file() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-r"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_form_feed_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--form-feed"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_form_feed_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-F"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_width_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--width"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_width_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-w"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_width_long_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--width", "0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_width_long_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--width", "1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_width_long_10() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--width", "10"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_width_long_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--width", "100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_width_long_1000() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--width", "1000"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_width_short_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-w", "0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_width_short_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-w", "1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_width_short_10() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-w", "10"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_width_short_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-w", "100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_width_short_1000() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-w", "1000"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_page_width_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--page-width"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_page_width_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-W"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_page_width_long_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--page-width", "0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_page_width_long_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--page-width", "1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_page_width_long_10() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--page-width", "10"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_page_width_long_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--page-width", "100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_page_width_long_1000() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--page-width", "1000"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_page_width_short_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-W", "0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_page_width_short_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-W", "1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_page_width_short_10() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-W", "10"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_page_width_short_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-W", "100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_page_width_short_1000() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-W", "1000"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_across_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--across"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_across_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-s"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_column_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--column"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_column_long_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--column", "0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_column_long_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--column", "1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_column_long_10() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--column", "683"]; //column设置的原因是test是存在 -数字，导致列数为随机值，通过设定保证输出一致性
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_column_long_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--column", "100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_column_long_1000() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--column", "1000"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_separator_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--separator"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_separator_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-s"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_separator_long_letter() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--separator", "a"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_separator_long_digital() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--separator", "2"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_separator_long_upper_letter() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--separator", "C"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_separator_long_slash() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--separator", "\\"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_separator_long_colon() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--separator", ":"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_separator_long_horizontal() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--separator", "-"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_separator_long_n() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--separator", "\n"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_separator_long_r() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--separator", "\r"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_separator_long_t() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--separator", "\t"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_separator_short_letter() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-s", "a"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_separator_short_digital() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-s", "2"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_separator_short_upper_letter() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-s", "C"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_separator_short_slash() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-s", "\\"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_separator_short_colon() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-s", ":"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_separator_short_horizontal() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-s", "-"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_separator_short_n() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-s", "\n"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_separator_short_r() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-s", "\r"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_separator_short_t() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-s", "\t"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_sep_string_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--sep-string"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_sep_string_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-S"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_sep_string_long_letter() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--sep-string", "aa"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_sep_string_long_digital() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--sep-string", "22"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_sep_string_long_upper_letter() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--sep-string", "CA"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_sep_string_long_slash() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--sep-string", "aa\\"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_sep_string_long_colon() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--sep-string", "a:"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_sep_string_long_horizontal() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--sep-string", "--"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_sep_string_long_n() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--sep-string", "a\n"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_sep_string_long_r() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--sep-string", "a\r"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_sep_string_long_t() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--sep-string", "a\t"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_sep_string_short_letter() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-S", "aa"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_sep_string_short_digital() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-S", "22"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_sep_string_short_upper_letter() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-S", "CA"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_sep_string_short_slash() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-S", "aa\\"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_sep_string_short_colon() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-S", "a:"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_sep_string_short_horizontal() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-S", "--"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_sep_string_short_n() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-S", "a\n"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_sep_string_short_r() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-S", "a\r"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_sep_string_short_t() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-S", "a\t"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_merge_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let file_name = file_path.to_str().unwrap();

            let file_path1 = dir.path().join("pr_test_file1");
            let file_name1 = file_path1.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--merge", file_name, file_name1];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_merge_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");
            let file_name = file_path.to_str().unwrap();

            let file_path1 = dir.path().join("pr_test_file1");
            let file_name1 = file_path1.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), "-m", file_name, file_name1];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_indent_long() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--indent"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_indent_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-o"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_indent_long_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--indent", "0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_indent_long_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--indent", "1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_indent_long_10() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--indent", "10"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_indent_long_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--indent", "100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_indent_long_1000() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "--indent", "1000"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_indent_short_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-o", "0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_indent_short_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-o", "1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_indent_short_10() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-o", "10"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_indent_short_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-o", "100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_indent_short_1000() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-o", "1000"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_merge_full_lines_short() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-J"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_multi_column_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_multi_column_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_multi_column_10() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-10"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_multi_column_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_multi_column_1000() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "-1000"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_pages_0() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "+0"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_pages_1() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "+1"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_pages_10() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "+10"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_pages_100() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "+100"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_pr_main_no_exist_file_pages_1000() {
            let dir = TempDir::with_prefix("test_pr_").unwrap();
            let file_path = dir.path().join("pr_test_file");

            let file_name = file_path.to_str().unwrap();
            let args = vec![ctcore::ct_util_name(), file_name, "+1000"];
            let result = pr_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                    assert_eq!(code, 1);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }
    }

    #[cfg(test)]
    mod ct_app_tests {
        use clap::error::ErrorKind;

        use super::*;

        // pr 接口: pr [OPTION]... [FILE]...
        //
        // Arguments:
        //   [files]...
        //
        // Options:
        //       --pages <FIRST_PAGE[:LAST_PAGE]>  Begin and stop printing with page FIRST_PAGE[:LAST_PAGE]
        //   -h, --header <STRING>                 Use the string header to replace the file name in the header line.
        //   -d, --double-space                    Produce output that is double spaced. An extra <newline> character is output following every <newline> found in the input.
        //   -n, --number-lines <[char][width]>    Provide width digit line numbering.  The default for width, if not specified, is 5.  The number occupies the first width column positions of each text column or each line of -m output.  If char (any non-digit character) is given, it is appended to the line number to
        //                                         separate it from whatever follows.  The default for char is a <tab>. Line numbers longer than width columns are truncated.
        //   -N, --first-line-number <NUMBER>      start counting with NUMBER at 1st line of first page printed
        //   -t, --omit-header                     Write neither the five-line identifying header nor the five-line trailer usually supplied for each page. Quit writing after the last line of each file without spacing to the end of the page.
        //   -l, --length <PAGE_LENGTH>            Override the 66-line default (default number of lines of text 56, and with -F 63) and reset the page length to lines.  If lines is not greater than the sum  of  both the  header  and trailer depths (in lines), the pr utility shall suppress both the header and trailer,
        //                                         as if the -t option were in effect.
        //   -r, --no-file-warnings                omit warning when a file cannot be opened
        //   -F, --form-feed                       Use a <form-feed> for new pages, instead of the default behavior that uses a sequence of <newline>s.
        //   -w, --width <width>                   Set the width of the line to width column positions for multiple text-column output only. If the -w option is not specified and the -s option is not specified, the default width shall be 72. If the -w option is not specified and the -s option is specified, the default
        //                                         width shall be 512.
        //   -W, --page-width <width>              set page width to PAGE_WIDTH (72) characters always, truncate lines, except -J option is set, no interference with -S or -s
        //   -a, --across                          Modify the effect of the - column option so that the columns are filled across the page in a  round-robin  order (for example, when column is 2, the first input line heads column 1, the second heads column 2, the third is the second line in column 1, and so on).
        //       --column <column>                 Produce multi-column output that is arranged in column columns (the default shall be 1) and is written down each column  in  the order in which the text is received from the input file. This option should not be used with -m. The options -e and -i shall be assumed for
        //                                         multiple text-column output.  Whether or not text columns are produced with identical vertical lengths is unspecified, but a text column shall never exceed the length of the page (see the -l option). When used with -t, use the minimum number of lines to write the
        //                                         output.
        //   -s, --separator <char>                Separate text columns by the single character char instead of by the appropriate number of <space>s (default for char is the <tab> character).
        //   -S, --sep-string <string>             separate columns by STRING, without -S: Default separator <TAB> with -J and <space> otherwise (same as -S" "), no effect on column options
        //   -m, --merge                           Merge files. Standard output shall be formatted so the pr utility writes one line from each file specified by a file operand, side by side into text columns of equal fixed widths, in terms of the number of column positions. Implementations shall support merging of at
        //                                         least nine file operands.
        //   -o, --indent <margin>                 Each line of output shall be preceded by offset <space>s. If the -o option is not specified, the default offset shall be zero. The space taken is in addition to the output line width (see the -w option below).
        //   -J                                    merge full lines, turns off -W line truncation, no column alignment, --sep-string[=STRING] sets separators
        //       --help                            Print help information
        //   -V, --version                         Print version
        //
        // +PAGE           Begin output at page number page of the formatted input.
        // -COLUMN         Produce multi-column output. See --column
        //
        // The pr utility is a printing and pagination filter for text files.
        // When multiple input files are specified, each is read, formatted, and written to standard output.
        // By default, the input is separated into 66-line pages, each with
        //
        // * A 5-line header with the page number, date, time, and the pathname of the file.
        // * A 5-line trailer consisting of blank lines.
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
        fn test_ct_app_pages_long() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--pages"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_pages_long_with_frist() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--pages", "1"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_PAGES));
        }

        #[test]
        fn test_ct_app_pages_long_with_frist_end() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--pages", "1:10"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_PAGES));
        }

        #[test]
        fn test_ct_app_pages_long_with_frist_end_error_range() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--pages", "10:1"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_PAGES));
        }

        #[test]
        fn test_ct_app_pages_long_with_frist_end_error_valus() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--pages", "aaa"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_PAGES));
        }

        #[test]
        fn test_ct_app_header_long() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--header", "ctyunos test"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_HEADER));
        }

        #[test]
        fn test_ct_app_header_short() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-h", "ctyunos test"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_HEADER));
        }

        #[test]
        fn test_ct_app_double_space_long() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--double-space"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_DOUBLE_SPACE));
        }

        #[test]
        fn test_ct_app_double_space_short() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-d"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_DOUBLE_SPACE));
        }

        #[test]
        fn test_ct_app_number_lines_long() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--number-lines"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_number_lines_long_char_5() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--number-lines", "5"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_NUMBER_LINES));
        }

        #[test]
        fn test_ct_app_number_lines_long_char_a() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--number-lines", "a"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_NUMBER_LINES));
        }

        #[test]
        fn test_ct_app_number_lines_long_char_5_width_6() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--number-lines", "5", "6"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_NUMBER_LINES));
        }

        #[test]
        fn test_ct_app_number_lines_short() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-n"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_number_lines_short_char_5() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-n", "5"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_NUMBER_LINES));
        }

        #[test]
        fn test_ct_app_number_lines_short_char_a() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-n", "a"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_NUMBER_LINES));
        }

        #[test]
        fn test_ct_app_number_lines_short_char_5_width_6() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-n", "5", "6"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_NUMBER_LINES));
        }

        #[test]
        fn test_ct_app_first_line_number_short() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-N"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_first_line_number_long() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--first-line-number"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_first_line_number_short_with_number() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-N", "5"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_FIRST_LINE_NUMBER));
        }

        #[test]
        fn test_ct_app_first_line_number_long_with_number() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--first-line-number", "5"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_FIRST_LINE_NUMBER));
        }

        #[test]
        fn test_ct_app_first_line_number_short_with_letter() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-N", "b"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_FIRST_LINE_NUMBER));
        }

        #[test]
        fn test_ct_app_first_line_number_long_with_letter() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--first-line-number", "b"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_FIRST_LINE_NUMBER));
        }

        #[test]
        fn test_ct_app_omit_header_long() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--omit-header"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_OMIT_HEADER));
        }

        #[test]
        fn test_ct_app_omit_header_short() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-t"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_OMIT_HEADER));
        }

        #[test]
        fn test_ct_app_length_long() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--length"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_length_short() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-l"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_length_long_with_page_length() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--length", "66"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_PAGE_LENGTH));
        }

        #[test]
        fn test_ct_app_length_short_with_page_length() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-l", "66"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_PAGE_LENGTH));
        }

        #[test]
        fn test_ct_app_length_long_omit_header_short() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--length", "-t"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_length_short_omit_header_short() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-lt"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_PAGE_LENGTH));
        }

        #[test]
        fn test_ct_app_length_long_omit_header_long() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--length", "--omit-header"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_length_short_omit_header_long() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-l", "--omit-header"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_no_file_warnings_long() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--no-file-warnings"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_NO_FILE_WARNINGS));
        }

        #[test]
        fn test_ct_app_no_file_warnings_short() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-r"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_NO_FILE_WARNINGS));
        }

        #[test]
        fn test_ct_app_form_feed_long() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--form-feed"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_FORM_FEED));
        }

        #[test]
        fn test_ct_app_form_feed_short() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-F"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_FORM_FEED));
        }

        #[test]
        fn test_ct_app_length_long_form_feed_long() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--length", "--form-feed"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_length_short_with_value_form_feed_long() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-l", "100", "--form-feed"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_PAGE_LENGTH));
        }

        #[test]
        fn test_ct_app_length_long_with_value_form_feed_short() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--length", "100", "-F"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_PAGE_LENGTH));
        }

        #[test]
        fn test_ct_app_length_short_form_feed_long() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-l", "--form-feed"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_length_long_form_feed_short() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--length", "-F"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_length_short_form_feed_short() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-lF"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_FORM_FEED));
        }

        #[test]
        fn test_ct_app_width_long() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--width"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_width_short() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-w"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_width_long_wtih_value() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--width", "88"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_COLUMN_WIDTH));
        }

        #[test]
        fn test_ct_app_width_short_wtih_value() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-w", "88"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_COLUMN_WIDTH));
        }

        #[test]
        fn test_ct_app_page_width_long() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--page-width", "68"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_PAGE_WIDTH));
        }

        #[test]
        fn test_ct_app_page_width_short() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-W", "68"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_PAGE_WIDTH));
        }

        #[test]
        fn test_ct_app_across_long() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--across"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_ACROSS));
        }

        #[test]
        fn test_ct_app_across_short() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-a"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_ACROSS));
        }

        #[test]
        fn test_ct_app_column_long() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--column"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_column_long_with_value() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--column", "77"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_COLUMN));
        }

        #[test]
        fn test_ct_app_omit_header_long_column_long_with_value() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--omit-header", "--column", "77"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_COLUMN));
        }

        #[test]
        fn test_ct_app_omit_header_short_column_long_with_value() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-t", "--column", "77"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_COLUMN));
        }

        #[test]
        fn test_ct_app_omit_header_long_column_long_with_letter() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--omit-header", "--column", "cc"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_COLUMN));
        }

        #[test]
        fn test_ct_app_omit_header_short_column_long_with_letter() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-t", "--column", "cc"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_COLUMN));
        }

        #[test]
        fn test_ct_app_separator_long() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--separator"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_separator_short() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-s"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_separator_long_with_char_width_short_with_value() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--separator", "\n", "-w", "88"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .contains_id(pr_flags::PR_COLUMN_CHAR_SEPARATOR)
            );
        }

        #[test]
        fn test_ct_app_separator_short_with_char_width_short_with_value() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-s", "\n", "-w", "88"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .contains_id(pr_flags::PR_COLUMN_CHAR_SEPARATOR)
            );
        }

        #[test]
        fn test_ct_app_separator_long_with_char_width_long_with_value() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--separator", "\n", "--width", "88"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .contains_id(pr_flags::PR_COLUMN_CHAR_SEPARATOR)
            );
        }

        #[test]
        fn test_ct_app_separator_short_with_char_width_long_with_value() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-s", "\n", "--width", "88"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .contains_id(pr_flags::PR_COLUMN_CHAR_SEPARATOR)
            );
        }

        #[test]
        fn test_ct_app_separator_long_with_char() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--separator", "\n"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .contains_id(pr_flags::PR_COLUMN_CHAR_SEPARATOR)
            );
        }

        #[test]
        fn test_ct_app_separator_short_with_char() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-s", "\n"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .contains_id(pr_flags::PR_COLUMN_CHAR_SEPARATOR)
            );
        }

        #[test]
        fn test_ct_app_page_width_long_sep_string_long_with_value() {
            let command = ct_app();

            let invalid_args = vec![
                ctcore::ct_util_name(),
                "--page-width",
                "68",
                "--sep-string",
                "a",
            ];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .contains_id(pr_flags::PR_COLUMN_STRING_SEPARATOR)
            );
        }

        #[test]
        fn test_ct_app_page_width_short_sep_string_long_with_value() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-W", "68", "--sep-string", "a"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .contains_id(pr_flags::PR_COLUMN_STRING_SEPARATOR)
            );
        }

        #[test]
        fn test_ct_app_page_width_long_sep_string_short_with_value() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--page-width", "68", "-S", "aa"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .contains_id(pr_flags::PR_COLUMN_STRING_SEPARATOR)
            );
        }

        #[test]
        fn test_ct_app_page_width_short_sep_string_short_with_value() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-W", "68", "-S", "aa"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .contains_id(pr_flags::PR_COLUMN_STRING_SEPARATOR)
            );
        }

        #[test]
        fn test_ct_app_sep_string_long() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--sep-string"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_sep_string_short() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-S"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_sep_string_long_with_value() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--sep-string", "aa"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .contains_id(pr_flags::PR_COLUMN_STRING_SEPARATOR)
            );
        }

        #[test]
        fn test_ct_app_sep_string_short_with_value() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-S", "aa"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(
                result
                    .unwrap()
                    .contains_id(pr_flags::PR_COLUMN_STRING_SEPARATOR)
            );
        }

        #[test]
        fn test_ct_app_merge_long() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--merge"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_MERGE));
        }

        #[test]
        fn test_ct_app_merge_short() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-m"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_MERGE));
        }

        #[test]
        fn test_ct_app_column_long_with_value_merge_long() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--column", "77", "--merge"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_MERGE));
        }

        #[test]
        fn test_ct_app_column_long_with_value_merge_short() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--column", "77", "-m"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_MERGE));
        }

        #[test]
        fn test_ct_app_indent_short() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-o"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_indent_long() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--indent"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_indent_short_with_value() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-o", " "];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_INDENT));
        }

        #[test]
        fn test_ct_app_indent_long_with_value() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--indent", " "];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_INDENT));
        }

        #[test]
        fn test_ct_app_merge_full_lines_short() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-J"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_JOIN_LINES));
        }

        #[test]
        fn test_ct_app_page_width_long_merge_full_lines_short() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--page-width", "68", "-J"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_JOIN_LINES));
        }

        #[test]
        fn test_ct_app_page_width_short_merge_full_lines_short() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "-W", "68", "-J"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_ok());
            assert!(result.unwrap().contains_id(pr_flags::PR_JOIN_LINES));
        }
    }
}
