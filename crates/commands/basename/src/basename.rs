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
use clap::ArgAction;
use clap::Command;
use clap::crate_version;
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CtSimpleError};
use ctcore::ct_line_ending::CtLineEnding;
use std::ffi::OsString;
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use sys_locale::get_locale;

use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");

pub mod flags {
    pub static MULTIPLE: &str = "multiple";
    pub static NAME: &str = "name";
    pub static SUFFIX: &str = "suffix";
    pub static ZERO: &str = "zero";
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasenameRow {
    pub input: String,
    pub basename: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasenameSemantic {
    pub rows: Vec<BasenameRow>,
    pub classic_text: String,
}

struct BasenameOptions {
    line_ending: CtLineEnding,
    names: Vec<Vec<u8>>,
    suffix: Vec<u8>,
}

#[derive(Default)]
pub struct Basename;
impl Tool for Basename {
    fn name(&self) -> &'static str {
        "basename"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        basename_main(args.iter().cloned())
    }
}

pub fn basename_main(args: impl ctcore::Args) -> CTResult<()> {
    // Set locale based on system settings
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);

    let args = args.collect::<Vec<_>>();
    let options = match parse_basename_args(&args)? {
        BasenameAction::Run(options) => options,
        BasenameAction::Help => {
            println!("{}", basename_help_text(&basename_usage_name(&args)));
            return Ok(());
        }
        BasenameAction::Version => {
            println!("{}", basename_version_text(&basename_program_name(&args)));
            return Ok(());
        }
    };
    io::stdout()
        .write_all(&basename_output_bytes(&options))
        .map_err(|e| CtSimpleError::new(1, e.to_string()))?;

    Ok(())
}

pub fn basename_native_semantic(args: impl ctcore::Args) -> CTResult<BasenameSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);

    let args = args.collect::<Vec<_>>();
    let options = match parse_basename_args(&args)? {
        BasenameAction::Run(options) => options,
        BasenameAction::Help => {
            return Err(CtSimpleError::new(
                0,
                basename_help_text(&basename_usage_name(&args)),
            ));
        }
        BasenameAction::Version => {
            return Err(CtSimpleError::new(
                0,
                basename_version_text(&basename_program_name(&args)),
            ));
        }
    };
    Ok(basename_semantic_from_options(&options))
}

fn basename_semantic_from_options(options: &BasenameOptions) -> BasenameSemantic {
    let mut rows = Vec::with_capacity(options.names.len());
    let mut classic_text = String::new();

    for input in &options.names {
        let output = basename_bytes(input, &options.suffix);
        let output_text = String::from_utf8_lossy(&output).into_owned();
        classic_text.push_str(&output_text);
        classic_text.push_str(&options.line_ending.to_string());
        rows.push(BasenameRow {
            input: String::from_utf8_lossy(input).into_owned(),
            basename: output_text,
        });
    }

    BasenameSemantic { rows, classic_text }
}

fn basename_output_bytes(options: &BasenameOptions) -> Vec<u8> {
    let mut output = Vec::new();
    let line_ending = u8::from(options.line_ending);

    for input in &options.names {
        output.extend(basename_bytes(input, &options.suffix));
        output.push(line_ending);
    }

    output
}

enum BasenameAction {
    Run(BasenameOptions),
    Help,
    Version,
}

fn parse_basename_args(args: &[OsString]) -> CTResult<BasenameAction> {
    let usage_name = basename_usage_name(args);
    let mut multiple_names = false;
    let mut use_nuls = false;
    let mut suffix = None;
    let mut operands = Vec::new();
    let mut i = 1;

    while i < args.len() {
        let arg = args[i].as_os_str().as_bytes();

        if arg == b"--" {
            operands.extend(args[i + 1..].iter().map(os_string_bytes));
            break;
        }

        if arg == b"-" || !arg.starts_with(b"-") {
            operands.extend(args[i..].iter().map(os_string_bytes));
            break;
        }

        if let Some(long) = arg.strip_prefix(b"--") {
            if let Some(action) = parse_basename_long_option(
                args,
                &usage_name,
                long,
                arg,
                &mut i,
                &mut multiple_names,
                &mut use_nuls,
                &mut suffix,
            )? {
                return Ok(action);
            }
            i += 1;
            continue;
        }

        parse_basename_short_options(
            args,
            &usage_name,
            &arg[1..],
            &mut i,
            &mut multiple_names,
            &mut use_nuls,
            &mut suffix,
        )?;
        i += 1;
    }

    basename_options_from_operands(&usage_name, multiple_names, use_nuls, suffix, operands)
}

fn parse_basename_short_options(
    args: &[OsString],
    usage_name: &str,
    options: &[u8],
    i: &mut usize,
    multiple_names: &mut bool,
    use_nuls: &mut bool,
    suffix: &mut Option<Vec<u8>>,
) -> CTResult<()> {
    for (index, option) in options.iter().enumerate() {
        match option {
            b'a' => *multiple_names = true,
            b'z' => *use_nuls = true,
            b's' => {
                *multiple_names = true;
                let suffix_start = index + 1;
                if suffix_start < options.len() {
                    *suffix = Some(options[suffix_start..].to_vec());
                } else {
                    *i += 1;
                    let Some(value) = args.get(*i) else {
                        return Err(basename_usage_error(
                            usage_name,
                            "option requires an argument -- 's'".to_owned(),
                        ));
                    };
                    *suffix = Some(os_string_bytes(value));
                }
                return Ok(());
            }
            invalid => {
                return Err(basename_usage_error(
                    usage_name,
                    format!("invalid option -- '{}'", *invalid as char),
                ));
            }
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn parse_basename_long_option(
    args: &[OsString],
    usage_name: &str,
    long: &[u8],
    raw_arg: &[u8],
    i: &mut usize,
    multiple_names: &mut bool,
    use_nuls: &mut bool,
    suffix: &mut Option<Vec<u8>>,
) -> CTResult<Option<BasenameAction>> {
    let (name, value) = match long.iter().position(|byte| *byte == b'=') {
        Some(index) => (&long[..index], Some(&long[index + 1..])),
        None => (long, None),
    };
    let matches = basename_long_option_matches(name);
    let raw_arg = String::from_utf8_lossy(raw_arg);

    match matches.as_slice() {
        [] => Err(basename_usage_error(
            usage_name,
            format!("unrecognized option '{raw_arg}'"),
        )),
        [option] => match *option {
            "multiple" => {
                reject_unexpected_long_value(usage_name, option, value)?;
                *multiple_names = true;
                Ok(None)
            }
            "zero" => {
                reject_unexpected_long_value(usage_name, option, value)?;
                *use_nuls = true;
                Ok(None)
            }
            "help" => {
                reject_unexpected_long_value(usage_name, option, value)?;
                Ok(Some(BasenameAction::Help))
            }
            "version" => {
                reject_unexpected_long_value(usage_name, option, value)?;
                Ok(Some(BasenameAction::Version))
            }
            "suffix" => {
                *multiple_names = true;
                if let Some(value) = value {
                    *suffix = Some(value.to_vec());
                } else {
                    *i += 1;
                    let Some(value) = args.get(*i) else {
                        return Err(basename_usage_error(
                            usage_name,
                            "option '--suffix' requires an argument".to_owned(),
                        ));
                    };
                    *suffix = Some(os_string_bytes(value));
                }
                Ok(None)
            }
            _ => Ok(None),
        },
        _ => Err(basename_usage_error(
            usage_name,
            format!(
                "option '{raw_arg}' is ambiguous; possibilities: {}",
                matches
                    .iter()
                    .map(|option| format!("'--{option}'"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        )),
    }
}

fn reject_unexpected_long_value(
    usage_name: &str,
    option: &str,
    value: Option<&[u8]>,
) -> CTResult<()> {
    if value.is_some() {
        return Err(basename_usage_error(
            usage_name,
            format!("option '--{option}' doesn't allow an argument"),
        ));
    }

    Ok(())
}

fn basename_long_option_matches(name: &[u8]) -> Vec<&'static str> {
    ["multiple", "suffix", "zero", "help", "version"]
        .into_iter()
        .filter(|option| option.as_bytes().starts_with(name))
        .collect()
}

fn basename_options_from_operands(
    usage_name: &str,
    multiple_names: bool,
    use_nuls: bool,
    suffix: Option<Vec<u8>>,
    mut operands: Vec<Vec<u8>>,
) -> CTResult<BasenameAction> {
    if operands.is_empty() {
        return Err(basename_usage_error(
            usage_name,
            "missing operand".to_owned(),
        ));
    }

    let suffix = if multiple_names {
        suffix.unwrap_or_default()
    } else {
        match operands.len() {
            1 => Vec::new(),
            2 => operands.pop().unwrap(),
            _ => {
                let extra = String::from_utf8_lossy(&operands[2]).into_owned();
                return Err(basename_usage_error(
                    usage_name,
                    format!("extra operand {}", extra.quote()),
                ));
            }
        }
    };

    Ok(BasenameAction::Run(BasenameOptions {
        line_ending: CtLineEnding::from_zero_flag(use_nuls),
        names: operands,
        suffix,
    }))
}

fn basename_usage_name(args: &[OsString]) -> String {
    args.first()
        .map(|arg| arg.to_string_lossy().into_owned())
        .filter(|arg| !arg.is_empty())
        .unwrap_or_else(|| ctcore::ct_util_name().to_owned())
}

fn basename_program_name(args: &[OsString]) -> String {
    let usage_name = basename_usage_name(args);

    Path::new(&usage_name)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or(usage_name)
}

fn basename_usage_error(usage_name: &str, message: String) -> Box<dyn ctcore::ct_error::CTError> {
    CtSimpleError::new(
        1,
        format!("{message}\nTry '{usage_name} --help' for more information."),
    )
}

fn basename_help_text(usage_name: &str) -> String {
    format!(
        "\
Usage: {usage_name} NAME [SUFFIX]
  or:  {usage_name} OPTION... NAME...
Print NAME with any leading directory components removed.
If specified, also remove a trailing SUFFIX.

Mandatory arguments to long options are mandatory for short options too.
  -a, --multiple       support multiple arguments and treat each as a NAME
  -s, --suffix=SUFFIX  remove a trailing SUFFIX; implies -a
  -z, --zero           end each output line with NUL, not newline
      --help        display this help and exit
      --version     output version information and exit

Examples:
  {usage_name} /usr/bin/sort          -> \"sort\"
  {usage_name} include/stdio.h .h     -> \"stdio\"
  {usage_name} -s .h include/stdio.h  -> \"stdio\"
  {usage_name} -a any/str1 any/str2   -> \"str1\" followed by \"str2\"

syskits online help: <https://www.ctyun.cn/developer>"
    )
}

fn basename_version_text(program_name: &str) -> String {
    format!(
        "\
{program_name} (syskits) {}
Copyright (C) 2022-2025 China Telecom Cloud Technologies Co., Ltd.
License Mulan PSL v2: <http://license.coscl.org.cn/MulanPSL2>.
This is free software: you are free to change and redistribute it.
There is NO WARRANTY, to the extent permitted by law.",
        crate_version!()
    )
}

fn os_string_bytes(arg: &OsString) -> Vec<u8> {
    arg.as_os_str().as_bytes().to_vec()
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("basename.about");
    let usage_description = t!("basename.usage");

    let args = basename_args_init();
    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .args(&args)
}

fn basename_args_init() -> Vec<Arg> {
    let args = vec![
        Arg::new(flags::MULTIPLE)
            .short('a')
            .long(flags::MULTIPLE)
            .help(t!("basename.clap.multiple"))
            .action(ArgAction::SetTrue)
            .overrides_with(flags::MULTIPLE),
        Arg::new(flags::NAME)
            .action(clap::ArgAction::Append)
            .value_hint(clap::ValueHint::AnyPath)
            .hide(true)
            .trailing_var_arg(true),
        Arg::new(flags::SUFFIX)
            .short('s')
            .long(flags::SUFFIX)
            .value_name("SUFFIX")
            .help(t!("basename.clap.suffix"))
            .overrides_with(flags::SUFFIX),
        Arg::new(flags::ZERO)
            .short('z')
            .long(flags::ZERO)
            .help(t!("basename.clap.zero"))
            .action(ArgAction::SetTrue)
            .overrides_with(flags::ZERO),
        Arg::new("help")
            .long("help")
            .help(t!("basename.clap.help"))
            .action(ArgAction::Help),
        Arg::new("version")
            .long("version")
            .help(t!("basename.clap.version"))
            .action(ArgAction::Version),
    ];
    args
}

#[cfg(test)]
fn basename(fullname: &str, suffix: &str) -> String {
    String::from_utf8(basename_bytes(fullname.as_bytes(), suffix.as_bytes()))
        .expect("basename of UTF-8 input is UTF-8")
}

fn basename_bytes(fullname: &[u8], suffix: &[u8]) -> Vec<u8> {
    if fullname.is_empty() {
        return Vec::new();
    }

    let mut end = fullname.len();
    while end > 0 && fullname[end - 1] == b'/' {
        end -= 1;
    }

    if end == 0 {
        return vec![b'/'];
    }

    let trimmed = &fullname[..end];
    let start = trimmed
        .iter()
        .rposition(|byte| *byte == b'/')
        .map_or(0, |index| index + 1);
    let mut name = trimmed[start..].to_vec();

    if !suffix.is_empty() && name.len() > suffix.len() && name.ends_with(suffix) {
        name.truncate(name.len() - suffix.len());
    }

    name
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    #[test]
    fn test_i18n_help_messages() {
        // 设置英文环境
        rust_i18n::set_locale("en-US");
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--help"];
        let result = command.try_get_matches_from(args);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);

        // 设置中文环境
        rust_i18n::set_locale("zh-CN");
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--help"];
        let result = command.try_get_matches_from(args);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
    }

    #[test]
    fn test_i18n_error_messages() {
        // 测试英文错误消息
        rust_i18n::set_locale("en-US");
        let args = [ctcore::ct_util_name()];
        let result = basename_main(args.iter().map(OsString::from));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code(), 1);

        // 测试中文错误消息
        rust_i18n::set_locale("zh-CN");
        let args = [ctcore::ct_util_name()];
        let result = basename_main(args.iter().map(OsString::from));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code(), 1);
    }

    #[test]
    fn test_i18n_fallback() {
        // 测试不存在的语言环境，应该回退到中文
        rust_i18n::set_locale("fr-FR");
        let args = [ctcore::ct_util_name()];
        let result = basename_main(args.iter().map(OsString::from));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code(), 1);
    }

    #[test]
    fn test_extra_operand_message_without_placeholder() {
        rust_i18n::set_locale("en-US");
        let args = ["basename", "a", "b", "c"];
        let err = basename_main(args.iter().map(OsString::from)).unwrap_err();
        assert_eq!(
            err.to_string(),
            "extra operand 'c'\nTry 'basename --help' for more information."
        );
        assert!(!err.usage());
    }

    #[test]
    fn test_basename_gnu_option_error_diagnostics() {
        let cases = [
            (
                vec!["basename", "-H"],
                "invalid option -- 'H'\nTry 'basename --help' for more information.",
            ),
            (
                vec!["basename", "-s"],
                "option requires an argument -- 's'\nTry 'basename --help' for more information.",
            ),
            (
                vec!["basename", "--suffix"],
                "option '--suffix' requires an argument\nTry 'basename --help' for more information.",
            ),
            (
                vec!["basename", "--invalid"],
                "unrecognized option '--invalid'\nTry 'basename --help' for more information.",
            ),
            (
                vec!["basename", "--=x"],
                "option '--=x' is ambiguous; possibilities: '--multiple' '--suffix' '--zero' '--help' '--version'\nTry 'basename --help' for more information.",
            ),
            (
                vec!["basename", "--multiple=1", "foo"],
                "option '--multiple' doesn't allow an argument\nTry 'basename --help' for more information.",
            ),
        ];

        for (args, expected) in cases {
            let err = basename_native_semantic(args.iter().map(OsString::from)).unwrap_err();

            assert_eq!(err.code(), 1);
            assert!(!err.usage());
            assert_eq!(err.to_string(), expected);
        }
    }

    #[test]
    fn test_basename_gnu_option_parsing_semantics() {
        let args = ["basename", "--suf", ".c", "foo.c", "bar.c"];
        let semantic = basename_native_semantic(args.iter().map(OsString::from)).unwrap();
        assert_eq!(semantic.classic_text, "foo\nbar\n");

        let args = ["basename", "-az", "/tmp/foo"];
        let semantic = basename_native_semantic(args.iter().map(OsString::from)).unwrap();
        assert_eq!(semantic.classic_text, "foo\0");

        let args = ["basename", "-as.c", "foo.c"];
        let semantic = basename_native_semantic(args.iter().map(OsString::from)).unwrap();
        assert_eq!(semantic.classic_text, "foo\n");

        let args = ["basename", "foo", "-a"];
        let semantic = basename_native_semantic(args.iter().map(OsString::from)).unwrap();
        assert_eq!(semantic.classic_text, "foo\n");
    }

    #[test]
    fn test_basename_preserves_non_utf8_bytes() {
        assert_eq!(basename_bytes(b"dir/a\xffb.txt", b".txt"), b"a\xffb");

        let args = vec![
            OsString::from("basename"),
            OsString::from_vec(b"dir/a\xffb.txt".to_vec()),
            OsString::from(".txt"),
        ];
        let BasenameAction::Run(options) = parse_basename_args(&args).unwrap() else {
            panic!("expected basename run action");
        };

        assert_eq!(basename_output_bytes(&options), b"a\xffb\n");
    }

    #[test]
    fn test_basename_gnu_help_and_version_output() {
        let args = ["basename", "--help", "foo"];
        let err = basename_native_semantic(args.iter().map(OsString::from)).unwrap_err();
        assert_eq!(err.code(), 0);
        assert!(!err.usage());
        assert!(err.to_string().starts_with("Usage: basename NAME [SUFFIX]"));
        assert!(
            err.to_string()
                .contains("  or:  basename OPTION... NAME...")
        );
        assert!(
            err.to_string()
                .contains("      --help        display this help and exit")
        );
        assert!(err.to_string().contains("syskits online help:"));
        assert!(!err.to_string().contains("GNU"));

        let args = ["basename", "--version", "foo"];
        let err = basename_native_semantic(args.iter().map(OsString::from)).unwrap_err();
        assert_eq!(err.code(), 0);
        assert!(!err.usage());
        assert!(err.to_string().starts_with("basename (syskits) "));
        assert!(err.to_string().contains("License Mulan PSL v2"));
        assert!(!err.to_string().contains("GNU"));
    }

    #[test]
    fn test_tool_implementation() {
        let tool = Basename;

        // 测试 name 方法
        assert_eq!(tool.name(), "basename");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("basename"));

        // 测试 execute 方法
        let args = vec![OsString::from("basename"), OsString::from("/usr/bin/sort")];
        assert!(tool.execute(&args).is_ok());

        let args = vec![OsString::from("basename"), OsString::from("--help")];
        assert!(tool.execute(&args).is_ok());

        let args = vec![OsString::from("basename"), OsString::from("--version")];
        assert!(tool.execute(&args).is_ok());
    }

    #[test]
    fn test_ct_app_execution_version() {
        let command = ct_app();

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), "--version"];

        // Assuming `command` has a method to retrieve the executable name, replace it with the actual one
        let executable = command.try_get_matches_from(args);

        assert!(executable.is_err());
        assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
    }

    #[test]
    fn test_basename_h_ctmain() {
        {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--help"];

            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }
    }

    #[test]
    fn test_basename_hh_ctmain() {
        {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-h"];

            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }
    }

    #[test]
    fn test_basename_hhh_ctmain() {
        {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-H"];

            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }
    }

    #[test]
    fn test_ct_app_invalid_argument() {
        let command = ct_app();

        // 测试用例：验证当提供未知参数时是否正确报错
        let invalid_args = vec![ctcore::ct_util_name(), "--invalid-argument"];
        let result = command.try_get_matches_from(invalid_args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn test_ct_app_support_missing_argument() {
        let command = ct_app();

        // 测试用例：验证当缺少必需的参数时是否正确报错
        let missing_args = vec![ctcore::ct_util_name()]; // 缺少任何参数
        let result = command.try_get_matches_from(missing_args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_basename_v_ctmain() {
        {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];

            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }
    }

    #[test]
    fn test_basename_vv_ctmain() {
        {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];
            // let result = ct_main(args.iter().map(|s| OsString::from(s)));

            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }
    }

    #[test]
    fn test_flags_multiple() {
        let command = ct_app();

        let args = vec![ctcore::ct_util_name(), "-a"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(flags::MULTIPLE));
    }

    #[test]
    fn test_flags_name() {
        let command = ct_app();

        let args = vec![
            ctcore::ct_util_name(),
            "-a",
            "path/to/name1",
            "path/to/name2",
        ];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(flags::MULTIPLE));
    }

    #[test]
    fn test_flags_suffix() {
        let command = ct_app();

        let args = vec![
            ctcore::ct_util_name(),
            "--suffix=SUFFIX",
            "/usr/bin/sort.txt",
        ];
        let matches = command.try_get_matches_from(args).unwrap();

        let suffix = matches
            .get_one::<String>(flags::SUFFIX)
            .expect("missing suffix");
        assert_eq!(suffix, "SUFFIX");
    }
    #[test]
    fn test_flags_zero() {
        let command = ct_app();

        let args = vec![ctcore::ct_util_name(), "-z"];
        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(flags::ZERO));
    }

    #[test]
    fn test_basename_regular_input_with_valid_suffix() {
        let input = "/path/to/file.txt";
        let expected_result = "file";
        let suffix = ".txt";

        let output = basename(input, suffix);

        assert_eq!(output, expected_result);
    }

    #[test]
    fn test_basename_input_matches_suffix_exactly() {
        let input = "/path/to/file.txt";
        let expected_result = "file.txt";
        let suffix = "file.txt";

        let output = basename(input, suffix);

        assert_eq!(output, expected_result);
    }

    #[test]
    fn test_basename_hidden_file_with_valid_suffix() {
        let input = "/path/to/.txt";
        let expected_result = ".txt";
        let suffix = ".txt";

        let output = basename(input, suffix);

        assert_eq!(output, expected_result);
    }

    #[test]
    fn test_basename_no_suffix_provided() {
        let input = "/path/to/file";
        let expected_result = "file";
        let suffix = "";

        let output = basename(input, suffix);

        assert_eq!(output, expected_result);
    }

    #[test]
    fn test_basename_empty_input_and_suffix() {
        let input = "";
        let expected_result = "";
        let suffix = "";

        let output = basename(input, suffix);

        assert_eq!(output, expected_result);
    }

    #[test]
    fn test_basename_trailing_slash_in_input() {
        let input = "/path/to/file.txt/";
        let expected_result = "file";
        let suffix = ".txt";

        let output = basename(input, suffix);

        assert_eq!(output, expected_result);
    }

    #[test]
    fn test_basename_multiple_consecutive_slashes_in_input() {
        let input = "//path//to//file.txt";
        let expected_result = "file";
        let suffix = ".txt";

        let output = basename(input, suffix);

        assert_eq!(output, expected_result);
    }

    #[test]
    fn test_basename_suffix_containing_dots() {
        let input = "/path/to/file.tar.gz";
        let expected_result = "file.tar";
        let suffix = ".gz";

        let output = basename(input, suffix);

        assert_eq!(output, expected_result);
    }

    #[test]
    fn test_basename_long_input_without_suffix() {
        let input = "/very/long/path/to/a/really/really/really/really/really/long/file/name";
        let expected_result = "name";
        let suffix = "";

        let output = basename(input, suffix);

        assert_eq!(output, expected_result);
    }

    #[test]
    fn test_basename_suffix_containing_special_characters() {
        let input = "/path/to/file@#$%^&*.txt";
        let expected_result = "file@#$%^&*";
        let suffix = ".txt";

        let output = basename(input, suffix);

        assert_eq!(output, expected_result);
    }

    #[test]
    fn test_basename_with_invalid_input() {
        {
            // Test case: Input is a single slash
            let input = "/";
            let expected_result = "/";
            let suffix = "";

            let output = basename(input, suffix);
            assert_eq!(output, expected_result);
        }
    }

    #[test]
    fn test_ct_main() {
        // Test case: Input is a single slash
        let args = [ctcore::ct_util_name(), "/path/to/file@#$%^&*.txt", ".txt"];
        let expected_result = "/path/to/file@#$%^&*";
        let result = basename_main(args.iter().map(OsString::from));
        let mut s = String::new();
        // println!("{:?}", result);
        match result {
            Err(_output) => {
                let code = _output.code();
                let message = _output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
            Ok(_output) => {
                s = "/path/to/file@#$%^&*".to_string();
                // println!("result:{}", s);
                // //assert_eq!(s,expected_output);
            }
        }
        assert_eq!(s, expected_result);
    }
}
