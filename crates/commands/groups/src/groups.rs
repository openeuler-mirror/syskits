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
};
use rust_i18n::t;
use std::error::Error;
use std::fmt::Display;
rust_i18n::i18n!("locales", fallback = "en-US");
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupsEntry {
    pub user: Option<String>,
    pub groups: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupsSemantic {
    pub entries: Vec<GroupsEntry>,
    pub classic_text: String,
    pub stderr_text: String,
    pub exit_code: i32,
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

impl Display for GroupsEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(user) = &self.user {
            write!(f, "{} : {}", user, self.groups.join(" "))
        } else {
            write!(f, "{}", self.groups.join(" "))
        }
    }
}

fn groups_infallible_gid2grp(gid: &u32, stderr_text: &mut String, exit_code: &mut i32) -> String {
    match gid2grp(*gid) {
        Ok(grp) => grp,
        Err(_) => {
            *exit_code = 1;
            stderr_text.push_str(&format!("groups: {}\n", GroupsError::GroupNotFound(*gid)));
            gid.to_string()
        }
    }
}

fn groups_render_classic_text(entries: &[GroupsEntry]) -> String {
    if entries.is_empty() {
        return String::new();
    }

    let mut classic_text = entries
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    classic_text.push('\n');
    classic_text
}

fn groups_collect_for_users(users: Vec<String>) -> CTResult<GroupsSemantic> {
    let mut entries = Vec::new();
    let mut stderr_text = String::new();
    let mut exit_code = 0;

    if users.is_empty() {
        let gids = get_groups_gnu(None).map_err(|_| GroupsError::GetGroupsFailed)?;
        let groups = gids
            .iter()
            .map(|gid| groups_infallible_gid2grp(gid, &mut stderr_text, &mut exit_code))
            .collect();
        entries.push(GroupsEntry { user: None, groups });
    } else {
        for user in users {
            match CtPasswd::locate(user.as_str()) {
                Ok(passwd) => {
                    let groups = passwd
                        .belongs_to()
                        .iter()
                        .map(|gid| groups_infallible_gid2grp(gid, &mut stderr_text, &mut exit_code))
                        .collect();
                    entries.push(GroupsEntry {
                        user: Some(user),
                        groups,
                    });
                }
                Err(_) => {
                    exit_code = 1;
                    stderr_text.push_str(&format!("groups: {}\n", GroupsError::UserNotFound(user)));
                }
            }
        }
    }

    let classic_text = groups_render_classic_text(&entries);
    Ok(GroupsSemantic {
        entries,
        classic_text,
        stderr_text,
        exit_code,
    })
}

pub fn groups_native_semantic(args: impl ctcore::Args) -> CTResult<GroupsSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let matches = ct_app().try_get_matches_from(args)?;
    let users: Vec<String> = matches
        .get_many::<String>(opt_flags::USERS)
        .map(|v| v.map(ToString::to_string).collect())
        .unwrap_or_default();

    groups_collect_for_users(users)
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
        let result = groups_native_semantic(args.iter().cloned());
        match result {
            Ok(semantic) => {
                if !semantic.classic_text.is_empty() {
                    print!("{}", semantic.classic_text);
                }
                if !semantic.stderr_text.is_empty() {
                    eprint!("{}", semantic.stderr_text);
                }

                if semantic.exit_code == 0 {
                    Ok(())
                } else {
                    Err(semantic.exit_code.into())
                }
            }
            Err(err) => {
                eprint!("{err}");
                Err(125.into())
            }
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
#[cfg(test)]
fn groups_main(args: impl ctcore::Args) -> CTResult<Vec<GroupsEntry>> {
    let semantic = groups_native_semantic(args)?;
    Ok(semantic.entries)
}

#[cfg(test)]
mod tests {
    mod tests_tool_implementation {
        use crate::Groups;
        use ctcore::Tool;
        use std::ffi::OsString;

        #[test]
        fn test_tool_implementation() {
            let tool = Groups;

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
        use crate::{groups_main, groups_native_semantic};

        use std::ffi::OsString;

        #[test]
        fn test_groups_main_version() {
            let args = [ctcore::ct_util_name(), "--version"];

            let result = groups_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_groups_main_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = groups_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_groups_main() {
            let args = [ctcore::ct_util_name()];
            let result = groups_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_groups_native_semantic_preserves_partial_output_and_diagnostics() {
            let args = [ctcore::ct_util_name(), "root", "no_such_user_123"];
            let result = groups_native_semantic(args.iter().map(OsString::from)).expect("semantic");

            assert_eq!(result.exit_code, 1);
            assert_eq!(result.classic_text, "root : root\n");
            assert_eq!(
                result.stderr_text,
                "groups: 'no_such_user_123': no such user\n"
            );
            assert_eq!(result.entries.len(), 1);
            assert_eq!(result.entries[0].user.as_deref(), Some("root"));
            assert_eq!(result.entries[0].groups, vec!["root".to_string()]);
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
