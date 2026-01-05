/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
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

use std::io::{self, Write};

use clap::{Arg, ArgAction, Command, crate_version};

use ctcore::ct_display::Quotable;
use ctcore::ct_entries::{self, CtPasswd, Locate};
use ctcore::ct_error::CTResult;
use ctcore::ct_error::{CtSimpleError, set_ct_exit_code};
use ctcore::ct_line_ending::CtLineEnding;
use ctcore::ct_process::{getegid, geteuid, getgid, getuid};
use ctcore::{ct_format_usage, ct_help_about, ct_help_section, ct_help_usage, ct_show_error};

const ID_ABOUT: &str = ct_help_about!("id.md");
const ID_USAGE: &str = ct_help_usage!("id.md");
const ID_AFTER_HELP: &str = ct_help_section!("after help", "id.md");

#[cfg(not(feature = "selinux"))]
static CONTEXT_HELP_TEXT: &str = "print only the security context of the process (not enabled)";
#[cfg(feature = "selinux")]
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
    // * “getgroups”系统调用仅在没有指定用户的情况下使用，这会导致“id”和“id $USER”之间显示组的顺序不同。
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

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    let stdout_info = io::stdout();
    let mut stdout_writer = stdout_info.lock();
    id_main(&mut stdout_writer, args)
}

pub fn id_main<W: Write>(writer: &mut W, args: impl ctcore::Args) -> CTResult<()> {
    let matches = ct_app()
        .after_help(ID_AFTER_HELP)
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
            #[cfg(feature = "selinux")]
            if let Ok(context) = selinux::SecurityContext::current(false) {
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

            write!(writer, "{}", gid_string).unwrap();
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

            write!(writer, "{}", name_string).unwrap();
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

            write!(writer, "{}{}", groups_string, delimit).unwrap();
        }

        if is_default_format {
            id_print(writer, id_state, &groups);
        }
        write!(writer, "{}", line_ending).unwrap();

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

        is_selinux_supported: {
            #[cfg(feature = "selinux")]
            {
                selinux::kernel_support() != selinux::KernelSupport::Unsupported
            }
            #[cfg(not(feature = "selinux"))]
            {
                false
            }
        },
        is_user_specified: !users.is_empty(),
        ids: None,
    }
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = ID_ABOUT;
    let usage_description = ct_format_usage(ID_USAGE);
    let args = vec![
        Arg::new(id_flags::ID_EFFECTIVE_USER)
            .short('u')
            .long(id_flags::ID_EFFECTIVE_USER)
            .conflicts_with(id_flags::ID_GROUP)
            .help("Display only the effective user ID as a number.")
            .action(ArgAction::SetTrue),
        Arg::new(id_flags::ID_GROUP)
            .short('g')
            .long(id_flags::ID_GROUP)
            .conflicts_with(id_flags::ID_EFFECTIVE_USER)
            .help("Display only the effective group ID as a number")
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
        writeln!(writer, "cannot find name for user ID {}", uid).unwrap();
        set_ct_exit_code(1);
        uid.to_string()
    });
    write!(writer, "uid={}({})", uid, uid_string).unwrap();

    let gid_string = ct_entries::gid2grp(gid).unwrap_or_else(|_| {
        writeln!(writer, "cannot find name for group ID {}", gid).unwrap();
        set_ct_exit_code(1);
        gid.to_string()
    });
    write!(writer, " gid={}({})", gid, gid_string).unwrap();

    if !id_state.is_user_specified && (euid != uid) {
        let euid_string = ct_entries::uid2usr(euid).unwrap_or_else(|_| {
            writeln!(writer, "cannot find name for user ID {}", euid).unwrap();
            set_ct_exit_code(1);
            euid.to_string()
        });
        write!(writer, " euid={}({})", euid, euid_string).unwrap();
    }

    if !id_state.is_user_specified && (egid != gid) {
        let egid_string = ct_entries::gid2grp(egid).unwrap_or_else(|_| {
            writeln!(writer, "cannot find name for group ID {}", egid).unwrap();
            set_ct_exit_code(1);
            egid.to_string()
        });
        write!(writer, " egid={}({})", egid, egid_string).unwrap();
    }

    let groups_string = groups
        .iter()
        .map(|&gr| {
            format!(
                "{}({})",
                gr,
                ct_entries::gid2grp(gr).unwrap_or_else(|_| {
                    writeln!(writer, "cannot find name for group ID {}", gr).unwrap();
                    set_ct_exit_code(1);
                    gr.to_string()
                })
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    write!(writer, " groups={}", groups_string).unwrap();

    if id_state.is_selinux_supported
        && !id_state.is_user_specified
        && std::env::var_os("POSIXLY_CORRECT").is_none()
    {
        // print SElinux context (does not depend on "-Z")
        #[cfg(feature = "selinux")]
        if let Ok(context) = selinux::SecurityContext::current(false) {
            let bytes = context.as_bytes();
            write!(writer, " context={}", String::from_utf8_lossy(bytes)).unwrap();
        }
    }
}

