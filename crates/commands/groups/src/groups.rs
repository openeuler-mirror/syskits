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
    ct_entries::{CtPasswd, Locate, get_groups, gid2grp_os},
    ct_error::{CTError, CTResult, FromIo, strip_errno},
    ct_process::{getegid, getgid, getuid},
};
use rust_i18n::t;
use std::error::Error;
use std::fmt::{Display, Write as FmtWrite};
rust_i18n::i18n!("locales", fallback = "en-US");
use clap::{
    Arg, ArgAction, Command,
    builder::OsStringValueParser,
    crate_version,
    error::{ContextKind, ContextValue},
};
use ctcore::Tool;
use std::ffi::{OsStr, OsString};
use std::io::Write as IoWrite;
use std::ops::Range;
use std::os::unix::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, Ordering};
use sys_locale::get_locale;

mod opt_flags {
    pub const USERS: &str = "USERNAME";
}

#[derive(Debug)]

enum GroupsError {
    GetGroupsForCurrentProcessFailed(std::io::Error),
    GroupNotFound(u32),
    UserNotFound(OsString),
    WriteFailed(std::io::Error),
}

impl Error for GroupsError {}
impl CTError for GroupsError {}

#[derive(Debug)]
struct GroupsClapError {
    inner: Box<dyn CTError>,
    diagnostic: Option<Vec<u8>>,
    show_usage: bool,
    exit_code: Option<i32>,
    display_request: bool,
}

impl GroupsClapError {
    #[cfg(test)]
    fn new(error: clap::Error) -> Self {
        Self::new_with_args(error, &[], false)
    }

    fn new_with_args(error: clap::Error, args: &[OsString], stdout_was_closed: bool) -> Self {
        let display_request = matches!(
            error.kind(),
            clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
        );
        let option_diagnostic = match error.kind() {
            clap::error::ErrorKind::UnknownArgument => args
                .iter()
                .skip(1)
                .find(|argument| {
                    argument.to_str().is_none() && argument.as_bytes().first() == Some(&b'-')
                })
                .map(OsString::as_os_str)
                .map(groups_invalid_option_diagnostic)
                .unwrap_or_else(|| match error.get(ContextKind::InvalidArg) {
                    Some(ContextValue::String(argument)) => {
                        groups_invalid_option_diagnostic(OsStr::new(argument))
                    }
                    _ => None,
                }),
            clap::error::ErrorKind::TooManyValues => match error.get(ContextKind::InvalidArg) {
                Some(ContextValue::String(argument))
                    if argument == "--help" || argument == "--version" =>
                {
                    Some(
                        t!("groups.errors.option_argument", option = argument)
                            .as_bytes()
                            .to_vec(),
                    )
                }
                _ => None,
            },
            _ => None,
        };
        let closed_stdout_error = display_request && stdout_was_closed;
        let diagnostic = if closed_stdout_error {
            Some(
                GroupsError::WriteFailed(std::io::Error::from_raw_os_error(libc::EBADF))
                    .to_string()
                    .into_bytes(),
            )
        } else {
            option_diagnostic.map(groups_append_help_hint)
        };
        let show_usage = diagnostic.is_none() && !display_request;
        Self {
            inner: error.into(),
            diagnostic,
            show_usage,
            exit_code: closed_stdout_error.then_some(1),
            display_request,
        }
    }
}

#[cfg(all(target_os = "linux", not(test)))]
fn groups_restore_default_sigpipe() {
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(any(not(target_os = "linux"), test))]
fn groups_restore_default_sigpipe() {}

#[cfg(target_os = "linux")]
static INHERITED_SIGPIPE_IGNORED: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "linux")]
unsafe extern "C" fn groups_capture_inherited_sigpipe() {
    let mut action = std::mem::MaybeUninit::<libc::sigaction>::uninit();
    if unsafe { libc::sigaction(libc::SIGPIPE, std::ptr::null(), action.as_mut_ptr()) } == 0 {
        let action = unsafe { action.assume_init() };
        INHERITED_SIGPIPE_IGNORED.store(action.sa_sigaction == libc::SIG_IGN, Ordering::Relaxed);
    }
}

#[cfg(target_os = "linux")]
#[used]
#[unsafe(link_section = ".init_array")]
static GROUPS_CAPTURE_SIGPIPE: unsafe extern "C" fn() = groups_capture_inherited_sigpipe;

#[cfg(target_os = "linux")]
fn groups_inherited_sigpipe_ignored() -> bool {
    INHERITED_SIGPIPE_IGNORED.load(Ordering::Relaxed)
}

#[cfg(not(target_os = "linux"))]
fn groups_inherited_sigpipe_ignored() -> bool {
    false
}

#[cfg(all(target_os = "linux", not(test)))]
fn groups_restore_inherited_sigpipe() {
    let handler = if groups_inherited_sigpipe_ignored() {
        libc::SIG_IGN
    } else {
        libc::SIG_DFL
    };
    unsafe {
        libc::signal(libc::SIGPIPE, handler);
    }
}

#[cfg(any(not(target_os = "linux"), test))]
fn groups_restore_inherited_sigpipe() {}

fn groups_invalid_option_diagnostic(argument: &OsStr) -> Option<Vec<u8>> {
    let argument = argument.as_bytes();
    if argument.starts_with(b"--") {
        Some(groups_render_option_diagnostic(
            &t!("groups.errors.unrecognized_option", option = "\u{1}"),
            argument,
        ))
    } else {
        argument.get(1).map(|option| {
            groups_render_option_diagnostic(
                &t!("groups.errors.invalid_option", option = "\u{1}"),
                &[*option],
            )
        })
    }
}

fn groups_render_option_diagnostic(template: &str, argument: &[u8]) -> Vec<u8> {
    let (prefix, suffix) = template.split_once('\u{1}').unwrap_or((template, ""));
    [prefix.as_bytes(), argument, suffix.as_bytes()].concat()
}

fn groups_append_help_hint(mut diagnostic: Vec<u8>) -> Vec<u8> {
    diagnostic.push(b'\n');
    diagnostic.extend_from_slice(
        t!(
            "groups.errors.try_help",
            command = ctcore::ct_help_utility_name()
        )
        .as_bytes(),
    );
    diagnostic
}

impl Display for GroupsClapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.diagnostic {
            Some(diagnostic) => match std::str::from_utf8(diagnostic) {
                Ok(diagnostic) => f.write_str(diagnostic),
                Err(_) => {
                    let mut stderr = std::io::stderr().lock();
                    let _ = stderr
                        .write_all(ctcore::ct_util_name().as_bytes())
                        .and_then(|_| stderr.write_all(b": "))
                        .and_then(|_| stderr.write_all(diagnostic))
                        .and_then(|_| stderr.write_all(b"\n"))
                        .and_then(|_| stderr.flush());
                    Ok(())
                }
            },
            None => {
                if self.display_request {
                    groups_restore_inherited_sigpipe();
                }
                Display::fmt(&self.inner, f)
            }
        }
    }
}

impl Error for GroupsClapError {}

impl CTError for GroupsClapError {
    fn code(&self) -> i32 {
        self.exit_code.unwrap_or_else(|| self.inner.code())
    }

    fn usage(&self) -> bool {
        self.show_usage
    }
}

impl Display for GroupsError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::GetGroupsForCurrentProcessFailed(err) => {
                write!(
                    f,
                    "{}: {}",
                    t!("groups.errors.current_process"),
                    strip_errno(err)
                )
            }
            Self::GroupNotFound(gid) => {
                write!(f, "{}", t!("groups.errors.group_not_found", gid = gid))
            }
            Self::UserNotFound(user) => {
                let user = groups_quote_username(user);
                write!(f, "{}", t!("groups.errors.user_not_found", user = user))
            }
            Self::WriteFailed(err) => {
                let detail = if err.raw_os_error() == Some(libc::EBADF) {
                    t!("groups.errors.bad_file_descriptor").to_string()
                } else {
                    strip_errno(err)
                };
                write!(f, "{}: {detail}", t!("groups.errors.write_error"))
            }
        }
    }
}

fn groups_quote_username(user: &OsStr) -> String {
    let quote_mark = t!("groups.quote_mark");
    if quote_mark == "'" && groups_locale_uses_utf8_quotes() {
        groups_quote_username_with_marks(user, "‘", "’")
    } else {
        groups_quote_username_with_mark(user, &quote_mark)
    }
}

fn groups_quote_username_with_mark(user: &OsStr, quote_mark: &str) -> String {
    groups_quote_username_with_marks(user, quote_mark, quote_mark)
}

fn groups_quote_username_with_marks(user: &OsStr, left_quote: &str, right_quote: &str) -> String {
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
            b'\\' => {
                quoted.push('\\');
                quoted.push(char::from(byte));
            }
            b' '..=b'~' => quoted.push(char::from(byte)),
            _ => write!(quoted, "\\{byte:03o}").unwrap(),
        }
        index += 1;
    }
    quoted.push_str(right_quote);
    quoted
}

fn groups_locale_uses_utf8_quotes() -> bool {
    let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
        .into_iter()
        .find_map(|name| std::env::var_os(name).filter(|value| !value.is_empty()));
    let Some(locale) = locale else {
        return false;
    };
    let locale = locale.to_string_lossy();
    let Some((_, codeset)) = locale.rsplit_once('.') else {
        return false;
    };
    let normalized: String = codeset
        .split('@')
        .next()
        .unwrap_or_default()
        .chars()
        .filter(|character| *character != '-' && *character != '_')
        .collect();
    normalized.eq_ignore_ascii_case("utf8")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupsEntry {
    pub user: Option<String>,
    pub groups: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupsOutputEvent {
    Stdout(Range<usize>),
    Stderr(Range<usize>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupsSemantic {
    pub entries: Vec<GroupsEntry>,
    pub classic_text: String,
    pub classic_bytes: Vec<u8>,
    pub stderr_text: String,
    pub output_events: Vec<GroupsOutputEvent>,
    pub exit_code: i32,
}

struct GroupsRawEntry {
    user: Option<OsString>,
    groups: Vec<OsString>,
}

pub fn ct_app() -> Command {
    let posixly_correct = std::env::var_os("POSIXLY_CORRECT").is_some();
    Command::new(ctcore::ct_util_name())
        .version(crate_version!())
        .about(t!("groups.about"))
        .override_usage(t!("groups.usage"))
        .infer_long_args(true)
        .arg(
            Arg::new(opt_flags::USERS)
                .action(ArgAction::Append)
                .value_parser(OsStringValueParser::new())
                .value_name(opt_flags::USERS)
                .value_hint(clap::ValueHint::Username)
                .trailing_var_arg(posixly_correct),
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

fn groups_infallible_gid2grp(gid: &u32, stderr_text: &mut String, exit_code: &mut i32) -> OsString {
    match gid2grp_os(*gid) {
        Ok(grp) => grp,
        Err(_) => {
            *exit_code = 1;
            stderr_text.push_str(&format!("groups: {}\n", GroupsError::GroupNotFound(*gid)));
            OsString::from(gid.to_string())
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

fn groups_render_classic_bytes(entries: &[GroupsRawEntry]) -> Vec<u8> {
    let mut classic_bytes = Vec::new();
    for entry in entries {
        if let Some(user) = &entry.user {
            classic_bytes.extend_from_slice(user.as_bytes());
            classic_bytes.extend_from_slice(b" : ");
        }
        for (index, group) in entry.groups.iter().enumerate() {
            if index != 0 {
                classic_bytes.push(b' ');
            }
            classic_bytes.extend_from_slice(group.as_bytes());
        }
        classic_bytes.push(b'\n');
    }
    classic_bytes
}

fn groups_record_stdout_event(
    entry: &GroupsRawEntry,
    classic_bytes: &mut Vec<u8>,
    output_events: &mut Vec<GroupsOutputEvent>,
) {
    let start = classic_bytes.len();
    classic_bytes.extend_from_slice(&groups_render_classic_bytes(std::slice::from_ref(entry)));
    output_events.push(GroupsOutputEvent::Stdout(start..classic_bytes.len()));
}

fn groups_record_stderr_event(
    start: usize,
    stderr_text: &str,
    output_events: &mut Vec<GroupsOutputEvent>,
) {
    if start != stderr_text.len() {
        output_events.push(GroupsOutputEvent::Stderr(start..stderr_text.len()));
    }
}

fn groups_for_current_process(rgid: u32, egid: u32, groups: Vec<u32>) -> Vec<u32> {
    let mut result = Vec::with_capacity(groups.len() + usize::from(rgid != egid) + 1);
    result.push(rgid);
    if egid != rgid {
        result.push(egid);
    }

    // GNU mgetgroups removes the effective GID and adjacent duplicates only.
    let mut last_group = egid;
    for group in groups {
        if group != egid && group != last_group {
            last_group = group;
            if group != rgid {
                result.push(group);
            }
        }
    }
    result
}

fn groups_for_named_user(primary_gid: u32, groups: Vec<u32>) -> Vec<u32> {
    let mut result = Vec::with_capacity(groups.len() + 1);
    result.push(primary_gid);
    result.extend(groups.into_iter().filter(|group| *group != primary_gid));
    result
}

fn groups_collect_for_users(users: Vec<OsString>) -> CTResult<GroupsSemantic> {
    let mut entries = Vec::new();
    let mut classic_bytes = Vec::new();
    let mut stderr_text = String::new();
    let mut output_events = Vec::new();
    let mut exit_code = 0;

    if users.is_empty() {
        let stderr_start = stderr_text.len();
        nix::errno::Errno::clear();
        let ruid = getuid();
        let uid_error = std::io::Error::last_os_error();
        if ruid == libc::uid_t::MAX && uid_error.raw_os_error().unwrap_or(0) != 0 {
            return Err(uid_error.map_err_context(|| t!("groups.errors.real_uid").to_string()));
        }
        nix::errno::Errno::clear();
        let egid = getegid();
        let gid_error = std::io::Error::last_os_error();
        if egid == libc::gid_t::MAX && gid_error.raw_os_error().unwrap_or(0) != 0 {
            return Err(gid_error.map_err_context(|| t!("groups.errors.effective_gid").to_string()));
        }
        nix::errno::Errno::clear();
        let rgid = getgid();
        let gid_error = std::io::Error::last_os_error();
        if rgid == libc::gid_t::MAX && gid_error.raw_os_error().unwrap_or(0) != 0 {
            return Err(gid_error.map_err_context(|| t!("groups.errors.real_gid").to_string()));
        }
        let (gids, get_groups_error) = match get_groups() {
            Ok(gids) => (groups_for_current_process(rgid, egid, gids), None),
            Err(err) => (
                groups_for_current_process(rgid, egid, Vec::new()),
                Some(GroupsError::GetGroupsForCurrentProcessFailed(err)),
            ),
        };
        let raw_groups: Vec<OsString> = gids
            .iter()
            .map(|gid| groups_infallible_gid2grp(gid, &mut stderr_text, &mut exit_code))
            .collect();
        let groups = raw_groups
            .iter()
            .map(|group| group.to_string_lossy().into_owned())
            .collect();
        if let Some(err) = get_groups_error {
            exit_code = 1;
            stderr_text.push_str(&format!("groups: {err}\n"));
        }
        groups_record_stderr_event(stderr_start, &stderr_text, &mut output_events);
        entries.push(GroupsEntry { user: None, groups });
        let raw_entry = GroupsRawEntry {
            user: None,
            groups: raw_groups,
        };
        groups_record_stdout_event(&raw_entry, &mut classic_bytes, &mut output_events);
    } else {
        for user in users {
            let stderr_start = stderr_text.len();
            match CtPasswd::locate_name_os(&user) {
                Ok(passwd) => {
                    let reverse_passwd = CtPasswd::locate(passwd.uid).ok();
                    if reverse_passwd.is_none() {
                        exit_code = 1;
                    }
                    let group_passwd = reverse_passwd.as_ref().unwrap_or(&passwd);
                    let gids =
                        groups_for_named_user(passwd.gid, group_passwd.belongs_to_name_os(&user));
                    let raw_groups: Vec<OsString> = gids
                        .iter()
                        .map(|gid| groups_infallible_gid2grp(gid, &mut stderr_text, &mut exit_code))
                        .collect();
                    let groups = raw_groups
                        .iter()
                        .map(|group| group.to_string_lossy().into_owned())
                        .collect();
                    groups_record_stderr_event(stderr_start, &stderr_text, &mut output_events);
                    entries.push(GroupsEntry {
                        user: Some(user.to_string_lossy().into_owned()),
                        groups,
                    });
                    let raw_entry = GroupsRawEntry {
                        user: Some(user),
                        groups: raw_groups,
                    };
                    groups_record_stdout_event(&raw_entry, &mut classic_bytes, &mut output_events);
                }
                Err(_) => {
                    exit_code = 1;
                    stderr_text.push_str(&format!("groups: {}\n", GroupsError::UserNotFound(user)));
                    groups_record_stderr_event(stderr_start, &stderr_text, &mut output_events);
                }
            }
        }
    }

    let classic_text = groups_render_classic_text(&entries);
    Ok(GroupsSemantic {
        entries,
        classic_text,
        classic_bytes,
        stderr_text,
        output_events,
        exit_code,
    })
}

pub fn groups_native_semantic(args: impl ctcore::Args) -> CTResult<GroupsSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let args: Vec<OsString> = args.collect();
    let matches = ct_app()
        .try_get_matches_from(args.iter().cloned())
        .map_err(|error| {
            GroupsClapError::new_with_args(error, &args, ctcore::ct_stdout_was_closed())
        })?;
    let users: Vec<OsString> = matches
        .get_many::<OsString>(opt_flags::USERS)
        .map(|v| v.cloned().collect())
        .unwrap_or_default();

    groups_collect_for_users(users)
}

fn groups_write_stdout(
    writer: &mut impl IoWrite,
    bytes: &[u8],
    stdout_was_closed: bool,
) -> CTResult<()> {
    let result = if stdout_was_closed {
        Err(std::io::Error::from_raw_os_error(libc::EBADF))
    } else {
        writer.write_all(bytes)
    };
    groups_handle_stdout_result(result)?;
    groups_handle_stdout_result(writer.flush())
}

fn groups_write_stderr(writer: &mut impl IoWrite, bytes: &[u8]) -> CTResult<()> {
    groups_handle_stderr_result(writer.write_all(bytes))?;
    groups_handle_stderr_result(writer.flush())
}

fn groups_handle_stderr_result(result: std::io::Result<()>) -> CTResult<()> {
    match result {
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {
            if groups_inherited_sigpipe_ignored() {
                Err(1.into())
            } else {
                groups_terminate_with_sigpipe()
            }
        }
        Err(error) => Err(error.into()),
        Ok(()) => Ok(()),
    }
}

fn groups_handle_stdout_result(result: std::io::Result<()>) -> CTResult<()> {
    match result {
        Err(error)
            if error.kind() == std::io::ErrorKind::BrokenPipe
                && !groups_inherited_sigpipe_ignored() =>
        {
            groups_terminate_with_sigpipe()
        }
        Err(error) => Err(Box::new(GroupsError::WriteFailed(error))),
        Ok(()) => Ok(()),
    }
}

fn groups_terminate_with_sigpipe() -> ! {
    // Rust ignores SIGPIPE by default; restore the exec-inherited default action.
    unsafe {
        groups_restore_default_sigpipe();
        libc::raise(libc::SIGPIPE);
        libc::_exit(128 + libc::SIGPIPE);
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
        let result = groups_native_semantic(args.iter().cloned());
        match result {
            Ok(semantic) => {
                let mut stdout = std::io::stdout().lock();
                let mut stderr = std::io::stderr().lock();
                let mut stdout_error = None;
                for event in &semantic.output_events {
                    match event {
                        GroupsOutputEvent::Stdout(range) => {
                            if stdout_error.is_none() {
                                if let Err(error) = groups_write_stdout(
                                    &mut stdout,
                                    &semantic.classic_bytes[range.clone()],
                                    ctcore::ct_stdout_was_closed(),
                                ) {
                                    stdout_error = Some(error);
                                }
                            }
                        }
                        GroupsOutputEvent::Stderr(range) => {
                            groups_write_stderr(
                                &mut stderr,
                                &semantic.stderr_text.as_bytes()[range.clone()],
                            )?;
                        }
                    }
                }

                if let Some(error) = stdout_error {
                    Err(error)
                } else if semantic.exit_code == 0 {
                    Ok(())
                } else {
                    Err(semantic.exit_code.into())
                }
            }
            Err(err) => Err(err),
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
            let err = tool.execute(&args).unwrap_err();
            assert_eq!(err.code(), 0);
        }
    }

    mod tests_groups_main {
        use crate::{
            GroupsOutputEvent, GroupsRawEntry, groups_for_current_process, groups_for_named_user,
            groups_main, groups_native_semantic, groups_quote_username,
            groups_quote_username_with_mark, groups_quote_username_with_marks,
            groups_render_classic_bytes,
        };

        use std::ffi::{OsStr, OsString};

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
        fn test_groups_for_current_process_includes_real_and_effective_gids() {
            assert_eq!(
                groups_for_current_process(65534, 0, vec![0, 10, 20, 65534]),
                vec![65534, 0, 10, 20]
            );
            assert_eq!(
                groups_for_current_process(1000, 1000, vec![1000, 10, 20]),
                vec![1000, 10, 20]
            );
        }

        #[test]
        fn test_groups_for_current_process_uses_gnu_duplicate_reduction() {
            assert_eq!(
                groups_for_current_process(0, 0, vec![1, 2, 1]),
                vec![0, 1, 2, 1]
            );
            assert_eq!(
                groups_for_current_process(0, 0, vec![1, 1, 2, 2]),
                vec![0, 1, 2]
            );
            assert_eq!(groups_for_current_process(0, 0, vec![1, 0, 1]), vec![0, 1]);
        }

        #[test]
        fn test_groups_for_named_user_starts_with_primary_gid() {
            assert_eq!(
                groups_for_named_user(1000, vec![10, 1000, 20]),
                vec![1000, 10, 20]
            );
            assert_eq!(
                groups_for_named_user(1000, vec![10, 20]),
                vec![1000, 10, 20]
            );
            assert_eq!(
                groups_for_named_user(1000, vec![1000, 10, 1000]),
                vec![1000, 10]
            );
        }

        #[test]
        fn test_groups_native_semantic_preserves_partial_output_and_diagnostics() {
            let args = [ctcore::ct_util_name(), "root", "no_such_user_123"];
            let result = groups_native_semantic(args.iter().map(OsString::from)).expect("semantic");

            assert_eq!(result.exit_code, 1);
            assert_eq!(
                result.stderr_text,
                format!(
                    "groups: {}: no such user\n",
                    groups_quote_username(OsStr::new("no_such_user_123"))
                )
            );
            assert_eq!(result.entries.len(), 1);
            assert_eq!(result.entries[0].user.as_deref(), Some("root"));
            assert!(!result.entries[0].groups.is_empty());
            assert!(result.entries[0].groups.iter().any(|group| group == "root"));
            assert_eq!(result.classic_text, format!("{}\n", result.entries[0]));
        }

        #[test]
        fn test_groups_output_events_follow_argument_order() {
            let args = [
                ctcore::ct_util_name(),
                "groups_compat_missing_one",
                "root",
                "groups_compat_missing_two",
            ];
            let result = groups_native_semantic(args.iter().map(OsString::from)).expect("semantic");

            assert_eq!(result.output_events.len(), 3);
            assert!(matches!(
                result.output_events[0],
                GroupsOutputEvent::Stderr(_)
            ));
            assert!(matches!(
                result.output_events[1],
                GroupsOutputEvent::Stdout(_)
            ));
            assert!(matches!(
                result.output_events[2],
                GroupsOutputEvent::Stderr(_)
            ));
        }

        #[test]
        fn test_groups_numeric_argument_is_username_not_uid() {
            let root = ctcore::ct_entries::CtPasswd::locate_name("root").expect("root user");
            let numeric_name = root.uid.to_string();
            if ctcore::ct_entries::CtPasswd::locate_name(&numeric_name).is_ok() {
                return;
            }

            let args = [ctcore::ct_util_name(), numeric_name.as_str()];
            let result = groups_native_semantic(args.iter().map(OsString::from)).expect("semantic");

            assert_eq!(result.exit_code, 1);
            assert!(result.entries.is_empty());
            assert_eq!(
                result.stderr_text,
                format!(
                    "groups: {}: no such user\n",
                    groups_quote_username(OsStr::new(&numeric_name))
                )
            );
            assert!(result.classic_text.is_empty());
        }

        #[cfg(unix)]
        #[test]
        fn test_groups_non_utf8_username_uses_raw_argument_bytes() {
            use std::os::unix::ffi::{OsStrExt, OsStringExt};

            let username = b"groups_compat_\xff_missing";
            let args = [
                OsString::from(ctcore::ct_util_name()),
                OsString::from_vec(username.to_vec()),
            ];
            let result = groups_native_semantic(args.into_iter()).expect("semantic");

            assert_eq!(result.exit_code, 1);
            assert!(result.entries.is_empty());
            assert_eq!(
                result.stderr_text,
                format!(
                    "groups: {}: no such user\n",
                    groups_quote_username(OsStr::from_bytes(username))
                )
            );
        }

        #[test]
        fn test_groups_quote_username_uses_gnu_escape_style() {
            assert_eq!(
                groups_quote_username_with_mark(OsStr::new("groups_compat_'_missing"), "'"),
                "'groups_compat_\\'_missing'"
            );
            assert_eq!(
                groups_quote_username_with_mark(OsStr::new("path\\name"), "'"),
                "'path\\\\name'"
            );
            assert_eq!(
                groups_quote_username_with_mark(OsStr::new("groups_compat_missing"), "\""),
                "\"groups_compat_missing\""
            );
            assert_eq!(
                groups_quote_username_with_marks(OsStr::new("groups_compat_'_missing"), "‘", "’"),
                "‘groups_compat_'_missing’"
            );
            assert_eq!(
                groups_quote_username_with_marks(OsStr::new("name’missing"), "‘", "’"),
                "‘name\\’missing’"
            );
        }

        #[cfg(unix)]
        #[test]
        fn test_groups_classic_bytes_preserve_non_utf8_group_name() {
            use std::os::unix::ffi::OsStringExt;

            let entries = [GroupsRawEntry {
                user: None,
                groups: vec![
                    OsString::from_vec(b"grp_\xff".to_vec()),
                    OsString::from("bin"),
                ],
            }];

            assert_eq!(groups_render_classic_bytes(&entries), b"grp_\xff bin\n");
        }

        #[test]
        fn test_groups_closed_stdout_reports_bad_file_descriptor() {
            let error = crate::groups_write_stdout(&mut std::io::sink(), b"root\n", true)
                .expect_err("closed stdout");
            assert_eq!(error.code(), 1);
            assert_eq!(error.to_string(), "write error: Bad file descriptor");
        }
    }

    mod tests_ct_app {
        use crate::{ct_app, groups_native_semantic};

        use clap::error::ErrorKind;
        use ctcore::ct_error::CTError;
        use std::ffi::{OsStr, OsString};

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

        #[test]
        fn test_abbreviated_help_and_version_do_not_request_usage_hint() {
            for option in ["--he", "--ver"] {
                let args = [ctcore::ct_util_name(), option];
                let error = groups_native_semantic(args.into_iter().map(OsString::from))
                    .expect_err("display request");
                assert_eq!(error.code(), 0);
                assert!(!error.usage());
            }
        }

        #[test]
        fn test_invalid_option_uses_gnu_diagnostic() {
            for (option, diagnostic) in [
                ("-x", "invalid option -- 'x'"),
                ("--bad", "unrecognized option '--bad'"),
            ] {
                let error = ct_app()
                    .try_get_matches_from([ctcore::ct_util_name(), option])
                    .expect_err("invalid option");
                let error = crate::GroupsClapError::new(error);
                assert_eq!(
                    error.to_string(),
                    format!(
                        "{diagnostic}\nTry '{} --help' for more information.",
                        ctcore::ct_help_utility_name()
                    )
                );
                assert_eq!(error.code(), 1);
                assert!(!error.usage());
            }
        }

        #[test]
        fn test_help_and_version_arguments_use_gnu_diagnostic() {
            for option in ["--help=x", "--version=x"] {
                let error = ct_app()
                    .try_get_matches_from([ctcore::ct_util_name(), option])
                    .expect_err("option argument");
                let error = crate::GroupsClapError::new(error);
                let option_name = option.split('=').next().unwrap();
                assert_eq!(
                    error.to_string(),
                    format!(
                        "option '{option_name}' doesn't allow an argument\n\
                         Try '{} --help' for more information.",
                        ctcore::ct_help_utility_name()
                    )
                );
                assert_eq!(error.code(), 1);
                assert!(!error.usage());
            }
        }

        #[test]
        fn test_help_and_version_diagnose_closed_stdout() {
            for option in ["--help", "--version"] {
                let error = ct_app()
                    .try_get_matches_from([ctcore::ct_util_name(), option])
                    .expect_err("display request");
                let error = crate::GroupsClapError::new_with_args(error, &[], true);
                assert_eq!(error.to_string(), "write error: Bad file descriptor");
                assert_eq!(error.code(), 1);
                assert!(!error.usage());
            }
        }

        #[cfg(unix)]
        #[test]
        fn test_invalid_option_diagnostic_preserves_non_utf8_bytes() {
            use std::os::unix::ffi::OsStrExt;

            assert_eq!(
                crate::groups_invalid_option_diagnostic(OsStr::from_bytes(b"-\xff")),
                Some(b"invalid option -- '\xff'".to_vec())
            );
            assert_eq!(
                crate::groups_invalid_option_diagnostic(OsStr::from_bytes(b"--bad\xff")),
                Some(b"unrecognized option '--bad\xff'".to_vec())
            );
        }
    }
}
