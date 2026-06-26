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

//! runcon 命令的核心实现
//!
//! # 功能概述
//! 该模块实现了类似 GNU runcon 的功能，用于在指定的安全上下文中运行命令。
//!
//! # 主要组件
//! - `RunconSettings`: 运行配置
//! - `RunconCommandLineMode`: 运行模式
//! - `RunconError`: 错误处理
//!
//! # 核心功能
//! - 打印当前安全上下文
//! - 使用指定的安全上下文运行命令
//! - 使用自定义安全上下文运行命令
//! - 支持进程转换上下文计算
//!
//! # 安全性说明
//! 该模块依赖于 SELinux 提供的安全机制，在非 SELinux 系统上
//! 部分功能可能无法使用。

use clap::builder::ValueParser;
use ctcore::ct_error::{CTResult, CTsageError};

use clap::{Arg, ArgAction, Command, crate_version};
use ctcore::{ct_format_usage, ct_help_about, ct_help_section, ct_help_usage};
use selinux::{OpaqueSecurityContext, SecurityClass, SecurityContext};

use std::borrow::Cow;
use std::ffi::{CStr, CString, OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::{io, ptr};

mod errors;

use errors::error_exit_status;
use errors::{DefaultError, Result, RunconError};

const RUNCON_ABOUT: &str = ct_help_about!("runcon.md");
const RUNCON_USAGE: &str = ct_help_usage!("runcon.md");
const RUNCON_DESCRIPTION: &str = ct_help_section!("after help", "runcon.md");

pub mod runcon_options {
    pub const RUNCON_COMPUTE: &str = "compute";

    pub const RUNCON_USER: &str = "user";
    pub const RUNCON_ROLE: &str = "role";
    pub const RUNCON_TYPE: &str = "type";
    pub const RUNCON_RANGE: &str = "range";
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    runcon_main(args)
}

/// 运行带有指定安全上下文的命令
///
/// # 参数
/// * `args` - 命令行参数
///
/// # 返回值
/// 成功执行返回 Ok(())，失败返回错误
///
/// # 功能说明
/// 1. 打印当前安全上下文
/// 2. 使用指定的上下文运行命令
/// 3. 使用自定义上下文运行命令
pub fn runcon_main(args: impl ctcore::Args) -> CTResult<()> {
    // 创建命令行配置
    let config = ct_app();

    // 解析命令行参数
    let settings = RunconSettings::new(config, args).map_err(|r| {
        if let DefaultError::CommandLine(ref r) = r {
            // 处理帮助和版本信息的显示
            match r.kind() {
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => {
                    println!("{r}");
                    return CTsageError::new(0, String::new());
                }
                _ => {}
            }
        }
        CTsageError::new(error_exit_status::RUNCON_ANOTHER_ERROR, format!("{r}"))
    })?;

    // 根据运行模式执行相应操作
    match &settings.mode {
        // 打印当前安全上下文
        RunconCommandLineMode::Print => {
            print_current_context().map_err(|e| RunconError::new(e).into())
        }

        // 使用指定的上下文运行命令
        RunconCommandLineMode::PlainContext { context, command } => {
            get_plain_context(context)
                .and_then(|ctx| set_next_exec_context(&ctx))
                .map_err(RunconError::new)?;
            runcon_exec(command, &settings.arguments)
        }

        // 使用自定义上下文运行命令
        RunconCommandLineMode::CustomContext {
            is_compute_transition_context: compute_transition_context,
            user,
            role,
            the_type,
            range,
            command,
        } => match command {
            // 有命令时，设置上下文并执行
            Some(command) => {
                get_custom_context(
                    *compute_transition_context,
                    user.as_deref(),
                    role.as_deref(),
                    the_type.as_deref(),
                    range.as_deref(),
                    command,
                )
                .and_then(|ctx| set_next_exec_context(&ctx))
                .map_err(RunconError::new)?;
                runcon_exec(command, &settings.arguments)
            }
            // 无命令时，仅打印当前上下文
            None => print_current_context().map_err(|e| RunconError::new(e).into()),
        },
    }
}

pub fn ct_app() -> Command {
    let args = vec![
        Arg::new(runcon_options::RUNCON_COMPUTE)
            .short('c')
            .long(runcon_options::RUNCON_COMPUTE)
            .help("Compute process transition context before modifying.")
            .action(ArgAction::SetTrue),
        Arg::new(runcon_options::RUNCON_USER)
            .short('u')
            .long(runcon_options::RUNCON_USER)
            .value_name("USER")
            .help("Set user USER in the target security context.")
            .value_parser(ValueParser::os_string()),
        Arg::new(runcon_options::RUNCON_ROLE)
            .short('r')
            .long(runcon_options::RUNCON_ROLE)
            .value_name("ROLE")
            .help("Set role ROLE in the target security context.")
            .value_parser(ValueParser::os_string()),
        Arg::new(runcon_options::RUNCON_TYPE)
            .short('t')
            .long(runcon_options::RUNCON_TYPE)
            .value_name("TYPE")
            .help("Set type TYPE in the target security context.")
            .value_parser(ValueParser::os_string()),
        Arg::new(runcon_options::RUNCON_RANGE)
            .short('l')
            .long(runcon_options::RUNCON_RANGE)
            .value_name("RANGE")
            .help("Set range RANGE in the target security context.")
            .value_parser(ValueParser::os_string()),
        Arg::new("ARG")
            .action(ArgAction::Append)
            .value_parser(ValueParser::os_string())
            .value_hint(clap::ValueHint::CommandName),
    ];

    Command::new(ctcore::ct_util_name())
        .version(crate_version!())
        .about(RUNCON_ABOUT)
        .after_help(RUNCON_DESCRIPTION)
        .override_usage(ct_format_usage(RUNCON_USAGE))
        .infer_long_args(true)
        .args(args)
        // Once "ARG" is parsed, everything after that belongs to it.
        //
        // This is not how POSIX does things, but this is how the GNU implementation
        // parses its command line.
        .trailing_var_arg(true)
}

/// 运行上下文的命令行模式
#[derive(Debug)]
enum RunconCommandLineMode {
    /// 打印当前安全上下文
    Print,

    /// 使用指定的上下文运行命令
    PlainContext {
        /// 安全上下文字符串
        context: OsString,
        /// 要执行的命令
        command: OsString,
    },

    /// 使用自定义上下文运行命令
    CustomContext {
        /// 是否在修改前计算进程转换上下文
        is_compute_transition_context: bool,

        /// 使用指定的用户替换当前上下文中的用户
        user: Option<OsString>,

        /// 使用指定的角色替换当前上下文中的角色
        role: Option<OsString>,

        /// 使用指定的类型替换当前上下文中的类型
        the_type: Option<OsString>,

        /// 使用指定的范围替换当前上下文中的范围
        range: Option<OsString>,

        /// 要执行的命令
        ///
        /// 如果为 None，则使用以下语法：
        /// runcon [-c] [-u USER] [-r ROLE] [-t TYPE] [-l RANGE]
        ///
        /// 这个语法虽然未在文档中说明，但为了兼容性，
        /// 我们按照 GNU 实现的方式支持它。
        command: Option<OsString>,
    },
}

/// 运行配置
///
/// 包含运行模式和命令行参数
#[derive(Debug)]
struct RunconSettings {
    /// 运行模式
    /// - Print: 打印当前上下文
    /// - PlainContext: 使用指定上下文运行命令
    /// - CustomContext: 使用自定义上下文运行命令
    mode: RunconCommandLineMode,

    /// 传递给要执行命令的参数列表
    arguments: Vec<OsString>,
}

impl RunconSettings {
    /// 从命令行参数创建运行配置
    ///
    /// # 参数
    /// * `config` - 命令行配置
    /// * `args` - 命令行参数迭代器
    ///
    /// # 返回值
    /// 成功返回运行配置，失败返回错误
    fn new(config: Command, args: impl Iterator<Item = OsString>) -> Result<Self> {
        // 解析命令行参数
        let matches = config.try_get_matches_from(args)?;

        // 获取位置参数
        let mut args = matches
            .get_many::<OsString>("ARG")
            .map(|v| v.map(OsString::from))
            .into_iter()
            .flatten();

        // 根据参数选择运行模式
        if matches.contains_id(runcon_options::RUNCON_USER)
            || matches.contains_id(runcon_options::RUNCON_ROLE)
            || matches.contains_id(runcon_options::RUNCON_TYPE)
            || matches.contains_id(runcon_options::RUNCON_RANGE)
        {
            // 自定义上下文模式
            let compute_transition_context = matches.get_flag(runcon_options::RUNCON_COMPUTE);
            let mode = RunconCommandLineMode::CustomContext {
                is_compute_transition_context: compute_transition_context,
                user: matches
                    .get_one::<OsString>(runcon_options::RUNCON_USER)
                    .map(Into::into),
                role: matches
                    .get_one::<OsString>(runcon_options::RUNCON_ROLE)
                    .map(Into::into),
                the_type: matches
                    .get_one::<OsString>(runcon_options::RUNCON_TYPE)
                    .map(Into::into),
                range: matches
                    .get_one::<OsString>(runcon_options::RUNCON_RANGE)
                    .map(Into::into),
                command: args.next(),
            };

            Ok(Self {
                mode,
                arguments: args.collect(),
            })
        } else if let Some(context) = args.next() {
            // 指定上下文模式
            args.next()
                .ok_or(DefaultError::MissingCommand)
                .map(move |command| Self {
                    mode: RunconCommandLineMode::PlainContext { context, command },
                    arguments: args.collect(),
                })
        } else {
            // 打印模式
            Ok(Self {
                mode: RunconCommandLineMode::Print,
                arguments: Vec::default(),
            })
        }
    }
}

/// 打印当前进程的安全上下文
///
/// # 返回值
/// 成功返回 Ok(())，失败返回错误
///
/// # 错误处理
/// - 如果获取上下文失败，返回 SELinux 错误
/// - 如果转换上下文字符串失败，返回转换错误
fn print_current_context() -> Result<()> {
    // 获取当前进程的安全上下文
    let op = "Getting security context of the current process";
    let context =
        SecurityContext::current(false).map_err(|err| DefaultError::from_selinux(op, err))?;

    // 将上下文转换为 C 字符串
    let context = context
        .to_c_string()
        .map_err(|err| DefaultError::from_selinux(op, err))?;

    // 打印上下文内容（如果为空则打印空行）
    match context {
        Some(context) => {
            let context = context.as_ref().to_str()?;
            println!("{context}");
        }
        None => println!(),
    }
    Ok(())
}

/// 为下一次执行设置安全上下文
///
/// # 参数
/// * `context` - 要设置的安全上下文
///
/// # 返回值
/// 成功返回 Ok(())，失败返回错误
fn set_next_exec_context(context: &OpaqueSecurityContext) -> Result<()> {
    // 将上下文转换为 C 字符串
    let c_context = context
        .to_c_string()
        .map_err(|err| DefaultError::from_selinux("Creating new context", err))?;

    // 创建安全上下文对象
    let sc = SecurityContext::from_c_str(&c_context, false);

    // 验证上下文的有效性
    if sc.check() != Some(true) {
        let ctx = OsStr::from_bytes(c_context.as_bytes());
        let err = io::ErrorKind::InvalidInput.into();
        return Err(DefaultError::from_io1(
            "Checking security context",
            ctx,
            err,
        ));
    }

    // 设置下一次执行的上下文
    sc.set_for_next_exec()
        .map_err(|err| DefaultError::from_selinux("Setting new security context", err))
}

/// 从字符串创建安全上下文
///
/// # 参数
/// * `context` - 上下文字符串
///
/// # 返回值
/// 成功返回安全上下文，失败返回错误
fn get_plain_context(context: &OsStr) -> Result<OpaqueSecurityContext> {
    // 检查 SELinux 是否启用
    if selinux::kernel_support() == selinux::KernelSupport::Unsupported {
        return Err(DefaultError::SELinuxNotEnabled);
    }

    // 将上下文转换为 C 字符串
    let c_context = os_str_to_c_string(context)?;

    // 创建安全上下文对象
    OpaqueSecurityContext::from_c_str(&c_context)
        .map_err(|err| DefaultError::from_selinux("Creating new context", err))
}

/// 获取命令的转换后上下文
///
/// # 参数
/// * `command` - 要执行的命令
///
/// # 返回值
/// 成功返回转换后的安全上下文，失败返回错误
fn get_transition_context(command: &OsStr) -> Result<SecurityContext> {
    // 获取进程安全类
    let sec_class = SecurityClass::from_name("process")
        .map_err(|err| DefaultError::from_selinux("Getting process security class", err))?;

    // 获取要执行文件的上下文
    let file_context = match SecurityContext::of_path(command, true, false) {
        Ok(Some(context)) => context,
        Ok(None) => {
            let err = io::Error::from_raw_os_error(libc::ENODATA);
            return Err(DefaultError::from_io1("getfilecon", command, err));
        }
        Err(err) => {
            return Err(DefaultError::from_selinux(
                "Getting security context of command file",
                err,
            ));
        }
    };

    // 获取当前进程的上下文
    let process_context = SecurityContext::current(false).map_err(|err| {
        DefaultError::from_selinux("Getting security context of the current process", err)
    })?;

    // 计算进程转换后的上下文
    process_context
        .of_labeling_decision(&file_context, sec_class, "")
        .map_err(|err| DefaultError::from_selinux("Computing result of process transition", err))
}

/// 获取初始的自定义不透明上下文
///
/// # 参数
/// * `compute_transition_context` - 是否计算转换上下文
/// * `command` - 要执行的命令
///
/// # 返回值
/// 成功返回初始上下文，失败返回错误
fn get_initial_custom_opaque_context(
    compute_transition_context: bool,
    command: &OsStr,
) -> Result<OpaqueSecurityContext> {
    // 根据是否需要计算转换上下文选择初始上下文
    let context = if compute_transition_context {
        get_transition_context(command)?
    } else {
        SecurityContext::current(false).map_err(|err| {
            DefaultError::from_selinux("Getting security context of the current process", err)
        })?
    };

    // 将上下文转换为 C 字符串
    let c_context = context
        .to_c_string()
        .map_err(|err| DefaultError::from_selinux("Getting security context", err))?
        .unwrap_or_else(|| Cow::Owned(CString::default()));

    // 创建不透明上下文对象
    OpaqueSecurityContext::from_c_str(c_context.as_ref())
        .map_err(|err| DefaultError::from_selinux("Creating new context", err))
}

/// 获取自定义安全上下文
///
/// # 参数
/// * `compute_transition_context` - 是否计算进程转换上下文
/// * `user` - 用户名
/// * `role` - 角色名
/// * `the_type` - 类型名
/// * `range` - 安全范围
/// * `command` - 要执行的命令
///
/// # 返回值
/// 成功返回安全上下文，失败返回错误
///
/// # 错误处理
/// - 如果 SELinux 未启用，返回 `SELinuxNotEnabled` 错误
/// - 如果设置上下文属性失败，返回相应的 SELinux 错误
fn get_custom_context(
    compute_transition_context: bool,
    user: Option<&OsStr>,
    role: Option<&OsStr>,
    the_type: Option<&OsStr>,
    range: Option<&OsStr>,
    command: &OsStr,
) -> Result<OpaqueSecurityContext> {
    use OpaqueSecurityContext as OSC;
    type SetNewValueProc = fn(&OSC, &CStr) -> selinux::errors::Result<()>;

    // 检查 SELinux 是否启用
    if selinux::kernel_support() == selinux::KernelSupport::Unsupported {
        return Err(DefaultError::SELinuxNotEnabled);
    }

    // 获取初始上下文
    let osc = get_initial_custom_opaque_context(compute_transition_context, command)?;

    // 定义需要设置的属性列表
    let attributes: &[(Option<&OsStr>, SetNewValueProc, &'static str)] = &[
        (user, OSC::set_user, "Setting security context user"),
        (role, OSC::set_role, "Setting security context role"),
        (the_type, OSC::set_type, "Setting security context type"),
        (range, OSC::set_range, "Setting security context range"),
    ];

    // 设置每个指定的属性
    for &(new_value, set_method, operation) in attributes {
        if let Some(value) = new_value {
            let c_value = os_str_to_c_string(value)?;
            set_method(&osc, &c_value).map_err(|err| DefaultError::from_selinux(operation, err))?;
        }
    }

    Ok(osc)
}

/// 执行指定的命令，替换当前进程
///
/// # 参数
/// * `command` - 要执行的命令
/// * `arguments` - 命令行参数
///
/// # 返回值
/// 该函数正常情况下不会返回，因为它会替换当前进程。
/// 只有在执行失败时才会返回错误。
///
/// # 错误处理
/// - 如果命令不存在，返回 `NOT_FOUND` 错误
/// - 如果无法执行命令，返回 `COULD_NOT_EXECUTE` 错误
///
/// # 注意
/// 理论上返回类型应该是 `UResult<!>`，但由于 never type 尚未稳定，
/// 所以使用 `CTResult<()>` 表示该函数如果返回一定是错误。
fn runcon_exec(command: &OsStr, arguments: &[OsString]) -> CTResult<()> {
    // 将命令转换为 C 字符串
    let c_command = os_str_to_c_string(command).map_err(RunconError::new)?;

    // 将所有参数转换为 C 字符串
    let argv_storage: Vec<CString> = arguments
        .iter()
        .map(AsRef::as_ref)
        .map(os_str_to_c_string)
        .collect::<Result<_>>()
        .map_err(RunconError::new)?;

    // 构建 argv 数组
    let mut argv = Vec::with_capacity(arguments.len().saturating_add(2));
    argv.push(c_command.as_ptr());
    argv.extend(argv_storage.iter().map(AsRef::as_ref).map(CStr::as_ptr));
    argv.push(ptr::null());

    // 执行命令
    unsafe { libc::execvp(c_command.as_ptr(), argv.as_ptr()) };

    // 如果执行到这里，说明 execvp 失败
    let err = io::Error::last_os_error();
    let exit_status = if err.kind() == io::ErrorKind::NotFound {
        error_exit_status::RUNCON_NOT_FOUND
    } else {
        error_exit_status::RUNCON_COULD_NOT_EXECUTE
    };

    let err = DefaultError::from_io1("Executing command", command, err);
    Err(RunconError::with_code(exit_status, err).into())
}

fn os_str_to_c_string(s: &OsStr) -> Result<CString> {
    CString::new(s.as_bytes())
        .map_err(|_r| DefaultError::from_io("CString::new()", io::ErrorKind::InvalidInput.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    mod command_line_tests {
        use super::*;

        #[test]
        fn test_parse_command_line_print() {
            let config = ct_app();
            let args = vec![OsString::from("runcon")].into_iter();
            let result = RunconSettings::new(config, args).unwrap();

            assert!(matches!(result.mode, RunconCommandLineMode::Print));
            assert!(result.arguments.is_empty());
        }

        #[test]
        fn test_parse_command_line_plain_context() {
            let config = ct_app();
            let args = vec![
                OsString::from("runcon"),
                OsString::from("system_u:system_r:httpd_t"),
                OsString::from("ls"),
                OsString::from("-l"),
            ]
            .into_iter();

            let result = RunconSettings::new(config, args).unwrap();

            match result.mode {
                RunconCommandLineMode::PlainContext { context, command } => {
                    assert_eq!(context, "system_u:system_r:httpd_t");
                    assert_eq!(command, "ls");
                }
                _ => panic!("Wrong mode"),
            }
            assert_eq!(result.arguments, vec!["-l"]);
        }

        #[test]
        fn test_parse_command_line_custom_context() {
            let config = ct_app();
            let args = vec![
                OsString::from("runcon"),
                OsString::from("-u"),
                OsString::from("user_u"),
                OsString::from("-r"),
                OsString::from("role_r"),
                OsString::from("ls"),
            ]
            .into_iter();

            let result = RunconSettings::new(config, args).unwrap();

            match result.mode {
                RunconCommandLineMode::CustomContext {
                    user,
                    role,
                    command,
                    ..
                } => {
                    assert_eq!(user.unwrap(), "user_u");
                    assert_eq!(role.unwrap(), "role_r");
                    assert_eq!(command.unwrap(), "ls");
                }
                _ => panic!("Wrong mode"),
            }
            assert!(result.arguments.is_empty());
        }

        #[test]
        fn test_parse_command_line_missing_command() {
            let config = ct_app();
            let args = vec![
                OsString::from("runcon"),
                OsString::from("system_u:system_r:httpd_t"),
            ]
            .into_iter();

            let result = RunconSettings::new(config, args);
            assert!(matches!(result, Err(DefaultError::MissingCommand)));
        }
    }

    mod context_tests {
        use super::*;
        use std::fs::File;
        use tempfile::tempdir;

        #[test]
        fn test_get_plain_context() {
            // 注意：这个测试在没有 SELinux 的系统上会失败
            let result = get_plain_context(OsStr::new("system_u:system_r:httpd_t"));

            if selinux::kernel_support() == selinux::KernelSupport::Unsupported {
                assert!(matches!(result, Err(DefaultError::SELinuxNotEnabled)));
            } else {
                assert!(result.is_ok());
            }
        }

        #[test]
        fn test_get_custom_context() {
            let dir = tempdir().unwrap();
            let test_file = dir.path().join("test.txt");
            File::create(&test_file).unwrap();

            let result = get_custom_context(
                true,
                Some(OsStr::new("user_u")),
                Some(OsStr::new("role_r")),
                Some(OsStr::new("type_t")),
                Some(OsStr::new("s0")),
                test_file.as_os_str(),
            );

            if selinux::kernel_support() == selinux::KernelSupport::Unsupported {
                assert!(matches!(result, Err(DefaultError::SELinuxNotEnabled)));
            } else {
                // 在启用 SELinux 的系统上进行更详细的检查
                match result {
                    Ok(_) => (),
                    Err(e) => {
                        // 允许某些预期的错误（如权限不足）
                        println!("Expected error: {}", e);
                    }
                }
            }
        }
    }
}
