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
use clap::{Arg, ArgAction, Command, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::CTResult;
use libc::c_long;
use std::ffi::OsString;
use sys_locale::get_locale;

// currently rust libc interface doesn't include gethostid
unsafe extern "C" {
    pub unsafe fn gethostid() -> c_long;
}

const OPERAND_NAME: &str = "OPERAND";

#[derive(Default)]
pub struct Hostid;
impl Tool for Hostid {
    fn name(&self) -> &'static str {
        "hostid"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        hostid_main(args.iter().cloned())
    }
}

pub fn hostid_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let args = args.collect::<Vec<_>>();
    validate_standard_options(&args)?;
    let matches = ct_app().try_get_matches_from(args)?;
    reject_extra_operand(&matches)?;
    hostid();
    Ok(())
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("hostid.about");
    let usage_description = t!("hostid.usage");

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_help(hostid_help_text(utility_name))
        .override_usage(usage_description)
        .infer_long_args(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .arg(Arg::new("help").long("help").action(ArgAction::Help))
        .arg(
            Arg::new("version")
                .long("version")
                .action(ArgAction::Version),
        )
        .arg(Arg::new(OPERAND_NAME).hide(true).action(ArgAction::Append))
}

fn hostid_help_text(usage_name: &str) -> String {
    format!(
        "\
Usage: {usage_name} [OPTION]
Print the numeric identifier (in hexadecimal) for the current host.

      --help        display this help and exit
      --version     output version information and exit

syskits online help: <https://www.ctyun.cn/developer>"
    )
}

fn validate_standard_options(args: &[OsString]) -> CTResult<()> {
    let mut options_ended = false;

    for arg in args.iter().skip(1) {
        let arg = arg.to_string_lossy();
        if options_ended {
            continue;
        }
        if arg == "--" {
            options_ended = true;
            continue;
        }
        if arg.starts_with("--") {
            validate_long_option(&arg)?;
            continue;
        }
        if let Some(short) = arg.strip_prefix('-').filter(|short| !short.is_empty()) {
            let invalid = short.chars().next().unwrap();
            return Err(ctcore::ct_error::CtSimpleError::new(
                1,
                format!(
                    "invalid option -- '{invalid}'\nTry '{} --help' for more information.",
                    ctcore::ct_help_utility_name()
                ),
            ));
        }
    }

    Ok(())
}

fn validate_long_option(arg: &str) -> CTResult<()> {
    let long = arg.strip_prefix("--").unwrap();
    let (name, has_value) = match long.split_once('=') {
        Some((name, _)) => (name, true),
        None => (long, false),
    };
    let matches = ["help", "version"]
        .into_iter()
        .filter(|option| option.starts_with(name))
        .collect::<Vec<_>>();

    let message = match matches.as_slice() {
        [option] if has_value => format!("option '--{option}' doesn't allow an argument"),
        [..] if !has_value && matches.len() == 1 => return Ok(()),
        [] => format!("unrecognized option '{arg}'"),
        _ => format!("option '{arg}' is ambiguous; possibilities: '--help' '--version'"),
    };

    Err(ctcore::ct_error::CtSimpleError::new(
        1,
        format!(
            "{message}\nTry '{} --help' for more information.",
            ctcore::ct_help_utility_name()
        ),
    ))
}

fn reject_extra_operand(matches: &clap::ArgMatches) -> CTResult<()> {
    if let Some(operand) = matches
        .get_many::<String>(OPERAND_NAME)
        .and_then(|operands| operands.into_iter().next())
    {
        return Err(ctcore::ct_error::CtSimpleError::new(
            1,
            format!(
                "extra operand {}\nTry '{} --help' for more information.",
                operand.quote(),
                ctcore::ct_help_utility_name()
            ),
        ));
    }
    Ok(())
}

fn hostid() {
    /*
     * POSIX says gethostid returns a "32-bit identifier" but is silent
     * whether it's sign-extended.  Turn off any sign-extension.  This
     * is a no-op unless unsigned int is wider than 32 bits.
     */

    let mut result: c_long = unsafe { gethostid() };

    #[allow(overflowing_literals)]
    let mask = 0xffff_ffff;

    result &= mask;
    println!("{result:0>8x}");
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostidSemantic {
    pub hostid: String,
}

pub fn hostid_native_semantic(args: impl ctcore::Args) -> CTResult<HostidSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let args = args.collect::<Vec<_>>();
    validate_standard_options(&args)?;
    let matches = ct_app().try_get_matches_from(args)?;
    reject_extra_operand(&matches)?;

    let mut result: c_long = unsafe { gethostid() };
    #[allow(overflowing_literals)]
    let mask = 0xffff_ffff;
    result &= mask;

    Ok(HostidSemantic {
        hostid: format!("{result:0>8x}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;
    use std::ffi::OsString;
    #[test]
    fn test_tool_implementation() {
        let tool = Hostid;

        // 测试 name 方法
        assert_eq!(tool.name(), "hostid");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("hostid"));

        // 测试 execute 方法
        let args = vec![OsString::from("hostid")];
        assert!(tool.execute(&args).is_ok());
    }

    #[cfg(test)]
    mod tests_ct_main {
        use super::*;
        use std::ffi::OsString;

        #[test]
        fn test_expand_main_version() {
            let args = [ctcore::ct_util_name(), "--version"];

            let result = hostid_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_expand_main_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = hostid_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_expand_main_v() {
            let args = [ctcore::ct_util_name(), "-V"];

            let result = hostid_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_expand_main_h() {
            let args = [ctcore::ct_util_name(), "-h"];
            let result = hostid_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_expand_main() {
            let args = [ctcore::ct_util_name()];
            let result = hostid_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }
    }

    #[cfg(test)]
    mod tests_ct_app {
        use super::*;
        // hostid 接口: hostid [options]...
        // Print the numeric identifier (in hexadecimal) for the current host
        // Usage: target/debug/syskits hostid [options]
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
        fn test_ct_app_rejects_short_version() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-V"];
            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_execution_help() {
            let command = ct_app();

            // 测试用例2：验证 --help 参数是否正确处理
            let help_args = vec![ctcore::ct_util_name(), "--help"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_execution_unsupport_help() {
            let command = ct_app();

            // 测试用例2：验证 --help 参数是否正确处理
            let help_args = vec![ctcore::ct_util_name(), "-H"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_invalid_argument() {
            let command = ct_app();

            // 测试用例3：验证当提供未知参数时是否正确报错
            let invalid_args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_support_missing_argument() {
            let command = ct_app();

            // 测试用例4：验证当缺少必需的参数时是否正确报错
            let missing_args = vec![ctcore::ct_util_name()]; // 缺少任何参数
            let result = command.try_get_matches_from(missing_args);
            assert!(result.is_ok());
        }
    }
}
