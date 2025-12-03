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

const VERSION: &str = env!("CARGO_PKG_VERSION");

include!(concat!(env!("OUT_DIR"), "/syskits_app_map.rs"));

fn usage<T>(utils: &AppMap<T>, name: &str) {
    println!("{} {VERSION} (multi-call binary)\n", name);
    println!("Usage: {} [function [arguments...]]\n", name);
    println!("Currently defined functions:\n");

    #[allow(clippy::map_clone)]
    let mut ctutils: Vec<&str> = utils.keys().map(|&str_info| str_info).collect();

    ctutils.sort_unstable();

    let display_list_info = ctutils.join(", ");
    let width = std::cmp::min(textwrap::termwidth(), 100) - 4 * 2;

    println!(
        "{}",
        textwrap::indent(&textwrap::fill(&display_list_info, width), "    ")
    );
}

fn binary_path(args: &mut impl Iterator<Item = OsString>) -> PathBuf {
    if let Some(s) = args.next() {
        if !s.is_empty() {
            return PathBuf::from(s);
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

fn name(binary_path: &Path) -> Option<&str> {
    if let Some(file_stem) = binary_path.file_stem() {
        return file_stem.to_str();
    }
    None
}

#[allow(clippy::cognitive_complexity)]
fn main() {
    ctcore::ct_panic::mute_sigpipe_panic();

    let utils_map = util_map();
    let mut args_info = ctcore::args_os();

    let binary = binary_path(&mut args_info);
    let binary_as_util = match name(&binary) {
        Some(name) => name,
        None => {
            usage(&utils_map, "<unknown binary name>");
            process::exit(0);
        }
    };

    // binary name equals util name?
    match utils_map.get(binary_as_util) {
        Some(&(ctmain, _)) => {
            process::exit(ctmain((vec![binary.into()].into_iter()).chain(args_info)));
        }
        None => {
            // Handle the case when the utility is not found
            // You might want to provide appropriate behavior here
            // For now, let's just flags
        }
    }

    let util_name = match utils_map.keys().find(|util| {
        binary_as_util.ends_with(*util)
            && !binary_as_util[..binary_as_util.len() - (*util).len()]
                .ends_with(char::is_alphanumeric)
    }) {
        Some(util) => {
            // prefixed util => replace 0th (aka, executable name) argument
            Some(OsString::from(*util))
        }
        None => {
            // unmatched binary name => regard as multi-binary container and advance argument list
            ctcore::set_utility_is_second_arg();
            args_info.next()
        }
    };

    if let Some(util_os) = util_name {
        fn not_found(util_os_str: &OsStr) -> ! {
            println!("{}: utility/function not found", util_os_str.maybe_quote());
            process::exit(1);
        }

        let util = match util_os.to_str() {
            Some(util) => util,
            None => {
                not_found(&util_os);
            }
        };

        match util {
            "completion" => gen_completions(args_info, &utils_map),
            "manpage" => gen_manpage(args_info, &utils_map),
            _ => {}
        }

        if let Some(&(ctmain, _)) = utils_map.get(util) {
            process::exit(ctmain((vec![util_os].into_iter()).chain(args_info)));
        } else if util == "-h" || util == "--help" {
            // see if they want help on a specific util
            if let Some(util_os) = args_info.next() {
                let util_str = match util_os.to_str() {
                    Some(util) => util,
                    None => not_found(&util_os),
                };

                if let Some(&(ctmain, _)) = utils_map.get(util_str) {
                    let code = ctmain(
                        (vec![util_os, OsString::from("--help")].into_iter()).chain(args_info),
                    );
                    io::stdout().flush().expect("could not flush stdout info");
                    process::exit(code);
                } else {
                    not_found(&util_os);
                }
            }
            usage(&utils_map, binary_as_util);
            process::exit(0);
        } else {
            not_found(&util_os);
        }
    } else {
        // no arguments provided
        usage(&utils_map, binary_as_util);
        process::exit(0);
    }
}

/// Prints completions for the utility in the first parameter for the shell in the second parameter to stdout
fn gen_completions<T: ctcore::Args>(
    args: impl Iterator<Item = OsString>,
    util_map: &AppMap<T>,
) -> ! {
    let all_utilities: Vec<_> = std::iter::once("syskits")
        .chain(util_map.keys().copied())
        .collect();

    let mut command_builder = Command::new("completion");
    command_builder = command_builder.about("Prints completions to stdout");
    command_builder = command_builder.arg(
        Arg::new("utility")
            .value_parser(clap::builder::PossibleValuesParser::new(all_utilities))
            .required(true),
    );

    let args_iter = std::iter::once(OsString::from("manpage")).chain(args);
    let matches = command_builder.get_matches_from(args_iter);

    let utility = match matches.get_one::<String>("utility") {
        Some(utility) => utility,
        None => {
            usage(util_map, "manpage");
            process::exit(1);
        }
    };

    let shell = match matches.get_one::<Shell>("shell") {
        Some(shell) => *shell,
        None => {
            eprintln!("Shell argument missing");
            process::exit(1);
        }
    };

    let mut cmd = if utility == "syskits" {
        gen_syskits_app(util_map)
    } else {
        match util_map.get(utility) {
            Some((_, sub_app)) => sub_app(),
            None => {
                eprintln!("Utility not found in map");
                process::exit(1);
            }
        }
    };

    let bin_name = match std::env::var("PROG_PREFIX") {
        Ok(prefix) => prefix + utility,
        Err(_) => utility.clone(),
    };

    clap_complete::generate(shell, &mut cmd, bin_name, &mut io::stdout());

    if let Err(err) = io::stdout().flush() {
        eprintln!("Failed to flush stdout: {}", err);
        process::exit(1);
    }
    process::exit(0);
}

/// Generate the manpage for the utility in the first parameter
fn gen_manpage<T: ctcore::Args>(args: impl Iterator<Item = OsString>, util_map: &AppMap<T>) -> ! {
    let all_utilities: Vec<_> = std::iter::once("syskits")
        .chain(util_map.keys().copied())
        .collect();

    let mut command_builder = Command::new("manpage");
    command_builder = command_builder.about("Prints manpage info to stdout");
    command_builder = command_builder.arg(
        Arg::new("utility")
            .value_parser(clap::builder::PossibleValuesParser::new(all_utilities))
            .required(true),
    );

    let args_iter = std::iter::once(OsString::from("manpage")).chain(args);
    let matches = command_builder.get_matches_from(args_iter);

    let utility = match matches.get_one::<String>("utility") {
        Some(utility) => utility,
        None => {
            usage(util_map, "manpage");
            process::exit(1);
        }
    };

    let cmd = if utility == "syskits" {
        gen_syskits_app(util_map)
    } else {
        match util_map.get(utility) {
            Some((_, sub_app)) => sub_app(),
            None => {
                eprintln!("Utility not found in map");
                process::exit(1);
            }
        }
    };
    let man = clap_mangen::Man::new(cmd);
    man.render(&mut io::stdout())
        .expect("Man page generation failed");
    io::stdout().flush().expect("Failed to flush stdout");
    process::exit(0);
}

fn gen_syskits_app<T: ctcore::Args>(util_map: &AppMap<T>) -> Command {
    let mut command = Command::new("syskits");

    for (name, (_, sub_app)) in util_map {
        let about = match sub_app().get_about() {
            Some(about) => about.to_string(),
            None => panic!("Could not get the 'about'"),
        };

        let subcommand = Command::new(name).about(about);
        command = command.subcommand(subcommand);
    }

    command
}
