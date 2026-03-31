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

//! 创建硬链接
//!
//! 调用 link 函数，创建一个名为 <文件2> 的硬链接，指向现有的文件 <文件1>。

use clap::builder::ValueParser;
use clap::{Arg, Command, crate_version};
use ctcore::{
    ct_display::Quotable,
    ct_error::{CTResult, CtSimpleError, FromIo},
    ct_format_usage, ct_help_about, ct_help_usage,
};
use std::{ffi::OsString, fs::hard_link, path::Path};

const LINK_ABOUT: &str = ct_help_about!("link.md");
const LINK_USAGE: &str = ct_help_usage!("link.md");

mod link_flags {
    pub const FILES: &str = "FILES";
}

/// 存储链接操作的源文件和目标文件路径
struct LinkFlags<'a> {
    source_path: &'a Path,
    target_path: &'a Path,
}

impl<'a> LinkFlags<'a> {
    /// 从命令行参数创建 LinkFlags 实例
    ///
    /// # Arguments
    /// * `matches` - 解析后的命令行参数
    ///
    /// # Returns
    /// * `CTResult<Self>` - 成功则返回 LinkFlags 实例，失败则返回错误
    fn new(matches: &'a clap::ArgMatches) -> CTResult<Self> {
        let files: Vec<_> = matches
            .get_many::<OsString>(link_flags::FILES)
            .unwrap_or_default()
            .collect();

        if files.len() != 2 {
            return Err(CtSimpleError::new(1, "wrong number of arguments"));
        }

        Ok(Self {
            source_path: Path::new(files[0]),
            target_path: Path::new(files[1]),
        })
    }
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    link_main(args)
}

/// 主函数：解析参数并执行链接操作
pub fn link_main(args: impl ctcore::Args) -> CTResult<()> {
    let matches = ct_app().try_get_matches_from(args)?;
    let flags = LinkFlags::new(&matches)?;
    link_exec(&flags)
}

/// 执行硬链接创建操作
fn link_exec(flags: &LinkFlags) -> CTResult<()> {
    hard_link(flags.source_path, flags.target_path).map_err_context(|| {
        format!(
            "cannot create link {} to {}",
            flags.target_path.quote(),
            flags.source_path.quote()
        )
    })
}

/// 创建命令行应用程序配置
pub fn ct_app() -> Command {
    let arg = Arg::new(link_flags::FILES)
        .hide(true)
        .required(true)
        .num_args(2)
        .value_hint(clap::ValueHint::AnyPath)
        .value_parser(ValueParser::os_string());

    Command::new(ctcore::ct_util_name())
        .version(crate_version!())
        .about(LINK_ABOUT)
        .override_usage(ct_format_usage(LINK_USAGE))
        .infer_long_args(true)
        .arg(arg)
}
