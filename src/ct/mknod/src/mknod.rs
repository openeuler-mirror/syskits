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

// mknod 命令在类 Unix 系统（如 Linux）中用于创建特殊的文件类型，如字符设备（character device）、块设备（block device）和命名管道（named pipe，也称为 FIFO）。
// 这些文件类型在操作系统中用于与硬件交互或实现进程间的通信。
// 主要功能：实现类Unix系统调用mknod的功能，用于创建特殊文件：块设备、字符设备、FIFO。
// 支持的平台：非Windows的Unix平台。
// 使用的库：clap用于命令行参数解析，libc提供Unix系统调用的接口。

use clap::{crate_version, value_parser, Arg, ArgMatches, Command};
use libc::{dev_t, mode_t};
use libc::{S_IFBLK, S_IFCHR, S_IFIFO, S_IRGRP, S_IROTH, S_IRUSR, S_IWGRP, S_IWOTH, S_IWUSR};
use std::ffi::CString;

use ctcore::ct_display::Quotable;
use ctcore::ct_error::{set_ct_exit_code, CTResult, CTsageError, CtSimpleError};
use ctcore::{ct_format_usage, ct_help_about, ct_help_section, ct_help_usage};

const MKNOD_ABOUT: &str = ct_help_about!("mknod.md");
const MKNOD_USAGE: &str = ct_help_usage!("mknod.md");
const MKNOD_AFTER_HELP: &str = ct_help_section!("after help", "mknod.md");

// 常量：用于设置文件模式的权限位。
const MODE_RW_UGO: mode_t = S_IRUSR | S_IWUSR | S_IRGRP | S_IWGRP | S_IROTH | S_IWOTH;

// 函数：根据给定的主、次设备号构造一个dev_t值。
#[inline(always)]
fn make_dev(maj: u64, min: u64) -> dev_t {
    // 根据设备号构造dev_t，细节来自<sys/sysmacros.h>
    ((min & 0xff) | ((maj & 0xfff) << 8) | ((min & !0xff) << 12) | ((maj & !0xfff) << 32)) as dev_t
}

// Windows平台不支持mknod，因此定义一个空实现。
#[cfg(windows)]
fn _mknod(file_name: &str, mode: mode_t, dev: dev_t) -> i32 {
    panic!("Unsupported for windows platform")
}

// 枚举：文件类型，包括块设备、字符设备和FIFO。
#[derive(Clone, PartialEq)]
enum MknodFileType {
    Block,
    Character,
    Fifo,
}

// Unix平台的_mknod实现。
#[cfg(unix)]
fn _mknod(file_name: &str, mode: mode_t, dev: dev_t) -> i32 {
    let c_str = CString::new(file_name).expect("Failed to convert to CString");

    // 检查是否设置了文件模式的权限位
    let set_umask = mode & MODE_RW_UGO != MODE_RW_UGO;

    unsafe {
        // 保存当前的umask，如果需要的话
        let last_umask = if set_umask { libc::umask(0) } else { 0 };

        // 执行mknod系统调用
        let errno = libc::mknod(c_str.as_ptr(), mode, dev);

        // 如果设置了umask，将其恢复到原始值
        if set_umask {
            libc::umask(last_umask);
        }

        // 处理系统调用失败的情况
        if errno == -1 {
            let c_str = CString::new(ctcore::ct_execute_phrase().as_bytes())
                .expect("Failed to convert to CString");
            libc::perror(c_str.as_ptr());
        }
        errno
    }
}

// ctmain函数：程序的入口点。
#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    mknod_main(args).map(|_| ())
}

pub fn mknod_main(args: impl ctcore::Args) -> CTResult<()> {
    // 从命令行参数中解析选项和参数
    let args_match = ct_app().try_get_matches_from(args)?;

    let mode = get_mode(&args_match).map_err(|e| CtSimpleError::new(1, e))?;

    let file_name = args_match
        .get_one::<String>("name")
        .expect("Missing argument 'NAME'");

    let file_type = args_match.get_one::<MknodFileType>("type").unwrap();

    // 根据文件类型执行不同的操作
    mknod_processing(&args_match, mode, file_name, file_type)
}

fn mknod_processing(
    args_match: &ArgMatches,
    mode: mode_t,
    file_name: &str,
    file_type: &MknodFileType,
) -> CTResult<()> {
    if *file_type == MknodFileType::Fifo {
        // FIFO文件不需要主、次设备号
        if args_match.contains_id("major") || args_match.contains_id("minor") {
            Err(CTsageError::new(
                1,
                "Fifos do not have major and minor device numbers.",
            ))
        } else {
            let exit_code = _mknod(file_name, S_IFIFO | mode, 0);
            set_ct_exit_code(exit_code);
            Ok(())
        }
    } else {
        // 对于块设备和字符设备，需要主、次设备号
        match (
            args_match.get_one::<u64>("major"),
            args_match.get_one::<u64>("minor"),
        ) {
            (_, None) | (None, _) => Err(CTsageError::new(
                1,
                "Special files require major and minor device numbers.",
            )),
            (Some(&major), Some(&minor)) => {
                let dev = make_dev(major, minor);
                let exit_code = match file_type {
                    MknodFileType::Block => _mknod(file_name, S_IFBLK | mode, dev),
                    MknodFileType::Character => _mknod(file_name, S_IFCHR | mode, dev),
                    _ => unreachable!("file_type was validated to be only block or character"),
                };
                set_ct_exit_code(exit_code);
                Ok(())
            }
        }
    }
}

// 构建命令行解析器。
pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = MKNOD_ABOUT;
    let usage_description = ct_format_usage(MKNOD_USAGE);
    let after_info = MKNOD_AFTER_HELP;

    let args = vec![
        Arg::new("mode")
            .short('m')
            .long("mode")
            .value_name("MODE")
            .help("set file permission bits to MODE, not a=rw - umask"),
        Arg::new("name")
            .value_name("NAME")
            .help("name of the new file")
            .required(true)
            .value_hint(clap::ValueHint::AnyPath),
        Arg::new("type")
            .value_name("TYPE")
            .help("type of the new file (b, c, u or p)")
            .required(true)
            .value_parser(parse_type),
        Arg::new("major")
            .value_name("MAJOR")
            .help("major file type")
            .value_parser(value_parser!(u64)),
        Arg::new("minor")
            .value_name("MINOR")
            .help("minor file type")
            .value_parser(value_parser!(u64)),
    ];

    Command::new(utility_name)
        .version(command_version)
        .override_usage(usage_description)
        .after_help(after_info)
        .about(application_info)
        .infer_long_args(true)
        .args(&args)
}

// 解析模式参数。
fn get_mode(matches: &ArgMatches) -> Result<mode_t, String> {
    match matches.get_one::<String>("mode") {
        None => Ok(MODE_RW_UGO),
        Some(str_mode) => ctcore::ct_mode::parse_mode(str_mode)
            .map_err(|e| format!("invalid mode ({e})"))
            .and_then(|mode| {
                if mode > 0o777 {
                    Err("mode must specify only file permission bits".to_string())
                } else {
                    Ok(mode)
                }
            }),
    }
}

// 解析文件类型参数。
fn parse_type(tpe: &str) -> Result<MknodFileType, String> {
    // 仅根据第一个字符解析文件类型
    tpe.chars()
        .next()
        .ok_or_else(|| "missing device type".to_string())
        .and_then(|first_char| match first_char {
            'b' => Ok(MknodFileType::Block),
            'c' | 'u' => Ok(MknodFileType::Character),
            'p' => Ok(MknodFileType::Fifo),
            _ => Err(format!("invalid device type {}", tpe.quote())),
        })
}

