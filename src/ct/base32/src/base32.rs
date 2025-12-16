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

use std::io::stdin;
use std::io::Read;

use crate::base_common::opt_flags;

use clap::crate_version;
use clap::Arg;
use clap::ArgAction;
use clap::Command;

use ctcore::{
    ct_encoding::Format, ct_error::CTResult, ct_format_usage, ct_help_about, ct_help_usage,
};

pub mod base_common;

const BASE32_ABOUT: &str = ct_help_about!("base32.md");
const BASE32_USAGE: &str = ct_help_usage!("base32.md");

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    base32_main(args).map(|_| ())
}

pub fn base32_main(args: impl ctcore::Args) -> CTResult<String> {
    let format_mod = Format::Base32;

    let config_info: base_common::BaseConfig =
        base_common::base_parsing_command_args(args, BASE32_ABOUT, BASE32_USAGE)?;

    let stdin_info = stdin();
    let mut input_info: Box<dyn Read> = base_common::get_base_input(&config_info, &stdin_info)?;

    base_common::handle_base_input(
        &mut input_info,
        format_mod,
        config_info.base_wrap_cols,
        config_info.base_ignore_garbage,
        config_info.base_decode,
    )
}

pub fn ct_app() -> Command {
    let util_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = BASE32_ABOUT;
    let usage_description = ct_format_usage(BASE32_USAGE);

    let args = base32_args_init();

    Command::new(util_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(&args)
}

fn base32_args_init() -> Vec<Arg> {
    let args = vec![
        Arg::new(opt_flags::BASE_DECODE)
            .short('d')
            .long(opt_flags::BASE_DECODE)
            .help("decode data")
            .action(ArgAction::SetTrue)
            .overrides_with(opt_flags::BASE_DECODE),
        Arg::new(opt_flags::BASE_IGNORE_GARBAGE)
            .short('i')
            .long(opt_flags::BASE_IGNORE_GARBAGE)
            .help("when decoding, ignore non-alphabetic characters")
            .action(ArgAction::SetTrue)
            .overrides_with(opt_flags::BASE_IGNORE_GARBAGE),
        Arg::new(opt_flags::BASE_WRAP)
            .short('w')
            .long(opt_flags::BASE_WRAP)
            .value_name("COLS")
            .help("wrap encoded lines after COLS character (default 76, 0 to disable wrapping)")
            .overrides_with(opt_flags::BASE_WRAP),
        Arg::new(opt_flags::BASE_FILE)
            .index(1)
            .action(clap::ArgAction::Append)
            .value_hint(clap::ValueHint::FilePath),
    ];
    args
}

