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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(test)]
    mod uname_output_tests {
        use super::*;

        fn generate_uname_flags(
            is_all: bool,
            is_kernel_name: bool,
            is_node_name: bool,
            is_kernel_release: bool,
            is_kernel_version: bool,
            is_machine: bool,
            is_processor: bool,
            is_hardware_platform: bool,
            is_os: bool,
        ) -> UnameFlags {
            UnameFlags {
                is_all,
                is_kernel_name,
                is_node_name,
                is_kernel_release,
                is_kernel_version,
                is_machine,
                is_processor,
                is_hardware_platform,
                is_os,
            }
        }

        #[test]
        fn test_uname_output_all_flags() {
            let flags =
                generate_uname_flags(true, false, false, false, false, false, false, false, false);
            let uname_output = UNameOutput::new(&flags).unwrap();

            assert_eq!(uname_output.kernel_name, Some("Linux".to_string()));
            assert!(uname_output.node_name.is_some());
            assert!(uname_output.kernel_release.is_some());
            assert!(uname_output.kernel_version.is_some());
            assert!(uname_output.machine.is_some());
            assert!(uname_output.os.is_some());
            assert!(uname_output.processor.is_none());
            assert!(uname_output.hardware_platform.is_none());
            assert!(!uname_output.display().is_empty());
        }

        #[test]
        fn test_uname_output_individual_flags() {
            let flags = generate_uname_flags(false, true, true, true, true, true, true, true, true);
            let uname_output = UNameOutput::new(&flags).unwrap();

            assert!(uname_output.kernel_name.is_some());
            assert!(uname_output.node_name.is_some());
            assert!(uname_output.kernel_release.is_some());
            assert!(uname_output.kernel_version.is_some());
            assert!(uname_output.machine.is_some());
            assert!(uname_output.os.is_some());
            assert!(uname_output.processor.is_some());
            assert!(uname_output.hardware_platform.is_some());
            assert!(!uname_output.display().is_empty());
        }

        #[test]
        fn test_uname_output_no_flags() {
            let flags = generate_uname_flags(
                false, false, false, false, false, false, false, false, false,
            );
            let uname_output = UNameOutput::new(&flags).unwrap();

            assert!(uname_output.kernel_name.is_some());
            assert!(uname_output.node_name.is_none());
            assert!(uname_output.kernel_release.is_none());
            assert!(uname_output.kernel_version.is_none());
            assert!(uname_output.machine.is_none());
            assert!(uname_output.os.is_none());
            assert!(uname_output.processor.is_none());
            assert!(uname_output.hardware_platform.is_none());

            let expected_output = "Linux ";
            assert_eq!(
                uname_output.display().trim_end(),
                expected_output.trim_end()
            );
        }

        #[test]
        fn test_uname_output_some_flags() {
            let flags =
                generate_uname_flags(false, true, false, true, false, true, false, true, false);
            let uname_output = UNameOutput::new(&flags).unwrap();

            assert!(uname_output.kernel_name.is_some());
            assert!(uname_output.node_name.is_none());
            assert!(uname_output.kernel_release.is_some());
            assert!(uname_output.kernel_version.is_none());
            assert!(uname_output.machine.is_some());
            assert!(uname_output.os.is_none());
            assert!(uname_output.processor.is_none());
            assert!(uname_output.hardware_platform.is_some());
            assert!(!uname_output.display().is_empty());
        }
    }

    #[cfg(test)]
    mod ct_main_tests {
        use std::ffi::OsString;

        use super::*;

        #[test]
        fn test_uname_main_execution_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = uname_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_uname_main_execution_other_version() {
            let args = vec![ctcore::ct_util_name(), "-V"];
            let result = uname_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_uname_main_execution_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = uname_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_uname_main_execution_unsupport_help() {
            let args = vec![ctcore::ct_util_name(), "-H"];
            let result = uname_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_uname_main_invalid_argument() {
            let args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = uname_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_uname_main_support_missing_argument() {
            let args = vec![ctcore::ct_util_name()]; // 缺少任何参数
            let result = uname_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_uname_main_long_option_all() {
            let args = vec![ctcore::ct_util_name(), "--all"];
            let result = uname_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_uname_main_long_option_kernel_name() {
            let args = vec![ctcore::ct_util_name(), "--kernel-name"];
            let result = uname_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_uname_main_long_option_nodename() {
            let args = vec![ctcore::ct_util_name(), "--nodename"];
            let result = uname_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_uname_main_long_option_kernel_release() {
            let args = vec![ctcore::ct_util_name(), "--kernel-release"];
            let result = uname_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_uname_main_long_option_kernel_version() {
            let args = vec![ctcore::ct_util_name(), "--kernel-version"];
            let result = uname_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_uname_main_long_option_machine() {
            let args = vec![ctcore::ct_util_name(), "--machine"];
            let result = uname_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_uname_main_long_option_os() {
            let args = vec![ctcore::ct_util_name(), "--operating-system"];
            let result = uname_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_uname_main_long_option_processor() {
            let args = vec![ctcore::ct_util_name(), "--processor"];
            let result = uname_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_uname_main_long_option_hardware_platform() {
            let args = vec![ctcore::ct_util_name(), "--hardware-platform"];
            let result = uname_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_uname_main_short_option_a() {
            let args = vec![ctcore::ct_util_name(), "-a"];
            let result = uname_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_uname_main_short_option_s() {
            let args = vec![ctcore::ct_util_name(), "-s"];
            let result = uname_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_uname_main_short_option_n() {
            let args = vec![ctcore::ct_util_name(), "-n"];
            let result = uname_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_uname_main_short_option_r() {
            let args = vec![ctcore::ct_util_name(), "-r"];
            let result = uname_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

        #[test]
        fn test_uname_main_short_option_v() {
            let args = vec![ctcore::ct_util_name(), "-v"];
            let result = uname_main(args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    let code = output.code();
                    let message = output.usage();
                    println!("Error code: {}", code);
                    println!("Error message: {}", message);
                }
                Ok(output) => {
                    assert_eq!(output, ());
                }
            }
        }

    }
}