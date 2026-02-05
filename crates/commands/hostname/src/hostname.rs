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
use rust_i18n::t;
use std::net::ToSocketAddrs;
rust_i18n::i18n!("locales", fallback = "en-US");
use std::str;

use std::collections::hash_set::HashSet;

use std::ffi::OsString;

use clap::builder::ValueParser;
use clap::crate_version;

use clap::Arg;
use clap::ArgAction;
use clap::ArgMatches;
use clap::Command;
use ctcore::ct_error::{CTResult, FromIo};
use sys_locale::get_locale;

use ctcore::Tool;
use nix::sys::socket::{AddressFamily, SockaddrLike};

static OPT_DOMAIN: &str = "domain";
static OPT_IP_ADDRESS: &str = "ip-address";
static OPT_FQDN: &str = "fqdn";
static OPT_SHORT: &str = "short";
static OPT_HOST: &str = "host";
static OPT_ALL_FQDNS: &str = "all-fqdns";
static OPT_FILE: &str = "file";
static OPT_ALIAS: &str = "alias";
static OPT_BOOT: &str = "boot";
static OPT_ALL_IP: &str = "all-ip-addresses";
static OPT_NIS: &str = "nis";

#[cfg(windows)]
mod wsa {
    use std::io;

    use windows_sys::Win32::Networking::WinSock::{WSACleanup, WSADATA, WSAStartup};

    pub(super) struct WsaHandle(());

    pub(super) fn start() -> io::Result<WsaHandle> {
        let err = unsafe {
            let mut data = std::mem::MaybeUninit::<WSADATA>::uninit();
            WSAStartup(0x0202, data.as_mut_ptr())
        };
        if err == 0 {
            Ok(WsaHandle(()))
        } else {
            Err(io::Error::from_raw_os_error(err))
        }
    }

    impl Drop for WsaHandle {
        fn drop(&mut self) {
            unsafe {
                // This possibly returns an error but we can't handle it
                let _err = WSACleanup();
            }
        }
    }
}

pub fn hostname_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let arg_matches = ct_app().try_get_matches_from(args)?;

    #[cfg(windows)]
    let _handle = wsa::start().map_err_context(|| "failed to start Winsock".to_owned())?;

    match arg_matches.get_one::<OsString>(OPT_HOST) {
        None => hostname_display(&arg_matches),
        Some(host) => hostname::set(host).map_err_context(|| "failed to set hostname".to_owned()),
    }
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("hostname.about");
    let usage_description = t!("hostname.usage");

    let args = vec![
        Arg::new(OPT_DOMAIN)
            .short('d')
            .long("domain")
            .overrides_with_all([OPT_DOMAIN, OPT_IP_ADDRESS, OPT_FQDN, OPT_SHORT])
            .help(t!("hostname.clap.opt_domain"))
            .action(ArgAction::SetTrue),
        Arg::new(OPT_IP_ADDRESS)
            .short('i')
            .long("ip-address")
            .overrides_with_all([OPT_DOMAIN, OPT_IP_ADDRESS, OPT_FQDN, OPT_SHORT])
            .help(t!("hostname.clap.opt_ip_address"))
            .action(ArgAction::SetTrue),
        Arg::new(OPT_FQDN)
            .short('f')
            .long("fqdn")
            .overrides_with_all([OPT_DOMAIN, OPT_IP_ADDRESS, OPT_FQDN, OPT_SHORT])
            .help(t!("hostname.clap.opt_fqdn"))
            .action(ArgAction::SetTrue),
        Arg::new(OPT_SHORT)
            .short('s')
            .long("short")
            .overrides_with_all([OPT_DOMAIN, OPT_IP_ADDRESS, OPT_FQDN, OPT_SHORT])
            .help(t!("hostname.clap.opt_short"))
            .action(ArgAction::SetTrue),
        Arg::new(OPT_HOST)
            .value_parser(ValueParser::os_string())
            .value_hint(clap::ValueHint::Hostname),
        Arg::new(OPT_ALL_FQDNS)
            .short('A')
            .long("all-fqdns")
            .help("Display all FQDNs for the host")
            .action(ArgAction::SetTrue),
        Arg::new(OPT_FILE)
            .short('F')
            .long("file")
            .value_name("FILE")
            .help("Read host name or NIS domain name from given file")
            .value_parser(ValueParser::os_string())
            .conflicts_with_all([
                OPT_DOMAIN,
                OPT_IP_ADDRESS,
                OPT_FQDN,
                OPT_SHORT,
                OPT_ALL_FQDNS,
            ]),
        Arg::new(OPT_ALIAS)
            .short('a')
            .long("alias")
            .help("Display alias names")
            .action(ArgAction::SetTrue)
            .conflicts_with_all([
                OPT_DOMAIN,
                OPT_IP_ADDRESS,
                OPT_FQDN,
                OPT_SHORT,
                OPT_ALL_FQDNS,
                OPT_FILE,
            ]),
        Arg::new(OPT_BOOT)
            .short('b')
            .long("boot")
            .help("Set default hostname if none available")
            .value_name("NAME")
            .value_parser(ValueParser::os_string())
            .num_args(0..=1) // 允许0或1个参数
            .default_missing_value("my-host") // 无参数时的默认值
            .conflicts_with_all([
                OPT_DOMAIN,
                OPT_IP_ADDRESS,
                OPT_FQDN,
                OPT_SHORT,
                OPT_ALL_FQDNS,
                OPT_ALIAS,
            ]),
        Arg::new(OPT_ALL_IP)
            .short('I')
            .long("all-ip-addresses")
            .help("Display all addresses for the host")
            .action(ArgAction::SetTrue)
            .conflicts_with_all([
                OPT_DOMAIN,
                OPT_IP_ADDRESS,
                OPT_FQDN,
                OPT_SHORT,
                OPT_ALL_FQDNS,
                OPT_FILE,
                OPT_ALIAS,
                OPT_BOOT,
            ]),
        Arg::new(OPT_NIS)
            .short('y')
            .long("yp")
            .alias("nis")
            .help("Display the NIS/YP domain name")
            .action(ArgAction::SetTrue)
            .conflicts_with_all([
                OPT_IP_ADDRESS,
                OPT_FQDN,
                OPT_SHORT,
                OPT_ALL_FQDNS,
                OPT_ALIAS,
                OPT_BOOT,
                OPT_ALL_IP,
            ]),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(&args)
}
/**
 * 显示主机名，根据命令行参数的不同，可以显示完整的主机名、短主机名、域名或IP地址。
 *
 * @param matches 命令行参数匹配结果，用于确定要显示哪种信息。
 * @return CTResult<()>，操作成功返回Ok(())，失败返回Err()。
 */
fn hostname_display(args_match: &ArgMatches) -> CTResult<()> {
    // 获取当前主机的主机名
    let hostname = hostname::get()
        .map_err_context(|| "failed to get hostname".to_owned())?
        .to_string_lossy()
        .into_owned();

    if args_match.get_flag(OPT_NIS) {
        // 如果同时指定了 -F 参数，从指定文件读取 NIS 域名并写入到 /proc/sys/kernel/domainname
        if let Some(file_path) = args_match.get_one::<OsString>(OPT_FILE) {
            let nis_domain = std::fs::read_to_string(file_path)
                .map_err_context(|| "failed to read from file".to_owned())?
                .trim()
                .to_string();

            // 将 NIS 域名写入到 /proc/sys/kernel/domainname
            std::fs::write("/proc/sys/kernel/domainname", &nis_domain)
                .map_err_context(|| "failed to write to /proc/sys/kernel/domainname".to_owned())?;
            return Ok(());
        }

        // 尝试从 /proc/sys/kernel/domainname 文件读取 NIS 域名
        let nis_domain = std::fs::read_to_string("/proc/sys/kernel/domainname")
            .map_err_context(|| "failed to read from file".to_owned())?
            .trim()
            .to_string();

        if nis_domain == "(none)" {
            println!("hostname: Local domain name not set");
        } else {
            println!("{}", nis_domain.trim());
        }

        Ok(())
    } else if args_match.get_flag(OPT_ALL_IP) {
        // 获取所有网络接口的 IP 地址
        let addrs = if_addrs::get_if_addrs()
            .map_err_context(|| "failed to get network interfaces".to_owned())?;

        // 收集所有唯一的 IP 地址
        let mut ips = HashSet::new();
        for addr in addrs {
            // 跳过回环接口
            if addr.name == "lo" {
                continue;
            }
            let ip = addr.ip();
            ips.insert(ip);
        }

        // 输出所有找到的 IP 地址
        let mut output = String::new();
        for ip in ips {
            output.push_str(&ip.to_string());
            output.push(' ');
        }

        // 移除最后一个多余的空格并打印
        if !output.is_empty() {
            println!("{}", output.trim_end());
        }

        Ok(())
    } else if let Some(default_name) = args_match.get_one::<OsString>(OPT_BOOT) {
        // 如果同时指定了 -F 参数，从指定文件读取主机名
        if let Some(file_path) = args_match.get_one::<OsString>(OPT_FILE) {
            let hostname_str = std::fs::read_to_string(file_path)
                .map_err_context(|| "failed to read from file".to_owned())?
                .trim()
                .to_string();
            hostname::set(&hostname_str)
                .map_err_context(|| "failed to set hostname from file".to_owned())?;
            return Ok(());
        }

        if default_name == "my-host" {
            let hostname_str = default_name.to_string_lossy().into_owned();
            let name = hostname::get()
                .map_err_context(|| "failed to get hostname".to_owned())?
                .to_string_lossy()
                .into_owned();

            if name.is_empty() {
                hostname::set(&hostname_str)
                    .map_err_context(|| "failed to set default hostname".to_owned())?;
            } else {
                println!("{name}");
            }
        } else {
            let hostname_str = default_name.to_string_lossy().into_owned();
            // 如果提供了默认主机名，则设置默认主机名
            hostname::set(&hostname_str)
                .map_err_context(|| "failed to set default hostname".to_owned())?;
        }

        Ok(())
    } else if args_match.get_flag(OPT_ALIAS) {
        // 读取 /etc/hosts 文件获取别名
        let hosts_content = std::fs::read_to_string("/etc/hosts")
            .map_err_context(|| "failed to read /etc/hosts".to_owned())?;

        // 解析 hosts 文件，查找当前主机名的别名
        for line in hosts_content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 2 {
                continue;
            }

            // 检查是否包含当前主机名
            if parts[1..].contains(&hostname.as_str()) {
                // 输出该行中的所有别名（除了IP地址和主机名本身）
                let aliases: Vec<&str> = parts[1..]
                    .iter()
                    .filter(|&&name| name != hostname)
                    .copied()
                    .collect();

                if !aliases.is_empty() {
                    println!("{}", aliases.join(" "));
                }
                break;
            }
        }
        Ok(())
    } else if let Some(file_path) = args_match.get_one::<OsString>(OPT_FILE) {
        // 从文件读取主机名
        let hostname_str = std::fs::read_to_string(file_path)
            .map_err_context(|| "failed to read from file".to_owned())?
            .trim()
            .to_string();

        hostname::set(&hostname_str).map_err_context(|| "failed to set hostname".to_owned())?;
        Ok(())
    } else if args_match.get_flag(OPT_ALL_FQDNS) {
        // 添加一个临时端口以使用 to_socket_addrs
        let hostname_with_port = format!("{hostname}:1");

        // 解析所有地址
        let addrs = hostname_with_port
            .to_socket_addrs()
            .map_err_context(|| "failed to resolve socket addresses".to_owned())?;

        // 收集所有唯一的 FQDN
        let mut fqdns = HashSet::new();
        for addr in addrs {
            // 使用原始主机名作为 FQDN
            fqdns.insert(hostname.clone());

            // 尝试通过 IP 地址解析主机名
            let host = addr.ip();
            if let Ok(names) = dns_lookup::lookup_addr(&host) {
                fqdns.insert((names).to_string());
            }
        }

        // 输出所有找到的 FQDN
        let mut output = String::new();
        for fqdn in fqdns {
            output.push_str(&fqdn);
            output.push(' ');
            println!("{output}");
        }

        Ok(())
    } else if args_match.get_flag(OPT_IP_ADDRESS) {
        // 如果要求显示IP地址，则解析主机名对应的IP地址
        // 由于to_socket_addrs需要hostname:port格式，因此临时添加一个dummy端口，后续再移除
        let hostname = hostname + ":1";

        let ip_addrs = hostname
            .to_socket_addrs()
            .map_err_context(|| "failed to resolve socket addresses".to_owned())?;

        // 去重，避免输出重复的IP地址
        let mut hash_set = HashSet::new();
        let mut out = String::new();

        for addr in ip_addrs {
            if !hash_set.contains(&addr) {
                let mut ip = addr.to_string();
                // 移除之前添加的dummy端口
                if ip.ends_with(":1") {
                    let len = ip.len();
                    ip.truncate(len - 2);
                }
                if addr.is_ipv6() {
                    let ip_str = addr.ip().to_string();
                    let interface_name = find_ipv6_interface_name(ip_str.clone());
                    out.push_str(&format!("{ip_str}%{interface_name}"));
                } else {
                    out.push_str(&ip);
                }
                out.push(' ');
                hash_set.insert(addr);
            }
        }
        // 输出去重后的IP地址列表
        let len = out.len();
        if len > 0 {
            println!("{}", &out[0..len - 1]);
        }

        Ok(())
    } else {
        // 根据命令行参数显示短主机名或域名
        if args_match.get_flag(OPT_SHORT) || args_match.get_flag(OPT_DOMAIN) {
            // 查找并处理主机名中的第一个'.'，以决定要显示的部分
            let mut it = hostname.char_indices().filter(|&ci| ci.1 == '.');

            if let Some(ci) = it.next() {
                if args_match.get_flag(OPT_SHORT) {
                    // 显示短主机名
                    println!("{}", &hostname[0..ci.0]);
                } else {
                    // 显示域名
                    println!("{}", &hostname[ci.0 + 1..]);
                }
                return Ok(());
            }
        }

        // 默认显示完整主机名
        println!("{hostname}");

        Ok(())
    }
}

/**
 * 查找给定IPv6地址对应的网络接口名称
 *
 * 该函数遍历系统的所有网络接口，查找与给定IPv6地址匹配的接口。
 * 它会跳过回环接口和IPv4接口，只关注IPv6接口。
 *
 * # 参数
 * * `ip_str` - 要查找的IPv6地址字符串
 *
 * # 返回值
 * 返回找到的网络接口名称。如果未找到匹配的接口，则返回空字符串。
 *
 * # 注意
 * - 该函数会忽略回环接口（loopback）
 * - 只处理IPv6地址，会跳过IPv4接口
 * - 如果获取网络接口信息失败，函数会panic
 */
fn find_ipv6_interface_name(ip_str: String) -> String {
    let if_addrs = nix::ifaddrs::getifaddrs().unwrap();

    for iface in if_addrs {
        if iface
            .flags
            .contains(nix::net::if_::InterfaceFlags::IFF_LOOPBACK)
        {
            continue;
        }

        let sock = if let Some(sock) = iface.netmask {
            sock
        } else {
            continue;
        };

        if sock.family() == Some(AddressFamily::Inet) {
            continue;
        } else if sock.family() == Some(AddressFamily::Inet6) {
            if let Some(addr_ip) = iface.address {
                let ip_str_v6 = addr_ip.as_sockaddr_in6().unwrap().ip().to_string();
                if ip_str == ip_str_v6 {
                    return iface.interface_name;
                }
            } else {
                continue;
            };
        }
    }

    "".to_string()
}

#[derive(Default)]
pub struct Hostname;
impl Tool for Hostname {
    fn name(&self) -> &'static str {
        "hostname"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        // 直接调用原有的 hostname_main 函数
        hostname_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Hostname;

        // 测试 name 方法
        assert_eq!(tool.name(), "hostname");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("hostname"));

        // 测试 execute 方法
        let args = vec![OsString::from("hostname"), OsString::from("--version")];
        assert!(tool.execute(&args).is_err());
    }

    mod tests_ct_app {
        use crate::{OPT_DOMAIN, OPT_FQDN, OPT_HOST, OPT_IP_ADDRESS, OPT_SHORT, ct_app};
        use clap::error::ErrorKind;

        #[test]
        fn test_ct_app_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_v() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_help() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_h() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-h"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_domain() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--domain"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.unwrap().get_flag(OPT_DOMAIN));
        }

        #[test]
        fn test_ct_app_d() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-d"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.unwrap().get_flag(OPT_DOMAIN));
        }

        #[test]
        fn test_ct_app_ip_address() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--ip-address"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.unwrap().get_flag(OPT_IP_ADDRESS));
        }

        #[test]
        fn test_ct_app_i() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-i"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.unwrap().get_flag(OPT_IP_ADDRESS));
        }

        #[test]
        fn test_ct_app_fqdn() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--fqdn"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.unwrap().get_flag(OPT_FQDN));
        }

        #[test]
        fn test_ct_app_f() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-f"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.unwrap().get_flag(OPT_FQDN));
        }

        #[test]
        fn test_ct_app_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--short"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.unwrap().get_flag(OPT_SHORT));
        }

        #[test]
        fn test_ct_app_s() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-s"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.unwrap().get_flag(OPT_SHORT));
        }

        #[test]
        fn test_ct_app_hostname() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name()];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(None, result.unwrap().get_one::<String>(OPT_HOST));
        }
    }

    mod tests_hostname_main {
        use crate::hostname_main;

        use std::ffi::OsString;
        //use std::fs::File;
        //use std::io::Write;
        //use tempfile::tempdir;

        #[test]
        fn test_hostname_main_version() {
            let args = [ctcore::ct_util_name(), "--version"];

            let result = hostname_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_hostname_main_v() {
            let args = [ctcore::ct_util_name(), "-V"];
            let result = hostname_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_hostname_main_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = hostname_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_hostname_main_h() {
            let args = [ctcore::ct_util_name(), "-h"];
            let result = hostname_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_hostname_main_domain() {
            let args = [ctcore::ct_util_name(), "--domain"];
            let result = hostname_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_hostname_main_d() {
            let args = [ctcore::ct_util_name(), "-d"];
            let result = hostname_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_hostname_main_ip_address() {
            let args = [ctcore::ct_util_name(), "--ip-address"];
            let result = hostname_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_hostname_main_i() {
            let args = [ctcore::ct_util_name(), "-i"];
            let result = hostname_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_hostname_main_fqdn() {
            let args = [ctcore::ct_util_name(), "--fqdn"];
            let result = hostname_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_hostname_main_f() {
            let args = [ctcore::ct_util_name(), "-f"];
            let result = hostname_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_hostname_main_short() {
            let args = [ctcore::ct_util_name(), "--short"];
            let result = hostname_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_hostname_main_s() {
            let args = [ctcore::ct_util_name(), "-s"];
            let result = hostname_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_hostname_main_hostname() {
            let args = [ctcore::ct_util_name()];
            let result = hostname_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_hostname_main_all_fqdns() {
            let args = [ctcore::ct_util_name(), "--all-fqdns"];

            let result = hostname_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        /*#[test]
        fn test_hostname_main_all_with_file() {
            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let temp_file_path = temp_dir
                .path()
                .join("test_read_file_with_permissions_denied.txt");
            let mut temp_file =
                File::create(&temp_file_path).expect("Failed to create temporary file");
            temp_file
                .write_all(b"test-hostname")
                .expect("Failed to write to temporary file");

            let args = vec![
                ctcore::ct_util_name(),
                "-F",
                temp_file_path.to_str().unwrap()
            ];

            let result = hostname_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }*/

        #[test]
        fn test_hostname_main_alias() {
            let args = [ctcore::ct_util_name(), "-a"];

            let result = hostname_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_hostname_main_boot() {
            let args = [ctcore::ct_util_name(), "-b"];

            let result = hostname_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_hostname_main_all_ip_address() {
            let args = [ctcore::ct_util_name(), "-I"];

            let result = hostname_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_hostname_main_nis() {
            let args = [ctcore::ct_util_name(), "-y"];

            let result = hostname_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }
    }

    mod test_find_ipv6_interface_name {
        use super::super::find_ipv6_interface_name;
        use nix::ifaddrs::getifaddrs;

        #[test]
        fn test_find_ipv6_interface_name_real_interfaces() {
            // 获取系统实际的网络接口
            let interfaces = getifaddrs().unwrap();

            // 遍历接口找到第一个IPv6地址
            for interface in interfaces {
                if let Some(addr) = interface.address {
                    if let Some(sock6) = addr.as_sockaddr_in6() {
                        let ip_str = sock6.ip().to_string();
                        let result = find_ipv6_interface_name(ip_str.clone());

                        // 如果找到了接口，验证返回的接口名是否正确
                        if !result.is_empty() {
                            assert_eq!(result, interface.interface_name);
                            return;
                        }
                    }
                }
            }
        }
    }
}
