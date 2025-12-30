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

use std::net::ToSocketAddrs;
use std::str;

use std::collections::hash_set::HashSet;

use std::ffi::OsString;

use clap::builder::ValueParser;
use clap::crate_version;

use clap::Arg;
use clap::ArgAction;
use clap::ArgMatches;
use clap::Command;

use ctcore::{
    ct_error::{CTResult, FromIo},
    ct_format_usage, ct_help_about, ct_help_usage,
};

const HOSTNAME_ABOUT: &str = ct_help_about!("hostname.md");
const HOSTNAME_USAGE: &str = ct_help_usage!("hostname.md");

static OPT_DOMAIN: &str = "domain";
static OPT_IP_ADDRESS: &str = "ip-address";
static OPT_FQDN: &str = "fqdn";
static OPT_SHORT: &str = "short";
static OPT_HOST: &str = "host";

#[cfg(windows)]
mod wsa {
    use std::io;

    use windows_sys::Win32::Networking::WinSock::{WSACleanup, WSAStartup, WSADATA};

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

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    hostname_main(args).map(|_| ())
}
pub fn hostname_main(args: impl ctcore::Args) -> CTResult<()> {
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
    let application_info = HOSTNAME_ABOUT;
    let usage_description = ct_format_usage(HOSTNAME_USAGE);

    let args = vec![
        Arg::new(OPT_DOMAIN)
            .short('d')
            .long("domain")
            .overrides_with_all([OPT_DOMAIN, OPT_IP_ADDRESS, OPT_FQDN, OPT_SHORT])
            .help("Display the name of the DNS domain if possible")
            .action(ArgAction::SetTrue),
        Arg::new(OPT_IP_ADDRESS)
            .short('i')
            .long("ip-address")
            .overrides_with_all([OPT_DOMAIN, OPT_IP_ADDRESS, OPT_FQDN, OPT_SHORT])
            .help("Display the network address(es) of the host")
            .action(ArgAction::SetTrue),
        Arg::new(OPT_FQDN)
            .short('f')
            .long("fqdn")
            .overrides_with_all([OPT_DOMAIN, OPT_IP_ADDRESS, OPT_FQDN, OPT_SHORT])
            .help("Display the FQDN (Fully Qualified Domain Name) (default)")
            .action(ArgAction::SetTrue),
        Arg::new(OPT_SHORT)
            .short('s')
            .long("short")
            .overrides_with_all([OPT_DOMAIN, OPT_IP_ADDRESS, OPT_FQDN, OPT_SHORT])
            .help("Display the short hostname (the portion before the first dot) if possible")
            .action(ArgAction::SetTrue),
        Arg::new(OPT_HOST)
            .value_parser(ValueParser::os_string())
            .value_hint(clap::ValueHint::Hostname),
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

    if args_match.get_flag(OPT_IP_ADDRESS) {
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
                out.push_str(&ip);
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

