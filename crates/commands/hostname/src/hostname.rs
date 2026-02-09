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

extern crate rust_i18n;
use clap::crate_version;
use clap::{Arg, ArgAction, Command};
use ctcore::Tool;
use ctcore::ct_error::{CTResult, ExitCode};
use nix::ifaddrs::getifaddrs;
use nix::libc;
use nix::net::if_::InterfaceFlags;
use nix::sys::socket::{AddressFamily, SockaddrLike};
use rust_i18n::t;
use std::ffi::{CStr, CString, OsString};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::os::raw::c_char;
use sys_locale::get_locale;

rust_i18n::i18n!("locales", fallback = "en-US");
mod opt_flags {
    pub const ALIAS: &str = "alias";
    pub const ALL_FQDNS: &str = "all-fqdns";
    pub const BOOT: &str = "boot";
    pub const DOMAIN: &str = "domain";
    pub const FQDN: &str = "fqdn";
    pub const FILE: &str = "file";
    pub const IP_ADDRESS: &str = "ip-address";
    pub const ALL_IP_ADDRESSES: &str = "all-ip-addresses";
    pub const SHORT: &str = "short";
    pub const NIS: &str = "nis";
    pub const HELP_Q: &str = "help-question";
    pub const NAME: &str = "name";
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NameType {
    Default,
    Dns,
    Fqdn,
    Short,
    Alias,
    Ip,
    Nis,
    NisDef,
    AllFqdns,
    AllIps,
}

#[derive(Default)]
pub struct Hostname;

fn execute_hostname(args: &[OsString]) -> CTResult<()> {
    hostname_main(args.iter().cloned())
}

impl Tool for Hostname {
    fn name(&self) -> &'static str {
        "hostname"
    }

    fn command(&self) -> Command {
        hostname_app("hostname")
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        execute_hostname(args)
    }
}

#[derive(Default)]
pub struct Dnsdomainname;

impl Tool for Dnsdomainname {
    fn name(&self) -> &'static str {
        "dnsdomainname"
    }

    fn command(&self) -> Command {
        hostname_app("dnsdomainname")
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        execute_hostname(args)
    }
}

#[derive(Default)]
pub struct Domainname;

impl Tool for Domainname {
    fn name(&self) -> &'static str {
        "domainname"
    }

    fn command(&self) -> Command {
        hostname_app("domainname")
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        execute_hostname(args)
    }
}

#[derive(Default)]
pub struct Nisdomainname;

impl Tool for Nisdomainname {
    fn name(&self) -> &'static str {
        "nisdomainname"
    }

    fn command(&self) -> Command {
        hostname_app("nisdomainname")
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        execute_hostname(args)
    }
}

#[derive(Default)]
pub struct Ypdomainname;

impl Tool for Ypdomainname {
    fn name(&self) -> &'static str {
        "ypdomainname"
    }

    fn command(&self) -> Command {
        hostname_app("ypdomainname")
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        execute_hostname(args)
    }
}

pub fn hostname_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);

    #[cfg(not(unix))]
    {
        let _ = args;
        return Err(ExitCode(1).into());
    }

    #[cfg(unix)]
    {
        let argv: Vec<OsString> = args.collect();
        hostname_main_unix(&argv)
    }
}

#[cfg(unix)]
fn hostname_main_unix(argv: &[OsString]) -> CTResult<()> {
    let progname = program_name(argv);
    let matches = match hostname_app("hostname").try_get_matches_from(argv.iter().cloned()) {
        Ok(m) => m,
        Err(err) => return handle_parse_error(&progname, argv, err),
    };

    if matches.get_flag(opt_flags::HELP_Q) {
        let mut cmd = hostname_app("hostname");
        print!("{}", cmd.render_help());
        return Ok(());
    }

    let boot = matches.get_flag(opt_flags::BOOT);
    let file_arg = matches.get_one::<String>(opt_flags::FILE).cloned();
    let mut operands: Vec<String> = matches
        .get_many::<String>(opt_flags::NAME)
        .map(|v| v.map(ToString::to_string).collect())
        .unwrap_or_default();
    let name_type = resolve_name_type(default_type(&progname), argv);

    let file_provided = file_arg.is_some();
    let mut name = if let Some(file) = file_arg {
        read_name_from_file(&progname, &file, boot)?
    } else {
        None
    };

    if file_provided && boot && name.as_deref().is_none_or(str::is_empty) {
        let mut local = local_host_name(&progname)?;
        if local.is_empty() || local == "(none)" {
            local = "localhost".to_string();
        }
        name = Some(local);
    }

    if !operands.is_empty() {
        if name.is_some() {
            return usage(&progname, false);
        }
        name = Some(operands.remove(0));
    }
    if !operands.is_empty() {
        return usage(&progname, false);
    }

    if let Some(value) = name {
        set_name(&progname, name_type, &value)
    } else {
        show_name(&progname, name_type)
    }
}

#[cfg(unix)]
fn handle_parse_error(progname: &str, argv: &[OsString], err: clap::Error) -> CTResult<()> {
    match err.kind() {
        clap::error::ErrorKind::DisplayHelp => {
            print!("{err}");
            return Err(ExitCode(255).into());
        }
        clap::error::ErrorKind::DisplayVersion => {
            print!("{err}");
            return Ok(());
        }
        _ => {}
    }

    if emit_getopt_style_error(progname, argv) {
        return usage(progname, true);
    }

    eprint!("{err}");
    Err(ExitCode(255).into())
}

#[cfg(unix)]
fn emit_getopt_style_error(progname: &str, argv: &[OsString]) -> bool {
    let mut i = 1usize;
    while i < argv.len() {
        let arg = argv[i].to_string_lossy().into_owned();

        if arg == "--" {
            return false;
        }

        if arg.starts_with("--") && arg.len() > 2 {
            let long = &arg[2..];
            let (name, inline_val) = match long.split_once('=') {
                Some((n, v)) => (n, Some(v)),
                None => (long, None),
            };

            match name {
                "domain" | "boot" | "fqdn" | "all-fqdns" | "help" | "long" | "short"
                | "version" | "alias" | "ip-address" | "all-ip-addresses" | "nis" | "yp" => {
                    if inline_val.is_some() {
                        eprintln!(
                            "{}",
                            t!(
                                "hostname.messages.option_no_argument",
                                progname = progname,
                                name = name
                            )
                        );
                        return true;
                    }
                }
                "file" => {
                    if inline_val.is_none() && i + 1 >= argv.len() {
                        eprintln!(
                            "{}",
                            t!(
                                "hostname.messages.option_file_requires_arg",
                                progname = progname
                            )
                        );
                        return true;
                    }
                    if inline_val.is_none() {
                        i += 1;
                    }
                }
                _ => {
                    eprintln!(
                        "{}",
                        t!(
                            "hostname.messages.unrecognized_option",
                            progname = progname,
                            name = name
                        )
                    );
                    return true;
                }
            }

            i += 1;
            continue;
        }

        if arg.starts_with('-') && arg.len() > 1 {
            let mut chars = arg[1..].chars().peekable();
            while let Some(ch) = chars.next() {
                match ch {
                    'a' | 'A' | 'd' | 'f' | 'b' | 'h' | '?' | 'i' | 'I' | 's' | 'V' | 'y' => {}
                    'F' => {
                        let rest: String = chars.collect();
                        if rest.is_empty() && i + 1 >= argv.len() {
                            eprintln!(
                                "{}",
                                t!(
                                    "hostname.messages.option_requires_arg_short",
                                    progname = progname,
                                    ch = "F"
                                )
                            );
                            return true;
                        }
                        if rest.is_empty() {
                            i += 1;
                        }
                        break;
                    }
                    _ => {
                        eprintln!(
                            "{}",
                            t!(
                                "hostname.messages.invalid_option_short",
                                progname = progname,
                                ch = ch.to_string()
                            )
                        );
                        return true;
                    }
                }
            }

            i += 1;
            continue;
        }

        i += 1;
    }

    false
}

#[cfg(unix)]
fn resolve_name_type(default: NameType, argv: &[OsString]) -> NameType {
    let mut name_type = default;
    let mut i = 1usize;

    while i < argv.len() {
        let arg = argv[i].to_string_lossy().into_owned();

        if arg == "--" {
            break;
        }

        if arg.starts_with("--") && arg.len() > 2 {
            let long = &arg[2..];
            let (name, inline_val) = match long.split_once('=') {
                Some((n, _)) => (n, true),
                None => (long, false),
            };

            match name {
                "domain" => name_type = NameType::Dns,
                "fqdn" | "long" => name_type = NameType::Fqdn,
                "all-fqdns" => name_type = NameType::AllFqdns,
                "short" => name_type = NameType::Short,
                "alias" => name_type = NameType::Alias,
                "ip-address" => name_type = NameType::Ip,
                "all-ip-addresses" => name_type = NameType::AllIps,
                "nis" | "yp" => name_type = NameType::NisDef,
                "file" if !inline_val => i += 1,
                _ => {}
            }

            i += 1;
            continue;
        }

        if arg.starts_with('-') && arg.len() > 1 {
            let mut chars = arg[1..].chars().peekable();
            while let Some(ch) = chars.next() {
                match ch {
                    'd' => name_type = NameType::Dns,
                    'a' => name_type = NameType::Alias,
                    'f' => name_type = NameType::Fqdn,
                    'A' => name_type = NameType::AllFqdns,
                    'i' => name_type = NameType::Ip,
                    'I' => name_type = NameType::AllIps,
                    's' => name_type = NameType::Short,
                    'y' => name_type = NameType::NisDef,
                    'F' => {
                        let rest: String = chars.collect();
                        if rest.is_empty() && i + 1 < argv.len() {
                            i += 1;
                        }
                        break;
                    }
                    _ => {}
                }
            }
        }

        i += 1;
    }

    name_type
}

fn hostname_app(name: &'static str) -> Command {
    let command_version = crate_version!();
    Command::new(name)
        .about(t!("hostname.about"))
        .after_help(t!("hostname.after_help"))
        .version(command_version)
        .override_usage(t!("hostname.usage"))
        .infer_long_args(true)
        .arg(
            Arg::new(opt_flags::ALIAS)
                .short('a')
                .long(opt_flags::ALIAS)
                .help(t!("hostname.clap.opt_alias"))
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new(opt_flags::ALL_FQDNS)
                .short('A')
                .long(opt_flags::ALL_FQDNS)
                .help(t!("hostname.clap.opt_all_fqdns"))
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new(opt_flags::BOOT)
                .short('b')
                .long(opt_flags::BOOT)
                .help(t!("hostname.clap.opt_boot"))
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new(opt_flags::DOMAIN)
                .short('d')
                .long(opt_flags::DOMAIN)
                .help(t!("hostname.clap.opt_domain"))
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new(opt_flags::FQDN)
                .short('f')
                .long(opt_flags::FQDN)
                .visible_alias("long")
                .help(t!("hostname.clap.opt_fqdn"))
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new(opt_flags::FILE)
                .short('F')
                .long(opt_flags::FILE)
                .help(t!("hostname.clap.opt_file")),
        )
        .arg(
            Arg::new(opt_flags::IP_ADDRESS)
                .short('i')
                .long(opt_flags::IP_ADDRESS)
                .help(t!("hostname.clap.opt_ip_address"))
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new(opt_flags::ALL_IP_ADDRESSES)
                .short('I')
                .long(opt_flags::ALL_IP_ADDRESSES)
                .help(t!("hostname.clap.opt_all_ip_addresses"))
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new(opt_flags::SHORT)
                .short('s')
                .long(opt_flags::SHORT)
                .help(t!("hostname.clap.opt_short"))
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new(opt_flags::NIS)
                .short('y')
                .long(opt_flags::NIS)
                .visible_alias("yp")
                .help(t!("hostname.clap.opt_nis"))
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new(opt_flags::HELP_Q)
                .short('?')
                .hide(true)
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new(opt_flags::NAME)
                .hide(true)
                .action(ArgAction::Append),
        )
}

fn usage(_progname: &str, to_stdout: bool) -> CTResult<()> {
    let mut cmd = hostname_app("hostname");
    let help = cmd.render_help().to_string();
    if to_stdout {
        print!("{help}");
    } else {
        eprint!("{help}");
    }
    Err(ExitCode(255).into())
}

fn default_type(progname: &str) -> NameType {
    match progname {
        "dnsdomainname" => NameType::Dns,
        "domainname" => NameType::Nis,
        "ypdomainname" | "nisdomainname" => NameType::NisDef,
        _ => NameType::Default,
    }
}

fn program_name(argv: &[OsString]) -> String {
    argv.first()
        .map(|x| x.to_string_lossy().into_owned())
        .and_then(|x| x.rsplit('/').next().map(std::string::ToString::to_string))
        .filter(|x| !x.is_empty())
        .unwrap_or_else(|| "hostname".to_string())
}

type CtErrBox = Box<dyn ctcore::ct_error::CTError>;

fn exit_with_code<T>(code: i32) -> Result<T, CtErrBox> {
    Err(ExitCode(code).into())
}

fn print_io_error<T>(progname: &str, err: &std::io::Error) -> Result<T, CtErrBox> {
    let mut msg = err.to_string();
    if let Some(pos) = msg.find(" (os error ") {
        msg.truncate(pos);
    }
    eprintln!(
        "{}",
        t!("hostname.messages.io_error", progname = progname, msg = msg)
    );
    exit_with_code(1)
}

fn c_string(input: &str, progname: &str) -> Result<CString, CtErrBox> {
    match CString::new(input) {
        Ok(s) => Ok(s),
        Err(_) => {
            eprintln!(
                "{}",
                t!(
                    "hostname.messages.specified_hostname_invalid",
                    progname = progname
                )
            );
            Err(ExitCode(1).into())
        }
    }
}

fn local_host_name(progname: &str) -> CTResult<String> {
    let mut size = 128usize;
    loop {
        let mut buf = vec![0u8; size];
        let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast::<c_char>(), buf.len()) };
        if rc == 0 {
            if let Some(pos) = buf.iter().position(|b| *b == 0) {
                return Ok(String::from_utf8_lossy(&buf[..pos]).into_owned());
            }
            size *= 2;
            continue;
        }

        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ENAMETOOLONG) {
            size *= 2;
            continue;
        }
        return print_io_error(progname, &err);
    }
}

fn local_domain_name(progname: &str) -> CTResult<String> {
    let mut size = 128usize;
    loop {
        let mut buf = vec![0u8; size];
        let rc = unsafe { libc::getdomainname(buf.as_mut_ptr().cast::<c_char>(), buf.len()) };
        if rc == 0 {
            if let Some(pos) = buf.iter().position(|b| *b == 0) {
                return Ok(String::from_utf8_lossy(&buf[..pos]).into_owned());
            }
            size *= 2;
            continue;
        }

        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ENAMETOOLONG) {
            size *= 2;
            continue;
        }
        return print_io_error(progname, &err);
    }
}

fn local_nis_domain_name(progname: &str) -> CTResult<String> {
    let domain = local_domain_name(progname)?;
    if domain == "(none)" {
        println!(
            "{}",
            t!(
                "hostname.messages.local_domain_not_set",
                progname = progname
            )
        );
        return exit_with_code(1);
    }
    Ok(domain)
}

fn check_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    if !bytes[0].is_ascii_alphanumeric() || !bytes[bytes.len() - 1].is_ascii_alphanumeric() {
        return false;
    }

    for (idx, ch) in bytes.iter().enumerate() {
        let allowed = ch.is_ascii_alphanumeric() || *ch == b'-' || *ch == b'.';
        if !allowed {
            return false;
        }
        if *ch == b'-' {
            if idx > 0 && bytes[idx - 1] == b'.' {
                return false;
            }
            if idx + 1 < bytes.len() && bytes[idx + 1] == b'.' {
                return false;
            }
        }
        if *ch == b'.' && idx > 0 && bytes[idx - 1] == b'.' {
            return false;
        }
    }

    true
}

fn set_name(progname: &str, name_type: NameType, original_name: &str) -> CTResult<()> {
    match name_type {
        NameType::Default => {
            let trimmed = original_name.trim_matches(char::is_whitespace).to_string();
            if !check_name(&trimmed) {
                eprintln!(
                    "{}",
                    t!(
                        "hostname.messages.specified_hostname_invalid",
                        progname = progname
                    )
                );
                return exit_with_code(1);
            }

            let raw = c_string(&trimmed, progname)?;
            let rc = unsafe { libc::sethostname(raw.as_ptr(), trimmed.len()) };
            if rc != 0 {
                match std::io::Error::last_os_error().raw_os_error() {
                    Some(libc::EPERM) => {
                        eprintln!(
                            "{}",
                            t!(
                                "hostname.messages.must_be_root_change_host",
                                progname = progname
                            )
                        );
                    }
                    Some(libc::EINVAL) => {
                        eprintln!(
                            "{}",
                            t!("hostname.messages.name_too_long", progname = progname)
                        );
                    }
                    _ => {}
                }
                return exit_with_code(1);
            }
            Ok(())
        }
        NameType::Nis | NameType::NisDef => {
            let raw = c_string(original_name, progname)?;
            let rc = unsafe { libc::setdomainname(raw.as_ptr(), original_name.len()) };
            if rc != 0 {
                match std::io::Error::last_os_error().raw_os_error() {
                    Some(libc::EPERM) => {
                        eprintln!(
                            "{}",
                            t!(
                                "hostname.messages.must_be_root_change_domain",
                                progname = progname
                            )
                        );
                    }
                    Some(libc::EINVAL) => {
                        eprintln!(
                            "{}",
                            t!("hostname.messages.name_too_long", progname = progname)
                        );
                    }
                    _ => {}
                }
                return exit_with_code(1);
            }
            Ok(())
        }
        _ => usage(progname, false),
    }
}

fn gai_error(code: i32) -> String {
    unsafe { CStr::from_ptr(libc::gai_strerror(code)) }
        .to_string_lossy()
        .into_owned()
}

fn show_all_ifaddrs(progname: &str, as_ips: bool) -> CTResult<()> {
    let flags = if as_ips {
        libc::NI_NUMERICHOST
    } else {
        libc::NI_NAMEREQD
    };

    let ifaddrs = match getifaddrs() {
        Ok(v) => v,
        Err(e) => {
            let io_err = std::io::Error::from_raw_os_error(e as i32);
            return print_io_error(progname, &io_err);
        }
    };

    for iface in ifaddrs {
        let Some(addr) = iface.address else {
            continue;
        };

        if iface.flags.contains(InterfaceFlags::IFF_LOOPBACK) {
            continue;
        }
        if !iface.flags.contains(InterfaceFlags::IFF_UP) {
            continue;
        }

        let family = addr.family();
        if family != Some(AddressFamily::Inet) && family != Some(AddressFamily::Inet6) {
            continue;
        }

        if let Some(in6) = addr.as_sockaddr_in6() {
            let ip = in6.ip();
            let first_seg = ip.segments()[0];
            let multicast_link_local = (first_seg & 0xff0f) == 0xff02;
            if ip.is_unicast_link_local() || multicast_link_local {
                continue;
            }
        }

        let mut host = [0 as c_char; libc::NI_MAXHOST as usize];
        let ret = unsafe {
            libc::getnameinfo(
                addr.as_ptr(),
                addr.len(),
                host.as_mut_ptr(),
                host.len() as libc::socklen_t,
                std::ptr::null_mut(),
                0,
                flags,
            )
        };

        if ret != 0 {
            if as_ips && ret != libc::EAI_NONAME {
                eprintln!(
                    "{}",
                    t!(
                        "hostname.messages.gai_error",
                        progname = progname,
                        msg = gai_error(ret)
                    )
                );
                return exit_with_code(1);
            }
            continue;
        }

        let value = unsafe { CStr::from_ptr(host.as_ptr()) }.to_string_lossy();
        print!("{value} ");
    }

    println!();
    Ok(())
}

fn show_resolved_name(progname: &str, name_type: NameType) -> CTResult<()> {
    let host = local_host_name(progname)?;
    let host_c = c_string(&host, progname)?;

    let mut hints: libc::addrinfo = unsafe { std::mem::zeroed() };
    hints.ai_socktype = libc::SOCK_DGRAM;
    hints.ai_flags = libc::AI_CANONNAME;

    let mut res: *mut libc::addrinfo = std::ptr::null_mut();
    let ret = unsafe { libc::getaddrinfo(host_c.as_ptr(), std::ptr::null(), &hints, &mut res) };
    if ret != 0 {
        eprintln!(
            "{}",
            t!(
                "hostname.messages.gai_error",
                progname = progname,
                msg = gai_error(ret)
            )
        );
        return exit_with_code(1);
    }

    struct AddrInfoGuard(*mut libc::addrinfo);
    impl Drop for AddrInfoGuard {
        fn drop(&mut self) {
            unsafe {
                if !self.0.is_null() {
                    libc::freeaddrinfo(self.0);
                }
            }
        }
    }
    let _guard = AddrInfoGuard(res);

    if res.is_null() {
        return Ok(());
    }

    let canon = unsafe {
        if (*res).ai_canonname.is_null() {
            String::new()
        } else {
            CStr::from_ptr((*res).ai_canonname)
                .to_string_lossy()
                .into_owned()
        }
    };

    match name_type {
        NameType::Alias => {
            let content = match std::fs::read_to_string("/etc/hosts") {
                Ok(c) => c,
                Err(e) => return print_io_error(progname, &e),
            };

            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                let fields: Vec<&str> = trimmed.split_whitespace().collect();
                if fields.len() < 2 {
                    continue;
                }
                if !fields[1..].contains(&host.as_str()) {
                    continue;
                }

                let aliases: Vec<&str> = fields[1..]
                    .iter()
                    .copied()
                    .filter(|name| *name != host)
                    .collect();
                if !aliases.is_empty() {
                    print!("{}", aliases.join(" "));
                }
                println!();
                return Ok(());
            }

            println!();
            Ok(())
        }
        NameType::Ip => {
            let mut first = true;
            let mut cur = res;
            while !cur.is_null() {
                let mut buf = [0 as c_char; 46];
                let rc = unsafe {
                    libc::getnameinfo(
                        (*cur).ai_addr,
                        (*cur).ai_addrlen,
                        buf.as_mut_ptr(),
                        buf.len() as libc::socklen_t,
                        std::ptr::null_mut(),
                        0,
                        libc::NI_NUMERICHOST,
                    )
                };
                if rc != 0 {
                    eprintln!(
                        "{}",
                        t!(
                            "hostname.messages.gai_error",
                            progname = progname,
                            msg = gai_error(rc)
                        )
                    );
                    return exit_with_code(1);
                }
                if !first {
                    print!(" ");
                }
                first = false;
                let value = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy();
                print!("{value}");
                cur = unsafe { (*cur).ai_next };
            }
            println!();
            Ok(())
        }
        NameType::Dns => {
            if let Some(pos) = canon.find('.') {
                println!("{}", &canon[pos + 1..]);
            }
            Ok(())
        }
        NameType::Fqdn => {
            println!("{canon}");
            Ok(())
        }
        _ => Ok(()),
    }
}

fn show_name(progname: &str, name_type: NameType) -> CTResult<()> {
    match name_type {
        NameType::Default => {
            println!("{}", local_host_name(progname)?);
            Ok(())
        }
        NameType::Short => {
            let host = local_host_name(progname)?;
            if let Some(pos) = host.find('.') {
                println!("{}", &host[..pos]);
            } else {
                println!("{host}");
            }
            Ok(())
        }
        NameType::Nis => {
            println!("{}", local_domain_name(progname)?);
            Ok(())
        }
        NameType::NisDef => {
            println!("{}", local_nis_domain_name(progname)?);
            Ok(())
        }
        NameType::AllIps => show_all_ifaddrs(progname, true),
        NameType::AllFqdns => show_all_ifaddrs(progname, false),
        NameType::Alias | NameType::Ip | NameType::Dns | NameType::Fqdn => {
            show_resolved_name(progname, name_type)
        }
    }
}

fn read_name_from_file(progname: &str, path: &str, boot: bool) -> CTResult<Option<String>> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) => {
            if boot {
                return Ok(None);
            }
            return print_io_error(progname, &e);
        }
    };

    let mut reader = BufReader::new(file);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => return Ok(Some(String::new())),
            Ok(_) => {
                if line.starts_with('\n') || line.starts_with('#') {
                    continue;
                }
                if line.ends_with('\n') {
                    line.pop();
                }
                return Ok(Some(line.clone()));
            }
            Err(e) => return print_io_error(progname, &e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{NameType, check_name, default_type};

    #[test]
    fn test_check_name() {
        assert!(check_name("host"));
        assert!(check_name("host.example"));
        assert!(check_name("host-1"));
        assert!(!check_name(""));
        assert!(!check_name("-host"));
        assert!(!check_name("host-"));
        assert!(!check_name("host..example"));
        assert!(!check_name("host.-example"));
        assert!(!check_name("host#.example"));
    }

    #[test]
    fn test_default_type_from_progname() {
        assert_eq!(default_type("hostname"), NameType::Default);
        assert_eq!(default_type("dnsdomainname"), NameType::Dns);
        assert_eq!(default_type("domainname"), NameType::Nis);
        assert_eq!(default_type("ypdomainname"), NameType::NisDef);
        assert_eq!(default_type("nisdomainname"), NameType::NisDef);
    }
}
