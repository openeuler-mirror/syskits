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

//! pwd命令, 在Linux和其他类Unix系统中用于显示当前工作目录的绝对路径。

use clap::ArgAction;
use clap::{Arg, Command, crate_version};
use ctcore::{ct_format_usage, ct_help_about, ct_help_usage};
use std::env;
use std::io;
use std::path::PathBuf;

use ctcore::ct_display::ct_println_verbatim;
use ctcore::ct_error::{CTResult, FromIo};

const PWD_ABOUT: &str = ct_help_about!("pwd.md");
const PWD_USAGE: &str = ct_help_usage!("pwd.md");

pub mod pwd_flags {
    pub const PWD_LOGICAL: &str = "logical";
    pub const PWD_PHYSICAL: &str = "physical";
    pub const PWD_ARG_OTHERS: &str = "others";
}
fn pwd_physical_path() -> io::Result<PathBuf> {
    // std::env::current_dir() 是 libc::getcwd() 的一个包装。

    // 在 Unix 上，getcwd() 必须返回物理路径：
    // https://pubs.opengroup.org/onlinepubs/9699919799/functions/getcwd.html
    #[cfg(unix)]
    {
        env::current_dir()
    }

    // 在 Windows 上，我们必须解析它。
    // 在其他系统上，我们也解析它，以防万一。
    #[cfg(not(unix))]
    {
        env::current_dir().and_then(|path| path.canonicalize())
    }
}

fn pwd_logical_path() -> io::Result<PathBuf> {
    // 如果我们不在 Windows 上，我们按 Unix 方式处理。
    //
    // 典型的类 Unix 内核实际上并不跟踪逻辑工作目录。它们知道进程所在的精确目录，getcwd()
    // 系统调用从中重建路径。
    //
    // 逻辑工作目录由 shell 维护，在 $PWD 环境变量中。所以我们仔细检查该变量是否看起来合理，
    // 如果不合理，我们会回退到物理路径。
    //
    // POSIX: https://pubs.opengroup.org/onlinepubs/9699919799/utilities/pwd.html
    #[cfg(not(windows))]
    {
        use std::fs::metadata;
        use std::os::unix::fs::MetadataExt;
        use std::path::Path;

        fn looks_reasonable(path: &Path) -> bool {
            // 首先，检查它是否是绝对路径。
            if !path.has_root() {
                return false;
            }

            // 然后，确保没有 . 或 .. 组件。
            // Path::components() 在这里没用，它会将这些组件标准化。
            // to_string_lossy() 可能会分配，但这没关系，我们每次运行只调用一次。
            // 它也可能丢失信息，但不会丢失我们检查所需的任何信息。
            if path
                .to_string_lossy()
                .split(std::path::is_separator)
                .any(|piece| piece == "." || piece == "..")
            {
                return false;
            }

            // 最后，检查它是否与我们所在的目录匹配。
            match (metadata(path), metadata(".")) {
                (Ok(path_md), Ok(current_dir_md)) => {
                    path_md.dev() == current_dir_md.dev() && path_md.ino() == current_dir_md.ino()
                }
                _ => false,
            }
        }

        if let Some(value) = env::var_os("PWD").map(PathBuf::from) {
            if looks_reasonable(&value) {
                Ok(value)
            } else {
                env::current_dir()
            }
        } else {
            env::current_dir()
        }
    }

    // Windows 上的 getcwd() 似乎包含符号链接，所以这很简单。
    #[cfg(windows)]
    {
        env::current_dir()
    }
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    pwd_main(args)
}

pub fn pwd_main(args: impl ctcore::Args) -> CTResult<()> {
    let matches = ct_app().try_get_matches_from(args)?;
    // 如果设置了 POSIXLY_CORRECT，我们希望进行逻辑解析。
    // 这在执行 mkdir -p a/b && ln -s a/b c && cd c && pwd 时会产生不同的输出
    // 在这种情况下，我们应该在路径末尾得到 c 而不是 a/b
    let cwd = if matches.get_flag(pwd_flags::PWD_PHYSICAL) {
        pwd_physical_path()
    } else if matches.get_flag(pwd_flags::PWD_LOGICAL) || env::var("POSIXLY_CORRECT").is_ok() {
        pwd_logical_path()
    } else {
        pwd_physical_path()
    }
    .map_err_context(|| "failed to get current directory".to_owned())?;

    // \\?\ 是 Windows 在某些情况下给路径加的前缀，包括对它们进行规范化时。
    // 有了正确的扩展特性，我们可以无损地删除它，但我们无损地打印它，所以没有理由麻烦。
    #[cfg(windows)]
    let cwd = cwd
        .to_string_lossy()
        .strip_prefix(r"\\?\")
        .map(Into::into)
        .unwrap_or(cwd);

    ct_println_verbatim(cwd).map_err_context(|| "failed to print current directory".to_owned())?;

    Ok(())
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = PWD_ABOUT;
    let usage_description = ct_format_usage(PWD_USAGE);
    let args = vec![
        Arg::new(pwd_flags::PWD_LOGICAL)
            .short('L')
            .long(pwd_flags::PWD_LOGICAL)
            .help("use PWD from environment, even if it contains symlinks")
            .action(ArgAction::SetTrue),
        Arg::new(pwd_flags::PWD_PHYSICAL)
            .short('P')
            .long(pwd_flags::PWD_PHYSICAL)
            .overrides_with(pwd_flags::PWD_LOGICAL)
            .help("avoid all symlinks")
            .action(ArgAction::SetTrue),
        Arg::new(pwd_flags::PWD_ARG_OTHERS)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::AnyPath),
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
    mod ct_main_tests {
        use super::*;
        use std::ffi::OsString;
        #[test]
        fn test_pwd_main_execution_version() {
            let args_vec = vec![ctcore::ct_util_name(), "--version"];
            let args = args_vec.iter().map(|s| OsString::from(s));
            let result = pwd_main(args);

            assert!(result.is_err());
        }

        #[test]
        fn test_pwd_main_execution_other_version() {
            let args_vec = vec![ctcore::ct_util_name(), "-V"];

            let args = args_vec.iter().map(|s| OsString::from(s));
            let result = pwd_main(args);

            assert!(result.is_err());
        }

        #[test]
        fn test_pwd_main_execution_help() {
            let args_vec = vec![ctcore::ct_util_name(), "--help"];
            let args = args_vec.iter().map(|s| OsString::from(s));
            let result = pwd_main(args);
            assert!(result.is_err());
        }

        #[test]
        fn test_pwd_main_execution_help_short() {
            let args_vec = vec![ctcore::ct_util_name(), "-h"];
            let args = args_vec.iter().map(|s| OsString::from(s));
            let result = pwd_main(args);
            assert!(result.is_err());
        }

        #[test]
        fn test_pwd_main_execution_unsupport_help() {
            let args_vec = vec![ctcore::ct_util_name(), "-H"];
            let args = args_vec.iter().map(|s| OsString::from(s));
            let result = pwd_main(args);
            assert!(result.is_err());
        }

        #[test]
        fn test_pwd_main_invalid_argument() {
            let args_vec = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let args = args_vec.iter().map(|s| OsString::from(s));
            let result = pwd_main(args);
            assert!(result.is_err());
        }
    }

    #[cfg(test)]
    mod env_data_tests {
        use super::*;
        use std::env;
        use std::ffi::OsString;
        use std::fs;
        use tempfile::TempDir;

        #[test]
        fn test_env_data() {
            // test_pwd_main_support_missing_argument()
            {
                let args_vec = vec![ctcore::ct_util_name()];
                let args = args_vec.iter().map(|s| OsString::from(s));
                let result = pwd_main(args);
                assert!(result.is_ok());
            }

            // test_pwd_main_logical_long()
            {
                let args_vec = vec![ctcore::ct_util_name(), "--logical"];
                let args = args_vec.iter().map(|s| OsString::from(s));
                let result = pwd_main(args);
                assert!(result.is_ok());
            }

            // test_pwd_main_logical_short()
            {
                let args_vec = vec![ctcore::ct_util_name(), "-L"];
                let args = args_vec.iter().map(|s| OsString::from(s));
                let result = pwd_main(args);
                assert!(result.is_ok());
            }

            //  test_pwd_main_physical_long()
            {
                let args_vec = vec![ctcore::ct_util_name(), "--physical"];
                let args = args_vec.iter().map(|s| OsString::from(s));
                let result = pwd_main(args);
                assert!(result.is_ok());
            }

            // test_pwd_main_physical_short()
            {
                let args_vec = vec![ctcore::ct_util_name(), "-P"];
                let args = args_vec.iter().map(|s| OsString::from(s));
                let result = pwd_main(args);
                assert!(result.is_ok());
            }

            // test_pwd_main_logical_long_with_file()
            {
                let file_name = "test_pwd_main_logical_long";

                let args_vec = vec![ctcore::ct_util_name(), "--logical", file_name];
                let args = args_vec.iter().map(|s| OsString::from(s));
                let result = pwd_main(args);
                assert!(result.is_ok());
            }

            // test_pwd_main_logical_short_with_file()
            {
                let file_name = "test_pwd_main_logical_short";

                let args_vec = vec![ctcore::ct_util_name(), "-L", file_name];
                let args = args_vec.iter().map(|s| OsString::from(s));
                let result = pwd_main(args);
                assert!(result.is_ok());
            }

            // test_pwd_main_physical_long_with_file()
            {
                let file_name = "test_pwd_main_physical_long";

                let args_vec = vec![ctcore::ct_util_name(), "--physical", file_name];
                let args = args_vec.iter().map(|s| OsString::from(s));
                let result = pwd_main(args);
                assert!(result.is_ok());
            }

            // test_pwd_main_physical_short_with_file()
            {
                let file_name = "test_pwd_main_physical_short";

                let args_vec = vec![ctcore::ct_util_name(), "-P", file_name];
                let args = args_vec.iter().map(|s| OsString::from(s));
                let result = pwd_main(args);
                assert!(result.is_ok());
            }

            // test_physical_path_basic()
            {
                let temp_dir = TempDir::new().unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                env::set_current_dir(&temp_dir_path).expect("failed to change directory");

                let result = pwd_physical_path().expect("failed to get physical path");
                assert_eq!(result, temp_dir_path);

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_physical_path_with_nested_symlink()
            {
                let temp_dir = TempDir::new().unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                let nested_dir = temp_dir_path.join("nested");
                std::fs::create_dir(&nested_dir).expect("failed to create nested dir");

                let target_path = nested_dir.join("target");
                let symlink_path = nested_dir.join("symlink");
                std::fs::create_dir(&target_path).expect("failed to create target dir");

                #[cfg(unix)]
                std::os::unix::fs::symlink(&target_path, &symlink_path)
                    .expect("failed to create symlink");
                #[cfg(windows)]
                std::os::windows::fs::symlink_dir(&target_path, &symlink_path)
                    .expect("failed to create symlink");

                env::set_current_dir(&symlink_path).expect("failed to change directory");

                let result = pwd_physical_path().expect("failed to get physical path");
                let expected_path = target_path
                    .canonicalize()
                    .expect("failed to canonicalize target path");
                assert_eq!(result, expected_path);

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_physical_path_with_multiple_symlinks()
            {
                let temp_dir = TempDir::new().unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                let nested_dir = temp_dir_path.join("nested");
                std::fs::create_dir(&nested_dir).expect("failed to create nested dir");

                let target_path = nested_dir.join("target");
                let symlink_path1 = nested_dir.join("symlink1");
                let symlink_path2 = temp_dir_path.join("symlink2");

                std::fs::create_dir(&target_path).expect("failed to create target dir");

                #[cfg(unix)]
                {
                    std::os::unix::fs::symlink(&target_path, &symlink_path1)
                        .expect("failed to create symlink1");
                    std::os::unix::fs::symlink(&symlink_path1, &symlink_path2)
                        .expect("failed to create symlink2");
                }

                #[cfg(windows)]
                {
                    std::os::windows::fs::symlink_dir(&target_path, &symlink_path1)
                        .expect("failed to create symlink1");
                    std::os::windows::fs::symlink_dir(&symlink_path1, &symlink_path2)
                        .expect("failed to create symlink2");
                }

                env::set_current_dir(&symlink_path2).expect("failed to change directory");

                let result = pwd_physical_path().expect("failed to get physical path");
                let expected_path = target_path
                    .canonicalize()
                    .expect("failed to canonicalize target path");
                assert_eq!(result, expected_path);

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_physical_path_with_nonexistent_directory()
            {
                let temp_dir = TempDir::new().unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                env::set_current_dir(&temp_dir_path).expect("failed to change directory");

                let nonexistent_path = temp_dir_path.join("nonexistent");

                env::set_current_dir(&nonexistent_path)
                    .expect_err("should fail to change directory");
                let result = pwd_physical_path();
                assert!(result.is_ok());

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_physical_path_with_long_path()
            {
                let temp_dir = TempDir::new().unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                let long_path = temp_dir_path.join("a".repeat(255));
                std::fs::create_dir_all(&long_path).expect("failed to create long path dir");
                env::set_current_dir(&long_path).expect("failed to change directory");

                let result = pwd_physical_path().expect("failed to get physical path");
                let expected_path = long_path
                    .canonicalize()
                    .expect("failed to canonicalize long path");
                assert_eq!(result, expected_path);

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_physical_path_basic()
            {
                let temp_dir = TempDir::new().unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                env::set_current_dir(&temp_dir_path).expect("failed to change directory");

                let result = pwd_physical_path().expect("failed to get physical path");
                assert_eq!(result, temp_dir_path);

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_physical_path_with_nested_symlink()
            {
                let temp_dir = TempDir::new().unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                let nested_dir = temp_dir_path.join("nested");
                std::fs::create_dir(&nested_dir).expect("failed to create nested dir");

                let target_path = nested_dir.join("target");
                let symlink_path = nested_dir.join("symlink");
                std::fs::create_dir(&target_path).expect("failed to create target dir");

                #[cfg(unix)]
                std::os::unix::fs::symlink(&target_path, &symlink_path)
                    .expect("failed to create symlink");
                #[cfg(windows)]
                std::os::windows::fs::symlink_dir(&target_path, &symlink_path)
                    .expect("failed to create symlink");

                env::set_current_dir(&symlink_path).expect("failed to change directory");

                let result = pwd_physical_path().expect("failed to get physical path");
                let expected_path = target_path
                    .canonicalize()
                    .expect("failed to canonicalize target path");
                assert_eq!(result, expected_path);

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_physical_path_with_multiple_symlinks()
            {
                let temp_dir = TempDir::new().unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                let nested_dir = temp_dir_path.join("nested");
                std::fs::create_dir(&nested_dir).expect("failed to create nested dir");

                let target_path = nested_dir.join("target");
                let symlink_path1 = nested_dir.join("symlink1");
                let symlink_path2 = temp_dir_path.join("symlink2");

                std::fs::create_dir(&target_path).expect("failed to create target dir");

                #[cfg(unix)]
                {
                    std::os::unix::fs::symlink(&target_path, &symlink_path1)
                        .expect("failed to create symlink1");
                    std::os::unix::fs::symlink(&symlink_path1, &symlink_path2)
                        .expect("failed to create symlink2");
                }

                #[cfg(windows)]
                {
                    std::os::windows::fs::symlink_dir(&target_path, &symlink_path1)
                        .expect("failed to create symlink1");
                    std::os::windows::fs::symlink_dir(&symlink_path1, &symlink_path2)
                        .expect("failed to create symlink2");
                }

                env::set_current_dir(&symlink_path2).expect("failed to change directory");

                let result = pwd_physical_path().expect("failed to get physical path");
                let expected_path = target_path
                    .canonicalize()
                    .expect("failed to canonicalize target path");
                assert_eq!(result, expected_path);

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_physical_path_with_nonexistent_directory()
            {
                let temp_dir = TempDir::new().unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                env::set_current_dir(&temp_dir_path).expect("failed to change directory");

                let nonexistent_path = temp_dir_path.join("nonexistent");

                env::set_current_dir(&nonexistent_path)
                    .expect_err("should fail to change directory");
                let result = pwd_physical_path();
                assert!(result.is_ok());

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_physical_path_with_long_path()
            {
                let temp_dir = TempDir::new().unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                let long_path = temp_dir_path.join("a".repeat(255));
                std::fs::create_dir_all(&long_path).expect("failed to create long path dir");
                env::set_current_dir(&long_path).expect("failed to change directory");

                let result = pwd_physical_path().expect("failed to get physical path");
                let expected_path = long_path
                    .canonicalize()
                    .expect("failed to canonicalize long path");
                assert_eq!(result, expected_path);

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_logical_path_unix()
            {
                let temp_dir = TempDir::with_prefix("test_logical_path_").unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                let logical_path_buf = temp_dir_path.join("logical");
                fs::create_dir(&logical_path_buf).expect("failed to create logical dir");

                let symlink_path = temp_dir_path.join("symlink");
                #[cfg(unix)]
                std::os::unix::fs::symlink(&logical_path_buf, &symlink_path)
                    .expect("failed to create symlink");
                #[cfg(windows)]
                std::os::windows::fs::symlink_dir(&logical_path_buf, &symlink_path)
                    .expect("failed to create symlink");

                // 切换到符号链接目录
                env::set_current_dir(&symlink_path).expect("failed to change directory");
                unsafe { env::set_var("PWD", &symlink_path) };

                let result = pwd_logical_path().expect("failed to get logical path");
                assert_eq!(result, symlink_path);
                unsafe { env::remove_var("PWD") };

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_logical_path_invalid_pwd()
            {
                let temp_dir = TempDir::with_prefix("test_logical_path_").unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                env::set_current_dir(&temp_dir_path).expect("failed to change directory");
                unsafe { env::set_var("PWD", "/invalid/path") };

                let result = pwd_logical_path().expect("failed to get logical path");
                assert_eq!(result, temp_dir_path);
                unsafe { env::remove_var("PWD") };

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_logical_path_unix_no_pwd()
            {
                let temp_dir = TempDir::with_prefix("test_logical_path_").unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                env::set_current_dir(&temp_dir_path).expect("failed to change directory");
                unsafe { env::remove_var("PWD") };

                let result = pwd_logical_path().expect("failed to get logical path");
                assert_eq!(result, temp_dir_path);
                unsafe { env::remove_var("PWD") };

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_logical_path_unix_relative_pwd()
            {
                let temp_dir = TempDir::with_prefix("test_logical_path_").unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                env::set_current_dir(&temp_dir_path).expect("failed to change directory");
                unsafe { env::set_var("PWD", "relative/path") };

                let result = pwd_logical_path().expect("failed to get logical path");
                assert_eq!(result, temp_dir_path);
                unsafe { env::remove_var("PWD") };

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_logical_path_windows_valid_pwd()
            {
                let temp_dir = TempDir::with_prefix("test_logical_path_").unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                env::set_current_dir(&temp_dir_path).expect("failed to change directory");
                unsafe { env::set_var("PWD", &temp_dir_path) };

                let result = pwd_logical_path().expect("failed to get logical path");
                assert_eq!(result, temp_dir_path);
                unsafe { env::remove_var("PWD") };

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_logical_path_windows_invalid_pwd()
            {
                let temp_dir = TempDir::with_prefix("test_logical_path_").unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                env::set_current_dir(&temp_dir_path).expect("failed to change directory");
                unsafe { env::set_var("PWD", "C:\\invalid\\path") };

                let result = pwd_logical_path().expect("failed to get logical path");
                assert_eq!(result, temp_dir_path);
                unsafe { env::remove_var("PWD") };

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_logical_path_windows_no_pwd()
            {
                let temp_dir = TempDir::with_prefix("test_logical_path_").unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                env::set_current_dir(&temp_dir_path).expect("failed to change directory");
                unsafe { env::remove_var("PWD") };

                let result = pwd_logical_path().expect("failed to get logical path");
                assert_eq!(result, temp_dir_path);

                temp_dir.close().expect("failed to close temp dir");
            }
        }
    }

    #[cfg(test)]
    mod ct_app_tests {
        use super::*;
        use clap::error::ErrorKind;

        // pwd 接口: pwd [OPTION]...
        //
        // Options:
        //   -L, --logical   use PWD from environment, even if it contains symlinks
        //   -P, --physical  avoid all symlinks
        //   -h, --help      Print help
        //   -V, --version   Print version

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
        fn test_ct_app_logical_long() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--logical"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_logical_short() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-L"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_physical_long() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--physical"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_physical_short() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-P"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_logical_long_with_file() {
            let file_name = "test_ct_app_logical_long";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--logical", file_name];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_logical_short_with_file() {
            let file_name = "test_ct_app_logical_short";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-L", file_name];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_physical_long_with_file() {
            let file_name = "test_ct_app_physical_long";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--physical", file_name];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_physical_short_with_file() {
            let file_name = "test_ct_app_physical_short";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-P", file_name];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }
    }
}