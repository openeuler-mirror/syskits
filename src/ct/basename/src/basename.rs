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

use clap::crate_version;
use clap::Arg;
use clap::ArgAction;
use clap::Command;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CTsageError};
use ctcore::ct_format_usage;
use ctcore::ct_help_about;
use ctcore::ct_help_usage;
use ctcore::ct_line_ending::CtLineEnding;
use std::path::is_separator;
use std::path::PathBuf;

static BASENAME_ABOUT: &str = ct_help_about!("basename.md");

const BASENAME_USAGE: &str = ct_help_usage!("basename.md");

pub mod flags {
    pub static MULTIPLE: &str = "multiple";
    pub static NAME: &str = "name";
    pub static SUFFIX: &str = "suffix";
    pub static ZERO: &str = "zero";
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    basename_main(args).map(|_| ())
}

pub fn basename_main(args: impl ctcore::Args) -> CTResult<()> {
    let args = args.collect_lossy();

    let args_match = ct_app().try_get_matches_from(args)?;

    let line_ending_info = CtLineEnding::from_zero_flag(args_match.get_flag(flags::ZERO));

    let mut names = args_match
        .get_many::<String>(flags::NAME)
        .unwrap_or_default()
        .collect::<Vec<_>>();
    if names.is_empty() {
        return Err(CTsageError::new(1, "missing operand".to_string()));
    }
    let paths = args_match.get_one::<String>(flags::SUFFIX).is_some()
        || args_match.get_flag(flags::MULTIPLE);
    let base_suffix = if paths {
        args_match
            .get_one::<String>(flags::SUFFIX)
            .cloned()
            .unwrap_or_default()
    } else {
        let length = names.len();

        if length == 0 {
            panic!("already checked");
        } else if length == 1 {
            String::default()
        } else if length == 2 {
            names.pop().unwrap().clone()
        } else {
            return Err(CTsageError::new(
                1,
                format!("extra operand {}", names[2].quote(),),
            ));
        }
    };

    for path in names {
        print!("{}{}", basename(path, &base_suffix), line_ending_info);
    }

    Ok(())
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = BASENAME_ABOUT;
    let usage_description = ct_format_usage(BASENAME_USAGE);

    let args = basename_args_init();
    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(&args)
}

fn basename_args_init() -> Vec<Arg> {
    let args = vec![
        Arg::new(flags::MULTIPLE)
            .short('a')
            .long(flags::MULTIPLE)
            .help("support multiple arguments and treat each as a NAME")
            .action(ArgAction::SetTrue)
            .overrides_with(flags::MULTIPLE),
        Arg::new(flags::NAME)
            .action(clap::ArgAction::Append)
            .value_hint(clap::ValueHint::AnyPath)
            .hide(true)
            .trailing_var_arg(true),
        Arg::new(flags::SUFFIX)
            .short('s')
            .long(flags::SUFFIX)
            .value_name("SUFFIX")
            .help("remove a trailing SUFFIX; implies -a")
            .overrides_with(flags::SUFFIX),
        Arg::new(flags::ZERO)
            .short('z')
            .long(flags::ZERO)
            .help("end each output line with NUL, not newline")
            .action(ArgAction::SetTrue)
            .overrides_with(flags::ZERO),
    ];
    args
}

fn basename(fullname: &str, suffix: &str) -> String {
    // 步骤1：从末尾移除所有平台特定的路径分隔符
    let trimmed_path = fullname.trim_end_matches(is_separator);

    // 步骤2：确保在修剪后路径不为空，处理仅由后缀字符组成的特殊情况
    let adjusted_path = if trimmed_path.is_empty() {
        // 恢复为原始的fullname以避免返回空路径
        fullname
    } else {
        trimmed_path
    };

    // 步骤3：将调整后的路径转换为PathBuf
    let path_buffer = PathBuf::from(adjusted_path);

    // 步骤4：获取路径的最后一部分
    let last_component_option = path_buffer.components().last();

    // 步骤5：处理最后一部分缺失的情况
    let result = match last_component_option {
        Some(last_component) => {
            // 步骤6：将最后一部分作为字符串获取
            let last_component_name = last_component.as_os_str().to_str().unwrap();

            // 步骤7：比较最后一部分名称与提供的后缀
            if last_component_name == suffix {
                // 步骤8：若两者相等，则将后缀本身作为基名称返回
                last_component_name.to_string()
            } else {
                // 步骤9：若两者不相等，则尝试从前一部分移除后缀
                let stripped_name = last_component_name.strip_suffix(suffix);

                // 步骤10：处理移除后缀的结果
                match stripped_name {
                    Some(stripped) => {
                        // 步骤11：如果成功，返回剥离后的名称作为基名称
                        stripped.to_string()
                    }
                    None => {
                        // 步骤12：如果剥离失败（即后缀不匹配），则返回原始的最后一部分名称作为基名称
                        last_component_name.to_string()
                    }
                }
            }
        }
        None => {
            // 步骤13：如果没有最后一部分，则返回空字符串作为基名称
            String::new()
        }
    };

    // 步骤14：返回计算出的基名称
    result
}

