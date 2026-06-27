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
rust_i18n::i18n!("locales", fallback = "zh-CN");
use sys_locale::get_locale;

use clap::Arg;
use clap::ArgAction;
use clap::Command;
use clap::crate_version;
use platform_info::*;

use ctcore::Tool;
use ctcore::ct_error::CTResult;
use ctcore::ct_error::CtSimpleError;
use rust_i18n::t;
use std::ffi::OsString;

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
                println!("{}", s);
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

    ct_app().try_get_matches_from(args)?;

    let platform_info =
        PlatformInfo::new().map_err(|_e| CtSimpleError::new(1, "cannot get system name"))?;

    let binding = platform_info.machine().to_string_lossy();
    let s = binding.trim();

    Ok(s.to_string())
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
                .short('h')
                .long("help")
                .help(t!("arch.clap.help"))
                .action(ArgAction::Help),
        )
        .arg(
            Arg::new("version")
                .short('V')
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
        let tool = Arch::default();
        assert_eq!(tool.name(), "arch");

        let command = tool.command();
        assert!(command.get_name().contains("arch"));

        let args = vec![OsString::from("arch")];
        let result = tool.execute(&args);
        assert!(result.is_ok());

        let args = vec![OsString::from("arch"), OsString::from("--help")];
        let result = tool.execute(&args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), 0);

        let args = vec![OsString::from("arch"), OsString::from("--version")];
        let result = tool.execute(&args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), 0);
    }

    #[test]
    fn test_command_line_args() {
        let tool = Arch::default();

        let args = vec![OsString::from("arch"), OsString::from("--help")];
        assert!(tool.execute(&args).is_err());

        let args = vec![OsString::from("arch"), OsString::from("--version")];
        assert!(tool.execute(&args).is_err());
    }

    #[test]
    fn test_arch_hh_ctmain() {
        {
            let args = ["-h", ""];
            let mut args_iter = args.iter().map(|s| OsString::from(*s));
            let result = arch_main(&mut args_iter);
            assert!(result.is_err());
        }

        {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-h"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }
    }

    #[test]
    fn test_arch_v_ctmain() {
        {
            let args = ["--version", ""];
            let mut args_iter = args.iter().map(|s| OsString::from(*s));
            let result = arch_main(&mut args_iter);
            assert!(result.is_err());
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
            let args = ["-V", ""];
            let mut args_iter = args.iter().map(|s| OsString::from(*s));
            let result = arch_main(&mut args_iter);
            assert!(result.is_err());
        }
        {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }
    }

    #[test]
    fn test_arch_ctmain() {
        let expected_arch = std::env::consts::ARCH;

        let args = vec![ctcore::ct_util_name()];
        let mut args_iter = args.iter().map(|s| OsString::from(s));
        let result = arch_main(&mut args_iter);
        let mut s = String::new();
        match result {
            Err(output) => {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {}", code);
                println!("Error message: {}", message);
            }
            Ok(output) => {
                s = output.to_string();
            }
        }
        assert_eq!(s, expected_arch);
    }

    #[test]
    fn test_arch_ctmain_help() {
        let args = vec![ctcore::ct_util_name(), "--help"];
        let mut args_iter = args.iter().map(|s| OsString::from(s));
        let result = arch_main(&mut args_iter);
        assert!(result.is_err());
    }
}
