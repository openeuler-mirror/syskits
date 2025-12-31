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

//dircolors 命令在Linux系统中主要用于控制 ls 命令显示目录和文件时使用的颜色
use std::borrow::Borrow;
use std::env;
use std::fs::File;

use std::io::{BufRead, BufReader};
use std::path::Path;

use clap::{crate_version, Arg, ArgAction, ArgMatches, Command};
use ctcore::ct_colors::{CT_FILE_ATTRIBUTE_CODES, CT_FILE_COLORS, CT_FILE_TYPES, CT_TERMS};
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CTsageError, CtSimpleError};
use ctcore::{ct_help_about, ct_help_section, ct_help_usage};

mod opt_flags {
    pub const BOURNE_SHELL: &str = "bourne-shell";
    pub const C_SHELL: &str = "c-shell";
    pub const PRINT_DATABASE: &str = "print-database";
    pub const PRINT_LS_COLORS: &str = "print-ls-colors";
    pub const FILE: &str = "FILE";
}

const DIRCOLORS_USAGE: &str = ct_help_usage!("dircolors.md");
const DIRCOLORS_ABOUT: &str = ct_help_about!("dircolors.md");
const DIRCOLORS_AFTER_HELP: &str = ct_help_section!("after help", "dircolors.md");

#[derive(PartialEq, Eq, Debug)]
pub enum DircolorsOutputFmt {
    Shell,
    CShell,
    Display,
    Unknown,
}

pub fn dircolors_guess_syntax() -> DircolorsOutputFmt {
    match env::var("SHELL") {
        Ok(ref s) if !s.is_empty() => {
            let shell_path: &Path = s.as_ref();
            if let Some(name) = shell_path.file_name() {
                if name == "csh" || name == "tcsh" {
                    DircolorsOutputFmt::CShell
                } else {
                    DircolorsOutputFmt::Shell
                }
            } else {
                DircolorsOutputFmt::Shell
            }
        }
        _ => DircolorsOutputFmt::Unknown,
    }
}

fn dircolors_get_colors_format_strings(fmt: &DircolorsOutputFmt) -> (String, String) {
    let prefix = match fmt {
        DircolorsOutputFmt::Shell => "LS_COLORS='".to_string(),
        DircolorsOutputFmt::CShell => "setenv LS_COLORS '".to_string(),
        DircolorsOutputFmt::Display => String::new(),
        DircolorsOutputFmt::Unknown => unreachable!(),
    };

    let suffix = match fmt {
        DircolorsOutputFmt::Shell => "';\nexport LS_COLORS".to_string(),
        DircolorsOutputFmt::CShell => "'".to_string(),
        DircolorsOutputFmt::Display => String::new(),
        DircolorsOutputFmt::Unknown => unreachable!(),
    };

    (prefix, suffix)
}

pub fn dircolors_generate_type_output(fmt: &DircolorsOutputFmt) -> String {
    match fmt {
        DircolorsOutputFmt::Display => CT_FILE_TYPES
            .iter()
            .map(|&(_, key, val)| format!("\x1b[{}m{}\t{}\x1b[0m", val, key, val))
            .collect::<Vec<String>>()
            .join("\n"),
        _ => {
            // Existing logic for other formats
            CT_FILE_TYPES
                .iter()
                .map(|&(_, v1, v2)| format!("{}={}", v1, v2))
                .collect::<Vec<String>>()
                .join(":")
        }
    }
}

/**
 * 生成用于配置ls颜色的字符串。
 *
 * 根据提供的`fmt`和`sep`参数，生成一个配置ls显示颜色的字符串。主要支持两种格式：
 * 1. `Display`格式，用于直接显示颜色样式的字符串，每个文件类型扩展名及其颜色代码会以分隔符`\n`分隔。
 * 2. 其他格式，用于生成`.bashrc`或其他配置文件中使用的`LS_COLORS`环境变量字符串格式。
 *
 */
fn dircolors_generate_ls_colors(fmt: &DircolorsOutputFmt, sep: &str) -> String {
    match fmt {
        DircolorsOutputFmt::Display => {
            // 为显示格式生成颜色配置字符串
            let mut display_parts = vec![];
            let type_output = dircolors_generate_type_output(fmt);
            display_parts.push(type_output);
            // 遍历文件类型颜色映射，生成带颜色的扩展名展示
            for &(extension, code) in CT_FILE_COLORS {
                let prefix = if extension.starts_with('*') { "" } else { "*" };
                let formatted_extension =
                    format!("\x1b[{}m{}{}\t{}\x1b[0m", code, prefix, extension, code);
                display_parts.push(formatted_extension);
            }
            // 用换行符连接所有部分并返回
            display_parts.join("\n")
        }
        _ => {
            // 为LS_COLORS环境变量格式生成颜色配置字符串
            let mut parts = vec![];
            // 格式化每个文件扩展名及其颜色代码
            for &(extension, code) in CT_FILE_COLORS {
                let prefix = if extension.starts_with('*') { "" } else { "*" };
                let formatted_extension = format!("{}{}", prefix, extension);
                parts.push(format!("{}={}", formatted_extension, code));
            }
            // 根据输出格式，获取前缀和后缀，并组装最终字符串
            let (prefix, suffix) = dircolors_get_colors_format_strings(fmt);
            let ls_colors = parts.join(sep);
            format!(
                "{}{}:{}:{}",
                prefix,
                dircolors_generate_type_output(fmt),
                ls_colors,
                suffix
            )
        }
    }
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    dircolors_main(args).map(|_| ())
}

/**
 * 命令行应用程序的主函数，用于解析参数并执行相应的操作，如打印目录颜色配置。
 *
 * # 参数
 * `args`: 实现了 `ctcore::Args` 的类型，提供访问命令行参数的方法。
 *
 * # 返回值
 * 一个 `CTResult<()>`，其中 `Ok(())` 表示成功，`Err(_)` 表示错误，并带有描述性消息。
 */
pub fn dircolors_main(args: impl ctcore::Args) -> CTResult<()> {
    // 使用clap库解析命令行参数。
    let args_match = ct_app().try_get_matches_from(args)?;

    // 提取文件参数，如果有的话。
    let files = args_match
        .get_many::<String>(opt_flags::FILE)
        .map_or(vec![], |file_values| file_values.collect());

    if let Some(value) = dircolors_parm_conflict_check(&args_match) {
        return value;
    }

    // 检查 `--print-database` 和 `--print-ls-colors` 选项之间的互斥性。
    if let Some(value) = dircolors_print_parm_check(&args_match) {
        return value;
    }

    // 处理 `--print-database` 选项。
    if let Some(value) = dircolors_print_database_check(&args_match, &files) {
        return value;
    }

    // 根据提供的选项确定输出格式。
    let mut out_format = dircolors_out_format_check(&args_match);

    // 如果输出格式未知，尝试猜测它。
    if out_format == DircolorsOutputFmt::Unknown {
        match dircolors_guess_syntax() {
            DircolorsOutputFmt::Unknown => {
                return Err(CtSimpleError::new(
                    1,
                    "no SHELL environment variable, and no shell type option given", //"未设置SHELL环境变量，且未提供shell类型选项",
                ));
            }
            fmt => out_format = fmt,
        }
    }

    // 根据确定的输出格式和文件处理输入。
    dircolors_output_format_process(files, &mut out_format).unwrap_or_else(|value| value)
}

fn dircolors_output_format_process(
    files: Vec<&String>,
    out_format: &mut DircolorsOutputFmt,
) -> Result<CTResult<()>, CTResult<()>> {
    let result;
    if files.is_empty() {
        println!("{}", dircolors_generate_ls_colors(out_format, ":"));
        return Err(Ok(()));
    } else if files.len() > 1 {
        return Err(Err(CTsageError::new(
            1,
            format!("extra operand {}", files[1].quote()), //"多余的参数
        )));
    } else if files[0].eq("-") {
        let fin = BufReader::new(std::io::stdin());
        // 当 "-" 作为文件指定时，从stdin处理输入。
        result = dircolors_parse(fin.lines().map_while(Result::ok), out_format, files[0]);
    } else {
        // 处理单个文件输入。
        let path = Path::new(files[0]);
        if path.is_dir() {
            return Err(Err(CtSimpleError::new(
                2,
                format!("expected file, got directory {}", path.quote()), //期望的文件，但得到的是目录
            )));
        }
        match File::open(path) {
            Ok(f) => {
                let fin = BufReader::new(f);
                result = dircolors_parse(
                    fin.lines().map_while(Result::ok),
                    out_format,
                    &path.to_string_lossy(),
                );
            }
            Err(e) => {
                return Err(Err(CtSimpleError::new(
                    1,
                    format!("{}: {}", path.maybe_quote(), e),
                )));
            }
        }
    }

    // 最后，打印结果或错误消息。
    Ok(match result {
        Ok(s) => {
            println!("{s}");
            Ok(())
        }
        Err(s) => Err(CtSimpleError::new(1, s)),
    })
}

fn dircolors_out_format_check(args_match: &ArgMatches) -> DircolorsOutputFmt {
    if args_match.get_flag(opt_flags::C_SHELL) {
        DircolorsOutputFmt::CShell
    } else if args_match.get_flag(opt_flags::BOURNE_SHELL) {
        DircolorsOutputFmt::Shell
    } else if args_match.get_flag(opt_flags::PRINT_LS_COLORS) {
        DircolorsOutputFmt::Display
    } else {
        DircolorsOutputFmt::Unknown
    }
}

fn dircolors_print_database_check(
    args_match: &ArgMatches,
    files: &[&String],
) -> Option<CTResult<()>> {
    if args_match.get_flag(opt_flags::PRINT_DATABASE) {
        if !files.is_empty() {
            return Some(Err(CTsageError::new(
                1,
                format!(
                    "extra operand {}\nfile operands cannot be combined with \
                     --print-database (-p)", //"多余的参数 {}\n不能将文件参数与 `--print-database (-p)` 结合使用",
                    files[0].quote()
                ),
            )));
        }

        println!("{}", generate_dircolors_config());
        return Some(Ok(()));
    }
    None
}

fn dircolors_print_parm_check(args_match: &ArgMatches) -> Option<CTResult<()>> {
    if args_match.get_flag(opt_flags::PRINT_DATABASE)
        && args_match.get_flag(opt_flags::PRINT_LS_COLORS)
    {
        return Some(Err(CTsageError::new(
            1,
            "options --print-database and --print-ls-colors are mutually exclusive", //"选项 `--print-database` 和 `--print-ls-colors` 互斥",
        )));
    }
    None
}

fn dircolors_parm_conflict_check(args_match: &ArgMatches) -> Option<CTResult<()>> {
    // 手动检查选项冲突，以匹配GNU coreutils的行为。
    if (args_match.get_flag(opt_flags::C_SHELL) || args_match.get_flag(opt_flags::BOURNE_SHELL))
        && (args_match.get_flag(opt_flags::PRINT_DATABASE)
            || args_match.get_flag(opt_flags::PRINT_LS_COLORS))
    {
        return Some(Err(CTsageError::new(
            1,
            "the options to output non shell syntax,\n\
             and to select a shell syntax are mutually exclusive",
        )));
    }
    None
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = DIRCOLORS_ABOUT;
    let usage_description = ct_format_usage(DIRCOLORS_USAGE);

    let args = dircolors_args_init();

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .after_help(DIRCOLORS_AFTER_HELP)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(&args)
}

fn dircolors_args_init() -> Vec<Arg> {
    let args = vec![
        Arg::new(opt_flags::BOURNE_SHELL)
            .long("sh")
            .short('b')
            .visible_alias("bourne-shell")
            .overrides_with(opt_flags::C_SHELL)
            .help("output Bourne shell code to set LS_COLORS")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::C_SHELL)
            .long("csh")
            .short('c')
            .visible_alias("c-shell")
            .overrides_with(opt_flags::BOURNE_SHELL)
            .help("output C shell code to set LS_COLORS")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::PRINT_DATABASE)
            .long("print-database")
            .short('p')
            .help("print the byte counts")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::PRINT_LS_COLORS)
            .long("print-ls-colors")
            .help("output fully escaped colors for display")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::FILE)
            .hide(true)
            .value_hint(clap::ValueHint::FilePath)
            .action(ArgAction::Append),
    ];
    args
}

/// 定义了`DircolorsStrUtils` trait，用于字符串的处理和匹配。
pub trait DircolorsStrUtils {
    /// 移除注释和去除空白字符。
    ///
    /// # 返回值
    /// 返回处理后的字符串的引用。
    fn purify(&self) -> &Self;

    /// 类似于`split_whitespace()`，但只分割成两个组件。
    ///
    /// # 返回值
    /// 返回一个元组，包含两个字符串的引用：第一个是键，第二个是值。
    fn split_two(&self) -> (&str, &str);

    /// 使用POSIX风格的模式匹配对字符串进行匹配。
    ///
    /// # 参数
    /// `pattern` - 用于匹配的模式字符串。
    ///
    /// # 返回值
    /// 如果字符串匹配模式，则返回`true`，否则返回`false`。
    fn fnmatch(&self, pattern: &str) -> bool;
}

impl DircolorsStrUtils for str {
    // 移除注释和去除首尾空白字符。
    //
    // 遍历字符串，找到第一个`'#'`字符，并检查其前是否有空白字符。
    // 如果有，移除从该`'#'`字符开始的全部内容；如果没有，移除首尾的空白字符。
    fn purify(&self) -> &Self {
        let mut line = self;
        for (n, _) in self
            .as_bytes()
            .iter()
            .enumerate()
            .filter(|(_, c)| **c == b'#')
        {
            match self[..n].chars().last() {
                Some(c) if c.is_whitespace() => {
                    line = &self[..n - c.len_utf8()];
                    break;
                }
                None => {
                    line = &self[..0];
                    break;
                }
                _ => (),
            }
        }
        line.trim()
    }

    // 分割字符串至第一个空白字符，并返回两部分。
    //
    // 如果字符串中没有空白字符，则返回一个空字符串作为第二个元素。
    fn split_two(&self) -> (&str, &str) {
        if let Some(b) = self.find(char::is_whitespace) {
            let key = &self[..b];
            if let Some(e) = self[b..].find(|c: char| !c.is_whitespace()) {
                (key, &self[b + e..])
            } else {
                (key, "")
            }
        } else {
            ("", "")
        }
    }

    // 使用给定的模式字符串匹配当前字符串。
    //
    // 利用`ct_parse_glob`库将模式字符串转换为匹配器，然后对当前字符串进行匹配。
    fn fnmatch(&self, pat: &str) -> bool {
        ct_parse_glob::ct_from_str(pat).unwrap().matches(self)
    }
}

#[derive(PartialEq)]
enum DircolorsParseState {
    Global,
    Matched,
    Continue,
    Pass,
}

use ctcore::{ct_format_usage, ct_parse_glob};

#[allow(clippy::cognitive_complexity)]
/// 解析dircolors格式的输入，生成相应的配置字符串。
///
/// # 参数
/// - `user_input`: 输入的数据，需要能够迭代器遍历字符串。
/// - `fmt`: 指定输出格式，决定如何处理解析结果。
/// - `fp`: 文件路径或者名称，用于在错误信息中引用。
///
/// # 返回值
/// - `Result<String, String>`: 成功时返回解析后的字符串，失败时返回错误信息字符串。
#[allow(clippy::cognitive_complexity)]
fn dircolors_parse<T>(user_input: T, fmt: &DircolorsOutputFmt, fp: &str) -> Result<String, String>
where
    T: IntoIterator,
    T::Item: Borrow<str>,
{
    // 初始化结果字符串，预设容量以减少动态扩容的消耗。
    let mut result = String::with_capacity(1790);
    // 根据输出格式获取前缀和后缀字符串。
    let (prefix, suffix) = dircolors_get_colors_format_strings(fmt);

    // 添加输出格式的前缀。
    result.push_str(&prefix);

    // 获取当前环境的TERM值，作为匹配条件之一。
    let term = env::var("TERM").unwrap_or_else(|_| "none".to_owned());
    let term = term.as_str();

    // 初始化解析状态为全局（Global）。
    let mut parse_state = DircolorsParseState::Global;

    // 遍历输入，逐行处理。
    for (num, line) in user_input.into_iter().enumerate() {
        let num = num + 1; // 行号从1开始计数。
        let line = line.borrow().purify();
        // 跳过空行。
        if line.is_empty() {
            continue;
        }

        // 对行进行转义处理。
        let line = dircolors_escape(line);

        // 分割键值对。
        let (key, value) = line.split_two();
        // 如果值为空，则报错。
        if value.is_empty() {
            return Err(format!(
                // 错误信息格式遵循GNU的风格，使用双空格分隔。
                "{}:{}: invalid line;  missing second token",
                fp.maybe_quote(),
                num
            ));
        }
        // 将键转换为小写，以支持不区分大小写的匹配。
        let lower = key.to_lowercase();
        // 处理TERM或COLORTERM匹配逻辑。
        if lower == "term" || lower == "colorterm" {
            if term.fnmatch(value) {
                parse_state = DircolorsParseState::Matched;
            } else if parse_state != DircolorsParseState::Matched {
                parse_state = DircolorsParseState::Pass;
            }
        } else {
            // 如果之前已匹配到TERM，则后续不同的TERM不会取消之前的输入。
            if parse_state == DircolorsParseState::Matched {
                parse_state = DircolorsParseState::Continue;
            }
            // 根据状态处理键值对。
            if parse_state != DircolorsParseState::Pass {
                let search_key = lower.as_str();

                // 根据键的前缀（.或*）和特定的键值处理逻辑。
                if key.starts_with('.') {
                    if *fmt == DircolorsOutputFmt::Display {
                        result.push_str(format!("\x1b[{value}m*{key}\t{value}\x1b[0m\n").as_str());
                    } else {
                        result.push_str(format!("*{key}={value}:").as_str());
                    }
                } else if key.starts_with('*') {
                    if *fmt == DircolorsOutputFmt::Display {
                        result.push_str(format!("\x1b[{value}m{key}\t{value}\x1b[0m\n").as_str());
                    } else {
                        result.push_str(format!("{key}={value}:").as_str());
                    }
                } else if lower == "options" || lower == "color" || lower == "eightbit" {
                    // 忽略特定的键，此处为 Slackware 的特有处理。
                } else if let Some((_, s)) = CT_FILE_ATTRIBUTE_CODES
                    .iter()
                    .find(|&&(key, _)| key == search_key)
                {
                    if *fmt == DircolorsOutputFmt::Display {
                        result.push_str(format!("\x1b[{value}m{s}\t{value}\x1b[0m\n").as_str());
                    } else {
                        result.push_str(format!("{s}={value}:").as_str());
                    }
                } else {
                    // 如果遇到未识别的关键词，则报错。
                    return Err(format!(
                        "{}:{}: unrecognized keyword {}",
                        fp.maybe_quote(),
                        num,
                        key
                    ));
                }
            }
        }
    }

    // 如果输出格式为Display，则移除最后一个换行符。
    if fmt == &DircolorsOutputFmt::Display {
        result.pop();
    }
    // 添加输出格式的后缀。
    result.push_str(&suffix);

    Ok(result)
}

/// Escape single quotes because they are not allowed between single quotes in shell code, and code
/// enclosed by single quotes is what is returned by `parse()`.
///
/// We also escape ":" to make the "quote" test pass in the GNU test suite:
/// <https://github.com/coreutils/coreutils/blob/master/tests/misc/dircolors.pl>
fn dircolors_escape(s: &str) -> String {
    let mut result = String::new();
    let mut previous = ' ';

    for c in s.chars() {
        match c {
            '\'' => result.push_str("'\\''"),
            ':' if previous != '\\' => result.push_str("\\:"),
            _ => result.push_str(&c.to_string()),
        }
        previous = c;
    }

    result
}

pub fn generate_dircolors_config() -> String {
    let mut config = String::new();

    config.push_str(
        "\
         # Configuration file for dircolors, a utility to help you set the\n\
         # LS_COLORS environment variable used by GNU ls with the --color option.\n\
         # The keywords COLOR, OPTIONS, and EIGHTBIT (honored by the\n\
         # slackware version of dircolors) are recognized but ignored.\n\
         # Global config options can be specified before TERM or COLORTERM entries\n\
         # Below are TERM or COLORTERM entries, which can be glob patterns, which\n\
         # restrict following config to systems with matching environment variables.\n\
        ",
    );
    config.push_str("COLORTERM ?*\n");
    for term in CT_TERMS {
        config.push_str(&format!("TERM {}\n", term));
    }

    config.push_str(
        "\
        # Below are the color init strings for the basic file types.\n\
        # One can use codes for 256 or more colors supported by modern terminals.\n\
        # The default color codes use the capabilities of an 8 color terminal\n\
        # with some additional attributes as per the following codes:\n\
        # Attribute codes:\n\
        # 00=none 01=bold 04=underscore 05=blink 07=reverse 08=concealed\n\
        # Text color codes:\n\
        # 30=black 31=red 32=green 33=yellow 34=blue 35=magenta 36=cyan 37=white\n\
        # Background color codes:\n\
        # 40=black 41=red 42=green 43=yellow 44=blue 45=magenta 46=cyan 47=white\n\
        #NORMAL 00 # no color code at all\n\
        #FILE 00 # regular file: use no color at all\n\
        ",
    );

    for (name, _, code) in CT_FILE_TYPES {
        config.push_str(&format!("{} {}\n", name, code));
    }

    config.push_str("# List any file extensions like '.gz' or '.tar' that you would like ls\n");
    config.push_str("# to color below. Put the extension, a space, and the color init string.\n");

    for (ext, color) in CT_FILE_COLORS {
        config.push_str(&format!("{} {}\n", ext, color));
    }
    config.push_str("# Subsequent TERM or COLORTERM entries, can be used to add / override\n");
    config.push_str("# config specific to those matching environment variables.");

    config
}

#[cfg(test)]
mod tests {
    use super::dircolors_escape;

    #[test]
    fn test_escape() {
        assert_eq!("", dircolors_escape(""));
        assert_eq!("'\\''", dircolors_escape("'"));
        assert_eq!("\\:", dircolors_escape(":"));
        assert_eq!("\\:", dircolors_escape("\\:"));
    }

    mod tests_dircolors_main {
        use crate::dircolors_main;

        use std::ffi::OsString;

        #[test]
        fn test_dircolors_main_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];

            let result = dircolors_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_dircolors_main_v() {
            let args = vec![ctcore::ct_util_name(), "-V"];
            let result = dircolors_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_dircolors_main_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = dircolors_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_dircolors_main_h() {
            let args = vec![ctcore::ct_util_name(), "-h"];
            let result = dircolors_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_dircolors_main_b() {
            let args = vec![ctcore::ct_util_name(), "-b"];
            let result = dircolors_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_dircolors_main_sh() {
            let args = vec![ctcore::ct_util_name(), "--sh"];
            let result = dircolors_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_dircolors_main_c() {
            let args = vec![ctcore::ct_util_name(), "-c"];
            let result = dircolors_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_dircolors_main_csh() {
            let args = vec![ctcore::ct_util_name(), "--csh"];
            let result = dircolors_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_dircolors_main_p() {
            let args = vec![ctcore::ct_util_name(), "-p"];
            let result = dircolors_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_dircolors_main_print_database() {
            let args = vec![ctcore::ct_util_name(), "--print-database"];
            let result = dircolors_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_dircolors_main_print_ls_colors() {
            let args = vec![ctcore::ct_util_name(), "--print-ls-colors"];
            let result = dircolors_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }
    }
}