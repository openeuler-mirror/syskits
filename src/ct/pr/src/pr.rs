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

use std::fs::{metadata, File};
use std::io::{stdin, stdout, BufRead, BufReader, Error, Lines, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;

use chrono::{DateTime, Local};
use clap::{crate_version, Arg, ArgAction, ArgMatches, Command};
use itertools::Itertools;
use quick_error::quick_error;
use quick_error::ResultExt;
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
                    && last_page.map_or(true, |last_page| current_page <= last_page)
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

