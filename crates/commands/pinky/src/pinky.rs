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

extern crate rust_i18n;
use clap::{Arg, ArgAction, Command, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "zh-CN");
use ctcore::Tool;
use ctcore::ct_entries::{CtPasswd, Locate};
use ctcore::ct_error::{CTResult, FromIo};
use ctcore::ct_locale::hard_locale_time;
use ctcore::ct_utmpx::{self, CtUtmpx, time};
use ctcore::libc::S_IWGRP;
use std::ffi::OsString;
use std::fs::File;
use std::io::BufReader;
use std::io::prelude::*;
use std::os::unix::fs::MetadataExt;
use sys_locale::get_locale;

use std::path::PathBuf;

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
            .help(t!("pinky.clap.pinky_long_format"))
            .action(ArgAction::SetTrue),
        Arg::new(pinky_options::PINKY_OMIT_HOME_DIR)
            .short('b')
            .help(t!("pinky.clap.pinky_omit_home_dir"))
            .action(ArgAction::SetTrue),
        Arg::new(pinky_options::PINKY_OMIT_PROJECT_FILE)
            .short('h')
            .help(t!("pinky.clap.pinky_omit_project_file"))
            .action(ArgAction::SetTrue),
        Arg::new(pinky_options::PINKY_OMIT_PLAN_FILE)
            .short('p')
            .help(t!("pinky.clap.pinky_omit_plan_file"))
            .action(ArgAction::SetTrue),
        Arg::new(pinky_options::PINKY_SHORT_FORMAT)
            .short('s')
            .help(t!("pinky.clap.pinky_short_format"))
            .action(ArgAction::SetTrue),
        Arg::new(pinky_options::PINKY_OMIT_HEADINGS)
            .short('f')
            .help(t!("pinky.clap.pinky_omit_headings"))
            .action(ArgAction::SetTrue),
        Arg::new(pinky_options::PINKY_OMIT_NAME)
            .short('w')
            .help(t!("pinky.clap.pinky_omit_name"))
            .action(ArgAction::SetTrue),
        Arg::new(pinky_options::PINKY_OMIT_NAME_HOST)
            .short('i')
            .help(t!("pinky.clap.pinky_omit_name_host"))
            .action(ArgAction::SetTrue),
        Arg::new(pinky_options::PINKY_OMIT_NAME_HOST_TIME)
            .short('q')
            .help(t!("pinky.clap.pinky_omit_name_host_time"))
            .action(ArgAction::SetTrue),
        Arg::new(pinky_options::PINKY_USER)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::Username),
        // Redefine the help argument to not include the short flag
        // since that conflicts with omit_project_file.
        Arg::new(pinky_options::PINKY_HELP)
            .long(pinky_options::PINKY_HELP)
            .help(t!("pinky.clap.pinky_help"))
            .action(ArgAction::Help),
    ];

    Command::new(ctcore::ct_util_name())
        .version(crate_version!())
        .about(t!("pinky.about"))
        .override_usage(t!("pinky.usage"))
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

#[derive(Default)]
pub struct Pinky;
impl Tool for Pinky {
    fn name(&self) -> &'static str {
        "pinky"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        pinky_main(args.iter().cloned())
    }
}

pub fn pinky_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let matches = ct_app()
        .after_help(get_long_usage())
        .try_get_matches_from(args)?;

    let pk = PinkyFlags::new(&matches);
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

struct PinkyFlags {
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

/// 格式化登录时间，根据locale决定格式
/// hard_locale为true时使用 "%Y-%m-%d %H:%M"，否则使用 "%b %e %H:%M"
fn time_string(ut: &CtUtmpx) -> String {
    if hard_locale_time() {
        // 使用ISO格式，包含年份
        const TIME_FORMAT: &str = "[year]-[month]-[day] [hour]:[minute]";
        let format = time::format_description::parse(TIME_FORMAT).unwrap();
        ut.login_time().format(&format).unwrap()
    } else {
        // 使用传统格式，不包含年份
        const TIME_FORMAT: &str = "[month repr:short] [day padding:space] [hour]:[minute]";
        let format = time::format_description::parse(TIME_FORMAT).unwrap();
        ut.login_time().format(&format).unwrap()
    }
}

/// 获取时间字符串的显示宽度
fn time_format_width() -> usize {
    if hard_locale_time() {
        16 // "2024-12-25 15:30" = 16 characters
    } else {
        12 // "Dec 25 15:30" = 12 characters  
    }
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

impl PinkyFlags {
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
                if meta.mode() & S_IWGRP == 0 { '*' } else { ' ' },
                meta.atime(),
            )),
            Err(_) => Ok(('?', 0)),
        }
    }

    fn print_user_info(&self, ut: &CtUtmpx) {
        print!("{:<8}", ut.user());
    }

    fn print_fullname(&self, ut: &CtUtmpx) {
        if !self.is_include_fullname {
            return;
        }
        let fullname = CtPasswd::locate(ut.user().as_ref())
            .ok()
            .and_then(|pw| gecos_to_fullname(&pw))
            .unwrap_or_else(|| "???".to_string());
        print!(" {:<19}", fullname);
    }

    fn print_tty_info(&self, ut: &CtUtmpx, mesg: char) {
        print!(" {}{:<8}", mesg, ut.tty_device());
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

    /// 打印列标题，使用固定格式匹配coreutils
    fn print_heading(&self) {
        if !self.is_include_heading {
            return;
        }

        // 使用与coreutils相同的固定格式
        print!("{:<8}", "Login");
        if self.is_include_fullname {
            print!(" {:<19}", "Name");
        }
        print!(" {:<9}", " TTY"); // 注意：包含前导空格，总宽度9
        if self.is_include_idle {
            print!(" {:<6}", "Idle");
        }
        print!(" {:<width$}", "When", width = time_format_width());
        if self.is_include_where {
            print!(" Where");
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

#[cfg(test)]
mod tests {
    use super::*;

    mod capitalize_tests {
        use super::*;

        #[test]
        fn test_capitalize_empty_string() {
            assert_eq!("".capitalize(), "");
        }

        #[test]
        fn test_capitalize_single_char() {
            assert_eq!("a".capitalize(), "A");
            assert_eq!("Z".capitalize(), "Z");
        }

        #[test]
        fn test_capitalize_word() {
            assert_eq!("hello".capitalize(), "Hello");
            assert_eq!("world".capitalize(), "World");
        }

        #[test]
        fn test_capitalize_already_capitalized() {
            assert_eq!("Hello".capitalize(), "Hello");
            assert_eq!("WORLD".capitalize(), "WORLD");
        }

        #[test]
        fn test_capitalize_with_spaces() {
            assert_eq!("hello world".capitalize(), "Hello world");
            assert_eq!(" hello".capitalize(), " hello");
        }

        #[test]
        fn test_capitalize_with_special_chars() {
            assert_eq!("123abc".capitalize(), "123abc");
            assert_eq!("!hello".capitalize(), "!hello");
        }
    }
}

#[cfg(test)]
mod tests_all {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    mod time_format_tests {
        use super::*;

        #[test]
        fn test_time_format_width_basic() {
            // 由于时间格式宽度依赖于实际的环境变量，
            // 我们只测试函数能正常返回合理的值
            let width = time_format_width();
            assert!(
                width == 12 || width == 16,
                "time_format_width should return 12 or 16, got {}",
                width
            );
        }
    }

    mod idle_string_tests {
        use super::*;

        #[test]
        fn test_idle_string_less_than_minute() {
            assert_eq!(
                pinky_idle_string(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs() as i64
                ),
                "     "
            );
        }

        #[test]
        fn test_idle_string_hours_minutes() {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;
            assert_eq!(pinky_idle_string(now - 3665), "01:01"); // 1 hour 1 minute
        }

        #[test]
        fn test_idle_string_days() {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;
            assert_eq!(pinky_idle_string(now - 172800), "2d"); // 2 days
        }
    }

    mod gecos_tests {
        use super::*;

        #[test]
        fn test_gecos_to_fullname_no_info() {
            let pw = CtPasswd {
                user_info: None,
                ..Default::default()
            };
            assert_eq!(gecos_to_fullname(&pw), None);
        }

        #[test]
        fn test_gecos_to_fullname_with_comma() {
            let pw = CtPasswd {
                name: "test".to_string(),
                user_info: Some("Test User,Other Info".to_string()),
                ..Default::default()
            };
            assert_eq!(gecos_to_fullname(&pw), Some("Test User".to_string()));
        }

        #[test]
        fn test_gecos_to_fullname_with_ampersand() {
            let pw = CtPasswd {
                name: "test".to_string(),
                user_info: Some("& User".to_string()),
                ..Default::default()
            };
            assert_eq!(gecos_to_fullname(&pw), Some("Test User".to_string()));
        }
    }
}

#[cfg(test)]
mod tests_tool_implementation {
    use crate::Pinky;
    use ctcore::Tool;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Pinky::default();

        // 测试 name 方法
        assert_eq!(tool.name(), "pinky");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("pinky"));

        // 测试 execute 方法
        let args = vec![OsString::from("pinky"), OsString::from("--help")];
        assert!(tool.execute(&args).is_err()); // --help参数通常会返回错误
    }
}
