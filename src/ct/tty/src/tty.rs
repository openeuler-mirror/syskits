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

//! tty 命令行工具，用于打印当前终端设备的文件名

use clap::{crate_version, Arg, ArgAction, ArgMatches, Command};
use ctcore::ct_error::{set_ct_exit_code, CTResult};
use ctcore::{ct_format_usage, ct_help_about, ct_help_usage};
use std::io::{IsTerminal, Write};

const TTY_ABOUT: &str = ct_help_about!("tty.md");
const TTY_USAGE: &str = ct_help_usage!("tty.md");

mod tty_flags {
    pub const TTY_SILENT: &str = "silent";
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    tty_main(args)
}
pub fn tty_main(args: impl ctcore::Args) -> CTResult<()> {
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
    let application_info = TTY_ABOUT;
    let usage_description = ct_format_usage(TTY_USAGE);

    let arg = Arg::new(tty_flags::TTY_SILENT)
        .long(tty_flags::TTY_SILENT)
        .visible_alias("quiet")
        .short('s')
        .help("print nothing, only return an exit status")
        .action(ArgAction::SetTrue);
    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .arg(arg)
}

