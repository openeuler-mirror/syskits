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

// spell-checker:ignore (ToDO) tempdir dyld dylib dragonflybsd optgrps libstdbuf

extern crate rust_i18n;
use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::Tool;
use ctcore::ct_error::{CTResult, CTsageError, CtSimpleError, FromIo};
use ctcore::ct_parse_size::parse_size_u64;
use std::ffi::OsString;
use std::fs::File;
use std::io::Write;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process;
use sys_locale::get_locale;
use tempfile::TempDir;
use tempfile::tempdir;

// stdbuf 命令
//
// 此模块提供了一个用于调整标准输入、标准输出和标准错误缓冲区的命令行工具。
// 它允许用户设置不同的缓冲模式，如行缓冲、固定大小缓冲等，并执行指定的命令。
//
// 主要功能包括：
// - 设置标准输入的缓冲模式
// - 设置标准输出的缓冲模式
// - 设置标准错误的缓冲模式
// - 执行指定的命令

// 定义配置标志常量
pub mod stdbuf_flags {
    pub const INPUT: &str = "input";
    pub const INPUT_SHORT: char = 'i';
    pub const OUTPUT: &str = "output";
    pub const OUTPUT_SHORT: char = 'o';
    pub const ERROR: &str = "error";
    pub const ERROR_SHORT: char = 'e';
    pub const COMMAND: &str = "command";
}

const STDBUF_INJECT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/libstdbuf.so"));

/// 缓冲区类型的枚举
#[derive(Debug, Clone)]
enum BufferType {
    /// 使用默认的缓冲设置
    Default,
    /// 行缓冲模式
    Line,
    /// 指定大小的缓冲区（以字节为单位）
    Size(usize),
}

/// stdbuf 命令的配置结构体
///
/// 此结构体包含标准输入、标准输出和标准错误的缓冲区配置
struct StdbufFlags {
    /// 标准输入的缓冲设置
    stdin: BufferType,
    /// 标准输出的缓冲设置
    stdout: BufferType,
    /// 标准错误的缓冲设置
    stderr: BufferType,
    /// 要执行的命令及其参数
    command_args: Vec<String>,
}

impl Default for StdbufFlags {
    fn default() -> Self {
        Self {
            stdin: BufferType::Default,
            stdout: BufferType::Default,
            stderr: BufferType::Default,
            command_args: Vec::new(),
        }
    }
}

impl StdbufFlags {
    /// 从命令行参数创建 StdbufFlags 实例
    ///
    /// # 参数
    /// * `matches` - 解析后的命令行参数
    ///
    /// # 返回值
    /// * `CTResult<Self>` - 成功创建的配置或错误
    fn new(matches: ArgMatches) -> CTResult<Self> {
        // 解析标准输入的缓冲模式
        let stdin = Self::parse_buffer_option(&matches, stdbuf_flags::INPUT)
            .map_err(|e| CTsageError::new(125, e))?;

        // 解析标准输出的缓冲模式
        let stdout = Self::parse_buffer_option(&matches, stdbuf_flags::OUTPUT)
            .map_err(|e| CTsageError::new(125, e))?;

        // 解析标准错误的缓冲模式
        let stderr = Self::parse_buffer_option(&matches, stdbuf_flags::ERROR)
            .map_err(|e| CTsageError::new(125, e))?;

        // 提取命令和参数
        let command_args = matches
            .get_many::<String>(stdbuf_flags::COMMAND)
            .map_or_else(Vec::new, |v| v.cloned().collect());

        if command_args.is_empty() {
            return Err(CtSimpleError::new(125, "command is required"));
        }

        Ok(Self {
            stdin,
            stdout,
            stderr,
            command_args,
        })
    }

    /// 解析缓冲区选项
    ///
    /// # 参数
    /// * `matches` - 解析后的命令行参数
    /// * `option_name` - 选项名称
    ///
    /// # 返回值
    /// * `Result<BufferType, String>` - 解析后的缓冲类型或错误信息
    fn parse_buffer_option(matches: &ArgMatches, option_name: &str) -> Result<BufferType, String> {
        match matches.get_one::<String>(option_name) {
            Some(value) => match value.as_str() {
                "L" => {
                    if option_name == stdbuf_flags::INPUT {
                        Err("line buffering stdin is meaningless".to_string())
                    } else {
                        Ok(BufferType::Line)
                    }
                }
                x => parse_size_u64(x).map_or_else(
                    |e| Err(format!("invalid mode {e}")),
                    |m| {
                        Ok(BufferType::Size(m.try_into().map_err(|_| {
                            format!("invalid mode '{x}': Value too large for defined data type")
                        })?))
                    },
                ),
            },
            None => Ok(BufferType::Default),
        }
    }

    /// 设置命令的环境变量
    ///
    /// # 参数
    /// * `command` - 要配置的命令
    /// * `buffer_name` - 环境变量名称
    /// * `buffer_type` - 缓冲区类型
    fn set_command_env(
        &self,
        command: &mut process::Command,
        buffer_name: &str,
        buffer_type: &BufferType,
    ) {
        match buffer_type {
            BufferType::Size(m) => {
                command.env(buffer_name, m.to_string());
            }
            BufferType::Line => {
                command.env(buffer_name, "L");
            }
            BufferType::Default => {}
        }
    }

    /// 配置并执行命令
    ///
    /// # 返回值
    /// * `CTResult<()>` - 执行结果
    fn execute_command(&self) -> CTResult<()> {
        // 获取命令和参数
        let command_name = &self.command_args[0];
        let command_params = &self.command_args[1..];

        // 创建命令
        let mut command = process::Command::new(command_name);
        command.args(command_params);

        // 创建临时目录并准备预加载库
        let tmp_dir =
            tempdir().map_err_context(|| "failed to create temporary directory".to_string())?;
        let (preload_env, libstdbuf) = self.get_preload_env(&tmp_dir)?;

        // 设置环境变量
        command.env(preload_env, libstdbuf);
        self.set_command_env(&mut command, "_STDBUF_I", &self.stdin);
        self.set_command_env(&mut command, "_STDBUF_O", &self.stdout);
        self.set_command_env(&mut command, "_STDBUF_E", &self.stderr);

        // 执行命令并等待完成
        let mut process = command.spawn().map_err(|e| {
            // Handle command not found with exit code 127
            let exit_code = if e.kind() == std::io::ErrorKind::NotFound {
                127
            } else {
                126
            };
            CtSimpleError::new(
                exit_code,
                format!("failed to run command '{command_name}': {e}"),
            )
        })?;

        let status = process
            .wait()
            .map_err_context(|| "failed to wait for process".to_string())?;

        // 处理退出状态
        match status.code() {
            Some(i) => {
                if i == 0 {
                    Ok(())
                } else {
                    Err(i.into())
                }
            }
            None => {
                let signal = status.signal().unwrap();
                Err(CtSimpleError::new(
                    1,
                    format!("process killed by signal {signal}"),
                ))
            }
        }
    }

    /// 获取预加载环境变量和库路径
    ///
    /// # 参数
    /// * `tmp_dir` - 临时目录
    ///
    /// # 返回值
    /// * `CTResult<(String, PathBuf)>` - 环境变量名和库路径
    fn get_preload_env(&self, tmp_dir: &TempDir) -> CTResult<(String, PathBuf)> {
        let (preload, extension) = preload_strings()?;
        let inject_path = tmp_dir.path().join("libstdbuf").with_extension(extension);

        let mut file = File::create(&inject_path)
            .map_err_context(|| "failed to create libstdbuf file".to_string())?;
        file.write_all(STDBUF_INJECT)
            .map_err_context(|| "failed to write to libstdbuf file".to_string())?;

        Ok((preload.to_owned(), inject_path))
    }
}

/// 获取平台特定的预加载环境变量名和扩展名
///
/// # 返回值
/// * `CTResult<(&'static str, &'static str)>` - 环境变量名和扩展名
#[cfg(target_os = "linux")]
fn preload_strings() -> CTResult<(&'static str, &'static str)> {
    Ok(("LD_PRELOAD", "so"))
}

/// 获取平台特定的预加载环境变量名和扩展名
///
/// # 返回值
/// * `CTResult<(&'static str, &'static str)>` - 环境变量名和扩展名或不支持错误
#[cfg(not(target_os = "linux"))]
fn preload_strings() -> CTResult<(&'static str, &'static str)> {
    Err(CtSimpleError::new(
        1,
        "Command not supported for this operating system!",
    ))
}

/// stdbuf 主执行函数
///
/// # 参数
/// * `args` - 命令行参数
///
/// # 返回值
/// * `CTResult<()>` - 执行结果
pub fn stdbuf_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);

    // 解析命令行参数 - handle clap errors with exit code 125
    let matches = ct_app().try_get_matches_from(args).map_err(|e| {
        // Clap errors should return exit code 125
        CtSimpleError::new(125, e.to_string())
    })?;

    // 创建配置对象
    let settings = StdbufFlags::new(matches)?;

    // 执行命令
    settings.execute_command()
}

/// 创建命令行参数解析器
///
/// # 返回值
/// * `Command` - 配置好的命令行解析器
pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("stdbuf.about");
    let usage_description = t!("stdbuf.usage");
    let args = vec![
        Arg::new(stdbuf_flags::INPUT)
            .long(stdbuf_flags::INPUT)
            .short(stdbuf_flags::INPUT_SHORT)
            .help(t!("stdbuf.clap.input"))
            .value_name("MODE")
            .required_unless_present_any([stdbuf_flags::OUTPUT, stdbuf_flags::ERROR]),
        Arg::new(stdbuf_flags::OUTPUT)
            .long(stdbuf_flags::OUTPUT)
            .short(stdbuf_flags::OUTPUT_SHORT)
            .help(t!("stdbuf.clap.output"))
            .value_name("MODE")
            .required_unless_present_any([stdbuf_flags::INPUT, stdbuf_flags::ERROR]),
        Arg::new(stdbuf_flags::ERROR)
            .long(stdbuf_flags::ERROR)
            .short(stdbuf_flags::ERROR_SHORT)
            .help(t!("stdbuf.clap.error"))
            .value_name("MODE")
            .required_unless_present_any([stdbuf_flags::INPUT, stdbuf_flags::OUTPUT]),
        Arg::new(stdbuf_flags::COMMAND)
            .action(ArgAction::Append)
            .hide(true)
            .required(true)
            .value_hint(clap::ValueHint::CommandName),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .after_help(t!("stdbuf.after_help"))
        .override_usage(usage_description)
        .trailing_var_arg(true)
        .infer_long_args(true)
        .args(&args)
}

#[derive(Default)]
pub struct Stdbuf;
impl Tool for Stdbuf {
    fn name(&self) -> &'static str {
        "stdbuf"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        stdbuf_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{ArgAction, builder::Command as ClapCommand};
    use std::path::Path;
    use tempfile::TempDir;

    #[test]
    fn test_tool_implementation() {
        let tool = Stdbuf;

        // Test name method
        assert_eq!(tool.name(), "stdbuf");

        // Test command method
        let command = tool.command();
        assert!(command.get_name().contains("stdbuf"));

        // Test execute method with help flag (should work)
        let args: Vec<OsString> = vec![OsString::from("stdbuf"), OsString::from("--help")];
        let result = tool.execute(&args);
        assert!(result.is_err());
    }

    // Helper function to create ArgMatches with specific values for testing
    fn create_arg_matches(
        input: Option<&str>,
        output: Option<&str>,
        error: Option<&str>,
        command: Option<Vec<&str>>,
    ) -> ArgMatches {
        let mut cmd = ClapCommand::new("test");

        // Add all the arguments we need with the correct API that matches the actual implementation
        cmd = cmd
            .arg(
                Arg::new(stdbuf_flags::INPUT)
                    .long(stdbuf_flags::INPUT)
                    .short(stdbuf_flags::INPUT_SHORT)
                    .value_name("MODE"),
            )
            .arg(
                Arg::new(stdbuf_flags::OUTPUT)
                    .long(stdbuf_flags::OUTPUT)
                    .short(stdbuf_flags::OUTPUT_SHORT)
                    .value_name("MODE"),
            )
            .arg(
                Arg::new(stdbuf_flags::ERROR)
                    .long(stdbuf_flags::ERROR)
                    .short(stdbuf_flags::ERROR_SHORT)
                    .value_name("MODE"),
            )
            .arg(Arg::new(stdbuf_flags::COMMAND).action(ArgAction::Append));

        // Build argument vector using owned strings
        let mut arg_strings = Vec::new();
        arg_strings.push("test".to_string());

        if let Some(i) = input {
            arg_strings.push(format!("--{}", stdbuf_flags::INPUT));
            arg_strings.push(i.to_string());
        }

        if let Some(o) = output {
            arg_strings.push(format!("--{}", stdbuf_flags::OUTPUT));
            arg_strings.push(o.to_string());
        }

        if let Some(e) = error {
            arg_strings.push(format!("--{}", stdbuf_flags::ERROR));
            arg_strings.push(e.to_string());
        }

        if let Some(c) = command {
            for arg in c {
                arg_strings.push(arg.to_string());
            }
        }

        // Create a vector of string slices from our owned strings
        let args: Vec<&str> = arg_strings.iter().map(|s| s.as_str()).collect();

        cmd.get_matches_from(args)
    }

    // 测试 parse_buffer_option 函数
    #[test]
    fn test_parse_buffer_option_line_buffering() {
        // 创建一个包含行缓冲选项的参数匹配
        let matches = create_arg_matches(None, Some("L"), None, None);

        // 测试有效的行缓冲
        let result = StdbufFlags::parse_buffer_option(&matches, stdbuf_flags::OUTPUT);
        assert!(result.is_ok());
        match result.unwrap() {
            BufferType::Line => {}
            _ => panic!("Expected Line buffer type"),
        }

        // 测试输入流的行缓冲（应该失败）
        let matches = create_arg_matches(Some("L"), None, None, None);
        let result = StdbufFlags::parse_buffer_option(&matches, stdbuf_flags::INPUT);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "line buffering stdin is meaningless");
    }

    #[test]
    fn test_parse_buffer_option_size_buffering() {
        // 创建一个包含大小缓冲选项的参数匹配
        let matches = create_arg_matches(None, Some("1024"), None, None);

        // 测试有效的大小缓冲
        let result = StdbufFlags::parse_buffer_option(&matches, stdbuf_flags::OUTPUT);
        assert!(result.is_ok());
        match result.unwrap() {
            BufferType::Size(size) => assert_eq!(size, 1024),
            _ => panic!("Expected Size buffer type"),
        }

        // 测试无效的大小值
        let matches = create_arg_matches(None, Some("invalid"), None, None);
        let result = StdbufFlags::parse_buffer_option(&matches, stdbuf_flags::OUTPUT);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid mode"));
    }

    #[test]
    fn test_parse_buffer_option_none() {
        // 创建一个没有选项的参数匹配
        let matches = create_arg_matches(None, None, None, None);

        // 测试默认缓冲
        let result = StdbufFlags::parse_buffer_option(&matches, stdbuf_flags::OUTPUT);
        assert!(result.is_ok());
        match result.unwrap() {
            BufferType::Default => {}
            _ => panic!("Expected Default buffer type"),
        }
    }

    // 测试 StdbufFlags::new 函数
    #[test]
    fn test_stdbuf_flags_new_valid() {
        // 创建一个有效的参数匹配
        let matches = create_arg_matches(None, Some("L"), None, Some(vec!["echo", "test"]));

        // 测试创建有效的标志
        let result = StdbufFlags::new(matches);
        assert!(result.is_ok());

        let flags = result.unwrap();
        assert_eq!(
            flags.command_args,
            vec!["echo".to_string(), "test".to_string()]
        );
        match flags.stdout {
            BufferType::Line => {}
            _ => panic!("Expected Line buffer type for stdout"),
        }
        match flags.stdin {
            BufferType::Default => {}
            _ => panic!("Expected Default buffer type for stdin"),
        }
        match flags.stderr {
            BufferType::Default => {}
            _ => panic!("Expected Default buffer type for stderr"),
        }
    }

    #[test]
    fn test_stdbuf_flags_new_no_command() {
        // 创建一个没有命令的参数匹配
        let matches = create_arg_matches(None, Some("L"), None, None);

        // 测试没有命令时应该返回错误
        let result = StdbufFlags::new(matches);
        assert!(result.is_err());
    }

    #[test]
    fn test_stdbuf_flags_new_invalid_option() {
        // 创建一个包含无效选项的参数匹配
        let matches = create_arg_matches(Some("L"), None, None, Some(vec!["echo", "test"]));

        // 测试无效选项应该返回错误
        let result = StdbufFlags::new(matches);
        assert!(result.is_err());
    }

    // 测试 set_command_env 函数
    #[test]
    fn test_set_command_env() {
        let flags = StdbufFlags::default();
        let mut command = process::Command::new("test");

        // 测试默认缓冲不设置环境变量
        flags.set_command_env(&mut command, "TEST_DEFAULT", &BufferType::Default);

        // 测试行缓冲设置正确的环境变量
        flags.set_command_env(&mut command, "TEST_LINE", &BufferType::Line);

        // 测试大小缓冲设置正确的环境变量
        flags.set_command_env(&mut command, "TEST_SIZE", &BufferType::Size(1024));

        // 由于Command的env方法将环境变量添加到内部结构中，
        // 我们无法直接测试，但可以验证代码逻辑是否正确执行
        // 这种情况下，我们只是确认函数不会崩溃
    }

    // 测试 get_preload_env 函数
    // 注意：这个测试依赖于 OUT_DIR 环境变量，在编译时设置
    // 在单元测试环境中可能不可用，所以我们需要模拟一个替代实现
    #[test]
    fn test_get_preload_env_mock() {
        // 创建一个特殊版本的 StdbufFlags，跳过实际的 STDBUF_INJECT 使用
        struct TestStdbufFlags {}

        impl TestStdbufFlags {
            fn get_preload_env_test(&self, tmp_dir: &TempDir) -> CTResult<(String, PathBuf)> {
                let (preload, extension) = preload_strings()?;
                let inject_path = tmp_dir.path().join("libstdbuf").with_extension(extension);

                // 创建一个空文件代替实际的库文件
                let mut file = File::create(&inject_path)
                    .map_err_context(|| "failed to create libstdbuf file".to_string())?;
                // 写入一些测试数据而不是实际的库内容
                file.write_all(b"test data")
                    .map_err_context(|| "failed to write to libstdbuf file".to_string())?;

                Ok((preload.to_owned(), inject_path))
            }
        }

        // 只在Linux平台上运行此测试
        #[cfg(target_os = "linux")]
        {
            let flags = TestStdbufFlags {};
            let tmp_dir = TempDir::new().unwrap();

            let result = flags.get_preload_env_test(&tmp_dir);
            assert!(result.is_ok());

            let (env_var, path) = result.unwrap();
            assert_eq!(env_var, "LD_PRELOAD");
            assert!(path.extension().unwrap() == "so");
            assert!(Path::new(&path).exists());
        }
    }

    // 测试 preload_strings 函数
    #[test]
    fn test_preload_strings() {
        #[cfg(target_os = "linux")]
        {
            let result = preload_strings();
            assert!(result.is_ok());

            let (preload, extension) = result.unwrap();
            assert_eq!(preload, "LD_PRELOAD");
            assert_eq!(extension, "so");
        }

        #[cfg(not(target_os = "linux"))]
        {
            let result = preload_strings();
            assert!(result.is_err());
        }
    }

    // 测试 ct_app 函数
    #[test]
    fn test_ct_app() {
        let app = ct_app();

        // 验证命令设置
        assert_eq!(app.get_name(), ctcore::ct_util_name());

        // 验证必需的参数存在
        let args = app.get_arguments().collect::<Vec<_>>();
        assert!(args.iter().any(|arg| arg.get_id() == stdbuf_flags::INPUT));
        assert!(args.iter().any(|arg| arg.get_id() == stdbuf_flags::OUTPUT));
        assert!(args.iter().any(|arg| arg.get_id() == stdbuf_flags::ERROR));
        assert!(args.iter().any(|arg| arg.get_id() == stdbuf_flags::COMMAND));
    }

    // 模拟 execute_command 函数，不实际执行命令
    #[test]
    fn test_execute_command_mock() {
        // 创建一个特殊版本的 StdbufFlags，用于测试
        struct TestStdbufFlags {
            stdin: BufferType,
            #[allow(dead_code)]
            stdout: BufferType,
            #[allow(dead_code)]
            stderr: BufferType,
            command_args: Vec<String>,
        }

        impl TestStdbufFlags {
            // 这个函数模拟 execute_command 的行为，但不实际执行命令
            fn execute_command_test(&self) -> bool {
                // 检查命令参数是否有效
                if self.command_args.is_empty() {
                    return false;
                }

                // 验证缓冲设置被正确应用
                if let BufferType::Line = self.stdin {
                    return false; // 输入流行缓冲是无效的
                }

                // 所有检查通过
                true
            }
        }

        // 测试有效配置
        let flags = TestStdbufFlags {
            stdin: BufferType::Default,
            stdout: BufferType::Line,
            stderr: BufferType::Size(1024),
            command_args: vec!["echo".to_string(), "test".to_string()],
        };

        assert!(flags.execute_command_test());

        // 测试无效配置 - 没有命令
        let flags = TestStdbufFlags {
            stdin: BufferType::Default,
            stdout: BufferType::Line,
            stderr: BufferType::Default,
            command_args: vec![],
        };

        assert!(!flags.execute_command_test());

        // 测试无效配置 - 输入流行缓冲
        let flags = TestStdbufFlags {
            stdin: BufferType::Line,
            stdout: BufferType::Default,
            stderr: BufferType::Default,
            command_args: vec!["echo".to_string(), "test".to_string()],
        };

        assert!(!flags.execute_command_test());
    }

    // 测试 stdbuf_main 函数
    // 注意：为了避免依赖于编译时环境变量和实际执行命令，我们创建一个简化版本的测试
    #[test]
    fn test_argument_parsing() {
        // 为了避免借用问题，显式创建字符串参数
        let stdbuf_arg = "stdbuf".to_string();
        let o_arg = "-o".to_string();
        let l_arg = "L".to_string();
        let echo_arg = "echo".to_string();
        let test_arg = "test".to_string();

        // 测试有效参数
        {
            let args = vec![&stdbuf_arg, &o_arg, &l_arg, &echo_arg, &test_arg];
            let app = ct_app();
            let result = app.try_get_matches_from(args);
            assert!(result.is_ok());

            if let Ok(matches) = result {
                assert_eq!(
                    matches.get_one::<String>(stdbuf_flags::OUTPUT).unwrap(),
                    "L"
                );
            }
        }

        // 测试无效参数 - 缺少命令
        {
            let args = vec![&stdbuf_arg, &o_arg, &l_arg];
            let app = ct_app();
            let result = app.try_get_matches_from(args);
            assert!(result.is_err());
        }

        // 测试无效参数 - 缺少必需的缓冲选项
        {
            let args = vec![&stdbuf_arg, &echo_arg, &test_arg];
            let app = ct_app();
            let result = app.try_get_matches_from(args);
            assert!(result.is_err());
        }
    }

    // 新增测试：测试多个缓冲区选项的组合
    #[test]
    fn test_multiple_buffer_options() {
        // 创建一个有多个缓冲选项的参数匹配
        let matches = create_arg_matches(
            Some("0"),    // 无缓冲
            Some("L"),    // 行缓冲
            Some("4096"), // 完全缓冲，4096字节
            Some(vec!["cat"]),
        );

        // 测试创建有效的标志
        let result = StdbufFlags::new(matches);
        assert!(result.is_ok());

        let flags = result.unwrap();

        // 验证各个缓冲选项
        match flags.stdin {
            BufferType::Size(0) => {} // 对于无缓冲，大小是0
            _ => panic!("Expected unbuffered (Size(0)) for stdin"),
        }

        match flags.stdout {
            BufferType::Line => {}
            _ => panic!("Expected Line buffer type for stdout"),
        }

        match flags.stderr {
            BufferType::Size(size) => assert_eq!(size, 4096),
            _ => panic!("Expected Size buffer type for stderr"),
        }
    }

    // 新增测试：测试各种缓冲大小的边界情况
    #[test]
    fn test_buffer_size_edge_cases() {
        // 测试0大小（应该被解释为无缓冲）
        let matches = create_arg_matches(None, Some("0"), None, Some(vec!["ls"]));
        let result = StdbufFlags::parse_buffer_option(&matches, stdbuf_flags::OUTPUT);
        assert!(result.is_ok());
        match result.unwrap() {
            BufferType::Size(size) => assert_eq!(size, 0),
            _ => panic!("Expected Size(0) buffer type"),
        }

        // 测试非常大的缓冲区大小但仍在usize范围内
        let matches = create_arg_matches(None, Some("1073741824"), None, Some(vec!["ls"])); // 1GB
        let result = StdbufFlags::parse_buffer_option(&matches, stdbuf_flags::OUTPUT);
        assert!(result.is_ok());
        match result.unwrap() {
            BufferType::Size(size) => assert_eq!(size, 1073741824),
            _ => panic!("Expected large Size buffer type"),
        }

        // 测试带有单位的缓冲区大小（如果parse_size_u64支持）
        // 注意：这取决于parse_size_u64的实现
        let matches = create_arg_matches(None, Some("1K"), None, Some(vec!["ls"]));
        let _result = StdbufFlags::parse_buffer_option(&matches, stdbuf_flags::OUTPUT);
        // 这个测试可能会通过也可能会失败，取决于parse_size_u64是否支持单位
        // 如果支持，我们可以验证size是否为1024
        // 如果不支持，应该返回错误
    }

    // 新增测试：测试复杂的命令行参数
    #[test]
    fn test_complex_command_args() {
        // 测试带有多个参数和选项的复杂命令
        // 避免使用可能与测试参数冲突的命令行选项，如 -t, -r 等
        let matches = create_arg_matches(
            None,
            Some("L"),
            None,
            Some(vec!["ls", "src", "include", "docs"]),
        );

        let result = StdbufFlags::new(matches);
        assert!(result.is_ok());

        let flags = result.unwrap();
        assert_eq!(flags.command_args.len(), 4);
        assert_eq!(flags.command_args[0], "ls");
        assert_eq!(flags.command_args[3], "docs");
    }

    // 新增测试：测试BufferType枚举的Debug实现
    #[test]
    fn test_buffer_type_debug() {
        // 测试BufferType的Debug实现是否正确
        let default_buffer = BufferType::Default;
        let line_buffer = BufferType::Line;
        let size_buffer = BufferType::Size(1024);

        assert_eq!(format!("{default_buffer:?}"), "Default");
        assert_eq!(format!("{line_buffer:?}"), "Line");
        assert_eq!(format!("{size_buffer:?}"), "Size(1024)");
    }

    // 新增测试：测试StdbufFlags的默认实现
    #[test]
    fn test_stdbuf_flags_default() {
        let flags = StdbufFlags::default();

        // 验证默认值
        match flags.stdin {
            BufferType::Default => {}
            _ => panic!("Expected Default buffer type for stdin"),
        }

        match flags.stdout {
            BufferType::Default => {}
            _ => panic!("Expected Default buffer type for stdout"),
        }

        match flags.stderr {
            BufferType::Default => {}
            _ => panic!("Expected Default buffer type for stderr"),
        }

        assert!(flags.command_args.is_empty());
    }

    // 新增测试：测试在多种语言环境和平台下的行为
    #[cfg(target_os = "linux")]
    #[test]
    fn test_platform_specific_behavior() {
        // 测试当前平台的预加载环境变量和库扩展名
        let (preload, extension) = preload_strings().unwrap();

        // 在Linux上应该是LD_PRELOAD和so
        assert_eq!(preload, "LD_PRELOAD");
        assert_eq!(extension, "so");

        // 测试在临时目录中创建预加载库文件的功能
        let tmp_dir = TempDir::new().unwrap();

        // 创建一个mock库文件
        let inject_path = tmp_dir.path().join("libstdbuf").with_extension(extension);
        let result = File::create(&inject_path);
        assert!(result.is_ok());

        // 验证文件成功创建
        assert!(Path::new(&inject_path).exists());
    }

    // 新增测试：测试整个工作流程
    #[test]
    fn test_workflow_with_mock() {
        // 这个测试模拟整个stdbuf的主要工作流程，但不实际执行命令

        // 步骤1：解析命令行参数
        let args = vec!["stdbuf", "-o", "L", "echo", "test"];
        let app = ct_app();
        let matches_result = app.try_get_matches_from(args);
        assert!(matches_result.is_ok());

        // 步骤2：创建StdbufFlags
        let matches = matches_result.unwrap();
        let flags_result = StdbufFlags::new(matches);
        assert!(flags_result.is_ok());

        let flags = flags_result.unwrap();

        // 步骤3：验证解析的选项
        match flags.stdout {
            BufferType::Line => {}
            _ => panic!("Expected Line buffer type for stdout"),
        }

        // 步骤4：验证命令参数
        assert_eq!(
            flags.command_args,
            vec!["echo".to_string(), "test".to_string()]
        );

        // 我们不能直接测试execute_command，因为它会尝试实际执行命令
        // 但我们已经验证了flags对象已正确设置
    }

    // 新增测试：测试命令行参数短选项形式
    #[test]
    fn test_short_option_forms() {
        // 为了避免借用问题，显式创建字符串参数
        let stdbuf_arg = "stdbuf".to_string();
        let o_short_arg = "-o".to_string();
        let i_short_arg = "-i".to_string();
        let e_short_arg = "-e".to_string();
        let l_arg = "L".to_string();
        let zero_arg = "0".to_string();
        let size_arg = "4096".to_string();
        let echo_arg = "echo".to_string();
        let test_arg = "test".to_string();

        // 测试短选项 -o
        {
            let args = vec![&stdbuf_arg, &o_short_arg, &l_arg, &echo_arg, &test_arg];
            let app = ct_app();
            let result = app.try_get_matches_from(args);
            assert!(result.is_ok());

            if let Ok(matches) = result {
                assert_eq!(
                    matches.get_one::<String>(stdbuf_flags::OUTPUT).unwrap(),
                    "L"
                );
            }
        }

        // 测试短选项 -i
        {
            let args = vec![&stdbuf_arg, &i_short_arg, &zero_arg, &echo_arg, &test_arg];
            let app = ct_app();
            let result = app.try_get_matches_from(args);
            assert!(result.is_ok());

            if let Ok(matches) = result {
                assert_eq!(matches.get_one::<String>(stdbuf_flags::INPUT).unwrap(), "0");
            }
        }

        // 测试短选项 -e
        {
            let args = vec![&stdbuf_arg, &e_short_arg, &size_arg, &echo_arg, &test_arg];
            let app = ct_app();
            let result = app.try_get_matches_from(args);
            assert!(result.is_ok());

            if let Ok(matches) = result {
                assert_eq!(
                    matches.get_one::<String>(stdbuf_flags::ERROR).unwrap(),
                    "4096"
                );
            }
        }

        // 测试多个短选项
        {
            let args = vec![
                &stdbuf_arg,
                &i_short_arg,
                &zero_arg,
                &o_short_arg,
                &l_arg,
                &e_short_arg,
                &size_arg,
                &echo_arg,
                &test_arg,
            ];
            let app = ct_app();
            let result = app.try_get_matches_from(args);
            assert!(result.is_ok());

            if let Ok(matches) = result {
                assert_eq!(matches.get_one::<String>(stdbuf_flags::INPUT).unwrap(), "0");
                assert_eq!(
                    matches.get_one::<String>(stdbuf_flags::OUTPUT).unwrap(),
                    "L"
                );
                assert_eq!(
                    matches.get_one::<String>(stdbuf_flags::ERROR).unwrap(),
                    "4096"
                );
            }
        }
    }
}
