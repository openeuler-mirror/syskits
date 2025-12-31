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

pub mod native_int_str;
pub mod parse_error;
pub mod split_iterator;
pub mod string_expander;
pub mod string_parser;
pub mod variable_parser;

use clap::builder::ValueParser;

use clap::crate_version;
use clap::Arg;
use clap::ArgAction;
use clap::Command;

use ini::Ini;
use native_int_str::{
    from_native_int_representation_owned, EnvConvert, NCvt, NativeIntStr, NativeIntString,
    NativeStr,
};
#[cfg(unix)]
use nix::sys::signal::raise;
use nix::sys::signal::sigaction;
use nix::sys::signal::SaFlags;
use nix::sys::signal::SigAction;
use nix::sys::signal::SigHandler;
use nix::sys::signal::SigSet;
use nix::sys::signal::Signal;

use std::borrow::Cow;
use std::env;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::io::{self, Write};
use std::ops::Deref;

use ctcore::ct_display::Quotable;
use ctcore::ct_error::CTError;
use ctcore::ct_error::CTResult;
use ctcore::ct_error::CTsageError;
use ctcore::ct_error::CtSimpleError;
use ctcore::ct_error::ExitCode;

use ctcore::ct_format_usage;
use ctcore::ct_help_about;
use ctcore::ct_help_section;
use ctcore::ct_help_usage;
use ctcore::ct_line_ending::CtLineEnding;
use ctcore::ct_show_warning;

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use std::process::{self};

const ENV_ABOUT: &str = ct_help_about!("env.md");
const ENV_USAGE: &str = ct_help_usage!("env.md");
const ENV_AFTER_HELP: &str = ct_help_section!("after help", "env.md");

const ERROR_MSG_S_SHEBANG: &str = "use -[v]S to pass options in shebang lines";

#[derive(Debug, PartialEq)]
struct EnvOptions<'a> {
    ignore_env: bool,
    line_ending: CtLineEnding,
    running_directory: Option<&'a OsStr>,
    files: Vec<&'a OsStr>,
    unsets: Vec<&'a OsStr>,
    sets: Vec<(Cow<'a, OsStr>, Cow<'a, OsStr>)>,
    program: Vec<&'a OsStr>,
}

fn print_env(line_ending: CtLineEnding) {
    let stdout_raw = io::stdout();
    let mut stdout = stdout_raw.lock();
    for (n, v) in env::vars() {
        write!(stdout, "{}={}{}", n, v, line_ending).unwrap();
    }
}

fn env_parse_name_value_opt<'a>(options: &mut EnvOptions<'a>, option: &'a OsStr) -> CTResult<bool> {
    // is it a NAME=VALUE like opt ?
    let wrap = NativeStr::<'a>::new(option);
    let split_o = wrap.split_once(&'=');
    if let Some((name, value)) = split_o {
        // yes, so push name, value pair
        options.sets.push((name, value));
        Ok(false)
    } else {
        // no, it's a program-like opt
        env_parse_program_opt(options, option).map(|_| true)
    }
}

fn env_parse_program_opt<'a>(options: &mut EnvOptions<'a>, option: &'a OsStr) -> CTResult<()> {
    if options.line_ending == CtLineEnding::Nul {
        Err(CTsageError::new(
            125,
            "cannot specify --null (-0) with command".to_string(),
        ))
    } else {
        options.program.push(option);
        Ok(())
    }
}

/**
 * 加载配置文件到环境变量中。
 *
 * 此函数遍历 `EnvOptions` 中指定的文件列表，将每个文件（或标准输入）解析为环境变量。
 * 支持从文件或标准输入中读取 ".env"-style 格式的配置。
 *
 * @param options 一个可变引用，指向 `EnvOptions` 结构体，其中包含了要加载的配置文件列表。
 * @return 返回一个 `CTResult<()>`，成功时为 `Ok(())`，失败时为包含错误信息的 `Err`。
 */
fn env_load_config_file(options: &mut EnvOptions) -> CTResult<()> {
    // 使用 INI 解析器来解析配置文件，尽管它实际上支持 ".env"-style 文件，但并不支持标准的 INI 文件。
    for &file in &options.files {
        let config = if file == "-" {
            // 从标准输入读取配置
            let stdin = io::stdin();
            let mut stdin_locked = stdin.lock();
            Ini::read_from(&mut stdin_locked)
        } else {
            // 从指定文件路径加载配置
            Ini::load_from_file(file)
        };

        // 尝试解析配置，出错时记录错误信息
        let config =
            config.map_err(|e| CtSimpleError::new(1, format!("{}: {}", file.maybe_quote(), e)))?;

        // 遍历配置项，忽略INI节行（将其视为注释），并将键值对设置为环境变量
        for (_, prop) in &config {
            for (key, value) in prop.iter() {
                env::set_var(key, value);
            }
        }
    }

    Ok(())
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = ENV_ABOUT;
    let usage_description = ct_format_usage(ENV_USAGE);
    let args = env_args_init();

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .after_help(ENV_AFTER_HELP)
        .infer_long_args(true)
        .trailing_var_arg(true)
        .args(&args)
}

fn env_args_init() -> Vec<Arg> {
    let args = vec![
        Arg::new("ignore-environment")
            .short('i')
            .long("ignore-environment")
            .help("start with an empty environment")
            .action(ArgAction::SetTrue),
        Arg::new("chdir")
            .short('C') // GNU env compatibility
            .long("chdir")
            .number_of_values(1)
            .value_name("DIR")
            .value_parser(ValueParser::os_string())
            .value_hint(clap::ValueHint::DirPath)
            .help("change working directory to DIR"),
        Arg::new("null")
            .short('0')
            .long("null")
            .help(
                "end each output line with a 0 byte rather than a newline (only \
            valid when printing the environment)",
            )
            .action(ArgAction::SetTrue),
        Arg::new("file")
            .short('f')
            .long("file")
            .value_name("PATH")
            .value_hint(clap::ValueHint::FilePath)
            .value_parser(ValueParser::os_string())
            .action(ArgAction::Append)
            .help(
                "read and set variables from a \".env\"-style configuration file \
            (prior to any unset and/or set)",
            ),
        Arg::new("unset")
            .short('u')
            .long("unset")
            .value_name("NAME")
            .action(ArgAction::Append)
            .value_parser(ValueParser::os_string())
            .help("remove variable from the environment"),
        Arg::new("debug")
            .short('v')
            .long("debug")
            .action(ArgAction::SetTrue)
            .help("print verbose information for each processing step"),
        Arg::new("split-string") // split string handling is implemented directly, not using CLAP. But this entry here is needed for the help information output.
            .short('S')
            .long("split-string")
            .value_name("S")
            .action(ArgAction::Set)
            .value_parser(ValueParser::os_string())
            .help("process and split S into separate arguments; used to pass multiple arguments on shebang lines"),
        Arg::new("vars")
            .action(ArgAction::Append)
            .value_parser(ValueParser::os_string())
    ];
    args
}

pub fn env_parse_args_from_str(native_text: &NativeIntStr) -> CTResult<Vec<NativeIntString>> {
    split_iterator::split(native_text).map_err(|e| match e {
        parse_error::EnvParseError::BackslashCNotAllowedInDoubleQuotes { pos: _ } => {
            CtSimpleError::new(125, "'\\c' must not appear in double-quoted -S string")
        }
        parse_error::EnvParseError::InvalidBackslashAtEndOfStringInMinusS {
            pos: _,
            quoting: _,
        } => CtSimpleError::new(125, "invalid backslash at end of string in -S"),
        parse_error::EnvParseError::InvalidSequenceBackslashXInMinusS { pos: _, c } => {
            CtSimpleError::new(125, format!("invalid sequence '\\{}' in -S", c))
        }
        parse_error::EnvParseError::MissingClosingQuote { pos: _, c: _ } => {
            CtSimpleError::new(125, "no terminating quote in -S string")
        }
        parse_error::EnvParseError::ParsingOfVariableNameFailed { pos, msg } => {
            CtSimpleError::new(125, format!("variable name issue (at {}): {}", pos, msg,))
        }
        _ => CtSimpleError::new(125, format!("Error: {:?}", e)),
    })
}

fn env_debug_print_args(args: &[OsString]) {
    eprintln!("input args:");
    for (i, arg) in args.iter().enumerate() {
        eprintln!("arg[{}]: {}", i, arg.quote());
    }
}

fn env_check_and_handle_string_args(
    arg_ostr: &OsString,
    prefix_to_test: &str,
    all_args: &mut Vec<std::ffi::OsString>,
    is_debug_print_args: Option<&Vec<OsString>>,
) -> CTResult<bool> {
    let native_arg = NCvt::convert(arg_ostr);
    if let Some(remaining_arg) = native_arg.strip_prefix(&*NCvt::convert(prefix_to_test)) {
        if let Some(input_args) = is_debug_print_args {
            env_debug_print_args(input_args); // do it here, such that its also printed when we get an error/panic during parsing
        }

        let arg_strings = env_parse_args_from_str(remaining_arg)?;
        all_args.extend(
            arg_strings
                .into_iter()
                .map(from_native_int_representation_owned),
        );

        Ok(true)
    } else {
        Ok(false)
    }
}

#[derive(Default)]
struct EnvAppData {
    do_debug_printing: bool,
    had_string_argument: bool,
}

impl EnvAppData {
    fn make_error_no_such_file_or_dir(&self, program: &OsStr) -> Box<dyn CTError> {
        ctcore::ct_show_error!("{}: No such file or directory", program.quote());
        if !self.had_string_argument {
            ctcore::ct_show_error!("{}", ERROR_MSG_S_SHEBANG);
        }
        ExitCode::new(127)
    }

    fn process_all_string_arguments(
        &mut self,
        source_args: &Vec<OsString>,
    ) -> CTResult<Vec<std::ffi::OsString>> {
        let mut all_args: Vec<std::ffi::OsString> = Vec::new();
        for arg in source_args {
            match arg {
                b if env_check_and_handle_string_args(
                    b,
                    "--split-string",
                    &mut all_args,
                    None,
                )? =>
                {
                    self.had_string_argument = true;
                }
                b if env_check_and_handle_string_args(b, "-S", &mut all_args, None)? => {
                    self.had_string_argument = true;
                }
                b if env_check_and_handle_string_args(
                    b,
                    "-vS",
                    &mut all_args,
                    Some(source_args),
                )? =>
                {
                    self.do_debug_printing = true;
                    self.had_string_argument = true;
                }
                _ => {
                    all_args.push(arg.clone());
                }
            }
        }

        Ok(all_args)
    }

    fn parse_arguments(
        &mut self,
        source_args: impl ctcore::Args,
    ) -> Result<(Vec<OsString>, clap::ArgMatches), Box<dyn CTError>> {
        let sources_args: Vec<OsString> = source_args.collect();
        let args = self.process_all_string_arguments(&sources_args)?;
        let app = ct_app();
        let args_match = app
            .try_get_matches_from(args)
            .map_err(|e| -> Box<dyn CTError> {
                match e.kind() {
                    clap::error::ErrorKind::DisplayHelp
                    | clap::error::ErrorKind::DisplayVersion => e.into(),
                    _ => {
                        // 通过 ERROR_MSG_S_SHEBANG 扩展参数解析中的任何实际问题

                        let s = format!("{}", e);
                        if !s.is_empty() {
                            let s = s.trim_end();
                            ctcore::ct_show_error!("{}", s);
                        }
                        ctcore::ct_show_error!("{}", ERROR_MSG_S_SHEBANG);
                        ctcore::ct_error::ExitCode::new(125)
                    }
                }
            })?;
        Ok((sources_args, args_match))
    }

    /**
     * 运行环境配置，解析参数并根据配置调整环境。
     *
     */
    fn run_env(&mut self, source_args: impl ctcore::Args) -> CTResult<()> {
        // 解析命令行参数
        let (sources_args, matches) = self.parse_arguments(source_args)?;

        // 计算当前是否应该进行调试打印，考虑了命令行中可能存在的"-debug"标志
        let is_debug_printing_before = self.do_debug_printing; // 已经进行的调试打印状态
        let is_debug_printing = self.do_debug_printing || matches.get_flag("debug");
        // 如果当前设置为调试模式但之前不是，则输出参数信息
        if is_debug_printing && !is_debug_printing_before {
            env_debug_print_args(&sources_args);
        }

        // 构建执行选项
        let mut options = env_make_options(&matches)?;

        // 根据命令行选项改变当前工作目录
        env_apply_change_directory(&options)?;

        // 清空环境变量
        apply_removal_of_all_env_vars(&options);

        // 加载环境配置文件
        env_load_config_file(&mut options)?;

        // 移除指定的环境变量
        env_apply_unset_env_vars(&options)?;

        // 设置指定的环境变量
        env_apply_specified_env_vars(&options);

        if options.program.is_empty() {
            // 如果没有指定程序，则仅打印环境变量
            print_env(options.line_ending);
        } else {
            // 执行指定的程序
            return self.run_program(options, is_debug_printing);
        }

        Ok(())
    }

    /**
     * 执行程序。
     *
     * 此函数尝试运行一个外部程序，并处理相关的错误情况。
     *
     * @param options 环境选项，包含将要执行的程序及其参数。
     * @param is_do_debug_printing 是否开启调试打印，如果开启，将打印执行的程序和其参数。
     * @return Result<(), Box<dyn CTError>> 如果成功执行程序，则返回Ok(())；如果出现错误，则返回Err包含错误信息。
     */
    fn run_program(
        &mut self,
        options: EnvOptions<'_>,
        is_do_debug_printing: bool,
    ) -> Result<(), Box<dyn CTError>> {
        // 准备执行的程序和参数
        let prog = Cow::from(options.program[0]);
        let args = &options.program[1..];

        // 如果开启了调试打印，则打印程序和其参数
        if is_do_debug_printing {
            eprintln!("executable: {}", prog.quote());
            for (i, arg) in args.iter().enumerate() {
                eprintln!("arg[{}]: {}", i, arg.quote());
            }
        }

        // 尝试执行命令
        match process::Command::new(&*prog).args(args).status() {
            // 如果命令执行失败，根据是Unix系统还是Windows系统，返回相应的错误信息
            Ok(exit) if !exit.success() => {
                #[cfg(unix)]
                if let Some(exit_code) = exit.code() {
                    return Err(exit_code.into());
                } else {
                    // 处理Unix系统中因信号而终止的情况
                    let signal_code = exit.signal().unwrap();
                    let signal = Signal::try_from(signal_code).unwrap();

                    // 禁用因信号导致的默认处理行为，确保能按信号退出
                    let _ = unsafe {
                        sigaction(
                            signal,
                            &SigAction::new(SigHandler::SigDfl, SaFlags::empty(), SigSet::all()),
                        )
                    };

                    // 重新引发信号，以按预期方式退出
                    let _ = raise(signal);
                }
                return Err(exit.code().unwrap().into());
            }
            // 处理找不到文件或输入无效的错误情况
            Err(ref err)
                if (err.kind() == io::ErrorKind::NotFound)
                    || (err.kind() == io::ErrorKind::InvalidInput) =>
            {
                return Err(self.make_error_no_such_file_or_dir(prog.deref()));
            }
            // 处理其他未知错误
            Err(e) => {
                ctcore::ct_show_error!("unknown error: {:?}", e);
                return Err(126.into());
            }
            // 命令执行成功，无错误返回
            Ok(_) => (),
        }
        Ok(())
    }
}

fn apply_removal_of_all_env_vars(options: &EnvOptions<'_>) {
    // 如果被指示忽略预设，则移除所有环境变量

    if options.ignore_env {
        for (ref name, _) in env::vars_os() {
            env::remove_var(name);
        }
    }
}

/**
 * 根据 clap::ArgMatches 生成 EnvOptions。
 *
 * # 参数
 * - `matches`: clap库解析命令行参数后得到的ArgMatches对象，用于获取用户输入的参数值。
 *
 * # 返回值
 * - `CTResult<EnvOptions<'_>>`: 表示操作结果的类型，成功时返回EnvOptions对象，失败时返回错误信息。
 */
fn env_make_options(args_match: &clap::ArgMatches) -> CTResult<EnvOptions<'_>> {
    // 解析用户是否忽略了环境变量
    let ignore_env = args_match.get_flag("ignore-environment");
    // 解析行结束符设置
    let line_ending = CtLineEnding::from_zero_flag(args_match.get_flag("null"));
    // 解析工作目录选项
    let running_directory = args_match
        .get_one::<OsString>("chdir")
        .map(|s| s.as_os_str());
    // 解析文件列表
    let files = match args_match.get_many::<OsString>("file") {
        Some(v) => v.map(|s| s.as_os_str()).collect(),
        None => Vec::with_capacity(0),
    };
    // 解析要取消设置的环境变量列表
    let unsets = match args_match.get_many::<OsString>("unset") {
        Some(v) => v.map(|s| s.as_os_str()).collect(),
        None => Vec::with_capacity(0),
    };

    // 初始化 EnvOptions 结构体
    let mut opts = EnvOptions {
        ignore_env,
        line_ending,
        running_directory,
        files,
        unsets,
        sets: vec![],
        program: vec![],
    };

    // 处理 "vars" 参数，解析环境变量设置和程序参数
    let mut begin_prog_opts = false;
    if let Some(mut iter) = args_match.get_many::<OsString>("vars") {
        // 解析 NAME=VALUE 参数，并切换到程序参数解析模式
        while !begin_prog_opts {
            if let Some(opt) = iter.next() {
                if opt == "-" {
                    opts.ignore_env = true;
                } else {
                    begin_prog_opts = env_parse_name_value_opt(&mut opts, opt)?;
                }
            } else {
                break;
            }
        }

        // 解析剩余的程序参数
        for opt in iter {
            env_parse_program_opt(&mut opts, opt)?;
        }
    }

    Ok(opts)
}

fn env_apply_unset_env_vars(options: &EnvOptions<'_>) -> Result<(), Box<dyn CTError>> {
    for opt_name in &options.unsets {
        let native_name = NativeStr::new(opt_name);
        if opt_name.is_empty()
            || native_name.contains(&'\0').unwrap()
            || native_name.contains(&'=').unwrap()
        {
            return Err(CtSimpleError::new(
                125,
                format!("cannot unset {}: Invalid argument", opt_name.quote()),
            ));
        }

        env::remove_var(opt_name);
    }
    Ok(())
}

fn env_apply_change_directory(options: &EnvOptions<'_>) -> Result<(), Box<dyn CTError>> {
    if options.program.is_empty() && options.running_directory.is_some() {
        return Err(CTsageError::new(
            125,
            "must specify command with --chdir (-C)".to_string(),
        ));
    }

    if let Some(d) = options.running_directory {
        match env::set_current_dir(d) {
            Ok(()) => d,
            Err(error) => {
                return Err(CtSimpleError::new(
                    125,
                    format!("cannot change directory to {}: {error}", d.quote()),
                ));
            }
        };
    }
    Ok(())
}

/**
 * 应用指定的环境变量。
 *
 * 此函数遍历 `opts.sets` 中的每个环境变量名称和值，将它们设置到当前进程的环境中。
 * 如果遇到空名称，则显示警告并跳过该变量的设置。
 */
fn env_apply_specified_env_vars(options: &EnvOptions<'_>) {
    // 遍历并设置指定的环境变量
    for (name, val) in &options.sets {
        // 如果环境变量名称为空，则显示警告并继续下一次迭代
        if name.is_empty() {
            ct_show_warning!("no name specified for value {}", val.quote());
            continue;
        }
        // 设置环境变量
        env::set_var(name, val);
    }
}

// 主函数，用于启动应用程序。
//
// # 参数
// `args` - 实现了 `ctcore::Args` 接口的参数对象，通常包含命令行参数。
//
// # 返回值
// 返回一个 `CTResult<()>`，表示操作的成功或失败。成功时返回 `Ok(())`，失败时返回 `Err` 包含错误信息。
#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    env_main(args).map(|_| ())
}

pub fn env_main(args: impl ctcore::Args) -> CTResult<()> {
    // 使用默认的环境应用数据执行环境设置
    EnvAppData::default().run_env(args)
}

#[cfg(test)]
mod tests {

    mod tests_env_main {
        use crate::env_main;

        use std::ffi::OsString;
        use std::fs;
        use std::fs::File;
        use std::io::Write;

        use tempfile::Builder;

        #[test]
        fn test_env_main_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = env_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 0);
        }

        #[test]
        fn test_env_main_v() {
            let args = vec![ctcore::ct_util_name(), "-V"];
            let result = env_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 0);
        }

        #[test]
        fn test_env_main_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = env_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 0);
        }

        #[test]
        fn test_env_main_h() {
            let args = vec![ctcore::ct_util_name(), "-h"];
            let result = env_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 0);
        }

        #[test]
        fn test_env_main_i() {
            let args = vec![ctcore::ct_util_name(), "-i", "arch"];
            let result = env_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_env_main_ignore_environment() {
            let args = vec![ctcore::ct_util_name(), "--ignore-environment", "arch"];
            let result = env_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_env_main_bad_args() {
            let args = vec![ctcore::ct_util_name(), "--bad-arg"];
            let result = env_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_env_main_chdir_args() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--chdir", env_dir, "ls"];

            let result = env_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_env_main_c_args() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-C", env_dir, "ls"];

            let result = env_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_env_main_0() {
            let args = vec![ctcore::ct_util_name(), "-0"];
            let result = env_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_env_main_null() {
            let args = vec![ctcore::ct_util_name(), "--null"];
            let result = env_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        use crate::env;

        #[test]
        fn test_env_main_f() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let f = test_file_1.to_str().unwrap();

            let content = "FVAR1=hello\n\
           FVAR2=CtyunOS\n\
           FVAR3=Rust\n\
           FVAR4=Syskit\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = vec![ctcore::ct_util_name(), "-f", f];

            let result = env_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());

            match env::var("FVAR1") {
                Ok(val) => {
                    assert_eq!(val, "hello");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("FVAR2") {
                Ok(val) => {
                    assert_eq!(val, "CtyunOS");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("FVAR3") {
                Ok(val) => {
                    assert_eq!(val, "Rust");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("FVAR4") {
                Ok(val) => {
                    assert_eq!(val, "Syskit");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }
        }

        #[test]
        fn test_env_main_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let content = "ENV_VAR1=hello\n\
           ENV_VAR2=CtyunOS\n\
           ENV_VAR3=Rust\n\
           ENV_VAR4=Syskit\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = vec![ctcore::ct_util_name(), "--file", filename, "ls"];

            let result = env_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());

            match env::var("ENV_VAR1") {
                Ok(val) => {
                    assert_eq!(val, "hello");
                }
                Err(e) => {
                    println!("env set fail:{}", e)
                }
            }

            match env::var("ENV_VAR2") {
                Ok(val) => {
                    assert_eq!(val, "CtyunOS");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("ENV_VAR3") {
                Ok(val) => {
                    assert_eq!(val, "Rust");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("ENV_VAR4") {
                Ok(val) => {
                    assert_eq!(val, "Syskit");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }
        }

        #[test]
        fn test_env_main_file_unset() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "UNSET_VAR1=hello\n\
           UNSET_VAR2=CtyunOS\n\
           UNSET_VAR3=Rust\n\
           UNSET_VAR4=Syskit\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = vec![ctcore::ct_util_name(), "--file", unset_filename, "env"];

            let result = env_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());

            match env::var("UNSET_VAR1") {
                Ok(val) => {
                    assert_eq!(val, "hello");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("UNSET_VAR2") {
                Ok(val) => {
                    assert_eq!(val, "CtyunOS");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("UNSET_VAR3") {
                Ok(val) => {
                    assert_eq!(val, "Rust");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("UNSET_VAR4") {
                Ok(val) => {
                    assert_eq!(val, "Syskit");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            let args = vec![
                ctcore::ct_util_name(),
                "-i",
                "env",
                "-u",
                unset_filename,
                "env",
            ];

            let result = env_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());

            match env::var("UNSET_VAR1") {
                Ok(val) => {
                    assert_eq!(val, "hello");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("UNSET_VAR2") {
                Ok(val) => {
                    assert_eq!(val, "CtyunOS");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("UNSET_VAR3") {
                Ok(val) => {
                    assert_eq!(val, "Rust");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("UNSET_VAR4") {
                Ok(val) => {
                    assert_eq!(val, "Syskit");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }
        }

        #[test]
        fn test_env_main_file_unset_whole() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "UNSET_VAR1=hello\n\
           UNSET_VAR2=CtyunOS\n\
           UNSET_VAR3=Rust\n\
           UNSET_VAR4=Syskit\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = vec![ctcore::ct_util_name(), "--file", unset_filename, "env"];

            let result = env_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());

            match env::var("UNSET_VAR1") {
                Ok(val) => {
                    assert_eq!(val, "hello");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("UNSET_VAR2") {
                Ok(val) => {
                    assert_eq!(val, "CtyunOS");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("UNSET_VAR3") {
                Ok(val) => {
                    assert_eq!(val, "Rust");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("UNSET_VAR4") {
                Ok(val) => {
                    assert_eq!(val, "Syskit");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            let args = vec![
                ctcore::ct_util_name(),
                "--ignore-environment",
                "env",
                "--unset",
                unset_filename,
                "env",
            ];

            let result = env_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());

            match env::var("UNSET_VAR1") {
                Ok(val) => {
                    assert_eq!(val, "hello");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("UNSET_VAR2") {
                Ok(val) => {
                    assert_eq!(val, "CtyunOS");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("UNSET_VAR3") {
                Ok(val) => {
                    assert_eq!(val, "Rust");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("UNSET_VAR4") {
                Ok(val) => {
                    assert_eq!(val, "Syskit");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }
        }

        #[test]
        fn test_env_main_debug() {
            let args = vec![ctcore::ct_util_name(), "-v", "arch"];
            let result = env_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_env_main_debug_whole() {
            let args = vec![ctcore::ct_util_name(), "--debug", "arch"];
            let result = env_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_env_main_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--split-string=''",
                "cat",
                file_path,
            ];

            let result = env_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_env_main_split_s() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = vec![ctcore::ct_util_name(), "-S", "", "cat", file_path];

            let result = env_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err()); //error  ,与 系统命令env不一致
        }

        #[test]
        fn test_env_main_s_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-S",
                "--split-string=''",
                "cat",
                file_path,
            ];

            let result = env_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }
    }

    mod tests_run_env {
        use crate::EnvAppData;

        use std::ffi::OsString;
        use std::fs;
        use std::fs::File;
        use std::io::Write;

        use tempfile::Builder;

        #[test]
        fn test_run_env_version() {
            let cmd = vec![ctcore::ct_util_name(), "--version"];
            let args = cmd.iter().map(|s| OsString::from(s));

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 0);
        }

        #[test]
        fn test_run_env_v() {
            let cmd = vec![ctcore::ct_util_name(), "-V"];
            let args = cmd.iter().map(|s| OsString::from(s));

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 0);
        }

        #[test]
        fn test_run_env_help() {
            let cmd = vec![ctcore::ct_util_name(), "--help"];
            let args = cmd.iter().map(|s| OsString::from(s));

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 0);
        }

        #[test]
        fn test_run_env_h() {
            let cmd = vec![ctcore::ct_util_name(), "-h"];
            let args = cmd.iter().map(|s| OsString::from(s));

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 0);
        }

        #[test]
        fn test_run_env_i() {
            let cmd = vec![ctcore::ct_util_name(), "-i", "arch"];
            let args = cmd.iter().map(|s| OsString::from(s));

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_run_env_ignore_environment() {
            let cmd = vec![ctcore::ct_util_name(), "--ignore-environment", "arch"];
            let args = cmd.iter().map(|s| OsString::from(s));

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_run_env_bad_cmd() {
            let cmd = vec![ctcore::ct_util_name(), "--bad-arg"];
            let args = cmd.iter().map(|s| OsString::from(s));

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_err());
        }

        #[test]
        fn test_run_env_chdir_cmd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let cmd = vec![ctcore::ct_util_name(), "--chdir", env_dir, "ls"];

            let args = cmd.iter().map(|s| OsString::from(s));

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_run_env_c_cmd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let cmd = vec![ctcore::ct_util_name(), "-C", env_dir, "ls"];

            let args = cmd.iter().map(|s| OsString::from(s));

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_run_env_0() {
            let cmd = vec![ctcore::ct_util_name(), "-0"];
            let args = cmd.iter().map(|s| OsString::from(s));

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_run_env_null() {
            let cmd = vec![ctcore::ct_util_name(), "--null"];
            let args = cmd.iter().map(|s| OsString::from(s));

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());
        }

        use crate::env;

        #[test]
        fn test_run_env_f() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let f = test_file_1.to_str().unwrap();

            let content = "FVAR1=hello\n\
           FVAR2=CtyunOS\n\
           FVAR3=Rust\n\
           FVAR4=Syskit\n";
            file.write_all(content.as_bytes()).unwrap();

            let cmd = vec![ctcore::ct_util_name(), "-f", f];

            let args = cmd.iter().map(|s| OsString::from(s));

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());

            match env::var("FVAR1") {
                Ok(val) => {
                    assert_eq!(val, "hello");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("FVAR2") {
                Ok(val) => {
                    assert_eq!(val, "CtyunOS");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("FVAR3") {
                Ok(val) => {
                    assert_eq!(val, "Rust");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("FVAR4") {
                Ok(val) => {
                    assert_eq!(val, "Syskit");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }
        }

        #[test]
        fn test_run_env_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let content = "ENV_VAR1=hello\n\
           ENV_VAR2=CtyunOS\n\
           ENV_VAR3=Rust\n\
           ENV_VAR4=Syskit\n";
            file.write_all(content.as_bytes()).unwrap();

            let cmd = vec![ctcore::ct_util_name(), "--file", filename, "ls"];

            let args = cmd.iter().map(|s| OsString::from(s));

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());

            match env::var("ENV_VAR1") {
                Ok(val) => {
                    assert_eq!(val, "hello");
                }
                Err(e) => {
                    println!("env set fail:{}", e)
                }
            }

            match env::var("ENV_VAR2") {
                Ok(val) => {
                    assert_eq!(val, "CtyunOS");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("ENV_VAR3") {
                Ok(val) => {
                    assert_eq!(val, "Rust");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("ENV_VAR4") {
                Ok(val) => {
                    assert_eq!(val, "Syskit");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }
        }

        #[test]
        fn test_run_env_file_unset() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "UNSET_VAR1=hello\n\
           UNSET_VAR2=CtyunOS\n\
           UNSET_VAR3=Rust\n\
           UNSET_VAR4=Syskit\n";
            file.write_all(content.as_bytes()).unwrap();

            let cmd = vec![ctcore::ct_util_name(), "--file", unset_filename, "env"];

            let args = cmd.iter().map(|s| OsString::from(s));

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());

            match env::var("UNSET_VAR1") {
                Ok(val) => {
                    assert_eq!(val, "hello");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("UNSET_VAR2") {
                Ok(val) => {
                    assert_eq!(val, "CtyunOS");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("UNSET_VAR3") {
                Ok(val) => {
                    assert_eq!(val, "Rust");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("UNSET_VAR4") {
                Ok(val) => {
                    assert_eq!(val, "Syskit");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            let cmd = vec![
                ctcore::ct_util_name(),
                "-i",
                "env",
                "-u",
                unset_filename,
                "env",
            ];

            let args = cmd.iter().map(|s| OsString::from(s));

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());

            match env::var("UNSET_VAR1") {
                Ok(val) => {
                    assert_ne!(val, "hello");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("UNSET_VAR2") {
                Ok(val) => {
                    assert_ne!(val, "CtyunOS");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("UNSET_VAR3") {
                Ok(val) => {
                    assert_ne!(val, "Rust");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("UNSET_VAR4") {
                Ok(val) => {
                    assert_ne!(val, "Syskit");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }
        }

        #[test]
        fn test_run_env_file_unset_whole() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "UNSET_VAR1=hello\n\
           UNSET_VAR2=CtyunOS\n\
           UNSET_VAR3=Rust\n\
           UNSET_VAR4=Syskit\n";
            file.write_all(content.as_bytes()).unwrap();

            let cmd = vec![ctcore::ct_util_name(), "--file", unset_filename, "env"];

            let args = cmd.iter().map(|s| OsString::from(s));

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());

            match env::var("UNSET_VAR1") {
                Ok(val) => {
                    assert_eq!(val, "hello");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("UNSET_VAR2") {
                Ok(val) => {
                    assert_eq!(val, "CtyunOS");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("UNSET_VAR3") {
                Ok(val) => {
                    assert_eq!(val, "Rust");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("UNSET_VAR4") {
                Ok(val) => {
                    assert_eq!(val, "Syskit");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            let cmd = vec![
                ctcore::ct_util_name(),
                "--ignore-environment",
                "env",
                "--unset",
                unset_filename,
                "env",
            ];

            let args = cmd.iter().map(|s| OsString::from(s));

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());

            match env::var("UNSET_VAR1") {
                Ok(val) => {
                    assert_ne!(val, "hello");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("UNSET_VAR2") {
                Ok(val) => {
                    assert_ne!(val, "CtyunOS");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("UNSET_VAR3") {
                Ok(val) => {
                    assert_ne!(val, "Rust");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }

            match env::var("UNSET_VAR4") {
                Ok(val) => {
                    assert_ne!(val, "Syskit");
                }
                Err(e) => {
                    println!("env set fail,{}", e)
                }
            }
        }

        #[test]
        fn test_run_env_debug() {
            let cmd = vec![ctcore::ct_util_name(), "-v", "arch"];
            let args = cmd.iter().map(|s| OsString::from(s));

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_run_env_debug_whole() {
            let cmd = vec![ctcore::ct_util_name(), "--debug", "arch"];
            let args = cmd.iter().map(|s| OsString::from(s));

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_run_env_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let cmd = vec![
                ctcore::ct_util_name(),
                "--split-string=''",
                "cat",
                file_path,
            ];

            let args = cmd.iter().map(|s| OsString::from(s));

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_run_env_split_s() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let cmd = vec![ctcore::ct_util_name(), "-S", "", "cat", file_path];

            let args = cmd.iter().map(|s| OsString::from(s));

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_err()); //error  ,与 系统命令env不一致
        }

        #[test]
        fn test_run_env_s_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let cmd = vec![
                ctcore::ct_util_name(),
                "-S",
                "--split-string=''",
                "cat",
                file_path,
            ];

            let args = cmd.iter().map(|s| OsString::from(s));

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());
        }
    }

    mod tests_parse_arguments {
        use crate::EnvAppData;

        use std::ffi::OsString;
        use std::fs;
        use std::fs::File;
        use std::io::Write;

        use tempfile::Builder;

        #[test]
        fn test_parse_arguments_version() {
            let cmd = vec![ctcore::ct_util_name(), "--version"];
            let args = cmd.iter().map(|s| OsString::from(s));

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.parse_arguments(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 0);
        }

        #[test]
        fn test_parse_arguments_v() {
            let cmd = vec![ctcore::ct_util_name(), "-V"];
            let args = cmd.iter().map(|s| OsString::from(s));

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.parse_arguments(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 0);
        }

        #[test]
        fn test_parse_arguments_help() {
            let cmd = vec![ctcore::ct_util_name(), "--help"];
            let args = cmd.iter().map(|s| OsString::from(s));

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.parse_arguments(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 0);
        }

        #[test]
        fn test_parse_arguments_h() {
            let cmd = vec![ctcore::ct_util_name(), "-h"];
            let args = cmd.iter().map(|s| OsString::from(s));

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.parse_arguments(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 0);
        }

        #[test]
        fn test_parse_arguments_i() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-i"), OsString::from("arch")];

            let expected_args = vec![OsString::from("-i"), OsString::from("arch")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }

        #[test]
        fn test_parse_arguments_ignore_environment() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--ignore-environment"),
                OsString::from("arch"),
            ];

            let expected_args = vec![
                OsString::from("--ignore-environment"),
                OsString::from("arch"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }

        #[test]
        fn test_parse_arguments_split_string_args() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("-S"),
                OsString::from("arg2"),
                OsString::from("-vS"),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("-S"),
                OsString::from("arg2"),
                OsString::from("-vS"),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }

        #[test]
        fn test_parse_arguments_chdir_cmd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--chdir"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let expected_args = vec![
                OsString::from("--chdir"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }

        #[test]
        fn test_parse_arguments_c_cmd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-C"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let expected_args = vec![
                OsString::from("-C"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }

        #[test]
        fn test_parse_arguments_0() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-0")];

            let expected_args = vec![OsString::from("-0")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }

        #[test]
        fn test_parse_arguments_null() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("--null")];

            let expected_args = vec![OsString::from("--null")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }

        #[test]
        fn test_parse_arguments_f() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let f = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-f"), OsString::from(f)];

            let expected_args = vec![OsString::from("-f"), OsString::from(f)];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }

        #[test]
        fn test_parse_arguments_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let f = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("--file"), OsString::from(f)];

            let expected_args = vec![OsString::from("--file"), OsString::from(f)];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }

        #[test]
        fn test_parse_arguments_unset() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--unset"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("--unset"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }

        #[test]
        fn test_parse_arguments_u() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-u"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("-u"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }

        #[test]
        fn test_parse_arguments_debug_whole() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--debug"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("--debug"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }

        #[test]
        fn test_parse_arguments_debug() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-v"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("-v"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }
        #[test]
        fn test_parse_arguments_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }
        #[test]
        fn test_parse_arguments_s_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-S"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("-S"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }
        #[test]
        fn test_parse_arguments_s_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-S"),
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("-S"),
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }
    }

    mod tests_process_all_string_arguments {
        use crate::EnvAppData;

        use std::ffi::OsString;
        use std::fs;
        use std::fs::File;
        use std::io::Write;

        use tempfile::Builder;

        #[test]
        fn test_process_all_string_arguments_i() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-i"), OsString::from("arch")];

            let expected_args = vec![OsString::from("-i"), OsString::from("arch")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args);
        }

        #[test]
        fn test_process_all_string_arguments_ignore_environment() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--ignore-environment"),
                OsString::from("arch"),
            ];

            let expected_args = vec![
                OsString::from("--ignore-environment"),
                OsString::from("arch"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args);
        }

        #[test]
        fn test_process_all_string_arguments_split_string_args() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("-S"),
                OsString::from("arg2"),
                OsString::from("-vS"),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("-S"),
                OsString::from("arg2"),
                OsString::from("-vS"),
                OsString::from("arg3"),
            ];

            let expected_args_2 = vec![
                OsString::from("arg1"),
                OsString::from("arg2"),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args_2);
        }

        #[test]
        fn test_process_all_string_arguments_chdir_cmd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--chdir"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let expected_args = vec![
                OsString::from("--chdir"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args);
        }

        #[test]
        fn test_process_all_string_arguments_c_cmd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-C"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let expected_args = vec![
                OsString::from("-C"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args);
        }

        #[test]
        fn test_process_all_string_arguments_0() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-0")];

            let expected_args = vec![OsString::from("-0")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args);
        }

        #[test]
        fn test_process_all_string_arguments_null() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("--null")];

            let expected_args = vec![OsString::from("--null")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args);
        }

        #[test]
        fn test_process_all_string_arguments_f() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let f = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-f"), OsString::from(f)];

            let expected_args = vec![OsString::from("-f"), OsString::from(f)];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args);
        }

        #[test]
        fn test_process_all_string_arguments_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let f = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("--file"), OsString::from(f)];

            let expected_args = vec![OsString::from("--file"), OsString::from(f)];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args);
        }

        #[test]
        fn test_process_all_string_arguments_unset() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--unset"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("--unset"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args);
        }

        #[test]
        fn test_process_all_string_arguments_u() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-u"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("-u"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args);
        }

        #[test]
        fn test_process_all_string_arguments_debug_whole() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--debug"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("--debug"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args);
        }

        #[test]
        fn test_process_all_string_arguments_debug() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-v"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("-v"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args);
        }
        #[test]
        fn test_process_all_string_arguments_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args_2 = vec![
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args_2);
        }
        #[test]
        fn test_process_all_string_arguments_s_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-S"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("-S"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args_2 = vec![
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args_2);
        }
        #[test]
        fn test_process_all_string_arguments_s_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-S"),
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("-S"),
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args_2 = vec![
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args_2);
        }
    }

    mod tests_make_error_no_such_file_or_dir {
        use crate::EnvAppData;
        use std::ffi::OsString;

        #[test]
        fn test_make_error_no_such_file_or_dir() {
            let env_app_data = EnvAppData::default();
            let prog = OsString::from("test_program");
            let error = env_app_data.make_error_no_such_file_or_dir(&prog);
            //println!("{:#?}", error.code());
            assert_eq!(error.code(), 127);
        }
    }

    mod tests_make_options {

        use crate::env_make_options;
        use crate::EnvAppData;
        use crate::EnvOptions;
        use ctcore::ct_line_ending::CtLineEnding::Newline;
        use std::ffi::{OsStr, OsString};
        use std::fs;
        use std::fs::File;
        use std::io::Write;

        use tempfile::Builder;

        #[test]
        fn test_make_options_i() {
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch"].iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-i"), OsString::from("arch")];

            let expected_args = vec![OsString::from("-i"), OsString::from("arch")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }

        #[test]
        fn test_make_options_ignore_environment() {
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch"].iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--ignore-environment"),
                OsString::from("arch"),
            ];

            let expected_args = vec![
                OsString::from("--ignore-environment"),
                OsString::from("arch"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();
            assert_eq!(original_args, expected_args);
            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }

        #[test]
        fn test_make_options_split_string_args() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("-S"),
                OsString::from("arg2"),
                OsString::from("-vS"),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("-S"),
                OsString::from("arg2"),
                OsString::from("-vS"),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arg2", "arg3"].iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }

        #[test]
        fn test_make_options_chdir_cmd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--chdir"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let expected_args = vec![
                OsString::from("--chdir"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let binding = [env_dir, "ls"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }

        #[test]
        fn test_make_options_c_cmd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-C"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let expected_args = vec![
                OsString::from("-C"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [env_dir, "ls"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }

        #[test]
        fn test_make_options_0() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-0")];

            let expected_args = vec![OsString::from("-0")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: [].to_vec(),
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }

        #[test]
        fn test_make_options_null() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("--null")];

            let expected_args = vec![OsString::from("--null")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: [].to_vec(),
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }

        #[test]
        fn test_make_options_f() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let f = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-f"), OsString::from(f)];

            let expected_args = vec![OsString::from("-f"), OsString::from(f)];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let binding = [f];
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }

        #[test]
        fn test_make_options_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let f = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("--file"), OsString::from(f)];

            let expected_args = vec![OsString::from("--file"), OsString::from(f)];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [f];
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }

        #[test]
        fn test_make_options_unset() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--unset"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("--unset"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [unset_filename, "env"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }

        #[test]
        fn test_make_options_u() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-u"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("-u"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [unset_filename, "env"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }

        #[test]
        fn test_make_options_debug_whole() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--debug"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("--debug"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch", "env"].iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }

        #[test]
        fn test_make_options_debug() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-v"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("-v"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch", "env"].iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }
        #[test]
        fn test_make_options_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = ["cat", file_path, "arg3"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }
        #[test]
        fn test_make_options_s_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-S"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("-S"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = ["cat", file_path, "arg3"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }
        #[test]
        fn test_make_options_s_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-S"),
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("-S"),
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let binding = ["cat", file_path, "arg3"];
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }
    }

    mod tests_apply_change_directory {
        use crate::env_apply_change_directory;
        use crate::env_make_options;
        use crate::EnvAppData;
        use crate::EnvOptions;

        use ctcore::ct_line_ending::CtLineEnding::Newline;
        use std::ffi::{OsStr, OsString};
        use std::fs;
        use std::fs::File;
        use std::io::Write;

        use tempfile::Builder;

        #[test]
        fn test_apply_change_directory_i() {
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch"].iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-i"), OsString::from("arch")];

            let expected_args = vec![OsString::from("-i"), OsString::from("arch")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }
        #[test]
        fn test_apply_change_directory_ignore_environment() {
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch"].iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--ignore-environment"),
                OsString::from("arch"),
            ];

            let expected_args = vec![
                OsString::from("--ignore-environment"),
                OsString::from("arch"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();
            assert_eq!(original_args, expected_args);
            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_apply_change_directory_split_string_args() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("-S"),
                OsString::from("arg2"),
                OsString::from("-vS"),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("-S"),
                OsString::from("arg2"),
                OsString::from("-vS"),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arg2", "arg3"].iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_apply_change_directory_chdir_cmd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--chdir"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let expected_args = vec![
                OsString::from("--chdir"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let binding = [env_dir, "ls"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_apply_change_directory_c_cmd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-C"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let expected_args = vec![
                OsString::from("-C"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [env_dir, "ls"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_apply_change_directory_0() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-0")];

            let expected_args = vec![OsString::from("-0")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: [].to_vec(),
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_apply_change_directory_null() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("--null")];

            let expected_args = vec![OsString::from("--null")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: [].to_vec(),
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_apply_change_directory_f() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let f = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-f"), OsString::from(f)];

            let expected_args = vec![OsString::from("-f"), OsString::from(f)];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let binding = [f];
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_apply_change_directory_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let f = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("--file"), OsString::from(f)];

            let expected_args = vec![OsString::from("--file"), OsString::from(f)];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [f];
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_apply_change_directory_unset() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--unset"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("--unset"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [unset_filename, "env"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_apply_change_directory_u() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-u"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("-u"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [unset_filename, "env"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_apply_change_directory_debug_whole() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--debug"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("--debug"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch", "env"].iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_apply_change_directory_debug() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-v"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("-v"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch", "env"].iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }
        #[test]
        fn test_apply_change_directory_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = ["cat", file_path, "arg3"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }
        #[test]
        fn test_apply_change_directory_s_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-S"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("-S"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = ["cat", file_path, "arg3"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }
        #[test]
        fn test_apply_change_directory_s_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-S"),
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("-S"),
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let binding = ["cat", file_path, "arg3"];
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }
    }

    mod tests_load_config_file {
        use crate::env_load_config_file;
        use crate::env_make_options;
        use crate::EnvAppData;
        use crate::EnvOptions;

        use ctcore::ct_line_ending::CtLineEnding::Newline;
        use std::ffi::{OsStr, OsString};
        use std::fs;
        use std::fs::File;
        use std::io::Write;

        use tempfile::Builder;

        #[test]
        fn test_load_config_file_i() {
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch"].iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-i"), OsString::from("arch")];

            let expected_args = vec![OsString::from("-i"), OsString::from("arch")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }
        #[test]
        fn test_load_config_file_ignore_environment() {
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch"].iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--ignore-environment"),
                OsString::from("arch"),
            ];

            let expected_args = vec![
                OsString::from("--ignore-environment"),
                OsString::from("arch"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();
            assert_eq!(original_args, expected_args);
            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_load_config_file_split_string_args() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("-S"),
                OsString::from("arg2"),
                OsString::from("-vS"),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("-S"),
                OsString::from("arg2"),
                OsString::from("-vS"),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arg2", "arg3"].iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_load_config_file_chdir_cmd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--chdir"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let expected_args = vec![
                OsString::from("--chdir"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let binding = [env_dir, "ls"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_load_config_file_c_cmd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-C"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let expected_args = vec![
                OsString::from("-C"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [env_dir, "ls"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_load_config_file_0() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-0")];

            let expected_args = vec![OsString::from("-0")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: [].to_vec(),
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_load_config_file_null() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("--null")];

            let expected_args = vec![OsString::from("--null")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: [].to_vec(),
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_load_config_file_f() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let f = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-f"), OsString::from(f)];

            let expected_args = vec![OsString::from("-f"), OsString::from(f)];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let binding = [f];
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_load_config_file_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let f = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("--file"), OsString::from(f)];

            let expected_args = vec![OsString::from("--file"), OsString::from(f)];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [f];
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_load_config_file_unset() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--unset"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("--unset"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [unset_filename, "env"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_load_config_file_u() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-u"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("-u"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [unset_filename, "env"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_load_config_file_debug_whole() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--debug"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("--debug"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch", "env"].iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_load_config_file_debug() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-v"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("-v"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch", "env"].iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }
        #[test]
        fn test_load_config_file_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = ["cat", file_path, "arg3"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }
        #[test]
        fn test_load_config_file_s_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-S"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("-S"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = ["cat", file_path, "arg3"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }
        #[test]
        fn test_load_config_file_s_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-S"),
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("-S"),
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let binding = ["cat", file_path, "arg3"];
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }
    }
}