/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

//! logname指令，它会显示目前用户的名称。

use clap::{crate_version, Command};
use ctcore::{ct_error::CTResult, ct_format_usage, ct_help_about, ct_help_usage, ct_show_error};
use std::ffi::CStr;

extern "C" {
    // POSIX 要求使用 getlogin（或同等代码）
    pub fn getlogin() -> *const libc::c_char;
}

fn get_user_login() -> Option<String> {
    unsafe {
        let login_name: *const libc::c_char = getlogin();
        match login_name.is_null() {
            true => None,
            false => {
                Some(String::from_utf8_lossy(CStr::from_ptr(login_name).to_bytes()).to_string())
            }
        }
    }
}

const LOGNAME_ABOUT: &str = ct_help_about!("logname.md");
const LOGNAME_USAGE: &str = ct_help_usage!("logname.md");

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    logname_main(args)
}

pub fn logname_main(args: impl ctcore::Args) -> CTResult<()> {
    let _ = ct_app().try_get_matches_from(args)?;

    match get_user_login() {
        Some(userlogin) => println!("{userlogin}"),
        None => ct_show_error!("no login name"),
    }

    Ok(())
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = LOGNAME_ABOUT;
    let usage_description = ct_format_usage(LOGNAME_USAGE);
    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(test)]
    mod ct_main_tests {
        use super::*;
        use std::ffi::OsString;

        #[test]
        fn test_logname_main_execution_default() {
            let args = vec![ctcore::ct_util_name()];
            let result = logname_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }
        #[test]
        fn test_logname_main_execution_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = logname_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_logname_main_execution_other_version() {
            let args = vec![ctcore::ct_util_name(), "-V"];
            let result = logname_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_logname_main_execution_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = logname_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }
        #[test]
        fn test_logname_main_execution_help_short() {
            let args = vec![ctcore::ct_util_name(), "-h"];
            let result = logname_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_logname_main_execution_unsupport_help() {
            let args = vec![ctcore::ct_util_name(), "-H"];
            let result = logname_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_logname_main_invalid_argument() {
            let args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = logname_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }
    }
}