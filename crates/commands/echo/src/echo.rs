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

extern crate rust_i18n;
use clap::Arg;
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use clap::ArgAction;
use clap::Command;
use clap::crate_version;

use ctcore::ct_error::FromIo;
use ctcore::ct_error::{CTResult, CtSimpleError, strip_errno};
#[cfg(all(target_os = "linux", not(test)))]
use ctcore::ct_signals::enable_pipe_errors;

use ctcore::Tool;
use std::ffi::OsString;
use std::io;
use std::io::Write;
use std::ops::ControlFlow;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use sys_locale::get_locale;

#[cfg(all(target_os = "linux", not(test)))]
static INHERITED_SIGPIPE_IGNORED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(all(target_os = "linux", not(test)))]
extern "C" fn record_inherited_sigpipe_disposition() {
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        if libc::sigaction(libc::SIGPIPE, std::ptr::null(), &mut action) == 0
            && action.sa_sigaction == libc::SIG_IGN
        {
            INHERITED_SIGPIPE_IGNORED.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

// This runs before Rust's runtime replaces SIGPIPE with SIG_IGN.
#[cfg(all(target_os = "linux", not(test)))]
#[used]
#[unsafe(link_section = ".preinit_array")]
static RECORD_INHERITED_SIGPIPE_DISPOSITION: extern "C" fn() = record_inherited_sigpipe_disposition;

#[cfg(all(target_os = "linux", not(test)))]
fn inherited_sigpipe_is_ignored() -> bool {
    INHERITED_SIGPIPE_IGNORED.load(std::sync::atomic::Ordering::Relaxed)
}

mod opt_flags {
    pub const STRING: &str = "STRING";
    pub const NO_NEWLINE: &str = "no_newline";
    pub const ENABLE_BACKSLASH_ESCAPE: &str = "enable_backslash_escape";
    pub const DISABLE_BACKSLASH_ESCAPE: &str = "disable_backslash_escape";
}

#[repr(u8)]
// 定义支持的基数枚举，及其最大数字位数
#[derive(Clone, Copy)]
enum EchoBase {
    Oct = 8,  // 八进制
    Hex = 16, // 十六进制
}

impl EchoBase {
    /// 返回基数的最大数字位数
    fn max_digits(&self) -> u8 {
        match self {
            Self::Oct => 3,
            Self::Hex => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EchoEscapeMode {
    Literal,
    Interpreted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EchoSemantic {
    pub inputs: Vec<String>,
    pub text: String,
    pub trailing_newline: bool,
    pub escape_mode: EchoEscapeMode,
    pub terminated_early: bool,
    pub classic_text: String,
    pub classic_bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EchoRender {
    text: String,
    trailing_newline: bool,
    terminated_early: bool,
    classic_text: String,
    classic_bytes: Vec<u8>,
}

struct EchoRenderBytes {
    output: Vec<u8>,
    trailing_newline: bool,
    terminated_early: bool,
}

/// 解析`\xHH`和`\0NNN`转义序列中的数值部分
fn echo_parse_code(input: &[u8], index: &mut usize, base: EchoBase) -> Option<u8> {
    // 由于八进制输入可能需要3个数字，这超过了`u8`的容量，因此这里需要使用溢出加法。
    // 注意，如果使用`u32`和`char::from_u32`，则会对大于`u8::MAX`的值错误地解释为Unicode字符。
    let mut ret = echo_digit(input.get(*index).copied()?, base)?;
    *index += 1;

    // 处理剩余的数字字符，根据基数进行解析。
    for _ in 1..base.max_digits() {
        match input
            .get(*index)
            .copied()
            .and_then(|byte| echo_digit(byte, base))
        {
            Some(n) => ret = ret.wrapping_mul(base as u8).wrapping_add(n),
            None => break,
        }
        *index += 1;
    }

    Some(ret)
}

fn echo_digit(byte: u8, base: EchoBase) -> Option<u8> {
    match (base, byte) {
        (EchoBase::Oct, b'0'..=b'7') => Some(byte - b'0'),
        (EchoBase::Hex, b'0'..=b'9') => Some(byte - b'0'),
        (EchoBase::Hex, b'a'..=b'f') => Some(byte - b'a' + 10),
        (EchoBase::Hex, b'A'..=b'F') => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// 将转义序列写入给定的输出流
fn echo_print_escaped(input: &[u8], mut output: impl Write) -> io::Result<ControlFlow<()>> {
    let mut index = 0;
    while let Some(&byte) = input.get(index) {
        index += 1;
        if byte != b'\\' {
            output.write_all(&[byte])?;
            continue;
        }

        // 处理八进制转义序列（\NNN）的逻辑
        if matches!(input.get(index), Some(b'1'..=b'7')) {
            if let Some(parsed) = echo_parse_code(input, &mut index, EchoBase::Oct) {
                output.write_all(&[parsed])?;
                continue;
            }
        }

        if let Some(&next) = input.get(index) {
            index += 1;
            let unescaped = match next {
                b'\\' => b'\\',
                b'a' => b'\x07',
                b'b' => b'\x08',
                b'c' => return Ok(ControlFlow::Break(())),
                b'e' => b'\x1b',
                b'f' => b'\x0c',
                b'n' => b'\n',
                b'r' => b'\r',
                b't' => b'\t',
                b'v' => b'\x0b',
                b'x' => {
                    if let Some(byte) = echo_parse_code(input, &mut index, EchoBase::Hex) {
                        output.write_all(&[byte])?;
                        continue;
                    } else {
                        output.write_all(b"\\")?;
                        b'x'
                    }
                }
                b'0' => {
                    let byte = echo_parse_code(input, &mut index, EchoBase::Oct).unwrap_or(0);
                    output.write_all(&[byte])?;
                    continue;
                }
                byte => {
                    output.write_all(b"\\")?;
                    byte
                }
            };
            output.write_all(&[unescaped])?;
        } else {
            output.write_all(b"\\")?;
        }
    }

    Ok(ControlFlow::Continue(()))
}

#[derive(Default)]
pub struct Echo;
impl Tool for Echo {
    fn name(&self) -> &'static str {
        "echo"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        echo_main(args.iter().cloned())
    }
}

#[cfg(any(not(unix), test))]
fn echo_is_help_or_version_request(args: &[String], posix_mode: bool) -> bool {
    !posix_mode
        && matches!(
            args,
            [arg] if arg == "--help" || arg == "--version"
        )
}

fn echo_short_options(arg: &str) -> Option<&str> {
    let options = arg.strip_prefix('-')?;
    (!options.is_empty() && options.chars().all(|c| matches!(c, 'n' | 'e' | 'E')))
        .then_some(options)
}

#[cfg(unix)]
fn echo_short_options_bytes(arg: &[u8]) -> Option<&[u8]> {
    let options = arg.strip_prefix(b"-")?;
    (!options.is_empty()
        && options
            .iter()
            .all(|byte| matches!(byte, b'n' | b'e' | b'E')))
    .then_some(options)
}

fn echo_parse_args(args_vec: &[String], posix_mode: bool) -> (bool, bool, Vec<String>) {
    let mut no_newline = false;
    let mut escaped = false;
    let mut values = Vec::new();

    if posix_mode {
        // POSIXLY_CORRECT 模式：
        // 只有单独的 -n 作为第一个参数时才启用选项处理。
        if args_vec.first().is_some_and(|arg| arg == "-n") {
            no_newline = true;
            let mut parsing_options = true;
            for arg in args_vec.iter().skip(1) {
                if parsing_options && let Some(options) = echo_short_options(arg) {
                    if options.contains('n') {
                        no_newline = true;
                    }
                    continue;
                }
                values.push(arg.clone());
                parsing_options = false;
            }
        } else {
            values = args_vec.to_vec();
        }
        // POSIX 模式下始终启用转义
        escaped = true;
    } else {
        let mut parsing_options = true;
        for arg in args_vec {
            if parsing_options {
                if arg == "--" {
                    values.push(arg.clone());
                    parsing_options = false;
                    continue;
                }

                if let Some(rest) = echo_short_options(arg) {
                    for c in rest.chars() {
                        match c {
                            'n' => no_newline = true,
                            'e' => escaped = true,
                            'E' => escaped = false,
                            _ => unreachable!("filtered by chars().all"),
                        }
                    }
                    continue;
                }
            }

            values.push(arg.clone());
            parsing_options = false;
        }
    }

    if values.is_empty() {
        values.push(String::new());
    }

    (no_newline, escaped, values)
}

#[cfg(unix)]
fn echo_parse_args_bytes<'a>(
    args: &'a [OsString],
    posix_mode: bool,
) -> (bool, bool, Vec<&'a [u8]>) {
    let mut no_newline = false;
    let mut escaped = false;
    let mut values = Vec::new();

    if posix_mode {
        if args
            .first()
            .is_some_and(|arg| arg.as_os_str().as_bytes() == b"-n")
        {
            no_newline = true;
            let mut parsing_options = true;
            for arg in args.iter().skip(1) {
                let arg = arg.as_os_str().as_bytes();
                if parsing_options && let Some(options) = echo_short_options_bytes(arg) {
                    if options.contains(&b'n') {
                        no_newline = true;
                    }
                    continue;
                }
                values.push(arg);
                parsing_options = false;
            }
        } else {
            values.extend(args.iter().map(|arg| arg.as_os_str().as_bytes()));
        }
        escaped = true;
    } else {
        let mut parsing_options = true;
        for arg in args {
            let arg = arg.as_os_str().as_bytes();
            if parsing_options {
                if arg == b"--" {
                    values.push(arg);
                    parsing_options = false;
                    continue;
                }

                if let Some(options) = echo_short_options_bytes(arg) {
                    for byte in options {
                        match byte {
                            b'n' => no_newline = true,
                            b'e' => escaped = true,
                            b'E' => escaped = false,
                            _ => unreachable!("filtered by iter().all"),
                        }
                    }
                    continue;
                }
            }

            values.push(arg);
            parsing_options = false;
        }
    }

    if values.is_empty() {
        values.push(b"");
    }

    (no_newline, escaped, values)
}

fn echo_render_output(no_newline: bool, escaped: bool, free: &[String]) -> io::Result<EchoRender> {
    let free = free.iter().map(String::as_bytes).collect::<Vec<_>>();
    let rendered = echo_render_output_bytes(no_newline, escaped, &free)?;
    let text_end = rendered.output.len() - usize::from(rendered.trailing_newline);
    let text = String::from_utf8_lossy(&rendered.output[..text_end]).into_owned();

    Ok(EchoRender {
        text,
        trailing_newline: rendered.trailing_newline,
        terminated_early: rendered.terminated_early,
        classic_text: String::from_utf8_lossy(&rendered.output).into_owned(),
        classic_bytes: rendered.output,
    })
}

fn echo_render_output_bytes(
    no_newline: bool,
    escaped: bool,
    free: &[&[u8]],
) -> io::Result<EchoRenderBytes> {
    let mut output = Vec::new();
    let mut terminated_early = false;

    for (i, input) in free.iter().enumerate() {
        if i > 0 {
            output.write_all(b" ")?;
        }
        if escaped {
            if echo_print_escaped(input, &mut output)?.is_break() {
                terminated_early = true;
                break;
            }
        } else {
            output.write_all(input)?;
        }
    }

    let trailing_newline = !no_newline && !terminated_early;
    if trailing_newline {
        output.push(b'\n');
    }

    Ok(EchoRenderBytes {
        output,
        trailing_newline,
        terminated_early,
    })
}

fn echo_native_semantic_from_args_vec(
    args_vec: &[String],
    posix_mode: bool,
) -> CTResult<EchoSemantic> {
    let (no_newline, escaped, values) = echo_parse_args(args_vec, posix_mode);
    let rendered = echo_render_output(no_newline, escaped, &values)
        .map_err_context(|| "could not render echo output".to_string())?;

    Ok(EchoSemantic {
        inputs: values,
        text: rendered.text,
        trailing_newline: rendered.trailing_newline,
        escape_mode: if escaped {
            EchoEscapeMode::Interpreted
        } else {
            EchoEscapeMode::Literal
        },
        terminated_early: rendered.terminated_early,
        classic_text: rendered.classic_text,
        classic_bytes: rendered.classic_bytes,
    })
}

pub fn echo_native_semantic(args: impl ctcore::Args) -> CTResult<EchoSemantic> {
    let args_vec: Vec<String> = args
        .skip(1)
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    let posix_mode = std::env::var("POSIXLY_CORRECT").is_ok();

    echo_native_semantic_from_args_vec(&args_vec, posix_mode)
}

fn echo_help_text(usage_name: &str) -> String {
    if echo_uses_zh_cn_locale() {
        return format!(
            "\
用法：{usage_name} [短选项]... [字符串]...
　或：{usage_name} 长选项
将 <字符串> 回显到标准输出。

  -n             不输出末尾的换行符
  -e             启用对反斜杠转义字符的解释
  -E             禁用对反斜杠转义字符的解释（默认）
      --help        显示此帮助信息并退出
      --version     显示版本信息并退出

若 -e 生效，则以下序列可被识别：

  \\\\      反斜杠
  \\a      警报（BEL 响铃字符）
  \\b      退格
  \\c      不再产生新的输出
  \\e      转义字符
  \\f      换页
  \\n      换行
  \\r      回车
  \\t      水平制表符
  \\v      垂直制表符
  \\0NNN   值为 NNN（1 至 3 位八进制数）的字节
  \\xHH    值为 HH（1 至 2 位十六进制数）的字节

注意：您的 shell 可能内置了自己的 echo 程序版本，它通常会取代这里所描述的
版本。请查阅您的 shell 的文档以了解内置版本所支持的选项。

注意：建议使用 printf(1) 替代本命令，
因为它可以正常输出与选项相似的字符串。

GNU coreutils 在线帮助：<https://www.gnu.org/software/coreutils/>
请向 <http://translationproject.org/team/zh_CN.html> 报告任何翻译错误
完整文档 <https://www.gnu.org/software/coreutils/echo>
或者在本地使用：info '(coreutils) echo invocation'
"
        );
    }

    format!(
        "\
Usage: {usage_name} [SHORT-OPTION]... [STRING]...
  or:  {usage_name} LONG-OPTION
Echo the STRING(s) to standard output.

  -n             do not output the trailing newline
  -e             enable interpretation of backslash escapes
  -E             disable interpretation of backslash escapes (default)
      --help        display this help and exit
      --version     output version information and exit

If -e is in effect, the following sequences are recognized:

  \\\\      backslash
  \\a      alert (BEL)
  \\b      backspace
  \\c      produce no further output
  \\e      escape
  \\f      form feed
  \\n      new line
  \\r      carriage return
  \\t      horizontal tab
  \\v      vertical tab
  \\0NNN   byte with octal value NNN (1 to 3 digits)
  \\xHH    byte with hexadecimal value HH (1 to 2 digits)

NOTE: your shell may have its own version of echo, which usually supersedes
the version described here.  Please refer to your shell's documentation
for details about the options it supports.

NOTE: printf(1) is a preferred alternative,
which does not have issues outputting option-like strings.

GNU coreutils online help: <https://www.gnu.org/software/coreutils/>
Report any translation bugs to <https://translationproject.org/team/>
Full documentation <https://www.gnu.org/software/coreutils/echo>
or available locally via: info '(coreutils) echo invocation'
"
    )
}

fn echo_uses_zh_cn_locale() -> bool {
    if let Some(language) = std::env::var("LANGUAGE")
        .ok()
        .filter(|value| !value.is_empty())
    {
        return echo_locale_is_zh_cn(language.split(':').next().unwrap_or_default());
    }

    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
        .is_some_and(|locale| echo_locale_is_zh_cn(&locale))
}

fn echo_locale_is_zh_cn(locale: &str) -> bool {
    locale
        .split(['.', '@'])
        .next()
        .is_some_and(|name| name.eq_ignore_ascii_case("zh_CN"))
}

const GNU_ECHO_VERSION_TEXT: &str = "\
echo (GNU coreutils) 9.4
Copyright (C) 2023 Free Software Foundation, Inc.
License GPLv3+: GNU GPL version 3 or later <https://gnu.org/licenses/gpl.html>.
This is free software: you are free to change and redistribute it.
There is NO WARRANTY, to the extent permitted by law.

Written by Brian Fox and Chet Ramey.
";

const GNU_ECHO_VERSION_TEXT_ZH_CN: &str = "\
echo (GNU coreutils) 9.4
Copyright © 2023 Free Software Foundation, Inc.
许可证 GPLv3+：GNU 通用公共许可证第 3 版或更新版本 <https://gnu.org/licenses/gpl.html>。
本软件是自由软件：您可以自由修改和重新发布它。
在法律允许的范围内，不提供任何保证。

由 Brian Fox 和 Chet Ramey 编写。
";

fn echo_print_help_or_version(help: bool) -> io::Result<()> {
    let mut output = Vec::new();
    if help {
        output.extend_from_slice(echo_help_text(ctcore::ct_util_name()).as_bytes());
    } else {
        let version_text = if echo_uses_zh_cn_locale() {
            GNU_ECHO_VERSION_TEXT_ZH_CN
        } else {
            GNU_ECHO_VERSION_TEXT
        };
        output.extend_from_slice(version_text.as_bytes());
    }
    echo_write_output(&output)
}

#[cfg(unix)]
pub fn echo_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);

    #[cfg(all(target_os = "linux", not(test)))]
    if !inherited_sigpipe_is_ignored() {
        enable_pipe_errors().map_err(|error| CtSimpleError::new(1, error.to_string()))?;
    }

    let args_vec = args.skip(1).collect::<Vec<_>>();
    let posix_mode = std::env::var("POSIXLY_CORRECT").is_ok();

    if !posix_mode
        && matches!(args_vec.as_slice(), [arg] if matches!(arg.to_str(), Some("--help" | "--version")))
    {
        return echo_write_result(echo_print_help_or_version(
            args_vec[0].to_str() == Some("--help"),
        ));
    }

    let (no_newline, escaped, values) = echo_parse_args_bytes(&args_vec, posix_mode);

    echo_write_result(echo_execute_bytes(no_newline, escaped, &values))
}

#[cfg(not(unix))]
pub fn echo_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);

    let args_vec: Vec<String> = args
        .skip(1)
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    let posix_mode = std::env::var("POSIXLY_CORRECT").is_ok();

    if echo_is_help_or_version_request(&args_vec, posix_mode) {
        return echo_write_result(echo_print_help_or_version(args_vec[0] == "--help"));
    }

    let (no_newline, escaped, values) = echo_parse_args(&args_vec, posix_mode);
    echo_write_result(echo_execute(no_newline, escaped, &values))
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("echo.about");
    let usage_description = t!("echo.usage");

    let args = vec![
        Arg::new("help")
            .long("help")
            .help(t!("echo.clap.help"))
            .action(ArgAction::Help),
        Arg::new("version")
            .long("version")
            .help(t!("echo.clap.version"))
            .action(ArgAction::Version),
        Arg::new(opt_flags::NO_NEWLINE)
            .short('n')
            .help(t!("echo.clap.no_newline"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::ENABLE_BACKSLASH_ESCAPE)
            .short('e')
            .help(t!("echo.clap.enable_backslash_escape"))
            .action(ArgAction::SetTrue)
            .overrides_with(opt_flags::DISABLE_BACKSLASH_ESCAPE),
        Arg::new(opt_flags::DISABLE_BACKSLASH_ESCAPE)
            .short('E')
            .help(t!("echo.clap.disable_backslash_escape"))
            .action(ArgAction::SetTrue)
            .overrides_with(opt_flags::ENABLE_BACKSLASH_ESCAPE),
        Arg::new(opt_flags::STRING).action(ArgAction::Append),
    ];

    Command::new(utility_name)
        // TrailingVarArg指定最后一个位置参数是一个VarArg，并且它不会进一步尝试解析任何其他参数。
        .trailing_var_arg(true)
        .allow_hyphen_values(true)
        .version(command_version)
        .about(application_info)
        .after_help(t!("echo.after_help"))
        .override_usage(usage_description)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .args(&args)
}

#[cfg(any(not(unix), test))]
fn echo_execute(no_newline: bool, escaped: bool, free: &[String]) -> io::Result<()> {
    let rendered = echo_render_output(no_newline, escaped, free)?;
    echo_write_output(&rendered.classic_bytes)
}

#[cfg(unix)]
fn echo_execute_bytes(no_newline: bool, escaped: bool, free: &[&[u8]]) -> io::Result<()> {
    let rendered = echo_render_output_bytes(no_newline, escaped, free)?;
    echo_write_output(&rendered.output)
}

fn echo_write_output(output: &[u8]) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    if ctcore::ct_stdout_was_closed() {
        return Err(io::Error::from_raw_os_error(libc::EBADF));
    }

    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    stdout.write_all(output)
}

fn echo_write_result(result: io::Result<()>) -> CTResult<()> {
    match result {
        Ok(()) => Ok(()),
        Err(error) => Err(CtSimpleError::new(
            1,
            format!("write error: {}", strip_errno(&error)),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Echo;

        // Test name method
        assert_eq!(tool.name(), "echo");

        // Test command method
        let command = tool.command();
        assert!(command.get_name().contains("echo"));

        // Test execute method with simple args
        let args = vec![OsString::from("echo"), OsString::from("hello")];
        assert!(tool.execute(&args).is_ok());
    }

    #[test]
    fn test_echo_is_help_or_version_request() {
        assert!(echo_is_help_or_version_request(
            &["--help".to_string()],
            false
        ));
        assert!(!echo_is_help_or_version_request(&["-h".to_string()], false));
        assert!(echo_is_help_or_version_request(
            &["--version".to_string()],
            false
        ));
        assert!(!echo_is_help_or_version_request(&["-V".to_string()], false));
        assert!(!echo_is_help_or_version_request(
            &["--help".to_string()],
            true
        ));
        assert!(!echo_is_help_or_version_request(
            &["--help".to_string(), "extra".to_string()],
            false
        ));
        assert!(!echo_is_help_or_version_request(&["--".to_string()], false));
    }

    #[test]
    fn test_echo_parse_args_rejects_invalid_short_cluster() {
        let (no_newline, escaped, values) =
            echo_parse_args(&["-nex".to_string(), "foo".to_string()], false);

        assert!(!no_newline);
        assert!(!escaped);
        assert_eq!(values, vec!["-nex".to_string(), "foo".to_string()]);
    }

    #[test]
    fn test_echo_parse_args_accepts_valid_short_cluster() {
        let (no_newline, escaped, values) =
            echo_parse_args(&["-En".to_string(), "foo".to_string()], false);

        assert!(no_newline);
        assert!(!escaped);
        assert_eq!(values, vec!["foo".to_string()]);
    }

    mod tests_echo_main {
        use crate::echo_main;

        use std::fs;
        use std::fs::File;
        use tempfile::Builder;

        use std::ffi::OsString;
        use std::io::Write;
        #[test]
        fn test_echo_main_version() {
            let args = [ctcore::ct_util_name(), "--version"];

            let result = echo_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_echo_main_v() {
            let args = [ctcore::ct_util_name(), "-V"];
            let result = echo_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_echo_main_h() {
            let args = [ctcore::ct_util_name(), "-h"];
            let result = echo_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_echo_main_n() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "-n", "12345", ">", filename1];
            let result = echo_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_echo_main_e() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "-e", "12345", ">", filename1];
            let result = echo_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_echo_main_ee() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_main_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            let mut file = File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let content = "Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n\
                   Hello world Rust Cut command.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "-E", "12345", ">", filename1];
            let result = echo_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }
    }

    mod tests_ct_app {
        use crate::ct_app;

        use clap::error::ErrorKind;

        #[test]
        fn test_echo_app_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_echo_app_v() {
            let args = vec![ctcore::ct_util_name(), "-V"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_echo_app_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_echo_app_h() {
            let args = vec![ctcore::ct_util_name(), "-h"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_echo_app_n() {
            let args = vec![ctcore::ct_util_name(), "-n"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_echo_app_e() {
            let args = vec![ctcore::ct_util_name(), "-e"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_echo_app_ee() {
            let args = vec![ctcore::ct_util_name(), "-E"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }
    }

    mod tests_echo_functions {
        use crate::{
            EchoEscapeMode, echo_execute, echo_native_semantic_from_args_vec, echo_print_escaped,
            echo_render_output,
        };
        use std::io::Cursor;
        use std::ops::ControlFlow;

        #[test]
        fn test_echo_execute() {
            echo_execute(false, true, &["hello".to_string(), "world".to_string()])
                .expect("echo_execute failed");

            echo_execute(true, false, &["hello".to_string(), "world".to_string()])
                .expect("echo_execute failed");

            echo_execute(true, true, &["hello".to_string(), "world".to_string()])
                .expect("echo_execute failed");
        }

        #[test]
        fn test_echo_print_escaped_continue() {
            let mut output = Cursor::new(Vec::new());
            let result = echo_print_escaped(b"\\n\\t\\x41", &mut output);

            // println!("{:#?}", result.unwrap());/**/
            assert_eq!(
                result.unwrap(),
                (Ok(ControlFlow::Continue(())) as Result<_, ()>).expect("REASON")
            );
            assert_eq!(output.into_inner(), b"\n\tA");
        }

        #[test]
        fn test_echo_print_escaped_break() {
            let mut output = Cursor::new(Vec::new());
            let result = echo_print_escaped(b"\\c", &mut output);

            // println!("{:#?}", result.unwrap());/**/
            assert_eq!(
                result.unwrap(),
                (Ok(ControlFlow::Break(())) as Result<_, ()>).expect("REASON")
            );
        }

        #[test]
        fn test_echo_render_output_preserves_space_before_early_termination() {
            let rendered = echo_render_output(
                false,
                true,
                &["a".to_string(), "\\c".to_string(), "tail".to_string()],
            )
            .expect("rendered");

            assert_eq!(rendered.text, "a ");
            assert!(!rendered.trailing_newline);
            assert!(rendered.terminated_early);
            assert_eq!(rendered.classic_text, "a ");
        }

        #[test]
        fn test_echo_native_semantic_reports_default_contract() {
            let semantic = echo_native_semantic_from_args_vec(
                &["hello".to_string(), "world".to_string()],
                false,
            )
            .expect("semantic");

            assert_eq!(
                semantic.inputs,
                vec!["hello".to_string(), "world".to_string()]
            );
            assert_eq!(semantic.text, "hello world");
            assert!(semantic.trailing_newline);
            assert_eq!(semantic.escape_mode, EchoEscapeMode::Literal);
            assert!(!semantic.terminated_early);
            assert_eq!(semantic.classic_text, "hello world\n");
            assert_eq!(semantic.classic_bytes, b"hello world\n");
        }
    }
}
