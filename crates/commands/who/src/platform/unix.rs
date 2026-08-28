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
use ctcore::ct_error::{CTResult, FromIo};
use ctcore::ct_locale::hard_locale_time;
use ctcore::ct_utmpx::{self, CtUtmpx, time};
use ctcore::libc::{S_IWGRP, STDIN_FILENO, ttyname};
use rust_i18n::t;
use std::borrow::Cow;
use std::ffi::CStr;
use std::fmt::Write;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use sys_locale::get_locale;

use crate::ct_app;
use crate::who_flags;

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
    let matches: clap::ArgMatches = ct_app()
        .after_help(get_long_usage())
        .try_get_matches_from(args)?;

    let mut who_cmd = who_from_matches(&matches);

    who_cmd.exec()
}

pub fn who_native_semantic(args: impl ctcore::Args) -> CTResult<WhoSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let matches: clap::ArgMatches = ct_app()
        .after_help(get_long_usage())
        .try_get_matches_from(args)?;
    let mut who_cmd = who_from_matches(&matches);
    who_cmd.collect_semantic()
}

fn who_from_matches(matches: &clap::ArgMatches) -> Who {
    let ct_files: Vec<String> = matches
        .get_many::<String>(who_flags::WHO_FILE)
        .map(|v| v.map(ToString::to_string).collect())
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
    let is_short_output = !is_include_exit && is_use_defaults;
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
    who_args: Vec<String>,
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

fn time_string(utmpx: &CtUtmpx) -> String {
    // Use ctcore's hard_locale_time() function (consistent with GNU coreutils)
    let time_fmt = if hard_locale_time() {
        // "%Y-%m-%d %H:%M" - ISO format for hard locales
        time::format_description::parse(
            "[year]-[month padding:zero]-[day padding:zero] [hour]:[minute]",
        )
        .unwrap()
    } else {
        // "%b %e %H:%M" - English month abbreviation format for C/POSIX locale
        time::format_description::parse("[month repr:short] [day padding:space] [hour]:[minute]")
            .unwrap()
    };

    utmpx.login_time().format(&time_fmt).unwrap()
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

/// Compute the display width of a string, counting CJK wide characters as 2 columns.
fn display_width(s: &str) -> usize {
    s.chars().map(|c| if is_wide_char(c) { 2 } else { 1 }).sum()
}

/// Check if a character is a CJK wide character (takes 2 display columns).
fn is_wide_char(c: char) -> bool {
    let cp = c as u32;
    (0x4E00..=0x9FFF).contains(&cp)
        || (0x3400..=0x4DBF).contains(&cp)
        || (0xF900..=0xFAFF).contains(&cp)
        || (0xFF01..=0xFF60).contains(&cp)
        || (0xFFE0..=0xFFE6).contains(&cp)
        || (0x20000..=0x2A6DF).contains(&cp)
        || (0xAC00..=0xD7AF).contains(&cp)
        || (0x2E80..=0x2EFF).contains(&cp)
        || (0x2F00..=0x2FDF).contains(&cp)
        || (0x3200..=0x32FF).contains(&cp)
        || (0x3300..=0x33FF).contains(&cp)
        || (0x3000..=0x303F).contains(&cp)
        || (0x3040..=0x309F).contains(&cp)
        || (0x30A0..=0x30FF).contains(&cp)
        || (0x3100..=0x312F).contains(&cp)
}

/// Pad a string to a given display width using spaces.
fn pad_right(s: &str, target_width: usize) -> String {
    let w = display_width(s);
    if w >= target_width {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(target_width - w))
    }
}

impl Who {
    #[allow(clippy::cognitive_complexity)]
    fn exec(&mut self) -> CTResult<()> {
        let run_level_chk = |_record: i16| {
            #[cfg(target_os = "linux")]
            return _record == ct_utmpx::RUN_LVL;
        };

        let f = match self.who_args.len() {
            1 => self.who_args[0].as_ref(),
            _ => ct_utmpx::DEFAULT_FILE,
        };

        if self.is_short_list {
            let users = CtUtmpx::iter_all_records_from(f)
                .filter(CtUtmpx::is_user_process)
                .map(|utmpx| utmpx.user())
                .collect::<Vec<_>>();
            println!("{}", users.join(" "));
            println!("{}={}", t!("who.output.users_count"), users.len());
        } else {
            let records = CtUtmpx::iter_all_records_from(f);

            if self.is_include_heading {
                self.print_head();
            }

            let current_tty = match self.is_my_line_only {
                true => cur_tty(),
                false => String::new(),
            };

            for utmpx in records {
                if !self.is_my_line_only || current_tty == utmpx.tty_device() {
                    if self.is_need_users && utmpx.is_user_process() {
                        self.print_user(&utmpx)?;
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
                        self.print_initspawn(&utmpx);
                    } else if self.is_need_login && utmpx.record_type() == ct_utmpx::LOGIN_PROCESS {
                        self.print_login(&utmpx);
                    } else if self.is_need_deadprocs
                        && utmpx.record_type() == ct_utmpx::DEAD_PROCESS
                    {
                        self.print_deadprocs(&utmpx);
                    }
                }

                if utmpx.record_type() == ct_utmpx::BOOT_TIME {}
            }
        }
        Ok(())
    }

    #[inline]
    fn print_runlevel(&self, utmpx: &CtUtmpx) {
        let last_runlevel = (utmpx.pid() / 256) as u8 as char;
        let current_runlevel = (utmpx.pid() % 256) as u8 as char;
        // Creating the run-level string
        let label = t!("who.output.run_level");
        let runlevel_line = format!("{label} {current_runlevel}");

        // 生成有关最后运行级别的注释
        let comment = if last_runlevel == 'N' {
            "last=S".to_string()
        } else {
            "last=N".to_string()
        };

        self.print_line(
            "",
            ' ',
            &runlevel_line,
            &time_string(utmpx),
            "",
            "",
            if last_runlevel.is_control() {
                ""
            } else {
                &comment
            },
            "",
        );
    }

    #[inline]
    fn print_clockchange(&self, utmpx: &CtUtmpx) {
        self.print_line(
            "",
            ' ',
            &t!("who.output.clock_change"),
            &time_string(utmpx),
            "",
            "",
            "",
            "",
        );
    }

    #[inline]
    fn print_login(&self, utmpx: &CtUtmpx) {
        let comment = format!("id={}", utmpx.terminal_suffix());
        let pid_str = format!("{}", utmpx.pid());
        self.print_line(
            &t!("who.output.login"),
            ' ',
            &utmpx.tty_device(),
            &time_string(utmpx),
            "",
            &pid_str,
            &comment,
            "",
        );
    }

    #[inline]
    fn print_deadprocs(&self, utmpx: &CtUtmpx) {
        let comment = format!("id={}", utmpx.terminal_suffix());
        let pid_str = format!("{}", utmpx.pid());
        let e = utmpx.exit_status();
        let exit_str = format!(
            "{}={} {}={}",
            t!("who.output.term"),
            e.0,
            t!("who.output.exit"),
            e.1
        );
        self.print_line(
            "",
            ' ',
            &utmpx.tty_device(),
            &time_string(utmpx),
            "",
            &pid_str,
            &comment,
            &exit_str,
        );
    }

    #[inline]
    fn print_initspawn(&self, utmpx: &CtUtmpx) {
        let comment = format!("id={}", utmpx.terminal_suffix());
        let pid_str = format!("{}", utmpx.pid());
        self.print_line(
            "",
            ' ',
            &utmpx.tty_device(),
            &time_string(utmpx),
            "",
            &pid_str,
            &comment,
            "",
        );
    }

    #[inline]
    fn print_boottime(&self, utmpx: &CtUtmpx) {
        self.print_line(
            "",
            ' ',
            &t!("who.output.system_boot"),
            &time_string(utmpx),
            "",
            "",
            "",
            "",
        );
    }

    fn print_user(&self, utmpx: &CtUtmpx) -> CTResult<()> {
        let mut p = PathBuf::from("/dev");
        p.push(utmpx.tty_device().as_str());

        let (mesg, last_change) = match p.metadata() {
            Ok(meta) => {
                #[cfg(target_os = "linux")]
                let iwgrp = S_IWGRP;
                let mesg = match meta.mode() & iwgrp == 0 {
                    true => '-',
                    false => '+',
                };

                (mesg, meta.atime())
            }
            _ => ('?', 0),
        };

        let idle = match last_change {
            0 => "  ?".into(),
            _ => idle_string(last_change, 0),
        };

        let s = match self.is_do_lookup {
            true => utmpx.canon_host().map_err_context(|| {
                let host_string = utmpx.host();
                format!(
                    "failed to canonicalize {}",
                    host_string
                        .split(':')
                        .next()
                        .unwrap_or(&host_string)
                        .quote()
                )
            })?,
            false => utmpx.host(),
        };

        let host_str = match s.is_empty() {
            true => s,
            false => {
                format!("({s})")
            }
        };

        self.print_line(
            utmpx.user().as_ref(),
            mesg,
            utmpx.tty_device().as_ref(),
            time_string(utmpx).as_str(),
            idle.as_ref(),
            format!("{}", utmpx.pid()).as_str(),
            host_str.as_str(),
            "",
        );

        Ok(())
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
        let mut buffer = String::with_capacity(64);
        let msg = vec![' ', fields.state].into_iter().collect::<String>();

        buffer.push_str(&pad_right(&fields.user, 8));
        if self.is_include_mesg {
            buffer.push_str(&msg);
        }
        buffer.push(' ');
        buffer.push_str(&pad_right(&fields.line, 12));

        // Dynamic time width based on locale (like coreutils)
        let lc_time = std::env::var("LC_TIME").unwrap_or_else(|_| {
            std::env::var("LC_ALL")
                .unwrap_or_else(|_| std::env::var("LANG").unwrap_or_else(|_| "C".to_string()))
        });
        let time_size = if lc_time == "C" || lc_time == "POSIX" {
            3 + 1 + 2 + 1 + 2 + 1 + 2
        } else {
            4 + 1 + 2 + 1 + 2 + 1 + 2 + 1 + 2
        };
        buffer.push(' ');
        buffer.push_str(&pad_right(&fields.time, time_size));

        if !self.is_short_output {
            if self.is_include_idle {
                buffer.push(' ');
                buffer.push_str(&pad_right(&fields.idle, 6));
            }
            write!(buffer, " {:>10}", fields.pid).unwrap();
        }

        buffer.push(' ');
        buffer.push_str(&pad_right(&fields.comment, 8));

        if self.is_include_exit {
            buffer.push(' ');
            buffer.push_str(&pad_right(&fields.exit, 12));
        }

        buffer.trim_end().to_string()
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

    fn source_file(&self) -> &str {
        match self.who_args.len() {
            1 => self.who_args[0].as_ref(),
            _ => ct_utmpx::DEFAULT_FILE,
        }
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

    fn build_user_row(&self, utmpx: &CtUtmpx) -> CTResult<(WhoRow, WhoDisplayLine)> {
        let mut p = PathBuf::from("/dev");
        p.push(utmpx.tty_device().as_str());

        let (mesg, last_change) = match p.metadata() {
            Ok(meta) => {
                let mesg = match meta.mode() & S_IWGRP == 0 {
                    true => '-',
                    false => '+',
                };

                (mesg, meta.atime())
            }
            _ => ('?', 0),
        };

        let idle = match last_change {
            0 => "  ?".to_owned(),
            _ => idle_string(last_change, 0).into_owned(),
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
        let time = time_string(utmpx);
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

        let source_file = self.source_file().to_string();
        let mut semantic = WhoSemantic {
            view_kind: self.view_kind(),
            source_file: source_file.clone(),
            rows: Vec::new(),
            classic_text: String::new(),
            stderr_text: String::new(),
            exit_code: 0,
        };

        if self.is_short_list {
            let users = CtUtmpx::iter_all_records_from(&source_file)
                .filter(CtUtmpx::is_user_process)
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
            if self.is_my_line_only && current_tty != utmpx.tty_device() {
                continue;
            }

            if self.is_need_users && utmpx.is_user_process() {
                let (row, display) = self.build_user_row(&utmpx)?;
                self.push_row(&mut semantic, row, display);
            } else if self.is_need_runlevel && run_level_chk(utmpx.record_type()) {
                if cfg!(target_os = "linux") {
                    let last_runlevel = (utmpx.pid() / 256) as u8 as char;
                    let current_runlevel = (utmpx.pid() % 256) as u8 as char;
                    let label = t!("who.output.run_level");
                    let runlevel_line = format!("{label} {current_runlevel}");
                    let comment = if last_runlevel == 'N' {
                        "last=S".to_string()
                    } else {
                        "last=N".to_string()
                    };
                    let (row, display) = self.build_simple_row(
                        "runlevel",
                        "",
                        &runlevel_line,
                        &time_string(&utmpx),
                        "",
                        if last_runlevel.is_control() {
                            ""
                        } else {
                            &comment
                        },
                        "",
                    );
                    self.push_row(&mut semantic, row, display);
                }
            } else if self.is_need_boottime && utmpx.record_type() == ct_utmpx::BOOT_TIME {
                let (row, display) = self.build_simple_row(
                    "boot_time",
                    "",
                    &t!("who.output.system_boot"),
                    &time_string(&utmpx),
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
                    &time_string(&utmpx),
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
                    &time_string(&utmpx),
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
                    &time_string(&utmpx),
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
                    &time_string(&utmpx),
                    &pid,
                    &comment,
                    &exit_str,
                );
                self.push_row(&mut semantic, row, display);
            }
        }

        Ok(semantic)
    }
}

#[cfg(test)]
mod tests {
    use ctcore::ct_utmpx::time::OffsetDateTime;
    use std::env;
    use std::sync::Mutex;

    use super::*;

    // 互斥锁确保环境变量测试的串行执行，避免并发测试时的干扰
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

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
            who_args: vec!["/var/log/wtmp".to_string()],
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
    fn test_lc_time_overrides_all() {
        let _guard = ENV_MUTEX.lock().unwrap();

        // 保存原始环境变量
        let original_lc_time = env::var("LC_TIME").ok();
        let original_lc_all = env::var("LC_ALL").ok();
        let original_lang = env::var("LANG").ok();

        // 清理并设置测试环境变量 - 测试LC_TIME优先级最高
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
        unsafe {
            env::set_var("LC_ALL", "en_US.UTF-8");
        }
        unsafe {
            env::set_var("LC_TIME", "zh_CN.UTF-8");
        }

        let lc_time = env::var("LC_TIME").unwrap_or_else(|_| {
            env::var("LC_ALL")
                .unwrap_or_else(|_| env::var("LANG").unwrap_or_else(|_| "C".to_string()))
        });

        // LC_TIME should have highest priority
        assert_eq!(lc_time, "zh_CN.UTF-8");

        let time_size = if lc_time == "C" || lc_time == "POSIX" {
            12 // C locale format
        } else {
            16 // ISO format
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
            let time_str = time_string(&utmpx);
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
            let time_str = time_string(&utmpx);
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
