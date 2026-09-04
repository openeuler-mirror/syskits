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
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::Tool;
use ctcore::ct_entries::CtPasswd;
use ctcore::ct_error::{CTResult, CTsageError, FromIo};
use ctcore::ct_locale::hard_locale_time;
use ctcore::ct_utmpx::{self, CtUtmpx, time};
use ctcore::libc::S_IWGRP;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::prelude::*;
use std::io::{self, BufReader};
use std::os::unix::ffi::OsStrExt;
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
            .overrides_with(pinky_options::PINKY_SHORT_FORMAT)
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
            .overrides_with(pinky_options::PINKY_LONG_FORMAT)
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
            .value_parser(clap::builder::OsStringValueParser::new())
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
        .args_override_self(true)
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
    let stdout = io::stdout();
    let mut output = stdout.lock();
    pinky_main_with_writer(args, &mut output)?;
    output
        .flush()
        .map_err_context(|| String::from("write error"))
}

fn pinky_main_with_writer<W: Write>(args: impl ctcore::Args, output: &mut W) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let matches = ct_app()
        .after_help(get_long_usage())
        .try_get_matches_from(pinky_args(args))?;

    let pk = PinkyFlags::new(&matches);
    let do_short_format = !matches.get_flag(pinky_options::PINKY_LONG_FORMAT);
    if !do_short_format && pk.pinky_names.is_empty() {
        return Err(CTsageError::new(
            1,
            t!("pinky.output.missing_username").to_string(),
        ));
    }

    if do_short_format {
        pk.short_pinky(output)
            .map_err_context(|| String::from("write error"))
    } else {
        pk.long_pinky(output)
            .map_err_context(|| String::from("write error"))
    }
}

pub fn pinky_native_semantic(args: impl ctcore::Args) -> CTResult<PinkySemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let matches = ct_app()
        .after_help(get_long_usage())
        .try_get_matches_from(pinky_args(args))?;

    let flags = PinkyFlags::new(&matches);
    let do_short_format = !matches.get_flag(pinky_options::PINKY_LONG_FORMAT);

    if do_short_format {
        flags.short_semantic().map_err(|e| e.into())
    } else {
        Ok(flags.long_semantic())
    }
}

fn pinky_args(args: impl ctcore::Args) -> Vec<OsString> {
    let mut args: Vec<OsString> = args.collect();
    if std::env::var_os("POSIXLY_CORRECT").is_some() {
        for index in 1..args.len() {
            let bytes = args[index].as_encoded_bytes();
            if bytes == b"--" {
                break;
            }
            if bytes.is_empty() || bytes == b"-" || bytes[0] != b'-' {
                args.insert(index, OsString::from("--"));
                break;
            }
        }
    }
    args
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinkyRow {
    pub kind: String,
    pub user: String,
    pub full_name: Option<String>,
    pub tty_device: Option<String>,
    pub mesg: Option<String>,
    pub idle: Option<String>,
    pub login_time: Option<String>,
    pub host: Option<String>,
    pub home_dir: Option<String>,
    pub shell: Option<String>,
    pub project_text: Option<String>,
    pub plan_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinkySemantic {
    pub view_kind: String,
    pub rows: Vec<PinkyRow>,
    pub classic_text: String,
    pub stderr_text: String,
    pub exit_code: i32,
}

struct PinkyFlags {
    is_include_idle: bool,
    is_include_heading: bool,
    is_include_fullname: bool,
    is_include_project: bool,
    is_include_plan: bool,
    is_include_where: bool,
    is_include_home_and_shell: bool,
    pinky_names: Vec<OsString>,
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
    format_login_time(ut.timestamp_seconds())
}

fn format_login_time(seconds: i64) -> String {
    let timestamp = seconds as ctcore::libc::time_t;
    let mut local_time = std::mem::MaybeUninit::<ctcore::libc::tm>::uninit();
    let local_time = unsafe {
        let result = ctcore::libc::localtime_r(&timestamp, local_time.as_mut_ptr());
        if result.is_null() {
            return seconds.to_string();
        }
        local_time.assume_init()
    };

    let format = if hard_locale_time() {
        b"%Y-%m-%d %H:%M\0".as_slice()
    } else {
        b"%b %e %H:%M\0".as_slice()
    };
    let mut output = [0_u8; 64];
    let length = unsafe {
        ctcore::libc::strftime(
            output.as_mut_ptr().cast(),
            output.len(),
            format.as_ptr().cast(),
            &local_time,
        )
    };
    String::from_utf8_lossy(&output[..length]).into_owned()
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
    gecos_to_fullname_bytes(pw).map(|name| String::from_utf8_lossy(&name).into_owned())
}

fn gecos_to_fullname_bytes(pw: &CtPasswd) -> Option<Vec<u8>> {
    let gecos = pw.user_info_bytes()?;
    let gecos = gecos
        .iter()
        .position(|byte| *byte == b',')
        .map_or(gecos, |position| &gecos[..position]);
    let username = pw.name_bytes();
    let mut capitalized = username.to_vec();
    if let Some(first) = capitalized.first_mut() {
        *first = locale_uppercase(*first);
    }

    let mut fullname = Vec::with_capacity(gecos.len());
    for byte in gecos {
        if *byte == b'&' {
            fullname.extend_from_slice(&capitalized);
        } else {
            fullname.push(*byte);
        }
    }
    Some(fullname)
}

fn locale_uppercase(byte: u8) -> u8 {
    let locale_name = ["LC_ALL", "LC_CTYPE", "LANG"]
        .into_iter()
        .find_map(|name| std::env::var_os(name).filter(|value| !value.is_empty()))
        .unwrap_or_else(|| OsString::from("C"));
    locale_uppercase_in(byte, &locale_name)
}

fn locale_uppercase_in(byte: u8, locale_name: &OsStr) -> u8 {
    let Ok(locale_name) = std::ffi::CString::new(locale_name.as_bytes()) else {
        return byte.to_ascii_uppercase();
    };

    unsafe {
        let locale = ctcore::libc::newlocale(
            ctcore::libc::LC_CTYPE_MASK,
            locale_name.as_ptr(),
            std::ptr::null_mut(),
        );
        if locale.is_null() {
            return byte.to_ascii_uppercase();
        }
        let value = if islower_l(i32::from(byte), locale) != 0 {
            toupper_l(i32::from(byte), locale) as u8
        } else {
            byte
        };
        ctcore::libc::freelocale(locale);
        value
    }
}

unsafe extern "C" {
    fn islower_l(
        character: ctcore::libc::c_int,
        locale: ctcore::libc::locale_t,
    ) -> ctcore::libc::c_int;
    fn toupper_l(
        character: ctcore::libc::c_int,
        locale: ctcore::libc::locale_t,
    ) -> ctcore::libc::c_int;
}

impl PinkyFlags {
    /// 从命令行参数创建新的 Pinky 实例
    fn new(matches: &clap::ArgMatches) -> Self {
        let users: Vec<OsString> = matches
            .get_many::<OsString>(pinky_options::PINKY_USER)
            .map(|values| values.cloned().collect())
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
    fn print_entry<W: Write>(&self, ut: &CtUtmpx, output: &mut W) -> io::Result<()> {
        let (mesg, last_change) = self.get_tty_info(ut)?;
        self.print_user_info(ut, output)?;
        self.print_fullname(ut, output)?;
        self.print_tty_info(ut, mesg, output)?;
        self.print_idle_time(last_change, output)?;
        self.print_login_time(ut, output)?;
        self.print_host_info(ut, output)?;
        writeln!(output)
    }

    fn get_tty_info(&self, ut: &CtUtmpx) -> std::io::Result<(char, i64)> {
        let tty_path = tty_status_path(ut.tty_device_bytes());
        match tty_path.metadata() {
            Ok(meta) => Ok((
                if meta.mode() & S_IWGRP == 0 { '*' } else { ' ' },
                meta.atime(),
            )),
            Err(_) => Ok(('?', 0)),
        }
    }

    fn print_user_info<W: Write>(&self, ut: &CtUtmpx, output: &mut W) -> io::Result<()> {
        write_padded_bytes(output, ut.user_bytes(), 8)
    }

    fn print_fullname<W: Write>(&self, ut: &CtUtmpx, output: &mut W) -> io::Result<()> {
        if !self.is_include_fullname {
            return Ok(());
        }
        let fullname = CtPasswd::locate_name_bytes(ut.user_bytes())
            .ok()
            .and_then(|pw| gecos_to_fullname_bytes(&pw));
        let unknown = t!("pinky.output.unknown_short");
        write_short_fullname(output, fullname.as_deref(), unknown.as_bytes())
    }

    fn print_tty_info<W: Write>(&self, ut: &CtUtmpx, mesg: char, output: &mut W) -> io::Result<()> {
        write!(output, " {mesg}")?;
        write_padded_bytes(output, ut.tty_device_bytes(), 8)
    }

    fn print_idle_time<W: Write>(&self, last_change: i64, output: &mut W) -> io::Result<()> {
        if !self.is_include_idle {
            return Ok(());
        }
        let idle = if last_change == 0 {
            "?????".to_string()
        } else {
            pinky_idle_string(last_change)
        };
        write!(output, " {idle:<6}")
    }

    fn print_login_time<W: Write>(&self, ut: &CtUtmpx, output: &mut W) -> io::Result<()> {
        write!(output, " {}", time_string(ut))
    }

    fn print_host_info<W: Write>(&self, ut: &CtUtmpx, output: &mut W) -> io::Result<()> {
        if !self.is_include_where {
            return Ok(());
        }
        let host = ut.host_bytes();
        if !host.is_empty() {
            output.write_all(b" ")?;
            match std::str::from_utf8(host) {
                Ok(_) => output.write_all(ut.canon_host()?.as_bytes())?,
                Err(_) => output.write_all(host)?,
            }
        }
        Ok(())
    }

    /// 打印列标题，使用固定格式匹配coreutils
    fn print_heading<W: Write>(&self, output: &mut W) -> io::Result<()> {
        if !self.is_include_heading {
            return Ok(());
        }

        // 使用与coreutils相同的固定格式
        write_padded_bytes(output, t!("pinky.output.login").as_bytes(), 8)?;
        if self.is_include_fullname {
            output.write_all(b" ")?;
            write_padded_bytes(output, t!("pinky.output.name").as_bytes(), 19)?;
        }
        output.write_all(b" ")?;
        write_padded_bytes(output, t!("pinky.output.tty").as_bytes(), 9)?;
        if self.is_include_idle {
            output.write_all(b" ")?;
            write_padded_bytes(output, t!("pinky.output.idle").as_bytes(), 6)?;
        }
        output.write_all(b" ")?;
        write_padded_bytes(
            output,
            t!("pinky.output.when").as_bytes(),
            time_format_width(),
        )?;
        if self.is_include_where {
            output.write_all(b" ")?;
            output.write_all(t!("pinky.output.where").as_bytes())?;
        }
        writeln!(output)
    }

    /// 以短格式显示用户信息
    fn short_pinky<W: Write>(&self, output: &mut W) -> io::Result<()> {
        self.print_heading(output)?;

        for ut in CtUtmpx::iter_all_records() {
            if self.should_display_user(&ut) {
                self.print_entry(&ut, output)?;
            }
        }
        Ok(())
    }

    fn should_display_user(&self, ut: &CtUtmpx) -> bool {
        ut.is_user_process()
            && (self.pinky_names.is_empty()
                || self
                    .pinky_names
                    .iter()
                    .any(|name| name.as_bytes() == ut.user_bytes()))
    }

    /// 以长格式显示用户信息
    fn long_pinky<W: Write>(&self, output: &mut W) -> io::Result<()> {
        for username in &self.pinky_names {
            self.print_long_user_info(username, output)?;
        }
        Ok(())
    }

    fn print_long_user_info<W: Write>(&self, username: &OsStr, output: &mut W) -> io::Result<()> {
        output.write_all(t!("pinky.output.login_name").as_bytes())?;
        write_padded_bytes(output, username.as_bytes(), 28)?;
        output.write_all(t!("pinky.output.real_life").as_bytes())?;

        match CtPasswd::locate_name_bytes(username.as_bytes()) {
            Ok(pw) => {
                let fullname = gecos_to_fullname_bytes(&pw).unwrap_or_default();
                let user_dir = pw.user_dir_bytes().unwrap_or_default();
                let user_shell = pw.user_shell_bytes().unwrap_or_default();

                output.write_all(b" ")?;
                output.write_all(&fullname)?;
                output.write_all(b"\n")?;
                self.print_home_and_shell(user_dir, user_shell, output)?;
                self.print_project_file(user_dir, output)?;
                self.print_plan_file(user_dir, output)?;
                writeln!(output)
            }
            Err(_) => {
                output.write_all(b" ")?;
                output.write_all(t!("pinky.output.unknown").as_bytes())?;
                output.write_all(b"\n")
            }
        }
    }

    fn print_home_and_shell<W: Write>(
        &self,
        user_dir: &[u8],
        user_shell: &[u8],
        output: &mut W,
    ) -> io::Result<()> {
        if self.is_include_home_and_shell {
            output.write_all(t!("pinky.output.directory").as_bytes())?;
            write_padded_bytes(output, user_dir, 29)?;
            output.write_all(t!("pinky.output.shell").as_bytes())?;
            output.write_all(b" ")?;
            output.write_all(user_shell)?;
            output.write_all(b"\n")?;
        }
        Ok(())
    }

    fn print_project_file<W: Write>(&self, user_dir: &[u8], output: &mut W) -> io::Result<()> {
        if self.is_include_project {
            if let Ok(f) = File::open(PathBuf::from(OsStr::from_bytes(user_dir)).join(".project")) {
                output.write_all(t!("pinky.output.project").as_bytes())?;
                read_to_console(f, output)?;
            }
        }
        Ok(())
    }

    fn print_plan_file<W: Write>(&self, user_dir: &[u8], output: &mut W) -> io::Result<()> {
        if self.is_include_plan {
            if let Ok(f) = File::open(PathBuf::from(OsStr::from_bytes(user_dir)).join(".plan")) {
                output.write_all(t!("pinky.output.plan").as_bytes())?;
                read_to_console(f, output)?;
            }
        }
        Ok(())
    }

    fn long_profile_row(&self, username: &OsStr) -> PinkyRow {
        let username_lossy = username.to_string_lossy().into_owned();
        match CtPasswd::locate_name_bytes(username.as_bytes()) {
            Ok(pw) => {
                let fullname = gecos_to_fullname(&pw).unwrap_or_default();
                let user_dir = pw.user_dir.unwrap_or_default();
                let user_shell = pw.user_shell.unwrap_or_default();
                let project_text = if self.is_include_project {
                    read_optional_file(PathBuf::from(&user_dir).join(".project"))
                } else {
                    None
                };
                let plan_text = if self.is_include_plan {
                    read_optional_file(PathBuf::from(&user_dir).join(".plan"))
                } else {
                    None
                };

                PinkyRow {
                    kind: "profile".into(),
                    user: username_lossy,
                    full_name: Some(fullname),
                    tty_device: None,
                    mesg: None,
                    idle: None,
                    login_time: None,
                    host: None,
                    home_dir: if self.is_include_home_and_shell {
                        Some(user_dir)
                    } else {
                        None
                    },
                    shell: if self.is_include_home_and_shell {
                        Some(user_shell)
                    } else {
                        None
                    },
                    project_text,
                    plan_text,
                }
            }
            Err(_) => PinkyRow {
                kind: "profile".into(),
                user: username_lossy,
                full_name: None,
                tty_device: None,
                mesg: None,
                idle: None,
                login_time: None,
                host: None,
                home_dir: None,
                shell: None,
                project_text: None,
                plan_text: None,
            },
        }
    }

    fn render_long_profile(&self, row: &PinkyRow) -> String {
        let mut out = String::new();
        out.push_str(&format!("Login name: {:<28}In real life: ", row.user));

        match &row.full_name {
            Some(full_name) => {
                out.push_str(&format!(" {full_name}\n"));
                if self.is_include_home_and_shell {
                    out.push_str(&format!(
                        "Directory: {:<29}Shell:  {}\n",
                        row.home_dir.as_deref().unwrap_or_default(),
                        row.shell.as_deref().unwrap_or_default()
                    ));
                }
                if self.is_include_project
                    && let Some(project_text) = &row.project_text
                {
                    out.push_str("Project: ");
                    out.push_str(project_text);
                }
                if self.is_include_plan
                    && let Some(plan_text) = &row.plan_text
                {
                    out.push_str("Plan:\n");
                    out.push_str(plan_text);
                }
                out.push('\n');
            }
            None => out.push_str(" ???\n"),
        }

        out
    }

    fn long_semantic(&self) -> PinkySemantic {
        let rows = self
            .pinky_names
            .iter()
            .map(|username| self.long_profile_row(username))
            .collect::<Vec<_>>();
        let classic_text = rows
            .iter()
            .map(|row| self.render_long_profile(row))
            .collect::<String>();

        PinkySemantic {
            view_kind: "long".into(),
            rows,
            classic_text,
            stderr_text: String::new(),
            exit_code: 0,
        }
    }

    fn short_session_row(&self, ut: &CtUtmpx) -> std::io::Result<PinkyRow> {
        let (mesg, last_change) = self.get_tty_info(ut)?;
        let full_name = if self.is_include_fullname {
            Some(
                CtPasswd::locate_name_bytes(ut.user_bytes())
                    .ok()
                    .and_then(|pw| gecos_to_fullname(&pw))
                    .unwrap_or_else(|| "???".to_string()),
            )
        } else {
            None
        };
        let idle = if self.is_include_idle {
            Some(if last_change == 0 {
                "?????".to_string()
            } else {
                pinky_idle_string(last_change)
            })
        } else {
            None
        };
        let host = if self.is_include_where {
            let host = ut.host();
            if host.is_empty() {
                None
            } else {
                Some(ut.canon_host()?)
            }
        } else {
            None
        };

        Ok(PinkyRow {
            kind: "session".into(),
            user: ut.user(),
            full_name,
            tty_device: Some(ut.tty_device()),
            mesg: Some(mesg.to_string()),
            idle,
            login_time: Some(time_string(ut)),
            host,
            home_dir: None,
            shell: None,
            project_text: None,
            plan_text: None,
        })
    }

    fn render_short_heading(&self) -> String {
        if !self.is_include_heading {
            return String::new();
        }

        let mut out = String::new();
        out.push_str(&format!("{:<8}", "Login"));
        if self.is_include_fullname {
            out.push_str(&format!(" {:<19}", "Name"));
        }
        out.push_str(&format!(" {:<9}", " TTY"));
        if self.is_include_idle {
            out.push_str(&format!(" {:<6}", "Idle"));
        }
        out.push_str(&format!(" {:<width$}", "When", width = time_format_width()));
        if self.is_include_where {
            out.push_str(" Where");
        }
        out.push('\n');
        out
    }

    fn render_short_row(&self, row: &PinkyRow) -> String {
        let mut out = String::new();
        out.push_str(&format!("{:<8}", row.user));
        if self.is_include_fullname {
            out.push_str(&format!(
                " {:<19}",
                row.full_name.as_deref().unwrap_or("???")
            ));
        }
        out.push_str(&format!(
            " {}{:<8}",
            row.mesg.as_deref().unwrap_or("?"),
            row.tty_device.as_deref().unwrap_or_default()
        ));
        if self.is_include_idle {
            out.push_str(&format!(" {:<6}", row.idle.as_deref().unwrap_or("?????")));
        }
        out.push_str(&format!(
            " {}",
            row.login_time.as_deref().unwrap_or_default()
        ));
        if self.is_include_where
            && let Some(host) = &row.host
        {
            out.push(' ');
            out.push_str(host);
        }
        out.push('\n');
        out
    }

    fn short_semantic(&self) -> std::io::Result<PinkySemantic> {
        let mut rows = Vec::new();
        let mut classic_text = self.render_short_heading();

        for ut in CtUtmpx::iter_all_records() {
            if self.should_display_user(&ut) {
                let row = self.short_session_row(&ut)?;
                classic_text.push_str(&self.render_short_row(&row));
                rows.push(row);
            }
        }

        Ok(PinkySemantic {
            view_kind: "short".into(),
            rows,
            classic_text,
            stderr_text: String::new(),
            exit_code: 0,
        })
    }
}

fn read_optional_file(path: PathBuf) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).to_string())
}

fn write_padded_bytes<W: Write>(output: &mut W, value: &[u8], width: usize) -> io::Result<()> {
    output.write_all(value)?;
    for _ in value.len()..width {
        output.write_all(b" ")?;
    }
    Ok(())
}

fn write_short_fullname<W: Write>(
    output: &mut W,
    fullname: Option<&[u8]>,
    unknown: &[u8],
) -> io::Result<()> {
    output.write_all(b" ")?;
    match fullname {
        Some(fullname) => write_left_field(output, fullname, 19),
        None => write_right_field(output, unknown, 19),
    }
}

fn write_left_field<W: Write>(output: &mut W, value: &[u8], width: usize) -> io::Result<()> {
    let value = &value[..value.len().min(width)];
    output.write_all(value)?;
    for _ in value.len()..width {
        output.write_all(b" ")?;
    }
    Ok(())
}

fn write_right_field<W: Write>(output: &mut W, value: &[u8], width: usize) -> io::Result<()> {
    for _ in value.len()..width {
        output.write_all(b" ")?;
    }
    output.write_all(value)
}

fn tty_status_path(line: &[u8]) -> PathBuf {
    let device = line
        .iter()
        .position(|byte| *byte == b' ')
        .map_or(line, |position| &line[position + 1..]);
    PathBuf::from("/dev").join(OsStr::from_bytes(device))
}

fn read_to_console<F: Read, W: Write>(f: F, output: &mut W) -> io::Result<()> {
    let mut reader = BufReader::new(f);
    let mut buffer = [0_u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(bytes_read) => output.write_all(&buffer[..bytes_read])?,
        }
    }
    Ok(())
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

    #[test]
    fn test_repeated_q_is_idempotent() {
        let matches = ct_app()
            .try_get_matches_from(["pinky", "-q", "-q"])
            .unwrap();
        let flags = PinkyFlags::new(&matches);

        assert!(!flags.is_include_fullname);
        assert!(!flags.is_include_where);
        assert!(!flags.is_include_idle);
    }

    #[test]
    fn test_last_format_option_wins() {
        let short = ct_app()
            .try_get_matches_from(["pinky", "-l", "-s"])
            .unwrap();
        assert!(!short.get_flag(pinky_options::PINKY_LONG_FORMAT));
        assert!(short.get_flag(pinky_options::PINKY_SHORT_FORMAT));

        let long = ct_app()
            .try_get_matches_from(["pinky", "-s", "-l", "root"])
            .unwrap();
        assert!(long.get_flag(pinky_options::PINKY_LONG_FORMAT));
        assert!(!long.get_flag(pinky_options::PINKY_SHORT_FORMAT));
    }

    #[test]
    fn test_profile_files_preserve_bytes() {
        let input = io::Cursor::new(b"profile:\xff\0tail\n");
        let mut output = Vec::new();

        read_to_console(input, &mut output).unwrap();

        assert_eq!(output, b"profile:\xff\0tail\n");
    }

    #[test]
    fn test_short_fullname_uses_gnu_width_and_alignment() {
        let mut known = Vec::new();
        write_short_fullname(
            &mut known,
            Some(b"Alpha User With A Very Long Name"),
            b"        ???",
        )
        .unwrap();
        assert_eq!(known, b" Alpha User With A V");

        let mut unknown = Vec::new();
        write_short_fullname(&mut unknown, None, b"        ???").unwrap();
        assert_eq!(unknown.len(), 20);
        assert_eq!(&unknown[17..], b"???");
        assert!(unknown[..17].iter().all(|byte| *byte == b' '));
    }

    #[test]
    fn test_tty_status_path_uses_device_after_space() {
        assert_eq!(tty_status_path(b"null"), PathBuf::from("/dev/null"));
        assert_eq!(tty_status_path(b"/dev/null"), PathBuf::from("/dev/null"));
        assert_eq!(tty_status_path(b"prefix null"), PathBuf::from("/dev/null"));
    }

    #[test]
    fn test_short_fields_preserve_native_bytes() {
        let mut output = Vec::new();
        write_padded_bytes(&mut output, b"a\xff", 8).unwrap();
        assert_eq!(output, b"a\xff      ");
    }

    mod time_format_tests {
        use super::*;

        #[test]
        fn test_time_format_width_basic() {
            // 由于时间格式宽度依赖于实际的环境变量，
            // 我们只测试函数能正常返回合理的值
            let width = time_format_width();
            assert!(
                width == 12 || width == 16,
                "time_format_width should return 12 or 16, got {width}"
            );
        }

        #[test]
        fn test_unrepresentable_login_time_falls_back_to_epoch() {
            assert_eq!(format_login_time(i64::MAX), i64::MAX.to_string());
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

        #[test]
        fn test_gecos_to_fullname_preserves_native_bytes() {
            let pw = CtPasswd {
                name: "alpha".to_string(),
                user_info: Some("ignored".to_string()),
                raw_name: Some(b"alpha".to_vec()),
                raw_user_info: Some(b"&\xff,ignored".to_vec()),
                ..Default::default()
            };

            assert_eq!(gecos_to_fullname_bytes(&pw), Some(b"Alpha\xff".to_vec()));
        }

        #[test]
        fn test_gecos_capitalization_uses_single_byte_locale() {
            assert_eq!(
                locale_uppercase_in(0xe9, OsStr::new("en_US.iso88591")),
                0xc9
            );
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
        let tool = Pinky;

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
