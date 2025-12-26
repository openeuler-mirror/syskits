/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

//! uname 是一个 Linux 系统命令，用于显示系统的基本信息。
//! 它提供了关于操作系统的内核名称、版本、主机名、硬件平台（体系结构）和操作系统发行版等信息。

use clap::{crate_version, Arg, ArgAction, Command};
use platform_info::*;

use ctcore::{
    ct_error::{CTResult, CtSimpleError},
    ct_format_usage, ct_help_about, ct_help_usage,
};

const UNAME_ABOUT: &str = ct_help_about!("uname.md");
const UNAME_USAGE: &str = ct_help_usage!("uname.md");

pub mod uname_flags {
    pub static UNAME_ALL: &str = "all";
    pub static UNAME_KERNEL_NAME: &str = "kernel-name";
    pub static UNAME_NODE_NAME: &str = "nodename";
    pub static UNAME_KERNEL_VERSION: &str = "kernel-version";
    pub static UNAME_KERNEL_RELEASE: &str = "kernel-release";
    pub static UNAME_MACHINE: &str = "machine";
    pub static UNAME_PROCESSOR: &str = "processor";
    pub static UNAME_HARDWARE_PLATFORM: &str = "hardware-platform";
    pub static UNAME_OS: &str = "operating-system";
}

pub struct UNameOutput {
    pub kernel_name: Option<String>,
    pub node_name: Option<String>,
    pub kernel_release: Option<String>,
    pub kernel_version: Option<String>,
    pub machine: Option<String>,
    pub os: Option<String>,
    pub processor: Option<String>,
    pub hardware_platform: Option<String>,
}

impl UNameOutput {
    fn display(&self) -> String {
        let mut output = String::new();
        for name in [
            self.kernel_name.as_ref(),
            self.node_name.as_ref(),
            self.kernel_release.as_ref(),
            self.kernel_version.as_ref(),
            self.machine.as_ref(),
            self.os.as_ref(),
            self.processor.as_ref(),
            self.hardware_platform.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            output.push_str(name);
            output.push(' ');
        }
        output
    }

    pub fn new(opts: &UnameFlags) -> CTResult<Self> {
        let uname =
            PlatformInfo::new().map_err(|_e| CtSimpleError::new(1, "cannot get system name"))?;
        let is_none = !(opts.is_all
            || opts.is_kernel_name
            || opts.is_node_name
            || opts.is_kernel_release
            || opts.is_kernel_version
            || opts.is_machine
            || opts.is_os
            || opts.is_processor
            || opts.is_hardware_platform);

        let kernel_name = (opts.is_kernel_name || opts.is_all || is_none)
            .then(|| uname.sysname().to_string_lossy().to_string());

        let node_name = (opts.is_node_name || opts.is_all)
            .then(|| uname.nodename().to_string_lossy().to_string());

        let kernel_release = (opts.is_kernel_release || opts.is_all)
            .then(|| uname.release().to_string_lossy().to_string());

        let kernel_version = (opts.is_kernel_version || opts.is_all)
            .then(|| uname.version().to_string_lossy().to_string());

        let machine =
            (opts.is_machine || opts.is_all).then(|| uname.machine().to_string_lossy().to_string());

        let os = (opts.is_os || opts.is_all).then(|| uname.osname().to_string_lossy().to_string());

        let processor = opts.is_processor.then(|| "unknown".to_string());

        let hardware_platform = opts.is_hardware_platform.then(|| "unknown".to_string());

        Ok(Self {
            kernel_name,
            node_name,
            kernel_release,
            kernel_version,
            machine,
            os,
            processor,
            hardware_platform,
        })
    }
}

pub struct UnameFlags {
    pub is_all: bool,
    pub is_kernel_name: bool,
    pub is_node_name: bool,
    pub is_kernel_version: bool,
    pub is_kernel_release: bool,
    pub is_machine: bool,
    pub is_processor: bool,
    pub is_hardware_platform: bool,
    pub is_os: bool,
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    uname_main(args)
}

pub fn uname_main(args: impl ctcore::Args) -> CTResult<()> {
    let matches = ct_app().try_get_matches_from(args)?;

    let flags = UnameFlags {
        is_all: matches.get_flag(uname_flags::UNAME_ALL),
        is_kernel_name: matches.get_flag(uname_flags::UNAME_KERNEL_NAME),
        is_node_name: matches.get_flag(uname_flags::UNAME_NODE_NAME),
        is_kernel_release: matches.get_flag(uname_flags::UNAME_KERNEL_RELEASE),
        is_kernel_version: matches.get_flag(uname_flags::UNAME_KERNEL_VERSION),
        is_machine: matches.get_flag(uname_flags::UNAME_MACHINE),
        is_processor: matches.get_flag(uname_flags::UNAME_PROCESSOR),
        is_hardware_platform: matches.get_flag(uname_flags::UNAME_HARDWARE_PLATFORM),
        is_os: matches.get_flag(uname_flags::UNAME_OS),
    };
    let output = UNameOutput::new(&flags)?;
    println!("{}", output.display().trim_end());
    Ok(())
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = UNAME_ABOUT;
    let usage_description = ct_format_usage(UNAME_USAGE);
    let args = vec![
        Arg::new(uname_flags::UNAME_ALL)
            .short('a')
            .long(uname_flags::UNAME_ALL)
            .help("Behave as though all of the flags -mnrsvo were specified.")
            .action(ArgAction::SetTrue),
        Arg::new(uname_flags::UNAME_KERNEL_NAME)
            .short('s')
            .long(uname_flags::UNAME_KERNEL_NAME)
            .alias("sysname") // Obsolescent option in GNU uname
            .help("print the kernel name.")
            .action(ArgAction::SetTrue),
        Arg::new(uname_flags::UNAME_NODE_NAME)
            .short('n')
            .long(uname_flags::UNAME_NODE_NAME)
            .help(
                "print the nodename (the nodename may be a name that the system \
                is known by to a communications network).",
            )
            .action(ArgAction::SetTrue),
        Arg::new(uname_flags::UNAME_KERNEL_RELEASE)
            .short('r')
            .long(uname_flags::UNAME_KERNEL_RELEASE)
            .alias("release") // Obsolescent option in GNU uname
            .help("print the operating system release.")
            .action(ArgAction::SetTrue),
        Arg::new(uname_flags::UNAME_KERNEL_VERSION)
            .short('v')
            .long(uname_flags::UNAME_KERNEL_VERSION)
            .help("print the operating system version.")
            .action(ArgAction::SetTrue),
        Arg::new(uname_flags::UNAME_MACHINE)
            .short('m')
            .long(uname_flags::UNAME_MACHINE)
            .help("print the machine hardware name.")
            .action(ArgAction::SetTrue),
        Arg::new(uname_flags::UNAME_OS)
            .short('o')
            .long(uname_flags::UNAME_OS)
            .help("print the operating system name.")
            .action(ArgAction::SetTrue),
        Arg::new(uname_flags::UNAME_PROCESSOR)
            .short('p')
            .long(uname_flags::UNAME_PROCESSOR)
            .help("print the processor type (non-portable)")
            .action(ArgAction::SetTrue)
            .hide(true),
        Arg::new(uname_flags::UNAME_HARDWARE_PLATFORM)
            .short('i')
            .long(uname_flags::UNAME_HARDWARE_PLATFORM)
            .help("print the hardware platform (non-portable)")
            .action(ArgAction::SetTrue)
            .hide(true),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(args)
}

