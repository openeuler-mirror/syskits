// This file is part of the cttils coreutils package.
//
// For the full copyright and license information, please view the LICENSE
// file that was distributed with this source code.

use std::ffi::OsString;

use clap::{crate_version, Command};

use ctcore::ct_display::ct_println_verbatim;
use ctcore::ct_error::{CTResult, FromIo};
use ctcore::{ct_format_usage, ct_help_about, ct_help_usage};

mod platform;

const WHOAMI_ABOUT: &str = ct_help_about!("whoami.md");
const WHOAMI_USAGE: &str = ct_help_usage!("whoami.md");

pub fn ctmain(args: impl ctcore::Args) -> i32 {
    pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
        whoami_main(args).map(|_| ())
    }

    let result = ctmain(args);
    match result {
        Ok(()) => ctcore::ct_error::get_ct_exit_code(),
        Err(err) => {
            let s_err = {
                let res = format!("{}", err);
                res
            };
            if !s_err.is_empty() {
                {
                    eprintln!("{}: ", ctcore::ct_util_name());
                    eprintln!("{}", s_err);
                }
            }
            if err.usage() {
                eprintln!(
                    "Try '{} --help' for more information.",
                    ctcore::ct_execute_phrase()
                );
            }
            err.code()
        }
    }
}

pub fn whoami_main(args: impl ctcore::Args) -> CTResult<String> {
    ct_app().try_get_matches_from(args)?;
    let username = whoami_exec()?;
    ct_println_verbatim(username.clone()).map_err_context(|| "failed to print username".into())?;

    let result = username.into_string().unwrap();
    Ok(result)
}

/// 获取当前用户名
pub fn whoami_exec() -> CTResult<OsString> {
    let username_result = platform::get_username();

    username_result.map_err_context(|| "failed to get username".into())
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = WHOAMI_ABOUT;
    let usage_description = ct_format_usage(WHOAMI_USAGE);

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
}

