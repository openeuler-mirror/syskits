/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved
 *   syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

// spell-checker:ignore manpages mangen

use clap::{Arg, Command};
use clap_complete::Shell;
use ctcore::ct_display::Quotable;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process;

const CT_VERSION: &str = env!("CARGO_PKG_VERSION");

include!(concat!(env!("OUT_DIR"), "/syskits_app_map.rs"));

#[allow(clippy::cognitive_complexity)]
fn main() {
    ctcore::ct_panic::ct_mute_set_panic_hook();

    let ct_utils_map = util_map();
    let mut cg_args = ctcore::ct_os_args();

    let execute = execute_path(&mut cg_args);
    let execute_as_util = match execute_name(&execute) {
        Some(name) => name,
        None => {
            ct_help(&ct_utils_map, "<unknown binary name>");
            process::exit(0);
        }
    };

    // 二进制文件名是否等于工具名称？
    match ct_utils_map.get(execute_as_util) {
        Some(&(ctmain, _)) => {
            process::exit(ctmain((vec![execute.into()].into_iter()).chain(cg_args)));
        }
        None => {
            // 处理找不到工具的情况
            // 在此处可能需要提供适当的行为
            // 当前，我们仅设置标志
        }
    }

    let ct_util_name = match ct_utils_map.keys().find(|util| {
        execute_as_util.ends_with(*util)
            && !execute_as_util[..execute_as_util.len() - (*util).len()]
                .ends_with(char::is_alphanumeric)
    }) {
        Some(util) => {
            // 带前缀的工具 => 替换第0个（即，可执行文件名）参数
            Some(OsString::from(*util))
        }
        None => {
            // 无法匹配的二进制文件名 => 视为包含多个二进制文件的容器并推进参数列表
            ctcore::ct_set_utility_is_second_arg();
            cg_args.next()
        }
    };

    if let Some(ct_util_os) = ct_util_name {
        let ct_util = match ct_util_os.to_str() {
            Some(ct_util) => ct_util,
            None => {
                ct_find(&ct_util_os);
            }
        };

        if ct_util == "completion" {
            generate_completions(cg_args, &ct_utils_map);
        } else if ct_util == "manpage" {
            generate_manpage(cg_args, &ct_utils_map);
        } else {
            // unreachable!("unreachable");
        }

        if let Some(&(ctmain, _)) = ct_utils_map.get(ct_util) {
            process::exit(ctmain((vec![ct_util_os].into_iter()).chain(cg_args)));
        } else if ct_util == "-h" || ct_util == "--help" {
            // 检查他们是否需要关于特定util的帮助
            if let Some(util_os) = cg_args.next() {
                let util_str = match util_os.to_str() {
                    Some(util) => util,
                    None => ct_find(&util_os),
                };

                if let Some(&(ctmain, _)) = ct_utils_map.get(util_str) {
                    let code = ctmain(
                        (vec![util_os, OsString::from("--help")].into_iter()).chain(cg_args),
                    );
                    io::stdout().flush().expect("could not flush stdout info");
                    process::exit(code);
                } else {
                    ct_find(&util_os);
                }
            }
            ct_help(&ct_utils_map, execute_as_util);
            process::exit(0);
        } else {
            ct_find(&ct_util_os);
        }
    } else {
        // 没有提供任何参数
        ct_help(&ct_utils_map, execute_as_util);
        process::exit(0);
    }
}

/// 打印针对第二个参数中指定 shell 的第一个参数中所指实用工具的补全信息至 stdout
fn generate_completions<T: ctcore::Args>(
    args: impl Iterator<Item = OsString>,
    ct_util_map: &AppMap<T>,
) -> ! {
    let all_utilities: Vec<_> = std::iter::once("syskits")
        .chain(ct_util_map.keys().copied())
        .collect();

    let matches = Command::new("completion")
        .about("Prints completions to stdout")
        .arg(
            Arg::new("utility")
                .value_parser(clap::builder::PossibleValuesParser::new(all_utilities))
                .required(true),
        )
        .arg(
            Arg::new("shell")
                .value_parser(clap::builder::EnumValueParser::<Shell>::new())
                .required(true),
        )
        .get_matches_from(std::iter::once(OsString::from("completion")).chain(args));

    let utility = matches.get_one::<String>("utility").unwrap();
    let shell = *matches.get_one::<Shell>("shell").unwrap();

    let mut command = if utility == "syskits" {
        // gen_utils_app(util_map = if utility == "syskits" {
        gen_utils_app(ct_util_map)
    } else {
        ct_util_map.get(utility).unwrap().1()
    };
    let bin_name = std::env::var("PROG_PREFIX").unwrap_or_default() + utility;

    clap_complete::generate(shell, &mut command, bin_name, &mut io::stdout());
    io::stdout().flush().unwrap();
    process::exit(0);
}
/// # Panics
/// Panics if the utility map is empty
fn gen_utils_app<T: ctcore::Args>(util_map: &AppMap<T>) -> Command {
    let mut command = Command::new("coreutils");
    for (name, (_, sub_app)) in util_map {
        // Recreate a small subcommand with only the relevant info
        // (name & short description)
        let about = sub_app()
            .get_about()
            .expect("Could not get the 'about'")
            .to_string();
        let sub_app = Command::new(name).about(about);
        command = command.subcommand(sub_app);
    }
    command
}

fn get_command<T: Iterator<Item = OsString>>(
    ct_util_map: &AppMap<T>,
    ct_utility: &String,
) -> Command {
    let ct_command = if ct_utility == "syskits" {
        generate_skits_app(ct_util_map)
    } else {
        match ct_util_map.get(ct_utility) {
            Some((_, ct_sub_app)) => ct_sub_app(),
            None => {
                eprintln!("Utility not found in map");
                process::exit(1);
            }
        }
    };
    ct_command
}

fn generate_skits_app<T: ctcore::Args>(util_map: &AppMap<T>) -> Command {
    let mut ct_command = Command::new("syskits");

    for (ct_name, (_, ct_sub_app)) in util_map {
        let about = match ct_sub_app().get_about() {
            Some(about) => about.to_string(),
            None => panic!("Could not get the 'about'"),
        };

        let ct_sub_command = Command::new(ct_name).about(about);
        ct_command = ct_command.subcommand(ct_sub_command);
    }

    ct_command
}

/// 为第一个参数中指定的实用工具生成 man 页面
fn generate_manpage<T: ctcore::Args>(
    ct_args: impl Iterator<Item = OsString>,
    ct_util_map: &AppMap<T>,
) -> ! {
    let ct_utilities: Vec<_> = std::iter::once("syskits")
        .chain(ct_util_map.keys().copied())
        .collect();

    let mut ct_commander = Command::new("manpage");
    ct_commander = ct_commander.about("Prints manpage info to stdout");
    ct_commander = ct_commander.arg(
        Arg::new("utility")
            .value_parser(clap::builder::PossibleValuesParser::new(ct_utilities))
            .required(true),
    );

    let ct_args_iter = std::iter::once(OsString::from("manpage")).chain(ct_args);
    let ct_matches = ct_commander.get_matches_from(ct_args_iter);

    let ct_utility = match ct_matches.get_one::<String>("utility") {
        Some(ct_utility) => ct_utility,
        None => {
            ct_help(ct_util_map, "manpage");
            process::exit(1);
        }
    };

    let ct_cmd = get_command(ct_util_map, ct_utility);
    let ct_man = clap_mangen::Man::new(ct_cmd);
    ct_man
        .render(&mut io::stdout())
        .expect("Man page generation failed");
    io::stdout().flush().expect("Failed to flush stdout");
    process::exit(0);
}

fn ct_find(ct_util_os_str: &OsStr) -> ! {
    println!(
        "{}: utility/function not found",
        ct_util_os_str.maybe_quote()
    );
    process::exit(1);
}

fn ct_help<T>(ct_utils: &AppMap<T>, ct_name: &str) {
    println!("{} {CT_VERSION} (multi-call binary)\n", ct_name);
    println!("Usage: {} [function [arguments...]]\n", ct_name);
    println!("Currently defined functions:\n");

    #[allow(clippy::map_clone)]
    let mut ctutils: Vec<&str> = ct_utils.keys().map(|&str_info| str_info).collect();

    ctutils.sort_unstable();

    let ct_display_list_info = ctutils.join(", ");
    let ct_width = std::cmp::min(textwrap::termwidth(), 100) - 4 * 2;

    println!(
        "{}",
        textwrap::indent(&textwrap::fill(&ct_display_list_info, ct_width), "    ")
    );
}

fn execute_name(ct_execute_path: &Path) -> Option<&str> {
    if let Some(item) = ct_execute_path.file_stem() {
        return item.to_str();
    }
    None
}

fn execute_path(ct_args: &mut impl Iterator<Item = OsString>) -> PathBuf {
    if let Some(str) = ct_args.next() {
        if !str.is_empty() {
            return PathBuf::from(str);
        }
    }
    match std::env::current_exe() {
        Ok(path) => path,
        Err(_) => {
            println!("Failed to retrieve current executable path.");
            std::process::exit(1);
        }
    }
}
