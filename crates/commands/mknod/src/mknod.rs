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
use ctcore::ct_error::{CTResult, CTsageError, CtSimpleError, strip_errno};
use ctcore::ct_quoting_style::{CtQuotingStyle, escape_name_with_chars};
use libc::{S_IFBLK, S_IFCHR, S_IFIFO, S_IRGRP, S_IROTH, S_IRUSR, S_IWGRP, S_IWOTH, S_IWUSR};
use libc::{dev_t, mode_t};
use selinux::label::{Labeler, back_end::File as FileBackEnd};
use selinux::{self, SecurityContext};
use std::ffi::{CString, OsStr, OsString};
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::Path;
use sys_locale::get_locale;

// 常量：用于设置文件模式的权限位。
const MODE_RW_UGO: mode_t = S_IRUSR | S_IWUSR | S_IRGRP | S_IWGRP | S_IROTH | S_IWOTH;
const MAX_DEVICE_COMPONENT: u64 = u32::MAX as u64;

const MKNOD_DIAGNOSTIC_QUOTING_STYLE: CtQuotingStyle = CtQuotingStyle::Shell {
    escape: true,
    always_quote: false,
    show_control: true,
};

const GNU_MKNOD_LONG_OPTIONS: &[&str] = &["context", "mode", "help", "version"];

fn validate_gnu_mknod_options(args: &[OsString]) -> CTResult<()> {
    let mut index = 1;
    while index < args.len() {
        let argument = args[index].as_os_str().as_bytes();
        if argument == b"--" {
            break;
        }

        if let Some(long_argument) = argument.strip_prefix(b"--") {
            if long_argument.is_empty() {
                break;
            }

            let option_end = long_argument
                .iter()
                .position(|byte| *byte == b'=')
                .unwrap_or(long_argument.len());
            let option_name = String::from_utf8_lossy(&long_argument[..option_end]);
            let supplied_value = option_end < long_argument.len();
            let mut candidates = GNU_MKNOD_LONG_OPTIONS
                .iter()
                .copied()
                .filter(|candidate| candidate.starts_with(option_name.as_ref()))
                .collect::<Vec<_>>();

            if let Some(exact) = candidates
                .iter()
                .position(|candidate| *candidate == option_name)
            {
                candidates = vec![candidates[exact]];
            }

            if candidates.is_empty() {
                return Err(CTsageError::new(
                    1,
                    format!(
                        "unrecognized option '{}'",
                        args[index].as_os_str().to_string_lossy()
                    ),
                ));
            }

            if candidates.len() > 1 {
                let possibilities = candidates
                    .iter()
                    .map(|candidate| format!("'--{candidate}'"))
                    .collect::<Vec<_>>()
                    .join(" ");
                return Err(CTsageError::new(
                    1,
                    format!(
                        "option '{}' is ambiguous; possibilities: {possibilities}",
                        args[index].as_os_str().to_string_lossy()
                    ),
                ));
            }

            match candidates[0] {
                "mode" if !supplied_value => {
                    index += 1;
                    if index == args.len() {
                        return Err(CTsageError::new(1, "option '--mode' requires an argument"));
                    }
                }
                "context" => {}
                option if supplied_value => {
                    return Err(CTsageError::new(
                        1,
                        format!("option '--{option}' doesn't allow an argument"),
                    ));
                }
                "help" | "version" => return Ok(()),
                _ => {}
            }
        } else if argument.starts_with(b"-") && argument.len() > 1 {
            let mut short_index = 1;
            while short_index < argument.len() {
                let option = argument[short_index];
                if option == b'm' {
                    if short_index + 1 == argument.len() {
                        index += 1;
                        if index == args.len() {
                            return Err(CTsageError::new(1, "option requires an argument -- 'm'"));
                        }
                    }
                    break;
                }

                // -h and -V are retained as syskits extensions.
                if !matches!(option, b'Z' | b'h' | b'V') {
                    return Err(CTsageError::new(
                        1,
                        format!("invalid option -- '{}'", char::from(option)),
                    ));
                }
                if matches!(option, b'h' | b'V') {
                    return Ok(());
                }
                short_index += 1;
            }
        }

        index += 1;
    }

    Ok(())
}

fn apply_posix_option_order(args: &mut Vec<OsString>, posixly_correct: bool) {
    if !posixly_correct {
        return;
    }

    let mut index = 1;
    while index < args.len() {
        let argument = args[index].as_os_str().as_bytes();
        if argument == b"--" {
            return;
        }
        if argument == b"-" || !argument.starts_with(b"-") {
            args.insert(index, OsString::from("--"));
            return;
        }

        if let Some(long_argument) = argument.strip_prefix(b"--") {
            let option = long_argument
                .split(|byte| *byte == b'=')
                .next()
                .unwrap_or_default();
            if !option.is_empty() && b"mode".starts_with(option) && !long_argument.contains(&b'=') {
                index += 2;
                continue;
            }
        } else if let Some(short_options) = argument.strip_prefix(b"-")
            && let Some(mode_index) = short_options.iter().position(|byte| *byte == b'm')
            && mode_index + 1 == short_options.len()
        {
            index += 2;
            continue;
        }

        index += 1;
    }
}

fn validate_gnu_mknod_operands(args: &[OsString]) -> CTResult<()> {
    let mut operands = Vec::new();
    let mut options_enabled = true;
    let mut index = 1;

    while index < args.len() {
        let argument = args[index].as_os_str().as_bytes();
        if options_enabled && argument == b"--" {
            options_enabled = false;
            index += 1;
            continue;
        }

        if options_enabled {
            if let Some(long_argument) = argument.strip_prefix(b"--") {
                let option_end = long_argument
                    .iter()
                    .position(|byte| *byte == b'=')
                    .unwrap_or(long_argument.len());
                let option = &long_argument[..option_end];
                if b"help".starts_with(option) || b"version".starts_with(option) {
                    return Ok(());
                }
                if b"mode".starts_with(option) && option_end == long_argument.len() {
                    index += 2;
                    continue;
                }
                index += 1;
                continue;
            }

            if let Some(short_options) = argument.strip_prefix(b"-")
                && !short_options.is_empty()
            {
                if short_options
                    .iter()
                    .any(|option| matches!(option, b'h' | b'V'))
                {
                    return Ok(());
                }
                if let Some(mode_index) = short_options.iter().position(|byte| *byte == b'm')
                    && mode_index + 1 == short_options.len()
                {
                    index += 2;
                    continue;
                }
                index += 1;
                continue;
            }
        }

        operands.push(&args[index]);
        index += 1;
    }

    let expected_operands = if operands.get(1).is_some_and(|operand| {
        operand
            .as_os_str()
            .as_bytes()
            .first()
            .is_some_and(|byte| *byte == b'p')
    }) {
        2
    } else {
        4
    };

    if operands.len() < expected_operands {
        let mut message = match operands.last() {
            Some(operand) => format!("missing operand after {}", operand.as_os_str().quote()),
            None => "missing operand".to_string(),
        };
        if expected_operands == 4 && operands.len() == 2 {
            message.push_str("\nSpecial files require major and minor device numbers.");
        }
        return Err(CTsageError::new(1, message));
    }

    if operands.len() > expected_operands {
        let mut message = format!(
            "extra operand {}",
            operands[expected_operands].as_os_str().quote()
        );
        if expected_operands == 2 && operands.len() == 4 {
            message.push_str("\nFifos do not have major and minor device numbers.");
        }
        return Err(CTsageError::new(1, message));
    }

    let type_bytes = operands[1].as_os_str().as_bytes();
    if !matches!(type_bytes.first(), Some(b'b' | b'c' | b'u' | b'p')) {
        return Err(CTsageError::new(
            1,
            format!("invalid device type {}", operands[1].as_os_str().quote()),
        ));
    }

    Ok(())
}

// 函数：解析设备号，支持十进制、十六进制和八进制格式
fn parse_device_number(s: &str) -> Result<u64, String> {
    let s = s.trim_start_matches(|character: char| {
        matches!(character, ' ' | '\t' | '\n' | '\r' | '\u{b}' | '\u{c}')
    });
    if s.starts_with("0x") || s.starts_with("0X") {
        // 十六进制格式
        u64::from_str_radix(&s[2..], 16)
            .map_err(|e| format!("invalid hexadecimal device number: {e}"))
    } else if let Some(stripped) = s.strip_prefix('0') {
        // 八进制格式
        if stripped.is_empty() {
            Ok(0)
        } else {
            u64::from_str_radix(stripped, 8)
                .map_err(|e| format!("invalid octal device number: {e}"))
        }
    } else {
        // 十进制格式
        s.parse::<u64>()
            .map_err(|e| format!("invalid decimal device number: {e}"))
    }
}

fn parse_device_component(raw: &str, component: &str) -> CTResult<u64> {
    parse_device_number(raw).map_err(|_| {
        CtSimpleError::new(
            1,
            format!("invalid {component} device number {}", raw.quote()),
        )
    })
}

// 函数：根据给定的主、次设备号构造一个dev_t值。
#[inline(always)]
fn make_dev(maj: u64, min: u64) -> dev_t {
    // 根据设备号构造dev_t，细节来自<sys/sysmacros.h>
    ((min & 0xff) | ((maj & 0xfff) << 8) | ((min & !0xff) << 12) | ((maj & !0xfff) << 32)) as dev_t
}

fn validate_device_number_range(value: u64, raw: &OsStr, component: &str) -> CTResult<()> {
    if value > MAX_DEVICE_COMPONENT {
        return Err(CtSimpleError::new(
            1,
            format!("invalid {component} device number {}", raw.quote()),
        ));
    }
    Ok(())
}

fn quote_operand(name: &OsStr) -> String {
    if name.is_empty() {
        "''".to_string()
    } else {
        escape_name_with_chars(name, &MKNOD_DIAGNOSTIC_QUOTING_STYLE, ":")
    }
}

fn creation_error_message(file_name: &OsStr, error: &io::Error) -> String {
    format!("{}: {}", quote_operand(file_name), strip_errno(error))
}

// Windows平台不支持mknod，因此定义一个空实现。
#[cfg(windows)]
fn _mknod(file_name: &OsStr, mode: mode_t, dev: dev_t) -> io::Result<()> {
    panic!("Unsupported for windows platform")
}

// 枚举：文件类型，包括块设备、字符设备和FIFO。
#[derive(Clone, PartialEq, Debug)]
enum MknodFileType {
    Block,
    Character,
    Fifo,
}

#[derive(Default)]
struct SecurityContextRequest {
    explicit_context: Option<OsString>,
    use_default: bool,
    explicit_occurrences: usize,
}

impl SecurityContextRequest {
    fn explicit_context(&self) -> Option<&OsString> {
        self.explicit_context.as_ref()
    }

    fn is_set(&self) -> bool {
        self.explicit_context.is_some() || self.use_default
    }
}

// Unix平台的_mknod实现。
#[cfg(unix)]
fn _mknod(file_name: &OsStr, mode: mode_t, dev: dev_t) -> io::Result<()> {
    let c_str = CString::new(file_name.as_bytes()).expect("Failed to convert to CString");

    // 检查是否设置了文件模式的权限位
    let set_umask = mode & MODE_RW_UGO != MODE_RW_UGO;

    unsafe {
        // 保存当前的umask，如果需要的话
        let last_umask = if set_umask { libc::umask(0) } else { 0 };

        // 执行mknod系统调用
        let result = libc::mknod(c_str.as_ptr(), mode, dev);
        let error = (result == -1).then(io::Error::last_os_error);

        // 如果设置了umask，将其恢复到原始值
        if set_umask {
            libc::umask(last_umask);
        }

        error.map_or(Ok(()), Err)
    }
}

fn set_security_context(
    request: &SecurityContextRequest,
    file_path: &OsStr,
    file_mode: mode_t,
) -> Result<(), String> {
    if selinux::kernel_support() == selinux::KernelSupport::Unsupported {
        return Ok(());
    }

    if let Some(ctx) = request.explicit_context() {
        let c_context = os_str_to_c_string(ctx);
        SecurityContext::from_c_str(&c_context, false)
            .set_for_new_file_system_objects(false)
            .map_err(|e| format!("failed to set default file creation context: {e}"))?;
    }

    if !request.use_default {
        return Ok(());
    }

    let labeler = match Labeler::<FileBackEnd>::restorecon_default(false) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("mknod: warning: cannot create SELinux labeler: {e}");
            return Ok(());
        }
    };

    let file_access_mode = mode_to_file_access_mode(file_mode);
    let path = Path::new(file_path);

    // 将相对路径转换为绝对路径，以便 SELinux 策略引擎能正确匹配正则
    let abs_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|p| p.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };

    // 传入 abs_path 而不是 path
    let default_context = match labeler.look_up_by_path(&abs_path, Some(file_access_mode)) {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!(
                "mknod: warning: cannot look up default SELinux context for {}: {e}",
                file_path.to_string_lossy()
            );
            return Ok(());
        }
    };

    if let Err(e) = default_context.set_for_new_file_system_objects(false) {
        eprintln!("mknod: warning: cannot set default file creation context: {e}");
    }

    Ok(())
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

fn parse_security_context_request(args: &[OsString]) -> SecurityContextRequest {
    let mut request = SecurityContextRequest::default();
    let mut index = 1;
    while index < args.len() {
        let argument = args[index].as_os_str().as_bytes();
        if argument == b"--" {
            break;
        }

        if let Some(long_argument) = argument.strip_prefix(b"--") {
            let option_end = long_argument
                .iter()
                .position(|byte| *byte == b'=')
                .unwrap_or(long_argument.len());
            let option = &long_argument[..option_end];
            if !option.is_empty() && b"context".starts_with(option) {
                if option_end < long_argument.len() {
                    request.explicit_context =
                        Some(OsString::from_vec(long_argument[option_end + 1..].to_vec()));
                    request.explicit_occurrences += 1;
                } else {
                    request.use_default = true;
                }
            } else if !option.is_empty()
                && b"mode".starts_with(option)
                && option_end == long_argument.len()
            {
                index += 1;
            }
            index += 1;
            continue;
        }

        if let Some(short_options) = argument.strip_prefix(b"-") {
            let mut short_index = 0;
            while short_index < short_options.len() {
                match short_options[short_index] {
                    b'Z' => request.use_default = true,
                    b'm' => {
                        if short_index + 1 == short_options.len() {
                            index += 1;
                        }
                        break;
                    }
                    _ => {}
                }
                short_index += 1;
            }
        }

        index += 1;
    }

    request
}

fn report_unsupported_context_warnings(request: &SecurityContextRequest) {
    if selinux::kernel_support() != selinux::KernelSupport::Unsupported {
        return;
    }
    for _ in 0..request.explicit_occurrences {
        eprintln!(
            "mknod: warning: ignoring --context; it requires an SELinux/SMACK-enabled kernel"
        );
    }
}

pub fn mknod_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let mut raw_args: Vec<OsString> = args.into_iter().collect();
    apply_posix_option_order(&mut raw_args, std::env::var_os("POSIXLY_CORRECT").is_some());
    validate_gnu_mknod_options(&raw_args)?;
    let context_request = parse_security_context_request(&raw_args);
    report_unsupported_context_warnings(&context_request);
    validate_gnu_mknod_operands(&raw_args)?;
    // 从命令行参数中解析选项和参数
    let args_match = ct_app().try_get_matches_from(raw_args)?;

    let mode = get_mode(&args_match).map_err(|e| CtSimpleError::new(1, e))?;

    let file_name = args_match
        .get_one::<OsString>("name")
        .expect("Missing argument 'NAME'");

    let type_argument = args_match
        .get_one::<OsString>("type")
        .expect("Missing argument 'TYPE'");
    let file_type =
        parse_type(type_argument.as_os_str()).map_err(|error| CtSimpleError::new(1, error))?;

    // 根据文件类型执行不同的操作
    mknod_processing(&args_match, mode, file_name, &file_type, &context_request)
}

fn mknod_processing(
    args_match: &ArgMatches,
    mode: mode_t,
    file_name: &OsStr,
    file_type: &MknodFileType,
    context_request: &SecurityContextRequest,
) -> CTResult<()> {
    // 提前计算包含文件类型的完整 mode
    let full_mode = match file_type {
        MknodFileType::Block => libc::S_IFBLK | mode,
        MknodFileType::Character => libc::S_IFCHR | mode,
        MknodFileType::Fifo => libc::S_IFIFO | mode,
    };

    if context_request.is_set() {
        // 传入 full_mode 而不是 mode
        set_security_context(context_request, file_name, full_mode)
            .map_err(|e| CtSimpleError::new(1, e))?;
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
            _mknod(file_name, S_IFIFO | mode, 0)
                .map_err(|error| CtSimpleError::new(1, creation_error_message(file_name, &error)))
        }
    } else {
        // 对于块设备和字符设备，需要主、次设备号
        match (
            args_match.get_one::<String>("major"),
            args_match.get_one::<String>("minor"),
        ) {
            (_, None) | (None, _) => Err(CTsageError::new(
                1,
                "Special files require major and minor device numbers.",
            )),
            (Some(raw_major), Some(raw_minor)) => {
                let major = parse_device_component(raw_major, "major")?;
                let minor = parse_device_component(raw_minor, "minor")?;
                validate_device_number_range(major, OsStr::new(raw_major), "major")?;
                validate_device_number_range(minor, OsStr::new(raw_minor), "minor")?;

                // 检查设备号是否有效（NODEV检查）
                #[cfg(target_os = "linux")]
                {
                    const NODEV: dev_t = !0;
                    let dev = make_dev(major, minor);
                    if dev == NODEV {
                        return Err(CtSimpleError::new(
                            1,
                            format!("invalid device {raw_major} {raw_minor}"),
                        ));
                    }
                }

                let dev = make_dev(major, minor);
                let create_result = match file_type {
                    MknodFileType::Block => _mknod(file_name, S_IFBLK | mode, dev),
                    MknodFileType::Character => _mknod(file_name, S_IFCHR | mode, dev),
                    _ => unreachable!("file_type was validated to be only block or character"),
                };
                create_result.map_err(|error| {
                    CtSimpleError::new(1, creation_error_message(file_name, &error))
                })
            }
        }
    };

    // 如果文件创建成功且用户指定了模式，使用chmod确保权限正确设置
    // 这与GNU的lchmod()调用等价（对于非符号链接文件）
    if result.is_ok() && args_match.contains_id("mode") {
        unsafe {
            let c_path =
                CString::new(file_name.as_bytes()).expect("Failed to convert path to CString");
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
            .allow_hyphen_values(true)
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
            .value_parser(ValueParser::os_string())
            .value_hint(clap::ValueHint::AnyPath),
        Arg::new("type")
            .value_name("TYPE")
            .help(t!("mknod.clap.type"))
            .required(true)
            .value_parser(ValueParser::os_string()),
        Arg::new("major")
            .value_name("MAJOR")
            .help(t!("mknod.clap.major")),
        Arg::new("minor")
            .value_name("MINOR")
            .help(t!("mknod.clap.minor")),
    ];

    Command::new(utility_name)
        .version(command_version)
        .override_usage(usage_description)
        .after_help(after_info)
        .about(application_info)
        .infer_long_args(true)
        .args_override_self(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .args(&args)
}

// 解析模式参数。
fn get_mode(matches: &ArgMatches) -> Result<mode_t, String> {
    match matches.get_one::<String>("mode") {
        None => Ok(MODE_RW_UGO),
        Some(str_mode) => parse_mknod_mode(str_mode)
            .map_err(|_| "invalid mode".to_string())
            .and_then(|mode| {
                if mode > 0o777 {
                    Err("mode must specify only file permission bits".to_string())
                } else {
                    Ok(mode)
                }
            }),
    }
}

fn parse_mknod_mode(str_mode: &str) -> Result<mode_t, String> {
    if str_mode.chars().any(|c| c.is_ascii_digit()) {
        ctcore::ct_mode::parse_mode(str_mode)
    } else {
        let mut mode = MODE_RW_UGO;
        let umask = ctcore::ct_mode::get_umask();
        for part in str_mode.split(',') {
            if part.is_empty() {
                return Err(str_mode.to_string());
            }
            mode = ctcore::ct_mode::parse_symbolic(mode, part, umask, false)?;
        }
        Ok(mode as mode_t)
    }
}

// 解析文件类型参数。
fn parse_type(tpe: &OsStr) -> Result<MknodFileType, String> {
    match tpe.as_bytes().first() {
        Some(b'b') => Ok(MknodFileType::Block),
        Some(b'c' | b'u') => Ok(MknodFileType::Character),
        Some(b'p') => Ok(MknodFileType::Fifo),
        None => Err("missing device type".to_string()),
        _ => Err(format!("invalid device type {}", tpe.quote())),
    }
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
        let tool = Mknod;

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
        use crate::{
            ct_app, get_mode, mknod_main, parse_device_component, parse_device_number,
            parse_mknod_mode, validate_device_number_range,
        };

        use std::ffi::{OsStr, OsString};

        #[test]
        fn test_mknod_main_version() {
            let args = [ctcore::ct_util_name(), "--version"];

            let result = mknod_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_mknod_main_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = mknod_main(args.iter().map(OsString::from));

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

        #[test]
        fn test_mknod_comma_separated_symbolic_mode() {
            let args = vec![ctcore::ct_util_name(), "--mode=u=rw,go=", "file", "p"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();

            assert_eq!(get_mode(&matches).unwrap(), 0o600);
        }

        #[test]
        fn test_mknod_rejects_empty_symbolic_mode_part() {
            for mode in ["u=rw,", ",u=rw", "u=rw,,go="] {
                let args = vec![ctcore::ct_util_name(), "--mode", mode, "file", "p"];
                let command = ct_app();
                let matches = command.try_get_matches_from(args).unwrap();

                assert!(get_mode(&matches).is_err());
            }
        }

        #[test]
        fn test_mknod_symbolic_mode_uses_nondirectory_semantics() {
            assert_eq!(parse_mknod_mode("a=X"), Ok(0));
            assert_eq!(parse_mknod_mode("u+s=u"), Ok(0o666));
            assert_eq!(parse_mknod_mode("g+s=g"), Ok(0o666));
        }

        #[test]
        fn test_invalid_modes_use_gnu_diagnostic() {
            for mode in ["999", "u=rw,", "not-a-mode"] {
                let matches = ct_app()
                    .try_get_matches_from(["mknod", "-m", mode, "file", "p"])
                    .unwrap();
                assert_eq!(get_mode(&matches), Err("invalid mode".to_string()));
            }
        }

        #[test]
        fn test_device_number_zero() {
            assert_eq!(parse_device_number("0"), Ok(0));
            assert_eq!(parse_device_number("00"), Ok(0));
        }

        #[test]
        fn test_device_number_leading_whitespace() {
            assert_eq!(parse_device_number(" \t\n\r\u{b}\u{c}0x10"), Ok(16));
            assert!(parse_device_number("10 ").is_err());
        }

        #[test]
        fn test_device_number_range() {
            assert!(
                validate_device_number_range(u32::MAX.into(), OsStr::new("max"), "major").is_ok()
            );
            assert_eq!(
                validate_device_number_range(
                    u64::from(u32::MAX) + 1,
                    OsStr::new("0x100000000"),
                    "minor"
                )
                .unwrap_err()
                .to_string(),
                "invalid minor device number '0x100000000'"
            );
        }

        #[test]
        fn test_invalid_device_component_diagnostic() {
            assert_eq!(
                parse_device_component("08", "major")
                    .unwrap_err()
                    .to_string(),
                "invalid major device number '08'"
            );
            assert_eq!(
                parse_device_component("0x", "minor")
                    .unwrap_err()
                    .to_string(),
                "invalid minor device number '0x'"
            );
        }
    }

    mod tests_mknod_app {
        use crate::{
            apply_posix_option_order, ct_app, parse_security_context_request, parse_type,
            validate_gnu_mknod_operands, validate_gnu_mknod_options,
        };

        use clap::error::ErrorKind;
        use std::ffi::OsString;
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

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

        #[test]
        fn test_ct_app_repeated_options() {
            let matches = ct_app()
                .try_get_matches_from([
                    ctcore::ct_util_name(),
                    "-m",
                    "600",
                    "--mode=700",
                    "-Z",
                    "-Z",
                    "--context",
                    "--context",
                    "file",
                    "p",
                ])
                .unwrap();

            assert_eq!(matches.get_one::<String>("mode").unwrap(), "700");
            assert!(matches.get_flag("ctx"));
            assert!(matches.contains_id("context"));
        }

        #[test]
        fn test_ct_app_accepts_separate_hyphen_mode() {
            let matches = ct_app()
                .try_get_matches_from([ctcore::ct_util_name(), "-m", "-w", "file", "p"])
                .unwrap();

            assert_eq!(matches.get_one::<String>("mode").unwrap(), "-w");
        }

        #[test]
        fn test_ct_app_accepts_non_utf8_name_and_type() {
            let name = OsString::from_vec(b"name-\xff".to_vec());
            let file_type = OsString::from_vec(b"p\xfe".to_vec());
            let matches = ct_app()
                .try_get_matches_from([OsString::from("mknod"), name.clone(), file_type.clone()])
                .unwrap();

            assert_eq!(matches.get_one::<OsString>("name"), Some(&name));
            assert!(matches!(
                parse_type(file_type.as_os_str()),
                Ok(crate::MknodFileType::Fifo)
            ));
        }

        #[test]
        fn test_gnu_option_diagnostics() {
            for (args, expected) in [
                (
                    vec!["mknod", "--definitely-unknown", "x", "p"],
                    "unrecognized option '--definitely-unknown'",
                ),
                (vec!["mknod", "-m"], "option requires an argument -- 'm'"),
                (
                    vec!["mknod", "--mode"],
                    "option '--mode' requires an argument",
                ),
                (vec!["mknod", "-x", "x", "p"], "invalid option -- 'x'"),
            ] {
                let args = args.into_iter().map(OsString::from).collect::<Vec<_>>();
                let error = validate_gnu_mknod_options(&args).unwrap_err();
                assert_eq!(error.to_string(), expected);
            }
        }

        #[test]
        fn test_posix_option_order_stops_at_first_operand() {
            let mut args = ["mknod", "-Z", "--mode", "600", "file", "p", "-m", "700"]
                .into_iter()
                .map(OsString::from)
                .collect::<Vec<_>>();

            apply_posix_option_order(&mut args, true);

            assert_eq!(
                args,
                [
                    "mknod", "-Z", "--mode", "600", "--", "file", "p", "-m", "700"
                ]
                .into_iter()
                .map(OsString::from)
                .collect::<Vec<_>>()
            );
        }

        #[test]
        fn test_gnu_operand_diagnostics() {
            for (args, expected) in [
                (vec!["mknod"], "missing operand"),
                (vec!["mknod", "x"], "missing operand after 'x'"),
                (
                    vec!["mknod", "x", "b"],
                    "missing operand after 'b'\nSpecial files require major and minor device numbers.",
                ),
                (
                    vec!["mknod", "x", "p", "1", "2"],
                    "extra operand '1'\nFifos do not have major and minor device numbers.",
                ),
                (
                    vec!["mknod", "x", "c", "1", "2", "extra"],
                    "extra operand 'extra'",
                ),
                (vec!["mknod", "x", "q", "1", "2"], "invalid device type 'q'"),
            ] {
                let args = args.into_iter().map(OsString::from).collect::<Vec<_>>();
                let error = validate_gnu_mknod_operands(&args).unwrap_err();
                assert_eq!(error.to_string(), expected);
            }
        }

        #[test]
        fn test_ct_app_context_without_value_maps_to_default_context() {
            let args = vec![ctcore::ct_util_name(), "--context", "file", "p"];
            let command = ct_app();
            let matches = command.try_get_matches_from(args).unwrap();

            assert!(matches.contains_id("context"));
            let request = parse_security_context_request(&[
                std::ffi::OsString::from("mknod"),
                std::ffi::OsString::from("--context"),
                std::ffi::OsString::from("file"),
                std::ffi::OsString::from("p"),
            ]);
            assert!(request.use_default);
            assert!(request.explicit_context.is_none());
        }

        #[test]
        fn test_ct_app_context_with_explicit_empty_value_is_preserved() {
            let request = parse_security_context_request(&[
                std::ffi::OsString::from("mknod"),
                std::ffi::OsString::from("--context="),
                std::ffi::OsString::from("file"),
                std::ffi::OsString::from("p"),
            ]);
            assert_eq!(request.explicit_context, Some(OsString::new()));
            assert_eq!(request.explicit_occurrences, 1);
        }

        #[test]
        fn test_non_utf8_explicit_context_is_preserved() {
            let value = OsString::from_vec(b"value-\xff".to_vec());
            let mut argument = b"--context=".to_vec();
            argument.extend_from_slice(value.as_os_str().as_bytes());
            let request = parse_security_context_request(&[
                OsString::from("mknod"),
                OsString::from_vec(argument),
                OsString::from("file"),
                OsString::from("p"),
            ]);

            assert_eq!(request.explicit_context, Some(value));
            assert_eq!(request.explicit_occurrences, 1);
        }

        #[test]
        fn test_context_scanner_stops_at_double_dash() {
            let request = parse_security_context_request(&[
                OsString::from("mknod"),
                OsString::from("--"),
                OsString::from("--context=not-an-option"),
                OsString::from("p"),
            ]);

            assert!(!request.is_set());
        }

        #[test]
        fn test_context_requests_preserve_explicit_warning_and_default_state() {
            let request = parse_security_context_request(&[
                OsString::from("mknod"),
                OsString::from("--context=first"),
                OsString::from("-Z"),
                OsString::from("--context=last"),
                OsString::from("file"),
                OsString::from("p"),
            ]);

            assert_eq!(request.explicit_context, Some(OsString::from("last")));
            assert!(request.use_default);
            assert_eq!(request.explicit_occurrences, 2);
        }
    }
}
