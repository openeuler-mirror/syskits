/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2.
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2.
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

// 用于显示用户所属的所有组。它对于理解用户权限和访问控制非常重要，因为用户所属的组决定了他们对系统资源的访问权限

extern crate rust_i18n;
use ctcore::{
    ct_display::Quotable,
    ct_entries::{CtPasswd, Locate, get_groups_gnu, gid2grp},
    ct_error::{CTError, CTResult},
    ct_show,
};
use rust_i18n::t;
use std::error::Error;
use std::fmt::Display;
rust_i18n::i18n!("locales", fallback = "zh-CN");
use clap::{Arg, ArgAction, Command, crate_version};
use ctcore::Tool;
use std::ffi::OsString;
use sys_locale::get_locale;

mod opt_flags {
    pub const USERS: &str = "USERNAME";
}

#[derive(Debug)]

enum GroupsError {
    GetGroupsFailed,
    GroupNotFound(u32),
    UserNotFound(String),
}

impl Error for GroupsError {}
impl CTError for GroupsError {}

impl Display for GroupsError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::GetGroupsFailed => write!(f, "failed to fetch groups"),
            Self::GroupNotFound(gid) => write!(f, "cannot find name for group ID {gid}"),
            Self::UserNotFound(user) => write!(f, "{}: no such user", user.quote()),
        }
    }
}

/**
 * 尝试将给定的组ID转换为组名。
 *
 * 使用 `gid2grp` 函数尝试查找与给定组ID对应的组名。如果找到，则返回该组名；
 * 如果未找到，则记录错误并返回给定的组ID的字符串表示形式。
 *
 * @param gid 组ID的引用，类型为 `&u32`。
 * @return `String` 类型，表示组名或组ID的字符串表示。
 */
fn groups_infallible_gid2grp(gid: &u32) -> String {
    // 尝试将组ID转换为组名
    match gid2grp(*gid) {
        Ok(grp) => grp, // 成功时返回组名
        Err(_) => {
            // 当转换失败时，使用 `ct_show!` 宏记录错误信息，并设置程序的全局退出码
            ct_show!(GroupsError::GroupNotFound(*gid));
            gid.to_string() // 将组ID转换为字符串并返回
        }
    }
}

pub fn ct_app() -> Command {
    Command::new(ctcore::ct_util_name())
        .version(crate_version!())
        .about(t!("groups.about"))
        .override_usage(t!("groups.usage"))
        .infer_long_args(true)
        .arg(
            Arg::new(opt_flags::USERS)
                .action(ArgAction::Append)
                .value_name(opt_flags::USERS)
                .value_hint(clap::ValueHint::Username),
        )
}

#[derive(Debug)]
struct GroupInfo {
    user: String,
    groups: Vec<String>,
}

impl Display for GroupInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.user.is_empty() {
            write!(f, "{}", self.groups.join(" "))
        } else {
            write!(f, "{} : {}", self.user, self.groups.join(" "))
        }
    }
}

#[derive(Default)]
pub struct Groups;
impl Tool for Groups {
    fn name(&self) -> &'static str {
        "groups"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        let result = groups_main(args.iter().cloned());
        match result {
            Ok(groups) => {
                for g in groups.iter() {
                    println!("{}", g);
                }

                Ok(())
            }
            _ => {
                // 如果出现错误，则打印错误信息并返回错误
                eprint!("{}", result.err().unwrap());
                Err(125.into())
            }
        }
    }
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    let result = groups_main(args);
    match result {
        Ok(groups) => {
            for g in groups.iter() {
                println!("{}", g);
            }

            Ok(())
        }
        _ => {
            // 如果出现错误，则打印错误信息并返回错误
            eprint!("{}", result.err().unwrap());
            Err(125.into())
        }
    }
}

/// 用于处理用户指定的用户组信息。
///
/// # 参数
/// `args` - 实现了 `ctcore::Args` 接口的对象，用于接收命令行参数。
///
/// # 返回值
/// 返回一个 `CTResult<()>`，成功时为 `Ok(())`，失败时为 `Err` 包含错误信息。
fn groups_main(args: impl ctcore::Args) -> CTResult<Vec<GroupInfo>> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    // 从命令行参数中解析匹配项
    let matches = ct_app().try_get_matches_from(args)?;
    let mut g = Vec::new();
    // 尝试从命令行参数中获取用户列表，如果未指定则默认为空
    let users: Vec<String> = matches
        .get_many::<String>(opt_flags::USERS)
        .map(|v| v.map(ToString::to_string).collect())
        .unwrap_or_default();

    // 如果未指定用户，则列出当前系统中所有用户组
    if users.is_empty() {
        let gids = match get_groups_gnu(None) {
            Ok(v) => v,
            Err(_) => return Err(GroupsError::GetGroupsFailed.into()),
        };
        // 将组ID转换为组名，并打印
        let groups: Vec<String> = gids.iter().map(groups_infallible_gid2grp).collect();
        // println!("{}", groups.join(" "));

        let mut group_info = GroupInfo {
            user: String::new(),
            groups: Vec::new(),
        };

        group_info.groups = groups;

        g.push(group_info);

        return Ok(g);
    }

    let mut g = Vec::new();
    // 处理指定用户的组信息
    for user in users {
        match CtPasswd::locate(user.as_str()) {
            Ok(p) => {
                // 获取用户所属的组，并打印
                let groups: Vec<String> = p
                    .belongs_to()
                    .iter()
                    .map(groups_infallible_gid2grp)
                    .collect();
                // println!("{} : {}", user, groups.join(" "));

                let mut group_info = GroupInfo {
                    user: String::new(),
                    groups: Vec::new(),
                };

                group_info.user = user;
                group_info.groups = groups;

                g.push(group_info);
            }
            Err(_) => {
                // 如果用户不存在，则显示错误信息并设置退出码
                ct_show!(GroupsError::UserNotFound(user));
            }
        }
    }

    Ok(g)
}

#[cfg(test)]
mod tests {
    mod tests_tool_implementation {
        use crate::Groups;
        use ctcore::Tool;
        use std::ffi::OsString;

        #[test]
        fn test_tool_implementation() {
            let tool = Groups::default();

            // 测试 name 方法
            assert_eq!(tool.name(), "groups");

            // 测试 command 方法
            let command = tool.command();
            assert!(command.get_name().contains("groups"));

            // 测试 execute 方法
            let args = vec![OsString::from("groups"), OsString::from("--help")];
            assert!(tool.execute(&args).is_err()); // --help参数通常会返回错误
        }
    }

    mod tests_groups_main {
        use crate::groups_main;

        use std::ffi::OsString;

        #[test]
        fn test_groups_main_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];

            let result = groups_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_groups_main_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = groups_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_groups_main() {
            let args = vec![ctcore::ct_util_name()];
            let result = groups_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }
    }

    mod tests_ct_app {
        use crate::ct_app;

        use clap::error::ErrorKind;

        #[test]
        fn test_ct_app_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_groups() {
            let args = vec![ctcore::ct_util_name()];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }
    }
}
