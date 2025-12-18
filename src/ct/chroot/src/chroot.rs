/*
 *    Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved
 *     syskits is licensed under Mulan PSL v2.
 *    You can use this software according to the terms and conditions of the Mulan PSL V2
 *    You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 *    THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 *    KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 *    NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 *    See the Mulan PSL v2 for more details.
 */

mod error;

use crate::error::ChrootError;
use clap::{crate_version, Arg, ArgAction, Command};
use ctcore::ct_error::{set_ct_exit_code, CTResult, CTsageError, UClapError};
use ctcore::ct_fs::{canonicalize, MissingHandling, ResolveMode};
use ctcore::libc::{self, chroot, setgid, setgroups, setuid};
use ctcore::{ct_entries, ct_format_usage, ct_help_about, ct_help_usage};
use libc::c_int;
use std::ffi::CString;
use std::io::Error;
use std::os::unix::prelude::OsStrExt;
use std::path::Path;
use std::process;
use std::process::ExitStatus;

static CHROOT_ABOUT: &str = ct_help_about!("chroot.md");
static CHROOT_USAGE: &str = ct_help_usage!("chroot.md");

mod opt_flags {
    pub const NEWROOT: &str = "newroot";
    pub const USER: &str = "user";
    pub const GROUP: &str = "group";
    pub const GROUPS: &str = "groups";
    pub const USERSPEC: &str = "userspec";
    pub const COMMAND: &str = "command";
    pub const SKIP_CHDIR: &str = "skip-chdir";
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
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
            .into())
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
    let application_info = CHROOT_ABOUT;
    let usage_description = ct_format_usage(CHROOT_USAGE);

    let args = args_init();

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .trailing_var_arg(true)
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
            .help("User (ID or name) to switch before running the program")
            .value_name("USER"),
        Arg::new(opt_flags::GROUP)
            .short('g')
            .long(opt_flags::GROUP)
            .help("Group (ID or name) to switch to")
            .value_name("GROUP"),
        Arg::new(opt_flags::GROUPS)
            .short('G')
            .long(opt_flags::GROUPS)
            .help("Comma-separated list of groups to switch to")
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
    // 从命令行参数中解析用户和组信息
    let user_spec_str = args_option.get_one::<String>(opt_flags::USERSPEC);
    let user_str = args_option
        .get_one::<String>(opt_flags::USER)
        .map(|s| s.as_str())
        .unwrap_or_default();
    let group_str = args_option
        .get_one::<String>(opt_flags::GROUP)
        .map(|s| s.as_str())
        .unwrap_or_default();
    let groups_str = args_option
        .get_one::<String>(opt_flags::GROUPS)
        .map(|s| s.as_str())
        .unwrap_or_default();
    let skip_chdir = args_option.contains_id(opt_flags::SKIP_CHDIR);

    // 解析用户规范字符串
    let user_spec = match chroot_parse_user_spec(user_spec_str) {
        Ok(value) => value,
        Err(value) => return value, // 如果解析失败，直接返回错误
    };

    // 根据用户规范决定使用哪个用户和组
    let (user, group) = if user_spec.is_empty() {
        (user_str, group_str)
    } else {
        (user_spec[0], user_spec[1])
    };

    // 进入新的根目录（如果配置中允许）
    chroot_enter(root_path, skip_chdir)?;

    // 设置补充组和主组
    chroot_set_groups_from_str(groups_str)?;
    chroot_set_main_group(group)?;
    // 设置有效用户
    chroot_set_user(user)?;
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
 *                                        如果解析失败，返回一个包含错误信息的错误结果。
 */
fn chroot_parse_user_spec(user_spec_str: Option<&String>) -> Result<Vec<&str>, CTResult<()>> {
    // 根据输入的用户规格字符串进行处理
    let user_spec = match user_spec_str {
        Some(u) => {
            // 将用户规格字符串按':'分割成两部分
            let s: Vec<&str> = u.split(':').collect();
            // 检查分割后的向量是否包含两部分且两部分都不为空
            if s.len() != 2 || s.iter().any(|&spec| spec.is_empty()) {
                // 如果格式不正确，返回错误
                return Err(Err(ChrootError::InvalidUserspec(u.to_string()).into()));
            };
            s
        }
        None => Vec::new(), // 如果输入为None，返回空向量
    };
    Ok(user_spec)
}

fn chroot_enter(root_path: &Path, is_skip_chdir: bool) -> CTResult<()> {
    let err = chroot_option(root_path);

    if err == 0 {
        if !is_skip_chdir {
            std::env::set_current_dir(root_path).unwrap();
        }
        Ok(())
    } else {
        Err(
            ChrootError::CannotEnter(format!("{}", root_path.display()), Error::last_os_error())
                .into(),
        )
    }
}

fn chroot_option(root_path: &Path) -> c_int {
    let err = unsafe {
        chroot(
            CString::new(root_path.as_os_str().as_bytes().to_vec())
                .unwrap()
                .as_bytes_with_nul()
                .as_ptr() as *const libc::c_char,
        )
    };
    err
}

fn chroot_set_main_group(chroot_group: &str) -> CTResult<()> {
    if !chroot_group.is_empty() {
        let group_id = match ct_entries::grp2gid(chroot_group) {
            Ok(g) => g,
            _ => return Err(ChrootError::NoSuchGroup(chroot_group.to_string()).into()),
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

#[cfg(any(target_vendor = "apple", target_os = "freebsd", target_os = "openbsd"))]
fn set_groups(groups: &[libc::gid_t]) -> libc::c_int {
    unsafe { setgroups(groups.len() as libc::c_int, groups.as_ptr()) }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn chroot_set_groups(groups: &[libc::gid_t]) -> libc::c_int {
    unsafe { setgroups(groups.len() as libc::size_t, groups.as_ptr()) }
}

fn chroot_set_groups_from_str(groups: &str) -> CTResult<()> {
    if !groups.is_empty() {
        let mut groups_vec = vec![];
        for group in groups.split(',') {
            let gid = match ct_entries::grp2gid(group) {
                Ok(g) => g,
                Err(_) => return Err(ChrootError::NoSuchGroup(group.to_string()).into()),
            };
            groups_vec.push(gid);
        }
        let err = chroot_set_groups(&groups_vec);
        if err != 0 {
            return Err(ChrootError::SetGroupsFailed(Error::last_os_error()).into());
        }
    }
    Ok(())
}

fn chroot_set_user(username: &str) -> CTResult<()> {
    if !username.is_empty() {
        let user_id = ct_entries::usr2uid(username).unwrap();
        let err = unsafe { setuid(user_id as libc::uid_t) };
        if err != 0 {
            return Err(
                ChrootError::SetUserFailed(username.to_string(), Error::last_os_error()).into(),
            );
        }
    }
    Ok(())
}

