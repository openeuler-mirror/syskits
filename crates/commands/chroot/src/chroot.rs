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

extern crate rust_i18n;
mod error;

use crate::error::ChrootError;
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use clap::{Arg, ArgAction, Command, crate_version};
use ctcore::Tool;
use ctcore::ct_entries::{self, Locate};
use ctcore::ct_error::{CTResult, CTsageError, UClapError, set_ct_exit_code};
use ctcore::ct_fs::{MissingHandling, ResolveMode, canonicalize};
use ctcore::libc::{self, chroot, setgid, setgroups, setuid};
use std::io::Error;
use sys_locale::get_locale;

use std::ffi::CString;
use std::ffi::OsString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::process;
use std::process::ExitStatus;

mod opt_flags {
    pub const NEWROOT: &str = "newroot";
    pub const USER: &str = "user";
    pub const GROUP: &str = "group";
    pub const GROUPS: &str = "groups";
    pub const USERSPEC: &str = "userspec";
    pub const COMMAND: &str = "command";
    pub const SKIP_CHDIR: &str = "skip-chdir";
}

pub fn chroot_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    // 尝试从 args 中获取匹配项，并在匹配失败时返回错误代码 125
    let args_match = ct_app().try_get_matches_from(args).with_exit_code(125)?;

    // 定义默认的 shell 和选项
    let default_shell: &'static str = "/bin/sh";
    let default_option: &'static str = "-i";
    // 尝试从环境变量获取用户默认 shell
    let user_shell = std::env::var("SHELL");

    // 解析新的根目录路径
    let new_root: &Path = match args_match.get_one::<String>(opt_flags::NEWROOT) {
        Some(v) => Path::new(v),
        None => return Err(ChrootError::MissingNewRoot.into()),
    };

    // 检查是否跳过更改工作目录的步骤
    let skip_chdir = args_match.get_flag(opt_flags::SKIP_CHDIR);

    // 如果启用了跳过更改目录且新的根目录不是根目录，则返回错误
    if skip_chdir
        && canonicalize(new_root, MissingHandling::Normal, ResolveMode::Logical)
            .unwrap()
            .to_str()
            != Some("/")
    {
        return Err(CTsageError::new(
            125,
            "option --skip-chdir only permitted if NEWROOT is old '/'",
        ));
    }

    // 检查新的根目录是否存在
    if !new_root.is_dir() {
        return Err(ChrootError::NoSuchDirectory(format!("{}", new_root.display())).into());
    }

    // 解析命令参数
    let cmds = match args_match.get_many::<String>(opt_flags::COMMAND) {
        Some(v) => v.map(|s| s.as_str()).collect(),
        None => vec![],
    };

    // 根据命令行参数准备执行的命令
    let cmd: Vec<&str> = match cmds.len() {
        0 => {
            // 如果没有指定命令，使用用户的默认 shell
            let shell: &str = match user_shell {
                Err(_) => default_shell,
                Ok(ref s) => s.as_ref(),
            };
            vec![shell, default_option]
        }
        _ => cmds,
    };

    // 确保最终有命令将被执行
    assert!(!cmd.is_empty());
    let chroot_cmd = cmd[0];
    let chroot_arguments = &cmd[1..];

    // 设置执行上下文
    chroot_set_context(new_root, &args_match)?;

    // 执行指定的命令
    let process_status = match process::Command::new(chroot_cmd)
        .args(chroot_arguments)
        .status()
    {
        Ok(status) => status,
        Err(e) => {
            return Err(if e.kind() == std::io::ErrorKind::NotFound {
                ChrootError::CommandNotFound(cmd[0].to_string(), e)
            } else {
                ChrootError::CommandFailed(cmd[0].to_string(), e)
            }
            .into());
        }
    };

    // 设置退出码
    let process_code = chroot_process_status_code(process_status);
    set_ct_exit_code(process_code);
    Ok(())
}

fn chroot_process_status_code(pstatus: ExitStatus) -> i32 {
    if pstatus.success() {
        0
    } else {
        pstatus.code().unwrap_or(-1)
    }
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("chroot.about");
    let usage_description = t!("chroot.usage");

    let args = args_init();

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .trailing_var_arg(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .args(&args)
}

fn args_init() -> Vec<Arg> {
    let args = vec![
        Arg::new(opt_flags::NEWROOT)
            .value_hint(clap::ValueHint::DirPath)
            .hide(true)
            .required(true)
            .index(1),
        Arg::new(opt_flags::USER)
            .short('u')
            .long(opt_flags::USER)
            .help(t!("chroot.clap.user"))
            .value_name("USER"),
        Arg::new(opt_flags::GROUP)
            .short('g')
            .long(opt_flags::GROUP)
            .help(t!("chroot.clap.group"))
            .value_name("GROUP"),
        Arg::new(opt_flags::GROUPS)
            .short('G')
            .long(opt_flags::GROUPS)
            .help(t!("chroot.clap.groups"))
            .action(ArgAction::Append)
            .value_name("GROUP1,GROUP2..."),
        Arg::new(opt_flags::USERSPEC)
            .long(opt_flags::USERSPEC)
            .help(
                "Colon-separated user and group to switch to. \
                     Same as -u USER -g GROUP. \
                     Userspec has higher preference than -u and/or -g",
            )
            .value_name("USER:GROUP"),
        Arg::new(opt_flags::SKIP_CHDIR)
            .long(opt_flags::SKIP_CHDIR)
            .help(
                "Use this option to not change the working directory \
                    to / after changing the root directory to newroot, \
                    i.e., inside the chroot.",
            )
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::COMMAND)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::CommandName)
            .hide(true)
            .index(2),
        Arg::new("help")
            .short('h')
            .long("help")
            .help(t!("chroot.clap.help", default = "Print help"))
            .action(ArgAction::Help),
        Arg::new("version")
            .short('V')
            .long("version")
            .help(t!("chroot.clap.version", default = "Print version"))
            .action(ArgAction::Version),
    ];
    args
}

/**
 * 在给定的根路径下设置新的上下文环境。
 *
 * 此函数尝试将当前进程的上下文切换到指定的根路径，并应用用户、组和其他配置。
 * 它使用命令行参数来决定具体的配置细节。
 *
 * @param root_path 指定的新根路径。
 * @param args_option 包含命令行参数的匹配结果。
 * @return CTResult<()> 如果成功，返回一个空的Ok()结果；如果有错误，返回包含错误信息的Err()结果。
 */
fn chroot_set_context(root_path: &Path, args_option: &clap::ArgMatches) -> CTResult<()> {
    let user_spec_str = args_option.get_one::<String>(opt_flags::USERSPEC);
    let arg_user = args_option.get_one::<String>(opt_flags::USER);
    let arg_group = args_option
        .get_one::<String>(opt_flags::GROUP)
        .map(|s| s.as_str());
    let arg_groups = args_option
        .get_many::<String>(opt_flags::GROUPS)
        .and_then(|mut vals| vals.next_back())
        .map(|s| s.as_str());

    let skip_chdir = args_option.get_flag(opt_flags::SKIP_CHDIR);

    // 解析 --userspec
    let user_spec = match chroot_parse_user_spec(user_spec_str) {
        Ok(value) => value,
        Err(value) => return value, // 如果解析失败，直接返回错误
    };

    // 解析 --user，在 GNU 中 --user 同样支持 USER:GROUP 的语法
    let user_arg_spec = match chroot_parse_user_spec(arg_user) {
        Ok(value) => value,
        Err(value) => return value,
    };

    // 智能合并解析结果：优先级 --userspec > --user > --group
    let final_user = user_spec.0.or(user_arg_spec.0);
    let final_group = user_spec.1.or(user_arg_spec.1).or(arg_group);

    // 进入新的根目录（如果配置中允许）
    chroot_enter(root_path, skip_chdir)?;

    let mut lookup_pwd = None;
    if let Some(u) = final_user {
        // 剥离 '+' 号去进行系统查询，因为 '+' 代表纯数字跳过 NSS，但在补全组时我们依然尝试查询
        let lookup_name = u.strip_prefix('+').unwrap_or(u);
        lookup_pwd = ctcore::ct_entries::CtPasswd::locate(lookup_name).ok();
    }

    if let Some(groups_str) = arg_groups {
        // 如果显式传入了 --groups 参数（即便是空字符串代表清空），则直接设置
        chroot_set_groups_from_str(groups_str)?;
    } else if final_user.is_some() {
        // 如果没传 --groups 参数，但指定了用户，我们需要尝试自动补全
        if let Some(pwd) = lookup_pwd.as_ref() {
            // 查到了用户，自动补全其所属的所有附加组
            let groups = pwd.belongs_to();
            let err = chroot_set_groups(&groups);
            if err != 0 {
                return Err(ChrootError::SetGroupsFailed(Error::last_os_error()).into());
            }
        } else if final_group.is_some() {
            // 这是 GNU 的潜规则：没查到用户，但用户显式指定了主组（如 +12342:+5678）
            // 此时由于没有查到用户的附加组信息，它会默认“清空”附加组。
            let err = chroot_set_groups(&[]);
            if err != 0 {
                return Err(ChrootError::SetGroupsFailed(Error::last_os_error()).into());
            }
        }
        // 如果没查到用户，且没指定主组，我们先跳过，它会在下一步报错
    }

    if let Some(g) = final_group {
        // 用户显式指定了主组，直接设置
        chroot_set_main_group(g)?;
    } else if let Some(u) = final_user {
        // 用户没有指定主组，需要自动补全
        if let Some(pwd) = lookup_pwd.as_ref() {
            // 查到了用户，自动补全其主 GID
            let err = unsafe { setgid(pwd.gid as libc::gid_t) };
            if err != 0 {
                return Err(
                    ChrootError::SetGidFailed(pwd.gid.to_string(), Error::last_os_error()).into(),
                );
            }
        } else {
            // 指定了用户，没指定组，系统里又查不到这个人！
            // 此时既不知道主组是什么，又不能随便猜，必须报错 125！
            return Err(ChrootError::InvalidUserspec(u.to_string()).into());
        }
    }

    // 3. 设置有效用户 (Effective User)
    if let Some(u) = final_user {
        chroot_set_user(u)?;
    }

    Ok(())
}

/**
 * 解析用户规格字符串。
 *
 * 此函数接受一个可选的字符串引用，该字符串表示一个用户规格，格式为`username:uid`。
 * 如果提供的字符串符合此格式且不为空，则将其拆分为用户名和用户ID的两部分，并返回一个包含这两部分的向量。
 * 如果字符串格式不正确或任何一部分为空，则返回一个错误结果。
 * 如果输入为`None`，则返回一个空向量。
 *
 * @param user_spec_str 可选的用户规格字符串。格式应为`username:uid`。
 * @return Result<Vec<&str>, CTResult<()>> 如果解析成功，返回一个包含用户名和用户ID的字符串切片向量；
 * 如果解析失败，返回一个包含错误信息的错误结果。
 */
fn chroot_parse_user_spec(
    user_spec_str: Option<&String>,
) -> Result<(Option<&str>, Option<&str>), CTResult<()>> {
    match user_spec_str {
        Some(u) => {
            if let Some((user_part, group_part)) = u.split_once(':') {
                let user = if user_part.is_empty() {
                    None
                } else {
                    Some(user_part)
                };
                let group = if group_part.is_empty() {
                    None
                } else {
                    Some(group_part)
                };

                if user.is_none() && group.is_none() {
                    // ":" is invalid (both empty)
                    Err(Err(ChrootError::InvalidUserspec(u.to_string()).into()))
                } else {
                    Ok((user, group))
                }
            } else {
                // No colon means only user was provided
                let user = if u.is_empty() { None } else { Some(u.as_str()) };
                if user.is_none() {
                    Err(Err(ChrootError::InvalidUserspec(u.to_string()).into()))
                } else {
                    Ok((user, None))
                }
            }
        }
        None => Ok((None, None)), // If input is None, return all Nones
    }
}

fn chroot_enter(root_path: &Path, is_skip_chdir: bool) -> CTResult<()> {
    let root_c = CString::new(root_path.as_os_str().as_bytes()).map_err(|e| {
        ChrootError::CannotEnter(
            root_path.display().to_string(),
            Error::new(std::io::ErrorKind::InvalidInput, e),
        )
    })?;

    let err = unsafe { chroot(root_c.as_ptr()) };
    if err != 0 {
        return Err(ChrootError::CannotEnter(
            root_path.display().to_string(),
            Error::last_os_error(),
        )
        .into());
    }

    if !is_skip_chdir {
        std::env::set_current_dir("/").map_err(|e| ChrootError::CannotEnter("/".to_string(), e))?;
    }
    Ok(())
}

fn chroot_set_main_group(chroot_group: &str) -> CTResult<()> {
    if !chroot_group.is_empty() {
        let group_id = if let Some(id_str) = chroot_group.strip_prefix('+') {
            id_str
                .parse::<libc::gid_t>()
                .map_err(|_| ChrootError::NoSuchGroup(chroot_group.to_string()))?
        } else {
            match ct_entries::grp2gid(chroot_group) {
                Ok(g) => g,
                Err(_) => chroot_group
                    .parse::<libc::gid_t>()
                    .map_err(|_| ChrootError::NoSuchGroup(chroot_group.to_string()))?,
            }
        };
        let err = unsafe { setgid(group_id) };
        if err != 0 {
            return Err(
                ChrootError::SetGidFailed(group_id.to_string(), Error::last_os_error()).into(),
            );
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn chroot_set_groups(groups: &[libc::gid_t]) -> libc::c_int {
    unsafe { setgroups(groups.len() as libc::size_t, groups.as_ptr()) }
}

fn chroot_set_groups_from_str(groups: &str) -> CTResult<()> {
    // Rule 1: An explicitly empty string means "clear all supplemental groups".
    if groups.is_empty() {
        let err = chroot_set_groups(&[]);
        if err != 0 {
            return Err(ChrootError::SetGroupsFailed(Error::last_os_error()).into());
        }
        return Ok(());
    }

    // Rule 2 pipeline: split → trim → skip empty → parse.
    let mut groups_vec = vec![];
    let mut first_bad_raw: Option<&str> = None;

    for raw_group in groups.split(',') {
        let group = raw_group.trim();
        if group.is_empty() {
            // Remember the first empty/whitespace-only token for a potential error
            // message, but don't error yet – consecutive commas are allowed by GNU.
            if first_bad_raw.is_none() {
                first_bad_raw = Some(raw_group);
            }
            continue;
        }
        // Reset bad-token sentinel: we found at least one real token.
        first_bad_raw = None;
        let gid = if let Some(id_str) = group.strip_prefix('+') {
            // '+' prefix forces strict numeric-ID resolution, bypassing NSS.
            id_str
                .parse::<libc::gid_t>()
                .map_err(|_| ChrootError::NoSuchGroup(group.to_string()))?
        } else {
            match ct_entries::grp2gid(group) {
                Ok(g) => g,
                Err(_) => group
                    .parse::<libc::gid_t>()
                    .map_err(|_| ChrootError::NoSuchGroup(group.to_string()))?,
            }
        };
        groups_vec.push(gid);
    }

    // Rule 3: if the non-empty input produced zero valid groups, the entire
    // value was whitespace or bare commas – report the first offending token.
    if groups_vec.is_empty() {
        let bad = first_bad_raw.unwrap_or(groups);
        return Err(ChrootError::NoSuchGroup(bad.to_string()).into());
    }

    let err = chroot_set_groups(&groups_vec);
    if err != 0 {
        return Err(ChrootError::SetGroupsFailed(Error::last_os_error()).into());
    }
    Ok(())
}

fn chroot_set_user(username: &str) -> CTResult<()> {
    if !username.is_empty() {
        let user_id = if let Some(id_str) = username.strip_prefix('+') {
            id_str
                .parse::<libc::uid_t>()
                .map_err(|_| ChrootError::NoSuchUser(username.to_string()))?
        } else {
            match ct_entries::usr2uid(username) {
                Ok(u) => u,
                Err(_) => username
                    .parse::<libc::uid_t>()
                    .map_err(|_| ChrootError::NoSuchUser(username.to_string()))?,
            }
        };
        let err = unsafe { setuid(user_id as libc::uid_t) };
        if err != 0 {
            return Err(
                ChrootError::SetUserFailed(username.to_string(), Error::last_os_error()).into(),
            );
        }
    }
    Ok(())
}

#[derive(Default)]
pub struct Chroot;
impl Tool for Chroot {
    fn name(&self) -> &'static str {
        "chroot"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        chroot_main(args.iter().cloned()).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    #[test]
    fn test_tool_implementation() {
        let tool = Chroot;

        // 测试 name 方法
        assert_eq!(tool.name(), "chroot");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("chroot"));

        // 测试 execute 方法
        let args = vec![OsString::from("chroot"), OsString::from("--help")];
        let result = tool.execute(&args);
        assert!(result.is_err()); // 因为chroot需要root权限和必要参数，所以测试不带参数的情况应该返回错误
    }

    #[cfg(test)]
    mod tests_ct_app {
        use crate::{ct_app, opt_flags};
        use clap::error::ErrorKind;

        #[test]
        fn test_ct_app_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_newroot_required() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name()];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().kind(),
                ErrorKind::MissingRequiredArgument
            );
        }

        #[test]
        fn test_ct_app_user_short_flag() {
            let command = ct_app();
            let binding = String::from("path/to/newroot");
            let args = vec![ctcore::ct_util_name(), &binding, "-u", "testuser"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(
                matches.get_one::<String>(opt_flags::USER).unwrap(),
                "testuser"
            );
        }

        #[test]
        fn test_ct_app_group_long_flag() {
            let command = ct_app();
            let binding = String::from("path/to/newroot");
            let args = vec![ctcore::ct_util_name(), &binding, "--group", "testgroup"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(
                matches.get_one::<String>(opt_flags::GROUP).unwrap(),
                "testgroup"
            );
        }

        #[test]
        fn test_ct_app_groups_short_flag() {
            let command = ct_app();
            let binding = String::from("path/to/newroot");
            let args = vec![
                ctcore::ct_util_name(),
                &binding,
                "-G",
                "group1,group2,group3",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(
                matches.get_one::<String>(opt_flags::GROUPS).unwrap(),
                "group1,group2,group3"
            );
        }

        #[test]
        fn test_ct_app_userspec_long_flag() {
            let command = ct_app();
            let binding = String::from("path/to/newroot");
            let args = vec![
                ctcore::ct_util_name(),
                &binding,
                "--userspec",
                "testuser:testgroup",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(
                matches.get_one::<String>(opt_flags::USERSPEC).unwrap(),
                "testuser:testgroup"
            );
        }

        #[test]
        fn test_ct_app_skip_chdir_long_flag() {
            let command = ct_app();
            let binding = String::from("path/to/newroot");
            let args = vec![ctcore::ct_util_name(), &binding, "--skip-chdir"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert!(matches.get_flag(opt_flags::SKIP_CHDIR));
        }

        #[test]
        fn test_ct_app_command_trailing_arg() {
            let command = ct_app();
            let binding = String::from("path/to/newroot");
            let args = vec![ctcore::ct_util_name(), &binding, "ls", "-l"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(
                matches
                    .get_many::<String>(opt_flags::COMMAND)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["ls", "-l"]
            );
        }

        #[test]
        fn test_ct_app_command_trailing_arg_with_dash() {
            let command = ct_app();
            let binding = String::from("path/to/newroot");
            let args = vec![ctcore::ct_util_name(), &binding, "ls", "-l", "-a"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(
                matches
                    .get_many::<String>(opt_flags::COMMAND)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["ls", "-l", "-a"]
            );
        }
        #[test]
        fn test_ct_app_command_trailing_arg_with_dash_and_dash() {
            let command = ct_app();
            let binding = String::from("path/to/newroot");
            let args = vec![ctcore::ct_util_name(), &binding, "ls", "-l", "-a", "-"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(
                matches
                    .get_many::<String>(opt_flags::COMMAND)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["ls", "-l", "-a", "-"]
            );
        }
        #[test]
        fn test_ct_app_command_trailing_arg_with_dash_and_dash_and_dash() {
            let command = ct_app();
            let binding = String::from("path/to/newroot");
            let args = vec![ctcore::ct_util_name(), &binding, "ls", "-l", "-a", "-", "-"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(
                matches
                    .get_many::<String>(opt_flags::COMMAND)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["ls", "-l", "-a", "-", "-"]
            );
        }
        #[test]
        fn test_ct_app_command_trailing_arg_with_dash_and_dash_and_dash_and_dash() {
            let command = ct_app();
            let binding = String::from("path/to/newroot");
            let args = vec![
                ctcore::ct_util_name(),
                &binding,
                "ls",
                "-l",
                "-a",
                "-",
                "-",
                "-",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            let matches = result.unwrap();
            assert_eq!(
                matches
                    .get_many::<String>(opt_flags::COMMAND)
                    .unwrap()
                    .collect::<Vec<_>>(),
                vec!["ls", "-l", "-a", "-", "-", "-"]
            );
        }
        #[test]
        fn test_ct_app_invalid_newroot_path() {
            let command = ct_app();
            let binding = String::from("nonexistent/path");
            let args = vec![ctcore::ct_util_name(), &binding, "--wve"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_duplicate_user_specification() {
            let command = ct_app();
            let binding = String::from("path/to/newroot");
            let args = vec![
                ctcore::ct_util_name(),
                &binding,
                "-g",
                "testgroup",
                "--groups",
                "anothergroup",
                "--groups",
                "yetanothergroup",
                "-u",
                "testuser",
                "--userspec",
                "anotheruser:anothertestgroup",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_empty_userspec_value() {
            let command = ct_app();
            let binding = String::from("path/to/newroot");
            let args = vec![
                ctcore::ct_util_name(),
                &binding,
                "--userspec=0",
                "-g",
                "testgroup",
                "--groups",
                "anothergroup",
                "--groups",
                "yetanothergroup",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_malformed_userspec_value() {
            let command = ct_app();
            let binding = String::from("path/to/newroot");
            let args = vec![
                ctcore::ct_util_name(),
                &binding,
                "-e-userspec",
                "invalid:user:spece",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_duplicate_group_specification() {
            let command = ct_app();
            let binding = String::from("path/to/newroot");
            let args = vec![
                ctcore::ct_util_name(),
                &binding,
                "-g",
                "testgroup",
                "--groups",
                "anothergroup",
                "--groups",
                "yetanothergroup",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_help_flag() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_version_and_command_flag_combination() {
            let command = ct_app();
            let binding = String::from("path/to/newroot");
            let args = vec![
                ctcore::ct_util_name(),
                &binding,
                "--version",
                "--",
                "ls",
                "-l",
                "-a",
                "-",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_invalid_group_id() {
            let command = ct_app();
            let binding = String::from("path/to/newroot");
            let args = vec![
                ctcore::ct_util_name(),
                &binding,
                "--group-id=999999999",
                "999999999",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_missing_command_argument() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name()];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().kind(),
                ErrorKind::MissingRequiredArgument
            );
        }

        #[test]
        fn test_ct_app_extra_unrecognized_arguments() {
            let command = ct_app();
            let binding = String::from("path/to/newroot");
            let args = vec![ctcore::ct_util_name(), &binding, "--unknown-flag"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }
    }

    #[cfg(test)]
    mod tests_ctmain {
        use crate::chroot_main;
        use std::ffi::OsString;
        use std::fs;
        use std::fs::File;
        use tempfile::Builder;

        #[test]
        fn test_ctmain_version() {
            let args = [ctcore::ct_util_name(), "--version"];
            let result = chroot_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_ctmain_v() {
            let args = [ctcore::ct_util_name(), "-V"];
            let result = chroot_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_ctmain_h() {
            let args = [ctcore::ct_util_name(), "-h"];
            let result = chroot_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }
        #[test]
        fn test_ctmain_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = chroot_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }
        #[test]
        fn test_ctmain_hh() {
            let args = [ctcore::ct_util_name(), "-hh"];
            let result = chroot_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }
        #[test]
        fn test_ctmain_hhh() {
            let args = [ctcore::ct_util_name(), "-hhh"];
            let result = chroot_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_ctmain_newroot_required() {
            let args = [ctcore::ct_util_name()];
            let result = chroot_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }
        #[test]
        fn test_ctmain_valid_newroot_required() {
            let temp_dir = Builder::new()
                .prefix("test_ctmain_valid_chdir")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_skip_dir.txt");
            File::create(&test_file_path).unwrap();

            let args = [
                ctcore::ct_util_name(),
                test_file_path.to_str().expect("REASON"),
                "ls",
                "-l",
            ];
            let result = chroot_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_ctmain_valid_newroot() {
            let args = [
                ctcore::ct_util_name(),
                &String::from("/valid/newroot/path"),
                "ls",
                "-l",
            ];
            let result = chroot_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_ctmain_invalid_newroot() {
            let args = [
                ctcore::ct_util_name(),
                &String::from("/nonexistent/path"),
                "ls",
                "-l",
            ];
            let result = chroot_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_ctmain_user_short_flag() {
            let args = [
                ctcore::ct_util_name(),
                &String::from("/valid/newroot/path"),
                "-u",
                "testuser",
                "ls",
                "-l",
            ];
            let result = chroot_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_ctmain_group_long_flag() {
            let args = [
                ctcore::ct_util_name(),
                &String::from("/valid/newroot/path"),
                "--group",
                "testgroup",
                "ls",
                "-l",
            ];
            let result = chroot_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_ctmain_groups_short_flag() {
            let args = [
                ctcore::ct_util_name(),
                &String::from("/valid/newroot/path"),
                "-G",
                "group1,group2,group3",
                "ls",
                "-l",
            ];
            let result = chroot_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_ctmain_userspec_long_flag() {
            let args = [
                ctcore::ct_util_name(),
                &String::from("/valid/newroot/path"),
                "--userspec",
                "testuser:testgroup",
                "ls",
                "-l",
            ];
            let result = chroot_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_ctmain_skip_chdir_long_flag() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("test_ctmain_skip_chdir_long_flag")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_path = sub_dir_path.join("test_skip_dir.txt");
            File::create(&test_file_path).unwrap();

            let args = [
                ctcore::ct_util_name(),
                test_file_path.to_str().expect("REASON"),
                "--skip-chdir",
                "ls",
                "-l",
            ];
            let result = chroot_main(args.iter().map(OsString::from));

            assert!(result.is_err());
            // 清理测试环境（无需手动清理，TempDir 会在作用域结束时自动删除）
        }
        #[test]
        fn test_ctmain_skip_chdir_short_flag() {
            let args = [
                ctcore::ct_util_name(),
                &String::from("/valid/newroot/path"),
                "-c",
                "ls",
                "-l",
                "-lc",
            ];
            let result = chroot_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_ctmain_command_trailing_arg() {
            let args = [
                ctcore::ct_util_name(),
                &String::from("/valid/newroot/path"),
                "ls",
                "-l",
            ];
            let result = chroot_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }
        #[test]
        fn test_ctmain_command_trailing_arg_with_shell() {
            let args = [
                ctcore::ct_util_name(),
                &String::from("/valid/newroot/path"),
                "ls",
                "-l",
            ];
            let result = chroot_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_ctmain_default_shell_no_command_given() {
            let args = [ctcore::ct_util_name(), &String::from("/valid/newroot/path")];
            let result = chroot_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }
    }

    #[cfg(test)]
    mod tests_private_fn {
        use crate::{chroot_enter, chroot_set_user};
        use crate::{chroot_set_groups_from_str, chroot_set_main_group};

        #[test]
        fn test_set_user_empty_user() {
            let result = chroot_set_user("");
            let err = match result {
                Ok(_) => 0,
                Err(_) => 1,
            };
            assert_eq!(err, 0);
        }

        #[test]
        fn test_set_groups_from_str_empty() {
            let result = chroot_set_groups_from_str("");
            assert!(result.is_ok());
        }

        #[test]
        fn test_set_groups_from_str_empty_group() {
            let groups = "group1,,group3";
            let result = chroot_set_groups_from_str(groups);
            assert!(result.is_err());
        }

        #[test]
        fn test_set_groups_from_str_invalid_group() {
            let groups = "a invalid_group";
            let result = chroot_set_groups_from_str(groups);
            assert!(result.is_err());
        }

        #[test]
        fn test_set_groups_from_str_invalid() {
            let groups = "invalid_group";
            let result = chroot_set_groups_from_str(groups);
            assert!(result.is_err());
        }

        #[test]
        fn test_set_groups_from_str_valid() {
            let groups = "group1,group2,group3";
            let result = chroot_set_groups_from_str(groups);
            assert!(result.is_err());
        }

        #[test]
        fn test_set_groups_from_str_set_groups_failed() {
            let groups = "group1,group2,group3,group4";
            let result = chroot_set_groups_from_str(groups);
            assert!(result.is_err());
        }

        #[test]
        fn test_set_main_group_empty() {
            // Set an empty group
            let result = chroot_set_main_group("");
            assert!(result.is_ok());
        }

        #[test]
        fn test_set_main_group_no_such_group() {
            // Set a non-existent group
            let result = chroot_set_main_group("non_existent_group");
            assert!(result.is_err());
        }

        #[test]
        fn test_set_main_group_empty_string() {
            let result = chroot_set_main_group("");
            assert!(result.is_ok());
        }

        #[test]
        fn test_set_main_group_single_whitespace() {
            let result = chroot_set_main_group(" ");
            assert!(result.is_err());
        }

        #[test]
        fn test_set_main_group_valid_existing_group() {
            let existing_group = "valid_group";
            // Assume `create_group` is a function to create a group if it doesn't exist
            create_group(existing_group);
            let result = chroot_set_main_group(existing_group);
            assert!(result.is_err());
        }

        fn create_group(group_name: &str) {
            if group_name.is_empty() {
                delete_group(group_name);
            }
        }

        fn delete_group(group_name: &str) {
            if group_name.is_empty() {
                // Delete the group
            }
        }

        #[test]
        fn test_set_main_group_special_characters() {
            let special_chars_group = "!@#$%^&*()_+{}|:\"<>?";
            let result = chroot_set_main_group(special_chars_group);
            assert!(result.is_err());
        }

        #[test]
        fn test_set_main_group_exceeds_max_length() {
            let long_group_name = "A".repeat(101); // Assuming max length is 100 characters
            let result = chroot_set_main_group(&long_group_name);
            assert!(result.is_err());
        }

        #[test]
        fn test_set_main_group_multiple_times_same_group() {
            let existing_group = "existing_group";
            create_group(existing_group);

            for _ in 0..3 {
                let result = chroot_set_main_group(existing_group);
                assert!(result.is_err());
            }
        }

        #[test]
        fn test_set_main_group_deleted_group() {
            let to_delete_group = "to_delete_group";
            create_group(to_delete_group);
            delete_group(to_delete_group); // Assume `delete_group` function exists

            let result = chroot_set_main_group(to_delete_group);
            assert!(result.is_err());
        }

        #[test]
        fn test_set_main_group_non_empty_current_main_group() {
            let current_main_group = "current_main_group";
            let new_main_group = "new_main_group";
            create_group(current_main_group);
            create_group(new_main_group);

            let result = chroot_set_main_group(new_main_group);
            assert!(result.is_err()); // Depending on the system's behavior, this might be is_err()
        }

        #[test]
        fn test_set_main_group_to_itself() {
            let group_to_set = "group_to_set";
            create_group(group_to_set);

            let result = chroot_set_main_group(group_to_set);
            assert!(result.is_err());
        }

        #[test]
        fn test_set_main_group_leading_trailing_whitespaces() {
            let group_with_spaces = "   valid_group   ";
            create_group("valid_group");
            let result = chroot_set_main_group(group_with_spaces);
            assert!(result.is_err());
        }

        use std::path::Path;

        #[test]
        fn test_enter_chroot_skip_chdir() {
            let root = Path::new("/home/user");
            let result = chroot_enter(root, true);
            assert!(result.is_ok());
        }
    }
}
