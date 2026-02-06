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

// mkfifo 是一个 Linux 和类 Unix 系统中的命令，它用于创建一个命名管道（named pipe），也称为FIFO（First In First Out）。
// 命名管道是一种特殊类型的文件，允许不同的进程之间通过它进行数据通信，而无需事先知道对方的进程ID。
// 与无名管道（匿名管道）不同，命名管道可以在进程之间持久存在，即使创建它的进程已经结束。
//
// 命名管道的使用通常涉及到两个或多个进程，其中一个进程将数据写入管道，另一个或多个进程从管道中读取数据。它们在文件系统中有一个名称，因此可以被多个进程引用。

extern crate rust_i18n;
use clap::{Arg, ArgAction, Command, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use clap::builder::ValueParser;
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CtSimpleError};
use ctcore::ct_show;
use libc::mkfifo;
use selinux::SecurityContext;
use selinux::label::{Labeler, back_end::File as FileBackEnd};
use std::ffi::{CString, OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use sys_locale::get_locale;

// 定义了用于创建FIFO（命名管道）的命令行工具的主逻辑。

// 用于命令行选项的常量模块
mod opt_flags {
    pub const MODE: &str = "mode"; // 文件权限模式选项
    pub const SE_LINUX_SECURITY_CONTEXT: &str = "Z"; // 设置SELinux安全上下文选项
    pub const CONTEXT: &str = "context"; // 安全上下文选项
    pub const FIFO: &str = "fifo"; // FIFO路径参数
}

pub fn mkfifo_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let args_match = ct_app().try_get_matches_from(args)?;

    // 处理安全上下文(仅当用户明确指定-Z或--context时)
    let has_z_flag = args_match.get_flag(opt_flags::SE_LINUX_SECURITY_CONTEXT);
    let has_context_flag =
        args_match.value_source(opt_flags::CONTEXT) == Some(clap::parser::ValueSource::CommandLine);
    let context = args_match
        .get_one::<OsString>(opt_flags::CONTEXT)
        .filter(|v| !v.is_empty());
    let set_context = has_z_flag || has_context_flag;
    let warn_on_unsupported = context.is_some();

    if set_context && context.is_some() {
        set_security_context_for_all(context).map_err(|e| CtSimpleError::new(1, e))?;
    }

    // 解析文件权限模式
    let fifo_mode = match args_match.get_one::<String>(opt_flags::MODE) {
        Some(mode_str) => {
            let mode =
                parse_mkfifo_mode(mode_str).map_err(|_| CtSimpleError::new(1, "invalid mode"))?;
            if mode > 0o777 {
                return Err(CtSimpleError::new(
                    1,
                    "mode must specify only file permission bits".to_string(),
                ));
            }
            mode as libc::mode_t
        }
        None => 0o666,
    };

    // 解析FIFO路径列表
    let fifo_strs: Vec<String> = match args_match.get_many::<String>(opt_flags::FIFO) {
        Some(v) => v.cloned().collect(),
        None => {
            let err_message = format!(
                "missing operand\nTry '{} --help' for more information.",
                ctcore::ct_util_name()
            );
            return Err(CtSimpleError::new(1, err_message));
        }
    };

    // 创建FIFO
    let mut has_error = false;
    let specified_mode =
        args_match.value_source(opt_flags::MODE) == Some(clap::parser::ValueSource::CommandLine);

    for fifo in fifo_strs {
        if set_context && context.is_none() {
            set_default_context_for_path(&fifo, fifo_mode as libc::mode_t, warn_on_unsupported)
                .map_err(|e| CtSimpleError::new(1, e))?;
        }

        let result = unsafe {
            let fifo_name = CString::new(fifo.as_bytes()).unwrap();
            mkfifo(fifo_name.as_ptr(), fifo_mode)
        };

        if result == -1 {
            let err = std::io::Error::last_os_error();
            ct_show!(CtSimpleError::new(
                1,
                format!("cannot create fifo {}: {}", fifo.quote(), err)
            ));
            has_error = true;
        } else if specified_mode {
            // 如果指定了模式,使用lchmod确保权限正确设置(匹配GNU的lchmod行为)
            unsafe {
                let c_path =
                    CString::new(fifo.as_bytes()).expect("Failed to convert path to CString");
                #[cfg(target_os = "linux")]
                let chmod_result = {
                    // Linux 没有 lchmod,但对于 FIFO 使用 chmod 是安全的
                    libc::chmod(c_path.as_ptr(), fifo_mode)
                };
                #[cfg(not(target_os = "linux"))]
                let chmod_result = libc::lchmod(c_path.as_ptr(), fifo_mode);

                if chmod_result != 0 {
                    let err = std::io::Error::last_os_error();
                    ct_show!(CtSimpleError::new(
                        1,
                        format!("cannot set permissions of {}: {}", fifo.quote(), err)
                    ));
                    has_error = true;
                }
            }
        }
    }

    if has_error {
        Err(CtSimpleError::new(1, String::new()))
    } else {
        Ok(())
    }
}

fn set_security_context_for_all(context: Option<&OsString>) -> Result<(), String> {
    // 首先检查SELinux是否启用
    if selinux::kernel_support() == selinux::KernelSupport::Unsupported {
        // SELinux未启用,如果用户明确指定了上下文,发出警告
        if context.is_some() {
            eprintln!(
                "mkfifo: warning: ignoring --context; it requires an SELinux/SMACK-enabled kernel"
            );
        }
        return Ok(());
    }

    match context {
        Some(ctx) => {
            let c_context = os_str_to_c_string(ctx);
            // 如果提供了具体的上下文,使用它
            SecurityContext::from_c_str(&c_context, false)
                .set_for_new_file_system_objects(false)
                .map_err(|e| format!("failed to set default file creation context: {e}"))
        }
        None => Ok(()),
    }
}

fn set_default_context_for_path(
    fifo_path: &str,
    fifo_mode: libc::mode_t,
    warn_on_unsupported: bool,
) -> Result<(), String> {
    if selinux::kernel_support() == selinux::KernelSupport::Unsupported {
        if warn_on_unsupported {
            eprintln!(
                "mkfifo: warning: ignoring --context; it requires an SELinux/SMACK-enabled kernel"
            );
        }
        return Ok(());
    }

    let labeler = match Labeler::<FileBackEnd>::restorecon_default(false) {
        Ok(labeler) => labeler,
        Err(_) => return Ok(()),
    };

    let file_mode = (libc::S_IFIFO as libc::mode_t) | fifo_mode;
    let file_access_mode =
        selinux::FileAccessMode::new(file_mode).expect("mode should be non-zero");
    let default_context = match labeler.look_up_by_path(fifo_path, Some(file_access_mode)) {
        Ok(context) => context,
        Err(_) => return Ok(()),
    };

    let _ = default_context.set_for_new_file_system_objects(false);
    Ok(())
}
pub fn os_str_to_c_string(os_str: &OsStr) -> CString {
    CString::new(os_str.as_bytes()).expect("Failed to convert OsStr to CString")
}

fn parse_mkfifo_mode(mode: &str) -> Result<libc::mode_t, String> {
    if mode.contains(',') && !mode.chars().any(|c| c.is_ascii_digit()) {
        let mut current: u32 = libc::S_IRUSR
            | libc::S_IWUSR
            | libc::S_IRGRP
            | libc::S_IWGRP
            | libc::S_IROTH
            | libc::S_IWOTH;
        let umask = ctcore::ct_mode::get_umask();
        for part in mode.split(',') {
            if part.is_empty() {
                return Err("invalid mode".to_string());
            }
            current = ctcore::ct_mode::parse_symbolic(current, part, umask, true)?;
        }
        return Ok(current as libc::mode_t);
    }

    ctcore::ct_mode::parse_mode(mode).map(|m| m as libc::mode_t)
}
// 构建命令行解析器
pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("mkfifo.about");
    let usage_description = t!("mkfifo.usage");
    let args = vec![
        Arg::new("help")
            .short('h')
            .long("help")
            .help(t!("mkfifo.clap.help"))
            .action(ArgAction::Help),
        Arg::new("version")
            .short('V')
            .long("version")
            .help(t!("mkfifo.clap.version"))
            .action(ArgAction::Version),
        Arg::new(opt_flags::MODE)
            .short('m')
            .long(opt_flags::MODE)
            .help(t!("mkfifo.clap.mode"))
            .value_name("MODE"),
        Arg::new(opt_flags::SE_LINUX_SECURITY_CONTEXT)
            .short('Z')
            .help(t!("mkfifo.clap.se_linux_security_context"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::CONTEXT)
            .long(opt_flags::CONTEXT)
            .value_name("CTX")
            .value_parser(ValueParser::os_string())
            .num_args(0..=1)
            .require_equals(true)
            .default_missing_value("")
            .help(
                "like -Z, or if CTX is specified then set the SELinux \
                    or SMACK security context to CTX",
            ),
        Arg::new(opt_flags::FIFO)
            .hide(true) // 隐藏此参数
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::AnyPath), // 提示参数类型为路径
    ];

    Command::new(utility_name)
        .version(command_version)
        .override_usage(usage_description)
        .about(application_info)
        .infer_long_args(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .args(&args)
}

#[derive(Default)]
pub struct Mkfifo;
impl Tool for Mkfifo {
    fn name(&self) -> &'static str {
        "mkfifo"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        // 将&[OsString]转换为符合Args trait要求的iterator
        mkfifo_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Mkfifo;

        // 测试 name 方法
        assert_eq!(tool.name(), "mkfifo");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("mkfifo"));

        // 测试 execute 方法
        let args = vec![OsString::from("mkfifo"), OsString::from("--help")];
        let result = tool.execute(&args);
        assert!(result.is_err()); // mkfifo命令需要参数，所以不带参数应该返回错误
    }

    mod tests_mkfio_main {
        use crate::mkfifo_main;

        use std::ffi::OsString;

        #[test]
        fn test_false_main_version() {
            let args = [ctcore::ct_util_name(), "--version"];

            let result = mkfifo_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_false_main_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = mkfifo_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }
    }

    mod tests_mkfio_app {
        use crate::ct_app;

        use clap::error::ErrorKind;

        #[test]
        fn test_ct_app_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }
    }
}
