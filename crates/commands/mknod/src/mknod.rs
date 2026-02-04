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

// mknod 命令在类 Unix 系统（如 Linux）中用于创建特殊的文件类型，如字符设备（character device）、块设备（block device）和命名管道（named pipe，也称为 FIFO）。
// 这些文件类型在操作系统中用于与硬件交互或实现进程间的通信。
// 主要功能：实现类Unix系统调用mknod的功能，用于创建特殊文件：块设备、字符设备、FIFO。
// 支持的平台：非Windows的Unix平台。
// 使用的库：clap用于命令行参数解析，libc提供Unix系统调用的接口。

extern crate rust_i18n;
use clap::{Arg, ArgMatches, Command, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use clap::ArgAction;
use clap::builder::ValueParser;
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CTsageError, CtSimpleError, set_ct_exit_code};
use libc::{S_IFBLK, S_IFCHR, S_IFIFO, S_IRGRP, S_IROTH, S_IRUSR, S_IWGRP, S_IWOTH, S_IWUSR};
use libc::{dev_t, mode_t};
use selinux::label::{Labeler, back_end::File as FileBackEnd};
use selinux::{self, SecurityContext};
use std::ffi::{CString, OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use sys_locale::get_locale;

// 常量：用于设置文件模式的权限位。
const MODE_RW_UGO: mode_t = S_IRUSR | S_IWUSR | S_IRGRP | S_IWGRP | S_IROTH | S_IWOTH;

// 函数：解析设备号，支持十进制、十六进制和八进制格式
fn parse_device_number(s: &str) -> Result<u64, String> {
    if s.starts_with("0x") || s.starts_with("0X") {
        // 十六进制格式
        u64::from_str_radix(&s[2..], 16)
            .map_err(|e| format!("invalid hexadecimal device number: {}", e))
    } else if let Some(stripped) = s.strip_prefix('0') {
        // 八进制格式
        u64::from_str_radix(stripped, 8).map_err(|e| format!("invalid octal device number: {}", e))
    } else {
        // 十进制格式
        s.parse::<u64>()
            .map_err(|e| format!("invalid decimal device number: {}", e))
    }
}

// 函数：根据给定的主、次设备号构造一个dev_t值。
#[inline(always)]
fn make_dev(maj: u64, min: u64) -> dev_t {
    // 根据设备号构造dev_t，细节来自<sys/sysmacros.h>
    ((min & 0xff) | ((maj & 0xfff) << 8) | ((min & !0xff) << 12) | ((maj & !0xfff) << 32)) as dev_t
}

// Windows平台不支持mknod，因此定义一个空实现。
#[cfg(windows)]
fn _mknod(file_name: &str, mode: mode_t, dev: dev_t) -> i32 {
    panic!("Unsupported for windows platform")
}

// 枚举：文件类型，包括块设备、字符设备和FIFO。
#[derive(Clone, PartialEq, Debug)]
enum MknodFileType {
    Block,
    Character,
    Fifo,
}

// Unix平台的_mknod实现。
#[cfg(unix)]
fn _mknod(file_name: &str, mode: mode_t, dev: dev_t) -> i32 {
    let c_str = CString::new(file_name).expect("Failed to convert to CString");

    // 检查是否设置了文件模式的权限位
    let set_umask = mode & MODE_RW_UGO != MODE_RW_UGO;

    unsafe {
        // 保存当前的umask，如果需要的话
        let last_umask = if set_umask { libc::umask(0) } else { 0 };

        // 执行mknod系统调用
        let errno = libc::mknod(c_str.as_ptr(), mode, dev);

        // 如果设置了umask，将其恢复到原始值
        if set_umask {
            libc::umask(last_umask);
        }

        // 处理系统调用失败的情况
        if errno == -1 {
            let c_str = CString::new(ctcore::ct_execute_phrase().as_bytes())
                .expect("Failed to convert to CString");
            libc::perror(c_str.as_ptr());
        }
        errno
    }
}

fn set_security_context(
    context: Option<&OsString>,
    file_path: &str,
    file_mode: mode_t,
) -> Result<(), String> {
    // 首先检查SELinux是否启用
    if selinux::kernel_support() == selinux::KernelSupport::Unsupported {
        // SELinux未启用，如果用户明确指定了上下文，发出警告
        if context.is_some() {
            eprintln!("mknod: warning: ignoring --context; it requires an SELinux-enabled kernel");
        }
        return Ok(());
    }

    match context {
        Some(ctx) => {
            let c_context = os_str_to_c_string(ctx);
            // 如果提供了具体的上下文，使用它
            SecurityContext::from_c_str(&c_context, false)
                .set_for_new_file_system_objects(false)
                .map_err(|e| format!("failed to set default file creation context: {}", e))
        }
        None => {
            // 使用selabel_lookup获取基于路径的默认上下文（与GNU的defaultcon()函数等价）
            //
            // GNU实现流程：
            //   1. 使用selabel_lookup(handle, &scon, path, mode)查询策略数据库
            //   2. 使用computecon()结合进程上下文和文件策略
            //   3. 提取类型字段并设置最终上下文
            //
            // 当前实现（使用selinux 0.6 crate）：
            //   1. 创建Labeler用于文件上下文查询
            //   2. 使用look_up_by_path()获取路径的默认上下文（等价于selabel_lookup）
            //   3. 设置为新文件系统对象的创建上下文

            // 创建文件上下文标签器（等价于GNU的selabel_open）
            let labeler = Labeler::<FileBackEnd>::restorecon_default(false).map_err(|e| {
                // 如果无法创建labeler，发出警告但继续（与GNU行为一致）
                eprintln!("mknod: warning: cannot create SELinux labeler: {}", e);
                String::new()
            })?;

            // 将mode_t转换为FileAccessMode
            let file_access_mode = mode_to_file_access_mode(file_mode);

            // 查询路径的默认SELinux上下文（等价于GNU的selabel_lookup）
            let path = Path::new(file_path);
            let default_context = labeler
                .look_up_by_path(path, Some(file_access_mode))
                .map_err(|e| {
                    // 如果查询失败，发出警告但继续（与GNU行为一致）
                    // GNU会在ENOENT时映射为ENODATA
                    eprintln!(
                        "mknod: warning: cannot look up default SELinux context for {}: {}",
                        file_path, e
                    );
                    String::new()
                })?;

            // 设置为新文件系统对象的创建上下文（等价于GNU的setfscreatecon）
            default_context
                .set_for_new_file_system_objects(false)
                .map_err(|e| {
                    // 如果设置失败，发出警告但继续（与GNU行为一致）
                    eprintln!(
                        "mknod: warning: cannot set default file creation context: {}",
                        e
                    );
                    String::new()
                })?;

            Ok(())
        }
    }
}

// 将mode_t转换为selinux::FileAccessMode
// FileAccessMode是一个包装mode_t的结构体，直接传递mode即可
fn mode_to_file_access_mode(mode: mode_t) -> selinux::FileAccessMode {
    // FileAccessMode::new()接受mode_t并返回Option<FileAccessMode>
    // 如果mode为0则返回None，但在mknod中mode总是非零的（包含文件类型位）
    selinux::FileAccessMode::new(mode).expect("mode should be non-zero in mknod context")
}

pub fn os_str_to_c_string(os_str: &OsStr) -> CString {
    CString::new(os_str.as_bytes()).expect("Failed to convert OsStr to CString")
}

pub fn mknod_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    // 从命令行参数中解析选项和参数
    let args_match = ct_app().try_get_matches_from(args)?;

    let mode = get_mode(&args_match).map_err(|e| CtSimpleError::new(1, e))?;

    let file_name = args_match
        .get_one::<String>("name")
        .expect("Missing argument 'NAME'");

    let file_type = args_match.get_one::<MknodFileType>("type").unwrap();

    // 根据文件类型执行不同的操作
    mknod_processing(&args_match, mode, file_name, file_type)
}

fn mknod_processing(
    args_match: &ArgMatches,
    mode: mode_t,
    file_name: &str,
    file_type: &MknodFileType,
) -> CTResult<()> {
    // 处理安全上下文（仅当用户明确指定-Z或--context时）
    // 注意：只有当用户实际提供了这些标志时才设置SELinux上下文
    let has_z_flag = args_match.get_flag("ctx");
    let has_context_flag =
        args_match.value_source("context") == Some(clap::parser::ValueSource::CommandLine);

    if has_z_flag || has_context_flag {
        let context = args_match.get_one::<OsString>("context");
        set_security_context(context, file_name, mode).map_err(|e| CtSimpleError::new(1, e))?;
    }
    // 如果既没有-Z也没有--context，则不设置SELinux上下文

    let result = if *file_type == MknodFileType::Fifo {
        // FIFO文件不需要主、次设备号
        if args_match.contains_id("major") || args_match.contains_id("minor") {
            Err(CTsageError::new(
                1,
                "Fifos do not have major and minor device numbers.",
            ))
        } else {
            let exit_code = _mknod(file_name, S_IFIFO | mode, 0);
            set_ct_exit_code(exit_code);
            if exit_code == 0 {
                Ok(())
            } else {
                Err(CtSimpleError::new(
                    1,
                    format!("failed to create FIFO: {}", file_name),
                ))
            }
        }
    } else {
        // 对于块设备和字符设备，需要主、次设备号
        match (
            args_match.get_one::<u64>("major"),
            args_match.get_one::<u64>("minor"),
        ) {
            (_, None) | (None, _) => Err(CTsageError::new(
                1,
                "Special files require major and minor device numbers.",
            )),
            (Some(&major), Some(&minor)) => {
                // 检查设备号是否有效（NODEV检查）
                #[cfg(target_os = "linux")]
                {
                    const NODEV: dev_t = !0;
                    let dev = make_dev(major, minor);
                    if dev == NODEV {
                        return Err(CtSimpleError::new(
                            1,
                            format!("invalid device {} {}", major, minor),
                        ));
                    }
                }

                let dev = make_dev(major, minor);
                let exit_code = match file_type {
                    MknodFileType::Block => _mknod(file_name, S_IFBLK | mode, dev),
                    MknodFileType::Character => _mknod(file_name, S_IFCHR | mode, dev),
                    _ => unreachable!("file_type was validated to be only block or character"),
                };
                set_ct_exit_code(exit_code);
                if exit_code == 0 {
                    Ok(())
                } else {
                    Err(CtSimpleError::new(
                        1,
                        format!("failed to create device: {}", file_name),
                    ))
                }
            }
        }
    };

    // 如果文件创建成功且用户指定了模式，使用chmod确保权限正确设置
    // 这与GNU的lchmod()调用等价（对于非符号链接文件）
    if result.is_ok() && args_match.contains_id("mode") {
        unsafe {
            let c_path = CString::new(file_name).expect("Failed to convert path to CString");
            if libc::chmod(c_path.as_ptr(), mode) != 0 {
                let err = std::io::Error::last_os_error();
                return Err(CtSimpleError::new(
                    1,
                    format!("cannot set permissions of {}: {}", file_name.quote(), err),
                ));
            }
        }
    }

    result
}

// 构建命令行解析器。
pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("mknod.about");
    let usage_description = t!("mknod.usage");
    let after_info = t!("mknod.after_help");

    let args = vec![
        Arg::new("help")
            .short('h')
            .long("help")
            .help(t!("mknod.clap.help"))
            .action(ArgAction::Help),
        Arg::new("version")
            .short('V')
            .long("version")
            .help(t!("mknod.clap.version"))
            .action(ArgAction::Version),
        Arg::new("mode")
            .short('m')
            .long("mode")
            .value_name("MODE")
            .help(t!("mknod.clap.mode")),
        Arg::new("ctx")
            .short('Z')
            .action(ArgAction::SetTrue)
            .help("set the default SELinux security context"),
        Arg::new("context")
            .long("context")
            .value_name("CTX")
            .help("if CTX is specified then set the SELinux security context to CTX")
            .value_parser(ValueParser::os_string())
            .num_args(0..=1)
            .require_equals(true)
            .default_missing_value(""),
        Arg::new("name")
            .value_name("NAME")
            .help(t!("mknod.clap.name"))
            .required(true)
            .value_hint(clap::ValueHint::AnyPath),
        Arg::new("type")
            .value_name("TYPE")
            .help(t!("mknod.clap.type"))
            .required(true)
            .value_parser(parse_type),
        Arg::new("major")
            .value_name("MAJOR")
            .help(t!("mknod.clap.major"))
            .value_parser(parse_device_number),
        Arg::new("minor")
            .value_name("MINOR")
            .help(t!("mknod.clap.minor"))
            .value_parser(parse_device_number),
    ];

    Command::new(utility_name)
        .version(command_version)
        .override_usage(usage_description)
        .after_help(after_info)
        .about(application_info)
        .infer_long_args(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .args(&args)
}

// 解析模式参数。
fn get_mode(matches: &ArgMatches) -> Result<mode_t, String> {
    match matches.get_one::<String>("mode") {
        None => Ok(MODE_RW_UGO),
        Some(str_mode) => ctcore::ct_mode::parse_mode(str_mode)
            .map_err(|e| format!("invalid mode ({e})"))
            .and_then(|mode| {
                if mode > 0o777 {
                    Err("mode must specify only file permission bits".to_string())
                } else {
                    Ok(mode)
                }
            }),
    }
}

// 解析文件类型参数。
fn parse_type(tpe: &str) -> Result<MknodFileType, String> {
    // 仅根据第一个字符解析文件类型
    tpe.chars()
        .next()
        .ok_or_else(|| "missing device type".to_string())
        .and_then(|first_char| match first_char {
            'b' => Ok(MknodFileType::Block),
            'c' | 'u' => Ok(MknodFileType::Character),
            'p' => Ok(MknodFileType::Fifo),
            _ => Err(format!("invalid device type {}", tpe.quote())),
        })
}

#[derive(Default)]
pub struct Mknod;
impl Tool for Mknod {
    fn name(&self) -> &'static str {
        "mknod"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        mknod_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Mknod::default();

        // 测试 name 方法
        assert_eq!(tool.name(), "mknod");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("mknod"));

        // 测试 execute 方法 - 帮助命令应该返回错误，但不会崩溃
        let args = vec![OsString::from("mknod"), OsString::from("--help")];
        assert!(tool.execute(&args).is_err());
    }

    mod tests_mknod_main {
        use crate::{ct_app, mknod_main};

        use std::ffi::OsString;

        #[test]
        fn test_mknod_main_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];

            let result = mknod_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_mknod_main_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = mknod_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_mknod_main_b() {
            let args = vec![ctcore::ct_util_name(), "file", "b", "8", "1"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_mknod_main_c() {
            let args = vec![ctcore::ct_util_name(), "file", "c", "1", "100"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_mknod_main_p() {
            let args = vec![ctcore::ct_util_name(), "file", "p"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }
    }

    mod tests_mknod_app {
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

        #[test]
        fn test_ct_app_b() {
            let args = vec![ctcore::ct_util_name(), "file", "b", "8", "1"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_c() {
            let args = vec![ctcore::ct_util_name(), "file", "c", "1", "100"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_p() {
            let args = vec![ctcore::ct_util_name(), "file", "p"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }
    }
}
