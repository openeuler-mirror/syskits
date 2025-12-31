/*
 *    Copyright(c) 2022-2024 China Telecom Cloud Technologies co., Ltd. All rights reserved
 *     syskits is licensed under Mulan PSL v2.
 *    You can use this software according to the terms and conditions of the Mulan PSL V2
 *    You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 *    THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 *    KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 *    NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 *    See the Mulan PSL v2 for more details.
 *
 */

//dirname命令主要用于从给定的文件或目录路径中剥离出目录部分，去掉路径末尾的文件名（或最后一个组件），仅保留上级目录的路径。

use clap::{crate_version, Arg, ArgAction, Command};
use ctcore::ct_display::ct_print_verbatim;
use ctcore::ct_error::{CTResult, CTsageError};
use ctcore::ct_line_ending::CtLineEnding;
use ctcore::{ct_format_usage, ct_help_about, ct_help_section, ct_help_usage};
use std::path::Path;

const DIRNAME_ABOUT: &str = ct_help_about!("dirname.md");
const DIRNAME_USAGE: &str = ct_help_usage!("dirname.md");
const DIRNAME_AFTER_HELP: &str = ct_help_section!("after help", "dirname.md");

mod opt_flags {
    pub const ZERO: &str = "zero";
    pub const DIR: &str = "dir";
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    dirname_main(args).map(|_| ())
}

pub fn dirname_main(args: impl ctcore::Args) -> CTResult<()> {
    let args_match = ct_app()
        .after_help(DIRNAME_AFTER_HELP)
        .try_get_matches_from(args)?;

    let line_ending = CtLineEnding::from_zero_flag(args_match.get_flag(opt_flags::ZERO));

    let dirnames: Vec<String> = args_match
        .get_many::<String>(opt_flags::DIR)
        .unwrap_or_default()
        .cloned()
        .collect();

    if let Some(value) = dirname_process(line_ending, &dirnames) {
        return value;
    }

    Ok(())
}

fn dirname_process(line_ending: CtLineEnding, dirnames: &Vec<String>) -> Option<CTResult<()>> {
    if dirnames.is_empty() {
        return Some(Err(CTsageError::new(1, "missing operand")));
    } else {
        for item in dirnames {
            let path = Path::new(item);
            match path.parent() {
                Some(dir) => {
                    if dir.components().next().is_none() {
                        print!(".");
                    } else {
                        ct_print_verbatim(dir).unwrap();
                    }
                }
                None => {
                    if path.is_absolute() || item == "/" {
                        print!("/");
                    } else {
                        print!(".");
                    }
                }
            }
            print!("{line_ending}");
        }
    }
    None
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = DIRNAME_ABOUT;
    let usage_description = ct_format_usage(DIRNAME_USAGE);

    let args = vec![
        Arg::new(opt_flags::ZERO)
            .long(opt_flags::ZERO)
            .short('z')
            .help("separate output with NUL rather than newline")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::DIR)
            .hide(true)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::AnyPath),
    ];

    Command::new(utility_name)
        .about(application_info)
        .version(command_version)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(&args)
}

