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

//! Pinky - 轻量级的用户信息查询工具
//!
//! 该模块实现了类似于 finger 命令的功能，用于显示系统用户的信息。
//! 主要功能包括:
//! - 显示用户的登录状态
//! - 显示用户的个人信息(全名、主目录、Shell等)
//! - 显示用户的项目和计划文件内容
//! - 支持短格式和长格式两种显示方式
//! - 提供多种自定义显示选项
//!
//! 短格式输出示例:
//! ```text
//! Login    Name            TTY      Idle    When            Where
//! alice    Alice Smith     tty1     12:31   Jun 12 09:32   localhost
//! bob      Bob Jones       pts/0    2d      Jun 10 15:45   remote.host
//! ```
//!
//! 长格式输出包含更详细的用户信息，如主目录、Shell、项目文件等。

// spell-checker:ignore (ToDO) BUFSIZE gecos fullname, mesg iobuf

use clap::{Arg, ArgAction, Command, crate_version};
use ctcore::ct_entries::{CtPasswd, Locate};
use ctcore::ct_error::{CTResult, FromIo};
use ctcore::ct_utmpx::{self, CtUtmpx, time};
use ctcore::libc::S_IWGRP;
use ctcore::{ct_format_usage, ct_help_about, ct_help_usage};

use std::io::BufReader;
use std::io::prelude::*;

use std::fs::File;
use std::os::unix::fs::MetadataExt;

use std::path::PathBuf;

const PINKY_ABOUT: &str = ct_help_about!("pinky.md");
const PINKY_USAGE: &str = ct_help_usage!("pinky.md");

mod pinky_options {
    pub const PINKY_LONG_FORMAT: &str = "long_format";
    pub const PINKY_OMIT_HOME_DIR: &str = "omit_home_dir";
    pub const PINKY_OMIT_PROJECT_FILE: &str = "omit_project_file";
    pub const PINKY_OMIT_PLAN_FILE: &str = "omit_plan_file";
    pub const PINKY_SHORT_FORMAT: &str = "short_format";
    pub const PINKY_OMIT_HEADINGS: &str = "omit_headings";
    pub const PINKY_OMIT_NAME: &str = "omit_name";
    pub const PINKY_OMIT_NAME_HOST: &str = "omit_name_host";
    pub const PINKY_OMIT_NAME_HOST_TIME: &str = "omit_name_host_time";
    pub const PINKY_USER: &str = "user";
    pub const PINKY_HELP: &str = "help";
}

pub fn ct_app() -> Command {
    let args = vec![
        Arg::new(pinky_options::PINKY_LONG_FORMAT)
            .short('l')
            .requires(pinky_options::PINKY_USER)
            .help("produce long ct_format output for the specified USERs")
            .action(ArgAction::SetTrue),
        Arg::new(pinky_options::PINKY_OMIT_HOME_DIR)
            .short('b')
            .help("omit the user's home directory and shell in long ct_format")
            .action(ArgAction::SetTrue),
        Arg::new(pinky_options::PINKY_OMIT_PROJECT_FILE)
            .short('h')
            .help("omit the user's project file in long ct_format")
            .action(ArgAction::SetTrue),
        Arg::new(pinky_options::PINKY_OMIT_PLAN_FILE)
            .short('p')
            .help("omit the user's plan file in long ct_format")
            .action(ArgAction::SetTrue),
        Arg::new(pinky_options::PINKY_SHORT_FORMAT)
            .short('s')
            .help("do short ct_format output, this is the default")
            .action(ArgAction::SetTrue),
        Arg::new(pinky_options::PINKY_OMIT_HEADINGS)
            .short('f')
            .help("omit the line of column headings in short ct_format")
            .action(ArgAction::SetTrue),
        Arg::new(pinky_options::PINKY_OMIT_NAME)
            .short('w')
            .help("omit the user's full name in short ct_format")
            .action(ArgAction::SetTrue),
        Arg::new(pinky_options::PINKY_OMIT_NAME_HOST)
            .short('i')
            .help("omit the user's full name and remote host in short ct_format")
            .action(ArgAction::SetTrue),
        Arg::new(pinky_options::PINKY_OMIT_NAME_HOST_TIME)
            .short('q')
            .help("omit the user's full name, remote host and idle time in short ct_format")
            .action(ArgAction::SetTrue),
        Arg::new(pinky_options::PINKY_USER)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::Username),
        // Redefine the help argument to not include the short flag
        // since that conflicts with omit_project_file.
        Arg::new(pinky_options::PINKY_HELP)
            .long(pinky_options::PINKY_HELP)
            .help("Print help information")
            .action(ArgAction::Help),
    ];

    Command::new(ctcore::ct_util_name())
        .version(crate_version!())
        .about(PINKY_ABOUT)
        .override_usage(ct_format_usage(PINKY_USAGE))
        .infer_long_args(true)
        .disable_help_flag(true)
        .args(args)
}

fn get_long_usage() -> String {
    format!(
        "A lightweight 'finger' program;  print user information.\n\
         The utmp file will be {}.",
        ct_utmpx::DEFAULT_FILE
    )
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    pinky_main(args)
}

pub fn pinky_main(args: impl ctcore::Args) -> CTResult<()> {
    let matches = ct_app()
        .after_help(get_long_usage())
        .try_get_matches_from(args)?;

    let pk = Pinky::new(&matches);
    let do_short_format = !matches.get_flag(pinky_options::PINKY_LONG_FORMAT);

    if do_short_format {
        match pk.short_pinky() {
            Ok(_) => Ok(()),
            Err(e) => Err(e.map_err_context(String::new)),
        }
    } else {
        pk.long_pinky();
        Ok(())
    }
}

struct Pinky {
    is_include_idle: bool,
    is_include_heading: bool,
    is_include_fullname: bool,
    is_include_project: bool,
    is_include_plan: bool,
    is_include_where: bool,
    is_include_home_and_shell: bool,
    pinky_names: Vec<String>,
}

/// 计算用户空闲时间的字符串表示
/// 返回格式:
/// - 小于1分钟: 5个空格
/// - 小于1天: "HH:MM"
/// - 大于1天: "Nd" (N是天数)
fn pinky_idle_string(when: i64) -> String {
    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;

    thread_local! {
        static NOW: time::OffsetDateTime = time::OffsetDateTime::now_local().unwrap();
    }

    NOW.with(|current_time| {
        let duration = current_time.unix_timestamp() - when;
        match duration {
            d if d < MINUTE => "     ".to_owned(),
            d if d < DAY => {
                let hours = d / HOUR;
                let minutes = (d % HOUR) / MINUTE;
                format!("{hours:02}:{minutes:02}")
            }
            d => format!("{}d", d / DAY),
        }
    })
}

/// 格式化登录时间为 "%b %e %H:%M" 格式
fn time_string(ut: &CtUtmpx) -> String {
    const TIME_FORMAT: &str = "[month repr:short] [day padding:space] [hour]:[minute]";
    let format = time::format_description::parse(TIME_FORMAT).unwrap();
    ut.login_time().format(&format).unwrap()
}

/// 从 GECOS 字段提取用户全名
fn gecos_to_fullname(pw: &CtPasswd) -> Option<String> {
    pw.user_info.as_ref().map(|gecos| {
        let name = match gecos.find(',') {
            Some(pos) => &gecos[..pos],
            None => gecos,
        };
        name.replace('&', &pw.name.capitalize())
    })
}

impl Pinky {
    /// 从命令行参数创建新的 Pinky 实例
    fn new(matches: &clap::ArgMatches) -> Self {
        let users: Vec<String> = matches
            .get_many::<String>(pinky_options::PINKY_USER)
            .map(|v| v.map(ToString::to_string).collect())
            .unwrap_or_default();

        let mut include_idle = true;
        let mut include_fullname = true;
        let mut include_where = true;

        // 处理各种显示选项
        if matches.get_flag(pinky_options::PINKY_OMIT_NAME) {
            include_fullname = false;
        }
        if matches.get_flag(pinky_options::PINKY_OMIT_NAME_HOST) {
            include_fullname = false;
            include_where = false;
        }
        if matches.get_flag(pinky_options::PINKY_OMIT_NAME_HOST_TIME) {
            include_fullname = false;
            include_idle = false;
            include_where = false;
        }

        Self {
            is_include_idle: include_idle,
            is_include_heading: !matches.get_flag(pinky_options::PINKY_OMIT_HEADINGS),
            is_include_fullname: include_fullname,
            is_include_project: !matches.get_flag(pinky_options::PINKY_OMIT_PROJECT_FILE),
            is_include_plan: !matches.get_flag(pinky_options::PINKY_OMIT_PLAN_FILE),
            is_include_home_and_shell: !matches.get_flag(pinky_options::PINKY_OMIT_HOME_DIR),
            is_include_where: include_where,
            pinky_names: users,
        }
    }

    /// 打印单个用户的登录信息
    fn print_entry(&self, ut: &CtUtmpx) -> std::io::Result<()> {
        let (mesg, last_change) = self.get_tty_info(ut)?;
        self.print_user_info(ut);
        self.print_fullname(ut);
        self.print_tty_info(ut, mesg);
        self.print_idle_time(last_change);
        self.print_login_time(ut);
        self.print_host_info(ut)?;
        println!();
        Ok(())
    }

    fn get_tty_info(&self, ut: &CtUtmpx) -> std::io::Result<(char, i64)> {
        let tty_path = PathBuf::from("/dev").join(ut.tty_device().as_str());
        match tty_path.metadata() {
            Ok(meta) => Ok((
                if meta.mode() & S_IWGRP == 0 {
                    '*'
                } else {
                    ' '
                },
                meta.atime(),
            )),
            Err(_) => Ok(('?', 0)),
        }
    }

    fn print_user_info(&self, ut: &CtUtmpx) {
        print!("{1:<8.0$}", ct_utmpx::UT_NAMESIZE, ut.user());
    }

    fn print_fullname(&self, ut: &CtUtmpx) {
        if !self.is_include_fullname {
            return;
        }
        let fullname = CtPasswd::locate(ut.user().as_ref())
            .ok()
            .and_then(|pw| gecos_to_fullname(&pw))
            .unwrap_or_else(|| "        ???".to_string());
        print!(" {:<19.19}", fullname);
    }

    fn print_tty_info(&self, ut: &CtUtmpx, mesg: char) {
        print!(" {}{:<8.*}", mesg, ct_utmpx::UT_LINESIZE, ut.tty_device());
    }

    fn print_idle_time(&self, last_change: i64) {
        if !self.is_include_idle {
            return;
        }
        let idle = if last_change == 0 {
            "?????".to_string()
        } else {
            pinky_idle_string(last_change)
        };
        print!(" {:<6}", idle);
    }

    fn print_login_time(&self, ut: &CtUtmpx) {
        print!(" {}", time_string(ut));
    }

    fn print_host_info(&self, ut: &CtUtmpx) -> std::io::Result<()> {
        if !self.is_include_where {
            return Ok(());
        }
        let host = ut.host();
        if !host.is_empty() {
            print!(" {}", ut.canon_host()?);
        }
        Ok(())
    }

    /// 打印列标题
    fn print_heading(&self) {
        if !self.is_include_heading {
            return;
        }

        let mut columns = vec![("Login", 8)];
        if self.is_include_fullname {
            columns.push(("Name", 19));
        }
        columns.push(("TTY", 9));
        if self.is_include_idle {
            columns.push(("Idle", 6));
        }
        columns.push(("When", 16));
        if self.is_include_where {
            columns.push(("Where", 0));
        }

        for (i, (title, width)) in columns.iter().enumerate() {
            if i > 0 {
                print!(" ");
            }
            if *width > 0 {
                print!("{:<width$}", title, width = width);
            } else {
                print!("{}", title);
            }
        }
        println!();
    }

    /// 以短格式显示用户信息
    fn short_pinky(&self) -> std::io::Result<()> {
        self.print_heading();

        for ut in CtUtmpx::iter_all_records() {
            if self.should_display_user(&ut) {
                self.print_entry(&ut)?;
            }
        }
        Ok(())
    }

    fn should_display_user(&self, ut: &CtUtmpx) -> bool {
        ut.is_user_process()
            && (self.pinky_names.is_empty()
                || self.pinky_names.iter().any(|n| n.as_str() == ut.user()))
    }

    /// 以长格式显示用户信息
    fn long_pinky(&self) {
        for username in &self.pinky_names {
            self.print_long_user_info(username);
        }
    }

    fn print_long_user_info(&self, username: &str) {
        print!("Login name: {:<28}In real life: ", username);

        match CtPasswd::locate(username) {
            Ok(pw) => {
                let fullname = gecos_to_fullname(&pw).unwrap_or_default();
                let user_dir = pw.user_dir.unwrap_or_default();
                let user_shell = pw.user_shell.unwrap_or_default();

                println!(" {}", fullname);
                self.print_home_and_shell(&user_dir, &user_shell);
                self.print_project_file(&user_dir);
                self.print_plan_file(&user_dir);
                println!();
            }
            Err(_) => println!(" ???"),
        }
    }

    fn print_home_and_shell(&self, user_dir: &str, user_shell: &str) {
        if self.is_include_home_and_shell {
            print!("Directory: {:<29}", user_dir);
            println!("Shell:  {}", user_shell);
        }
    }

    fn print_project_file(&self, user_dir: &str) {
        if self.is_include_project {
            if let Ok(f) = File::open(PathBuf::from(user_dir).join(".project")) {
                print!("Project: ");
                read_to_console(f);
            }
        }
    }

    fn print_plan_file(&self, user_dir: &str) {
        if self.is_include_plan {
            if let Ok(f) = File::open(PathBuf::from(user_dir).join(".plan")) {
                println!("Plan:");
                read_to_console(f);
            }
        }
    }
}

fn read_to_console<F: Read>(f: F) {
    let mut reader = BufReader::new(f);
    let mut iobuf = Vec::new();
    if reader.read_to_end(&mut iobuf).is_ok() {
        print!("{}", String::from_utf8_lossy(&iobuf));
    }
}

/// 为字符串提供首字母大写功能的 trait
pub trait Capitalize {
    /// 将字符串的第一个字母转换为大写
    fn capitalize(&self) -> String;
}

impl Capitalize for str {
    fn capitalize(&self) -> String {
        // 预分配足够的容量以避免重新分配
        self.char_indices()
            .fold(String::with_capacity(self.len()), |mut acc, x| {
                if x.0 == 0 {
                    // 如果是第一个字符，转换为大写
                    acc.push(x.1.to_ascii_uppercase());
                } else {
                    // 其他字符保持不变
                    acc.push(x.1);
                }
                acc
            })
    }
}
