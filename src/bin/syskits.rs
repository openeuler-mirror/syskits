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

use clap::{Arg, Command};
use clap_complete::Shell;
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process;

extern crate tool_derive;

const CT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 主应用程序结构体, tool_derive::Tools用于自动注册所有工具
#[derive(tool_derive::Tools)]
struct SysKits {}

/// 执行上下文，包含命令执行所需的所有信息
#[derive(Clone)]
struct ExecutionContext {
    binary_name: String, // 二进制文件名
    args: Vec<OsString>, // 命令行参数列表
}

impl ExecutionContext {
    /// 创建新的执行上下文，预分配容量以减少内存分配
    fn new(binary_name: String, args: Vec<OsString>) -> Self {
        Self { binary_name, args }
    }

    /// 更新执行上下文的名称
    fn update_name(&mut self, util_str: &str) {
        self.binary_name = util_str.to_string();
    }

    /// 构建参数列表，使用更高效的方式
    fn build_args(&self) -> Vec<OsString> {
        let mut args = Vec::with_capacity(self.args.len() + 1);
        args.push(self.binary_name.clone().into());
        args.extend(self.args.iter().cloned());
        args
    }
}

impl SysKits {
    /// 运行应用程序的主入口
    fn run(mut context: ExecutionContext) {
        // 首先尝试通过二进制名称直接执行
        if let Some(exit_code) = Self::try_execute_by_binary_name(&context) {
            process::exit(exit_code);
        }

        // 然后尝试解析工具名称（从前缀或下一个参数）
        match Self::preprocess_args(&mut context) {
            Some(util_name) => {
                let util_str = util_name.to_string_lossy().to_string();
                // 更新context的name
                context.update_name(&util_str);
                let handler = CommandHandler::new(util_name, context);
                handler.execute();
            }
            None => {
                // 没有提供参数，显示帮助信息
                Self::show_help(&context.binary_name);
                process::exit(0);
            }
        }
    }

    /// 尝试通过二进制名称直接执行工具
    fn try_execute_by_binary_name(context: &ExecutionContext) -> Option<i32> {
        // 使用 CommandHandler 处理工具执行
        let handler = CommandHandler::new(OsString::from(&context.binary_name), context.clone());
        handler.try_execute_utility(&context.binary_name)
    }

    /// 预处理参数：
    /// 1. 如果参数以工具名称结尾，则使用工具名称
    /// 2. 如果参数不以工具名称结尾，则视为多态并获取下一个参数
    fn preprocess_args(context: &mut ExecutionContext) -> Option<OsString> {
        // 使用更高效的字符串匹配
        if let Some(util) = ALL_COMMANDS.iter().find(|&&util| {
            context.binary_name.ends_with(util)
                && !context.binary_name[..context.binary_name.len() - util.len()]
                    .ends_with(char::is_alphanumeric)
        }) {
            Some(OsString::from(*util))
        } else {
            // 无法匹配的二进制名称 => 视为多态并推进参数列表
            ctcore::ct_set_utility_is_second_arg();
            if !context.args.is_empty() {
                Some(context.args.remove(0))
            } else {
                None
            }
        }
    }

    /// 显示帮助信息，包括可用命令列表
    fn show_help(binary_name: &str) {
        // 预分配字符串缓冲区
        let mut output = String::with_capacity(1024);
        output.push_str(&format!(
            "{} {CT_VERSION} (multi-call binary)\n",
            binary_name
        ));
        output.push_str(&format!(
            "Usage: {} [function [arguments...]]\n",
            binary_name
        ));
        output.push_str("Currently defined functions:\n\n");

        let mut utils: Vec<&str> = ALL_COMMANDS.to_vec();
        utils.sort_unstable();

        let display_list = utils.join(", ");
        let width = std::cmp::min(textwrap::termwidth(), 100) - 4 * 2;

        output.push_str(&textwrap::indent(
            &textwrap::fill(&display_list, width),
            "    ",
        ));
        println!("{}", output);
    }
}

/// 处理特定工具的命令执行
struct CommandHandler {
    util_name: OsString,       // 工具名称
    context: ExecutionContext, // 执行上下文
}

impl CommandHandler {
    /// 创建新的命令处理器
    fn new(util_name: OsString, context: ExecutionContext) -> Self {
        Self { util_name, context }
    }

    /// 尝试执行工具，如果工具不存在则返回 None
    fn try_execute_utility(&self, util_str: &str) -> Option<i32> {
        get_tool(util_str).map(|tool| execute_tool(tool, &self.context.build_args()))
    }

    /// 执行命令
    fn execute(&self) -> ! {
        let util_str = match self.util_name.to_str() {
            Some(util) => util,
            None => {
                Self::report_not_found(&self.util_name);
            }
        };

        // 处理特殊命令
        match util_str {
            "completion" => self.handle_completion(), // 处理shell补全
            "manpage" => self.handle_manpage(),       // 处理man页面生成
            "-h" | "--help" => self.handle_help(),    // 处理帮助请求
            "-v" | "--version" => self.handle_version(), // 处理版本请求
            _ => self.handle_utility(util_str),       // 处理普通工具
        }
    }

    /// 处理工具执行
    fn handle_utility(&self, util_str: &str) -> ! {
        if let Some(exit_code) = self.try_execute_utility(util_str) {
            process::exit(exit_code);
        } else {
            Self::report_not_found(&OsString::from(util_str));
        }
    }

    /// 处理版本请求
    fn handle_version(&self) -> ! {
        println!("{}", CT_VERSION);
        process::exit(0);
    }

    /// 处理帮助请求
    fn handle_help(&self) -> ! {
        SysKits::show_help(&self.context.binary_name);
        process::exit(0);
    }

    /// 处理shell补全生成
    fn handle_completion(&self) -> ! {
        // 预分配容量
        let mut all_utilities = Vec::with_capacity(ALL_COMMANDS.len() + 1);
        all_utilities.push("syskits");
        all_utilities.extend(ALL_COMMANDS.iter().copied());

        // 构建参数列表
        let mut args = Vec::with_capacity(self.context.args.len() + 1);
        args.push(OsString::from("completion"));
        args.extend(self.context.args.iter().cloned());

        // 解析命令行参数
        let matches = Command::new("completion")
            .about("Prints completions to stdout")
            .arg(
                Arg::new("utility")
                    .value_parser(clap::builder::PossibleValuesParser::new(all_utilities))
                    .required(true),
            )
            .arg(
                Arg::new("shell")
                    .value_parser(clap::builder::EnumValueParser::<Shell>::new())
                    .required(true),
            )
            .get_matches_from(args);

        let utility = matches.get_one::<String>("utility").unwrap();
        let shell = *matches.get_one::<Shell>("shell").unwrap();

        // 生成补全脚本
        let mut command = if utility == "syskits" {
            Self::gen_utils_app()
        } else if let Some(tool) = get_tool(utility) {
            tool.command()
        } else {
            eprintln!("Unknown utility: {utility}");
            process::exit(1);
        };

        let bin_name = std::env::var("PROG_PREFIX").unwrap_or_default() + utility;

        clap_complete::generate(shell, &mut command, bin_name, &mut io::stdout());
        io::stdout().flush().unwrap();
        process::exit(0);
    }

    /// 处理man页面生成
    fn handle_manpage(&self) -> ! {
        // 预分配容量
        let mut utilities = Vec::with_capacity(ALL_COMMANDS.len() + 1);
        utilities.push("syskits");
        utilities.extend(ALL_COMMANDS.iter().copied());

        // 解析命令行参数
        let mut commander = Command::new("manpage");
        commander = commander.about("Prints manpage info to stdout");
        commander = commander.arg(
            Arg::new("utility")
                .value_parser(clap::builder::PossibleValuesParser::new(utilities))
                .required(true),
        );
        let ct_args_iter =
            std::iter::once(OsString::from("manpage")).chain(self.context.args.iter().cloned());
        let matches = commander.get_matches_from(ct_args_iter);

        let utility = match matches.get_one::<String>("utility") {
            Some(utility) => utility,
            None => {
                SysKits::show_help("manpage");
                process::exit(1);
            }
        };

        // 生成man页面
        let cmd = Self::get_command(utility);
        let man = clap_mangen::Man::new(cmd);
        man.render(&mut io::stdout())
            .expect("Man page generation failed");
        io::stdout().flush().expect("Failed to flush stdout");
        process::exit(0);
    }

    /// 创建包含所有工具作为子命令的命令
    fn gen_utils_app() -> Command {
        let mut command = Command::new("syskits");
        for &name in ALL_COMMANDS {
            if let Some(tool) = get_tool(name) {
                // 提取工具的说明文本
                let about = tool.command().get_about().unwrap_or_default().to_string();

                // 创建子命令
                let sub_app = Command::new(name).about(about);
                command = command.subcommand(sub_app);
            }
        }
        command
    }

    /// 生成syskits命令及其所有子命令
    fn generate_skits_app() -> Command {
        let mut command = Command::new("syskits");

        for &name in ALL_COMMANDS {
            if let Some(tool) = get_tool(name) {
                let about = tool.command().get_about().unwrap_or_default().to_string();
                let sub_command = Command::new(name).about(about);
                command = command.subcommand(sub_command);
            }
        }

        command
    }

    /// 获取工具的命令对象
    fn get_command(utility: &String) -> Command {
        if utility == "syskits" {
            Self::generate_skits_app()
        } else if let Some(tool) = get_tool(utility) {
            tool.command()
        } else {
            eprintln!("Utility not found: {utility}");
            process::exit(1);
        }
    }

    /// 报告工具未找到并退出
    fn report_not_found(util_os_str: &OsStr) -> ! {
        println!("{}: utility/function not found", util_os_str.maybe_quote());
        process::exit(1);
    }
}

/// 执行工具并处理其结果
fn execute_tool(tool: Box<dyn Tool>, args: &[OsString]) -> i32 {
    let result = tool.execute(args);
    match result {
        Ok(()) => ctcore::ct_error::get_ct_exit_code(),
        Err(err) => {
            let s_err = format!("{}", err);
            if !s_err.is_empty() {
                ctcore::ct_show_error!("{}", s_err);
            }
            if err.usage() {
                eprintln!(
                    "Try '{} --help' for more information.",
                    ctcore::ct_execute_phrase()
                );
            }
            err.code()
        }
    }
}

/// 主函数
fn main() {
    ctcore::ct_panic::ct_mute_set_panic_hook();

    let mut args = ctcore::ct_os_args();
    let execute_path = execute_path(&mut args);
    let execute_as_util = match execute_name(&execute_path) {
        Some(name) => name,
        None => {
            SysKits::show_help("<unknown binary name>");
            process::exit(0);
        }
    };

    // 创建执行上下文
    let context = ExecutionContext::new(
        execute_as_util.to_string(),
        args.collect(), // 将迭代器收集到Vec中
    );

    SysKits::run(context);
}

/// 从路径中提取可执行文件名
fn execute_name(ct_execute_path: &Path) -> Option<&str> {
    ct_execute_path.file_stem()?.to_str()
}

/// 获取执行路径
fn execute_path(ct_args: &mut impl Iterator<Item = OsString>) -> PathBuf {
    if let Some(str) = ct_args.next() {
        if !str.is_empty() {
            return PathBuf::from(str);
        }
    }
    match std::env::current_exe() {
        Ok(path) => path,
        Err(_) => {
            println!("Failed to retrieve current executable path.");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_execute_name() {
        // 测试正常路径
        let path = Path::new("/usr/bin/syskits-ls");
        assert_eq!(execute_name(path), Some("syskits-ls"));

        // 测试无扩展名的路径
        let path = Path::new("/usr/bin/syskits");
        assert_eq!(execute_name(path), Some("syskits"));

        // 测试无效路径
        let path = Path::new("/usr/bin/");
        assert_eq!(execute_name(path), Some("bin"));

        // 测试相对路径
        let path = Path::new("./syskits-ls");
        assert_eq!(execute_name(path), Some("syskits-ls"));
    }

    #[test]
    fn test_preprocess_args() {
        // 测试多态模式
        let mut context = ExecutionContext {
            binary_name: "syskits".to_string(),
            args: vec![OsString::from("arg1"), OsString::from("arg2")],
        };
        let result = SysKits::preprocess_args(&mut context);
        assert_eq!(result, Some(OsString::from("arg1")));

        // 测试空参数
        let mut context = ExecutionContext {
            binary_name: "syskits".to_string(),
            args: vec![],
        };
        let result = SysKits::preprocess_args(&mut context);
        assert_eq!(result, None);

        // 测试不匹配的工具名称
        let mut context = ExecutionContext {
            binary_name: "syskits-unknown".to_string(),
            args: vec![OsString::from("arg1")],
        };
        let result = SysKits::preprocess_args(&mut context);
        assert_eq!(result, Some(OsString::from("arg1")));
    }

    #[test]
    fn test_try_execute_by_binary_name() {
        // 测试不匹配的工具名称
        let context = ExecutionContext {
            binary_name: "nonexistent".to_string(),
            args: vec![OsString::from("arg1")],
        };
        let result = SysKits::try_execute_by_binary_name(&context);
        assert!(result.is_none());

        // 测试空参数
        let context = ExecutionContext {
            binary_name: "syskits".to_string(),
            args: vec![],
        };
        let result = SysKits::try_execute_by_binary_name(&context);
        assert!(result.is_none());
    }

    #[test]
    fn test_execution_context() {
        // 测试上下文克隆
        let context = ExecutionContext {
            binary_name: "test".to_string(),
            args: vec![OsString::from("arg1")],
        };
        let cloned = context.clone();
        assert_eq!(context.binary_name, cloned.binary_name);
        assert_eq!(context.args, cloned.args);
    }

    #[test]
    fn test_subcommand_handler() {
        // 测试版本请求
        let context = ExecutionContext {
            binary_name: "syskits".to_string(),
            args: vec![],
        };
        let _handler = CommandHandler::new(OsString::from("-v"), context);
        // 注意：这里我们无法直接测试 handle_version 因为它会退出程序
        // 在实际测试中，你可能需要重构代码以使其可测试

        // 测试工具未找到
        let context = ExecutionContext {
            binary_name: "syskits".to_string(),
            args: vec![],
        };
        let _handler = CommandHandler::new(OsString::from("nonexistent"), context);
        // 注意：同样，report_not_found 会退出程序
    }

    #[test]
    fn test_execute_path() {
        // 测试正常参数
        let mut args = vec![OsString::from("/usr/bin/syskits")].into_iter();
        let path = execute_path(&mut args);
        assert_eq!(path.to_str().unwrap(), "/usr/bin/syskits");

        // 测试空参数
        let mut args = vec![].into_iter();
        let path = execute_path(&mut args);
        // 这里我们无法直接测试 current_exe() 的结果
        // 但可以确认它返回了一个有效的路径
        assert!(path.exists());
    }

    #[test]
    fn test_show_help() {
        // 测试帮助信息显示
        // 注意：这是一个输出测试，可能需要重定向 stdout 来捕获输出
        SysKits::show_help("syskits");
        // 在实际测试中，你可能需要重构代码以使其可测试
    }
}
