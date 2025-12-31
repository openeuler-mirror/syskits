/*
 *    Copyright(c) 2022-2024 China Telecom Cloud Technologies co., Ltd. All rights reserved
 *     syskits is licensed under Mulan PSL v2.
 *    You can use this software according to the terms and conditions of the Mulan PSL V2
 *    You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 *    THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 *    KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 *    NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 *    See the Mulan PSL v2 for more details.
 *
 */

use std::borrow::Cow;
use std::ffi::CStr;
use std::fmt::Write;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, FromIo};
use ctcore::ct_utmpx::{self, CtUtmpx, time};
use ctcore::libc::{S_IWGRP, STDIN_FILENO, ttyname};

use crate::ct_app;
use crate::who_flags;

fn get_long_usage() -> String {
    format!(
        "If FILE is not specified, use {}.  /var/log/wtmp as FILE is common.\n\
          If ARG1 ARG2 given, -m presumed: 'am i' or 'mom likes' are usual.",
        ct_utmpx::DEFAULT_FILE,
    )
}

pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    who_main(args)
}

pub fn who_main(args: impl ctcore::Args) -> CTResult<()> {
    let matches: clap::ArgMatches = ct_app()
        .after_help(get_long_usage())
        .try_get_matches_from(args)?;

    let ct_files: Vec<String> = matches
        .get_many::<String>(who_flags::WHO_FILE)
        .map(|v| v.map(ToString::to_string).collect())
        .unwrap_or_default();

    // 如果为 "true"，则尝试通过 DNS 查询对主机名进行规范化。
    let is_do_lookup = matches.get_flag(who_flags::WHO_LOOKUP);

    // 如果为 "true"，则只显示用户名列表和已登录用户的计数。
    //   忽略 'who am i'.
    let is_short_list = matches.get_flag(who_flags::WHO_COUNT);

    let si_all = matches.get_flag(who_flags::WHO_ALL);

    // 如果为 "true"，则在顶部显示一行，描述每个字段。
    let is_include_heading = matches.get_flag(who_flags::WHO_HEADING);

    // 如果为 "true"，则在 mesg 为 y 时为每个用户显示 "+"，
    // 在 mesg 为 n 时显示"-"，或者在无法统计其 tty 时显示"?
    let is_include_mesg = si_all || matches.get_flag(who_flags::WHO_MESG);

    // 如果为 "true"，则显示上次启动时间。
    let is_need_boottime = si_all || matches.get_flag(who_flags::WHO_BOOT);

    // 如果为 "true"，则显示死亡进程。
    let is_need_deadprocs = si_all || matches.get_flag(who_flags::WHO_DEAD);

    // 如果为 "true"，则显示等待用户登录的进程。
    let is_need_login = si_all || matches.get_flag(who_flags::WHO_LOGIN);

    // 如果为 true，则显示 init 启动的进程。
    let is_need_initspawn = si_all || matches.get_flag(who_flags::WHO_PROCESS);

    // 如果为 "true"，则显示最后一次时钟变化。
    let is_need_clockchange = si_all || matches.get_flag(who_flags::WHO_TIME);

    // 如果为 true，则显示当前运行级别。
    let is_need_runlevel = si_all || matches.get_flag(who_flags::WHO_RUNLEVEL);

    let is_use_defaults = !(si_all
        || is_need_boottime
        || is_need_deadprocs
        || is_need_login
        || is_need_initspawn
        || is_need_runlevel
        || is_need_clockchange
        || matches.get_flag(who_flags::WHO_USERS));

    // 如果为 "true"，则显示用户进程。
    let is_need_users = si_all || matches.get_flag(who_flags::WHO_USERS) || is_use_defaults;

    // 如果为 "true"，则显示每个用户触摸键盘后的小时：分钟，如果在最后一分钟内，则显示"."，如果 则显示 "old"。
    let is_include_idle = is_need_deadprocs || is_need_login || is_need_runlevel || is_need_users;

    // 如果为 "true"，则显示进程终止和退出状态。
    let is_include_exit = is_need_deadprocs;

    // 如果为 "true"，则只显示名称、行和时间字段。
    let is_short_output = !is_include_exit && is_use_defaults;

    // 如果为 true，则只显示控制 tty 的信息。
    let is_my_line_only =
        matches.get_flag(who_flags::WHO_ONLY_HOSTNAME_USER) || ct_files.len() == 2;

    let mut who_cmd = Who {
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
    };

    who_cmd.exec()
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
    // "%b %e %H:%M"
    let time_fmt: Vec<time::format_description::FormatItem> =
        time::format_description::parse("[month repr:short] [day padding:space] [hour]:[minute]")
            .unwrap();
    utmpx.login_time().format(&time_fmt).unwrap() // LC_ALL=C
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

impl Who {
    #[allow(clippy::cognitive_complexity)]
    fn exec(&mut self) -> CTResult<()> {
        let run_level_chk = |_record: i16| {
            #[cfg(not(target_os = "linux"))]
            return false;

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
            println!("# users={}", users.len());
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
        let runlevel_line = format!("run-level {}", current_runlevel);

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
        self.print_line("", ' ', "clock change", &time_string(utmpx), "", "", "", "");
    }

    #[inline]
    fn print_login(&self, utmpx: &CtUtmpx) {
        let comment = format!("id={}", utmpx.terminal_suffix());
        let pid_str = format!("{}", utmpx.pid());
        self.print_line(
            "LOGIN",
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
        let exit_str = format!("term={} exit={}", e.0, e.1);
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
        self.print_line("", ' ', "system boot", &time_string(utmpx), "", "", "", "");
    }

    fn print_user(&self, utmpx: &CtUtmpx) -> CTResult<()> {
        let mut p = PathBuf::from("/dev");
        p.push(utmpx.tty_device().as_str());

        let (mesg, last_change) = match p.metadata() {
            Ok(meta) => {
                #[cfg(all(
                    not(target_os = "freebsd"),
                    not(target_os = "android"),
                    not(target_vendor = "apple")
                ))]
                let iwgrp = S_IWGRP;
                #[cfg(any(target_os = "android", target_os = "freebsd", target_vendor = "apple"))]
                let iwgrp = S_IWGRP as u32;
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
        let mut buffer = String::with_capacity(64);
        let msg = vec![' ', state].into_iter().collect::<String>();

        write!(buffer, "{user:<8}").unwrap();
        if self.is_include_mesg {
            buffer.push_str(&msg);
        }
        write!(buffer, " {line:<12}").unwrap();
        // "%b %e %H:%M" (LC_ALL=C)
        let time_size = 3 + 2 + 2 + 1 + 2;
        write!(buffer, " {time:<time_size$}").unwrap();

        if !self.is_short_output {
            if self.is_include_idle {
                write!(buffer, " {idle:<6}").unwrap();
            }
            write!(buffer, " {pid:>10}").unwrap();
        }

        write!(buffer, " {comment:<8}").unwrap();

        if self.is_include_exit {
            write!(buffer, " {exit:<12}").unwrap();
        }

        println!("{}", buffer.trim_end());
    }

    #[inline]
    fn print_head(&self) {
        self.print_line(
            "NAME", ' ', "LINE", "TIME", "IDLE", "PID", "COMMENT", "EXIT",
        );
    }
}

#[cfg(test)]
mod tests {
    use ctcore::ct_utmpx::time::OffsetDateTime;

    use super::*;

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

        assert_eq!(who.exec().is_ok(), true);
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
}