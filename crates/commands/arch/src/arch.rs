/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL v2.
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2.
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

extern crate rust_i18n;

// 声明 i18n 宏和初始化函数
rust_i18n::i18n!("locales", fallback = "en-US");
use sys_locale::get_locale;

use clap::Arg;
use clap::ArgAction;
use clap::Command;
use clap::crate_version;
use platform_info::*;

use ctcore::Tool;
use ctcore::ct_display::locale_quote;
use ctcore::ct_error::CTResult;
use ctcore::ct_error::CtSimpleError;
use rust_i18n::t;
use std::ffi::OsString;
use std::path::Path;

#[derive(Default)]
pub struct Arch;
impl Tool for Arch {
    fn name(&self) -> &'static str {
        "arch"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        let result = arch_main(args.iter().cloned());
        match result {
            Ok(s) => {
                println!("{s}");
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

pub fn arch_main(args: impl ctcore::Args) -> CTResult<String> {
    // 设置语言
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);

    let args = args.collect::<Vec<_>>();
    match parse_arch_args(&args)? {
        ArchAction::Help => return Ok(arch_help_text(&arch_usage_name(&args))),
        ArchAction::Version => return Ok(arch_version_text(&arch_program_name(&args))),
        ArchAction::PrintMachine => {}
    }

    let platform_info =
        PlatformInfo::new().map_err(|_e| CtSimpleError::new(1, "cannot get system name"))?;

    let binding = platform_info.machine().to_string_lossy();
    let s = binding.trim();

    Ok(s.to_string())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ArchAction {
    PrintMachine,
    Help,
    Version,
}

fn parse_arch_args(args: &[OsString]) -> CTResult<ArchAction> {
    let usage_name = arch_usage_name(args);
    let mut iter = args.iter().skip(1);

    while let Some(arg) = iter.next() {
        let arg = arg.to_string_lossy();

        if arg == "--" {
            if let Some(extra) = iter.next() {
                return Err(extra_operand_error(&usage_name, &extra.to_string_lossy()));
            }
            return Ok(ArchAction::PrintMachine);
        }

        if let Some(long) = arg.strip_prefix("--") {
            if let Some((name, _value)) = long.split_once('=') {
                return validate_long_option_value(&usage_name, &arg, name);
            }

            if let Some(option) = unique_arch_long_option(long) {
                return Ok(match option {
                    "help" => ArchAction::Help,
                    "version" => ArchAction::Version,
                    _ => ArchAction::PrintMachine,
                });
            }

            return Err(arch_usage_error(
                &usage_name,
                format!("unrecognized option '{arg}'"),
            ));
        }

        if let Some(short) = arg.strip_prefix('-')
            && !short.is_empty()
        {
            let invalid = short.chars().next().unwrap();
            return Err(arch_usage_error(
                &usage_name,
                format!("invalid option -- '{invalid}'"),
            ));
        }

        return Err(extra_operand_error(&usage_name, &arg));
    }

    Ok(ArchAction::PrintMachine)
}

fn validate_long_option_value(usage_name: &str, arg: &str, name: &str) -> CTResult<ArchAction> {
    let matches = arch_long_option_matches(name);

    match matches.as_slice() {
        [option] => Err(arch_usage_error(
            usage_name,
            format!("option '--{option}' doesn't allow an argument"),
        )),
        [] => Err(arch_usage_error(
            usage_name,
            format!("unrecognized option '{arg}'"),
        )),
        _ => Err(arch_usage_error(
            usage_name,
            format!("option '{arg}' is ambiguous; possibilities: '--help' '--version'"),
        )),
    }
}

fn unique_arch_long_option(name: &str) -> Option<&'static str> {
    let matches = arch_long_option_matches(name);

    match matches.as_slice() {
        [option] => Some(option),
        _ => None,
    }
}

fn arch_long_option_matches(name: &str) -> Vec<&'static str> {
    ["help", "version"]
        .into_iter()
        .filter(|option| option.starts_with(name))
        .collect()
}

fn extra_operand_error(usage_name: &str, operand: &str) -> Box<dyn ctcore::ct_error::CTError> {
    arch_usage_error(
        usage_name,
        format!("extra operand {}", locale_quote(operand)),
    )
}

fn arch_usage_error(usage_name: &str, message: String) -> Box<dyn ctcore::ct_error::CTError> {
    CtSimpleError::new(
        1,
        format!("{message}\nTry '{usage_name} --help' for more information."),
    )
}

fn arch_usage_name(args: &[OsString]) -> String {
    args.first()
        .map(|arg| arg.to_string_lossy().into_owned())
        .filter(|arg| !arg.is_empty())
        .unwrap_or_else(|| ctcore::ct_util_name().to_owned())
}

fn arch_program_name(args: &[OsString]) -> String {
    let usage_name = arch_usage_name(args);

    Path::new(&usage_name)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or(usage_name)
}

fn arch_help_text(usage_name: &str) -> String {
    format!(
        "\
Usage: {usage_name} [OPTION]...
Print machine architecture.

      --help        display this help and exit
      --version     output version information and exit

syskits online help: <https://www.ctyun.cn/developer>"
    )
}

fn arch_version_text(program_name: &str) -> String {
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

pub fn ct_app() -> Command {
    let util_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("arch.about");
    let usage_description = t!("arch.usage");

    Command::new(util_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .arg(
            Arg::new("help")
                .long("help")
                .help(t!("arch.clap.help"))
                .action(ArgAction::Help),
        )
        .arg(
            Arg::new("version")
                .long("version")
                .help(t!("arch.clap.version"))
                .action(ArgAction::Version),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Arch;
        assert_eq!(tool.name(), "arch");

        let command = tool.command();
        assert!(command.get_name().contains("arch"));

        let args = vec![OsString::from("arch")];
        let result = tool.execute(&args);
        assert!(result.is_ok());

        let args = vec![OsString::from("arch"), OsString::from("--help")];
        let result = tool.execute(&args);
        assert!(result.is_ok());

        let args = vec![OsString::from("arch"), OsString::from("--version")];
        let result = tool.execute(&args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_command_line_args() {
        let tool = Arch;

        let args = vec![OsString::from("arch"), OsString::from("--help")];
        assert!(tool.execute(&args).is_ok());

        let args = vec![OsString::from("arch"), OsString::from("--version")];
        assert!(tool.execute(&args).is_ok());
    }

    #[test]
    fn test_arch_hh_ctmain() {
        {
            let args = [ctcore::ct_util_name(), "-h"];
            let mut args_iter = args.iter().map(|s| OsString::from(*s));
            let result = arch_main(&mut args_iter);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 1);
        }

        {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-h"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }
    }

    #[test]
    fn test_arch_v_ctmain() {
        {
            let args = ["arch", "--version"];
            let mut args_iter = args.iter().map(|s| OsString::from(*s));
            let result = arch_main(&mut args_iter);
            assert!(result.is_ok());
            assert!(result.unwrap().starts_with("arch (syskits) "));
        }
        {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }
    }

    #[test]
    fn test_arch_vv_ctmain() {
        {
            let args = [ctcore::ct_util_name(), "-V"];
            let mut args_iter = args.iter().map(|s| OsString::from(*s));
            let result = arch_main(&mut args_iter);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 1);
        }
        {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }
    }

    #[test]
    fn test_arch_ctmain() {
        let expected_arch = std::env::consts::ARCH;

        let args = [ctcore::ct_util_name()];
        let mut args_iter = args.iter().map(OsString::from);
        let result = arch_main(&mut args_iter);
        let mut s = String::new();
        match result {
            Err(output) => {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
            Ok(output) => {
                s = output.to_string();
            }
        }
        assert_eq!(s, expected_arch);
    }

    #[test]
    fn test_arch_ctmain_help() {
        let args = ["arch", "--help"];
        let mut args_iter = args.iter().map(OsString::from);
        let result = arch_main(&mut args_iter);
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.starts_with("Usage: arch [OPTION]..."));
        assert!(output.contains("Print machine architecture."));
        assert!(output.contains("--help        display this help and exit"));
        assert!(output.contains("syskits online help:"));
        assert!(!output.contains("GNU"));
    }

    #[test]
    fn test_arch_gnu_help_and_version_ignore_trailing_args() {
        let args = ["arch", "--help", "foo"];
        let mut args_iter = args.iter().map(OsString::from);
        let result = arch_main(&mut args_iter);
        assert!(result.is_ok());
        assert!(result.unwrap().starts_with("Usage: arch [OPTION]..."));

        let args = ["arch", "--version", "foo"];
        let mut args_iter = args.iter().map(OsString::from);
        let result = arch_main(&mut args_iter);
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.starts_with("arch (syskits) "));
        assert!(!output.contains("GNU"));
    }

    #[test]
    fn test_arch_gnu_invalid_short_option_diagnostic() {
        let args = ["arch", "-a"];
        let mut args_iter = args.iter().map(OsString::from);
        let err = arch_main(&mut args_iter).unwrap_err();

        assert_eq!(err.code(), 1);
        assert!(!err.usage());
        assert_eq!(
            err.to_string(),
            "invalid option -- 'a'\nTry 'arch --help' for more information."
        );
    }

    #[test]
    fn test_arch_gnu_unrecognized_long_option_diagnostic() {
        let args = ["arch", "--unknown"];
        let mut args_iter = args.iter().map(OsString::from);
        let err = arch_main(&mut args_iter).unwrap_err();

        assert_eq!(err.code(), 1);
        assert!(!err.usage());
        assert_eq!(
            err.to_string(),
            "unrecognized option '--unknown'\nTry 'arch --help' for more information."
        );
    }

    #[test]
    fn test_arch_gnu_long_option_value_diagnostic() {
        let args = ["arch", "--h=1"];
        let mut args_iter = args.iter().map(OsString::from);
        let err = arch_main(&mut args_iter).unwrap_err();

        assert_eq!(err.code(), 1);
        assert!(!err.usage());
        assert_eq!(
            err.to_string(),
            "option '--help' doesn't allow an argument\nTry 'arch --help' for more information."
        );
    }

    #[test]
    fn test_arch_gnu_ambiguous_long_option_diagnostic() {
        let args = ["arch", "--=1"];
        let mut args_iter = args.iter().map(OsString::from);
        let err = arch_main(&mut args_iter).unwrap_err();

        assert_eq!(err.code(), 1);
        assert!(!err.usage());
        assert_eq!(
            err.to_string(),
            "option '--=1' is ambiguous; possibilities: '--help' '--version'\nTry 'arch --help' for more information."
        );
    }

    #[test]
    fn test_arch_gnu_extra_operand_diagnostic() {
        let args = ["arch", "foo", "bar"];
        let mut args_iter = args.iter().map(OsString::from);
        let err = arch_main(&mut args_iter).unwrap_err();

        assert_eq!(err.code(), 1);
        assert!(!err.usage());
        assert!(err.to_string().contains("extra operand"));
        assert!(err.to_string().contains("foo"));
        assert!(
            err.to_string()
                .contains("Try 'arch --help' for more information.")
        );
    }
}
