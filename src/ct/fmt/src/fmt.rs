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

// ! fmt命令可以从指定的文件里面读取内容，并且将其按照指定格式重新编排后，输出到标准输出设备。

use std::fs::File;
use std::io::{stdin, stdout, BufReader, BufWriter, Read, Write};

use clap::{crate_version, Arg, ArgAction, ArgMatches, Command};

use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CtSimpleError, FromIo};
use ctcore::{ct_format_usage, ct_help_about, ct_help_usage, ct_show_warning};
use line_break::fmt_break_lines;
use para_split::FmtParagraphStream;

mod line_break;
mod para_split;

const FMT_ABOUT: &str = ct_help_about!("fmt.md");
const FMT_USAGE: &str = ct_help_usage!("fmt.md");
const FMT_MAX_WIDTH: usize = 2500;
const FMT_DEFAULT_GOAL: usize = 70;
const FMT_DEFAULT_WIDTH: usize = 75;
// 默认情况下，目标为宽度的 93
const FMT_DEFAULT_GOAL_TO_WIDTH_RATIO: usize = 93;

mod fmt_flags {
    pub const FMT_CROWN_MARGIN: &str = "crown-margin";
    pub const FMT_TAGGED_PARAGRAPH: &str = "tagged-paragraph";
    pub const FMT_PRESERVE_HEADERS: &str = "preserve-headers";
    pub const FMT_SPLIT_ONLY: &str = "split-only";
    pub const FMT_UNIFORM_SPACING: &str = "uniform-spacing";
    pub const FMT_PREFIX: &str = "prefix";
    pub const FMT_SKIP_PREFIX: &str = "skip-prefix";
    pub const FMT_EXACT_PREFIX: &str = "exact-prefix";
    pub const FMT_EXACT_SKIP_PREFIX: &str = "exact-skip-prefix";
    pub const FMT_WIDTH: &str = "width";
    pub const FMT_GOAL: &str = "goal";
    pub const FMT_QUICK: &str = "quick";
    pub const FMT_TAB_WIDTH: &str = "tab-width";
    pub const FMT_FILES: &str = "files";
}

pub type FmtFileOrStdReader = BufReader<Box<dyn Read + 'static>>;

#[derive(PartialEq, Debug, Default)]
pub struct FmtConfigs {
    is_crown: bool,
    is_tagged: bool,
    is_mail: bool,
    is_split_only: bool,
    prefix_option: Option<String>,
    is_xprefix: bool,
    anti_prefix_option: Option<String>,
    is_xanti_prefix: bool,
    is_uniform: bool,
    is_quick: bool,
    width: usize,
    goal: usize,
    tab_width: usize,
}

impl FmtConfigs {
    fn from_matches(arg_matches: &ArgMatches) -> CTResult<Self> {
        let mut is_tagged = arg_matches.get_flag(fmt_flags::FMT_TAGGED_PARAGRAPH);
        let mut is_crown = arg_matches.get_flag(fmt_flags::FMT_CROWN_MARGIN);

        let is_mail = arg_matches.get_flag(fmt_flags::FMT_PRESERVE_HEADERS);
        let is_uniform = arg_matches.get_flag(fmt_flags::FMT_UNIFORM_SPACING);
        let is_quick = arg_matches.get_flag(fmt_flags::FMT_QUICK);
        let is_split_only = arg_matches.get_flag(fmt_flags::FMT_SPLIT_ONLY);

        if is_crown {
            is_tagged = false;
        }
        if is_split_only {
            is_crown = false;
            is_tagged = false;
        }

        let is_xprefix = arg_matches.contains_id(fmt_flags::FMT_EXACT_PREFIX);
        let is_xanti_prefix = arg_matches.contains_id(fmt_flags::FMT_SKIP_PREFIX);

        let prefix_option = arg_matches
            .get_one::<String>(fmt_flags::FMT_PREFIX)
            .map(String::from);
        let anti_prefix_option = arg_matches
            .get_one::<String>(fmt_flags::FMT_SKIP_PREFIX)
            .map(String::from);

        let (width, goal) = Self::parse_width_and_goal(arg_matches)?;
        let tab_width = Self::parse_tab_width(arg_matches)?;

        Ok(Self {
            is_crown,
            is_tagged,
            is_mail,
            is_uniform,
            is_quick,
            is_split_only,
            prefix_option,
            is_xprefix,
            anti_prefix_option,
            is_xanti_prefix,
            width,
            goal,
            tab_width,
        })
    }

    fn parse_tab_width(arg_matches: &ArgMatches) -> CTResult<usize> {
        let mut tabwidth = if let Some(s) = arg_matches.get_one::<String>(fmt_flags::FMT_TAB_WIDTH)
        {
            match s.parse::<usize>() {
                Ok(t) => t,
                Err(e) => {
                    return Err(CtSimpleError::new(
                        1,
                        format!("Invalid TABWIDTH specification: {}: {}", s.quote(), e),
                    ));
                }
            }
        } else {
            8
        };

        if tabwidth < 1 {
            tabwidth = 1;
        }
        Ok(tabwidth)
    }

    fn parse_width_and_goal(arg_matches: &ArgMatches) -> CTResult<(usize, usize)> {
        let width_opt = arg_matches.get_one::<usize>(fmt_flags::FMT_WIDTH);
        let goal_opt = arg_matches.get_one::<usize>(fmt_flags::FMT_GOAL);
        let (width, goal) = match (width_opt, goal_opt) {
            (Some(&w), Some(&g)) => {
                if g > w {
                    return Err(CtSimpleError::new(1, "GOAL cannot be greater than WIDTH."));
                }
                (w, g)
            }
            (Some(&w), None) => {
                // 只有当宽度设置为零时，才允许目标值为零
                let g = (w * FMT_DEFAULT_GOAL_TO_WIDTH_RATIO / 100).max(if w == 0 { 0 } else { 1 });
                (w, g)
            }
            (None, Some(&g)) => {
                if g > FMT_DEFAULT_WIDTH {
                    return Err(CtSimpleError::new(1, "GOAL cannot be greater than WIDTH."));
                }
                let w = (g * 100 / FMT_DEFAULT_GOAL_TO_WIDTH_RATIO).max(g + 3);
                (w, g)
            }
            (None, None) => (FMT_DEFAULT_WIDTH, FMT_DEFAULT_GOAL),
        };

        debug_assert!(width >= goal, "GOAL {goal} should not be greater than WIDTH {width} when given {width_opt:?} and {goal_opt:?}.");

        if width > FMT_MAX_WIDTH {
            return Err(CtSimpleError::new(
                1,
                format!("invalid width: '{}': Numerical result out of range", width),
            ));
        }
        Ok((width, goal))
    }
}

/// 处理文件内容，并根据提供的选项对文件进行ct_format处理。
///
/// # 参数
///
/// * `file_name` - 要处理的文件名。值为"-"代表标准输入。
/// * `fmt_opts` - 指向包含格式化选项的 `FmtOptions` 结构的引用。
/// * `ostream` - 一个对封装标准输出的 `BufWriter` 的可变引用。
///
/// # 返回
///
/// `UResult<()>` 表示成功或失败。
fn fmt_process_file<W: ?Sized + Write>(
    file_name: &str,
    fmt_configs: &FmtConfigs,
    output_stream: &mut W,
) -> CTResult<()> {
    let mut fp = if file_name == "-" {
        BufReader::new(Box::new(stdin()) as Box<dyn Read + 'static>)
    } else {
        match File::open(file_name) {
            Ok(f) => BufReader::new(Box::new(f) as Box<dyn Read + 'static>),
            Err(e) => {
                ct_show_warning!("{}: {}", file_name.maybe_quote(), e);
                return Ok(());
            }
        }
    };

    let paragraph_stream = FmtParagraphStream::new(fmt_configs, &mut fp);
    for para_result in paragraph_stream {
        match para_result {
            Err(s) => {
                output_stream
                    .write_all(s.as_bytes())
                    .map_err_context(|| "failed to write output".to_string())?;
                output_stream
                    .write_all(b"\n")
                    .map_err_context(|| "failed to write output".to_string())?;
            }
            Ok(para) => fmt_break_lines(&para, fmt_configs, output_stream)
                .map_err_context(|| "failed to write output".to_string())?,
        }
    }

    // 清除每个文件后的输出
    output_stream
        .flush()
        .map_err_context(|| "failed to write output".to_string())?;

    Ok(())
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    fmt_main(args)
}

pub fn fmt_main(args: impl ctcore::Args) -> CTResult<()> {
    let matches = ct_app().try_get_matches_from(args)?;

    let files: Vec<String> = matches
        .get_many::<String>(fmt_flags::FMT_FILES)
        .map(|v| v.map(ToString::to_string).collect())
        .unwrap_or(vec!["-".into()]);

    let fmt_opts = FmtConfigs::from_matches(&matches)?;

    let mut ostream = BufWriter::new(stdout());

    for file_name in &files {
        fmt_process_file(file_name, &fmt_opts, &mut ostream)?;
    }

    Ok(())
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = FMT_ABOUT;
    let usage_description = ct_format_usage(FMT_USAGE);
    let args = vec![
        Arg::new(fmt_flags::FMT_CROWN_MARGIN)
            .short('c')
            .long(fmt_flags::FMT_CROWN_MARGIN)
            .help(
                "First and second line of paragraph \
                    may have different indentations, in which \
                    case the first line's indentation is preserved, \
                    and each subsequent line's indentation matches the second line.",
            )
            .action(ArgAction::SetTrue),
        Arg::new(fmt_flags::FMT_TAGGED_PARAGRAPH)
            .short('t')
            .long("tagged-paragraph")
            .help(
                "Like -c, except that the first and second line of a paragraph *must* \
                    have different indentation or they are treated as separate paragraphs.",
            )
            .action(ArgAction::SetTrue),
        Arg::new(fmt_flags::FMT_PRESERVE_HEADERS)
            .short('m')
            .long("preserve-headers")
            .help(
                "Attempt to detect and preserve mail headers in the input. \
                    Be careful when combining this flag with -p.",
            )
            .action(ArgAction::SetTrue),
        Arg::new(fmt_flags::FMT_SPLIT_ONLY)
            .short('s')
            .long("split-only")
            .help("Split lines only, do not reflow.")
            .action(ArgAction::SetTrue),
        Arg::new(fmt_flags::FMT_UNIFORM_SPACING)
            .short('u')
            .long("uniform-spacing")
            .help(
                "Insert exactly one \
                    space between words, and two between sentences. \
                    Sentence breaks in the input are detected as [?!.] \
                    followed by two spaces or a newline; other punctuation \
                    is not interpreted as a sentence break.",
            )
            .action(ArgAction::SetTrue),
        Arg::new(fmt_flags::FMT_PREFIX)
            .short('p')
            .long("prefix")
            .help(
                "Reformat only lines \
                    beginning with PREFIX, reattaching PREFIX to reformatted lines. \
                    Unless -x is specified, leading whitespace will be ignored \
                    when matching PREFIX.",
            )
            .value_name("PREFIX"),
        Arg::new(fmt_flags::FMT_SKIP_PREFIX)
            .short('P')
            .long("skip-prefix")
            .help(
                "Do not reformat lines \
                    beginning with PSKIP. Unless -X is specified, leading whitespace \
                    will be ignored when matching PSKIP",
            )
            .value_name("PSKIP"),
        Arg::new(fmt_flags::FMT_EXACT_PREFIX)
            .short('x')
            .long("exact-prefix")
            .help(
                "PREFIX must match at the \
                    beginning of the line with no preceding whitespace.",
            )
            .action(ArgAction::SetTrue),
        Arg::new(fmt_flags::FMT_EXACT_SKIP_PREFIX)
            .short('X')
            .long("exact-skip-prefix")
            .help(
                "PSKIP must match at the \
                    beginning of the line with no preceding whitespace.",
            )
            .action(ArgAction::SetTrue),
        Arg::new(fmt_flags::FMT_WIDTH)
            .short('w')
            .long("width")
            .help("Fill output lines up to a maximum of WIDTH columns, default 75.")
            .value_name("WIDTH")
            .value_parser(clap::value_parser!(usize)),
        Arg::new(fmt_flags::FMT_GOAL)
            .short('g')
            .long("goal")
            .help("Goal width, default of 93% of WIDTH. Must be less than or equal to WIDTH.")
            .value_name("GOAL")
            .value_parser(clap::value_parser!(usize)),
        Arg::new(fmt_flags::FMT_QUICK)
            .short('q')
            .long("quick")
            .help(
                "Break lines more quickly at the \
            expense of a potentially more ragged appearance.",
            )
            .action(ArgAction::SetTrue),
        Arg::new(fmt_flags::FMT_TAB_WIDTH)
            .short('T')
            .long("tab-width")
            .help(
                "Treat tabs as TABWIDTH spaces for \
                    determining line length, default 8. Note that this is used only for \
                    calculating line lengths; tabs are preserved in the output.",
            )
            .value_name("TABWIDTH"),
        Arg::new(fmt_flags::FMT_FILES)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::FilePath),
    ];
    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(test)]
    mod fmt_configs_tests {
        use std::fs;

        use tempfile::TempDir;

        use super::*;

        #[test]
        fn test_fmt_configs_with_file_crown_margin_long() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--crown-margin", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: true,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_crown_margin_short() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-c", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: true,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_tagged_paragraph_long() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--tagged-paragraph", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: true,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_tagged_paragraph_short() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-t", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: true,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_preserve_headers_long() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--preserve-headers", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: true,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_preserve_headers_short() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-m", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: true,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_split_only_long() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--split-only", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: true,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_split_only_short() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-s", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: true,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_uniform_spacing_long() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--uniform-spacing", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: true,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_uniform_spacing_short() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-u", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: true,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };

            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_with_space() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--prefix", " ", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(" ".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_with_letter() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--prefix", "a", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("a".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_with_digital() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--prefix", "5", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("5".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_with_comma() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--prefix", ",", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(",".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_with_semicolon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--prefix", ";", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(";".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_with_colon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--prefix", ":", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(":".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_with_vertical() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--prefix", "|", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("|".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_with_tab() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--prefix", "\t", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("	".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_with_group_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--prefix", "\u{001d}", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(
                    "\u{1d}"
                        .to_string()
                        .to_string()
                        .to_string()
                        .to_string()
                        .to_string()
                        .to_string()
                        .to_string(),
                ),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_with_unit_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--prefix", "\u{001f}", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\u{1f}".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_with_record_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--prefix", "\u{001e}", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\u{1e}".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_with_space() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--skip-prefix", " ", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some(" ".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_with_letter() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--skip-prefix", "a", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("a".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_with_digital() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--skip-prefix", "5", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("5".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_with_comma() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--skip-prefix", ",", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some(",".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_with_semicolon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--skip-prefix", ";", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some(";".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_with_colon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--skip-prefix", ":", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some(":".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_with_vertical() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--skip-prefix", "|", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("|".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_with_tab() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--skip-prefix", "\t", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("\t".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_with_group_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                "\u{001d}",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("\u{1d}".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_with_unit_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                "\u{001f}",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("\u{1f}".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_with_record_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                "\u{001e}",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("\u{1e}".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_with_space() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-P", " ", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some(" ".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_with_letter() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-P", "a", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("a".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_with_digital() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-P", "5", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("5".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_with_comma() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-P", ",", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some(",".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_with_semicolon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-P", ";", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some(";".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_with_colon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-P", ":", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some(":".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_with_vertical() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-P", "|", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("|".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_with_tab() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-P", "\t", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("\t".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_with_group_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-P", "\u{001d}", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("\u{1d}".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_with_unit_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-P", "\u{001f}", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("\u{1f}".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_with_record_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-P", "\u{001e}", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("\u{1e}".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_with_space() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", " ", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(" ".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_with_letter() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", "a", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("a".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_with_digital() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", "5", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("5".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_with_comma() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", ",", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(",".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_with_semicolon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", ";", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(";".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_with_colon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", ":", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(":".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_with_vertical() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", "|", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("|".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_with_tab() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", "\t", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\t".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };

            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_with_group_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", "\u{001d}", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\u{1d}".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_with_unit_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", "\u{001f}", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\u{1f}".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_with_record_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", "\u{001e}", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\u{1e}".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_prefix_long_with_space() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-p",
                " ",
                "--exact-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(" ".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_prefix_long_with_letter() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-p",
                "a",
                "--exact-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("a".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_prefix_long_with_digital() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-p",
                "5",
                "--exact-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("5".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_prefix_long_with_comma() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-p",
                ",",
                "--exact-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(",".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_prefix_long_with_semicolon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-p",
                ";",
                "--exact-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(";".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_prefix_long_with_colon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-p",
                ":",
                "--exact-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(":".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_prefix_long_with_vertical() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-p",
                "|",
                "--exact-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("|".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_prefix_long_with_tab() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-p",
                "\t",
                "--exact-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("	".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_prefix_long_with_group_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-p",
                "\u{001d}",
                "--exact-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\u{1d}".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_prefix_long_with_unit_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-p",
                "\u{001f}",
                "--exact-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\u{1f}".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_prefix_long_with_record_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-p",
                "\u{001e}",
                "--exact-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\u{1e}".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_prefix_short_with_space() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", " ", "-x", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(" ".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_prefix_short_with_letter() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", "a", "-x", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("a".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_prefix_short_with_digital() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", "5", "-x", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("5".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_prefix_short_with_comma() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", ",", "-x", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(",".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_prefix_short_with_semicolon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", ";", "-x", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(";".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_prefix_short_with_colon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", ":", "-x", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(":".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_prefix_short_with_vertical() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", "|", "-x", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("|".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_prefix_short_with_tab() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", "\t", "-x", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\t".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_prefix_short_with_group_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", "\u{001d}", "-x", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\u{1d}".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_prefix_short_with_unit_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", "\u{001f}", "-x", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\u{1f}".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_prefix_short_with_record_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", "\u{001e}", "-x", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\u{1e}".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_prefix_short_with_space() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                " ",
                "-x",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some(" ".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_prefix_short_with_letter() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                "a",
                "-x",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("a".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_prefix_short_with_digital() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                "5",
                "-x",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("5".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_prefix_short_with_comma() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                ",",
                "-x",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some(",".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_prefix_short_with_semicolon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                ";",
                "-x",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some(";".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_prefix_short_with_colon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                ":",
                "-x",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some(":".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_prefix_short_with_vertical() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                "|",
                "-x",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("|".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_prefix_short_with_tab() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                "\t",
                "-x",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("\t".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_prefix_short_with_group_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                "\u{001d}",
                "-x",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("\u{1d}".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_prefix_short_with_unit_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                "\u{001f}",
                "-x",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("\u{1f}".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_prefix_short_with_record_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                "\u{001e}",
                "-x",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("\u{1e}".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_prefix_long_with_space() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--prefix", " ", "-x", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(" ".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_prefix_long_with_letter() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--prefix", "a", "-x", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("a".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_prefix_long_with_digital() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--prefix", "5", "-x", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("5".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_prefix_long_with_comma() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--prefix", ",", "-x", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(",".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_prefix_long_with_semicolon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--prefix", ";", "-x", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(";".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_prefix_long_with_colon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--prefix", ":", "-x", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(":".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_prefix_long_with_vertical() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--prefix", "|", "-x", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("|".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_prefix_long_with_tab() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--prefix", "\t", "-x", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\t".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_prefix_long_with_group_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--prefix",
                "\u{001d}",
                "-x",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\u{1d}".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_prefix_long_with_unit_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--prefix",
                "\u{001f}",
                "-x",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\u{1f}".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_prefix_long_with_record_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--prefix",
                "\u{001e}",
                "-x",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\u{1e}".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_skip_prefix_long_with_space() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--prefix",
                " ",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(" ".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_skip_prefix_long_with_letter() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--prefix",
                "a",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("a".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_skip_prefix_long_with_digital() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--prefix",
                "5",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("5".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_skip_prefix_long_with_comma() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--prefix",
                ",",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(",".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_skip_prefix_long_with_semicolon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--prefix",
                ";",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(";".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_skip_prefix_long_with_colon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--prefix",
                ":",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(":".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_skip_prefix_long_with_vertical() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--prefix",
                "|",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("|".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_skip_prefix_long_with_tab() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--prefix",
                "\t",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("	".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_skip_prefix_long_with_group_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--prefix",
                "\u{001d}",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\u{1d}".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_skip_prefix_long_with_unit_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--prefix",
                "\u{001f}",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\u{1f}".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_skip_prefix_long_with_record_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--prefix",
                "\u{001e}",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\u{1e}".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_skip_prefix_long_with_space() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                " ",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some(" ".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_skip_prefix_long_with_letter() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                "a",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("a".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_skip_prefix_long_with_digital() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                "5",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("5".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_skip_prefix_long_with_comma() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                ",",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some(",".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_skip_prefix_long_with_semicolon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                ";",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some(";".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_skip_prefix_long_with_colon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                ":",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some(":".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_skip_prefix_long_with_vertical() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                "|",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("|".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_skip_prefix_long_with_tab() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                "\t",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("\t".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_skip_prefix_long_with_group_separator()
        {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                "\u{001d}",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("\u{1d}".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_skip_prefix_long_with_unit_separator()
        {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                "\u{001f}",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("\u{1f}".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_skip_prefix_long_with_record_separator(
        ) {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                "\u{001e}",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("\u{1e}".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_exact_skip_prefix_long_with_space() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-P",
                " ",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some(" ".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_exact_skip_prefix_long_with_letter() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-P",
                "a",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("a".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_exact_skip_prefix_long_with_digital() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-P",
                "5",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("5".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_exact_skip_prefix_long_with_comma() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-P",
                ",",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some(",".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_exact_skip_prefix_long_with_semicolon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-P",
                ";",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some(";".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_exact_skip_prefix_long_with_colon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-P",
                ":",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some(":".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_exact_skip_prefix_long_with_vertical() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-P",
                "|",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("|".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_exact_skip_prefix_long_with_tab() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-P",
                "\t",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("\t".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_exact_skip_prefix_long_with_group_separator(
        ) {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-P",
                "\u{001d}",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("\u{1d}".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_exact_skip_prefix_long_with_unit_separator()
        {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-P",
                "\u{001f}",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("\u{1f}".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_exact_skip_prefix_long_with_record_separator(
        ) {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-P",
                "\u{001e}",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("\u{1e}".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_skip_prefix_long_with_space() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-p",
                " ",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(" ".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_skip_prefix_long_with_letter() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-p",
                "a",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("a".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_skip_prefix_long_with_digital() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-p",
                "5",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("5".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_skip_prefix_long_with_comma() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-p",
                ",",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(",".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_skip_prefix_long_with_semicolon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-p",
                ";",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(";".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_skip_prefix_long_with_colon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-p",
                ":",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(":".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_skip_prefix_long_with_vertical() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-p",
                "|",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("|".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_skip_prefix_long_with_tab() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-p",
                "\t",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\t".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_skip_prefix_long_with_group_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-p",
                "\u{001d}",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\u{1d}".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_skip_prefix_long_with_unit_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-p",
                "\u{001f}",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\u{1f}".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_skip_prefix_long_with_record_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "-p",
                "\u{001e}",
                "--exact-skip-prefix",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\u{1e}".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_skip_prefix_short_with_space() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--prefix", " ", "-X", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(" ".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_skip_prefix_short_with_letter() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--prefix", "a", "-X", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("a".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_skip_prefix_short_with_digital() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--prefix", "5", "-X", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("5".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_skip_prefix_short_with_comma() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--prefix", ",", "-X", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(",".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_skip_prefix_short_with_semicolon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--prefix", ";", "-X", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(";".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_skip_prefix_short_with_colon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--prefix", ":", "-X", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(":".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_skip_prefix_short_with_vertical() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--prefix", "|", "-X", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("|".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_skip_prefix_short_with_tab() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--prefix", "\t", "-X", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("	".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_skip_prefix_short_with_group_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--prefix",
                "\u{001d}",
                "-X",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\u{1d}".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_skip_prefix_short_with_unit_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--prefix",
                "\u{001f}",
                "-X",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\u{1f}".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_long_exact_skip_prefix_short_with_record_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--prefix",
                "\u{001e}",
                "-X",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\u{1e}".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_skip_prefix_short_with_space() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                " ",
                "-X",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some(" ".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_skip_prefix_short_with_letter() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                "a",
                "-X",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("a".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_skip_prefix_short_with_digital() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                "5",
                "-X",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("5".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_skip_prefix_short_with_comma() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                ",",
                "-X",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some(",".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_skip_prefix_short_with_semicolon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                ";",
                "-X",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some(";".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_skip_prefix_short_with_colon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                ":",
                "-X",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some(":".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_skip_prefix_short_with_vertical() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                "|",
                "-X",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("|".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_skip_prefix_short_with_tab() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                "\t",
                "-X",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("\t".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_skip_prefix_short_with_group_separator(
        ) {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                "\u{001d}",
                "-X",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("\u{1d}".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_skip_prefix_short_with_unit_separator()
        {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                "\u{001f}",
                "-X",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("\u{1f}".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_long_exact_skip_prefix_short_with_record_separator(
        ) {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![
                ctcore::ct_util_name(),
                "--skip-prefix",
                "\u{001e}",
                "-X",
                file_name,
            ];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("\u{1e}".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_exact_skip_prefix_short_with_space() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-P", " ", "-X", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some(" ".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_exact_skip_prefix_short_with_letter() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-P", "a", "-X", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("a".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_exact_skip_prefix_short_with_digital() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-P", "5", "-X", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("5".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_exact_skip_prefix_short_with_comma() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-P", ",", "-X", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some(",".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_exact_skip_prefix_short_with_semicolon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-P", ";", "-X", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some(";".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_exact_skip_prefix_short_with_colon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-P", ":", "-X", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some(":".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_exact_skip_prefix_short_with_vertical() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-P", "|", "-X", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("|".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_exact_skip_prefix_short_with_tab() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-P", "\t", "-X", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("\t".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_exact_skip_prefix_short_with_group_separator(
        ) {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-P", "\u{001d}", "-X", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("\u{1d}".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_exact_skip_prefix_short_with_unit_separator(
        ) {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-P", "\u{001f}", "-X", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("\u{1f}".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_skip_prefix_short_exact_skip_prefix_short_with_record_separator(
        ) {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-P", "\u{001e}", "-X", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: Some("\u{1e}".to_string()),
                is_xanti_prefix: true,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_skip_prefix_short_with_space() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", " ", "-X", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(" ".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_skip_prefix_short_with_letter() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", "a", "-X", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("a".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_skip_prefix_short_with_digital() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", "5", "-X", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("5".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_skip_prefix_short_with_comma() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", ",", "-X", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(",".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_skip_prefix_short_with_semicolon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", ";", "-X", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(";".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_skip_prefix_short_with_colon() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", ":", "-X", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some(":".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_skip_prefix_short_with_vertical() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", "|", "-X", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("|".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_skip_prefix_short_with_tab() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", "\t", "-X", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\t".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_skip_prefix_short_with_group_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", "\u{001d}", "-X", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\u{1d}".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_skip_prefix_short_with_unit_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", "\u{001f}", "-X", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\u{1f}".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };

            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_prefix_short_exact_skip_prefix_short_with_record_separator() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-p", "\u{001e}", "-X", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: Some("\u{1e}".to_string()),
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_width_long_0() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--width", "0", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 0,
                goal: 0,
                tab_width: 8,
            };

            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_width_long_1() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--width", "1", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 1,
                goal: 1,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_width_long_10() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--width", "10", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 10,
                goal: 9,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_width_long_100() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--width", "100", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 100,
                goal: 93,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_width_long_1000() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--width", "1000", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 1000,
                goal: 930,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_width_short_0() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-w", "0", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 0,
                goal: 0,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_width_short_1() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-w", "1", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 1,
                goal: 1,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_width_short_10() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-w", "10", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 10,
                goal: 9,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_width_short_100() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-w", "100", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 100,
                goal: 93,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_width_short_1000() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-w", "1000", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 1000,
                goal: 930,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_goal_long_none() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--goal", file_name];
            let matches = command.try_get_matches_from(cmd_args);

            assert_eq!(
                matches.unwrap_err().kind(),
                clap::error::ErrorKind::ValueValidation
            );
        }

        #[test]
        fn test_fmt_configs_with_file_goal_long_0() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--goal", "0", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 3,
                goal: 0,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_goal_long_1() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--goal", "1", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 4,
                goal: 1,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_goal_long_10() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--goal", "10", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 13,
                goal: 10,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_goal_long_100() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--goal", "100", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap_err();
            assert_eq!(configs.code(), 1);
            assert_eq!(format!("{}", configs), "GOAL cannot be greater than WIDTH.");
        }

        #[test]
        fn test_fmt_configs_with_file_goal_long_1000() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--goal", "1000", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap_err();
            assert_eq!(configs.code(), 1);
            assert_eq!(format!("{}", configs), "GOAL cannot be greater than WIDTH.");
        }

        #[test]
        fn test_fmt_configs_with_file_goal_short_none() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-g", file_name];
            let matches = command.try_get_matches_from(cmd_args);

            assert_eq!(
                matches.unwrap_err().kind(),
                clap::error::ErrorKind::ValueValidation
            );
        }

        #[test]
        fn test_fmt_configs_with_file_goal_short_0() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-g", "0", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 3,
                goal: 0,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_goal_short_1() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-g", "1", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 4,
                goal: 1,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_goal_short_10() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-g", "10", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 13,
                goal: 10,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_goal_short_100() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-g", "100", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap_err();
            assert_eq!(configs.code(), 1);
            assert_eq!(format!("{}", configs), "GOAL cannot be greater than WIDTH.");
        }

        #[test]
        fn test_fmt_configs_with_file_goal_short_1000() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-g", "1000", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap_err();
            assert_eq!(configs.code(), 1);
            assert_eq!(format!("{}", configs), "GOAL cannot be greater than WIDTH.");
        }

        #[test]
        fn test_fmt_configs_with_file_quick_long() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--quick", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: true,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_quick_short() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-q", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: true,
                width: 75,
                goal: 70,
                tab_width: 8,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_tab_width_long_0() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--tab-width", "0", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 1,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_tab_width_long_1() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--tab-width", "1", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 1,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_tab_width_long_10() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--tab-width", "10", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 10,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_tab_width_long_100() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--tab-width", "100", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 100,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_tab_width_long_1000() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "--tab-width", "1000", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 1000,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_tab_width_short_0() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-T", "0", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 1,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_tab_width_short_1() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-T", "1", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 1,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_tab_width_short_10() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-T", "10", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 10,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_tab_width_short_100() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-T", "100", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 100,
            };
            assert_eq!(configs, expected_configs);
        }

        #[test]
        fn test_fmt_configs_with_file_tab_width_short_1000() {
            let tmp_dir = TempDir::with_prefix("test_fmt_").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join("test_fmt_file");
            File::create(&test_file_path).unwrap();
            let _ = fs::write(&test_file_path, b"qqqqq\nwwwwww\neeeeee\nrrrrrr\n");
            let file_name = test_file_path.to_str().unwrap();
            let command = ct_app();
            let cmd_args = vec![ctcore::ct_util_name(), "-T", "1000", file_name];
            let matches = command.try_get_matches_from(cmd_args).unwrap();
            let configs = FmtConfigs::from_matches(&matches).unwrap();
            let expected_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: true,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 75,
                goal: 70,
                tab_width: 1000,
            };
            assert_eq!(configs, expected_configs);
        }
    }
}