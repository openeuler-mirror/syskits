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

use clap::Arg;
use clap::ArgAction;
use clap::Command;
use ct_base32::base_common::{self, BaseConfig, BASE_CMD_PARSE_ERROR};

use ctcore::{
    ct_encoding::Format,
    ct_error::{CTResult, CTsageError},
};

use ctcore::ct_error::UClapError;
use std::io::stdin;
use std::io::Read;

use ctcore::ct_help_about;
use ctcore::ct_help_usage;

const BASE64_ABOUT: &str = ct_help_about!("basenc.md");
const BASE64_USAGE: &str = ct_help_usage!("basenc.md");

const BASE64_ENCODINGS: &[(&str, Format, &str)] = &[
    ("base64", Format::Base64, "same as 'base64' program"),
    ("base64url", Format::Base64Url, "file- and url-safe base64"),
    ("base32", Format::Base32, "same as 'base32' program"),
    (
        "base32hex",
        Format::Base32Hex,
        "extended hex alphabet base32",
    ),
    ("base16", Format::Base16, "hex encoding"),
    (
        "base2lsbf",
        Format::Base2Lsbf,
        "bit string with least significant bit (lsb) first",
    ),
    (
        "base2msbf",
        Format::Base2Msbf,
        "bit string with most significant bit (msb) first",
    ),
    (
        "z85",
        Format::Z85,
        "ascii85-like encoding;\n\
         when encoding, input length must be a multiple of 4;\n\
         when decoding, input length must be a multiple of 5",
    ),
];

pub fn ct_app() -> Command {
    let mut ct_cmd = base_common::base_common_app(BASE64_ABOUT, BASE64_USAGE);
    for encoding in BASE64_ENCODINGS {
        let raw = Arg::new(encoding.0)
            .long(encoding.0)
            .help(encoding.2)
            .action(ArgAction::SetTrue);
        let overriding = BASE64_ENCODINGS
            .iter()
            .fold(raw, |arg, enc| arg.overrides_with(enc.0));
        ct_cmd = ct_cmd.arg(overriding);
    }
    ct_cmd
}

fn basenc_parse_cmd_args(args: impl ctcore::Args) -> CTResult<(BaseConfig, Format)> {
    let args_match = ct_app()
        .try_get_matches_from(args.collect_lossy())
        .with_exit_code(1)?;
    let format_mod = BASE64_ENCODINGS
        .iter()
        .find(|encoding| args_match.get_flag(encoding.0))
        .ok_or_else(|| CTsageError::new(BASE_CMD_PARSE_ERROR, "missing encoding type"))?
        .1;
    let config_mod = BaseConfig::from(&args_match)?;
    Ok((config_mod, format_mod))
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    basenc_main(args).map(|_| ())
}

pub fn basenc_main(args: impl ctcore::Args) -> CTResult<String> {
    let (config_mod, format_mod) = basenc_parse_cmd_args(args)?;

    // 创建对stdin的引用，以便我们能从parse_base_cmd_args返回锁定的stdin
    let ct_stdin = stdin();
    let mut ct_input: Box<dyn Read> = base_common::get_base_input(&config_mod, &ct_stdin)?;

    base_common::handle_base_input(
        &mut ct_input,
        format_mod,
        config_mod.base_wrap_cols,
        config_mod.base_ignore_garbage,
        config_mod.base_decode,
    )
}

