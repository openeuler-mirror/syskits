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

// 在GNU/Linux系统中，nice命令的主要作用是调整程序的执行优先级，从而影响其对CPU资源的访问

extern crate rust_i18n;
use libc::{c_char, c_int, execvp, nice as libc_nice};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use std::borrow::Cow;
use std::ffi::{CString, OsStr, OsString};
use std::fmt::{Display, Formatter};
use std::io::{Error, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::ptr;

use clap::{Arg, ArgAction, Command, builder::OsStringValueParser, crate_version};
use ctcore::{
    Tool,
    ct_error::{CTResult, CtSimpleError, UClapError, set_ct_exit_code, strip_errno},
    ct_quoting_style::gnu_quote_locale,
    ct_show_error,
};
use sys_locale::get_locale;

pub mod opt_flags {
    pub static ADJUSTMENT: &str = "adjustment";
    pub static COMMAND: &str = "COMMAND";
}

fn is_prefix_of(prefix: &str, target: &str, min_match: usize) -> bool {
    if prefix.len() < min_match || prefix.len() > target.len() {
        return false;
    }

    &target[0..prefix.len()] == prefix
}

fn legacy_adjustment(argument: &OsStr) -> Option<OsString> {
    let bytes = argument.as_bytes();
    if bytes.first() != Some(&b'-') {
        return None;
    }

    let number_index = 1 + usize::from(matches!(bytes.get(1), Some(b'-' | b'+')));
    bytes
        .get(number_index)
        .filter(|byte| byte.is_ascii_digit())
        .map(|_| OsString::from_vec(bytes[1..].to_vec()))
}

const NZERO: i32 = 20;
const MIN_ADJUSTMENT: i64 = 1 - 2 * NZERO as i64;
const MAX_ADJUSTMENT: i64 = 2 * NZERO as i64 - 1;

fn parse_adjustment(value: &OsStr) -> Result<i32, Box<dyn ctcore::ct_error::CTError>> {
    let original_value = value;
    let bytes = value.as_bytes();
    let mut digit_start = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let negative = match bytes.get(digit_start) {
        Some(b'-') => {
            digit_start += 1;
            true
        }
        Some(b'+') => {
            digit_start += 1;
            false
        }
        _ => false,
    };
    let digits = &bytes[digit_start..];

    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return Err(CtSimpleError::new(
            125,
            nice_invalid_adjustment_message(original_value, nice_uses_simplified_chinese()),
        ));
    }

    let limit = if negative {
        -MIN_ADJUSTMENT
    } else {
        MAX_ADJUSTMENT
    };
    let mut adjustment = 0i64;
    for byte in digits {
        adjustment = adjustment
            .saturating_mul(10)
            .saturating_add((byte - b'0') as i64);
        if adjustment > limit {
            return Ok(if negative {
                MIN_ADJUSTMENT as i32
            } else {
                MAX_ADJUSTMENT as i32
            });
        }
    }

    Ok(if negative {
        -(adjustment as i32)
    } else {
        adjustment as i32
    })
}

fn perm_related_errno(err: i32) -> bool {
    err == libc::EACCES || err == libc::EPERM
}

fn set_niceness_error_message(error: &Error) -> String {
    set_niceness_error_message_for_locale(error, nice_uses_simplified_chinese())
}

fn set_niceness_error_message_for_locale(error: &Error, simplified_chinese: bool) -> String {
    let errno = error.raw_os_error().unwrap_or_default();
    let description = if perm_related_errno(errno) {
        String::from("Permission denied")
    } else {
        strip_errno(error)
    };
    let message = if simplified_chinese {
        t!("nice.errors.cannot_set_niceness", locale = "zh-CN")
    } else {
        t!("nice.errors.cannot_set_niceness", locale = "en-US")
    };
    format!("{message}: {description}")
}

fn exec_error_message(command: &OsStr, error: &Error) -> String {
    exec_error_message_for_locale(command, error, nice_uses_simplified_chinese())
}

fn exec_error_message_for_locale(
    command: &OsStr,
    error: &Error,
    simplified_chinese: bool,
) -> String {
    format!(
        "{}: {}",
        nice_quote_operand(command, simplified_chinese),
        strip_errno(error)
    )
}

fn nice_uses_simplified_chinese() -> bool {
    rust_i18n::locale() == "zh-CN"
}

fn nice_invalid_adjustment_message(value: &OsStr, simplified_chinese: bool) -> String {
    let quoted = nice_quote_operand(value, simplified_chinese);
    if simplified_chinese {
        t!(
            "nice.errors.invalid_adjustment",
            locale = "zh-CN",
            value = quoted
        )
        .to_string()
    } else {
        format!("invalid adjustment {quoted}")
    }
}

fn nice_missing_command_message(simplified_chinese: bool) -> String {
    if simplified_chinese {
        t!("nice.errors.missing_command", locale = "zh-CN").to_string()
    } else {
        t!("nice.errors.missing_command", locale = "en-US").to_string()
    }
}

fn nice_get_niceness_message(simplified_chinese: bool) -> String {
    if simplified_chinese {
        t!("nice.errors.cannot_get_niceness", locale = "zh-CN").to_string()
    } else {
        t!("nice.errors.cannot_get_niceness", locale = "en-US").to_string()
    }
}

fn nice_usage_hint(utility_name: &str, simplified_chinese: bool) -> Vec<u8> {
    if simplified_chinese {
        format!("请尝试执行 \"{utility_name} --help\" 来获取更多信息。").into_bytes()
    } else {
        format!("Try '{utility_name} --help' for more information.").into_bytes()
    }
}

fn nice_quote_operand(value: &OsStr, simplified_chinese: bool) -> String {
    if !simplified_chinese {
        return gnu_quote_locale(value);
    }

    let bytes = value.as_bytes();
    let mut quoted = String::with_capacity(bytes.len() + 2);
    quoted.push('"');
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii()
            && let Ok(remaining) = std::str::from_utf8(&bytes[index..])
        {
            quoted.push_str(remaining);
            break;
        }

        match bytes[index] {
            b'\x07' => quoted.push_str("\\a"),
            b'\x08' => quoted.push_str("\\b"),
            b'\t' => quoted.push_str("\\t"),
            b'\n' => quoted.push_str("\\n"),
            b'\x0b' => quoted.push_str("\\v"),
            b'\x0c' => quoted.push_str("\\f"),
            b'\r' => quoted.push_str("\\r"),
            b'\\' => quoted.push_str("\\\\"),
            b'"' => quoted.push_str("\\\""),
            0x20..=0x7e => quoted.push(char::from(bytes[index])),
            byte => {
                quoted.push('\\');
                quoted.push(char::from(b'0' + (byte >> 6)));
                quoted.push(char::from(b'0' + ((byte >> 3) & 0o7)));
                quoted.push(char::from(b'0' + (byte & 0o7)));
            }
        }
        index += 1;
    }
    quoted.push('"');
    quoted
}

fn command_arg_to_cstring(argument: &OsStr) -> CString {
    CString::new(argument.as_encoded_bytes())
        .expect("process arguments received from the operating system cannot contain NUL bytes")
}

fn get_current_niceness() -> Result<c_int, Error> {
    nix::errno::Errno::clear();
    let value = unsafe { libc_nice(0) };
    let err = Error::last_os_error();
    if value == -1 && err.raw_os_error().unwrap_or(0) != 0 {
        return Err(err);
    }
    Ok(value)
}

fn nice_normalize_stdout_write_error(error: Error, stdout_was_closed: bool) -> Error {
    #[cfg(unix)]
    if stdout_was_closed {
        return Error::from_raw_os_error(libc::EBADF);
    }

    error
}

fn nice_write_error(error: Error, stdout_was_closed: bool) -> Box<dyn ctcore::ct_error::CTError> {
    let error = nice_normalize_stdout_write_error(error, stdout_was_closed);
    CtSimpleError::new(
        125,
        format!("{}: {}", t!("nice.errors.write_error"), strip_errno(&error)),
    )
}

fn write_niceness(
    writer: &mut impl Write,
    niceness: c_int,
    stdout_was_closed: bool,
) -> CTResult<()> {
    writeln!(writer, "{niceness}").map_err(|error| nice_write_error(error, stdout_was_closed))
}

fn apply_adjustment(adjustment: c_int) -> Result<(), CTResult<()>> {
    nix::errno::Errno::clear();
    let value = unsafe { libc_nice(adjustment) };
    let err = Error::last_os_error();
    if value == -1 && err.raw_os_error().unwrap_or(0) != 0 {
        let errno = err.raw_os_error().unwrap_or(0);
        let message = set_niceness_error_message(&err);
        if perm_related_errno(errno) {
            if writeln!(std::io::stderr(), "{}: {}", ctcore::ct_util_name(), message).is_err() {
                set_ct_exit_code(125);
                return Err(Ok(()));
            }
            return Ok(());
        }
        return Err(Err(CtSimpleError::new(125, message)));
    }
    Ok(())
}

/// 将传统的参数转换为标准化形式。
///
/// 下面是GNU nice命令合法的参数序列：
/// - "-1"
/// - "-n1"
/// - "-+1"
/// - "--1"
/// - "-n -1"
///
/// 最初看起来，我们可以在处理"-{i}"、"--{i}"和"-+{i}"形式的整数{i}时，
/// 使用clap进行正常处理。然而，"-1"等参数的意义取决于其在传统参数解析中的上下文。
/// clap会将连字符值优先匹配为已知参数，而不是将其解释为前一个参数的值。因此，在这种情况下，
/// "-n" "-1"会被解释为两个参数，而不是一个带有值的参数。
///
/// 由于这种上下文依赖性，以及在这种情况下使用clap所带来的深层次问题，
/// 最简单的方法是在clap开始工作之前，将nice的参数标准化。在这里，
/// 我们将所有形式为"-{i}"、"--{i}"和"-+{i}"的参数（如果不已经被"-n"预置）前面插入"-n"前缀。
fn standardize_nice_args(mut args: impl ctcore::Args) -> impl ctcore::Args {
    let mut vec = Vec::<OsString>::new(); // 存储标准化后的参数
    let mut is_saw_n = false; // 标记是否已看到"-n"参数
    let mut is_saw_command = false; // 标记是否已看到命令参数

    // 处理第一个参数，通常是命令名
    if let Some(cmd) = args.next() {
        vec.push(cmd);
    }

    // 遍历剩余的参数
    for str in args {
        if is_saw_command {
            vec.push(str);
        } else if is_saw_n {
            // getopt_long treats `--` as the required value of -n.
            if str.is_empty() {
                vec.push("--adjustment=".into());
            } else {
                let mut new_arg: OsString = "-n".into();
                new_arg.push(str);
                vec.push(new_arg);
            }
            is_saw_n = false; // 重置"-n"标记
        } else if str.to_str() == Some("--") {
            // "--" 结束选项解析，后续均视为命令参数
            vec.push(str);
            is_saw_command = true;
        } else if str.to_str() == Some("-n")
            || str
                .to_str()
                .map(|s| is_prefix_of(s, "--adjustment", "--a".len()))
                .unwrap_or_default()
        {
            // 处理"-n"和"--adjustment"参数
            is_saw_n = true;
        } else if let Some(adjustment) = legacy_adjustment(str.as_os_str()) {
            // GNU在转换数值前按首数字识别传统-N、--N和-+N形式。
            let mut new_arg: OsString = "-n".into();
            new_arg.push(adjustment);
            vec.push(new_arg);
        } else if let Ok(s) = str.clone().into_string() {
            // 首个非选项参数开始命令及其参数；未知选项仍交由clap诊断。
            is_saw_command |= !s.starts_with('-');
            vec.push(s.into());
        } else {
            // 添加非字符串可转换的参数作为命令
            is_saw_command = true;
            vec.push(str);
        }
    }

    // 如果最后未看到"-n"，则添加
    if is_saw_n {
        vec.push("-n".into());
    }

    vec.into_iter() // 返回标准化后的参数迭代器
}

const NICE_LONG_OPTIONS: &[&str] = &["adjustment", "help", "version"];

enum NiceLongOptionMatch {
    None,
    Recognized(&'static str),
    Ambiguous(Vec<&'static str>),
}

enum NiceOptionAction {
    Continue,
    ConsumeNext,
    Stop,
}

fn match_nice_long_option(name: &[u8]) -> NiceLongOptionMatch {
    if let Some(option) = NICE_LONG_OPTIONS
        .iter()
        .copied()
        .find(|option| option.as_bytes() == name)
    {
        return NiceLongOptionMatch::Recognized(option);
    }

    let matches = NICE_LONG_OPTIONS
        .iter()
        .copied()
        .filter(|option| option.as_bytes().starts_with(name))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => NiceLongOptionMatch::None,
        [option] => NiceLongOptionMatch::Recognized(option),
        _ => NiceLongOptionMatch::Ambiguous(matches),
    }
}

fn nice_usage_error(message: Vec<u8>) -> Box<dyn ctcore::ct_error::CTError> {
    Box::new(NiceUsageError {
        message,
        usage_hint: nice_usage_hint(
            ctcore::ct_help_utility_name(),
            nice_uses_simplified_chinese(),
        ),
    })
}

fn validate_nice_long_option(argument: &[u8], has_next: bool) -> CTResult<NiceOptionAction> {
    let long = &argument[2..];
    let separator = long.iter().position(|byte| *byte == b'=');
    let name = &long[..separator.unwrap_or(long.len())];

    match match_nice_long_option(name) {
        NiceLongOptionMatch::None => {
            let mut message = b"unrecognized option '".to_vec();
            message.extend_from_slice(argument);
            message.push(b'\'');
            Err(nice_usage_error(message))
        }
        NiceLongOptionMatch::Ambiguous(options) => {
            let mut message = b"option '".to_vec();
            message.extend_from_slice(argument);
            message.extend_from_slice(b"' is ambiguous; possibilities:");
            for option in options {
                message.extend_from_slice(b" '--");
                message.extend_from_slice(option.as_bytes());
                message.push(b'\'');
            }
            Err(nice_usage_error(message))
        }
        NiceLongOptionMatch::Recognized("adjustment") if separator.is_none() && !has_next => Err(
            nice_usage_error(b"option '--adjustment' requires an argument".to_vec()),
        ),
        NiceLongOptionMatch::Recognized("adjustment") if separator.is_none() => {
            Ok(NiceOptionAction::ConsumeNext)
        }
        NiceLongOptionMatch::Recognized("adjustment") => Ok(NiceOptionAction::Continue),
        NiceLongOptionMatch::Recognized(canonical) if separator.is_some() => Err(nice_usage_error(
            format!("option '--{canonical}' doesn't allow an argument").into_bytes(),
        )),
        NiceLongOptionMatch::Recognized("help" | "version") => Ok(NiceOptionAction::Stop),
        NiceLongOptionMatch::Recognized(_) => unreachable!("all nice long options are handled"),
    }
}

fn validate_nice_short_option(argument: &[u8], has_next: bool) -> CTResult<NiceOptionAction> {
    let option = argument[1];
    if option != b'n' {
        let mut message = b"invalid option -- '".to_vec();
        message.push(option);
        message.push(b'\'');
        return Err(nice_usage_error(message));
    }

    if argument.len() == 2 {
        if !has_next {
            return Err(nice_usage_error(
                b"option requires an argument -- 'n'".to_vec(),
            ));
        }
        return Ok(NiceOptionAction::ConsumeNext);
    }

    Ok(NiceOptionAction::Continue)
}

fn prepare_nice_args(args: impl ctcore::Args) -> CTResult<Vec<OsString>> {
    let args = args.collect::<Vec<_>>();
    let mut index = 1;

    while index < args.len() {
        let argument = args[index].as_os_str();
        let bytes = argument.as_bytes();

        if legacy_adjustment(argument).is_some() {
            index += 1;
            continue;
        }
        if bytes == b"--" || bytes.len() <= 1 || bytes[0] != b'-' {
            break;
        }

        let action = if bytes.starts_with(b"--") {
            validate_nice_long_option(bytes, index + 1 < args.len())?
        } else {
            validate_nice_short_option(bytes, index + 1 < args.len())?
        };
        match action {
            NiceOptionAction::Continue => index += 1,
            NiceOptionAction::ConsumeNext => index += 2,
            NiceOptionAction::Stop => break,
        }
    }

    Ok(args)
}

#[derive(Debug)]
struct NiceUsageError {
    message: Vec<u8>,
    usage_hint: Vec<u8>,
}

impl Display for NiceUsageError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        String::from_utf8_lossy(&self.message).fmt(formatter)
    }
}

impl std::error::Error for NiceUsageError {}

impl ctcore::ct_error::CTError for NiceUsageError {
    fn code(&self) -> i32 {
        125
    }

    fn diagnostic_bytes(&self) -> Cow<'_, [u8]> {
        Cow::Borrowed(&self.message)
    }

    fn usage_hint_bytes(&self) -> Option<Cow<'_, [u8]>> {
        Some(Cow::Borrowed(&self.usage_hint))
    }

    fn usage(&self) -> bool {
        true
    }
}

/**
 * 主要功能：根据提供的参数调整当前进程的优先级，并执行指定的命令。
 *
 * 参数：
 * args - 实现了 `ctcore::Args` 接口的对象，代表命令行参数。
 *
 * 返回值：
 * `CTResult<()>` - 成功时返回 `Ok(())`，错误时返回包含错误信息的 `Err`。
 *
 * 注意：
 * 此函数会根据提供的参数调整当前进程的优先级，如果调整失败或无法执行指定的命令，
 * 会返回相应的错误代码和错误信息。
 */
pub fn nice_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    // Match GNU getopt_long diagnostics before clap parses normalized arguments.
    let args = standardize_nice_args(prepare_nice_args(args)?.into_iter());

    // 使用 clap 库解析命令行参数
    let args_match = ct_app().try_get_matches_from(args).with_exit_code(125)?;

    let has_command = args_match.contains_id(opt_flags::COMMAND);
    let adjustment_value = args_match.get_one::<OsString>(opt_flags::ADJUSTMENT);

    if !has_command {
        if let Some(value) = adjustment_value {
            let _ = parse_adjustment(value.as_os_str())?;
            let simplified_chinese = nice_uses_simplified_chinese();
            ct_show_error!("{}", nice_missing_command_message(simplified_chinese));
            eprintln!(
                "{}",
                String::from_utf8_lossy(&nice_usage_hint(
                    ctcore::ct_help_utility_name(),
                    simplified_chinese,
                ))
            );
            set_ct_exit_code(125);
            return Ok(());
        }

        match get_current_niceness() {
            Ok(value) => {
                let mut stdout = std::io::stdout().lock();
                write_niceness(&mut stdout, value, ctcore::ct_stdout_was_closed())?;
                return Ok(());
            }
            Err(err) => {
                ct_show_error!(
                    "{}: {}",
                    nice_get_niceness_message(nice_uses_simplified_chinese()),
                    strip_errno(&err)
                );
                set_ct_exit_code(125);
                return Ok(());
            }
        }
    }

    let adjustment = match adjustment_value {
        Some(value) => parse_adjustment(value.as_os_str())?,
        None => 10,
    };

    if let Err(value) = apply_adjustment(adjustment) {
        return value;
    }

    // 准备执行命令需要的参数，并执行命令
    let command_args: Vec<OsString> = args_match
        .get_many::<OsString>(opt_flags::COMMAND)
        .unwrap()
        .cloned()
        .collect();
    let cstr: Vec<CString> = command_args
        .iter()
        .map(|x| command_arg_to_cstring(x.as_os_str()))
        .collect();

    let mut args: Vec<*const c_char> = cstr.iter().map(|s| s.as_ptr()).collect();
    args.push(ptr::null::<c_char>());
    unsafe {
        execvp(args[0], args.as_mut_ptr());
    }

    // 执行命令失败的处理
    let exec_error = Error::last_os_error();
    ct_show_error!(
        "{}",
        exec_error_message(command_args[0].as_os_str(), &exec_error)
    );
    let exit_code = if exec_error.raw_os_error().unwrap_or(0) as c_int == libc::ENOENT {
        127
    } else {
        126
    };
    set_ct_exit_code(exit_code);
    Ok(())
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("nice.about");
    let usage_description = t!("nice.usage");

    let args = vec![
        Arg::new(opt_flags::ADJUSTMENT)
            .short('n')
            .long(opt_flags::ADJUSTMENT)
            .help(t!("nice.clap.adjustment"))
            .action(ArgAction::Set)
            .value_parser(OsStringValueParser::new())
            .overrides_with(opt_flags::ADJUSTMENT)
            .allow_hyphen_values(true),
        Arg::new(opt_flags::COMMAND)
            .action(ArgAction::Append)
            .value_parser(OsStringValueParser::new())
            .value_hint(clap::ValueHint::CommandName),
    ];

    Command::new(utility_name)
        .about(application_info)
        .override_usage(usage_description)
        .trailing_var_arg(true)
        .infer_long_args(true)
        .version(command_version)
        .args(&args)
}

#[derive(Default)]
pub struct Nice;
impl Tool for Nice {
    fn name(&self) -> &'static str {
        "nice"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        // 将&[OsString]转换为符合Args trait要求的iterator
        nice_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Nice;

        // 测试 name 方法
        assert_eq!(tool.name(), "nice");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("nice"));

        // 测试 execute 方法
        let args = vec![OsString::from("nice"), OsString::from("--help")];
        assert!(tool.execute(&args).is_err());
    }

    #[test]
    fn test_is_prefix_of() {
        // 测试前缀判断函数
        assert!(is_prefix_of("--a", "--adjustment", "--a".len()));
        assert!(is_prefix_of("--adj", "--adjustment", "--a".len()));
        assert!(is_prefix_of("--adjustment", "--adjustment", "--a".len()));

        // 不满足最小匹配长度
        assert!(!is_prefix_of("--", "--adjustment", "--a".len()));

        // 前缀长度超过目标字符串
        assert!(!is_prefix_of("--adjustmentx", "--adjustment", "--a".len()));

        // 非前缀字符串
        assert!(!is_prefix_of("--other", "--adjustment", "--a".len()));
    }

    #[test]
    fn test_parse_adjustment_matches_gnu_xstrtol_rules() {
        assert_eq!(parse_adjustment(OsStr::new(" 1")).unwrap(), 1);
        assert_eq!(
            parse_adjustment(OsStr::new("9223372036854775808")).unwrap(),
            39
        );
        assert_eq!(
            parse_adjustment(OsStr::new("-9223372036854775809")).unwrap(),
            -39
        );
        assert!(parse_adjustment(OsStr::new("1 ")).is_err());
    }

    #[test]
    fn test_prepare_nice_args_uses_gnu_getopt_diagnostics() {
        let cases = [
            (vec!["nice", "-z"], b"invalid option -- 'z'".as_slice()),
            (
                vec!["nice", "--unknown"],
                b"unrecognized option '--unknown'".as_slice(),
            ),
            (vec!["nice", "---"], b"unrecognized option '---'".as_slice()),
            (
                vec!["nice", "-n"],
                b"option requires an argument -- 'n'".as_slice(),
            ),
            (
                vec!["nice", "--adjustment"],
                b"option '--adjustment' requires an argument".as_slice(),
            ),
            (
                vec!["nice", "--a"],
                b"option '--adjustment' requires an argument".as_slice(),
            ),
            (
                vec!["nice", "--help=value"],
                b"option '--help' doesn't allow an argument".as_slice(),
            ),
            (
                vec!["nice", "--=value"],
                b"option '--=value' is ambiguous; possibilities: '--adjustment' '--help' '--version'"
                    .as_slice(),
            ),
        ];

        for (args, expected) in cases {
            let error = prepare_nice_args(args.into_iter().map(OsString::from)).unwrap_err();
            assert_eq!(error.code(), 125);
            assert_eq!(error.diagnostic_bytes().as_ref(), expected);
            assert!(error.usage());
        }
    }

    #[test]
    fn test_prepare_nice_args_stops_at_command_and_terminal_options() {
        let command_args = [
            OsString::from("nice"),
            OsString::from("command"),
            OsString::from("--unknown"),
        ];
        assert_eq!(
            prepare_nice_args(command_args.clone().into_iter()).unwrap(),
            command_args
        );

        let help_args = [
            OsString::from("nice"),
            OsString::from("--help"),
            OsString::from("--unknown"),
        ];
        assert_eq!(
            prepare_nice_args(help_args.clone().into_iter()).unwrap(),
            help_args
        );
    }

    #[test]
    fn test_write_niceness_reports_gnu_style_output_errors() {
        struct FullWriter;

        impl Write for FullWriter {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(Error::from_raw_os_error(libc::ENOSPC))
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let error = write_niceness(&mut FullWriter, 0, false).unwrap_err();
        assert_eq!(error.code(), 125);
        assert_eq!(error.to_string(), "write error: No space left on device");

        let closed =
            nice_normalize_stdout_write_error(Error::from_raw_os_error(libc::ENOSPC), true);
        assert_eq!(closed.raw_os_error(), Some(libc::EBADF));
    }

    #[test]
    fn test_set_niceness_permission_error_matches_gnu_linux_text() {
        let error = Error::from_raw_os_error(libc::EPERM);
        assert_eq!(
            set_niceness_error_message(&error),
            "cannot set niceness: Permission denied"
        );
    }

    #[test]
    fn test_simplified_chinese_diagnostics_match_gnu() {
        let permission_error = Error::from_raw_os_error(libc::EPERM);
        let missing_error = Error::from_raw_os_error(libc::ENOENT);

        assert_eq!(
            nice_invalid_adjustment_message(OsStr::new("bad"), true),
            "无效的调整值 \"bad\""
        );
        assert_eq!(
            nice_missing_command_message(true),
            "指定调整值时，必须同时指定命令"
        );
        assert_eq!(
            nice_usage_hint("nice", true),
            "请尝试执行 \"nice --help\" 来获取更多信息。".as_bytes()
        );
        assert_eq!(
            set_niceness_error_message_for_locale(&permission_error, true),
            "无法设置优先级: Permission denied"
        );
        assert_eq!(
            exec_error_message_for_locale(OsStr::new("no_such"), &missing_error, true),
            "\"no_such\": No such file or directory"
        );
        assert_eq!(
            nice_quote_operand(OsStr::new("a\"b\\c"), true),
            "\"a\\\"b\\\\c\""
        );
    }

    #[test]
    fn test_exec_error_message_omits_rust_os_error_suffix() {
        let error = Error::from_raw_os_error(libc::ENOENT);
        let message = exec_error_message(OsStr::new("no_such"), &error);
        assert!(message.ends_with(": No such file or directory"));
        assert!(!message.contains("(os error"));
    }

    #[test]
    fn test_standardize_nice_args() {
        // 测试基本场景
        let args = vec!["nice", "-5", "command", "arg1"];
        let result: Vec<OsString> =
            standardize_nice_args(args.into_iter().map(OsString::from)).collect();
        assert_eq!(
            result,
            vec![
                OsString::from("nice"),
                OsString::from("-n5"),
                OsString::from("command"),
                OsString::from("arg1"),
            ]
        );

        // 测试 -n 参数
        let args = vec!["nice", "-n", "10", "command"];
        let result: Vec<OsString> =
            standardize_nice_args(args.into_iter().map(OsString::from)).collect();
        assert_eq!(
            result,
            vec![
                OsString::from("nice"),
                OsString::from("-n10"),
                OsString::from("command"),
            ]
        );

        // 测试 --adjustment 参数
        let args = vec!["nice", "--adjustment", "15", "command"];
        let result: Vec<OsString> =
            standardize_nice_args(args.into_iter().map(OsString::from)).collect();
        assert_eq!(
            result,
            vec![
                OsString::from("nice"),
                OsString::from("-n15"),
                OsString::from("command"),
            ]
        );

        let args = vec!["nice", "-n", "", "command"];
        let result: Vec<OsString> =
            standardize_nice_args(args.into_iter().map(OsString::from)).collect();
        assert_eq!(
            result,
            vec![
                OsString::from("nice"),
                OsString::from("--adjustment="),
                OsString::from("command"),
            ]
        );

        // 测试没有指定数值的 -n 参数
        let args = vec!["nice", "-n"];
        let result: Vec<OsString> =
            standardize_nice_args(args.into_iter().map(OsString::from)).collect();
        assert_eq!(result, vec![OsString::from("nice"), OsString::from("-n"),]);

        // GNU getopt_long treats `--` as the required value of a preceding
        // -n option, so it must reach the adjustment parser rather than end
        // option processing.
        let args = vec!["nice", "-n", "--"];
        let result: Vec<OsString> =
            standardize_nice_args(args.into_iter().map(OsString::from)).collect();
        assert_eq!(result, vec![OsString::from("nice"), OsString::from("-n--")]);

        // 测试非数值类型的参数
        let args = vec!["nice", "-invalid", "command"];
        let result: Vec<OsString> =
            standardize_nice_args(args.into_iter().map(OsString::from)).collect();
        assert_eq!(
            result,
            vec![
                OsString::from("nice"),
                OsString::from("-invalid"),
                OsString::from("command"),
            ]
        );

        // 测试 "--" 结束选项解析
        let args = vec!["nice", "--", "-1", "command"];
        let result: Vec<OsString> =
            standardize_nice_args(args.into_iter().map(OsString::from)).collect();
        assert_eq!(
            result,
            vec![
                OsString::from("nice"),
                OsString::from("--"),
                OsString::from("-1"),
                OsString::from("command"),
            ]
        );

        // GNU在转换数值前按首数字识别传统调整值形式，因此超出i64或
        // 带有无效后缀的值仍必须作为调整值进入后续诊断。
        let args = vec!["nice", "-+9223372036854775808", "command"];
        let result: Vec<OsString> =
            standardize_nice_args(args.into_iter().map(OsString::from)).collect();
        assert_eq!(
            result,
            vec![
                OsString::from("nice"),
                OsString::from("-n+9223372036854775808"),
                OsString::from("command"),
            ]
        );

        let args = vec!["nice", "--1a", "command"];
        let result: Vec<OsString> =
            standardize_nice_args(args.into_iter().map(OsString::from)).collect();
        assert_eq!(
            result,
            vec![
                OsString::from("nice"),
                OsString::from("-n-1a"),
                OsString::from("command"),
            ]
        );

        #[cfg(target_os = "linux")]
        {
            use std::os::unix::ffi::{OsStrExt, OsStringExt};

            let args = vec![
                OsString::from("nice"),
                OsString::from_vec(vec![b'-', b'1', 0xff]),
                OsString::from("command"),
            ];
            let result: Vec<OsString> = standardize_nice_args(args.into_iter()).collect();
            assert_eq!(result[1].as_os_str().as_bytes(), b"-n1\xff");
        }
    }

    #[test]
    fn test_parse_adjustment() {
        assert_eq!(parse_adjustment(OsStr::new("5")).unwrap(), 5);
        assert_eq!(parse_adjustment(OsStr::new("+1")).unwrap(), 1);
        assert_eq!(parse_adjustment(OsStr::new("-1")).unwrap(), -1);
        assert_eq!(
            parse_adjustment(OsStr::new("100")).unwrap(),
            MAX_ADJUSTMENT as i32
        );
        assert_eq!(
            parse_adjustment(OsStr::new("-100")).unwrap(),
            MIN_ADJUSTMENT as i32
        );
        assert!(parse_adjustment(OsStr::new("invalid")).is_err());
    }

    // 以下测试需要使用 mock 或集成测试才能完全测试，
    // 因为它们涉及系统调用和进程优先级的修改
    #[test]
    fn test_nice_getprority_mock() {
        // 这个测试主要测试函数的结构性，
        // 真正的系统调用测试需要在集成测试中完成
        let result = get_current_niceness();

        // 在大多数系统上，应该能成功获取当前进程的优先级
        if result.is_ok() {
            let nice_ness = result.unwrap();
            // 优先级通常在-20到19之间
            assert!((-20..=19).contains(&nice_ness));
        }
    }

    mod tests_nice_main {
        use crate::nice_main;

        use std::ffi::OsString;

        #[test]
        fn test_nice_main_version() {
            let args = [ctcore::ct_util_name(), "--version"];

            let result = nice_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_nice_main_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = nice_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }
    }

    mod tests_false_app {
        use crate::{ct_app, opt_flags};

        use clap::error::ErrorKind;
        use std::ffi::OsString;

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
        fn test_ct_app_adjustment() {
            let args = vec![ctcore::ct_util_name(), "--adjustment=20", "ls", "-l"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_n() {
            let args = vec![ctcore::ct_util_name(), "-n", "20", "ls", "-l"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_hyphen_values() {
            // 测试 -n 参数允许以连字符开头的值
            let args = vec![ctcore::ct_util_name(), "-n", "-5", "ls"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(
                matches.get_one::<OsString>("adjustment").unwrap(),
                &OsString::from("-5")
            );
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn test_ct_app_preserves_non_utf8_command_arguments() {
            use std::os::unix::ffi::{OsStrExt, OsStringExt};

            let command = ct_app();
            let arguments = vec![
                OsString::from(ctcore::ct_util_name()),
                OsString::from("/usr/bin/printf"),
                OsString::from("%s"),
                OsString::from_vec(vec![0xff]),
            ];

            let matches = command.try_get_matches_from(arguments).unwrap();
            let command_args: Vec<_> = matches
                .get_many::<OsString>(opt_flags::COMMAND)
                .unwrap()
                .collect();

            assert_eq!(command_args[2].as_os_str().as_bytes(), [0xff]);
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn test_ct_app_preserves_non_utf8_adjustment() {
            use std::os::unix::ffi::OsStringExt;

            let command = ct_app();
            let arguments = vec![
                OsString::from(ctcore::ct_util_name()),
                OsString::from("-n"),
                OsString::from_vec(vec![b'1', 0xff]),
                OsString::from("/usr/bin/true"),
            ];

            assert!(command.try_get_matches_from(arguments).is_ok());
        }
    }
}
