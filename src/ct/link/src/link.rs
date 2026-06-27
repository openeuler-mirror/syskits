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

//! 创建硬链接
//!
//! 调用 link 函数，创建一个名为 <文件2> 的硬链接，指向现有的文件 <文件1>。

extern crate rust_i18n;
use clap::builder::ValueParser;
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "zh-CN");
use clap::{Arg, Command, crate_version};
use ctcore::{
    Tool,
    ct_display::Quotable,
    ct_error::{CTResult, CtSimpleError, FromIo},
};
use std::{ffi::OsString, fs::hard_link, path::Path};
use sys_locale::get_locale;

mod link_flags {
    pub const FILES: &str = "FILES";
}

/// 存储链接操作的源文件和目标文件路径
struct LinkFlags<'a> {
    source_path: &'a Path,
    target_path: &'a Path,
}

impl<'a> LinkFlags<'a> {
    /// 从命令行参数创建 LinkFlags 实例
    ///
    /// # Arguments
    /// * `matches` - 解析后的命令行参数
    ///
    /// # Returns
    /// * `CTResult<Self>` - 成功则返回 LinkFlags 实例，失败则返回错误
    fn new(matches: &'a clap::ArgMatches) -> CTResult<Self> {
        let files: Vec<_> = matches
            .get_many::<OsString>(link_flags::FILES)
            .unwrap_or_default()
            .collect();

        if files.len() != 2 {
            return Err(CtSimpleError::new(1, "wrong number of arguments"));
        }

        Ok(Self {
            source_path: Path::new(files[0]),
            target_path: Path::new(files[1]),
        })
    }
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    link_main(args)
}

/// 主函数：解析参数并执行链接操作
pub fn link_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let matches = ct_app().try_get_matches_from(args)?;
    let flags = LinkFlags::new(&matches)?;
    link_exec(&flags)
}

/// 执行硬链接创建操作
fn link_exec(flags: &LinkFlags) -> CTResult<()> {
    hard_link(flags.source_path, flags.target_path).map_err_context(|| {
        format!(
            "cannot create link {} to {}",
            flags.target_path.quote(),
            flags.source_path.quote()
        )
    })
}

/// 创建命令行应用程序配置
pub fn ct_app() -> Command {
    let arg = Arg::new(link_flags::FILES)
        .hide(true)
        .required(true)
        .num_args(2)
        .value_hint(clap::ValueHint::AnyPath)
        .value_parser(ValueParser::os_string());

    Command::new(ctcore::ct_util_name())
        .version(crate_version!())
        .about(t!("link.about"))
        .override_usage(t!("link.usage"))
        .infer_long_args(true)
        .arg(arg)
}

#[derive(Default)]
pub struct Link;
impl Tool for Link {
    fn name(&self) -> &'static str {
        "link"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        // 将&[OsString]转换为符合Args trait要求的iterator
        link_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::os::unix::fs::MetadataExt;
    use std::path::PathBuf;
    use tempfile::Builder;

    #[test]
    fn test_tool_implementation() {
        let tool = Link::default();

        // 测试 name 方法
        assert_eq!(tool.name(), "link");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("link"));

        // 测试 execute 方法
        let args = vec![OsString::from("link"), OsString::from("--help")];
        let result = tool.execute(&args);
        assert!(result.is_err()); // link命令需要参数，所以不带参数应该返回错误
    }

    mod ct_app_tests {
        /*
        测试版本和帮助标志
        测试无效参数
        测试缺少必需参数
        测试有效参数

        短版本标志测试 (-V)
        短帮助标志测试 (-h)
        参数过多的测试
        */
        use super::*;
        use clap::error::ErrorKind;

        #[test]
        fn test_ct_app_execution_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_help() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_invalid_argument() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_missing_required_args() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name()];
            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().kind(),
                ErrorKind::MissingRequiredArgument
            );
        }

        #[test]
        fn test_ct_app_valid_args() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "source.txt", "target.txt"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_execution_short_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_short_help() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-h"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_too_many_args() {
            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "source.txt",
                "target.txt",
                "extra.txt",
            ];
            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
        }
    }

    mod link_exec_tests {
        /*
        测试成功创建硬链接
        测试源文件不存在的情况
        测试目标文件已存在的情况

        目录作为源文件的测试
        源文件和目标文件相同的测试
        目标文件在不存在目录中的测试
        验证硬链接创建成功的测试
         */
        use super::*;

        fn setup_test_files() -> (tempfile::TempDir, PathBuf, PathBuf) {
            let temp_dir = Builder::new().prefix("link_test").tempdir().unwrap();
            let source_file = temp_dir.path().join("source.txt");
            let target_file = temp_dir.path().join("target.txt");
            File::create(&source_file).unwrap();
            (temp_dir, source_file, target_file)
        }

        #[test]
        fn test_link_exec_success() {
            let (_temp_dir, source_file, target_file) = setup_test_files();
            let flags = LinkFlags {
                source_path: source_file.as_path(),
                target_path: target_file.as_path(),
            };
            let result = link_exec(&flags);
            assert!(result.is_ok());
        }

        #[test]
        fn test_link_exec_nonexistent_source() {
            let temp_dir = Builder::new().prefix("link_test").tempdir().unwrap();
            let source_file = temp_dir.path().join("nonexistent.txt");
            let target_file = temp_dir.path().join("target.txt");

            let flags = LinkFlags {
                source_path: source_file.as_path(),
                target_path: target_file.as_path(),
            };
            let result = link_exec(&flags);
            assert!(result.is_err());
        }

        #[test]
        fn test_link_exec_existing_target() {
            let (_temp_dir, source_file, target_file) = setup_test_files();
            File::create(&target_file).unwrap();

            let flags = LinkFlags {
                source_path: source_file.as_path(),
                target_path: target_file.as_path(),
            };
            let result = link_exec(&flags);
            assert!(result.is_err());
        }

        #[test]
        fn test_link_exec_directory_as_source() {
            let temp_dir = Builder::new().prefix("link_test").tempdir().unwrap();
            let source_dir = temp_dir.path().join("source_dir");
            let target_file = temp_dir.path().join("target.txt");
            std::fs::create_dir(&source_dir).unwrap();

            let flags = LinkFlags {
                source_path: source_dir.as_path(),
                target_path: target_file.as_path(),
            };
            let result = link_exec(&flags);
            assert!(result.is_err());
        }

        #[test]
        fn test_link_exec_same_file() {
            let (_temp_dir, source_file, _) = setup_test_files();
            let flags = LinkFlags {
                source_path: source_file.as_path(),
                target_path: source_file.as_path(),
            };
            let result = link_exec(&flags);
            assert!(result.is_err());
        }

        #[test]
        fn test_link_exec_target_in_nonexistent_directory() {
            let (_temp_dir, source_file, _) = setup_test_files();
            let target_file = Path::new("/nonexistent/directory/target.txt");

            let flags = LinkFlags {
                source_path: source_file.as_path(),
                target_path: target_file,
            };
            let result = link_exec(&flags);
            assert!(result.is_err());
        }

        #[test]
        fn test_link_exec_verify_hard_link() {
            let (_temp_dir, source_file, target_file) = setup_test_files();
            let flags = LinkFlags {
                source_path: source_file.as_path(),
                target_path: target_file.as_path(),
            };
            link_exec(&flags).unwrap();

            // 验证是否创建了硬链接
            let source_metadata = std::fs::metadata(&source_file).unwrap();
            let target_metadata = std::fs::metadata(&target_file).unwrap();
            assert_eq!(source_metadata.nlink(), 2);
            assert_eq!(target_metadata.nlink(), 2);
        }
    }

    mod link_main_tests {
        /*
        测试成功执行
        测试参数数量错误
        测试源文件不存在
        测试帮助标志
        空参数测试
        带空格的路径测试
        相对路径测试
        Unicode 路径测试
        长路径名测试
        */
        use super::*;

        fn setup_test_files() -> (tempfile::TempDir, String, String) {
            let temp_dir = Builder::new().prefix("link_test").tempdir().unwrap();
            let source_file = temp_dir.path().join("source.txt");
            let target_file = temp_dir.path().join("target.txt");
            File::create(&source_file).unwrap();
            (
                temp_dir,
                source_file.to_str().unwrap().to_string(),
                target_file.to_str().unwrap().to_string(),
            )
        }

        #[test]
        fn test_link_main_success() {
            let (_temp_dir, source, target) = setup_test_files();
            let args = vec![ctcore::ct_util_name(), &source, &target];
            let result = link_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_link_main_wrong_number_of_arguments() {
            let args = vec![ctcore::ct_util_name(), "single_file.txt"];
            let result = link_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_link_main_nonexistent_source() {
            let args = vec![ctcore::ct_util_name(), "nonexistent.txt", "target.txt"];
            let result = link_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_link_main_with_help_flag() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = link_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_link_main_empty_arguments() {
            let args = vec![ctcore::ct_util_name()];
            let result = link_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
        }

        #[test]
        fn test_link_main_with_spaces_in_paths() {
            let temp_dir = Builder::new().prefix("link_test").tempdir().unwrap();
            let source_file = temp_dir.path().join("source file.txt");
            let target_file = temp_dir.path().join("target file.txt");
            File::create(&source_file).unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                source_file.to_str().unwrap(),
                target_file.to_str().unwrap(),
            ];
            let result = link_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_link_main_with_relative_paths() {
            let (_temp_dir, source, target) = setup_test_files();
            let current_dir = std::env::current_dir().unwrap();
            let relative_source = Path::new(&source)
                .strip_prefix(&current_dir)
                .unwrap_or(Path::new(&source));
            let relative_target = Path::new(&target)
                .strip_prefix(&current_dir)
                .unwrap_or(Path::new(&target));

            let args = vec![
                ctcore::ct_util_name(),
                relative_source.to_str().unwrap(),
                relative_target.to_str().unwrap(),
            ];
            let result = link_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_link_main_with_unicode_paths() {
            let temp_dir = Builder::new().prefix("link_test").tempdir().unwrap();
            let source_file = temp_dir.path().join("源文件.txt");
            let target_file = temp_dir.path().join("目标文件.txt");
            File::create(&source_file).unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                source_file.to_str().unwrap(),
                target_file.to_str().unwrap(),
            ];
            let result = link_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }

        #[test]
        fn test_link_main_with_long_paths() {
            let temp_dir = Builder::new().prefix("link_test").tempdir().unwrap();
            let long_name = "a".repeat(100);
            let source_file = temp_dir.path().join(format!("{}.txt", long_name));
            let target_file = temp_dir.path().join(format!("{}_target.txt", long_name));
            File::create(&source_file).unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                source_file.to_str().unwrap(),
                target_file.to_str().unwrap(),
            ];
            let result = link_main(args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
        }
    }
}
