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
use std::ffi::OsString;
rust_i18n::i18n!("locales", fallback = "en-US");
use std::path::Path;

use clap::Command;
use sys_locale::get_locale;

use ct_ls::{LsConfig, LsDereference, LsFormat, PathData, ls_flags};

use ctcore::Tool;
use ctcore::ct_error::CTResult;
use ctcore::ct_quoting_style::{CtQuotes, CtQuotingStyle};

pub fn vdir_main(args: impl ctcore::Args) -> CTResult<(Vec<PathData>, Vec<PathData>)> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);

    let command = ct_app();
    let matches = command.get_matches_from(args);

    let mut default_quoting_style = false;
    let mut default_format_style = false;

    // 我们会检查是否给出了格式化或引号样式标志。
    // 如果没有，我们将使用 dir 默认的格式化和引用样式标志
    if !matches.contains_id(ls_flags::LS_QUOTING_STYLE)
        && !matches.get_flag(ls_flags::quoting::LS_C)
        && !matches.get_flag(ls_flags::quoting::LS_ESCAPE)
        && !matches.get_flag(ls_flags::quoting::LS_LITERAL)
        && !matches.get_flag(ls_flags::LS_ZERO)
    {
        default_quoting_style = true;
    }
    if !matches.contains_id(ls_flags::LS_FORMAT)
        && !matches.get_flag(ls_flags::format::LS_ACROSS)
        && !matches.get_flag(ls_flags::format::LS_COLUMNS)
        && !matches.get_flag(ls_flags::format::LS_COMMAS)
        && !matches.get_flag(ls_flags::format::LS_LONG)
        && !matches.get_flag(ls_flags::format::LS_LONG_NO_GROUP)
        && !matches.get_flag(ls_flags::format::LS_LONG_NO_OWNER)
        && !matches.get_flag(ls_flags::format::LS_LONG_NUMERIC_UID_GID)
        && !matches.get_flag(ls_flags::format::LS_ONE_LINE)
        && !matches.get_flag(ls_flags::LS_ZERO)
    {
        default_format_style = true;
    }

    let mut config = LsConfig::from(&matches)?;

    if default_quoting_style {
        config.quoting_style = CtQuotingStyle::C {
            quotes: CtQuotes::None,
        };
    }
    if default_format_style {
        config.format = LsFormat::Long;
        if matches.get_flag(ls_flags::LS_DIRED) {
            config.is_dired = true;
        }
        if !matches.get_flag(ls_flags::dereference::LS_ALL)
            && !matches.get_flag(ls_flags::dereference::LS_ARGS)
            && !matches.get_flag(ls_flags::dereference::LS_DIR_ARGS)
        {
            config.dereference = LsDereference::LsNone;
        }
    }

    let paths_list = matches.get_many::<OsString>(ls_flags::LS_PATHS);
    let paths_from_args: Vec<_> = paths_list
        .map(|v| v.map(Path::new).collect())
        .unwrap_or_else(|| vec![Path::new(".")]);

    ct_ls::list(paths_from_args, &config)
}

pub fn ct_app() -> Command {
    ct_ls::ct_app()
}

#[derive(Default)]
pub struct Vdir;
impl Tool for Vdir {
    fn name(&self) -> &'static str {
        "vdir"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        vdir_main(args.iter().cloned()).map(|_| ())
    }
}

#[cfg(test)]
#[allow(clippy::needless_borrow)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Vdir;

        // Test name method
        assert_eq!(tool.name(), "vdir");

        // Test command method
        let command = tool.command();
        assert!(command.get_name().contains("vdir"));

        // Test execute method with help flag (should work)
        let args: Vec<OsString> = vec![OsString::from("vdir"), OsString::from("--help")];
        let result = tool.execute(&args);
        assert!(result.is_ok());
    }

    #[cfg(test)]
    mod ct_main_tests {
        use std::fs::File;
        use std::io::Write;

        use tempfile::TempDir;

        use super::*;

        #[test]
        fn test_ctmain_input_err_no_app_name_v() {
            let args = ["--version", ""];
            let result = vdir_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_ctmain_input_err_no_app_name_uppercase_v() {
            let args = ["-V", ""];
            let result = vdir_main(args.iter().map(OsString::from));
            //println!("{}", result);
            assert!(result.is_err());
        }

        #[test]
        fn test_ctmain_return() {
            let args = [ctcore::ct_util_name()];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                }
            }
        }

        #[test]
        fn test_ctmain_vdir_dir_return() {
            let args = [ctcore::ct_util_name(), "./"];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                }
            }
        }

        // vdir 文件测试
        #[test]
        fn test_ct_main_with_vdir_file() {
            let content = "hello world\nhello rust\n";
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            let mut file = File::create(&test_file_path).unwrap();
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), test_file_path.to_str().unwrap()];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    assert!(!file_vec.is_empty());
                    assert!(dir_vec.is_empty());
                }
            }
        }

        #[test]
        fn test_ct_main_execution_a() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-a", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_execution_all() {
            // 创建临时目录结构
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-all", dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    println!("{file_vec:?}, {dir_vec:?}");
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_execution_block_size() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "--block-size=1", dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_support_missing_argument() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "--block-size=1", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_format_long() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "--format=long", dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_columns_short() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-C", dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_long_short() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-l", dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_across_short() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-x", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_tab_size_short() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-T", "4", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_tab_size_long() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "--tabsize=8", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_commas_short() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-m", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_one_line_short() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-1", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_long_no_group_short() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-o", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_long_no_owner_short() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-g", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_long_numeric_uid_gid_short() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-n", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_long_numeric_uid_gid_long() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "--numeric-uid-gid", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_quoting_style_long() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "--quoting-style=literal", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_literal_short() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-N", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_escape_short() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-b", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_c_quoting_short() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-Q", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_hide_control_chars_short() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-q", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_show_control_chars_long() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "--show-control-chars", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_time_long() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "--time=access", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_hide_long() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "--hide=*", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_ignore_short() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-I", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_ignore_backups_short() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-B", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_change_short_c() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-c", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_access_short_u() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-u", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_ignore_short_uppercase_i() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-I", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_sort_long() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "--sort=size", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_size_short() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-S", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_time_sort_short() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-t", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_extension_short() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-X", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_none_sort_short() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-U", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_dereference_all_short() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-L", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_dereference_dir_args_long() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [
                ctcore::ct_util_name(),
                "--dereference-command-line-symlink-to-dir",
                &dir_name,
            ];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_dereference_args_short() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-H", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_no_group_short() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-G", dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                    assert_eq!(0, file_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_all_files_short() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-a", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_almost_all_short() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-A", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_directory_short() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-d", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(!file_vec.is_empty());
                    assert!(dir_vec.is_empty());
                    assert_eq!(0, dir_vec.len());
                    assert_eq!(1, file_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_kibibytes_short() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-k", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_si_long() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "--si", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_inode_short() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-i", dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                    assert_eq!(0, file_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_reverse_short() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-r", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }

        #[test]
        fn test_ct_main_recursive_short() {
            let file_name = "test_vdir_file";
            let tmp_dir = TempDir::with_prefix("test_vdir-").unwrap();
            let temp_dir_path = tmp_dir.path();
            let test_file_path = temp_dir_path.join(file_name);
            File::create(&test_file_path).unwrap();
            let dir_name = temp_dir_path.to_str().unwrap();
            let args = [ctcore::ct_util_name(), "-R", &dir_name];
            let result = vdir_main(args.iter().map(OsString::from));

            // 使用模式匹配提取字段值
            match result {
                Err(output) => {
                    panic!("err: {output}")
                }
                Ok((file_vec, dir_vec)) => {
                    //println!("{:?}, {:?}", file_vec, dir_vec);
                    assert!(file_vec.is_empty());
                    assert!(!dir_vec.is_empty());
                    assert_eq!(1, dir_vec.len());
                }
            }
        }
    }

    #[cfg(test)]
    mod ct_app_tests {
        use clap::error::ErrorKind;

        use super::*;

        // vdir 接口: vdir [OPTION]... [FILE]...
        // List information about the FILEs (the current directory by default).
        // Sort entries alphabetically if none of -cftuvSUX nor --sort is specified.
        //
        // Mandatory arguments to long options are mandatory for short options too.
        //   -a, --all                  do not ignore entries starting with .
        //   -A, --almost-all           do not list implied . and ..
        //       --author               with -l, print the author of each file
        //   -b, --escape               print C-style escapes for nongraphic characters
        //       --block-size=SIZE      with -l, scale sizes by SIZE when printing them;
        //                                e.g., '--block-size=M'; see SIZE format below
        //   -B, --ignore-backups       do not list implied entries ending with ~
        //   -c                         with -lt: sort by, and show, ctime (time of last
        //                                modification of file status information);
        //                                with -l: show ctime and sort by name;
        //                                otherwise: sort by ctime, newest first
        //   -C                         list entries by columns
        //       --color[=WHEN]         colorize the output; WHEN can be 'always' (default
        //                                if omitted), 'auto', or 'never'; more info below
        //   -d, --directory            list directories themselves, not their contents
        //   -D, --dired                generate output designed for Emacs' dired mode
        //   -f                         list all entries in directory order
        //   -F, --classify[=WHEN]      append indicator (one of */=>@|) to entries;
        //                                WHEN can be 'always' (default if omitted),
        //                                'auto', or 'never'
        //       --file-type            likewise, except do not append '*'
        //       --format=WORD          across -x, commas -m, horizontal -x, long -l,
        //                                single-column -1, verbose -l, vertical -C
        //       --full-time            like -l --time-style=full-iso
        //   -g                         like -l, but do not list owner
        //       --group-directories-first
        //                              group directories before files;
        //                                can be augmented with a --sort option, but any
        //                                use of --sort=none (-U) disables grouping
        //   -G, --no-group             in a long listing, don't print group names
        //   -h, --human-readable       with -l and -s, print sizes like 1K 234M 2G etc.
        //       --si                   likewise, but use powers of 1000 not 1024
        //   -H, --dereference-command-line
        //                              follow symbolic links listed on the command line
        //       --dereference-command-line-symlink-to-dir
        //                              follow each command line symbolic link
        //                                that points to a directory
        //       --hide=PATTERN         do not list implied entries matching shell PATTERN
        //                                (overridden by -a or -A)
        //       --hyperlink[=WHEN]     hyperlink file names; WHEN can be 'always'
        //                                (default if omitted), 'auto', or 'never'
        //       --indicator-style=WORD  append indicator with style WORD to entry names:
        //                                none (default), slash (-p),
        //                                file-type (--file-type), classify (-F)
        //   -i, --inode                print the index number of each file
        //   -I, --ignore=PATTERN       do not list implied entries matching shell PATTERN
        //   -k, --kibibytes            default to 1024-byte blocks for file system usage;
        //                                used only with -s and per directory totals
        //   -l                         use a long listing format
        //   -L, --dereference          when showing file information for a symbolic
        //                                link, show information for the file the link
        //                                references rather than for the link itself
        //   -m                         fill width with a comma separated list of entries
        //   -n, --numeric-uid-gid      like -l, but list numeric user and group IDs
        //   -N, --literal              print entry names without quoting
        //   -o                         like -l, but do not list group information
        //   -p, --indicator-style=slash
        //                              append / indicator to directories
        //   -q, --hide-control-chars   print ? instead of nongraphic characters
        //       --show-control-chars   show nongraphic characters as-is (the default,
        //                                unless program is 'ls' and output is a terminal)
        //   -Q, --quote-name           enclose entry names in double quotes
        //       --quoting-style=WORD   use quoting style WORD for entry names:
        //                                literal, locale, shell, shell-always,
        //                                shell-escape, shell-escape-always, c, escape
        //                                (overrides QUOTING_STYLE environment variable)
        //   -r, --reverse              reverse order while sorting
        //   -R, --recursive            list subdirectories recursively
        //   -s, --size                 print the allocated size of each file, in blocks
        //   -S                         sort by file size, largest first
        //       --sort=WORD            sort by WORD instead of name: none (-U), size (-S),
        //                                time (-t), version (-v), extension (-X), width
        //       --time=WORD            change the default of using modification times;
        //                                access time (-u): atime, access, use;
        //                                change time (-c): ctime, status;
        //                                birth time: birth, creation;
        //                              with -l, WORD determines which time to show;
        //                              with --sort=time, sort by WORD (newest first)
        //       --time-style=TIME_STYLE  time/date format with -l; see TIME_STYLE below
        //   -t                         sort by time, newest first; see --time
        //   -T, --tabsize=COLS         assume tab stops at each COLS instead of 8
        //   -u                         with -lt: sort by, and show, access time;
        //                                with -l: show access time and sort by name;
        //                                otherwise: sort by access time, newest first
        //   -U                         do not sort; list entries in directory order
        //   -v                         natural sort of (version) numbers within text
        //   -w, --width=COLS           set output width to COLS.  0 means no limit
        //   -x                         list entries by lines instead of by columns
        //   -X                         sort alphabetically by entry extension
        //   -Z, --context              print any security context of each file
        //       --zero                 end each output line with NUL, not newline
        //   -1                         list one file per line
        //       --help     display this help and exit
        //       --version  output version information and exit

        #[test]
        fn test_ct_app_execution_version() {
            let command = ct_app();

            // 测试用例1：有效输入
            let args = vec![ctcore::ct_util_name(), "--version"];

            // Assuming `command` has a method to retrieve the executable name, replace it with the actual one
            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_other_version() {
            let command = ct_app();

            // 测试用例1：有效输入
            let args = vec![ctcore::ct_util_name(), "-V"];

            // Assuming `command` has a method to retrieve the executable name, replace it with the actual one
            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_help() {
            let command = ct_app();

            // 测试用例2：验证 --help 参数是否正确处理
            let help_args = vec![ctcore::ct_util_name(), "--help"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_execution_not_support_help() {
            let command = ct_app();

            // 测试用例2：验证 --help 参数是否正确处理
            let help_args = vec![ctcore::ct_util_name(), "-H"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_invalid_argument() {
            let command = ct_app();

            // 测试用例3：验证当提供未知参数时是否正确报错
            let invalid_args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_support_missing_argument() {
            let command = ct_app();

            // 测试用例4：验证当缺少必需的参数时是否正确报错
            let missing_args = vec![ctcore::ct_util_name()]; // 缺少任何参数
            let result = command.try_get_matches_from(missing_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_help_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--help"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_err());
            assert_eq!(
                matches.unwrap_err().kind(),
                clap::error::ErrorKind::DisplayHelp
            );
        }

        #[test]
        fn test_ct_app_format_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--format=long"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            // assert!(matches.unwrap().args_present(flags::LS_FORMAT));
            assert!(matches.unwrap().contains_id(ls_flags::LS_FORMAT));
        }

        #[test]
        fn test_ct_app_columns_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-C"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::format::LS_COLUMNS));
        }

        #[test]
        fn test_ct_app_long_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-l"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::format::LS_LONG));
        }

        #[test]
        fn test_ct_app_across_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-x"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::format::LS_ACROSS));
        }

        #[test]
        fn test_ct_app_tab_size_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-T", "4"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert_eq!(
                matches
                    .unwrap()
                    .get_one::<String>(ls_flags::format::LS_TAB_SIZE)
                    .unwrap(),
                "4"
            );
        }

        #[test]
        fn test_ct_app_tab_size_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--tabsize=8"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert_eq!(
                matches
                    .unwrap()
                    .get_one::<String>(ls_flags::format::LS_TAB_SIZE)
                    .unwrap(),
                "8"
            );
        }

        #[test]
        fn test_ct_app_commas_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-m"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::format::LS_COMMAS));
        }

        #[test]
        fn test_ct_app_one_line_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-1"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::format::LS_ONE_LINE));
        }

        #[test]
        fn test_ct_app_long_no_group_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-o"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(
                matches
                    .unwrap()
                    .contains_id(ls_flags::format::LS_LONG_NO_GROUP)
            );
        }

        #[test]
        fn test_ct_app_long_no_owner_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-g"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(
                matches
                    .unwrap()
                    .contains_id(ls_flags::format::LS_LONG_NO_OWNER)
            );
        }

        #[test]
        fn test_ct_app_long_no_owner_short_help() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-g", "--help"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_err());
            assert_eq!(
                matches.unwrap_err().kind(),
                clap::error::ErrorKind::DisplayHelp
            );
        }

        #[test]
        fn test_ct_app_long_numeric_uid_gid_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-n"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(
                matches
                    .unwrap()
                    .contains_id(ls_flags::format::LS_LONG_NUMERIC_UID_GID)
            );
        }

        #[test]
        fn test_ct_app_long_numeric_uid_gid_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--numeric-uid-gid"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(
                matches
                    .unwrap()
                    .contains_id(ls_flags::format::LS_LONG_NUMERIC_UID_GID)
            );
        }

        #[test]
        fn test_ct_app_quoting_style_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--quoting-style=literal"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_QUOTING_STYLE));
        }

        #[test]
        fn test_ct_app_literal_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-N"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::quoting::LS_LITERAL));
        }

        #[test]
        fn test_ct_app_escape_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-b"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::quoting::LS_ESCAPE));
        }

        #[test]
        fn test_ct_app_c_quoting_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-Q"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::quoting::LS_C));
        }

        #[test]
        fn test_ct_app_hide_control_chars_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-q"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(
                matches
                    .unwrap()
                    .contains_id(ls_flags::LS_HIDE_CONTROL_CHARS)
            );
        }

        #[test]
        fn test_ct_app_show_control_chars_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--show-control-chars"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(
                matches
                    .unwrap()
                    .contains_id(ls_flags::LS_SHOW_CONTROL_CHARS)
            );
        }

        #[test]
        fn test_ct_app_time_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--time=access"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_TIME));
        }

        #[test]
        fn test_ct_app_change_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-c"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::time::LS_CHANGE));
        }

        #[test]
        fn test_ct_app_access_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-u"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::time::LS_ACCESS));
        }

        #[test]
        fn test_ct_app_hide_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--hide=*"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_HIDE));
        }

        #[test]
        fn test_ct_app_ignore_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-I", "*.tmp"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_IGNORE));
        }

        #[test]
        fn test_ct_app_ignore_backups_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-B"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_IGNORE_BACKUPS));
        }

        #[test]
        fn test_ct_app_change_short_c() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-c"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::time::LS_CHANGE));
        }

        #[test]
        fn test_ct_app_access_short_u() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-u"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::time::LS_ACCESS));
        }

        #[test]
        fn test_ct_app_ignore_short_uppercase_i() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-I", "*.tmp"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_IGNORE));
        }

        #[test]
        fn test_ct_app_sort_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--sort=size"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_SORT));
        }

        #[test]
        fn test_ct_app_size_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-S"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::sort::LS_SIZE));
        }

        #[test]
        fn test_ct_app_time_sort_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-t"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::sort::LS_TIME));
        }

        #[test]
        fn test_ct_app_version_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-v"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::sort::LS_VERSION));
        }

        #[test]
        fn test_ct_app_extension_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-X"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::sort::LS_EXTENSION));
        }

        #[test]
        fn test_ct_app_none_sort_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-U"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::sort::LS_NONE));
        }

        #[test]
        fn test_ct_app_dereference_all_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-L"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::dereference::LS_ALL));
        }

        #[test]
        fn test_ct_app_dereference_dir_args_long() {
            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--dereference-command-line-symlink-to-dir",
            ];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(
                matches
                    .unwrap()
                    .contains_id(ls_flags::dereference::LS_DIR_ARGS)
            );
        }

        #[test]
        fn test_ct_app_dereference_args_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-H"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::dereference::LS_ARGS));
        }

        #[test]
        fn test_ct_app_no_group_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-G"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_NO_GROUP));
        }

        #[test]
        fn test_ct_app_all_files_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-a"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::files::LS_ALL));
        }

        #[test]
        fn test_ct_app_almost_all_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-A"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::files::LS_ALMOST_ALL));
        }

        #[test]
        fn test_ct_app_directory_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-d"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_DIRECTORY));
        }

        #[test]
        fn test_ct_app_human_readable_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-h"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(
                matches
                    .unwrap()
                    .contains_id(ls_flags::size::LS_HUMAN_READABLE)
            );
        }

        #[test]
        fn test_ct_app_kibibytes_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-k"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::size::LS_KIBIBYTES));
        }

        #[test]
        fn test_ct_app_si_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--si"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::size::LS_SI));
        }

        #[test]
        fn test_ct_app_block_size_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--block-size=1024"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::size::LS_BLOCK_SIZE));
        }

        #[test]
        fn test_ct_app_inode_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-i"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_INODE));
        }

        #[test]
        fn test_ct_app_reverse_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-r"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_REVERSE));
        }

        #[test]
        fn test_ct_app_recursive_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-R"];
            let matches = command.try_get_matches_from(args);
            assert!(matches.is_ok());
            assert!(matches.unwrap().contains_id(ls_flags::LS_RECURSIVE));
        }
    }
}
