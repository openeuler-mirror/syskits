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

//! sum 命令用于计算文件的校验和及其块数。
//! 1. 计算文件的校验和：
//!    计算文件的内容的校验和，用于检查文件是否被修改或损坏。
//!    sum 命令提供了两种不同的算法来计算校验和：BSD 算法和 System V 算法。
//! 2. 计算文件的块数：
//!    计算文件的大小，并以块的形式报告。块的大小可以是 512 字节（System V 算法）或 1024 字节（BSD 算法）。

use clap::{Arg, ArgAction, Command, crate_version};
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CtSimpleError, FromIo};
use ctcore::{ct_format_usage, ct_help_about, ct_help_usage, ct_show};
use std::fs::File;
use std::io::{Read, stdin};
use std::path::Path;

const SUM_USAGE: &str = ct_help_usage!("sum.md");
const SUM_ABOUT: &str = ct_help_about!("sum.md");

// 这个可以被 usize::div_ceil 替代，一旦它稳定下来。
// 这种实现方式针对 b 是一个常量的情况进行了优化，
// 尤其是 b 是 2 的幂时。
const fn sum_div_ceil(a: usize, b: usize) -> usize {
    a.div_ceil(b)
}

fn sum_bsd(mut reader: Box<dyn Read>) -> (usize, u16) {
    let mut data_buf = [0; 4096];
    let mut bytes_read = 0;
    let mut check_sum: u16 = 0;
    loop {
        match reader.read(&mut data_buf) {
            Ok(n) if n != 0 => {
                bytes_read += n;
                for &byte in &data_buf[..n] {
                    check_sum = check_sum.rotate_right(1);
                    check_sum = check_sum.wrapping_add(u16::from(byte));
                }
            }
            _ => break,
        }
    }

    // 报告读取的块数，以 1024 字节为单位。
    let blocks_read = sum_div_ceil(bytes_read, 1024);
    (blocks_read, check_sum)
}

fn sum_sysv(mut r: Box<dyn Read>) -> (usize, u16) {
    let mut data_buf = [0; 4096];
    let mut bytes_read = 0;
    let mut check_sum = 0u32;

    loop {
        match r.read(&mut data_buf) {
            Ok(n) if n != 0 => {
                bytes_read += n;
                for &byte in &data_buf[..n] {
                    check_sum = check_sum.wrapping_add(u32::from(byte));
                }
            }
            _ => break,
        }
    }

    check_sum = (check_sum & 0xffff) + (check_sum >> 16);
    check_sum = (check_sum & 0xffff) + (check_sum >> 16);

    // 报告读取的块数，以 512 字节为单位。
    let blocks_read = sum_div_ceil(bytes_read, 512);
    (blocks_read, check_sum as u16)
}

fn sum_open(name: &str) -> CTResult<Box<dyn Read>> {
    match name {
        "-" => Ok(Box::new(stdin()) as Box<dyn Read>),
        _ => {
            let path = &Path::new(name);
            if path.is_dir() {
                let err_message = format!("{}: Is a directory", name.maybe_quote());
                return Err(CtSimpleError::new(2, err_message));
            };
            // 消除警告，因为我们想要错误信息
            if path.metadata().is_err() {
                let err_message = format!("{}: No such file or directory", name.maybe_quote());
                return Err(CtSimpleError::new(2, err_message));
            };
            let f = File::open(path).map_err_context(String::new)?;
            Ok(Box::new(f) as Box<dyn Read>)
        }
    }
}

mod sum_flags {
    pub static SUM_FILE: &str = "file";
    pub static SUM_BSD_COMPATIBLE: &str = "r";
    pub static SUM_SYSTEM_V_COMPATIBLE: &str = "sysv";
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    sum_main(args)
}

pub fn sum_main(args: impl ctcore::Args) -> CTResult<()> {
    let matches = ct_app().try_get_matches_from(args)?;
    let files: Vec<String> = if let Some(v) = matches.get_many::<String>(sum_flags::SUM_FILE) {
        v.cloned().collect()
    } else {
        vec!["-".to_owned()]
    };

    let is_sysv = matches.get_flag(sum_flags::SUM_SYSTEM_V_COMPATIBLE);
    let is_print_names = files.len() > 1 || files[0] != "-";
    let width = match is_sysv {
        true => 1,
        false => 5,
    };

    for file in &files {
        let reader = match sum_open(file) {
            Ok(f) => f,
            Err(error) => {
                ct_show!(error);
                continue;
            }
        };
        let (blocks, sum) = match is_sysv {
            true => sum_sysv(reader),
            false => sum_bsd(reader),
        };

        match is_print_names {
            true => {
                println!("{sum:0width$} {blocks:width$} {file}");
            }
            false => {
                println!("{sum:0width$} {blocks:width$}");
            }
        };
    }
    Ok(())
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = SUM_ABOUT;
    let usage_description = ct_format_usage(SUM_USAGE);
    let args = vec![
        Arg::new(sum_flags::SUM_FILE)
            .action(ArgAction::Append)
            .hide(true)
            .value_hint(clap::ValueHint::FilePath),
        Arg::new(sum_flags::SUM_BSD_COMPATIBLE)
            .short('r')
            .help("use the BSD sum algorithm, use 1K blocks (default)")
            .action(ArgAction::SetTrue),
        Arg::new(sum_flags::SUM_SYSTEM_V_COMPATIBLE)
            .short('s')
            .long(sum_flags::SUM_SYSTEM_V_COMPATIBLE)
            .help("use System V sum algorithm, use 512 bytes blocks")
            .action(ArgAction::SetTrue),
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
    mod div_ceil_tests {
        use super::*;

        #[test]
        fn test_div_ceil() {
            // 测试 a 是 b 的倍数
            assert_eq!(sum_div_ceil(10, 2), 5);
            assert_eq!(sum_div_ceil(9, 3), 3);

            // 测试 a 不是 b 的倍数
            assert_eq!(sum_div_ceil(10, 3), 4);
            assert_eq!(sum_div_ceil(9, 4), 3);

            // 测试 b 是 1
            assert_eq!(sum_div_ceil(10, 1), 10);
            assert_eq!(sum_div_ceil(0, 1), 0);

            // 测试 a 是 0
            assert_eq!(sum_div_ceil(0, 2), 0);

            // 测试 a 和 b 相等
            assert_eq!(sum_div_ceil(5, 5), 1);

            // 测试较大的数值
            assert_eq!(sum_div_ceil(1000, 3), 334);
            assert_eq!(sum_div_ceil(1000, 500), 2);

            // 测试 b 大于 a
            assert_eq!(sum_div_ceil(3, 10), 1);
            assert_eq!(sum_div_ceil(0, usize::MAX), 0);

            // 测试较小的数值
            assert_eq!(sum_div_ceil(1, 2), 1);
            assert_eq!(sum_div_ceil(2, 3), 1);
        }
    }

    #[cfg(test)]
    mod bsd_sum_tests {
        use super::*;
        use std::io::Cursor;

        #[test]
        fn test_bsd_sum() {
            // 测试空输入
            let data = b"";
            let reader = Box::new(Cursor::new(data));
            let (blocks, checksum) = sum_bsd(reader);
            assert_eq!(blocks, 0);
            assert_eq!(checksum, 0);

            // 测试单个字节
            let data = b"a";
            let reader = Box::new(Cursor::new(data));
            let (blocks, checksum) = sum_bsd(reader);
            assert_eq!(blocks, 1);
            assert_eq!(checksum, 'a' as u16);

            // 测试短字符串
            let data = b"abc";
            let reader = Box::new(Cursor::new(data));
            let (blocks, checksum) = sum_bsd(reader);
            assert_eq!(blocks, 1);
            assert_eq!(checksum, 16556);

            // 测试长字符串（跨越多个块）
            let data = vec![b'a'; 5000];
            let reader = Box::new(Cursor::new(data));
            let (blocks, checksum) = sum_bsd(reader);
            assert_eq!(blocks, 5);
            assert_eq!(checksum, 41146);

            // 测试包含所有字节值的输入
            let data: Vec<u8> = (0..=255).collect();
            let reader = Box::new(Cursor::new(data));
            let (blocks, checksum) = sum_bsd(reader);
            assert_eq!(blocks, 1);
            assert_eq!(checksum, 512);

            // 测试混合数据
            let data = b"1234567890";
            let reader = Box::new(Cursor::new(data));
            let (blocks, checksum) = sum_bsd(reader);
            assert_eq!(blocks, 1);
            assert_eq!(checksum, 59623);

            // 测试较大数据
            let data = vec![b'Z'; 10000];
            let reader = Box::new(Cursor::new(data));
            let (blocks, checksum) = sum_bsd(reader);
            assert_eq!(blocks, 10);
            assert_eq!(checksum, 43443);
        }
    }

    #[cfg(test)]
    mod sysv_sum_tests {
        use super::*;
        use std::io::Cursor;

        #[test]
        fn test_sysv_sum() {
            // 测试空输入
            let data = b"";
            let reader = Box::new(Cursor::new(data));
            let (blocks, checksum) = sum_sysv(reader);
            assert_eq!(blocks, 0);
            assert_eq!(checksum, 0);

            // 测试单个字节
            let data = b"a";
            let reader = Box::new(Cursor::new(data));
            let (blocks, checksum) = sum_sysv(reader);
            assert_eq!(blocks, 1);
            assert_eq!(checksum, 'a' as u16);

            // 测试短字符串
            let data = b"abc";
            let reader = Box::new(Cursor::new(data));
            let (blocks, checksum) = sum_sysv(reader);
            assert_eq!(blocks, 1);
            assert_eq!(checksum, 'a' as u16 + 'b' as u16 + 'c' as u16);

            // 测试短字符串（不同字符）
            let data = b"xyz";
            let reader = Box::new(Cursor::new(data));
            let (blocks, checksum) = sum_sysv(reader);
            assert_eq!(blocks, 1);
            assert_eq!(checksum, 'x' as u16 + 'y' as u16 + 'z' as u16);

            // 测试长字符串（跨越多个块）
            let data = vec![b'a'; 5000];
            let reader = Box::new(Cursor::new(data));
            let (blocks, checksum) = sum_sysv(reader);
            assert_eq!(blocks, 10);
            assert_eq!(checksum, 26255);

            // 测试包含所有字节值的输入
            let data: Vec<u8> = (0..=255).collect();
            let reader = Box::new(Cursor::new(data));
            let (blocks, checksum) = sum_sysv(reader);
            assert_eq!(blocks, 1);
            assert_eq!(checksum, 32640);

            // 测试混合数据
            let data = b"1234567890";
            let reader = Box::new(Cursor::new(data));
            let (blocks, checksum) = sum_sysv(reader);
            assert_eq!(blocks, 1);
            assert_eq!(
                checksum,
                '1' as u16
                    + '2' as u16
                    + '3' as u16
                    + '4' as u16
                    + '5' as u16
                    + '6' as u16
                    + '7' as u16
                    + '8' as u16
                    + '9' as u16
                    + '0' as u16
            );

            // 测试较大数据
            let data = vec![b'Z'; 10000];
            let reader = Box::new(Cursor::new(data));
            let (blocks, checksum) = sum_sysv(reader);
            assert_eq!(blocks, 20);
            assert_eq!(checksum, 48045);

            // 测试数据跨越多块（精确到块边界）
            let data = vec![b'b'; 1024];
            let reader = Box::new(Cursor::new(data));
            let (blocks, checksum) = sum_sysv(reader);
            assert_eq!(blocks, 2);
            assert_eq!(checksum, 34817);

            // 测试数据跨越多块（块大小的倍数）
            let data = vec![b'c'; 2048];
            let reader = Box::new(Cursor::new(data));
            let (blocks, checksum) = sum_sysv(reader);
            assert_eq!(blocks, 4);
            assert_eq!(checksum, 6147);

            // 测试大数据集（更多块）
            let data = vec![b'd'; 100000];
            let reader = Box::new(Cursor::new(data));
            let (blocks, checksum) = sum_sysv(reader);
            assert_eq!(blocks, 196);
            assert_eq!(checksum, 38680);

            // 测试重复字符数据
            let data = b"aaaaaa";
            let reader = Box::new(Cursor::new(data));
            let (blocks, checksum) = sum_sysv(reader);
            assert_eq!(blocks, 1);
            assert_eq!(checksum, 6 * 'a' as u16);

            // 测试极端数据
            let data = vec![255u8; 5000];
            let reader = Box::new(Cursor::new(data));
            let (blocks, checksum) = sum_sysv(reader);
            assert_eq!(blocks, 10);
            assert_eq!(checksum, 29835);
        }
    }

    #[cfg(test)]
    mod open_tests {
        use super::*;
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        use tempfile::{NamedTempFile, tempdir};

        #[test]
        fn test_open_stdin() {
            // 模拟从标准输入读取
            let result = sum_open("-");
            assert!(result.is_ok());
        }

        #[test]
        fn test_open_file() {
            // 创建一个临时文件，并写入一些数据
            let mut file = NamedTempFile::new().unwrap();
            writeln!(file, "Hello, world!").unwrap();
            let file_path = file.path().to_str().unwrap();

            // 测试打开文件
            let result = sum_open(file_path);
            assert!(result.is_ok());

            // 验证读取的内容
            let mut reader = result.unwrap();
            let mut content = String::new();
            reader.read_to_string(&mut content).unwrap();
            assert_eq!(content, "Hello, world!\n");
        }

        #[test]
        fn test_open_directory() {
            // 创建一个临时目录
            let dir = tempdir().unwrap();
            let dir_path = dir.path().to_str().unwrap();

            // 测试打开目录
            let result = sum_open(dir_path);
            assert!(result.is_err());

            // 验证错误信息
            if let Err(error) = result {
                assert_eq!(error.to_string(), format!("{}: Is a directory", dir_path));
            }
        }

        #[test]
        fn test_open_non_existent_file() {
            // 测试打开不存在的文件
            let result = sum_open("non_existent_file.txt");
            assert!(result.is_err());

            // 验证错误信息
            if let Err(error) = result {
                assert_eq!(
                    error.to_string(),
                    "non_existent_file.txt: No such file or directory"
                );
            }
        }

        #[test]
        fn test_open_special_characters_filename() {
            // 创建一个带有特殊字符的临时文件，并写入一些数据
            let file = NamedTempFile::new().unwrap();
            let special_filename = file
                .path()
                .with_extension("!@#$%^&*()")
                .to_str()
                .unwrap()
                .to_string();
            std::fs::rename(file.path(), &special_filename).unwrap();

            // 打开这个带有特殊字符的文件
            let result = sum_open(&special_filename);
            assert!(result.is_ok());

            // 验证读取的内容
            let mut reader = result.unwrap();
            let mut content = String::new();
            reader.read_to_string(&mut content).unwrap();
            assert!(content.is_empty()); // 因为没有写入数据，内容应该是空的
        }

        #[test]
        fn test_open_file_with_long_name() {
            // 创建一个非常长名字的临时文件，并写入一些数据
            let long_filename = "a".repeat(255);
            let file = NamedTempFile::new().unwrap();
            let long_filepath = file.path().with_file_name(&long_filename);
            std::fs::rename(file.path(), &long_filepath).unwrap();

            // 测试打开这个长名字的文件
            let result = sum_open(long_filepath.to_str().unwrap());
            assert!(result.is_ok());

            // 验证读取的内容
            let mut reader = result.unwrap();
            let mut content = String::new();
            reader.read_to_string(&mut content).unwrap();
            assert!(content.is_empty()); // 因为没有写入数据，内容应该是空的
        }

        #[test]
        fn test_open_file_no_permissions() {
            // 创建一个临时文件，并移除所有权限
            let file = NamedTempFile::new().unwrap();
            let file_path = file.path().to_str().unwrap();
            std::fs::set_permissions(file_path, std::fs::Permissions::from_mode(0o000)).unwrap();

            // 尝试打开没有权限的文件
            let result = sum_open(file_path);
            assert!(result.is_ok());

            // 验证错误信息
            if let Err(error) = result {
                assert!(error.to_string().contains("Permission denied"));
            }
        }

        #[test]
        fn test_open_symlink() {
            // 创建一个临时文件
            let mut file = NamedTempFile::new().unwrap();
            writeln!(file, "Hello, symlink!").unwrap();
            let file_path = file.path().to_str().unwrap();

            // 创建一个临时目录
            let dir = tempdir().unwrap();
            let symlink_path = dir.path().join("symlink");

            // 创建指向临时文件的符号链接
            #[cfg(unix)]
            std::os::unix::fs::symlink(file_path, &symlink_path).unwrap();

            #[cfg(windows)]
            std::os::windows::fs::symlink_file(file_path, &symlink_path).unwrap();

            // 测试打开符号链接
            let result = sum_open(symlink_path.to_str().unwrap());
            assert!(result.is_ok());

            // 验证读取的内容
            let mut reader = result.unwrap();
            let mut content = String::new();
            reader.read_to_string(&mut content).unwrap();
            assert_eq!(content, "Hello, symlink!\n");
        }
    }
    #[cfg(test)]
    mod ct_main_tests {
        use super::*;
        use std::ffi::OsString;
        use std::io::Write;
        use tempfile::tempdir;
        #[test]
        fn test_sum_main_execution_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = sum_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_sum_main_execution_other_version() {
            let args = vec![ctcore::ct_util_name(), "-V"];
            let result = sum_main(args.iter().map(|s| OsString::from(s)));

            assert!(result.is_err());
        }

        #[test]
        fn test_sum_main_execution_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = sum_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_sum_main_execution_help_short() {
            let args = vec![ctcore::ct_util_name(), "-h"];
            let result = sum_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_sum_main_execution_unsupport_help() {
            let args = vec![ctcore::ct_util_name(), "-H"];
            let result = sum_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_sum_main_invalid_argument() {
            let args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = sum_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_sum_main_default() {
            let filename = "test_sum_main_default";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), file_name];
            let result = sum_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_sum_main_bsd_sum_algorithm_short() {
            let filename = "test_sum_main_bsd_sum_algorithm_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-r", file_name];
            let result = sum_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_sum_main_system_v_sum_algorithm_short() {
            let filename = "test_sum_main_system_v_sum_algorithm_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-s", file_name];
            let result = sum_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_sum_main_bsd_sum_algorithm_long() {
            let filename = "test_sum_main_bsd_sum_algorithm_short";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join(filename);
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "test\nctyunos\nhello\nworld\n").unwrap();
            let file_name = file_path.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--sysv", file_name];
            let result = sum_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }
    }
    #[cfg(test)]
    mod ct_app_tests {
        use clap::error::ErrorKind;

        use super::*;

        // sum 接口: sum [OPTION]... [FILE]...
        //
        // Options:
        //   -r             use the BSD sum algorithm, use 1K blocks (default)
        //   -s, --sysv     use System V sum algorithm, use 512 bytes blocks
        //   -h, --help     Print help
        //   -V, --version  Print version

        #[test]
        fn test_ct_app_execution_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_other_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];

            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_help() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "--help"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_execution_help_short() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-h"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_execution_unsupport_help() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-H"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_invalid_argument() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_support_missing_argument() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name()];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_bsd_sum_algorithm_short() {
            let filename = "test_ct_app_bsd_sum_algorithm_short";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-r", filename];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_system_v_sum_algorithm_short() {
            let filename = "test_ct_app_system_v_sum_algorithm_short";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-s", filename];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_bsd_sum_algorithm_long() {
            let filename = "test_ct_app_bsd_sum_algorithm_short";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--sysv", filename];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }
    }
}
