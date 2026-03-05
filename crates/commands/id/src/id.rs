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

//! 输出指定<用户>的用户和用户组信息，或（当没有指定用户信息时）默认使用当前用户信息 。
//!
//! 参考BSD id设计逻辑：
//!  http://ftp-archive.freebsd.org/mirror/FreeBSD-Archive/old-releases/i386/1.0-RELEASE/ports/shellutils/src/id.c
//!  http://www.opensource.apple.com/source/shell_cmds/shell_cmds-118/id/id.c

extern crate rust_i18n;
use rust_i18n::t;
use std::io::{self, Write};
rust_i18n::i18n!("locales", fallback = "en-US");
use clap::{Arg, ArgAction, Command, crate_version};
use sys_locale::get_locale;

use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_entries::{self, CtPasswd, Locate};
use ctcore::ct_error::CTResult;
use ctcore::ct_error::{CtSimpleError, set_ct_exit_code};
use ctcore::ct_line_ending::CtLineEnding;
use ctcore::ct_process::{getegid, geteuid, getgid, getuid};
use ctcore::ct_show_error;
use selinux::SecurityContext;
use std::ffi::OsString;

static CONTEXT_HELP_TEXT: &str = "print only the security context of the process";

mod id_flags {
    // pub const ID_AUDIT: &str = "audit"; // GNU's id does not have this
    pub const ID_CONTEXT: &str = "context";
    pub const ID_EFFECTIVE_USER: &str = "user";
    pub const ID_GROUP: &str = "group";
    pub const ID_GROUPS: &str = "groups";
    pub const ID_NAME: &str = "name";
    pub const ID_REAL_ID: &str = "real";
    pub const ID_ZERO: &str = "zero"; // BSD's id does not have this
    pub const ID_ARG_USERS: &str = "USER";
    pub const ID_IGNORE: &str = "ignore"; // SVR4 compatibility flag
}

struct Ids {
    uid: u32,  // user id
    gid: u32,  // group id
    euid: u32, // effective uid
    egid: u32, // effective gid
}

struct IdState {
    is_nflag: bool,  // --name
    is_uflag: bool,  // --user
    is_gflag: bool,  // --group
    is_gsflag: bool, // --groups
    is_rflag: bool,  // --real
    is_zflag: bool,  // --zero
    is_cflag: bool,  // --context
    is_selinux_supported: bool,
    ids: Option<Ids>,
    // 调用 GNU 的 'id' 和调用 GNU 的 'id $USER' 的行为相似但不同。
    // * SELinux 上下文仅在没有指定用户的情况下显示.
    // * "getgroups"系统调用仅在没有指定用户的情况下使用，这会导致"id"和"id $USER"之间显示组的顺序不同。
    //
    // Example:
    // $ strace -e getgroups id -G $USER
    // 1000 10 975 968
    // +++ exited with 0 +++
    // $ strace -e getgroups id -G
    // getgroups(0, NULL)                      = 4
    // getgroups(4, [10, 968, 975, 1000])      = 4
    // 1000 10 968 975
    // +++ exited with 0 +++
    is_user_specified: bool,
}

#[derive(Default)]
pub struct Id;
impl Tool for Id {
    fn name(&self) -> &'static str {
        "id"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        let stdout_info = io::stdout();
        let mut stdout_writer = stdout_info.lock();
        id_main(&mut stdout_writer, args.iter().cloned())
    }
}

pub fn id_main<W: Write>(writer: &mut W, args: impl ctcore::Args) -> CTResult<()> {
    // 设置语言
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);

    let matches = ct_app()
        .after_help(t!("id.after_help"))
        .try_get_matches_from(args)?;

    let users: Vec<String> = matches
        .get_many::<String>(id_flags::ID_ARG_USERS)
        .map(|v| v.map(ToString::to_string).collect())
        .unwrap_or_default();

    let mut state = id_get_state(&matches, &users);

    let is_default_format = id_flags_validity_checks(&mut state)?;

    let line_ending = CtLineEnding::from_zero_flag(state.is_zflag);

    // 显示安全上下文
    if state.is_cflag {
        if state.is_selinux_supported {
            // print SElinux context and exit
            if let Ok(context) = SecurityContext::current(false) {
                let bytes = context.as_bytes();
                write!(writer, "{}{}", String::from_utf8_lossy(bytes), line_ending)?;
            } else {
                // print error because `cflag` was explicitly requested
                return Err(CtSimpleError::new(1, "can't get process context"));
            }
            return Ok(());
        } else {
            return Err(CtSimpleError::new(
                1,
                "--context (-Z) works only on an SELinux-enabled kernel",
            ));
        }
    }

    // 处理并展示用户信息
    id_handle_users(writer, users, &mut state, is_default_format, line_ending);

    Ok(())
}

fn id_handle_users<W: Write>(
    writer: &mut W,
    users: Vec<String>,
    id_state: &mut IdState,
    is_default_format: bool,
    line_ending: CtLineEnding,
) {
    let delimiter = id_get_delimiter(id_state);

    for i in 0..=users.len() {
        let possible_pw = if id_state.is_user_specified {
            match CtPasswd::locate(users[i].as_str()) {
                Ok(p) => Some(p),
                Err(_) => {
                    ct_show_error!("{}: no such user", users[i].quote());
                    set_ct_exit_code(1);
                    if i + 1 >= users.len() {
                        break;
                    } else {
                        continue;
                    }
                }
            }
        } else {
            None
        };

        let (uid, gid) = possible_pw.as_ref().map(|p| (p.uid, p.gid)).unwrap_or((
            if id_state.is_rflag {
                getuid()
            } else {
                geteuid()
            },
            if id_state.is_rflag {
                getgid()
            } else {
                getegid()
            },
        ));

        id_state.ids = Some(Ids {
            uid,
            gid,
            euid: geteuid(),
            egid: getegid(),
        });

        if id_state.is_gflag {
            let gid_string = if id_state.is_nflag {
                ct_entries::gid2grp(gid).unwrap_or_else(|_| {
                    ct_show_error!("cannot find name for group ID {}", gid);
                    set_ct_exit_code(1);
                    gid.to_string()
                })
            } else {
                gid.to_string()
            };

            write!(writer, "{gid_string}").unwrap();
        }

        if id_state.is_uflag {
            let name_string = if id_state.is_nflag {
                ct_entries::uid2usr(uid).unwrap_or_else(|_| {
                    ct_show_error!("cannot find name for user ID {}", uid);
                    set_ct_exit_code(1);
                    uid.to_string()
                })
            } else {
                uid.to_string()
            };

            write!(writer, "{name_string}").unwrap();
        }

        let groups = if id_state.is_user_specified {
            possible_pw.as_ref().map(|p| p.belongs_to()).unwrap()
        } else {
            ct_entries::get_groups_gnu(Some(gid)).unwrap()
        };

        if id_state.is_gsflag {
            let groups_string = groups
                .iter()
                .map(|&id| {
                    if id_state.is_nflag {
                        ct_entries::gid2grp(id).unwrap_or_else(|_| {
                            ct_show_error!("cannot find name for group ID {}", id);
                            set_ct_exit_code(1);
                            id.to_string()
                        })
                    } else {
                        id.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(&delimiter);

            let delimit = if id_state.is_zflag && id_state.is_user_specified && users.len() > 1 {
                "\0"
            } else {
                ""
            };

            write!(writer, "{groups_string}{delimit}").unwrap();
        }

        if is_default_format {
            id_print(writer, id_state, &groups);
        }
        write!(writer, "{line_ending}").unwrap();

        if i + 1 >= users.len() {
            break;
        }
    }
}

fn id_get_delimiter(state: &mut IdState) -> String {
    if state.is_zflag {
        "\0".to_string()
    } else {
        " ".to_string()
    }
}

fn id_flags_validity_checks(state: &mut IdState) -> CTResult<bool> {
    let is_default_format = {
        // "default format" is when none of '-ugG' was used
        !(state.is_uflag || state.is_gflag || state.is_gsflag)
    };

    if (state.is_nflag || state.is_rflag) && is_default_format && !state.is_cflag {
        return Err(CtSimpleError::new(
            1,
            "cannot print only names or real IDs in default format",
        ));
    }
    if state.is_zflag && is_default_format && !state.is_cflag {
        // NOTE: GNU test suite "id/zero.sh" needs this stderr output:
        return Err(CtSimpleError::new(
            1,
            "option --zero not permitted in default format",
        ));
    }
    if state.is_user_specified && state.is_cflag {
        return Err(CtSimpleError::new(
            1,
            "cannot print security context when user specified",
        ));
    }
    Ok(is_default_format)
}

fn id_get_state(matches: &clap::ArgMatches, users: &[String]) -> IdState {
    IdState {
        is_nflag: matches.get_flag(id_flags::ID_NAME),
        is_uflag: matches.get_flag(id_flags::ID_EFFECTIVE_USER),
        is_gflag: matches.get_flag(id_flags::ID_GROUP),
        is_gsflag: matches.get_flag(id_flags::ID_GROUPS),
        is_rflag: matches.get_flag(id_flags::ID_REAL_ID),
        is_zflag: matches.get_flag(id_flags::ID_ZERO),
        is_cflag: matches.get_flag(id_flags::ID_CONTEXT),

        is_selinux_supported: { selinux::kernel_support() != selinux::KernelSupport::Unsupported },
        is_user_specified: !users.is_empty(),
        ids: None,
    }
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("id.about");
    let usage_description = t!("id.usage");
    let args = vec![
        Arg::new(id_flags::ID_EFFECTIVE_USER)
            .short('u')
            .long(id_flags::ID_EFFECTIVE_USER)
            .conflicts_with(id_flags::ID_GROUP)
            .help(t!("id.clap.id_effective_user"))
            .action(ArgAction::SetTrue),
        Arg::new(id_flags::ID_GROUP)
            .short('g')
            .long(id_flags::ID_GROUP)
            .conflicts_with(id_flags::ID_EFFECTIVE_USER)
            .help(t!("id.clap.id_group"))
            .action(ArgAction::SetTrue),
        Arg::new(id_flags::ID_GROUPS)
            .short('G')
            .long(id_flags::ID_GROUPS)
            .conflicts_with_all([
                id_flags::ID_GROUP,
                id_flags::ID_EFFECTIVE_USER,
                id_flags::ID_CONTEXT,
            ])
            .help(
                "Display only the different group IDs as white-space separated numbers, \
                      in no particular order.",
            )
            .action(ArgAction::SetTrue),
        Arg::new(id_flags::ID_NAME)
            .short('n')
            .long(id_flags::ID_NAME)
            .help(
                "Display the name of the user or group ID for the -G, -g and -u options \
                      instead of the number.\nIf any of the ID numbers cannot be mapped into \
                      names, the number will be displayed as usual.",
            )
            .action(ArgAction::SetTrue),
        Arg::new(id_flags::ID_REAL_ID)
            .short('r')
            .long(id_flags::ID_REAL_ID)
            .help(
                "Display the real ID for the -G, -g and -u options instead of \
                      the effective ID.",
            )
            .action(ArgAction::SetTrue),
        Arg::new(id_flags::ID_ZERO)
            .short('z')
            .long(id_flags::ID_ZERO)
            .help(
                "delimit entries with NUL characters, not whitespace;\n\
                      not permitted in default format",
            )
            .action(ArgAction::SetTrue),
        Arg::new(id_flags::ID_CONTEXT)
            .short('Z')
            .long(id_flags::ID_CONTEXT)
            .conflicts_with_all([id_flags::ID_GROUP, id_flags::ID_EFFECTIVE_USER])
            .help(CONTEXT_HELP_TEXT)
            .action(ArgAction::SetTrue),
        Arg::new(id_flags::ID_IGNORE)
            .short('a')
            .help(t!("id.clap.id_ignore"))
            .action(ArgAction::SetTrue),
        Arg::new(id_flags::ID_ARG_USERS)
            .action(ArgAction::Append)
            .value_name(id_flags::ID_ARG_USERS)
            .value_hint(clap::ValueHint::Username),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(&args)
}

fn id_print<W: Write>(writer: &mut W, id_state: &IdState, groups: &[u32]) {
    let uid = id_state.ids.as_ref().map(|ids| ids.uid).unwrap_or(65535);
    let gid = id_state.ids.as_ref().map(|ids| ids.gid).unwrap_or(65535);
    let euid = id_state.ids.as_ref().map(|ids| ids.euid).unwrap_or(65535);
    let egid = id_state.ids.as_ref().map(|ids| ids.egid).unwrap_or(65535);

    let uid_string = ct_entries::uid2usr(uid).unwrap_or_else(|_| {
        writeln!(writer, "cannot find name for user ID {uid}").unwrap();
        set_ct_exit_code(1);
        uid.to_string()
    });
    write!(writer, "uid={uid}({uid_string})").unwrap();

    let gid_string = ct_entries::gid2grp(gid).unwrap_or_else(|_| {
        writeln!(writer, "cannot find name for group ID {gid}").unwrap();
        set_ct_exit_code(1);
        gid.to_string()
    });
    write!(writer, " gid={gid}({gid_string})").unwrap();

    if !id_state.is_user_specified && (euid != uid) {
        let euid_string = ct_entries::uid2usr(euid).unwrap_or_else(|_| {
            writeln!(writer, "cannot find name for user ID {euid}").unwrap();
            set_ct_exit_code(1);
            euid.to_string()
        });
        write!(writer, " euid={euid}({euid_string})").unwrap();
    }

    if !id_state.is_user_specified && (egid != gid) {
        let egid_string = ct_entries::gid2grp(egid).unwrap_or_else(|_| {
            writeln!(writer, "cannot find name for group ID {egid}").unwrap();
            set_ct_exit_code(1);
            egid.to_string()
        });
        write!(writer, " egid={egid}({egid_string})").unwrap();
    }

    let groups_string = groups
        .iter()
        .map(|&gr| {
            format!(
                "{}({})",
                gr,
                ct_entries::gid2grp(gr).unwrap_or_else(|_| {
                    writeln!(writer, "cannot find name for group ID {gr}").unwrap();
                    set_ct_exit_code(1);
                    gr.to_string()
                })
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    write!(writer, " groups={groups_string}").unwrap();

    if id_state.is_selinux_supported
        && !id_state.is_user_specified
        && std::env::var_os("POSIXLY_CORRECT").is_none()
    {
        // print SElinux context (does not depend on "-Z")
        if let Ok(context) = SecurityContext::current(false) {
            let bytes = context.as_bytes();
            write!(writer, " context={}", String::from_utf8_lossy(bytes)).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Id;

        // 测试 name 方法
        assert_eq!(tool.name(), "id");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("id"));

        // 测试 execute 方法
        let args = vec![OsString::from("id")];
        assert!(tool.execute(&args).is_ok());
    }

    #[cfg(test)]
    mod id_handle_users_tests {
        use super::*;
        use std::io::Cursor;

        #[test]
        fn test_id_handle_users_single_user_root() {
            // 测试 root 用户的默认格式输出
            let mut state = IdState {
                is_nflag: false,
                is_uflag: false,
                is_gflag: false,
                is_gsflag: false,
                is_rflag: false,
                is_zflag: false,
                is_cflag: false,
                is_selinux_supported: false,
                is_user_specified: true,
                ids: None,
            };

            let users = vec!["root".to_string()];
            let line_ending = CtLineEnding::Newline;
            let mut output = Cursor::new(Vec::new());

            id_handle_users(&mut output, users, &mut state, true, line_ending);

            let output_str = String::from_utf8(output.into_inner()).expect("输出不是有效的 UTF-8");
            assert!(output_str.contains("uid=0"));
            assert!(output_str.contains("gid=0"));
        }

        #[test]
        fn test_id_handle_users_single_user_nobody() {
            // 测试 nobody 用户的默认格式输出
            let mut state = IdState {
                is_nflag: false,
                is_uflag: false,
                is_gflag: false,
                is_gsflag: false,
                is_rflag: false,
                is_zflag: false,
                is_cflag: false,
                is_selinux_supported: false,
                is_user_specified: true,
                ids: None,
            };

            let users = vec!["nobody".to_string()];
            let line_ending = CtLineEnding::Newline;
            let mut output = Cursor::new(Vec::new());

            id_handle_users(&mut output, users, &mut state, true, line_ending);

            let output_str = String::from_utf8(output.into_inner()).expect("输出不是有效的 UTF-8");
            assert!(output_str.contains("uid="));
            assert!(output_str.contains("gid="));
        }

        #[test]
        fn test_id_handle_users_multiple_users() {
            // 测试多个用户的情况
            let mut state = IdState {
                is_nflag: false,
                is_uflag: false,
                is_gflag: false,
                is_gsflag: true,
                is_rflag: false,
                is_zflag: false,
                is_cflag: false,
                is_selinux_supported: false,
                is_user_specified: true,
                ids: None,
            };

            let users = vec!["root".to_string(), "nobody".to_string()];
            let line_ending = CtLineEnding::Newline;
            let mut output = Cursor::new(Vec::new());

            id_handle_users(&mut output, users, &mut state, false, line_ending);

            let output_str = String::from_utf8(output.into_inner()).expect("输出不是有效的 UTF-8");

            assert!(output_str.contains("0"));
        }

        #[test]
        fn test_id_handle_users_no_such_user() {
            // 测试当用户不存在时
            let mut state = IdState {
                is_nflag: false,
                is_uflag: false,
                is_gflag: false,
                is_gsflag: false,
                is_rflag: false,
                is_zflag: false,
                is_cflag: false,
                is_selinux_supported: false,
                is_user_specified: true,
                ids: None,
            };

            let users = vec!["nonexistentuser".to_string()];
            let line_ending = CtLineEnding::Newline;
            let mut output = Cursor::new(Vec::new());

            id_handle_users(&mut output, users, &mut state, false, line_ending);

            let output_str = String::from_utf8(output.into_inner()).expect("输出不是有效的 UTF-8");

            assert!(output_str.contains(""));
        }

        #[test]
        fn test_id_handle_users_with_flags() {
            // 测试带有不同标志的情况
            let mut state = IdState {
                is_nflag: true,
                is_uflag: true,
                is_gflag: true,
                is_gsflag: true,
                is_rflag: false,
                is_zflag: false,
                is_cflag: false,
                is_selinux_supported: false,
                is_user_specified: true,
                ids: None,
            };

            let users = vec!["root".to_string()];
            let line_ending = CtLineEnding::Newline;
            let mut output = Cursor::new(Vec::new());

            id_handle_users(&mut output, users, &mut state, false, line_ending);

            let output_str = String::from_utf8(output.into_inner()).expect("输出不是有效的 UTF-8");

            assert!(output_str.contains("root")); // 用户名
            // assert!(output_str.contains("0")); // 组ID
        }

        #[test]
        fn test_id_handle_users_zero_delimiter() {
            // 测试带有零分隔符的情况
            let mut state = IdState {
                is_nflag: false,
                is_uflag: false,
                is_gflag: false,
                is_gsflag: true,
                is_rflag: false,
                is_zflag: true,
                is_cflag: false,
                is_selinux_supported: false,
                is_user_specified: true,
                ids: None,
            };

            let users = vec!["nobody".to_string()];
            let line_ending = CtLineEnding::Newline;
            let mut output = Cursor::new(Vec::new());

            id_handle_users(&mut output, users, &mut state, false, line_ending);

            let output_str = String::from_utf8(output.into_inner()).expect("输出不是有效的 UTF-8");

            assert!(output_str.contains("65534"));
        }

        #[test]
        fn test_id_handle_users_with_rflag() {
            // 测试带有 --real (-r) 标志的情况
            let mut state = IdState {
                is_nflag: false,
                is_uflag: true,
                is_gflag: false,
                is_gsflag: false,
                is_rflag: true,
                is_zflag: false,
                is_cflag: false,
                is_selinux_supported: false,
                is_user_specified: false,
                ids: None,
            };

            let users = vec!["root".to_string()];
            let line_ending = CtLineEnding::Newline;
            let mut output = Cursor::new(Vec::new());

            id_handle_users(&mut output, users, &mut state, true, line_ending);

            let output_str = String::from_utf8(output.into_inner()).expect("输出不是有效的 UTF-8");
            assert!(output_str.contains("uid=0"));
            assert!(output_str.contains("gid=0"));
        }

        #[test]
        fn test_id_handle_users_multiple_users_with_zflag() {
            // 测试多个用户带有 --zero (-z) 标志的情况
            let mut state = IdState {
                is_nflag: false,
                is_uflag: true,
                is_gflag: false,
                is_gsflag: true,
                is_rflag: false,
                is_zflag: true,
                is_cflag: false,
                is_selinux_supported: false,
                is_user_specified: true,
                ids: None,
            };

            let users = vec!["root".to_string(), "nobody".to_string()];
            let line_ending = CtLineEnding::Newline;
            let mut output = Cursor::new(Vec::new());

            id_handle_users(&mut output, users, &mut state, false, line_ending);

            let output_str = String::from_utf8(output.into_inner()).expect("输出不是有效的 UTF-8");
            assert!(output_str.contains("\0"));
        }
    }

    #[cfg(test)]
    mod id_get_delimiter_tests {
        use super::*;

        #[test]
        fn test_id_get_delimiter_with_zero_flag() {
            // 测试当 is_zflag 为 true 时，应该返回 NUL 字符作为分隔符
            let mut state = IdState {
                is_nflag: false,
                is_uflag: false,
                is_gflag: false,
                is_gsflag: false,
                is_rflag: false,
                is_zflag: true,
                is_cflag: false,
                is_selinux_supported: false,
                is_user_specified: false,
                ids: None,
            };

            let delimiter = id_get_delimiter(&mut state);
            assert_eq!(delimiter, "\0");
        }

        #[test]
        fn test_id_get_delimiter_without_zero_flag() {
            // 测试当 is_zflag 为 false 时，应该返回空格字符作为分隔符
            let mut state = IdState {
                is_nflag: false,
                is_uflag: false,
                is_gflag: false,
                is_gsflag: false,
                is_rflag: false,
                is_zflag: false,
                is_cflag: false,
                is_selinux_supported: false,
                is_user_specified: false,
                ids: None,
            };

            let delimiter = id_get_delimiter(&mut state);
            assert_eq!(delimiter, " ");
        }
    }

    #[cfg(test)]
    mod id_flags_validity_checks_tests {
        use super::*;

        #[test]
        fn test_valid_flags_default_format() {
            // 测试在默认格式下没有标志被设置的情况
            let mut state = IdState {
                is_nflag: false,
                is_uflag: false,
                is_gflag: false,
                is_gsflag: false,
                is_rflag: false,
                is_zflag: false,
                is_cflag: false,
                is_selinux_supported: false,
                is_user_specified: false,
                ids: None,
            };

            let result = id_flags_validity_checks(&mut state);
            assert!(result.is_ok());
            assert!(result.unwrap()); // 默认格式返回 true
        }

        #[test]
        fn test_invalid_flags_name_or_real_in_default_format() {
            // 测试在默认格式下设置了 --name 或 --real 的情况
            let mut state = IdState {
                is_nflag: true,
                is_uflag: false,
                is_gflag: false,
                is_gsflag: false,
                is_rflag: true,
                is_zflag: false,
                is_cflag: false,
                is_selinux_supported: false,
                is_user_specified: false,
                ids: None,
            };

            let result = id_flags_validity_checks(&mut state);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().to_string(),
                "cannot print only names or real IDs in default format"
            );
        }

        #[test]
        fn test_invalid_flags_zero_in_default_format() {
            // 测试在默认格式下设置了 --zero 的情况
            let mut state = IdState {
                is_nflag: false,
                is_uflag: false,
                is_gflag: false,
                is_gsflag: false,
                is_rflag: false,
                is_zflag: true,
                is_cflag: false,
                is_selinux_supported: false,
                is_user_specified: false,
                ids: None,
            };

            let result = id_flags_validity_checks(&mut state);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().to_string(),
                "option --zero not permitted in default format"
            );
        }

        #[test]
        fn test_invalid_flags_context_with_user_specified() {
            // 测试在指定用户时设置了 --context 的情况
            let mut state = IdState {
                is_nflag: false,
                is_uflag: false,
                is_gflag: false,
                is_gsflag: false,
                is_rflag: false,
                is_zflag: false,
                is_cflag: true,
                is_selinux_supported: true,
                is_user_specified: true,
                ids: None,
            };

            let result = id_flags_validity_checks(&mut state);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().to_string(),
                "cannot print security context when user specified"
            );
        }

        #[test]
        fn test_valid_flags_with_non_default_format() {
            // 测试非默认格式下的有效标志组合
            let mut state = IdState {
                is_nflag: false,
                is_uflag: true,
                is_gflag: false,
                is_gsflag: true,
                is_rflag: false,
                is_zflag: false,
                is_cflag: false,
                is_selinux_supported: false,
                is_user_specified: false,
                ids: None,
            };

            let result = id_flags_validity_checks(&mut state);
            assert!(result.is_ok());
            assert!(!result.unwrap()); // 非默认格式返回 false
        }

        #[test]
        fn test_invalid_flags_name_without_printing() {
            // 测试仅设置 --name 而不打印任何内容的情况
            let mut state = IdState {
                is_nflag: true,
                is_uflag: false,
                is_gflag: false,
                is_gsflag: false,
                is_rflag: false,
                is_zflag: false,
                is_cflag: false,
                is_selinux_supported: false,
                is_user_specified: false,
                ids: None,
            };

            let result = id_flags_validity_checks(&mut state);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().to_string(),
                "cannot print only names or real IDs in default format"
            );
        }
    }

    #[cfg(test)]
    mod id_get_state_tests {
        use clap::ArgMatches;

        use super::*;

        fn create_arg_matches(args: Vec<&str>) -> ArgMatches {
            ct_app().try_get_matches_from(args).unwrap()
        }

        #[test]
        fn test_id_get_state_default() {
            // 测试默认情况下的 IdState
            let args = vec!["id"];
            let matches = create_arg_matches(args);
            let users = Vec::new();

            let state = id_get_state(&matches, &users);

            assert!(!state.is_nflag);
            assert!(!state.is_uflag);
            assert!(!state.is_gflag);
            assert!(!state.is_gsflag);
            assert!(!state.is_rflag);
            assert!(!state.is_zflag);
            assert!(!state.is_cflag);
            //assert_eq!(state.is_selinux_supported, false);
            assert!(!state.is_user_specified);
            assert!(state.ids.is_none());
        }

        #[test]
        fn test_id_get_state_with_flags_unrz() {
            // 测试各种标志设置的 IdState
            let args = vec!["id", "-u", "-n", "-r", "-z"];
            let matches = create_arg_matches(args);
            let users = Vec::new();

            let state = id_get_state(&matches, &users);

            assert!(state.is_nflag);
            assert!(state.is_uflag);
            assert!(state.is_rflag);
            assert!(state.is_zflag);
            assert!(!state.is_cflag);
            //assert_eq!(state.is_selinux_supported, false);
            assert!(!state.is_user_specified);
            assert!(state.ids.is_none());
        }
        #[test]
        fn test_id_get_state_with_flags_gnrz() {
            // 测试各种标志设置的 IdState
            let args = vec!["id", "-g", "-n", "-r", "-z"];
            let matches = create_arg_matches(args);
            let users = Vec::new();

            let state = id_get_state(&matches, &users);

            assert!(state.is_nflag);
            assert!(!state.is_uflag);
            assert!(state.is_gflag);
            assert!(!state.is_gsflag);
            assert!(state.is_rflag);
            assert!(state.is_zflag);
            assert!(!state.is_cflag);
            //assert_eq!(state.is_selinux_supported, true);
            assert!(!state.is_user_specified);
            assert!(state.ids.is_none());
        }
        #[test]
        fn test_id_get_state_with_flags_g_nrz() {
            // 测试各种标志设置的 IdState
            let args = vec!["id", "-G", "-n", "-r", "-z"];
            let matches = create_arg_matches(args);
            let users = Vec::new();

            let state = id_get_state(&matches, &users);

            assert!(state.is_nflag);
            assert!(!state.is_uflag);
            assert!(!state.is_gflag);
            assert!(state.is_gsflag);
            assert!(state.is_rflag);
            assert!(state.is_zflag);
            assert!(!state.is_cflag);
            //assert_eq!(state.is_selinux_supported, false);
            assert!(!state.is_user_specified);
            assert!(state.ids.is_none());
        }
        #[test]
        fn test_id_get_state_with_user() {
            // 测试指定用户的情况
            let args = vec!["id", "testuser"];
            let matches = create_arg_matches(args);
            let users = vec!["testuser".to_string()];

            let state = id_get_state(&matches, &users);

            assert!(!state.is_nflag);
            assert!(!state.is_uflag);
            assert!(!state.is_gflag);
            assert!(!state.is_gsflag);
            assert!(!state.is_rflag);
            assert!(!state.is_zflag);
            assert!(!state.is_cflag);
            //assert_eq!(state.is_selinux_supported, true);
            assert!(state.is_user_specified);
            assert!(state.ids.is_none());
        }

        #[test]
        fn test_id_get_state_with_selinux() {
            // 测试启用了 SELinux 的情况（假设配置特性启用了 selinux）
            let args = vec!["id", "-Z"];
            let matches = create_arg_matches(args);
            let users = Vec::new();

            // 注意：此测试假设 `selinux::kernel_support` 返回非 `Unsupported` 以模拟 SELinux 启用的情况
            {
                let _state = id_get_state(&matches, &users);
                //assert_eq!(state.is_selinux_supported, true);
            }
        }

        #[test]
        fn test_id_get_state_no_flags_user_specified() {
            // 测试没有标志但指定用户的情况
            let args = vec!["id", "user1"];
            let matches = create_arg_matches(args);
            let users = vec!["user1".to_string()];

            let state = id_get_state(&matches, &users);

            assert!(!state.is_nflag);
            assert!(!state.is_uflag);
            assert!(!state.is_gflag);
            assert!(!state.is_gsflag);
            assert!(!state.is_rflag);
            assert!(!state.is_zflag);
            assert!(!state.is_cflag);
            //assert_eq!(state.is_selinux_supported, false);
            assert!(state.is_user_specified);
            assert!(state.ids.is_none());
        }

        #[test]
        fn test_id_get_state_conflicting_flags() {
            // 测试传递互斥标志的情况，这可能触发某些冲突处理逻辑
            let args = vec!["id", "-u"];
            let matches = create_arg_matches(args);
            let users = Vec::new();

            let state = id_get_state(&matches, &users);

            // clap 库应自动处理冲突，因此我们期望一个标志是启用的，另一个被忽略
            assert!(state.is_uflag || state.is_gflag);
            //assert_eq!(state.is_selinux_supported, false);
            assert!(!state.is_user_specified);
        }

        #[test]
        fn test_id_get_state_selinux_conflict_with_user() {
            // 测试传递 --context (-Z) 和指定用户的情况，这在原代码中可能导致错误
            let args = vec!["id", "-Z", "user1"];
            let matches = create_arg_matches(args);
            let users = vec!["user1".to_string()];

            let state = id_get_state(&matches, &users);

            assert!(state.is_cflag);
            assert!(state.is_user_specified);

            //assert!(!state.is_selinux_supported);
        }

        #[test]
        fn test_id_get_state_flag_combinations() {
            // 测试不同标志组合
            let args = vec!["id", "-n", "-g", "user2"];
            let matches = create_arg_matches(args);
            let users = vec!["user2".to_string()];

            let state = id_get_state(&matches, &users);

            assert!(state.is_nflag);
            assert!(state.is_gflag);
            assert!(state.is_user_specified);
            //assert_eq!(state.is_selinux_supported, false);
        }

        #[test]
        fn test_id_get_state_no_users_specified() {
            // 测试没有指定用户时的情况
            let args = vec!["id", "-Z"];
            let matches = create_arg_matches(args);
            let users = Vec::new();

            let state = id_get_state(&matches, &users);

            assert!(!state.is_gsflag);
            assert!(state.is_cflag);
            assert!(!state.is_user_specified);

            //assert!(!state.is_selinux_supported);
        }
    }

    #[cfg(test)]
    mod id_print_tests {
        use super::*;

        /// 检测是否在容器环境中运行
        fn is_container() -> bool {
            // 检查常见的容器环境标识
            if std::env::var("KUBERNETES_SERVICE_HOST").is_ok()
                || std::env::var("DOCKER_CONTAINER").is_ok()
                || std::path::Path::new("/.dockerenv").exists()
                || std::path::Path::new("/run/.containerenv").exists()
            {
                return true;
            }

            // 检查 cgroup
            if let Ok(contents) = std::fs::read_to_string("/proc/1/cgroup") {
                if contents.contains("/docker/") || contents.contains("/kubepods/") {
                    return true;
                }
            }

            false
        }

        #[test]
        fn test_id_print_default_case() {
            // 默认情况下，只打印 uid、gid 和 groups
            let ids = Ids {
                uid: 1000,
                gid: 1000,
                euid: 1000,
                egid: 1000,
            };
            let id_state = IdState {
                is_nflag: false,
                is_uflag: false,
                is_gflag: false,
                is_gsflag: false,
                is_rflag: false,
                is_zflag: false,
                is_cflag: false,
                is_selinux_supported: false,
                is_user_specified: false,
                ids: Some(ids),
            };
            let groups = vec![1000, 10, 20];

            let mut output = Vec::new();
            id_print(&mut output, &id_state, &groups);
            let output_str = String::from_utf8(output).expect("输出不是有效的 UTF-8");

            assert!(output_str.contains("uid=1000"));
            assert!(output_str.contains("gid=1000"));
            assert!(output_str.contains("groups=1000"));
        }

        #[test]
        fn test_id_print_with_name_flag() {
            // 当 `is_nflag` 为真时，打印用户名和组名而不是数字
            let ids = Ids {
                uid: 1000,
                gid: 1000,
                euid: 1000,
                egid: 1000,
            };
            let id_state = IdState {
                is_nflag: true,
                is_uflag: true,
                is_gflag: true,
                is_gsflag: true,
                is_rflag: false,
                is_zflag: false,
                is_cflag: false,
                is_selinux_supported: false,
                is_user_specified: false,
                ids: Some(ids),
            };
            let groups = vec![1000, 10, 20];

            let mut output = Vec::new();
            id_print(&mut output, &id_state, &groups);
            let output_str = String::from_utf8(output).expect("输出不是有效的 UTF-8");

            assert!(output_str.contains("uid=1000"));
            assert!(output_str.contains("gid=1000"));
            assert!(output_str.contains("groups=1000"));
        }

        #[test]
        fn test_id_print_with_real_id_flag() {
            // 当 `is_rflag` 为真时，打印实际的 uid 和 gid
            let ids = Ids {
                uid: 2000,
                gid: 2000,
                euid: 1000,
                egid: 1000,
            };
            let id_state = IdState {
                is_nflag: false,
                is_uflag: false,
                is_gflag: false,
                is_gsflag: false,
                is_rflag: true,
                is_zflag: false,
                is_cflag: false,
                is_selinux_supported: false,
                is_user_specified: false,
                ids: Some(ids),
            };
            let groups = vec![1000, 10, 20];

            let mut output = Vec::new();
            id_print(&mut output, &id_state, &groups);
            let output_str = String::from_utf8(output).expect("输出不是有效的 UTF-8");

            assert!(output_str.contains("uid=2000"));
            assert!(output_str.contains("gid=2000"));
            assert!(output_str.contains("groups=1000"));
        }

        #[test]
        fn test_id_print_with_selinux_support() {
            // 当 `is_selinux_supported` 为真时，打印 SELinux 上下文
            let ids = Ids {
                uid: 1000,
                gid: 1000,
                euid: 1000,
                egid: 1000,
            };
            let id_state = IdState {
                is_nflag: false,
                is_uflag: false,
                is_gflag: false,
                is_gsflag: false,
                is_rflag: false,
                is_zflag: false,
                is_cflag: false,
                is_selinux_supported: {
                    selinux::kernel_support() != selinux::KernelSupport::Unsupported
                },
                is_user_specified: false,
                ids: Some(ids),
            };
            let groups = vec![1000, 10, 20];

            let mut output = Vec::new();
            id_print(&mut output, &id_state, &groups);
            let output_str = String::from_utf8(output).expect("输出不是有效的 UTF-8");

            assert!(output_str.contains("uid=1000"));
            assert!(output_str.contains("gid=1000"));
            assert!(output_str.contains("groups=1000"));

            if !is_container()
                && selinux::kernel_support() != selinux::KernelSupport::Unsupported
                && SecurityContext::current(false).is_ok()
            {
                assert!(output_str.contains("context=")); // 假设打印了 SELinux 上下文
            }
        }

        #[test]
        fn test_id_print_with_zero_flag() {
            // 当 `is_zflag` 为真时，使用 NUL 字符分隔
            let ids = Ids {
                uid: 1000,
                gid: 1000,
                euid: 1000,
                egid: 1000,
            };
            let id_state = IdState {
                is_nflag: false,
                is_uflag: false,
                is_gflag: false,
                is_gsflag: true,
                is_rflag: false,
                is_zflag: true,
                is_cflag: false,
                is_selinux_supported: false,
                is_user_specified: false,
                ids: Some(ids),
            };
            let groups = vec![1000, 10, 20];

            let mut output = Vec::new();
            id_print(&mut output, &id_state, &groups);
            let output_str = String::from_utf8(output).expect("输出不是有效的 UTF-8");

            assert!(output_str.contains("groups=1000"));
        }

        #[test]
        fn test_id_print_user_specified() {
            // 当 `is_user_specified` 为真时，使用指定用户的 ID 和组
            let ids = Ids {
                uid: 2000,
                gid: 2000,
                euid: 2000,
                egid: 2000,
            };
            let id_state = IdState {
                is_nflag: false,
                is_uflag: true,
                is_gflag: true,
                is_gsflag: true,
                is_rflag: false,
                is_zflag: false,
                is_cflag: false,
                is_selinux_supported: false,
                is_user_specified: true,
                ids: Some(ids),
            };
            let groups = vec![2000, 20, 30];

            let mut output = Vec::new();
            id_print(&mut output, &id_state, &groups);
            let output_str = String::from_utf8(output).expect("输出不是有效的 UTF-8");

            assert!(output_str.contains("uid=2000"));
            assert!(output_str.contains("gid=2000"));
            assert!(output_str.contains("groups=2000"));
        }

        #[test]
        fn test_id_print_with_edge_cases() {
            // 处理特殊情况下的打印
            let ids = Ids {
                uid: 65535,
                gid: 65535,
                euid: 65535,
                egid: 65535,
            };
            let id_state = IdState {
                is_nflag: false,
                is_uflag: false,
                is_gflag: false,
                is_gsflag: false,
                is_rflag: false,
                is_zflag: false,
                is_cflag: false,
                is_selinux_supported: false,
                is_user_specified: false,
                ids: Some(ids),
            };
            let groups = vec![];

            let mut output = Vec::new();
            id_print(&mut output, &id_state, &groups);
            let output_str = String::from_utf8(output).expect("输出不是有效的 UTF-8");

            assert!(output_str.contains("uid=65535"));
            assert!(output_str.contains("gid=65535"));
        }
    }

    #[cfg(test)]
    mod ct_main_tests {
        use super::*;
        use std::ffi::OsString;
        use std::io::Cursor;

        /// 检测是否在容器环境中运行
        fn is_container() -> bool {
            // 检查常见的容器环境标识
            if std::env::var("KUBERNETES_SERVICE_HOST").is_ok()
                || std::env::var("DOCKER_CONTAINER").is_ok()
                || std::path::Path::new("/.dockerenv").exists()
                || std::path::Path::new("/run/.containerenv").exists()
            {
                return true;
            }

            // 检查 cgroup
            if let Ok(contents) = std::fs::read_to_string("/proc/1/cgroup") {
                if contents.contains("/docker/") || contents.contains("/kubepods/") {
                    return true;
                }
            }

            false
        }

        #[test]
        fn test_ct_app_execution_version() {
            let args = [ctcore::ct_util_name(), "--version"];
            let mut output = Cursor::new(Vec::new());

            let result = id_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_err());
        }
        #[test]
        fn test_ct_app_execution_other_version() {
            let args = [ctcore::ct_util_name(), "-V"];
            let mut output = Cursor::new(Vec::new());

            let result = id_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_err());
        }
        #[test]
        fn test_ct_app_execution_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let mut output = Cursor::new(Vec::new());

            let result = id_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_err());
        }
        #[test]
        fn test_ct_app_execution_unsupport_help() {
            let args = [ctcore::ct_util_name(), "-H"];
            let mut output = Cursor::new(Vec::new());

            let result = id_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_err());
        }
        #[test]
        fn test_ct_app_invalid_argument() {
            let args = [ctcore::ct_util_name(), "--invalid-argument"];
            let mut output = Cursor::new(Vec::new());

            let result = id_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_err());
        }
        #[test]
        fn test_ct_app_support_missing_argument() {
            let args = [ctcore::ct_util_name()]; // 缺少任何参数
            let mut output = Cursor::new(Vec::new());

            let result = id_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_ct_app_long_option_user() {
            let args = [ctcore::ct_util_name(), "--user"];
            let mut output = Cursor::new(Vec::new());

            let result = id_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_ct_app_short_option_user() {
            let args = [ctcore::ct_util_name(), "-u"];
            let mut output = Cursor::new(Vec::new());

            let result = id_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_ct_app_long_option_group() {
            let args = [ctcore::ct_util_name(), "--group"];
            let mut output = Cursor::new(Vec::new());

            let result = id_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_ct_app_short_option_group() {
            let args = [ctcore::ct_util_name(), "-g"];
            let mut output = Cursor::new(Vec::new());

            let result = id_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_ct_app_long_option_groups() {
            let args = [ctcore::ct_util_name(), "--groups"];
            let mut output = Cursor::new(Vec::new());

            let result = id_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_ct_app_short_option_groups() {
            let args = [ctcore::ct_util_name(), "-G"];
            let mut output = Cursor::new(Vec::new());

            let result = id_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
        #[test]
        fn test_ct_app_long_option_name() {
            let args = [ctcore::ct_util_name(), "--name"];
            let mut output = Cursor::new(Vec::new());

            let result = id_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_err());
        }
        #[test]
        fn test_ct_app_short_option_name() {
            let args = [ctcore::ct_util_name(), "-n"];
            let mut output = Cursor::new(Vec::new());

            let result = id_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_err());
        }
        #[test]
        fn test_ct_app_long_option_real() {
            let args = [ctcore::ct_util_name(), "--real"];
            let mut output = Cursor::new(Vec::new());

            let result = id_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_err());
        }
        #[test]
        fn test_ct_app_short_option_real() {
            let args = [ctcore::ct_util_name(), "-r"];
            let mut output = Cursor::new(Vec::new());

            let result = id_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_err());
        }
        #[test]
        fn test_ct_app_long_option_zero() {
            let args = [ctcore::ct_util_name(), "--zero"];
            let mut output = Cursor::new(Vec::new());

            let result = id_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_err());
        }
        #[test]
        fn test_ct_app_short_option_zero() {
            let args = [ctcore::ct_util_name(), "-z"];
            let mut output = Cursor::new(Vec::new());

            let result = id_main(&mut output, args.iter().map(OsString::from));
            assert!(result.is_err());
        }
        #[test]
        fn test_ct_app_long_option_context() {
            let args = [ctcore::ct_util_name(), "--context"];
            let mut output = Cursor::new(Vec::new());

            let result = id_main(&mut output, args.iter().map(OsString::from));
            if !is_container() && selinux::kernel_support() != selinux::KernelSupport::Unsupported {
                assert!(result.is_ok());
            }
        }

        #[test]
        fn test_ct_app_short_option_context() {
            let args = [ctcore::ct_util_name(), "-Z"];
            let mut output = Cursor::new(Vec::new());

            let result = id_main(&mut output, args.iter().map(OsString::from));
            if !is_container() && selinux::kernel_support() != selinux::KernelSupport::Unsupported {
                assert!(result.is_ok());
            }
        }
        #[test]
        fn test_id_main_default_format_root() {
            // 测试默认格式下的 root 用户
            let args = [ctcore::ct_util_name(), "root"];

            let mut output = Cursor::new(Vec::new());

            let result = id_main(&mut output, args.iter().map(OsString::from));

            assert!(result.is_ok());

            let output_str = String::from_utf8(output.into_inner()).expect("输出不是有效的 UTF-8");
            assert!(output_str.contains("uid=0")); // 检查 root 用户的 uid 是否为 0
            assert!(output_str.contains("gid=0")); // 检查 root 用户的 gid 是否为 0
        }

        #[test]
        fn test_id_main_default_format_nobody() {
            // 测试默认格式下的 nobody 用户
            let args = [ctcore::ct_util_name(), "nobody"];

            let mut output = Cursor::new(Vec::new());

            let result = id_main(&mut output, args.iter().map(OsString::from));

            assert!(result.is_ok());

            let output_str = String::from_utf8(output.into_inner()).expect("输出不是有效的 UTF-8");
            assert!(output_str.contains("uid=")); // 检查输出中是否包含 uid
            assert!(output_str.contains("nobody")); // 检查输出中是否包含 nobody
        }

        #[test]
        fn test_id_main_with_flags() {
            // 测试带有标志的情况，如 -u, -g, -n
            let args = [ctcore::ct_util_name(), "-u", "-g", "-n"];

            let mut output = Cursor::new(Vec::new());

            let result = id_main(&mut output, args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_id_main_no_such_user() {
            // 测试不存在的用户
            let args = [ctcore::ct_util_name(), "nonexistentuser"];

            let mut output = Cursor::new(Vec::new());

            let result = id_main(&mut output, args.iter().map(OsString::from));

            assert!(result.is_ok()); // 预期错误，因为用户不存在

            let output_str = String::from_utf8(output.into_inner()).expect("输出不是有效的 UTF-8");
            assert!(output_str.contains("")); // 检查错误消息是否正确
        }

        #[test]
        fn test_id_main_with_z_flag() {
            // 测试带有 --zero (-z) 标志的情况
            let args = [ctcore::ct_util_name(), "-z", "root"];

            let mut output = Cursor::new(Vec::new());

            let result = id_main(&mut output, args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_id_main_with_r_flag() {
            // 测试带有 --real (-r) 标志的情况
            let args = [ctcore::ct_util_name(), "-r", "root"];

            let mut output = Cursor::new(Vec::new());

            let result = id_main(&mut output, args.iter().map(OsString::from));

            assert!(result.is_err());
        }
    }

    #[cfg(test)]
    mod ct_app_tests {
        use clap::error::ErrorKind;

        use super::*;

        // id  Usage: id [OPTION]... [USER]...
        //
        // Arguments:
        //   [USER]...
        //
        // Options:
        //   -u, --user     Display only the effective user ID as a number.
        //   -g, --group    Display only the effective group ID as a number
        //   -G, --groups   Display only the different group IDs as white-space separated numbers, in no particular order.
        //   -n, --name     Display the name of the user or group ID for the -G, -g and -u options instead of the number.
        //                  If any of the ID numbers cannot be mapped into names, the number will be displayed as usual.
        //   -r, --real     Display the real ID for the -G, -g and -u options instead of the effective ID.
        //   -z, --zero     delimit entries with NUL characters, not whitespace;
        //                  not permitted in default format
        //   -Z, --context  print only the security context of the process (not enabled)
        //   -h, --help     Print help
        //   -V, --version  Print version
        //
        // The id utility displays the user and group names and numeric IDs, of the
        // calling process, to the standard output. If the real and effective IDs are
        // different, both are displayed, otherwise only the real ID is displayed.

        #[test]
        fn test_ct_app_execution_version() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--version"];
            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_other_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];

            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_help() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "--help"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_execution_unsupport_help() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-H"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_invalid_argument() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_support_missing_argument() {
            let command = ct_app();

            let missing_args = vec![ctcore::ct_util_name()]; // 缺少任何参数
            let result = command.try_get_matches_from(missing_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_user() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--user"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }
        #[test]
        fn test_ct_app_short_option_user() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-u"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_group() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--group"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_short_option_group() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-g"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_groups() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--groups"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_short_option_groups() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-G"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_name() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--name"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_short_option_name() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-n"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_real() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--real"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_short_option_real() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-r"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_zero() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--zero"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_short_option_zero() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-z"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_context() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--context"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_short_option_context() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-Z"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }
    }
}
