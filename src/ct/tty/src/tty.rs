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

//! tty 命令行工具，用于打印当前终端设备的文件名

extern crate rust_i18n;
use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "zh-CN");
use ctcore::Tool;
use ctcore::ct_error::{CTResult, set_ct_exit_code};
use std::ffi::OsString;
use std::io::IsTerminal;
use std::io::Write;
use sys_locale::get_locale;
mod tty_flags {
    pub const TTY_SILENT: &str = "silent";
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    tty_main(args)
}

pub fn tty_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let matches = ct_app().try_get_matches_from(args)?;

    if let Some(value) = tty_handle_silent(matches) {
        return value;
    }

    let mut stdout = std::io::stdout();

    let tty_name = nix::unistd::ttyname(std::io::stdin());

    let tty_write_result = match tty_name {
        Ok(name) => writeln!(stdout, "{}", name.display()),
        Err(_) => {
            set_ct_exit_code(1);
            writeln!(stdout, "not a tty")
        }
    };

    if tty_write_result.is_err() || stdout.flush().is_err() {
        // 避免返回以防止稍后在尝试另一次刷新时引发panic
        // 因为`ctcore_procs::main`宏在每个实用程序执行后都会插入一次刷新。
        std::process::exit(3);
    };

    Ok(())
}

fn tty_handle_silent(matches: ArgMatches) -> Option<CTResult<()>> {
    let is_silent = matches.get_flag(tty_flags::TTY_SILENT);

    // 如果处于静默模式，我们不需要名称，只需要判断标准输入是否是TTY
    if is_silent {
        return Some(match std::io::stdin().is_terminal() {
            true => Ok(()),
            false => Err(1.into()),
        });
    };
    None
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("tty.about");
    let usage_description = t!("tty.usage");

    let arg = Arg::new(tty_flags::TTY_SILENT)
        .long(tty_flags::TTY_SILENT)
        .visible_alias("quiet")
        .short('s')
        .help(t!("tty.clap.tty_silent"))
        .action(ArgAction::SetTrue);
    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .arg(arg)
}

#[derive(Default)]
pub struct Tty;
impl Tool for Tty {
    fn name(&self) -> &'static str {
        "tty"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        // 直接调用原有的 tty_main 函数
        tty_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Tty::default();

        // 测试 name 方法
        assert_eq!(tool.name(), "tty");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("tty"));

        // 测试 execute 方法
        let args = vec![OsString::from("tty"), OsString::from("--version")];
        assert!(tool.execute(&args).is_err());
    }

    #[cfg(test)]
    mod ct_main_tests {
        use std::ffi::OsString;

        use super::*;

        #[test]
        fn test_tty_main_execution_default() {
            let args = vec![ctcore::ct_util_name()];
            let result = tty_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_tty_main_execution_version() {
            let args_vec = vec![ctcore::ct_util_name(), "--version"];
            let args = args_vec.iter().map(|s| OsString::from(s));
            let result = tty_main(args);

            assert!(result.is_err());
        }

        #[test]
        fn test_tty_main_execution_other_version() {
            let args = vec![ctcore::ct_util_name(), "-V"];
            let result = tty_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_tty_main_execution_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = tty_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_tty_main_execution_help_short() {
            let args = vec![ctcore::ct_util_name(), "-h"];
            let result = tty_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_tty_main_execution_unsupport_help() {
            let args = vec![ctcore::ct_util_name(), "-H"];
            let result = tty_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_tty_main_invalid_argument() {
            let args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = tty_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }
        #[test]
        fn test_tty_main_silent_long() {
            let args = vec![ctcore::ct_util_name(), "--silent"];
            let result = tty_main(args.iter().map(|s| OsString::from(s)));
            if std::io::stdin().is_terminal() {
                assert!(result.is_ok());
            } else {
                assert!(result.is_err());
            }
        }
        #[test]
        fn test_tty_main_silent_short() {
            let args = vec![ctcore::ct_util_name(), "-s"];
            let result = tty_main(args.iter().map(|s| OsString::from(s)));
            if std::io::stdin().is_terminal() {
                assert!(result.is_ok());
            } else {
                assert!(result.is_err());
            }
        }

        #[test]
        fn test_tty_main_quiet_long() {
            let args = vec![ctcore::ct_util_name(), "--quiet"];
            let result = tty_main(args.iter().map(|s| OsString::from(s)));
            if std::io::stdin().is_terminal() {
                assert!(result.is_ok());
            } else {
                assert!(result.is_err());
            }
        }
    }

    #[cfg(test)]
    mod ct_app_tests {
        use clap::error::ErrorKind;

        use super::*;

        // tty 接口: tty [OPTION]...
        //
        // Options:
        //   -s, --silent   print nothing, only return an exit status [aliases: quiet]
        //   -h, --help     Print help
        //   -V, --version  Print version

        #[test]
        fn test_ct_app_execution_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];
            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_other_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];

            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_help() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "--help"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }
        #[test]
        fn test_ct_app_execution_help_short() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-h"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_execution_unsupport_help() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-H"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_invalid_argument() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_support_missing_argument() {
            let command = ct_app();

            let missing_args = vec![ctcore::ct_util_name()]; // 缺少任何参数
            let result = command.try_get_matches_from(missing_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_silent_long() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--silent"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }
        #[test]
        fn test_ct_app_silent_short() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-s"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_quiet_long() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--quiet"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }
    }
}
