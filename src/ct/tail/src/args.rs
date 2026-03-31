/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

use std::ffi::OsString;
use std::io::IsTerminal;
use std::time::Duration;

use clap::{crate_version, value_parser};
use clap::{Arg, ArgAction, ArgMatches, Command};
use fundu::{DurationParser, SaturatingInto};
use same_file::Handle;

use ctcore::ct_error::{CTResult, CTsageError, CtSimpleError};
use ctcore::ct_parse_size::{parse_size_u64, ParseSizeError};
use ctcore::{ct_format_usage, ct_help_about, ct_help_usage, ct_show_warning};

use crate::paths::TailInput;
use crate::{parse, platform, Quotable};

const TAIL_ABOUT: &str = ct_help_about!("tail.md");
const TAIL_USAGE: &str = ct_help_usage!("tail.md");

/// tail_flags 模块定义了 tail 命令的所有命令行参数标志和常量。
/// 这些常量用于构建命令行参数解析器和处理用户输入。
pub mod tail_flags {
    /// verbosity 子模块包含控制输出详细程度的标志
    pub mod verbosity {
        /// 安静模式标志，禁止输出文件名头部
        /// 使用方式：--quiet 或 -q
        pub const TAIL_QUIET: &str = "quiet";
        
        /// 详细模式标志，总是输出文件名头部
        /// 使用方式：--verbose 或 -v
        pub const TAIL_VERBOSE: &str = "verbose";
    }

    /// 指定要输出的字节数
    /// 使用方式：--bytes=N 或 -c N
    /// 可以使用 +N 表示从第N个字节开始，-N 表示最后N个字节
    pub const TAIL_BYTES: &str = "bytes";

    /// 跟随模式标志，持续输出文件新增内容
    /// 使用方式：--follow[=method] 或 -f
    /// method 可以是 descriptor（默认）或 name
    pub const TAIL_FOLLOW: &str = "follow";

    /// 指定要输出的行数
    /// 使用方式：--lines=N 或 -n N
    /// 可以使用 +N 表示从第N行开始，-N 表示最后N行
    pub const TAIL_LINES: &str = "lines";

    /// 指定进程ID，当该进程终止时停止跟随
    /// 使用方式：--pid=N
    /// 仅在跟随模式（--follow）下有效
    pub const TAIL_PID: &str = "pid";

    /// 指定在跟随模式下的睡眠间隔（秒）
    /// 使用方式：--sleep-interval=N 或 -s N
    /// 默认为1秒
    pub const TAIL_SLEEP_INT: &str = "sleep-interval";

    /// 使用空字符（\0）作为行分隔符
    /// 使用方式：--zero-terminated 或 -z
    /// 默认使用换行符（\n）作为分隔符
    pub const TAIL_ZERO_TERM: &str = "zero-terminated";

    /// 禁用 inotify 支持（Linux特有）
    /// 这是一个内部标志，主要用于测试
    pub const TAIL_DISABLE_INOTIFY_TERM: &str = "-disable-inotify";

    /// 强制使用轮询方式检测文件变化
    /// 使用方式：--use-polling
    /// 禁用系统特有的文件变化通知机制
    pub const TAIL_USE_POLLING: &str = "use-polling";

    /// 在文件不可访问时持续重试
    /// 使用方式：--retry
    /// 仅在跟随模式（--follow）下有效
    pub const TAIL_RETRY: &str = "retry";

    /// 组合了 --follow=name 和 --retry 的快捷方式
    /// 使用方式：-F
    pub const TAIL_FOLLOW_RETRY: &str = "F";

    /// 指定检查文件状态改变的最大次数
    /// 使用方式：--max-unchanged-stats=N
    /// 仅在使用轮询且按名称跟随时有效
    pub const TAIL_MAX_UNCHANGED_STATS: &str = "max-unchanged-stats";

    /// 指定要处理的文件列表
    /// 这是一个位置参数，可以指定多个文件
    /// 如果未指定，默认为标准输入
    pub const TAIL_ARG_FILES: &str = "files";

    /// 假定输入来自管道
    /// 这是一个内部标志，用于特殊场景
    pub const TAIL_PRESUME_INPUT_PIPE: &str = "-presume-input-pipe";
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TailSignum {
    Negative(u64),
    Positive(u64),
    PlusZero,
    MinusZero,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TailFilterMode {
    Bytes(TailSignum),

    /// Mode for lines delimited by delimiter as u8
    Lines(TailSignum, u8),
}

impl TailFilterMode {
    fn from_obsolete_args(args: &parse::TailObsoleteArgs) -> Self {
        let signum = if args.plus {
            TailSignum::Positive(args.num)
        } else {
            TailSignum::Negative(args.num)
        };
        if args.lines {
            Self::Lines(signum, b'\n')
        } else {
            Self::Bytes(signum)
        }
    }

    fn from(matches: &ArgMatches) -> CTResult<Self> {
        let zero_term = matches.get_flag(tail_flags::TAIL_ZERO_TERM);
        let mode = if let Some(arg) = matches.get_one::<String>(tail_flags::TAIL_BYTES) {
            match tail_parse_num(arg) {
                Ok(signum) => Self::Bytes(signum),
                Err(e) => {
                    return Err(CtSimpleError::new(
                        1,
                        format!("invalid number of bytes: '{e}'"),
                    ));
                }
            }
        } else if let Some(arg) = matches.get_one::<String>(tail_flags::TAIL_LINES) {
            match tail_parse_num(arg) {
                Ok(signum) => {
                    let delimiter = if zero_term { 0 } else { b'\n' };
                    Self::Lines(signum, delimiter)
                }
                Err(e) => {
                    return Err(CtSimpleError::new(
                        1,
                        format!("invalid number of lines: {e}"),
                    ));
                }
            }
        } else if zero_term {
            Self::default_zero()
        } else {
            Self::default()
        };

        Ok(mode)
    }

    fn default_zero() -> Self {
        Self::Lines(TailSignum::Negative(10), 0)
    }
}

impl Default for TailFilterMode {
    fn default() -> Self {
        Self::Lines(TailSignum::Negative(10), b'\n')
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum TailFollowMode {
    Descriptor,
    Name,
}

#[derive(Debug)]
pub enum TailVerificationResult {
    Ok,
    CannotFollowStdinByName,
    NoOutput,
}

#[derive(Debug)]
pub struct TailOptions {
    pub follow: Option<TailFollowMode>,
    pub max_unchanged_stats: u32,
    pub mode: TailFilterMode,
    pub pid: platform::Pid,
    pub retry: bool,
    pub sleep_sec: Duration,
    pub use_polling: bool,
    pub verbose: bool,
    pub presume_input_pipe: bool,
    /// `FILE(s)` positional arguments
    pub inputs: Vec<TailInput>,
}

impl Default for TailOptions {
    fn default() -> Self {
        Self {
            max_unchanged_stats: 5,
            sleep_sec: Duration::from_secs_f32(1.0),
            follow: Option::default(),
            mode: TailFilterMode::default(),
            pid: Default::default(),
            retry: Default::default(),
            use_polling: Default::default(),
            verbose: Default::default(),
            presume_input_pipe: Default::default(),
            inputs: Vec::default(),
        }
    }
}

impl TailOptions {
    pub fn from_obsolete_args(args: &parse::TailObsoleteArgs, name: Option<&OsString>) -> Self {
        let mut settings = Self::default();
        if args.follow {
            settings.follow = if name.is_some() {
                Some(TailFollowMode::Name)
            } else {
                Some(TailFollowMode::Descriptor)
            };
        }
        settings.mode = TailFilterMode::from_obsolete_args(args);
        let input = if let Some(name) = name {
            TailInput::from(name)
        } else {
            TailInput::default()
        };
        settings.inputs.push(input);
        settings
    }

    pub fn from(matches: &clap::ArgMatches) -> CTResult<Self> {
        // We're parsing --follow, -F and --retry under the following conditions:
        // * -F sets --retry and --follow=name
        // * plain --follow or short -f is the same like specifying --follow=descriptor
        // * All these options and flags can occur multiple times as command line arguments
        let follow_retry = matches.get_flag(tail_flags::TAIL_FOLLOW_RETRY);
        // We don't need to check for occurrences of --retry if -F was specified which already sets
        // retry
        let retry = follow_retry || matches.get_flag(tail_flags::TAIL_RETRY);
        let follow = match (
            follow_retry,
            matches
                .get_one::<String>(tail_flags::TAIL_FOLLOW)
                .map(|s| s.as_str()),
        ) {
            // -F and --follow if -F is specified after --follow. We don't need to care about the
            // value of --follow.
            (true, Some(_))
            // It's ok to use `index_of` instead of `indices_of` since -F and  --follow
            // overwrite themselves (not only the value but also the index).
            if matches.index_of(tail_flags::TAIL_FOLLOW_RETRY) > matches.index_of(tail_flags::TAIL_FOLLOW) =>
                {
                    Some(TailFollowMode::Name)
                }
            // * -F and --follow=name if --follow=name is specified after -F
            // * No occurrences of -F but --follow=name
            // * -F and no occurrences of --follow
            (_, Some("name")) | (true, None) => Some(TailFollowMode::Name),
            // * -F and --follow=descriptor (or plain --follow, -f) if --follow=descriptor is
            // specified after -F
            // * No occurrences of -F but --follow=descriptor, --follow, -f
            (_, Some(_)) => Some(TailFollowMode::Descriptor),
            // The default for no occurrences of -F or --follow
            (false, None) => None,
        };

        let mut settings: Self = Self {
            follow,
            retry,
            use_polling: matches.get_flag(tail_flags::TAIL_USE_POLLING),
            mode: TailFilterMode::from(matches)?,
            verbose: matches.get_flag(tail_flags::verbosity::TAIL_VERBOSE),
            presume_input_pipe: matches.get_flag(tail_flags::TAIL_PRESUME_INPUT_PIPE),
            ..Default::default()
        };

        if let Some(source) = matches.get_one::<String>(tail_flags::TAIL_SLEEP_INT) {
            // Advantage of `fundu` over `Duration::(try_)from_secs_f64(source.parse().unwrap())`:
            // * doesn't panic on errors like `Duration::from_secs_f64` would.
            // * no precision loss, rounding errors or other floating point problems.
            // * evaluates to `Duration::MAX` if the parsed number would have exceeded
            //   `DURATION::MAX` or `infinity` was given
            // * not applied here but it supports customizable time units and provides better error
            //   messages
            settings.sleep_sec = match DurationParser::without_time_units().parse(source) {
                Ok(duration) => SaturatingInto::<std::time::Duration>::saturating_into(duration),
                Err(_) => {
                    return Err(CTsageError::new(
                        1,
                        format!("invalid number of seconds: '{source}'"),
                    ));
                }
            }
        }

        if let Some(s) = matches.get_one::<String>(tail_flags::TAIL_MAX_UNCHANGED_STATS) {
            settings.max_unchanged_stats = match s.parse::<u32>() {
                Ok(s) => s,
                Err(_) => {
                    return Err(CTsageError::new(
                        1,
                        format!(
                            "invalid maximum number of unchanged stats between opens: {}",
                            s.quote()
                        ),
                    ));
                }
            }
        }

        if let Some(pid_str) = matches.get_one::<String>(tail_flags::TAIL_PID) {
            match pid_str.parse() {
                Ok(pid) => {
                    // NOTE: on unix platform::Pid is i32, on windows platform::Pid is u32
                    #[cfg(unix)]
                    if pid < 0 {
                        // NOTE: tail only accepts an unsigned pid
                        return Err(CtSimpleError::new(
                            1,
                            format!("invalid PID: {}", pid_str.quote()),
                        ));
                    }

                    settings.pid = pid;
                }
                Err(e) => {
                    return Err(CtSimpleError::new(
                        1,
                        format!("invalid PID: {}: {}", pid_str.quote(), e),
                    ));
                }
            }
        }

        settings.inputs = matches
            .get_many::<OsString>(tail_flags::TAIL_ARG_FILES)
            .map(|v| v.map(TailInput::from).collect())
            .unwrap_or_else(|| vec![TailInput::default()]);

        settings.verbose =
            settings.inputs.len() > 1 && !matches.get_flag(tail_flags::verbosity::TAIL_QUIET);

        Ok(settings)
    }

    pub fn has_only_stdin(&self) -> bool {
        self.inputs.iter().all(|input| input.is_stdin())
    }

    pub fn has_stdin(&self) -> bool {
        self.inputs.iter().any(|input| input.is_stdin())
    }

    pub fn num_inputs(&self) -> usize {
        self.inputs.len()
    }

    /// Check [`TailSettings`] for problematic configurations of tail originating from user provided
    /// command line arguments and print appropriate warnings.
    pub fn check_warnings(&self) {
        if self.retry {
            if self.follow.is_none() {
                ct_show_warning!("--retry ignored; --retry is useful only when following");
            } else if self.follow == Some(TailFollowMode::Descriptor) {
                ct_show_warning!("--retry only effective for the initial open");
            }
        }

        if self.pid != 0 {
            if self.follow.is_none() {
                ct_show_warning!("PID ignored; --pid=PID is useful only when following");
            } else if !platform::supports_pid_checks(self.pid) {
                ct_show_warning!("--pid=PID is not supported on this system");
            }
        }

        // This warning originates from gnu's tail implementation of the equivalent warning. If the
        // user wants to follow stdin, but tail is blocking indefinitely anyways, because of stdin
        // as `tty` (but no otherwise blocking stdin), then we print a warning that `--follow`
        // cannot be applied under these circumstances and is therefore ineffective.
        if self.follow.is_some() && self.has_stdin() {
            let blocking_stdin = self.pid == 0
                && self.follow == Some(TailFollowMode::Descriptor)
                && self.num_inputs() == 1
                && Handle::stdin().map_or(false, |handle| {
                    handle
                        .as_file()
                        .metadata()
                        .map_or(false, |meta| !meta.is_file())
                });

            if !blocking_stdin && std::io::stdin().is_terminal() {
                ct_show_warning!("following standard input indefinitely is ineffective");
            }
        }
    }

    /// Verify [`TailSettings`] and try to find unsolvable misconfigurations of tail originating from
    /// user provided command line arguments. In contrast to [`TailSettings::check_warnings`] these
    /// misconfigurations usually lead to the immediate exit or abortion of the running `tail`
    /// process.
    pub fn verify(&self) -> TailVerificationResult {
        // Mimic GNU's tail for `tail -F`
        if self.inputs.iter().any(|i| i.is_stdin()) && self.follow == Some(TailFollowMode::Name) {
            return TailVerificationResult::CannotFollowStdinByName;
        }

        // Mimic GNU's tail for -[nc]0 without -f and exit immediately
        if self.follow.is_none()
            && matches!(
                self.mode,
                TailFilterMode::Lines(TailSignum::MinusZero, _)
                    | TailFilterMode::Bytes(TailSignum::MinusZero)
            )
        {
            return TailVerificationResult::NoOutput;
        }

        TailVerificationResult::Ok
    }
}

pub fn tail_parse_obsolete(
    arg: &OsString,
    input: Option<&OsString>,
) -> CTResult<Option<TailOptions>> {
    match parse::tail_parse_obsolete(arg) {
        Some(Ok(args)) => Ok(Some(TailOptions::from_obsolete_args(&args, input))),
        None => Ok(None),
        Some(Err(e)) => {
            let arg_str = arg.to_string_lossy();
            Err(CtSimpleError::new(
                1,
                match e {
                    parse::TailParseError::OutOfRange => format!(
                        "invalid number: {}: Numerical result out of range",
                        arg_str.quote()
                    ),
                    parse::TailParseError::Overflow => {
                        format!("invalid number: {}", arg_str.quote())
                    }
                    // this ensures compatibility to GNU's error message (as tested in misc/tail)
                    parse::TailParseError::Context => format!(
                        "option used in invalid context -- {}",
                        arg_str.chars().nth(1).unwrap_or_default()
                    ),
                    parse::TailParseError::InvalidEncoding => {
                        format!("bad argument encoding: '{arg_str}'")
                    }
                },
            ))
        }
    }
}

fn tail_parse_num(src: &str) -> Result<TailSignum, ParseSizeError> {
    let mut size_string = src.trim();
    let mut starting_with = false;

    if let Some(c) = size_string.chars().next() {
        if c == '+' || c == '-' {
            // tail: '-' is not documented (8.32 man pages)
            size_string = &size_string[1..];
            if c == '+' {
                starting_with = true;
            }
        }
    }

    match parse_size_u64(size_string) {
        Ok(n) => match (n, starting_with) {
            (0, true) => Ok(TailSignum::PlusZero),
            (0, false) => Ok(TailSignum::MinusZero),
            (n, true) => Ok(TailSignum::Positive(n)),
            (n, false) => Ok(TailSignum::Negative(n)),
        },
        Err(_) => Err(ParseSizeError::ParseFailure(size_string.to_string())),
    }
}

/// 解析命令行参数并返回 TailOptions
/// 
/// 此函数处理两种语法：
/// 1. 现代语法：使用标准的命令行选项（如 -n 10, --follow 等）
/// 2. 过时语法：支持旧式的参数格式（如 -10, +10 等）
pub fn tail_parse_args(args: impl ctcore::Args) -> CTResult<TailOptions> {
    let args_vec: Vec<OsString> = args.collect();
    
    // 首先尝试使用现代语法解析
    let modern_result = parse_modern_syntax(&args_vec);
    
    // 如果不需要尝试过时语法，直接返回现代语法的结果
    if !should_try_obsolete_syntax(&args_vec, &modern_result) {
        return modern_result;
    }

    // 尝试解析过时语法
    match parse_obsolete_syntax(&args_vec)? {
        Some(settings) => Ok(settings),
        None => modern_result,
    }
}

/// 使用现代语法解析参数
fn parse_modern_syntax(args: &[OsString]) -> CTResult<TailOptions> {
    match ct_app().try_get_matches_from(args) {
        Ok(matches) => Ok(TailOptions::from(&matches)?),
        Err(err) => Err(err.into()),
    }
}

/// 判断是否需要尝试过时语法
fn should_try_obsolete_syntax(args: &[OsString], modern_result: &CTResult<TailOptions>) -> bool {
    // 只有在参数数量为2或3时才考虑过时语法
    if args.len() != 2 && args.len() != 3 {
        return false;
    }

    // 如果现代语法解析成功，只有当第一个参数以'+'开头时才尝试过时语法
    // 因为这可能是过时语法的正数偏移量
    if modern_result.is_ok() {
        return args[1].to_string_lossy().starts_with('+');
    }

    // 如果现代语法解析失败，可能是过时语法
    true
}

/// 解析过时语法
fn parse_obsolete_syntax(args: &[OsString]) -> CTResult<Option<TailOptions>> {
    let possible_obsolete_args = &args[1];
    let input = args.get(2);
    
    tail_parse_obsolete(possible_obsolete_args, input)
}

pub fn ct_app() -> Command {
    #[cfg(target_os = "linux")]
    const TAIL_POLLING_HELP: &str = "Disable 'inotify' support and use polling instead";
    #[cfg(all(unix, not(target_os = "linux")))]
    const TAIL_POLLING_HELP: &str = "Disable 'kqueue' support and use polling instead";
    #[cfg(target_os = "windows")]
    const TAIL_POLLING_HELP: &str =
        "Disable 'ReadDirectoryChanges' support and use polling instead";

    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = TAIL_ABOUT;
    let usage_description = ct_format_usage(TAIL_USAGE);
    let args = vec![Arg::new(tail_flags::TAIL_BYTES)
                        .short('c')
                        .long(tail_flags::TAIL_BYTES)
                        .allow_hyphen_values(true)
                        .overrides_with_all([tail_flags::TAIL_BYTES, tail_flags::TAIL_LINES])
                        .help("Number of bytes to print"),
                    Arg::new(tail_flags::TAIL_FOLLOW)
                        .short('f')
                        .long(tail_flags::TAIL_FOLLOW)
                        .default_missing_value("descriptor")
                        .num_args(0..=1)
                        .require_equals(true)
                        .value_parser(["descriptor", "name"])
                        .overrides_with(tail_flags::TAIL_FOLLOW)
                        .help("Print the file as it grows"),
                    Arg::new(tail_flags::TAIL_LINES)
                        .short('n')
                        .long(tail_flags::TAIL_LINES)
                        .allow_hyphen_values(true)
                        .overrides_with_all([tail_flags::TAIL_BYTES, tail_flags::TAIL_LINES])
                        .help("Number of lines to print"),
                    Arg::new(tail_flags::TAIL_PID)
                        .long(tail_flags::TAIL_PID)
                        .value_name("PID")
                        .help("With -f, terminate after process ID, PID dies"),
                    Arg::new(tail_flags::verbosity::TAIL_QUIET)
                        .short('q')
                        .long(tail_flags::verbosity::TAIL_QUIET)
                        .visible_alias("silent")
                        .overrides_with_all([tail_flags::verbosity::TAIL_QUIET, tail_flags::verbosity::TAIL_VERBOSE])
                        .help("Never output headers giving file names")
                        .action(ArgAction::SetTrue),
                    Arg::new(tail_flags::TAIL_SLEEP_INT)
                        .short('s')
                        .value_name("N")
                        .long(tail_flags::TAIL_SLEEP_INT)
                        .help("Number of seconds to sleep between polling the file when running with -f"),
                    Arg::new(tail_flags::TAIL_MAX_UNCHANGED_STATS)
                        .value_name("N")
                        .long(tail_flags::TAIL_MAX_UNCHANGED_STATS)
                        .help(
                            "Reopen a FILE which has not changed size after N (default 5) iterations \
                        to see if it has been unlinked or renamed (this is the usual case of rotated \
                        log files); This option is meaningful only when polling \
                        (i.e., with --use-polling) and when --follow=name",
                        ),
                    Arg::new(tail_flags::verbosity::TAIL_VERBOSE)
                        .short('v')
                        .long(tail_flags::verbosity::TAIL_VERBOSE)
                        .overrides_with_all([tail_flags::verbosity::TAIL_QUIET, tail_flags::verbosity::TAIL_VERBOSE])
                        .help("Always output headers giving file names")
                        .action(ArgAction::SetTrue),
                    Arg::new(tail_flags::TAIL_ZERO_TERM)
                        .short('z')
                        .long(tail_flags::TAIL_ZERO_TERM)
                        .help("Line delimiter is NUL, not newline")
                        .action(ArgAction::SetTrue),
                    Arg::new(tail_flags::TAIL_USE_POLLING)
                        .alias(tail_flags::TAIL_DISABLE_INOTIFY_TERM) // NOTE: Used by GNU's test suite
                        .alias("dis") // NOTE: Used by GNU's test suite
                        .long(tail_flags::TAIL_USE_POLLING)
                        .help(TAIL_POLLING_HELP)
                        .action(ArgAction::SetTrue),
                    Arg::new(tail_flags::TAIL_RETRY)
                        .long(tail_flags::TAIL_RETRY)
                        .help("Keep trying to open a file if it is inaccessible")
                        .overrides_with(tail_flags::TAIL_RETRY)
                        .action(ArgAction::SetTrue),
                    Arg::new(tail_flags::TAIL_FOLLOW_RETRY)
                        .short('F')
                        .help("Same as --follow=name --retry")
                        .overrides_with(tail_flags::TAIL_FOLLOW_RETRY)
                        .action(ArgAction::SetTrue),
                    Arg::new(tail_flags::TAIL_PRESUME_INPUT_PIPE)
                        .long("presume-input-pipe")
                        .alias(tail_flags::TAIL_PRESUME_INPUT_PIPE)
                        .hide(true)
                        .action(ArgAction::SetTrue),
                    Arg::new(tail_flags::TAIL_ARG_FILES)
                        .action(ArgAction::Append)
                        .num_args(1..)
                        .value_parser(value_parser!(OsString))
                        .value_hint(clap::ValueHint::FilePath)];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(&args)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use crate::parse::TailObsoleteArgs;

    use super::*;

    mod test_tail_parse_args {
        use super::*;
        use std::ffi::OsString;
    
        fn create_args(args: &[&str]) -> impl ctcore::Args {
            let mut vec = vec![OsString::from("tail")];
            vec.extend(args.iter().map(|s| OsString::from(s)));
            vec.into_iter()
        }
    
        #[test]
        fn test_basic_options() {
            // 测试基本的命令行选项
            let args = create_args(&["-n", "10"]);
            let options = tail_parse_args(args).unwrap();
            assert_eq!(
                options.mode,
                TailFilterMode::Lines(TailSignum::Negative(10), b'\n')
            );
        }
    
        #[test]
        fn test_bytes_option() {
            // 测试字节数选项
            let args = create_args(&["-c", "20"]);
            let options = tail_parse_args(args).unwrap();
            assert_eq!(options.mode, TailFilterMode::Bytes(TailSignum::Negative(20)));
    
            // 测试带加号的字节数
            let args = create_args(&["-c", "+20"]);
            let options = tail_parse_args(args).unwrap();
            assert_eq!(options.mode, TailFilterMode::Bytes(TailSignum::Positive(20)));
        }
    
        #[test]
        fn test_follow_options() {
            // 测试跟随模式选项
            let args = create_args(&["-f"]);
            let options = tail_parse_args(args).unwrap();
            assert_eq!(options.follow, Some(TailFollowMode::Descriptor));
    
            // 测试指定跟随方式
            let args = create_args(&["--follow=name"]);
            let options = tail_parse_args(args).unwrap();
            assert_eq!(options.follow, Some(TailFollowMode::Name));
        }
    
        #[test]
        fn test_zero_terminated() {
            // 测试零终止符选项
            let args = create_args(&["-z"]);
            let options = tail_parse_args(args).unwrap();
            match options.mode {
                TailFilterMode::Lines(_, delimiter) => assert_eq!(delimiter, 0),
                _ => panic!("Expected Lines mode"),
            }
        }
    
        #[test]
        fn test_retry_option() {
            // 测试重试选项
            let args = create_args(&["--retry"]);
            let options = tail_parse_args(args).unwrap();
            assert!(options.retry);
        }
    
        #[test]
        fn test_multiple_files() {
            // 测试多文件输入
            let args = create_args(&["file1", "file2", "file3"]);
            let options = tail_parse_args(args).unwrap();
            assert_eq!(options.inputs.len(), 3);
        }
    
        #[test]
        fn test_invalid_number() {
            // 测试无效的数字输入
            let args = create_args(&["-n", "invalid"]);
            assert!(tail_parse_args(args).is_err());
        }
    
        #[test]
        fn test_combined_options() {
            // 测试组合选项
            let args = create_args(&["-n", "10", "-f", "--retry"]);
            let options = tail_parse_args(args).unwrap();
            assert_eq!(
                options.mode,
                TailFilterMode::Lines(TailSignum::Negative(10), b'\n')
            );
            assert_eq!(options.follow, Some(TailFollowMode::Descriptor));
            assert!(options.retry);
        }
    
        #[test]
        fn test_obsolete_syntax() {
            // 测试过时的语法
            let args = create_args(&["-10"]);
            let options = tail_parse_args(args).unwrap();
            assert_eq!(
                options.mode,
                TailFilterMode::Lines(TailSignum::Negative(10), b'\n')
            );
    
            let args = create_args(&["+10"]);
            let options = tail_parse_args(args).unwrap();
            assert_eq!(
                options.mode,
                TailFilterMode::Lines(TailSignum::Positive(10), b'\n')
            );
        }
    
        #[test]
        fn test_default_values() {
            // 测试默认值
            let args = create_args(&[]);
            let options = tail_parse_args(args).unwrap();
            assert_eq!(
                options.mode,
                TailFilterMode::Lines(TailSignum::Negative(10), b'\n')
            );
            assert_eq!(options.follow, None);
            assert!(!options.retry);
        }
    }
    #[test]
    fn test_parse_num_when_sign_is_given() {
        let result = tail_parse_num("+0");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), TailSignum::PlusZero);

        let result = tail_parse_num("+1");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), TailSignum::Positive(1));

        let result = tail_parse_num("-0");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), TailSignum::MinusZero);

        let result = tail_parse_num("-1");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), TailSignum::Negative(1));
    }

    #[test]
    fn test_parse_num_when_no_sign_is_given() {
        let result = tail_parse_num("0");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), TailSignum::MinusZero);

        let result = tail_parse_num("1");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), TailSignum::Negative(1));
    }

    #[test]
    fn test_parse_obsolete_settings_f() {
        let args = TailObsoleteArgs {
            follow: true,
            ..Default::default()
        };
        let result = TailOptions::from_obsolete_args(&args, None);
        assert_eq!(result.follow, Some(TailFollowMode::Descriptor));

        let result = TailOptions::from_obsolete_args(&args, Some(&"file".into()));
        assert_eq!(result.follow, Some(TailFollowMode::Name));
    }

    #[rstest]
    #[case::default(vec ! [], None, false)]
    #[case::retry(vec ! ["--retry"], None, true)]
    #[case::multiple_retry(vec ! ["--retry", "--retry"], None, true)]
    #[case::follow_long(vec ! ["--follow"], Some(TailFollowMode::Descriptor), false)]
    #[case::follow_short(vec ! ["-f"], Some(TailFollowMode::Descriptor), false)]
    #[case::follow_long_with_retry(vec ! ["--follow", "--retry"], Some(TailFollowMode::Descriptor), true)]
    #[case::follow_short_with_retry(vec ! ["-f", "--retry"], Some(TailFollowMode::Descriptor), true)]
    #[case::follow_overwrites_previous_selection_1(vec ! ["--follow=name", "--follow=descriptor"], Some(TailFollowMode::Descriptor), false)]
    #[case::follow_overwrites_previous_selection_2(vec ! ["--follow=descriptor", "--follow=name"], Some(TailFollowMode::Name), false)]
    #[case::big_f(vec ! ["-F"], Some(TailFollowMode::Name), true)]
    #[case::multiple_big_f(vec ! ["-F", "-F"], Some(TailFollowMode::Name), true)]
    #[case::big_f_with_retry_then_does_not_change(vec ! ["-F", "--retry"], Some(TailFollowMode::Name), true)]
    #[case::big_f_with_follow_descriptor_then_change(vec ! ["-F", "--follow=descriptor"], Some(TailFollowMode::Descriptor), true)]
    #[case::multiple_big_f_with_follow_descriptor_then_no_change(vec ! ["-F", "--follow=descriptor", "-F"], Some(TailFollowMode::Name), true)]
    #[case::big_f_with_follow_short_then_change(vec ! ["-F", "-f"], Some(TailFollowMode::Descriptor), true)]
    #[case::follow_descriptor_with_big_f_then_change(vec ! ["--follow=descriptor", "-F"], Some(TailFollowMode::Name), true)]
    #[case::follow_short_with_big_f_then_change(vec ! ["-f", "-F"], Some(TailFollowMode::Name), true)]
    #[case::big_f_with_follow_name_then_not_change(vec ! ["-F", "--follow=name"], Some(TailFollowMode::Name), true)]
    #[case::follow_name_with_big_f_then_not_change(vec ! ["--follow=name", "-F"], Some(TailFollowMode::Name), true)]
    #[case::big_f_with_multiple_long_follow(vec ! ["--follow=name", "-F", "--follow=descriptor"], Some(TailFollowMode::Descriptor), true)]
    #[case::big_f_with_multiple_long_follow_name(vec ! ["--follow=name", "-F", "--follow=name"], Some(TailFollowMode::Name), true)]
    #[case::big_f_with_multiple_short_follow(vec ! ["-f", "-F", "-f"], Some(TailFollowMode::Descriptor), true)]
    #[case::multiple_big_f_with_multiple_short_follow(vec ! ["-f", "-F", "-f", "-F"], Some(TailFollowMode::Name), true)]
    fn test_parse_settings_follow_mode_and_retry(
        #[case] args: Vec<&str>,
        #[case] expected_follow_mode: Option<TailFollowMode>,
        #[case] expected_retry: bool,
    ) {
        let settings =
            TailOptions::from(&ct_app().no_binary_name(true).get_matches_from(args)).unwrap();
        assert_eq!(settings.follow, expected_follow_mode);
        assert_eq!(settings.retry, expected_retry);
    }
}
