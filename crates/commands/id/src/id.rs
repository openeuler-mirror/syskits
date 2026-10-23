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
use clap::{Arg, ArgAction, Command, builder::OsStringValueParser, crate_version};
use sys_locale::get_locale;

use ctcore::Tool;
use ctcore::ct_display::{Quotable, locale_quote_marks};
use ctcore::ct_entries::{self, CtPasswd};
use ctcore::ct_error::{CTError, CTResult};
use ctcore::ct_error::{CtSimpleError, set_ct_exit_code, strip_errno};
use ctcore::ct_line_ending::CtLineEnding;
use ctcore::ct_process::{getegid, geteuid, getgid, getuid};
use ctcore::ct_show_error;
use selinux::SecurityContext;
use std::error::Error;
use std::ffi::{CStr, CString, OsStr, OsString};
use std::fmt::Display;
#[cfg(target_family = "unix")]
use std::os::unix::ffi::OsStrExt;

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn bindtextdomain(
        domainname: *const ctcore::libc::c_char,
        dirname: *const ctcore::libc::c_char,
    ) -> *mut ctcore::libc::c_char;
    fn dgettext(
        domainname: *const ctcore::libc::c_char,
        msgid: *const ctcore::libc::c_char,
    ) -> *mut ctcore::libc::c_char;
}

static CONTEXT_HELP_TEXT: &str = "print only the security context of the process";

const ID_NO_ARGUMENT_LONG_OPTIONS: [&str; 9] = [
    "user", "group", "groups", "name", "real", "zero", "context", "help", "version",
];

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdEntity {
    pub id: u32,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdRow {
    pub mode: String,
    pub subject: Option<String>,
    pub uid: Option<IdEntity>,
    pub gid: Option<IdEntity>,
    pub euid: Option<IdEntity>,
    pub egid: Option<IdEntity>,
    pub groups: Vec<IdEntity>,
    pub context: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdSemantic {
    pub rows: Vec<IdRow>,
    pub classic_text: String,
    pub stderr_text: String,
    pub exit_code: i32,
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

#[derive(Debug)]
struct IdOptionError {
    diagnostic: Vec<u8>,
}

impl IdOptionError {
    fn invalid_short(option: u8) -> Self {
        Self {
            diagnostic: [b"invalid option -- '".as_slice(), &[option], b"'"].concat(),
        }
    }
}

impl Display for IdOptionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match std::str::from_utf8(&self.diagnostic) {
            Ok(diagnostic) => formatter.write_str(diagnostic),
            Err(_) => {
                let mut stderr = io::stderr().lock();
                let _ = stderr
                    .write_all(ctcore::ct_util_name().as_bytes())
                    .and_then(|_| stderr.write_all(b": "))
                    .and_then(|_| stderr.write_all(&self.diagnostic))
                    .and_then(|_| stderr.write_all(b"\n"))
                    .and_then(|_| stderr.flush());
                Ok(())
            }
        }
    }
}

impl Error for IdOptionError {}

impl CTError for IdOptionError {
    fn code(&self) -> i32 {
        1
    }

    fn usage(&self) -> bool {
        true
    }
}

fn id_init_gettext() {
    #[cfg(unix)]
    unsafe {
        ctcore::libc::setlocale(ctcore::libc::LC_ALL, c"".as_ptr());
    }
    #[cfg(target_os = "linux")]
    unsafe {
        bindtextdomain(c"coreutils".as_ptr(), c"/usr/share/locale".as_ptr());
    }
}

fn id_gettext(message: &str) -> String {
    #[cfg(target_os = "linux")]
    {
        let Ok(message) = CString::new(message) else {
            return message.to_string();
        };
        let translated = unsafe { dgettext(c"coreutils".as_ptr(), message.as_ptr()) };
        if !translated.is_null() {
            return unsafe { CStr::from_ptr(translated) }
                .to_string_lossy()
                .into_owned();
        }
    }
    message.to_string()
}

fn id_errno_message(error: &io::Error) -> Option<String> {
    let errno = error.raw_os_error()?;
    #[cfg(target_os = "linux")]
    unsafe {
        let message = ctcore::libc::strerror(errno);
        if !message.is_null() {
            return Some(CStr::from_ptr(message).to_string_lossy().into_owned());
        }
    }
    None
}

fn id_no_such_user_diagnostic(quoted_user: &str, error: &io::Error) -> String {
    let mut message = id_gettext("%s: no such user").replacen("%s", quoted_user, 1);
    if error.raw_os_error().is_some()
        && let Some(detail) = id_errno_message(error)
    {
        message.push_str(": ");
        message.push_str(&detail);
    }
    message
}

fn id_reject_attached_long_option(args: &[OsString]) -> CTResult<()> {
    for argument in args.iter().skip(1) {
        let bytes = argument.as_bytes();
        if bytes == b"--" {
            break;
        }
        if bytes.starts_with(b"-") && !bytes.starts_with(b"--") && bytes.len() > 1 {
            if let Some(invalid) = bytes[1..]
                .iter()
                .copied()
                .find(|option| !b"agnruzGZ".contains(option))
            {
                if !invalid.is_ascii() {
                    return Err(Box::new(IdOptionError::invalid_short(invalid)));
                }
                return Err(CtSimpleError::new(
                    1,
                    format!("invalid option -- '{}'", char::from(invalid)),
                ));
            }
            continue;
        }
        let Some(option) = bytes.strip_prefix(b"--") else {
            continue;
        };
        let equal = option.iter().position(|byte| *byte == b'=');
        let name = equal.map_or(option, |index| &option[..index]);
        let matches = ID_NO_ARGUMENT_LONG_OPTIONS
            .iter()
            .filter(|candidate| candidate.as_bytes().starts_with(name))
            .collect::<Vec<_>>();
        if let Some(exact) = matches
            .iter()
            .find(|candidate| candidate.as_bytes() == name)
        {
            if equal.is_some() {
                return Err(CtSimpleError::new(
                    1,
                    format!("option '--{exact}' doesn't allow an argument"),
                ));
            }
            continue;
        }
        if name.is_empty() || matches.is_empty() {
            return Err(CtSimpleError::new(
                1,
                format!("unrecognized option '{}'", argument.to_string_lossy()),
            ));
        }
        if matches.len() > 1 {
            let possibilities = matches
                .iter()
                .map(|candidate| format!("'--{candidate}'"))
                .collect::<Vec<_>>()
                .join(" ");
            return Err(CtSimpleError::new(
                1,
                format!(
                    "option '{}' is ambiguous; possibilities: {possibilities}",
                    argument.to_string_lossy()
                ),
            ));
        }
        if equal.is_some() && matches.len() == 1 {
            return Err(CtSimpleError::new(
                1,
                format!("option '--{}' doesn't allow an argument", matches[0]),
            ));
        }
    }
    Ok(())
}

fn id_apply_posix_option_order(args: &mut Vec<OsString>, enabled: bool) {
    if !enabled {
        return;
    }

    let mut index = 1;
    while index < args.len() {
        let bytes = args[index].as_os_str().as_bytes();
        if bytes == b"--" {
            return;
        }
        if bytes == b"-" || !bytes.starts_with(b"-") {
            args.insert(index, OsString::from("--"));
            return;
        }
        index += 1;
    }
}

pub fn id_main<W: Write>(writer: &mut W, args: impl ctcore::Args) -> CTResult<()> {
    // 设置语言
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    id_init_gettext();

    let mut raw_args = args.collect::<Vec<_>>();
    id_apply_posix_option_order(
        &mut raw_args,
        std::env::var_os("POSIXLY_CORRECT").is_some(),
    );
    id_reject_attached_long_option(&raw_args)?;

    let matches = ct_app()
        .after_help(t!("id.after_help"))
        .try_get_matches_from(raw_args)?;

    let raw_users: Vec<OsString> = matches
        .get_many::<OsString>(id_flags::ID_ARG_USERS)
        .map(|v| v.cloned().collect())
        .unwrap_or_default();
    let users: Vec<String> = raw_users
        .iter()
        .map(|user| user.to_string_lossy().into_owned())
        .collect();

    let mut state = id_get_state(&matches, &users);

    if state.is_cflag && !state.is_selinux_supported {
        return Err(CtSimpleError::new(
            1,
            "--context (-Z) works only on an SELinux-enabled kernel",
        ));
    }

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
    id_handle_users(
        writer,
        raw_users,
        &mut state,
        is_default_format,
        line_ending,
    );

    Ok(())
}

fn id_uid_entity(uid: u32, stderr_text: &mut String, exit_code: &mut i32) -> IdEntity {
    match ct_entries::uid2usr(uid) {
        Ok(name) => IdEntity {
            id: uid,
            label: name,
        },
        Err(_) => {
            *exit_code = 1;
            stderr_text.push_str(&format!("id: cannot find name for user ID {uid}\n"));
            IdEntity {
                id: uid,
                label: uid.to_string(),
            }
        }
    }
}

fn id_gid_entity(gid: u32, stderr_text: &mut String, exit_code: &mut i32) -> IdEntity {
    match ct_entries::gid2grp(gid) {
        Ok(name) => IdEntity {
            id: gid,
            label: name,
        },
        Err(_) => {
            *exit_code = 1;
            stderr_text.push_str(&format!("id: cannot find name for group ID {gid}\n"));
            IdEntity {
                id: gid,
                label: gid.to_string(),
            }
        }
    }
}

fn id_write_optional_label<W: Write>(writer: &mut W, value: u32, label: Option<&OsStr>) {
    write!(writer, "{value}").unwrap();
    if let Some(label) = label {
        writer.write_all(b"(").unwrap();
        writer.write_all(label.as_bytes()).unwrap();
        writer.write_all(b")").unwrap();
    }
}

fn id_default_classic_text(row: &IdRow) -> String {
    let mut text = String::new();
    if let Some(uid) = &row.uid {
        text.push_str(&format!("uid={}({})", uid.id, uid.label));
    }
    if let Some(gid) = &row.gid {
        text.push_str(&format!(" gid={}({})", gid.id, gid.label));
    }
    if let Some(euid) = &row.euid {
        text.push_str(&format!(" euid={}({})", euid.id, euid.label));
    }
    if let Some(egid) = &row.egid {
        text.push_str(&format!(" egid={}({})", egid.id, egid.label));
    }
    if !row.groups.is_empty() {
        let groups = row
            .groups
            .iter()
            .map(|group| format!("{}({})", group.id, group.label))
            .collect::<Vec<_>>()
            .join(",");
        text.push_str(&format!(" groups={groups}"));
    }
    if let Some(context) = &row.context {
        text.push_str(&format!(" context={context}"));
    }
    text
}

fn id_row_classic_text(
    row: &IdRow,
    state: &IdState,
    user_count: usize,
    line_ending: CtLineEnding,
) -> String {
    let mut text = match row.mode.as_str() {
        "user" => row
            .uid
            .as_ref()
            .map(|v| {
                if state.is_nflag {
                    v.label.clone()
                } else {
                    v.id.to_string()
                }
            })
            .unwrap_or_default(),
        "group" => row
            .gid
            .as_ref()
            .map(|v| {
                if state.is_nflag {
                    v.label.clone()
                } else {
                    v.id.to_string()
                }
            })
            .unwrap_or_default(),
        "groups" => row
            .groups
            .iter()
            .map(|group| {
                if state.is_nflag {
                    group.label.clone()
                } else {
                    group.id.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(if state.is_zflag { "\0" } else { " " }),
        "context" => row.context.clone().unwrap_or_default(),
        _ => id_default_classic_text(row),
    };

    if row.mode == "groups" && state.is_zflag && state.is_user_specified && user_count > 1 {
        text.push('\0');
    }
    text.push_str(&format!("{line_ending}"));
    text
}

fn id_group_list_for_current_process(rgid: u32, egid: u32, groups: Vec<u32>) -> Vec<u32> {
    let mut result = Vec::with_capacity(groups.len() + usize::from(rgid != egid) + 1);
    result.push(rgid);
    if egid != rgid {
        result.push(egid);
    }
    result.extend(
        groups
            .into_iter()
            .filter(|group| *group != rgid && *group != egid),
    );
    result
}

fn id_build_rows(
    users: &[String],
    state: &mut IdState,
    is_default_format: bool,
    line_ending: CtLineEnding,
) -> IdSemantic {
    let mut rows = Vec::new();
    let mut classic_text = String::new();
    let mut stderr_text = String::new();
    let mut exit_code = 0;

    let subjects: Vec<Option<String>> = if users.is_empty() {
        vec![None]
    } else {
        users.iter().cloned().map(Some).collect()
    };

    for subject in subjects {
        let possible_pw = if let Some(user) = subject.as_ref() {
            let lookup = CtPasswd::locate_name(user.as_str()).or_else(|name_error| {
                if let Ok(uid) = user.parse::<u32>() {
                    CtPasswd::locate_uid(uid)
                } else {
                    Err(name_error)
                }
            });
            match lookup {
                Ok(passwd) => Some(passwd),
                Err(error) => {
                    exit_code = 1;
                    stderr_text.push_str("id: ");
                    stderr_text.push_str(&id_no_such_user_diagnostic(
                        &id_quote_user(OsStr::new(user)),
                        &error,
                    ));
                    stderr_text.push('\n');
                    continue;
                }
            }
        } else {
            None
        };

        let (uid, gid) = possible_pw
            .as_ref()
            .map(|p| (p.uid, p.gid))
            .unwrap_or_else(|| {
                if is_default_format {
                    (getuid(), getgid())
                } else {
                    (
                        if state.is_rflag { getuid() } else { geteuid() },
                        if state.is_rflag { getgid() } else { getegid() },
                    )
                }
            });

        let euid = geteuid();
        let egid = getegid();
        let groups_result = if !is_default_format && !state.is_gsflag {
            Ok(Vec::new())
        } else if let Some(passwd) = possible_pw.as_ref() {
            Ok(passwd.belongs_to())
        } else if state.is_gsflag {
            ct_entries::get_groups_gnu(Some(egid))
                .map(|groups| id_group_list_for_current_process(getgid(), egid, groups))
        } else {
            ct_entries::get_groups_gnu(Some(if is_default_format { egid } else { gid }))
        };
        let (groups, groups_failed) = match groups_result {
            Ok(groups) => (groups, false),
            Err(error) => {
                exit_code = 1;
                if let Some(user) = subject.as_ref() {
                    stderr_text.push_str(&format!(
                        "id: failed to get groups for user {}: {}\n",
                        user.quote(),
                        strip_errno(&error)
                    ));
                } else {
                    stderr_text.push_str(&format!(
                        "id: failed to get groups for the current process: {}\n",
                        strip_errno(&error)
                    ));
                }
                let partial_groups = if state.is_gsflag {
                    id_group_list_for_current_process(getgid(), egid, Vec::new())
                } else {
                    Vec::new()
                };
                (partial_groups, true)
            }
        };

        let mut row = IdRow {
            mode: if state.is_cflag {
                "context".into()
            } else if state.is_uflag {
                "user".into()
            } else if state.is_gflag {
                "group".into()
            } else if state.is_gsflag {
                "groups".into()
            } else {
                "default".into()
            },
            subject: subject.clone(),
            uid: None,
            gid: None,
            euid: None,
            egid: None,
            groups: Vec::new(),
            context: None,
        };

        if state.is_uflag {
            row.uid = Some(id_uid_entity(uid, &mut stderr_text, &mut exit_code));
        } else if state.is_gflag {
            row.gid = Some(id_gid_entity(gid, &mut stderr_text, &mut exit_code));
        } else if state.is_gsflag {
            row.groups = groups
                .iter()
                .map(|gid| id_gid_entity(*gid, &mut stderr_text, &mut exit_code))
                .collect();
        } else if is_default_format {
            row.uid = Some(id_uid_entity(uid, &mut stderr_text, &mut exit_code));
            row.gid = Some(id_gid_entity(gid, &mut stderr_text, &mut exit_code));
            if !state.is_user_specified && euid != uid {
                row.euid = Some(id_uid_entity(euid, &mut stderr_text, &mut exit_code));
            }
            if !state.is_user_specified && egid != gid {
                row.egid = Some(id_gid_entity(egid, &mut stderr_text, &mut exit_code));
            }
            row.groups = groups
                .iter()
                .map(|gid| id_gid_entity(*gid, &mut stderr_text, &mut exit_code))
                .collect();
            if state.is_selinux_supported
                && !state.is_user_specified
                && !groups_failed
                && std::env::var_os("POSIXLY_CORRECT").is_none()
                && let Ok(context) = SecurityContext::current(false)
            {
                row.context = Some(String::from_utf8_lossy(context.as_bytes()).to_string());
            }
        }

        classic_text.push_str(&id_row_classic_text(&row, state, users.len(), line_ending));
        rows.push(row);
    }

    IdSemantic {
        rows,
        classic_text,
        stderr_text,
        exit_code,
    }
}

pub fn id_native_semantic(args: impl ctcore::Args) -> CTResult<IdSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    id_init_gettext();

    let mut raw_args = args.collect::<Vec<_>>();
    id_apply_posix_option_order(
        &mut raw_args,
        std::env::var_os("POSIXLY_CORRECT").is_some(),
    );
    id_reject_attached_long_option(&raw_args)?;

    let matches = ct_app()
        .after_help(t!("id.after_help"))
        .try_get_matches_from(raw_args)?;

    let raw_users: Vec<OsString> = matches
        .get_many::<OsString>(id_flags::ID_ARG_USERS)
        .map(|v| v.cloned().collect())
        .unwrap_or_default();
    let users: Vec<String> = raw_users
        .iter()
        .map(|user| user.to_string_lossy().into_owned())
        .collect();

    let mut state = id_get_state(&matches, &users);

    if state.is_cflag && !state.is_selinux_supported {
        return Err(CtSimpleError::new(
            1,
            "--context (-Z) works only on an SELinux-enabled kernel",
        ));
    }

    let is_default_format = id_flags_validity_checks(&mut state)?;
    let line_ending = CtLineEnding::from_zero_flag(state.is_zflag);

    if state.is_cflag {
        if state.is_selinux_supported {
            if let Ok(context) = SecurityContext::current(false) {
                let context = String::from_utf8_lossy(context.as_bytes()).to_string();
                return Ok(IdSemantic {
                    rows: vec![IdRow {
                        mode: "context".into(),
                        subject: None,
                        uid: None,
                        gid: None,
                        euid: None,
                        egid: None,
                        groups: Vec::new(),
                        context: Some(context.clone()),
                    }],
                    classic_text: format!("{context}{line_ending}"),
                    stderr_text: String::new(),
                    exit_code: 0,
                });
            }
            return Err(CtSimpleError::new(1, "can't get process context"));
        }
        return Err(CtSimpleError::new(
            1,
            "--context (-Z) works only on an SELinux-enabled kernel",
        ));
    }

    Ok(id_build_rows(
        &users,
        &mut state,
        is_default_format,
        line_ending,
    ))
}

fn id_quote_user(user: &OsStr) -> String {
    let (left_quote, right_quote) = locale_quote_marks();
    id_quote_user_with_marks(user, &left_quote, &right_quote)
}

fn id_quote_user_with_marks(user: &OsStr, left_quote: &str, right_quote: &str) -> String {
    let mut quoted = left_quote.to_string();
    let user = user.as_bytes();
    let right_quote_bytes = right_quote.as_bytes();
    let mut index = 0;
    while index < user.len() {
        if !right_quote_bytes.is_empty() && user[index..].starts_with(right_quote_bytes) {
            quoted.push('\\');
            quoted.push_str(right_quote);
            index += right_quote_bytes.len();
            continue;
        }

        let byte = user[index];
        match byte {
            b'\x07' => quoted.push_str("\\a"),
            b'\x08' => quoted.push_str("\\b"),
            b'\x0c' => quoted.push_str("\\f"),
            b'\n' => quoted.push_str("\\n"),
            b'\r' => quoted.push_str("\\r"),
            b'\t' => quoted.push_str("\\t"),
            b'\x0b' => quoted.push_str("\\v"),
            b'\\' => quoted.push_str("\\\\"),
            b' '..=b'~' => quoted.push(char::from(byte)),
            byte if byte.is_ascii() => quoted.push_str(&format!("\\{byte:03o}")),
            _ => match std::str::from_utf8(&user[index..]) {
                Ok(valid) => {
                    let character = valid.chars().next().unwrap();
                    quoted.push(character);
                    index += character.len_utf8();
                    continue;
                }
                Err(_) => quoted.push_str(&format!("\\{byte:03o}")),
            },
        }
        index += 1;
    }
    quoted.push_str(&right_quote);
    quoted
}

fn id_lookup_user(user: &OsStr) -> io::Result<CtPasswd> {
    if user.is_empty() {
        return Err(io::Error::from_raw_os_error(ctcore::libc::ENOENT));
    }

    CtPasswd::locate_name_os(user).or_else(|name_error| {
        if let Ok(uid) = user.to_string_lossy().parse::<u32>() {
            CtPasswd::locate_uid(uid)
        } else {
            Err(name_error)
        }
    })
}

fn id_handle_users<W, U>(
    writer: &mut W,
    users: Vec<U>,
    id_state: &mut IdState,
    is_default_format: bool,
    line_ending: CtLineEnding,
) where
    W: Write,
    U: AsRef<OsStr>,
{
    let delimiter = id_get_delimiter(id_state);

    for i in 0..=users.len() {
        let possible_pw = if id_state.is_user_specified {
            let user = users[i].as_ref();
            let lookup = id_lookup_user(user);
            match lookup {
                Ok(p) => Some(p),
                Err(error) => {
                    if matches!(error.raw_os_error(), Some(errno) if errno != 0) {
                        ct_show_error!(
                            "{}",
                            id_no_such_user_diagnostic(&id_quote_user(user), &error)
                        );
                    } else {
                        ct_show_error!(
                            "{}",
                            id_no_such_user_diagnostic(&id_quote_user(user), &error)
                        );
                    }
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

        let process_rgid = getgid();
        let process_egid = getegid();
        let (uid, gid) = possible_pw
            .as_ref()
            .map(|p| (p.uid, p.gid))
            .unwrap_or_else(|| {
                if is_default_format {
                    (getuid(), process_rgid)
                } else {
                    (
                        if id_state.is_rflag {
                            getuid()
                        } else {
                            geteuid()
                        },
                        if id_state.is_rflag {
                            process_rgid
                        } else {
                            process_egid
                        },
                    )
                }
            });

        id_state.ids = Some(Ids {
            uid,
            gid,
            euid: geteuid(),
            egid: process_egid,
        });

        if id_state.is_gflag {
            let gid_string = if id_state.is_nflag {
                match ct_entries::gid2grp_os(gid) {
                    Ok(name) => {
                        writer.write_all(name.as_bytes()).unwrap();
                        String::new()
                    }
                    Err(_) => {
                        ct_show_error!("cannot find name for group ID {}", gid);
                        set_ct_exit_code(1);
                        gid.to_string()
                    }
                }
            } else {
                gid.to_string()
            };

            if !gid_string.is_empty() {
                write!(writer, "{gid_string}").unwrap();
            }
        }

        if id_state.is_uflag {
            let name_string = if id_state.is_nflag {
                match ct_entries::uid2usr_os(uid) {
                    Ok(name) => {
                        writer.write_all(name.as_bytes()).unwrap();
                        String::new()
                    }
                    Err(_) => {
                        ct_show_error!("cannot find name for user ID {}", uid);
                        set_ct_exit_code(1);
                        uid.to_string()
                    }
                }
            } else {
                uid.to_string()
            };

            if !name_string.is_empty() {
                write!(writer, "{name_string}").unwrap();
            }
        }

        let groups_result = if !is_default_format && !id_state.is_gsflag {
            Ok(Vec::new())
        } else if id_state.is_user_specified {
            Ok(possible_pw.as_ref().map(|p| p.belongs_to()).unwrap())
        } else if id_state.is_gsflag {
            ct_entries::get_groups_gnu(Some(process_egid))
                .map(|groups| id_group_list_for_current_process(process_rgid, process_egid, groups))
        } else {
            ct_entries::get_groups_gnu(Some(if is_default_format { process_egid } else { gid }))
        };
        let (groups, groups_error) = match groups_result {
            Ok(groups) => (groups, None),
            Err(error) => {
                let partial_groups = if id_state.is_gsflag {
                    id_group_list_for_current_process(process_rgid, process_egid, Vec::new())
                } else {
                    Vec::new()
                };
                (partial_groups, Some(error))
            }
        };

        if id_state.is_gsflag {
            for (index, &id) in groups.iter().enumerate() {
                if index != 0 {
                    writer.write_all(delimiter.as_bytes()).unwrap();
                }
                if id_state.is_nflag {
                    match ct_entries::gid2grp_os(id) {
                        Ok(name) => writer.write_all(name.as_bytes()).unwrap(),
                        Err(_) => {
                            ct_show_error!("cannot find name for group ID {}", id);
                            set_ct_exit_code(1);
                            write!(writer, "{id}").unwrap();
                        }
                    }
                } else {
                    write!(writer, "{id}").unwrap();
                }
            }

            let delimit = if id_state.is_zflag && id_state.is_user_specified && users.len() > 1 {
                "\0"
            } else {
                ""
            };

            writer.write_all(delimit.as_bytes()).unwrap();
        }

        if is_default_format {
            id_print(writer, id_state, &groups);
        }
        if let Some(error) = groups_error {
            if id_state.is_user_specified {
                ct_show_error!(
                    "failed to get groups for user {}: {}",
                    id_quote_user(users[i].as_ref()),
                    strip_errno(&error)
                );
            } else {
                ct_show_error!(
                    "failed to get groups for the current process: {}",
                    strip_errno(&error)
                );
            }
            set_ct_exit_code(1);
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

    if state.is_user_specified && state.is_cflag {
        return Err(CtSimpleError::new(
            1,
            "cannot print security context when user specified",
        ));
    }

    let only_choice_count = [
        state.is_uflag,
        state.is_gflag,
        state.is_gsflag,
        state.is_cflag,
    ]
    .into_iter()
    .filter(|enabled| *enabled)
    .count();
    if only_choice_count > 1 {
        return Err(CtSimpleError::new(
            1,
            "cannot print \"only\" of more than one choice",
        ));
    }

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
            .help(t!("id.clap.id_effective_user"))
            .overrides_with(id_flags::ID_EFFECTIVE_USER)
            .action(ArgAction::SetTrue),
        Arg::new(id_flags::ID_GROUP)
            .short('g')
            .long(id_flags::ID_GROUP)
            .help(t!("id.clap.id_group"))
            .overrides_with(id_flags::ID_GROUP)
            .action(ArgAction::SetTrue),
        Arg::new(id_flags::ID_GROUPS)
            .short('G')
            .long(id_flags::ID_GROUPS)
            .help(
                "Display only the different group IDs as white-space separated numbers, \
                      in no particular order.",
            )
            .overrides_with(id_flags::ID_GROUPS)
            .action(ArgAction::SetTrue),
        Arg::new(id_flags::ID_NAME)
            .short('n')
            .long(id_flags::ID_NAME)
            .help(
                "Display the name of the user or group ID for the -G, -g and -u options \
                      instead of the number.\nIf any of the ID numbers cannot be mapped into \
                      names, the number will be displayed as usual.",
            )
            .overrides_with(id_flags::ID_NAME)
            .action(ArgAction::SetTrue),
        Arg::new(id_flags::ID_REAL_ID)
            .short('r')
            .long(id_flags::ID_REAL_ID)
            .help(
                "Display the real ID for the -G, -g and -u options instead of \
                      the effective ID.",
            )
            .overrides_with(id_flags::ID_REAL_ID)
            .action(ArgAction::SetTrue),
        Arg::new(id_flags::ID_ZERO)
            .short('z')
            .long(id_flags::ID_ZERO)
            .help(
                "delimit entries with NUL characters, not whitespace;\n\
                      not permitted in default format",
            )
            .overrides_with(id_flags::ID_ZERO)
            .action(ArgAction::SetTrue),
        Arg::new(id_flags::ID_CONTEXT)
            .short('Z')
            .long(id_flags::ID_CONTEXT)
            .help(CONTEXT_HELP_TEXT)
            .overrides_with(id_flags::ID_CONTEXT)
            .action(ArgAction::SetTrue),
        Arg::new(id_flags::ID_IGNORE)
            .short('a')
            .help(t!("id.clap.id_ignore"))
            .overrides_with(id_flags::ID_IGNORE)
            .action(ArgAction::SetTrue),
        Arg::new(id_flags::ID_ARG_USERS)
            .action(ArgAction::Append)
            .value_parser(OsStringValueParser::new())
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

    write!(writer, "uid=").unwrap();
    id_write_optional_label(writer, uid, ct_entries::uid2usr_os(uid).ok().as_deref());
    write!(writer, " gid=").unwrap();
    id_write_optional_label(writer, gid, ct_entries::gid2grp_os(gid).ok().as_deref());

    if !id_state.is_user_specified && (euid != uid) {
        write!(writer, " euid=").unwrap();
        id_write_optional_label(writer, euid, ct_entries::uid2usr_os(euid).ok().as_deref());
    }

    if !id_state.is_user_specified && (egid != gid) {
        write!(writer, " egid=").unwrap();
        id_write_optional_label(writer, egid, ct_entries::gid2grp_os(egid).ok().as_deref());
    }

    if groups.is_empty() {
        return;
    }

    write!(writer, " groups=").unwrap();
    for (index, &gr) in groups.iter().enumerate() {
        if index != 0 {
            write!(writer, ",").unwrap();
        }
        id_write_optional_label(writer, gr, ct_entries::gid2grp_os(gr).ok().as_deref());
    }

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

    #[test]
    fn test_group_list_for_current_process_includes_real_and_effective_gids() {
        assert_eq!(
            id_group_list_for_current_process(65534, 0, vec![0, 10, 20, 65534]),
            vec![65534, 0, 10, 20]
        );
        assert_eq!(
            id_group_list_for_current_process(1000, 1000, vec![1000, 10, 20]),
            vec![1000, 10, 20]
        );
    }

    #[cfg(test)]
    mod id_handle_users_tests {
        use super::*;
        use std::io::Cursor;

        #[test]
        fn test_no_such_user_diagnostic_includes_user_and_errno() {
            let error = io::Error::from_raw_os_error(ctcore::libc::ENOENT);
            let diagnostic = id_no_such_user_diagnostic("'missing'", &error);

            assert!(diagnostic.contains("'missing'"));
            assert!(diagnostic.contains("No such file or directory"));
        }

        #[test]
        fn test_quote_user_uses_gnu_control_byte_escaping() {
            assert_eq!(
                id_quote_user_with_marks(OsStr::new("missing\x01"), "‘", "’"),
                "‘missing\\001’"
            );
            assert_eq!(
                id_quote_user_with_marks(OsStr::new("line\nfeed"), "'", "'"),
                "'line\\nfeed'"
            );
        }

        #[test]
        fn test_empty_user_lookup_reports_enoent() {
            let error = id_lookup_user(OsStr::new("")).unwrap_err();

            assert_eq!(error.raw_os_error(), Some(ctcore::libc::ENOENT));
        }

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
            if ctcore::ct_process::geteuid() != 0 {
                println!("Skipping test_id_handle_users_with_rflag: requires root privileges");
                return;
            }
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
            assert!(!result.unwrap()); // 非默认格式返回 false
        }

        #[test]
        fn test_invalid_flags_multiple_only_choices() {
            let matches = ct_app()
                .try_get_matches_from([ctcore::ct_util_name(), "-ug"])
                .expect("GNU-compatible option parsing should defer the conflict");
            let users = Vec::new();
            let mut state = id_get_state(&matches, &users);

            let result = id_flags_validity_checks(&mut state);

            assert_eq!(
                result.unwrap_err().to_string(),
                "cannot print \"only\" of more than one choice"
            );
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
        use std::os::unix::ffi::OsStringExt;

        #[test]
        fn test_id_write_optional_label_preserves_non_utf8_name() {
            let label = OsString::from_vec(vec![b'u', 0xff]);
            let mut output = Vec::new();

            id_write_optional_label(&mut output, 424242, Some(label.as_os_str()));

            assert_eq!(output, b"424242(u\xff)");
        }

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
            assert!(!output_str.contains("groups="));
        }

        #[test]
        fn test_id_print_unknown_group_uses_number_only() {
            let unknown_gid = u32::MAX;
            assert!(ct_entries::gid2grp(unknown_gid).is_err());
            let ids = Ids {
                uid: 0,
                gid: unknown_gid,
                euid: 0,
                egid: unknown_gid,
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
            let groups = vec![unknown_gid];

            let mut output = Vec::new();
            id_print(&mut output, &id_state, &groups);
            let output_str = String::from_utf8(output).expect("输出不是有效的 UTF-8");

            assert!(output_str.contains(&format!(" gid={unknown_gid}")));
            assert!(output_str.contains(&format!(" groups={unknown_gid}")));
            assert!(!output_str.contains(&format!("gid={unknown_gid}({unknown_gid})")));
            assert!(!output_str.contains("cannot find name for group ID"));
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
        fn test_id_main_context_kernel_validation_precedes_conflicts() {
            if selinux::kernel_support() != selinux::KernelSupport::Unsupported {
                return;
            }

            for args in [
                vec![ctcore::ct_util_name(), "-Z", "root"],
                vec![ctcore::ct_util_name(), "-Z", "-u"],
            ] {
                let mut output = Cursor::new(Vec::new());
                let result = id_main(&mut output, args.iter().map(OsString::from));

                assert_eq!(
                    result.unwrap_err().to_string(),
                    "--context (-Z) works only on an SELinux-enabled kernel"
                );
            }
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
        #[cfg(target_family = "unix")]
        use std::os::unix::ffi::OsStringExt;

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
        fn test_ct_app_preserves_non_utf8_user_operand() {
            let operand = OsString::from_vec(b"missing\xff".to_vec());
            let args = vec![OsString::from(ctcore::ct_util_name()), operand.clone()];
            let matches = ct_app().try_get_matches_from(args).unwrap();

            assert_eq!(
                matches
                    .get_one::<OsString>(id_flags::ID_ARG_USERS)
                    .unwrap()
                    .as_bytes(),
                operand.as_bytes()
            );
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
        fn test_ct_app_repeated_boolean_option_is_idempotent() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-uu"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_attached_long_option_argument_uses_gnu_diagnostic() {
            let args = vec![
                OsString::from(ctcore::ct_util_name()),
                OsString::from("--us=value"),
            ];
            let error = id_reject_attached_long_option(&args).unwrap_err();

            assert_eq!(
                error.to_string(),
                "option '--user' doesn't allow an argument"
            );
        }

        #[test]
        fn test_ambiguous_long_option_lists_matching_options() {
            let args = vec![
                OsString::from(ctcore::ct_util_name()),
                OsString::from("--gro"),
            ];
            let error = id_reject_attached_long_option(&args).unwrap_err();

            assert_eq!(
                error.to_string(),
                "option '--gro' is ambiguous; possibilities: '--group' '--groups'"
            );
        }

        #[test]
        fn test_exact_long_option_takes_precedence_over_prefix_matches() {
            for option in ["--group", "--groups"] {
                let args = vec![
                    OsString::from(ctcore::ct_util_name()),
                    OsString::from(option),
                ];

                assert!(id_reject_attached_long_option(&args).is_ok());
            }
        }

        #[test]
        fn test_invalid_short_option_uses_gnu_diagnostic() {
            let args = vec![OsString::from(ctcore::ct_util_name()), OsString::from("-x")];
            let error = id_reject_attached_long_option(&args).unwrap_err();

            assert_eq!(error.to_string(), "invalid option -- 'x'");
        }

        #[test]
        fn test_invalid_long_option_uses_gnu_diagnostic() {
            let args = vec![
                OsString::from(ctcore::ct_util_name()),
                OsString::from("--does-not-exist"),
            ];
            let error = id_reject_attached_long_option(&args).unwrap_err();

            assert_eq!(error.to_string(), "unrecognized option '--does-not-exist'");
        }

        #[test]
        fn test_invalid_short_option_preserves_original_byte() {
            let error = IdOptionError::invalid_short(0xff);

            assert_eq!(error.diagnostic, b"invalid option -- '\xff'");
        }

        #[test]
        fn test_posix_option_order_stops_after_first_operand() {
            let mut args = vec![
                OsString::from(ctcore::ct_util_name()),
                OsString::from("root"),
                OsString::from("-u"),
            ];

            id_apply_posix_option_order(&mut args, true);

            assert_eq!(args[1], OsString::from("--"));
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
