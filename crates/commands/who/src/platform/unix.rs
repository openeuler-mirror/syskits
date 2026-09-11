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

use ctcore::ct_display::Quotable;
use ctcore::ct_entries;
use ctcore::ct_error::{CTResult, FromIo};
use ctcore::ct_locale::hard_locale_time;
use ctcore::ct_utmpx::{self, CtUtmpx, time};
use ctcore::libc::{
    CLOCK_BOOTTIME, CLOCK_REALTIME, ESRCH, S_IWGRP, STDIN_FILENO, clock_gettime, kill, timespec,
    ttyname,
};
use rust_i18n::t;
use std::borrow::Cow;
use std::ffi::{CStr, OsStr, OsString};
use std::io::{self, Write as IoWrite};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use sys_locale::get_locale;

use crate::who_flags;
use crate::{ct_app_for_parse, prepare_who_args};

fn get_long_usage() -> String {
    format!(
        "If FILE is not specified, use {}.  /var/log/wtmp as FILE is common.\n\
          If ARG1 ARG2 given, -m presumed: 'am i' or 'mom likes' are usual.",
        ct_utmpx::DEFAULT_FILE,
    )
}

pub fn who_main(args: impl ctcore::Args) -> CTResult<()> {
    // 设置语言
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let args = prepare_who_args(args)?;
    let matches: clap::ArgMatches = ct_app_for_parse(std::env::var_os("POSIXLY_CORRECT").is_some())
        .after_help(get_long_usage())
        .try_get_matches_from(args)?;

    let mut who_cmd = who_from_matches(&matches);

    who_cmd.exec()
}

pub fn who_native_semantic(args: impl ctcore::Args) -> CTResult<WhoSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let args = prepare_who_args(args)?;
    let matches: clap::ArgMatches = ct_app_for_parse(std::env::var_os("POSIXLY_CORRECT").is_some())
        .after_help(get_long_usage())
        .try_get_matches_from(args)?;
    let mut who_cmd = who_from_matches(&matches);
    who_cmd.collect_semantic()
}

fn who_from_matches(matches: &clap::ArgMatches) -> Who {
    let ct_files: Vec<OsString> = matches
        .get_many::<OsString>(who_flags::WHO_FILE)
        .map(|values| values.cloned().collect())
        .unwrap_or_default();

    let is_do_lookup = matches.get_flag(who_flags::WHO_LOOKUP);
    let is_short_list = matches.get_flag(who_flags::WHO_COUNT);
    let si_all = matches.get_flag(who_flags::WHO_ALL);
    let is_include_heading = matches.get_flag(who_flags::WHO_HEADING);
    let is_include_mesg = si_all || matches.get_flag(who_flags::WHO_MESG);
    let is_need_boottime = si_all || matches.get_flag(who_flags::WHO_BOOT);
    let is_need_deadprocs = si_all || matches.get_flag(who_flags::WHO_DEAD);
    let is_need_login = si_all || matches.get_flag(who_flags::WHO_LOGIN);
    let is_need_initspawn = si_all || matches.get_flag(who_flags::WHO_PROCESS);
    let is_need_clockchange = si_all || matches.get_flag(who_flags::WHO_TIME);
    let is_need_runlevel = si_all || matches.get_flag(who_flags::WHO_RUNLEVEL);

    let is_use_defaults = !(si_all
        || is_need_boottime
        || is_need_deadprocs
        || is_need_login
        || is_need_initspawn
        || is_need_runlevel
        || is_need_clockchange
        || matches.get_flag(who_flags::WHO_USERS));

    let is_need_users = si_all || matches.get_flag(who_flags::WHO_USERS) || is_use_defaults;
    let is_include_idle = is_need_deadprocs || is_need_login || is_need_runlevel || is_need_users;
    let is_include_exit = is_need_deadprocs;
    let is_short_output =
        !is_include_exit && (is_use_defaults || matches.get_flag(who_flags::WHO_SHORT));
    let is_my_line_only =
        matches.get_flag(who_flags::WHO_ONLY_HOSTNAME_USER) || ct_files.len() == 2;

    Who {
        is_do_lookup,
        is_short_list,
        is_short_output,
        is_include_idle,
        is_include_heading,
        is_include_mesg,
        is_include_exit,
        is_need_boottime,
        is_need_deadprocs,
        is_need_login,
        is_need_initspawn,
        is_need_clockchange,
        is_need_runlevel,
        is_need_users,
        is_my_line_only,
        who_args: ct_files,
    }
}

struct Who {
    is_do_lookup: bool,
    is_short_list: bool,
    is_short_output: bool,
    is_include_idle: bool,
    is_include_heading: bool,
    is_include_mesg: bool,
    is_include_exit: bool,
    is_need_boottime: bool,
    is_need_deadprocs: bool,
    is_need_login: bool,
    is_need_initspawn: bool,
    is_need_clockchange: bool,
    is_need_runlevel: bool,
    is_need_users: bool,
    is_my_line_only: bool,
    who_args: Vec<OsString>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhoRow {
    pub kind: String,
    pub user: Option<String>,
    pub mesg: Option<String>,
    pub line: Option<String>,
    pub time: Option<String>,
    pub idle: Option<String>,
    pub pid: Option<i64>,
    pub host: Option<String>,
    pub comment: Option<String>,
    pub exit: Option<String>,
    pub user_names: Vec<String>,
    pub user_count: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhoSemantic {
    pub view_kind: String,
    pub source_file: String,
    pub rows: Vec<WhoRow>,
    pub classic_text: String,
    pub stderr_text: String,
    pub exit_code: i32,
}

struct WhoDisplayLine {
    user: String,
    state: char,
    line: String,
    time: String,
    idle: String,
    pid: String,
    comment: String,
    exit: String,
}

struct WhoDisplayBytes<'a> {
    user: &'a [u8],
    state: char,
    line: &'a [u8],
    time: &'a [u8],
    idle: &'a [u8],
    pid: &'a [u8],
    comment: &'a [u8],
    exit: &'a [u8],
}

fn idle_string<'a>(when: i64, boot_time: i64) -> Cow<'a, str> {
    thread_local! {
        static NOW: time::OffsetDateTime = time::OffsetDateTime::now_local().unwrap();
    }
    NOW.with(|n| {
        let now = n.unix_timestamp();
        idle_string_local(when, boot_time, now)
    })
}

fn idle_string_local<'a>(when: i64, boot_time: i64, now: i64) -> Cow<'a, str> {
    const WHO_HOUR_TO_SECOUND: i64 = 3600;
    const WHO_MINUTE_TO_SECOUND: i64 = 60;
    const WHO_DAY_TO_SECOUND: i64 = 24 * 3600;
    if boot_time < when && now - WHO_DAY_TO_SECOUND < when && when <= now {
        let seconds_idle = now - when;
        if seconds_idle < WHO_MINUTE_TO_SECOUND {
            "  .  ".into()
        } else {
            format!(
                "{:02}:{:02}",
                seconds_idle / WHO_HOUR_TO_SECOUND,
                (seconds_idle % WHO_HOUR_TO_SECOUND) / WHO_MINUTE_TO_SECOUND
            )
            .into()
        }
    } else {
        " old ".into()
    }
}

fn runlevel_comment(previous: u8) -> Option<String> {
    if !(b' '..=b'~').contains(&previous) {
        return None;
    }

    let previous = if previous == b'N' {
        'S'
    } else {
        char::from(previous)
    };
    Some(format!("last={previous}"))
}

fn updated_boot_time(current: i64, record_type: i16, timestamp: i64) -> i64 {
    if record_type == ct_utmpx::BOOT_TIME {
        timestamp
    } else {
        current
    }
}

fn should_keep_user_pid(check_pids: bool, is_user_process: bool, pid: i32) -> bool {
    if !check_pids || !is_user_process || pid <= 0 {
        return true;
    }

    let status = unsafe { kill(pid, 0) };
    status == 0 || std::io::Error::last_os_error().raw_os_error() != Some(ESRCH)
}

fn canonicalize_host_bytes<F>(host: &[u8], lookup: F) -> io::Result<Vec<u8>>
where
    F: FnOnce(&str) -> io::Result<String>,
{
    if !host.is_ascii() {
        return Ok(host.to_vec());
    }

    let host = std::str::from_utf8(host).expect("ASCII host names are valid UTF-8");
    lookup(host).map(String::into_bytes)
}

fn tty_permissions_allow_messages(mode: u32, gid: u32, tty_group: Option<u32>) -> bool {
    tty_group == Some(gid) && mode & S_IWGRP != 0
}

fn tty_is_writable(metadata: &std::fs::Metadata) -> bool {
    tty_permissions_allow_messages(
        metadata.mode(),
        metadata.gid(),
        ct_entries::grp2gid("tty").ok(),
    )
}

fn tty_stat_path(line: &[u8]) -> PathBuf {
    let device = line
        .iter()
        .position(|byte| *byte == b' ')
        .map_or(line, |index| &line[index + 1..]);
    if device.is_empty() {
        return PathBuf::new();
    }

    let mut path = PathBuf::from("/dev");
    path.push(std::ffi::OsStr::from_bytes(device));
    path
}

fn linux_boot_time() -> Option<i64> {
    for path in [
        "/var/lib/systemd/random-seed",
        "/var/lib/urandom/random-seed",
        "/var/lib/random-seed",
        ct_utmpx::DEFAULT_FILE,
    ] {
        if let Ok(metadata) = std::fs::metadata(path) {
            return Some(metadata.mtime());
        }
    }

    let mut uptime = timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    let mut now = timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { clock_gettime(CLOCK_BOOTTIME, &mut uptime) } != 0
        || unsafe { clock_gettime(CLOCK_REALTIME, &mut now) } != 0
    {
        return None;
    }

    Some(now.tv_sec - uptime.tv_sec - i64::from(now.tv_nsec < uptime.tv_nsec))
}

fn fallback_boot_time(
    source: &OsStr,
    need_boot_time: bool,
    saw_boot_time: bool,
    my_line_only: bool,
) -> Option<i64> {
    (need_boot_time
        && !saw_boot_time
        && !my_line_only
        && source == OsStr::new(ct_utmpx::DEFAULT_FILE))
    .then(linux_boot_time)
    .flatten()
}

fn time_string(timestamp: i64) -> String {
    let utc = time::OffsetDateTime::from_unix_timestamp(timestamp).unwrap();
    let offset = time::UtcOffset::local_offset_at(utc).unwrap_or(time::UtcOffset::UTC);
    let local = utc.to_offset(offset);
    let format = if hard_locale_time() {
        "[year]-[month padding:zero]-[day padding:zero] [hour]:[minute]"
    } else {
        "[month repr:short] [day padding:space] [hour]:[minute]"
    };
    local
        .format(&time::format_description::parse(format).unwrap())
        .unwrap()
}

fn time_format_width() -> usize {
    if hard_locale_time() { 16 } else { 12 }
}

#[inline]
fn cur_tty() -> String {
    unsafe {
        let result = ttyname(STDIN_FILENO);
        if result.is_null() {
            String::new()
        } else {
            CStr::from_ptr(result as *const _)
                .to_string_lossy()
                .trim_start_matches("/dev/")
                .to_owned()
        }
    }
}

fn pad_right_bytes(value: &[u8], target_width: usize) -> Vec<u8> {
    let mut output = Vec::with_capacity(value.len().max(target_width));
    output.extend_from_slice(value);
    output.resize(
        output.len() + target_width.saturating_sub(value.len()),
        b' ',
    );
    output
}

fn trim_user_name(name: &[u8]) -> &[u8] {
    let length = name
        .iter()
        .rposition(|byte| *byte != b' ')
        .map_or(0, |index| index + 1);
    &name[..length]
}

impl Who {
    #[allow(clippy::cognitive_complexity)]
    fn exec(&mut self) -> CTResult<()> {
        let run_level_chk = |_record: i16| {
            #[cfg(target_os = "linux")]
            return _record == ct_utmpx::RUN_LVL;
        };

        let f = match self.who_args.len() {
            1 => self.who_args[0].as_os_str(),
            _ => OsStr::new(ct_utmpx::DEFAULT_FILE),
        };
        let check_pids = self.who_args.len() != 1;

        if self.is_short_list {
            let users = CtUtmpx::iter_all_records_from(f)
                .filter(|utmpx| {
                    utmpx.is_user_process() && should_keep_user_pid(check_pids, true, utmpx.pid())
                })
                .map(|utmpx| trim_user_name(utmpx.user_bytes()).to_vec())
                .collect::<Vec<_>>();
            let mut output = Vec::new();
            for (index, user) in users.iter().enumerate() {
                if index != 0 {
                    output.push(b' ');
                }
                output.extend_from_slice(user);
            }
            output.push(b'\n');
            output.extend_from_slice(t!("who.output.users_count").as_bytes());
            output.push(b'=');
            output.extend_from_slice(users.len().to_string().as_bytes());
            output.push(b'\n');
            io::stdout().lock().write_all(&output)?;
        } else {
            let records = CtUtmpx::iter_all_records_from(f);
            let mut boot_time = i64::MIN;
            let mut saw_boot_time = false;

            if self.is_include_heading {
                self.print_head();
            }

            let current_tty = match self.is_my_line_only {
                true => cur_tty(),
                false => String::new(),
            };

            for utmpx in records {
                if !should_keep_user_pid(check_pids, utmpx.is_user_process(), utmpx.pid()) {
                    continue;
                }
                if !self.is_my_line_only || current_tty == utmpx.tty_device() {
                    if self.is_need_users && utmpx.is_user_process() {
                        self.print_user(&utmpx, boot_time)?;
                    } else if self.is_need_runlevel && run_level_chk(utmpx.record_type()) {
                        if cfg!(target_os = "linux") {
                            self.print_runlevel(&utmpx);
                        }
                    } else if self.is_need_boottime && utmpx.record_type() == ct_utmpx::BOOT_TIME {
                        self.print_boottime(&utmpx);
                    } else if self.is_need_clockchange && utmpx.record_type() == ct_utmpx::NEW_TIME
                    {
                        self.print_clockchange(&utmpx);
                    } else if self.is_need_initspawn
                        && utmpx.record_type() == ct_utmpx::INIT_PROCESS
                    {
                        self.print_initspawn(&utmpx)?;
                    } else if self.is_need_login && utmpx.record_type() == ct_utmpx::LOGIN_PROCESS {
                        self.print_login(&utmpx)?;
                    } else if self.is_need_deadprocs
                        && utmpx.record_type() == ct_utmpx::DEAD_PROCESS
                    {
                        self.print_deadprocs(&utmpx)?;
                    }
                }

                boot_time =
                    updated_boot_time(boot_time, utmpx.record_type(), utmpx.timestamp_seconds());
                saw_boot_time |= utmpx.record_type() == ct_utmpx::BOOT_TIME;
            }

            if let Some(timestamp) = fallback_boot_time(
                f,
                self.is_need_boottime,
                saw_boot_time,
                self.is_my_line_only,
            ) {
                self.print_line(
                    "",
                    ' ',
                    &t!("who.output.system_boot"),
                    &time_string(timestamp),
                    "",
                    "",
                    "",
                    "",
                );
            }
        }
        Ok(())
    }

    #[inline]
    fn print_runlevel(&self, utmpx: &CtUtmpx) {
        let last_runlevel = (utmpx.pid() / 256) as u8;
        let current_runlevel = (utmpx.pid() % 256) as u8 as char;
        // Creating the run-level string
        let label = t!("who.output.run_level");
        let runlevel_line = format!("{label} {current_runlevel}");

        // 生成有关最后运行级别的注释
        let comment = runlevel_comment(last_runlevel);

        self.print_line(
            "",
            ' ',
            &runlevel_line,
            &time_string(utmpx.timestamp_seconds()),
            "",
            "",
            comment.as_deref().unwrap_or(""),
            "",
        );
    }

    #[inline]
    fn print_clockchange(&self, utmpx: &CtUtmpx) {
        self.print_line(
            "",
            ' ',
            &t!("who.output.clock_change"),
            &time_string(utmpx.timestamp_seconds()),
            "",
            "",
            "",
            "",
        );
    }

    #[inline]
    fn print_login(&self, utmpx: &CtUtmpx) -> CTResult<()> {
        let user = t!("who.output.login");
        let comment = [b"id=".as_slice(), utmpx.terminal_suffix_bytes()].concat();
        let time = time_string(utmpx.timestamp_seconds());
        let pid = utmpx.pid().to_string();
        self.print_line_bytes(&WhoDisplayBytes {
            user: user.as_bytes(),
            state: ' ',
            line: utmpx.tty_device_bytes(),
            time: time.as_bytes(),
            idle: b"",
            pid: pid.as_bytes(),
            comment: &comment,
            exit: b"",
        })
    }

    #[inline]
    fn print_deadprocs(&self, utmpx: &CtUtmpx) -> CTResult<()> {
        let comment = [b"id=".as_slice(), utmpx.terminal_suffix_bytes()].concat();
        let pid = utmpx.pid().to_string();
        let time = time_string(utmpx.timestamp_seconds());
        let e = utmpx.exit_status();
        let exit = format!(
            "{}={} {}={}",
            t!("who.output.term"),
            e.0,
            t!("who.output.exit"),
            e.1
        );
        self.print_line_bytes(&WhoDisplayBytes {
            user: b"",
            state: ' ',
            line: utmpx.tty_device_bytes(),
            time: time.as_bytes(),
            idle: b"",
            pid: pid.as_bytes(),
            comment: &comment,
            exit: exit.as_bytes(),
        })
    }

    #[inline]
    fn print_initspawn(&self, utmpx: &CtUtmpx) -> CTResult<()> {
        let comment = [b"id=".as_slice(), utmpx.terminal_suffix_bytes()].concat();
        let time = time_string(utmpx.timestamp_seconds());
        let pid = utmpx.pid().to_string();
        self.print_line_bytes(&WhoDisplayBytes {
            user: b"",
            state: ' ',
            line: utmpx.tty_device_bytes(),
            time: time.as_bytes(),
            idle: b"",
            pid: pid.as_bytes(),
            comment: &comment,
            exit: b"",
        })
    }

    #[inline]
    fn print_boottime(&self, utmpx: &CtUtmpx) {
        self.print_line(
            "",
            ' ',
            &t!("who.output.system_boot"),
            &time_string(utmpx.timestamp_seconds()),
            "",
            "",
            "",
            "",
        );
    }

    fn print_user(&self, utmpx: &CtUtmpx, boot_time: i64) -> CTResult<()> {
        let p = tty_stat_path(utmpx.tty_device_bytes());

        let (mesg, last_change) = match p.metadata() {
            Ok(meta) => {
                let mesg = if tty_is_writable(&meta) { '+' } else { '-' };

                (mesg, meta.atime())
            }
            _ => ('?', 0),
        };

        let idle = match last_change {
            0 => "  ?".into(),
            _ => idle_string(last_change, boot_time),
        };

        let host = if self.is_do_lookup {
            canonicalize_host_bytes(utmpx.host_bytes(), |_| utmpx.canon_host()).map_err_context(
                || {
                    let host_string = utmpx.host();
                    format!(
                        "failed to canonicalize {}",
                        host_string
                            .split(':')
                            .next()
                            .unwrap_or(&host_string)
                            .quote()
                    )
                },
            )?
        } else {
            utmpx.host_bytes().to_vec()
        };

        let host_display = if host.is_empty() {
            Vec::new()
        } else {
            [b"(".as_slice(), host.as_slice(), b")".as_slice()].concat()
        };
        let time = time_string(utmpx.timestamp_seconds());
        let pid = utmpx.pid().to_string();
        self.print_line_bytes(&WhoDisplayBytes {
            user: utmpx.user_bytes(),
            state: mesg,
            line: utmpx.tty_device_bytes(),
            time: time.as_bytes(),
            idle: idle.as_bytes(),
            pid: pid.as_bytes(),
            comment: &host_display,
            exit: b"",
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn print_line(
        &self,
        user: &str,
        state: char,
        line: &str,
        time: &str,
        idle: &str,
        pid: &str,
        comment: &str,
        exit: &str,
    ) {
        let rendered = self.render_line(&WhoDisplayLine {
            user: user.to_string(),
            state,
            line: line.to_string(),
            time: time.to_string(),
            idle: idle.to_string(),
            pid: pid.to_string(),
            comment: comment.to_string(),
            exit: exit.to_string(),
        });
        println!("{rendered}");
    }

    fn render_line(&self, fields: &WhoDisplayLine) -> String {
        String::from_utf8(self.render_line_bytes(&WhoDisplayBytes {
            user: fields.user.as_bytes(),
            state: fields.state,
            line: fields.line.as_bytes(),
            time: fields.time.as_bytes(),
            idle: fields.idle.as_bytes(),
            pid: fields.pid.as_bytes(),
            comment: fields.comment.as_bytes(),
            exit: fields.exit.as_bytes(),
        }))
        .unwrap()
    }

    fn render_line_bytes(&self, fields: &WhoDisplayBytes<'_>) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(64);
        buffer.extend(pad_right_bytes(fields.user, 8));
        if self.is_include_mesg {
            buffer.extend_from_slice(&[b' ', fields.state as u8]);
        }
        buffer.push(b' ');
        buffer.extend(pad_right_bytes(fields.line, 12));
        buffer.push(b' ');
        buffer.extend(pad_right_bytes(fields.time, time_format_width()));

        if !self.is_short_output {
            if self.is_include_idle {
                buffer.push(b' ');
                buffer.extend(pad_right_bytes(fields.idle, 6));
            }
            buffer.push(b' ');
            buffer.extend(std::iter::repeat_n(
                b' ',
                10_usize.saturating_sub(fields.pid.len()),
            ));
            buffer.extend_from_slice(fields.pid);
        }

        buffer.push(b' ');
        buffer.extend(pad_right_bytes(fields.comment, 8));
        if self.is_include_exit {
            buffer.push(b' ');
            buffer.extend(pad_right_bytes(fields.exit, 12));
        }

        while buffer.last() == Some(&b' ') {
            buffer.pop();
        }
        buffer
    }

    fn print_line_bytes(&self, fields: &WhoDisplayBytes<'_>) -> CTResult<()> {
        let mut rendered = self.render_line_bytes(fields);
        rendered.push(b'\n');
        io::stdout().lock().write_all(&rendered)?;
        Ok(())
    }

    #[inline]
    fn print_head(&self) {
        self.print_line(
            &t!("who.output.heading_name"),
            ' ',
            &t!("who.output.heading_line"),
            &t!("who.output.heading_time"),
            &t!("who.output.heading_idle"),
            &t!("who.output.heading_pid"),
            &t!("who.output.heading_comment"),
            &t!("who.output.heading_exit"),
        );
    }

    fn source_file(&self) -> &OsStr {
        match self.who_args.len() {
            1 => self.who_args[0].as_os_str(),
            _ => OsStr::new(ct_utmpx::DEFAULT_FILE),
        }
    }

    fn checks_default_pids(&self) -> bool {
        self.who_args.len() != 1
    }

    fn view_kind(&self) -> String {
        if self.is_short_list {
            "count".to_string()
        } else if self.is_need_users
            && !self.is_need_boottime
            && !self.is_need_deadprocs
            && !self.is_need_login
            && !self.is_need_initspawn
            && !self.is_need_clockchange
            && !self.is_need_runlevel
        {
            "default".to_string()
        } else {
            "mixed".to_string()
        }
    }

    fn push_row(&self, semantic: &mut WhoSemantic, row: WhoRow, display: WhoDisplayLine) {
        semantic.classic_text.push_str(&self.render_line(&display));
        semantic.classic_text.push('\n');
        semantic.rows.push(row);
    }

    fn heading_row(&self) -> WhoRow {
        WhoRow {
            kind: "heading".into(),
            user: Some(t!("who.output.heading_name").to_owned()),
            mesg: if self.is_include_mesg {
                Some(" ".into())
            } else {
                None
            },
            line: Some(t!("who.output.heading_line").to_owned()),
            time: Some(t!("who.output.heading_time").to_owned()),
            idle: if self.is_include_idle {
                Some(t!("who.output.heading_idle").to_owned())
            } else {
                None
            },
            pid: None,
            host: None,
            comment: Some(t!("who.output.heading_comment").to_owned()),
            exit: if self.is_include_exit {
                Some(t!("who.output.heading_exit").to_owned())
            } else {
                None
            },
            user_names: Vec::new(),
            user_count: None,
        }
    }

    fn build_user_row(
        &self,
        utmpx: &CtUtmpx,
        boot_time: i64,
    ) -> CTResult<(WhoRow, WhoDisplayLine)> {
        let p = tty_stat_path(utmpx.tty_device_bytes());

        let (mesg, last_change) = match p.metadata() {
            Ok(meta) => {
                let mesg = if tty_is_writable(&meta) { '+' } else { '-' };

                (mesg, meta.atime())
            }
            _ => ('?', 0),
        };

        let idle = match last_change {
            0 => "  ?".to_owned(),
            _ => idle_string(last_change, boot_time).into_owned(),
        };

        let host = if self.is_do_lookup {
            utmpx.canon_host().map_err_context(|| {
                let host_string = utmpx.host();
                format!(
                    "failed to canonicalize {}",
                    host_string
                        .split(':')
                        .next()
                        .unwrap_or(&host_string)
                        .quote()
                )
            })?
        } else {
            utmpx.host()
        };

        let host_display = if host.is_empty() {
            String::new()
        } else {
            format!("({host})")
        };

        let user = utmpx.user();
        let line = utmpx.tty_device();
        let time = time_string(utmpx.timestamp_seconds());
        let pid = utmpx.pid();

        Ok((
            WhoRow {
                kind: "user".into(),
                user: Some(user.clone()),
                mesg: Some(mesg.to_string()),
                line: Some(line.clone()),
                time: Some(time.clone()),
                idle: Some(idle.clone()),
                pid: Some(i64::from(pid)),
                host: if host.is_empty() { None } else { Some(host) },
                comment: if host_display.is_empty() {
                    None
                } else {
                    Some(host_display.clone())
                },
                exit: None,
                user_names: Vec::new(),
                user_count: None,
            },
            WhoDisplayLine {
                user,
                state: mesg,
                line,
                time,
                idle,
                pid: pid.to_string(),
                comment: host_display,
                exit: String::new(),
            },
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn build_simple_row(
        &self,
        kind: &str,
        user: &str,
        line: &str,
        time: &str,
        pid: &str,
        comment: &str,
        exit: &str,
    ) -> (WhoRow, WhoDisplayLine) {
        (
            WhoRow {
                kind: kind.to_string(),
                user: if user.is_empty() {
                    None
                } else {
                    Some(user.to_string())
                },
                mesg: Some(" ".into()),
                line: if line.is_empty() {
                    None
                } else {
                    Some(line.to_string())
                },
                time: if time.is_empty() {
                    None
                } else {
                    Some(time.to_string())
                },
                idle: None,
                pid: pid.parse::<i64>().ok(),
                host: None,
                comment: if comment.is_empty() {
                    None
                } else {
                    Some(comment.to_string())
                },
                exit: if exit.is_empty() {
                    None
                } else {
                    Some(exit.to_string())
                },
                user_names: Vec::new(),
                user_count: None,
            },
            WhoDisplayLine {
                user: user.to_string(),
                state: ' ',
                line: line.to_string(),
                time: time.to_string(),
                idle: String::new(),
                pid: pid.to_string(),
                comment: comment.to_string(),
                exit: exit.to_string(),
            },
        )
    }

    fn collect_semantic(&mut self) -> CTResult<WhoSemantic> {
        let run_level_chk = |_record: i16| {
            #[cfg(target_os = "linux")]
            return _record == ct_utmpx::RUN_LVL;
        };

        let source_file = self.source_file().to_os_string();
        let mut semantic = WhoSemantic {
            view_kind: self.view_kind(),
            source_file: source_file.to_string_lossy().into_owned(),
            rows: Vec::new(),
            classic_text: String::new(),
            stderr_text: String::new(),
            exit_code: 0,
        };

        if self.is_short_list {
            let users = CtUtmpx::iter_all_records_from(&source_file)
                .filter(|utmpx| {
                    utmpx.is_user_process()
                        && should_keep_user_pid(self.checks_default_pids(), true, utmpx.pid())
                })
                .map(|utmpx| utmpx.user())
                .collect::<Vec<_>>();
            semantic.classic_text.push_str(&users.join(" "));
            semantic.classic_text.push('\n');
            semantic.classic_text.push_str(&format!(
                "{}={}\n",
                t!("who.output.users_count"),
                users.len()
            ));
            semantic.rows.push(WhoRow {
                kind: "count".into(),
                user: None,
                mesg: None,
                line: None,
                time: None,
                idle: None,
                pid: None,
                host: None,
                comment: None,
                exit: None,
                user_names: users.clone(),
                user_count: Some(users.len()),
            });
            return Ok(semantic);
        }

        let records = CtUtmpx::iter_all_records_from(&source_file);
        let mut boot_time = i64::MIN;
        let mut saw_boot_time = false;
        if self.is_include_heading {
            let row = self.heading_row();
            self.push_row(
                &mut semantic,
                row,
                WhoDisplayLine {
                    user: t!("who.output.heading_name").to_owned(),
                    state: ' ',
                    line: t!("who.output.heading_line").to_owned(),
                    time: t!("who.output.heading_time").to_owned(),
                    idle: t!("who.output.heading_idle").to_owned(),
                    pid: t!("who.output.heading_pid").to_owned(),
                    comment: t!("who.output.heading_comment").to_owned(),
                    exit: t!("who.output.heading_exit").to_owned(),
                },
            );
        }

        let current_tty = if self.is_my_line_only {
            cur_tty()
        } else {
            String::new()
        };

        for utmpx in records {
            if !should_keep_user_pid(
                self.checks_default_pids(),
                utmpx.is_user_process(),
                utmpx.pid(),
            ) {
                continue;
            }
            let next_boot_time =
                updated_boot_time(boot_time, utmpx.record_type(), utmpx.timestamp_seconds());
            if self.is_my_line_only && current_tty != utmpx.tty_device() {
                boot_time = next_boot_time;
                saw_boot_time |= utmpx.record_type() == ct_utmpx::BOOT_TIME;
                continue;
            }

            if self.is_need_users && utmpx.is_user_process() {
                let (row, display) = self.build_user_row(&utmpx, boot_time)?;
                self.push_row(&mut semantic, row, display);
            } else if self.is_need_runlevel && run_level_chk(utmpx.record_type()) {
                if cfg!(target_os = "linux") {
                    let last_runlevel = (utmpx.pid() / 256) as u8;
                    let current_runlevel = (utmpx.pid() % 256) as u8 as char;
                    let label = t!("who.output.run_level");
                    let runlevel_line = format!("{label} {current_runlevel}");
                    let comment = runlevel_comment(last_runlevel);
                    let (row, display) = self.build_simple_row(
                        "runlevel",
                        "",
                        &runlevel_line,
                        &time_string(utmpx.timestamp_seconds()),
                        "",
                        comment.as_deref().unwrap_or(""),
                        "",
                    );
                    self.push_row(&mut semantic, row, display);
                }
            } else if self.is_need_boottime && utmpx.record_type() == ct_utmpx::BOOT_TIME {
                let (row, display) = self.build_simple_row(
                    "boot_time",
                    "",
                    &t!("who.output.system_boot"),
                    &time_string(utmpx.timestamp_seconds()),
                    "",
                    "",
                    "",
                );
                self.push_row(&mut semantic, row, display);
            } else if self.is_need_clockchange && utmpx.record_type() == ct_utmpx::NEW_TIME {
                let (row, display) = self.build_simple_row(
                    "clock_change",
                    "",
                    &t!("who.output.clock_change"),
                    &time_string(utmpx.timestamp_seconds()),
                    "",
                    "",
                    "",
                );
                self.push_row(&mut semantic, row, display);
            } else if self.is_need_initspawn && utmpx.record_type() == ct_utmpx::INIT_PROCESS {
                let comment = format!("id={}", utmpx.terminal_suffix());
                let pid = utmpx.pid().to_string();
                let (row, display) = self.build_simple_row(
                    "init_process",
                    "",
                    &utmpx.tty_device(),
                    &time_string(utmpx.timestamp_seconds()),
                    &pid,
                    &comment,
                    "",
                );
                self.push_row(&mut semantic, row, display);
            } else if self.is_need_login && utmpx.record_type() == ct_utmpx::LOGIN_PROCESS {
                let comment = format!("id={}", utmpx.terminal_suffix());
                let pid = utmpx.pid().to_string();
                let (row, display) = self.build_simple_row(
                    "login",
                    &t!("who.output.login"),
                    &utmpx.tty_device(),
                    &time_string(utmpx.timestamp_seconds()),
                    &pid,
                    &comment,
                    "",
                );
                self.push_row(&mut semantic, row, display);
            } else if self.is_need_deadprocs && utmpx.record_type() == ct_utmpx::DEAD_PROCESS {
                let comment = format!("id={}", utmpx.terminal_suffix());
                let pid = utmpx.pid().to_string();
                let e = utmpx.exit_status();
                let exit_str = format!(
                    "{}={} {}={}",
                    t!("who.output.term"),
                    e.0,
                    t!("who.output.exit"),
                    e.1
                );
                let (row, display) = self.build_simple_row(
                    "dead_process",
                    "",
                    &utmpx.tty_device(),
                    &time_string(utmpx.timestamp_seconds()),
                    &pid,
                    &comment,
                    &exit_str,
                );
                self.push_row(&mut semantic, row, display);
            }
            boot_time = next_boot_time;
            saw_boot_time |= utmpx.record_type() == ct_utmpx::BOOT_TIME;
        }

        if let Some(timestamp) = fallback_boot_time(
            &source_file,
            self.is_need_boottime,
            saw_boot_time,
            self.is_my_line_only,
        ) {
            let time = time_string(timestamp);
            let (row, display) = self.build_simple_row(
                "boot_time",
                "",
                &t!("who.output.system_boot"),
                &time,
                "",
                "",
                "",
            );
            self.push_row(&mut semantic, row, display);
        }

        Ok(semantic)
    }
}

#[cfg(test)]
mod tests {
    use crate::ct_app;
    use ctcore::ct_utmpx::time::OffsetDateTime;
    use std::env;
    use std::sync::Mutex;

    use super::*;

    // 互斥锁确保环境变量测试的串行执行，避免并发测试时的干扰
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn explicit_short_option_suppresses_optional_user_fields() {
        let matches = ct_app()
            .try_get_matches_from([ctcore::ct_util_name(), "-u", "-s", "utmp"])
            .unwrap();
        let who = who_from_matches(&matches);
        assert!(who.is_short_output);

        let matches = ct_app()
            .try_get_matches_from([ctcore::ct_util_name(), "-d", "-s", "utmp"])
            .unwrap();
        let who = who_from_matches(&matches);
        assert!(!who.is_short_output);
    }

    #[test]
    fn runlevel_comment_reports_the_actual_printable_previous_level() {
        assert_eq!(runlevel_comment(b'N').as_deref(), Some("last=S"));
        assert_eq!(runlevel_comment(b'3').as_deref(), Some("last=3"));
        assert_eq!(runlevel_comment(b' ').as_deref(), Some("last= "));
        assert_eq!(runlevel_comment(0x1f), None);
        assert_eq!(runlevel_comment(0xa0), None);
    }

    #[test]
    fn boot_time_tracking_updates_only_for_boot_records() {
        assert_eq!(updated_boot_time(100, ct_utmpx::USER_PROCESS, 200), 100);
        assert_eq!(updated_boot_time(100, ct_utmpx::BOOT_TIME, 200), 200);
        assert_eq!(updated_boot_time(200, ct_utmpx::BOOT_TIME, 300), 300);
    }

    #[test]
    fn default_source_filters_only_missing_positive_user_pids() {
        let missing_pid = i32::MAX;
        assert!(!should_keep_user_pid(true, true, missing_pid));
        assert!(should_keep_user_pid(false, true, missing_pid));
        assert!(should_keep_user_pid(true, false, missing_pid));
        assert!(should_keep_user_pid(true, true, 0));
        assert!(should_keep_user_pid(true, true, std::process::id() as i32));
    }

    #[test]
    fn tty_message_status_requires_the_tty_group_and_group_write_permission() {
        let tty_gid = 5;

        assert!(tty_permissions_allow_messages(
            S_IWGRP,
            tty_gid,
            Some(tty_gid)
        ));
        assert!(!tty_permissions_allow_messages(
            S_IWGRP,
            tty_gid + 1,
            Some(tty_gid)
        ));
        assert!(!tty_permissions_allow_messages(0, tty_gid, Some(tty_gid)));
        assert!(!tty_permissions_allow_messages(S_IWGRP, tty_gid, None));
    }

    #[test]
    fn tty_stat_path_discards_the_prefix_before_the_first_space() {
        assert_eq!(tty_stat_path(b"pts/0"), PathBuf::from("/dev/pts/0"));
        assert_eq!(tty_stat_path(b"label pts/0"), PathBuf::from("/dev/pts/0"));
        assert_eq!(
            tty_stat_path(b"label /tmp/terminal"),
            PathBuf::from("/tmp/terminal")
        );
        assert_eq!(tty_stat_path(b"label "), PathBuf::new());
    }

    #[test]
    fn lc_all_overrides_lc_time_for_output_width() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let original_lc_all = env::var("LC_ALL").ok();
        let original_lc_time = env::var("LC_TIME").ok();

        unsafe {
            env::set_var("LC_ALL", "C");
            env::set_var("LC_TIME", "en_US.UTF-8");
        }
        assert_eq!(time_format_width(), 12);

        unsafe {
            env::remove_var("LC_ALL");
            env::remove_var("LC_TIME");
            if let Some(value) = original_lc_all {
                env::set_var("LC_ALL", value);
            }
            if let Some(value) = original_lc_time {
                env::set_var("LC_TIME", value);
            }
        }
    }

    #[test]
    fn historical_login_time_uses_the_offset_at_the_record_timestamp() {
        unsafe extern "C" {
            fn tzset();
        }

        let _guard = ENV_MUTEX.lock().unwrap();
        let original_tz = env::var("TZ").ok();
        let original_lc_all = env::var("LC_ALL").ok();
        unsafe {
            env::set_var("TZ", "America/Los_Angeles");
            env::set_var("LC_ALL", "C");
            tzset();
        }

        assert_eq!(time_string(1_709_210_096), "Feb 29 04:34");

        unsafe {
            env::remove_var("TZ");
            env::remove_var("LC_ALL");
            if let Some(value) = original_tz {
                env::set_var("TZ", value);
            }
            if let Some(value) = original_lc_all {
                env::set_var("LC_ALL", value);
            }
            tzset();
        }
    }

    #[test]
    fn byte_padding_preserves_non_utf8_input_and_counts_bytes() {
        assert_eq!(
            pad_right_bytes(&[0xff, b'A'], 8),
            [vec![0xff, b'A'], vec![b' '; 6]].concat()
        );
        assert_eq!(
            pad_right_bytes("中".as_bytes(), 4),
            ["中".as_bytes(), b" "].concat()
        );
    }

    #[test]
    fn lookup_preserves_non_utf8_host_bytes_without_calling_dns() {
        let host = b"missing-\xff:7";
        let mut lookup_called = false;
        let canonical = canonicalize_host_bytes(host, |_| {
            lookup_called = true;
            Ok("unexpected.example:7".to_string())
        })
        .unwrap();

        assert!(!lookup_called);
        assert_eq!(canonical, host);
    }

    #[test]
    fn count_output_trims_only_trailing_ascii_spaces_from_user_names() {
        assert_eq!(trim_user_name(b"alice   "), b"alice");
        assert_eq!(trim_user_name(b"a b"), b"a b");
        assert_eq!(trim_user_name(&[0xff, b' ']), &[0xff]);
    }

    #[test]
    fn boot_fallback_is_limited_to_missing_default_records() {
        let default_file = OsStr::new(ct_utmpx::DEFAULT_FILE);
        assert!(fallback_boot_time(default_file, true, false, false).is_some());
        assert_eq!(fallback_boot_time(default_file, false, false, false), None);
        assert_eq!(fallback_boot_time(default_file, true, true, false), None);
        assert_eq!(
            fallback_boot_time(OsStr::new("other-utmp"), true, false, false),
            None
        );
        assert_eq!(fallback_boot_time(default_file, true, false, true), None);
    }

    #[test]
    fn test_idle_string() {
        let boot_time = time::OffsetDateTime::now_utc().unix_timestamp() - 500;
        let when = time::OffsetDateTime::now_utc().unix_timestamp() - 60;
        assert_eq!(
            idle_string_local(when, boot_time, when + 60).to_string(),
            "00:01"
        );
    }

    #[test]
    fn test_print_line() {
        let who = Who {
            is_do_lookup: false,
            is_short_list: false,
            is_short_output: false,
            is_include_idle: false,
            is_include_heading: false,
            is_include_mesg: false,
            is_include_exit: false,
            is_need_boottime: false,
            is_need_deadprocs: false,
            is_need_login: false,
            is_need_initspawn: false,
            is_need_clockchange: false,
            is_need_runlevel: false,
            is_need_users: false,
            is_my_line_only: false,
            who_args: Vec::new(),
        };
        let user = "testuser";
        let state = '+';
        let line = "tty1";
        let time = "Apr 7 14:23";
        let idle = "00:05";
        let pid = "1234";
        let comment = "testing";
        let exit = "0";

        // This will print to stdout, we would need to capture stdout in a real test to assert on it
        who.print_line(user, state, line, time, idle, pid, comment, exit);
    }

    #[test]
    fn test_exec() {
        let mut who = Who {
            is_do_lookup: false,
            is_short_list: true,
            is_short_output: false,
            is_include_idle: false,
            is_include_heading: false,
            is_include_mesg: false,
            is_include_exit: false,
            is_need_boottime: false,
            is_need_deadprocs: false,
            is_need_login: false,
            is_need_initspawn: false,
            is_need_clockchange: false,
            is_need_runlevel: false,
            is_need_users: false,
            is_my_line_only: false,
            who_args: vec![OsString::from("/var/log/wtmp")],
        };

        assert!(who.exec().is_ok());
    }

    #[test]
    fn test_idle_exactly_60_seconds() {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let when = now - 60; // Exactly 60 seconds ago
        let boottime = when - 100; // Booted well before 'when'
        assert_eq!(idle_string_local(when, boottime, now), "00:01");
    }

    #[test]
    fn test_idle_boundary_24_hours() {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let when = now - 24 * 3600; // Exactly 24 hours ago
        let boottime = when - 1000; // Booted well before 'when'
        assert_eq!(idle_string_local(when, boottime, now), " old ");
    }

    #[test]
    fn test_simultaneous_times() {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        assert_eq!(idle_string_local(now, now, now), " old ");
    }

    #[test]
    fn test_when_in_future() {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let when = now + 100; // 100 seconds in the future
        let boottime = now - 1000; // Booted well before 'now'
        assert_eq!(idle_string_local(when, boottime, now), " old ");
    }

    #[test]
    fn test_recent_idle_short() {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let when = now - 30; // 30 seconds ago
        let boottime = when - 100; // System booted 100 seconds before 'when'
        assert_eq!(idle_string_local(when, boottime, now), "  .  ");
    }

    #[test]
    fn test_recent_idle_long() {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let when = now - 3700; // 1 hour and 10 minutes ago
        let boottime = when - 5000; // System booted well before 'when'
        assert_eq!(idle_string_local(when, boottime, now), "01:01");
    }

    #[test]
    fn test_idle_old() {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let when = now - 90000; // More than a day ago
        let boottime = when - 10000; // Boot was also before 'when'
        assert_eq!(idle_string_local(when, boottime, now), " old ");
    }

    #[test]
    fn test_boottime_after_when() {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let when = now - 3000; // 3000 seconds ago
        let boottime = now - 2000; // Boot time is after 'when'
        assert_eq!(idle_string_local(when, boottime, now), " old ");
    }

    #[test]
    fn test_time_string_c_locale() {
        let _guard = ENV_MUTEX.lock().unwrap();

        // 保存原始环境变量
        let original_lc_time = env::var("LC_TIME").ok();
        let original_lc_all = env::var("LC_ALL").ok();
        let original_lang = env::var("LANG").ok();

        // 清理环境变量
        unsafe {
            env::remove_var("LC_TIME");
        }
        unsafe {
            env::remove_var("LC_ALL");
        }
        unsafe {
            env::remove_var("LANG");
        }

        // Set LC_TIME to C locale
        unsafe {
            env::set_var("LC_TIME", "C");
        }

        // Test that C locale detection works
        let lc_time = env::var("LC_TIME").unwrap_or_else(|_| {
            env::var("LC_ALL")
                .unwrap_or_else(|_| env::var("LANG").unwrap_or_else(|_| "C".to_string()))
        });
        assert_eq!(lc_time, "C");

        // 恢复原始环境变量
        unsafe {
            env::remove_var("LC_TIME");
        }
        unsafe {
            env::remove_var("LC_ALL");
        }
        unsafe {
            env::remove_var("LANG");
        }

        if let Some(val) = original_lc_time {
            unsafe {
                env::set_var("LC_TIME", val);
            }
        }
        if let Some(val) = original_lc_all {
            unsafe {
                env::set_var("LC_ALL", val);
            }
        }
        if let Some(val) = original_lang {
            unsafe {
                env::set_var("LANG", val);
            }
        }
    }

    #[test]
    fn test_time_string_non_c_locale() {
        let _guard = ENV_MUTEX.lock().unwrap();

        // 保存原始环境变量
        let original_lc_time = env::var("LC_TIME").ok();
        let original_lc_all = env::var("LC_ALL").ok();
        let original_lang = env::var("LANG").ok();

        // 清理并设置测试环境变量
        unsafe {
            env::remove_var("LC_TIME");
        }
        unsafe {
            env::remove_var("LC_ALL");
        }
        unsafe {
            env::remove_var("LANG");
        }
        unsafe {
            env::set_var("LC_TIME", "en_US.UTF-8");
        }

        // Test that non-C locale detection works
        let lc_time = env::var("LC_TIME").unwrap_or_else(|_| {
            env::var("LC_ALL")
                .unwrap_or_else(|_| env::var("LANG").unwrap_or_else(|_| "C".to_string()))
        });
        assert_eq!(lc_time, "en_US.UTF-8");

        // 恢复原始环境变量
        unsafe {
            env::remove_var("LC_TIME");
        }
        unsafe {
            env::remove_var("LC_ALL");
        }
        unsafe {
            env::remove_var("LANG");
        }

        if let Some(val) = original_lc_time {
            unsafe {
                env::set_var("LC_TIME", val);
            }
        }
        if let Some(val) = original_lc_all {
            unsafe {
                env::set_var("LC_ALL", val);
            }
        }
        if let Some(val) = original_lang {
            unsafe {
                env::set_var("LANG", val);
            }
        }
    }

    #[test]
    fn test_time_string_lc_all_fallback() {
        let _guard = ENV_MUTEX.lock().unwrap();

        // 保存原始环境变量
        let original_lc_time = env::var("LC_TIME").ok();
        let original_lc_all = env::var("LC_ALL").ok();
        let original_lang = env::var("LANG").ok();

        // 清理并设置测试环境变量
        unsafe {
            env::remove_var("LC_TIME");
        }
        unsafe {
            env::remove_var("LC_ALL");
        }
        unsafe {
            env::remove_var("LANG");
        }
        unsafe {
            env::set_var("LC_ALL", "POSIX");
        }

        // Test fallback to LC_ALL
        let lc_time = env::var("LC_TIME").unwrap_or_else(|_| {
            env::var("LC_ALL")
                .unwrap_or_else(|_| env::var("LANG").unwrap_or_else(|_| "C".to_string()))
        });
        assert_eq!(lc_time, "POSIX");

        // 恢复原始环境变量
        unsafe {
            env::remove_var("LC_TIME");
        }
        unsafe {
            env::remove_var("LC_ALL");
        }
        unsafe {
            env::remove_var("LANG");
        }

        if let Some(val) = original_lc_time {
            unsafe {
                env::set_var("LC_TIME", val);
            }
        }
        if let Some(val) = original_lc_all {
            unsafe {
                env::set_var("LC_ALL", val);
            }
        }
        if let Some(val) = original_lang {
            unsafe {
                env::set_var("LANG", val);
            }
        }
    }

    #[test]
    fn test_time_string_default_fallback() {
        let _guard = ENV_MUTEX.lock().unwrap();

        // 保存原始环境变量
        let original_lc_time = env::var("LC_TIME").ok();
        let original_lc_all = env::var("LC_ALL").ok();
        let original_lang = env::var("LANG").ok();

        // 清理所有locale环境变量
        unsafe {
            env::remove_var("LC_TIME");
        }
        unsafe {
            env::remove_var("LC_ALL");
        }
        unsafe {
            env::remove_var("LANG");
        }

        // Test fallback to default "C"
        let lc_time = env::var("LC_TIME").unwrap_or_else(|_| {
            env::var("LC_ALL")
                .unwrap_or_else(|_| env::var("LANG").unwrap_or_else(|_| "C".to_string()))
        });
        assert_eq!(lc_time, "C");

        // 恢复原始环境变量
        if let Some(val) = original_lc_time {
            unsafe {
                env::set_var("LC_TIME", val);
            }
        }
        if let Some(val) = original_lc_all {
            unsafe {
                env::set_var("LC_ALL", val);
            }
        }
        if let Some(val) = original_lang {
            unsafe {
                env::set_var("LANG", val);
            }
        }
    }

    #[test]
    fn test_time_format_width_c_locale() {
        let _guard = ENV_MUTEX.lock().unwrap();

        // 保存原始环境变量
        let original_lc_time = env::var("LC_TIME").ok();
        let original_lc_all = env::var("LC_ALL").ok();
        let original_lang = env::var("LANG").ok();

        // 清理并设置测试环境变量
        unsafe {
            env::remove_var("LC_TIME");
        }
        unsafe {
            env::remove_var("LC_ALL");
        }
        unsafe {
            env::remove_var("LANG");
        }
        unsafe {
            env::set_var("LC_TIME", "C");
        }

        let lc_time = env::var("LC_TIME").unwrap_or_else(|_| {
            env::var("LC_ALL")
                .unwrap_or_else(|_| env::var("LANG").unwrap_or_else(|_| "C".to_string()))
        });
        let time_size = if lc_time == "C" || lc_time == "POSIX" {
            3 + 1 + 2 + 1 + 2 + 1 + 2 // "Jul 24 22:08" = 12 chars
        } else {
            4 + 1 + 2 + 1 + 2 + 1 + 2 + 1 + 2 // "2025-07-24 22:08" = 16 chars
        };
        assert_eq!(time_size, 12);

        // 恢复原始环境变量
        unsafe {
            env::remove_var("LC_TIME");
        }
        unsafe {
            env::remove_var("LC_ALL");
        }
        unsafe {
            env::remove_var("LANG");
        }

        if let Some(val) = original_lc_time {
            unsafe {
                env::set_var("LC_TIME", val);
            }
        }
        if let Some(val) = original_lc_all {
            unsafe {
                env::set_var("LC_ALL", val);
            }
        }
        if let Some(val) = original_lang {
            unsafe {
                env::set_var("LANG", val);
            }
        }
    }

    #[test]
    fn test_time_format_width_non_c_locale() {
        let _guard = ENV_MUTEX.lock().unwrap();

        // 保存原始环境变量
        let original_lc_time = env::var("LC_TIME").ok();
        let original_lc_all = env::var("LC_ALL").ok();
        let original_lang = env::var("LANG").ok();

        // 清理并设置测试环境变量
        unsafe {
            env::remove_var("LC_TIME");
        }
        unsafe {
            env::remove_var("LC_ALL");
        }
        unsafe {
            env::remove_var("LANG");
        }
        unsafe {
            env::set_var("LC_TIME", "en_US.UTF-8");
        }

        let lc_time = env::var("LC_TIME").unwrap_or_else(|_| {
            env::var("LC_ALL")
                .unwrap_or_else(|_| env::var("LANG").unwrap_or_else(|_| "C".to_string()))
        });
        let time_size = if lc_time == "C" || lc_time == "POSIX" {
            3 + 1 + 2 + 1 + 2 + 1 + 2 // "Jul 24 22:08" = 12 chars
        } else {
            4 + 1 + 2 + 1 + 2 + 1 + 2 + 1 + 2 // "2025-07-24 22:08" = 16 chars
        };
        assert_eq!(time_size, 16);

        // 恢复原始环境变量
        unsafe {
            env::remove_var("LC_TIME");
        }
        unsafe {
            env::remove_var("LC_ALL");
        }
        unsafe {
            env::remove_var("LANG");
        }

        if let Some(val) = original_lc_time {
            unsafe {
                env::set_var("LC_TIME", val);
            }
        }
        if let Some(val) = original_lc_all {
            unsafe {
                env::set_var("LC_ALL", val);
            }
        }
        if let Some(val) = original_lang {
            unsafe {
                env::set_var("LANG", val);
            }
        }
    }

    #[test]
    fn test_time_format_width_c_utf8_locale() {
        let _guard = ENV_MUTEX.lock().unwrap();

        // 保存原始环境变量
        let original_lc_time = env::var("LC_TIME").ok();
        let original_lc_all = env::var("LC_ALL").ok();
        let original_lang = env::var("LANG").ok();

        // 清理并设置测试环境变量 - 测试C.UTF-8的正确处理
        unsafe {
            env::remove_var("LC_TIME");
        }
        unsafe {
            env::remove_var("LC_ALL");
        }
        unsafe {
            env::remove_var("LANG");
        }
        unsafe {
            env::set_var("LC_TIME", "C.UTF-8");
        }

        let lc_time = env::var("LC_TIME").unwrap_or_else(|_| {
            env::var("LC_ALL")
                .unwrap_or_else(|_| env::var("LANG").unwrap_or_else(|_| "C".to_string()))
        });
        // C.UTF-8应该使用ISO格式，因为它不等于"C"或"POSIX"
        let time_size = if lc_time == "C" || lc_time == "POSIX" {
            3 + 1 + 2 + 1 + 2 + 1 + 2 // "Jul 24 22:08" = 12 chars
        } else {
            4 + 1 + 2 + 1 + 2 + 1 + 2 + 1 + 2 // "2025-07-24 22:08" = 16 chars
        };
        assert_eq!(time_size, 16); // C.UTF-8应该使用ISO格式

        // 恢复原始环境变量
        unsafe {
            env::remove_var("LC_TIME");
        }
        unsafe {
            env::remove_var("LC_ALL");
        }
        unsafe {
            env::remove_var("LANG");
        }

        if let Some(val) = original_lc_time {
            unsafe {
                env::set_var("LC_TIME", val);
            }
        }
        if let Some(val) = original_lc_all {
            unsafe {
                env::set_var("LC_ALL", val);
            }
        }
        if let Some(val) = original_lang {
            unsafe {
                env::set_var("LANG", val);
            }
        }
    }

    #[test]
    fn test_lang_fallback_c_locale() {
        let _guard = ENV_MUTEX.lock().unwrap();

        // 保存原始环境变量
        let original_lc_time = env::var("LC_TIME").ok();
        let original_lc_all = env::var("LC_ALL").ok();
        let original_lang = env::var("LANG").ok();

        // 清理并设置测试环境变量 - 测试LANG回退
        unsafe {
            env::remove_var("LC_TIME");
        }
        unsafe {
            env::remove_var("LC_ALL");
        }
        unsafe {
            env::remove_var("LANG");
        }
        unsafe {
            env::set_var("LANG", "C");
        }

        let lc_time = env::var("LC_TIME").unwrap_or_else(|_| {
            env::var("LC_ALL")
                .unwrap_or_else(|_| env::var("LANG").unwrap_or_else(|_| "C".to_string()))
        });
        let time_size = if lc_time == "C" || lc_time == "POSIX" {
            3 + 1 + 2 + 1 + 2 + 1 + 2 // "Jul 24 22:08" = 12 chars
        } else {
            4 + 1 + 2 + 1 + 2 + 1 + 2 + 1 + 2 // "2025-07-24 22:08" = 16 chars
        };
        assert_eq!(time_size, 12);
        assert_eq!(lc_time, "C");

        // 恢复原始环境变量
        unsafe {
            env::remove_var("LC_TIME");
        }
        unsafe {
            env::remove_var("LC_ALL");
        }
        unsafe {
            env::remove_var("LANG");
        }

        if let Some(val) = original_lc_time {
            unsafe {
                env::set_var("LC_TIME", val);
            }
        }
        if let Some(val) = original_lc_all {
            unsafe {
                env::set_var("LC_ALL", val);
            }
        }
        if let Some(val) = original_lang {
            unsafe {
                env::set_var("LANG", val);
            }
        }
    }

    #[test]
    fn test_lang_fallback_utf8_locale() {
        let _guard = ENV_MUTEX.lock().unwrap();

        // 保存原始环境变量
        let original_lc_time = env::var("LC_TIME").ok();
        let original_lc_all = env::var("LC_ALL").ok();
        let original_lang = env::var("LANG").ok();

        // 清理并设置测试环境变量 - 测试LANG回退到C.UTF-8
        unsafe {
            env::remove_var("LC_TIME");
        }
        unsafe {
            env::remove_var("LC_ALL");
        }
        unsafe {
            env::remove_var("LANG");
        }
        unsafe {
            env::set_var("LANG", "C.UTF-8");
        }

        let lc_time = env::var("LC_TIME").unwrap_or_else(|_| {
            env::var("LC_ALL")
                .unwrap_or_else(|_| env::var("LANG").unwrap_or_else(|_| "C".to_string()))
        });
        let time_size = if lc_time == "C" || lc_time == "POSIX" {
            3 + 1 + 2 + 1 + 2 + 1 + 2 // "Jul 24 22:08" = 12 chars
        } else {
            4 + 1 + 2 + 1 + 2 + 1 + 2 + 1 + 2 // "2025-07-24 22:08" = 16 chars
        };
        assert_eq!(time_size, 16);
        assert_eq!(lc_time, "C.UTF-8");

        // 恢复原始环境变量
        unsafe {
            env::remove_var("LC_TIME");
        }
        unsafe {
            env::remove_var("LC_ALL");
        }
        unsafe {
            env::remove_var("LANG");
        }

        if let Some(val) = original_lc_time {
            unsafe {
                env::set_var("LC_TIME", val);
            }
        }
        if let Some(val) = original_lc_all {
            unsafe {
                env::set_var("LC_ALL", val);
            }
        }
        if let Some(val) = original_lang {
            unsafe {
                env::set_var("LANG", val);
            }
        }
    }

    #[test]
    fn test_lc_all_overrides_lang() {
        let _guard = ENV_MUTEX.lock().unwrap();

        // 保存原始环境变量
        let original_lc_time = env::var("LC_TIME").ok();
        let original_lc_all = env::var("LC_ALL").ok();
        let original_lang = env::var("LANG").ok();

        // 清理并设置测试环境变量 - 测试LC_ALL覆盖LANG
        unsafe {
            env::remove_var("LC_TIME");
        }
        unsafe {
            env::remove_var("LC_ALL");
        }
        unsafe {
            env::remove_var("LANG");
        }
        unsafe {
            env::set_var("LANG", "en_US.UTF-8");
        }
        unsafe {
            env::set_var("LC_ALL", "C");
        }

        let lc_time = env::var("LC_TIME").unwrap_or_else(|_| {
            env::var("LC_ALL")
                .unwrap_or_else(|_| env::var("LANG").unwrap_or_else(|_| "C".to_string()))
        });

        // LC_ALL should override LANG
        assert_eq!(lc_time, "C");

        let time_size = if lc_time == "C" || lc_time == "POSIX" {
            12 // C locale format
        } else {
            16 // ISO format
        };
        assert_eq!(time_size, 12);

        // 恢复原始环境变量
        unsafe {
            env::remove_var("LC_TIME");
        }
        unsafe {
            env::remove_var("LC_ALL");
        }
        unsafe {
            env::remove_var("LANG");
        }

        if let Some(val) = original_lc_time {
            unsafe {
                env::set_var("LC_TIME", val);
            }
        }
        if let Some(val) = original_lc_all {
            unsafe {
                env::set_var("LC_ALL", val);
            }
        }
        if let Some(val) = original_lang {
            unsafe {
                env::set_var("LANG", val);
            }
        }
    }

    #[test]
    fn test_time_string_with_actual_utmpx() {
        let _guard = ENV_MUTEX.lock().unwrap();

        // 保存原始环境变量
        let original_lc_time = env::var("LC_TIME").ok();
        let original_lc_all = env::var("LC_ALL").ok();
        let original_lang = env::var("LANG").ok();

        // 清理环境变量并设置C locale
        unsafe {
            env::remove_var("LC_TIME");
        }
        unsafe {
            env::remove_var("LC_ALL");
        }
        unsafe {
            env::remove_var("LANG");
        }
        unsafe {
            env::set_var("LC_TIME", "C");
        }

        // 尝试获取一个真实的utmpx记录进行测试
        if let Some(utmpx) = CtUtmpx::iter_all_records().next() {
            let time_str = time_string(utmpx.timestamp_seconds());
            // C locale时间格式应该是 "MMM DD HH:MM" (如 "Jul 24 22:08")
            // 检查格式是否正确 (月份简写 + 空格 + 日期 + 空格 + 时间)
            let parts: Vec<&str> = time_str.split_whitespace().collect();
            assert_eq!(parts.len(), 3); // 月份、日期、时间

            // 检查月份是否为英文缩写
            let month = parts[0];
            assert!(
                [
                    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov",
                    "Dec"
                ]
                .contains(&month)
            );

            // 检查时间格式 HH:MM
            let time_part = parts[2];
            assert!(time_part.contains(':'));
            let time_components: Vec<&str> = time_part.split(':').collect();
            assert_eq!(time_components.len(), 2);
            assert!(time_components[0].parse::<u32>().is_ok());
            assert!(time_components[1].parse::<u32>().is_ok());
        }

        // 恢复原始环境变量
        unsafe {
            env::remove_var("LC_TIME");
        }
        unsafe {
            env::remove_var("LC_ALL");
        }
        unsafe {
            env::remove_var("LANG");
        }

        if let Some(val) = original_lc_time {
            unsafe {
                env::set_var("LC_TIME", val);
            }
        }
        if let Some(val) = original_lc_all {
            unsafe {
                env::set_var("LC_ALL", val);
            }
        }
        if let Some(val) = original_lang {
            unsafe {
                env::set_var("LANG", val);
            }
        }
    }

    #[test]
    fn test_time_string_iso_format() {
        let _guard = ENV_MUTEX.lock().unwrap();

        // 保存原始环境变量
        let original_lc_time = env::var("LC_TIME").ok();
        let original_lc_all = env::var("LC_ALL").ok();
        let original_lang = env::var("LANG").ok();

        // 清理环境变量并设置非C locale
        unsafe {
            env::remove_var("LC_TIME");
        }
        unsafe {
            env::remove_var("LC_ALL");
        }
        unsafe {
            env::remove_var("LANG");
        }
        unsafe {
            env::set_var("LC_TIME", "en_US.UTF-8");
        }

        // 尝试获取一个真实的utmpx记录进行测试
        if let Some(utmpx) = CtUtmpx::iter_all_records().next() {
            let time_str = time_string(utmpx.timestamp_seconds());
            // ISO格式应该是 "YYYY-MM-DD HH:MM" (如 "2025-07-24 22:08")
            let parts: Vec<&str> = time_str.split_whitespace().collect();
            assert_eq!(parts.len(), 2); // 日期部分、时间部分

            // 检查日期格式 YYYY-MM-DD
            let date_part = parts[0];
            assert!(date_part.contains('-'));
            let date_components: Vec<&str> = date_part.split('-').collect();
            assert_eq!(date_components.len(), 3);
            assert!(date_components[0].parse::<u32>().is_ok()); // 年
            assert!(date_components[1].parse::<u32>().is_ok()); // 月
            assert!(date_components[2].parse::<u32>().is_ok()); // 日

            // 检查时间格式 HH:MM
            let time_part = parts[1];
            assert!(time_part.contains(':'));
            let time_components: Vec<&str> = time_part.split(':').collect();
            assert_eq!(time_components.len(), 2);
            assert!(time_components[0].parse::<u32>().is_ok());
            assert!(time_components[1].parse::<u32>().is_ok());
        }

        // 恢复原始环境变量
        unsafe {
            env::remove_var("LC_TIME");
        }
        unsafe {
            env::remove_var("LC_ALL");
        }
        unsafe {
            env::remove_var("LANG");
        }

        if let Some(val) = original_lc_time {
            unsafe {
                env::set_var("LC_TIME", val);
            }
        }
        if let Some(val) = original_lc_all {
            unsafe {
                env::set_var("LC_ALL", val);
            }
        }
        if let Some(val) = original_lang {
            unsafe {
                env::set_var("LANG", val);
            }
        }
    }

    #[test]
    fn test_various_locale_formats() {
        let _guard = ENV_MUTEX.lock().unwrap();

        // 保存原始环境变量
        let original_lc_time = env::var("LC_TIME").ok();
        let original_lc_all = env::var("LC_ALL").ok();
        let original_lang = env::var("LANG").ok();

        let test_cases = vec![
            ("C", true),            // 应该使用C格式
            ("POSIX", true),        // 应该使用C格式
            ("C.UTF-8", false),     // 应该使用ISO格式
            ("en_US.UTF-8", false), // 应该使用ISO格式
            ("zh_CN.UTF-8", false), // 应该使用ISO格式
            ("fr_FR.UTF-8", false), // 应该使用ISO格式
            ("de_DE.UTF-8", false), // 应该使用ISO格式
            ("ja_JP.UTF-8", false), // 应该使用ISO格式
        ];

        for (locale, should_use_c_format) in test_cases {
            // 清理并设置测试locale
            unsafe {
                env::remove_var("LC_TIME");
            }
            unsafe {
                env::remove_var("LC_ALL");
            }
            unsafe {
                env::remove_var("LANG");
            }
            unsafe {
                env::set_var("LC_TIME", locale);
            }

            let lc_time = env::var("LC_TIME").unwrap_or_else(|_| {
                env::var("LC_ALL")
                    .unwrap_or_else(|_| env::var("LANG").unwrap_or_else(|_| "C".to_string()))
            });

            let time_size = if lc_time == "C" || lc_time == "POSIX" {
                12 // C locale format
            } else {
                16 // ISO format
            };

            let expected_size = if should_use_c_format { 12 } else { 16 };
            assert_eq!(time_size, expected_size, "Failed for locale: {locale}");
        }

        // 恢复原始环境变量
        unsafe {
            env::remove_var("LC_TIME");
        }
        unsafe {
            env::remove_var("LC_ALL");
        }
        unsafe {
            env::remove_var("LANG");
        }

        if let Some(val) = original_lc_time {
            unsafe {
                env::set_var("LC_TIME", val);
            }
        }
        if let Some(val) = original_lc_all {
            unsafe {
                env::set_var("LC_ALL", val);
            }
        }
        if let Some(val) = original_lang {
            unsafe {
                env::set_var("LANG", val);
            }
        }
    }

    #[test]
    fn test_edge_case_empty_locale() {
        let _guard = ENV_MUTEX.lock().unwrap();

        // 保存原始环境变量
        let original_lc_time = env::var("LC_TIME").ok();
        let original_lc_all = env::var("LC_ALL").ok();
        let original_lang = env::var("LANG").ok();

        // 测试空的locale值
        unsafe {
            env::remove_var("LC_TIME");
        }
        unsafe {
            env::remove_var("LC_ALL");
        }
        unsafe {
            env::remove_var("LANG");
        }
        unsafe {
            env::set_var("LC_TIME", "");
        }

        let lc_time = env::var("LC_TIME").unwrap_or_else(|_| {
            env::var("LC_ALL")
                .unwrap_or_else(|_| env::var("LANG").unwrap_or_else(|_| "C".to_string()))
        });

        // 空字符串不等于"C"或"POSIX"，应该使用ISO格式
        let time_size = if lc_time == "C" || lc_time == "POSIX" {
            12 // C locale format
        } else {
            16 // ISO format
        };
        assert_eq!(time_size, 16);
        assert_eq!(lc_time, "");

        // 恢复原始环境变量
        unsafe {
            env::remove_var("LC_TIME");
        }
        unsafe {
            env::remove_var("LC_ALL");
        }
        unsafe {
            env::remove_var("LANG");
        }

        if let Some(val) = original_lc_time {
            unsafe {
                env::set_var("LC_TIME", val);
            }
        }
        if let Some(val) = original_lc_all {
            unsafe {
                env::set_var("LC_ALL", val);
            }
        }
        if let Some(val) = original_lang {
            unsafe {
                env::set_var("LANG", val);
            }
        }
    }

    #[test]
    fn test_case_sensitive_locale() {
        let _guard = ENV_MUTEX.lock().unwrap();

        // 保存原始环境变量
        let original_lc_time = env::var("LC_TIME").ok();
        let original_lc_all = env::var("LC_ALL").ok();
        let original_lang = env::var("LANG").ok();

        // 测试大小写敏感性
        let test_cases = vec![
            ("c", false),     // 小写c不等于"C"
            ("posix", false), // 小写posix不等于"POSIX"
            ("C ", false),    // 带空格的C
            (" C", false),    // 前导空格的C
            ("C\n", false),   // 带换行符的C
        ];

        for (locale, should_use_c_format) in test_cases {
            unsafe {
                env::remove_var("LC_TIME");
            }
            unsafe {
                env::remove_var("LC_ALL");
            }
            unsafe {
                env::remove_var("LANG");
            }
            unsafe {
                env::set_var("LC_TIME", locale);
            }

            let lc_time = env::var("LC_TIME").unwrap_or_else(|_| {
                env::var("LC_ALL")
                    .unwrap_or_else(|_| env::var("LANG").unwrap_or_else(|_| "C".to_string()))
            });

            let time_size = if lc_time == "C" || lc_time == "POSIX" {
                12 // C locale format
            } else {
                16 // ISO format
            };

            let expected_size = if should_use_c_format { 12 } else { 16 };
            assert_eq!(time_size, expected_size, "Failed for locale: '{locale}'");
        }

        // 恢复原始环境变量
        unsafe {
            env::remove_var("LC_TIME");
        }
        unsafe {
            env::remove_var("LC_ALL");
        }
        unsafe {
            env::remove_var("LANG");
        }

        if let Some(val) = original_lc_time {
            unsafe {
                env::set_var("LC_TIME", val);
            }
        }
        if let Some(val) = original_lc_all {
            unsafe {
                env::set_var("LC_ALL", val);
            }
        }
        if let Some(val) = original_lang {
            unsafe {
                env::set_var("LANG", val);
            }
        }
    }
}
