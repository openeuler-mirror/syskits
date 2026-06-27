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

//! rmdir命令在Linux和其他类Unix系统中用于删除空目录, 如果目录非空，rmdir命令将会失败.

extern crate rust_i18n;
use clap::builder::ValueParser;
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "zh-CN");
use clap::{Arg, ArgAction, Command, crate_version};
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, set_ct_exit_code, strip_errno};
use ctcore::{ct_show_error, ct_util_name};
use std::ffi::OsString;
use std::fs::{read_dir, remove_dir};
use std::io;
use std::path::Path;
use sys_locale::get_locale;

pub mod rmdir_flags {
    pub const RMDIR_IGNORE_FAIL_NON_EMPTY: &str = "ignore-fail-on-non-empty";
    pub const RMDIR_PARENTS: &str = "parents";
    pub const RMDIR_VERBOSE: &str = "verbose";

    pub const RMDIR_ARG_DIRS: &str = "dirs";
}

pub fn rmdir_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let matches = ct_app().try_get_matches_from(args)?;

    let configs = RmdirConfigs {
        is_ignore: matches.get_flag(rmdir_flags::RMDIR_IGNORE_FAIL_NON_EMPTY),
        is_parents: matches.get_flag(rmdir_flags::RMDIR_PARENTS),
        is_verbose: matches.get_flag(rmdir_flags::RMDIR_VERBOSE),
    };

    for path in matches
        .get_many::<OsString>(rmdir_flags::RMDIR_ARG_DIRS)
        .unwrap_or_default()
        .map(Path::new)
    {
        if let Err(error) = rmdir_remove(path, configs) {
            let RmdirError { error, path } = error;

            if configs.is_ignore && rmdir_dir_not_empty(&error, path) {
                continue;
            }

            set_ct_exit_code(1);

            // 如果 `foo` 是一个指向目录的符号链接，那么 `rmdir foo/` 可能会给出 "不是目录" 错误。
            // 这很令人困惑，因为 `rm foo/` 会说 "是一个目录"。
            // 这在不同的系统中有所不同。有些不会报错。
            // Windows 允许对符号链接调用 RemoveDirectory，所以我们不需要在这里担心。
            // GNU rmdir 似乎会打印 "符号链接未跟随" 如果：
            // - 它有一个尾部斜杠
            // - 它是一个符号链接
            // - 它指向一个目录或是悬挂的
            #[cfg(unix)]
            {
                use std::ffi::OsStr;
                use std::os::unix::ffi::OsStrExt;

                fn points_to_directory(path: &Path) -> io::Result<bool> {
                    Ok(path.metadata()?.file_type().is_dir())
                }

                let bytes = path.as_os_str().as_bytes();
                if error.raw_os_error() == Some(libc::ENOTDIR) && bytes.ends_with(b"/") {
                    // 去除尾部斜杠，否则 .symlink_metadata() 会跟随符号链接
                    let no_slash: &Path = OsStr::from_bytes(&bytes[..bytes.len() - 1]).as_ref();
                    if no_slash.is_symlink() && points_to_directory(no_slash).unwrap_or(true) {
                        ct_show_error!(
                            "failed to remove {}: Symbolic link not followed",
                            path.quote()
                        );
                        continue;
                    }
                }
            }

            ct_show_error!("failed to remove {}: {}", path.quote(), strip_errno(&error));
        }
    }

    Ok(())
}

struct RmdirError<'a> {
    error: io::Error,
    path: &'a Path,
}

fn rmdir_remove(mut path: &Path, configs: RmdirConfigs) -> Result<(), RmdirError<'_>> {
    rmdir_remove_single(path, configs)?;
    if configs.is_parents {
        while let Some(new) = path.parent() {
            path = new;
            if path.as_os_str().is_empty() {
                break;
            }
            rmdir_remove_single(path, configs)?;
        }
    }
    Ok(())
}

fn rmdir_remove_single(path: &Path, configs: RmdirConfigs) -> Result<(), RmdirError<'_>> {
    if configs.is_verbose {
        println!("{}: removing directory, {}", ct_util_name(), path.quote());
    }
    remove_dir(path).map_err(|error| RmdirError { error, path })
}

// POSIX: https://pubs.opengroup.org/onlinepubs/009696799/functions/rmdir.html
#[cfg(not(windows))]
const RMDIR_NOT_EMPTY_CODES: &[i32] = &[libc::ENOTEMPTY, libc::EEXIST];

// 145 是 ERROR_DIR_NOT_EMPTY，通过实验确定。
#[cfg(windows)]
const RMDIR_NOT_EMPTY_CODES: &[i32] = &[145];

// 其他你可能会遇到的目录错误码，这些错误码表明目录存在但不为空。
// 这是来自 Linux man-pages 项目的 rmdir(2) 列出的错误码的一个子集。也许其他系统有额外的适用错误码？
#[cfg(not(windows))]
const RMDIR_PERHAPS_EMPTY_CODES: &[i32] = &[libc::EACCES, libc::EBUSY, libc::EPERM, libc::EROFS];

// 可能不完整，我找不到任何地方列出了 RemoveDirectory 可能的错误码。
#[cfg(windows)]
const RMDIR_PERHAPS_EMPTY_CODES: &[i32] = &[
    5, // ERROR_ACCESS_DENIED，通过实验确定。
];

fn rmdir_dir_not_empty(error: &io::Error, path: &Path) -> bool {
    if let Some(code) = error.raw_os_error() {
        if RMDIR_NOT_EMPTY_CODES.contains(&code) {
            return true;
        }
        // 如果使用了 --ignore-fail-on-non-empty，我们希望忽略所有由于目录不为空而导致的错误，
        // 即使错误是由于没有权限等原因造成的。因此我们进行额外的检查。
        if RMDIR_PERHAPS_EMPTY_CODES.contains(&code) {
            if let Ok(mut iterator) = read_dir(path) {
                return iterator.next().is_some();
            }
        }
    }
    false
}

#[derive(Clone, Copy, Debug)]
struct RmdirConfigs {
    is_ignore: bool,
    is_parents: bool,
    is_verbose: bool,
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("rmdir.about");
    let usage_description = t!("rmdir.usage");
    let args = vec![
        Arg::new(rmdir_flags::RMDIR_IGNORE_FAIL_NON_EMPTY)
            .long(rmdir_flags::RMDIR_IGNORE_FAIL_NON_EMPTY)
            .help(t!("rmdir.clap.rmdir_ignore_fail_non_empty"))
            .action(ArgAction::SetTrue),
        Arg::new(rmdir_flags::RMDIR_PARENTS)
            .short('p')
            .long(rmdir_flags::RMDIR_PARENTS)
            .help(
                "remove DIRECTORY and its ancestors; e.g.,
                  'rmdir -p a/b/c' is similar to rmdir a/b/c a/b a",
            )
            .action(ArgAction::SetTrue),
        Arg::new(rmdir_flags::RMDIR_VERBOSE)
            .short('v')
            .long(rmdir_flags::RMDIR_VERBOSE)
            .help(t!("rmdir.clap.rmdir_verbose"))
            .action(ArgAction::SetTrue),
        Arg::new(rmdir_flags::RMDIR_ARG_DIRS)
            .action(ArgAction::Append)
            .num_args(1..)
            .value_parser(ValueParser::os_string())
            .value_hint(clap::ValueHint::DirPath),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(args)
}

#[derive(Default)]
pub struct Rmdir;
impl Tool for Rmdir {
    fn name(&self) -> &'static str {
        "rmdir"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        rmdir_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_implementation() {
        let tool = Rmdir;

        // Test name method
        assert_eq!(tool.name(), "rmdir");

        // Test command method
        let command = tool.command();
        assert!(command.get_name().contains("rmdir"));

        // Test execute method with help flag (should work)
        let args: Vec<OsString> = vec![OsString::from("rmdir"), OsString::from("--help")];
        let result = tool.execute(&args);
        assert!(result.is_err());
    }

    #[cfg(test)]
    mod dir_not_empty_tests {
        use super::*;
        use std::fs::create_dir;
        use std::io::{self, Error, ErrorKind};
        use std::path::Path;
        use tempfile::TempDir;

        fn simulate_error(path: &Path) -> io::Error {
            if cfg!(unix) {
                let err = std::fs::remove_file(path);
                match err {
                    Ok(_) => Error::new(ErrorKind::Other, "File was removed"),
                    Err(e) => e,
                }
            } else {
                Error::new(ErrorKind::Other, "Simulated error")
            }
        }

        #[test]
        fn test_dir_not_empty_for_empty_directory() {
            let tmp_dir = TempDir::new().unwrap();
            let dir_path = tmp_dir.path().join("empty_dir");
            create_dir(&dir_path).unwrap();

            let err = simulate_error(&dir_path);
            assert!(!rmdir_dir_not_empty(&err, &dir_path));
        }

        #[test]
        fn test_dir_not_empty_for_non_existent_directory() {
            let tmp_dir = TempDir::new().unwrap();
            let dir_path = tmp_dir.path().join("non_existent_dir");

            let err = simulate_error(&dir_path);
            assert!(!rmdir_dir_not_empty(&err, &dir_path));
        }
    }

    #[cfg(test)]
    mod remove_empty_tests {
        use super::*;
        use std::fs::{self, File, create_dir};
        use std::os::unix::fs::{PermissionsExt, symlink};
        use tempfile::TempDir;

        #[test]
        fn test_remove_single_empty_directory() {
            let tmp_dir = TempDir::new().unwrap();
            let dir_path = tmp_dir.path().join("single_empty_dir");
            create_dir(&dir_path).unwrap();

            let opts = RmdirConfigs {
                is_ignore: false,
                is_parents: false,
                is_verbose: false,
            };

            assert!(rmdir_remove_single(&dir_path, opts).is_ok());
            assert!(!dir_path.exists());
        }

        #[test]
        fn test_remove_single_non_empty_directory() {
            let tmp_dir = TempDir::new().unwrap();
            let dir_path = tmp_dir.path().join("single_non_empty_dir");
            create_dir(&dir_path).unwrap();
            File::create(dir_path.join("file.txt")).unwrap();

            let opts = RmdirConfigs {
                is_ignore: false,
                is_parents: false,
                is_verbose: false,
            };

            assert!(rmdir_remove_single(&dir_path, opts).is_err());
            assert!(dir_path.exists());
        }

        #[test]
        fn test_remove_single_symbolic_link() {
            let tmp_dir = TempDir::new().unwrap();
            let dir_path = tmp_dir.path().join("single_target_dir");
            create_dir(&dir_path).unwrap();
            let symlink_path = tmp_dir.path().join("single_symlink_dir");
            symlink(&dir_path, &symlink_path).unwrap();

            let opts = RmdirConfigs {
                is_ignore: false,
                is_parents: false,
                is_verbose: false,
            };

            assert!(rmdir_remove_single(&symlink_path, opts).is_err());
            assert!(dir_path.exists()); // Original directory should still exist
            assert!(symlink_path.exists()); // Symlink should be removed
        }

        #[test]
        fn test_remove_single_directory_with_permission_denied() {
            let tmp_dir = TempDir::new().unwrap();
            let dir_path = tmp_dir.path().join("single_restricted_dir");
            create_dir(&dir_path).unwrap();
            File::create(dir_path.join("file.txt")).unwrap();
            let _ = fs::set_permissions(&dir_path, fs::Permissions::from_mode(0o000)); // Remove all permissions

            let opts = RmdirConfigs {
                is_ignore: false,
                is_parents: false,
                is_verbose: false,
            };

            let result = rmdir_remove_single(&dir_path, opts);
            assert!(result.is_err());

            // Restore permissions to clean up directory
            let _ = fs::set_permissions(&dir_path, fs::Permissions::from_mode(0o755));
            assert!(dir_path.exists());
        }

        #[test]
        fn test_remove_single_non_existent_directory() {
            let tmp_dir = TempDir::new().unwrap();
            let dir_path = tmp_dir.path().join("non_existent_dir");

            let opts = RmdirConfigs {
                is_ignore: false,
                is_parents: false,
                is_verbose: false,
            };

            assert!(rmdir_remove_single(&dir_path, opts).is_err());
            assert!(!dir_path.exists());
        }

        #[test]
        fn test_remove_single_symbolic_link_to_directory() {
            let tmp_dir = TempDir::new().unwrap();
            let dir_path = tmp_dir.path().join("target_dir");
            create_dir(&dir_path).unwrap();
            let symlink_path = tmp_dir.path().join("symlink_dir");
            symlink(&dir_path, &symlink_path).unwrap();

            let opts = RmdirConfigs {
                is_ignore: false,
                is_parents: false,
                is_verbose: false,
            };

            assert!(rmdir_remove_single(&symlink_path, opts).is_err());
            assert!(dir_path.exists()); // Original directory should still exist
            assert!(symlink_path.exists()); // Symlink should be removed
        }

        #[test]
        fn test_remove_single_directory_with_special_characters() {
            let tmp_dir = TempDir::new().unwrap();
            let dir_path = tmp_dir.path().join("dir_with_special_@#$%^&*()_chars");
            create_dir(&dir_path).unwrap();

            let opts = RmdirConfigs {
                is_ignore: false,
                is_parents: false,
                is_verbose: false,
            };

            assert!(rmdir_remove_single(&dir_path, opts).is_ok());
            assert!(!dir_path.exists());
        }

        #[test]
        fn test_remove_single_with_empty_path() {
            let opts = RmdirConfigs {
                is_ignore: false,
                is_parents: false,
                is_verbose: false,
            };

            let empty_path = Path::new("");
            assert!(rmdir_remove_single(empty_path, opts).is_err());
        }

        #[test]
        fn test_remove_single_directory_with_trailing_slash() {
            let tmp_dir = TempDir::new().unwrap();
            let dir_path = tmp_dir.path().join("dir_with_trailing_slash");
            create_dir(&dir_path).unwrap();

            let opts = RmdirConfigs {
                is_ignore: false,
                is_parents: false,
                is_verbose: false,
            };

            // Adding trailing slash
            let dir_path_with_slash = format!("{}/", dir_path.display());
            let dir_path_with_slash = Path::new(&dir_path_with_slash);

            assert!(rmdir_remove_single(&dir_path_with_slash, opts).is_ok());
            assert!(!dir_path.exists());
        }

        #[test]
        fn test_remove_single_symbolic_link_to_non_existent_directory() {
            let tmp_dir = TempDir::new().unwrap();
            let target_path = tmp_dir.path().join("non_existent_target");
            let symlink_path = tmp_dir.path().join("symlink_to_non_existent");
            symlink(&target_path, &symlink_path).unwrap();

            let opts = RmdirConfigs {
                is_ignore: false,
                is_parents: false,
                is_verbose: false,
            };

            assert!(rmdir_remove_single(&symlink_path, opts).is_err());
            assert!(!symlink_path.exists()); // Symlink should be removed
        }
    }
    #[cfg(test)]
    mod remove_tests {
        use super::*;
        use std::fs;
        use std::fs::{File, create_dir, create_dir_all};
        use std::os::unix::fs::{PermissionsExt, symlink};
        use std::path::PathBuf;
        use tempfile::TempDir;

        #[test]
        fn test_remove_empty_directory() {
            let tmp_dir = TempDir::new().unwrap();
            let dir_path = tmp_dir.path().join("empty_dir");
            create_dir(&dir_path).unwrap();

            let opts = RmdirConfigs {
                is_ignore: false,
                is_parents: false,
                is_verbose: false,
            };

            assert!(rmdir_remove(&dir_path, opts).is_ok());
            assert!(!dir_path.exists());
        }

        #[test]
        fn test_remove_non_empty_directory_with_ignore() {
            let tmp_dir = TempDir::new().unwrap();
            let dir_path = tmp_dir.path().join("non_empty_dir");
            create_dir(&dir_path).unwrap();
            File::create(dir_path.join("file.txt")).unwrap();

            let opts = RmdirConfigs {
                is_ignore: true,
                is_parents: false,
                is_verbose: false,
            };

            assert!(rmdir_remove(&dir_path, opts).is_err()); // remove 外层屏蔽的报错
            assert!(dir_path.exists());
        }

        #[test]
        fn test_remove_non_empty_directory_without_ignore() {
            let tmp_dir = TempDir::new().unwrap();
            let dir_path = tmp_dir.path().join("non_empty_dir");
            create_dir(&dir_path).unwrap();
            File::create(dir_path.join("file.txt")).unwrap();

            let opts = RmdirConfigs {
                is_ignore: false,
                is_parents: false,
                is_verbose: false,
            };

            assert!(rmdir_remove(&dir_path, opts).is_err());
            assert!(dir_path.exists());
        }

        #[test]
        fn test_remove_directory_with_parents() {
            let dir_path = PathBuf::from("a/b/c");
            create_dir_all(&dir_path).unwrap();

            let opts = RmdirConfigs {
                is_ignore: false,
                is_parents: true,
                is_verbose: false,
            };
            let result = rmdir_remove(&dir_path, opts);
            assert!(result.is_ok());
            assert!(!dir_path.exists());
            assert!(!dir_path.parent().unwrap().exists()); // a/b should also be removed
            assert!(!dir_path.parent().unwrap().parent().unwrap().exists()); // a should also be removed
        }

        #[test]
        fn test_remove_directory_with_parents_err() {
            let tmp_dir = TempDir::new().unwrap();
            let dir_path = tmp_dir.path().join("a/b/c");
            create_dir_all(&dir_path).unwrap();

            let opts = RmdirConfigs {
                is_ignore: false,
                is_parents: true,
                is_verbose: false,
            };

            assert!(rmdir_remove(&dir_path, opts).is_err()); // 顶层目录存在文件报错，不删除
            assert!(!dir_path.exists());
            assert!(!dir_path.parent().unwrap().exists()); // a/b should also be removed
            assert!(!dir_path.parent().unwrap().parent().unwrap().exists()); // a should also be removed
        }

        #[test]
        fn test_remove_nested_non_empty_directory() {
            let tmp_dir = TempDir::new().unwrap();
            let dir_path = tmp_dir.path().join("nested_dir");
            create_dir_all(dir_path.join("subdir")).unwrap();
            File::create(dir_path.join("subdir/file.txt")).unwrap();

            let opts = RmdirConfigs {
                is_ignore: false,
                is_parents: false,
                is_verbose: false,
            };

            assert!(rmdir_remove(&dir_path, opts).is_err());
            assert!(dir_path.exists());
        }

        #[test]
        fn test_remove_symbolic_link() {
            let tmp_dir = TempDir::new().unwrap();
            let dir_path = tmp_dir.path().join("target_dir");
            create_dir(&dir_path).unwrap();
            let symlink_path = tmp_dir.path().join("symlink_dir");
            symlink(&dir_path, &symlink_path).unwrap();

            let opts = RmdirConfigs {
                is_ignore: false,
                is_parents: false,
                is_verbose: false,
            };

            assert!(rmdir_remove(&symlink_path, opts).is_err());
            assert!(dir_path.exists()); // Original directory should still exist
            assert!(symlink_path.exists()); // Symlink should be removed
        }

        #[test]
        fn test_remove_directory_with_permission_denied() {
            let tmp_dir = TempDir::new().unwrap();
            let dir_path = tmp_dir.path().join("restricted_dir");
            create_dir(&dir_path).unwrap();
            File::create(dir_path.join("file.txt")).unwrap();
            let _ = fs::set_permissions(&dir_path, fs::Permissions::from_mode(0o000)); // Remove all permissions

            let opts = RmdirConfigs {
                is_ignore: false,
                is_parents: false,
                is_verbose: false,
            };

            let result = rmdir_remove(&dir_path, opts);
            assert!(result.is_err());

            // Restore permissions to clean up directory
            let _ = fs::set_permissions(&dir_path, fs::Permissions::from_mode(0o755));
            assert!(dir_path.exists());
        }

        #[test]
        fn test_remove_empty_path() {
            let opts = RmdirConfigs {
                is_ignore: false,
                is_parents: false,
                is_verbose: false,
            };

            let empty_path = Path::new("");
            assert!(rmdir_remove(empty_path, opts).is_err());
        }

        #[test]
        fn test_remove_parent_directory() {
            let tmp_dir = TempDir::new().unwrap();
            let parent_dir_path = tmp_dir.path().join("parent_dir");
            let child_dir_path = parent_dir_path.join("child_dir");
            create_dir_all(&child_dir_path).unwrap();

            let opts = RmdirConfigs {
                is_ignore: false,
                is_parents: true,
                is_verbose: false,
            };

            assert!(rmdir_remove(&child_dir_path, opts).is_err());
            assert!(!child_dir_path.exists());
            assert!(!parent_dir_path.exists());
        }
    }
    #[cfg(test)]
    mod ct_main_tests {
        use std::fs::create_dir_all;
        use tempfile::TempDir;

        use super::*;

        #[test]
        fn test_rmdir_main_execution_version() {
            let args_vec = vec![ctcore::ct_util_name(), "--version"];
            let args = args_vec.iter().map(|s| OsString::from(s));
            let result = rmdir_main(args);

            assert!(result.is_err());
        }

        #[test]
        fn test_rmdir_main_execution_other_version() {
            let args_vec = vec![ctcore::ct_util_name(), "-V"];
            let args = args_vec.iter().map(|s| OsString::from(s));
            let result = rmdir_main(args);

            assert!(result.is_err());
        }

        #[test]
        fn test_rmdir_main_execution_help() {
            let args_vec = vec![ctcore::ct_util_name(), "--help"];
            let args = args_vec.iter().map(|s| OsString::from(s));
            let result = rmdir_main(args);

            assert!(result.is_err());
        }

        #[test]
        fn test_rmdir_main_execution_help_short() {
            let args_vec = vec![ctcore::ct_util_name(), "-h"];
            let args = args_vec.iter().map(|s| OsString::from(s));
            let result = rmdir_main(args);

            assert!(result.is_err());
        }

        #[test]
        fn test_rmdir_main_execution_unsupport_help() {
            let args_vec = vec![ctcore::ct_util_name(), "-H"];
            let args = args_vec.iter().map(|s| OsString::from(s));
            let result = rmdir_main(args);

            assert!(result.is_err());
        }

        #[test]
        fn test_rmdir_main_invalid_argument() {
            let args_vec = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let args = args_vec.iter().map(|s| OsString::from(s));
            let result = rmdir_main(args);

            assert!(result.is_err());
        }

        #[test]
        fn test_rmdir_main_support_missing_argument() {
            let args_vec = vec![ctcore::ct_util_name()];
            let args = args_vec.iter().map(|s| OsString::from(s));
            let result = rmdir_main(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_rmdir_main_parents_long() {
            let tmp_dir = TempDir::new().unwrap();
            let dir_path = tmp_dir.path().join("a/b/c");
            create_dir_all(&dir_path).unwrap();

            let args_vec = vec![
                ctcore::ct_util_name(),
                "--parents",
                dir_path.to_str().unwrap(),
            ];
            let args = args_vec.iter().map(|s| OsString::from(s));
            let result = rmdir_main(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_rmdir_main_parents_short() {
            let tmp_dir = TempDir::new().unwrap();
            let dir_path = tmp_dir.path().join("a/b/c");
            create_dir_all(&dir_path).unwrap();

            let args_vec = vec![ctcore::ct_util_name(), "-p", dir_path.to_str().unwrap()];
            let args = args_vec.iter().map(|s| OsString::from(s));
            let result = rmdir_main(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_rmdir_main_ignore_fail_on_non_empty_long() {
            let tmp_dir = TempDir::new().unwrap();
            let dir_path = tmp_dir.path().join("a/b/c");
            create_dir_all(&dir_path).unwrap();

            let args_vec = vec![
                ctcore::ct_util_name(),
                "--ignore-fail-on-non-empty",
                dir_path.to_str().unwrap(),
            ];
            let args = args_vec.iter().map(|s| OsString::from(s));
            let result = rmdir_main(args);

            assert!(result.is_ok());
        }

        #[test]
        fn test_rmdir_main_verbose_long() {
            let tmp_dir = TempDir::new().unwrap();
            let dir_path = tmp_dir.path().join("a/b/c");
            create_dir_all(&dir_path).unwrap();

            let args_vec = vec![
                ctcore::ct_util_name(),
                "--verbose",
                dir_path.to_str().unwrap(),
            ];
            let args = args_vec.iter().map(|s| OsString::from(s));

            let result = rmdir_main(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_rmdir_main_verbose_short() {
            let tmp_dir = TempDir::new().unwrap();
            let dir_path = tmp_dir.path().join("a/b/c");
            create_dir_all(&dir_path).unwrap();

            let args_vec = vec![ctcore::ct_util_name(), "-v", dir_path.to_str().unwrap()];
            let args = args_vec.iter().map(|s| OsString::from(s));

            let result = rmdir_main(args);
            assert!(result.is_ok());
        }
    }

    #[cfg(test)]
    mod ct_app_tests {
        use clap::error::ErrorKind;

        use super::*;

        // rmdir 接口: rmdir [OPTION]... DIRECTORY...
        //
        // Arguments:
        //   [dirs]...
        //
        // Options:
        //       --ignore-fail-on-non-empty  ignore each failure that is solely because a directory is non-empty
        //   -p, --parents                   remove DIRECTORY and its ancestors; e.g.,
        //                                                     'rmdir -p a/b/c' is similar to rmdir a/b/c a/b a
        //   -v, --verbose                   output a diagnostic for every directory processed
        //   -h, --help                      Print help
        //   -V, --version                   Print version

        #[test]
        fn test_ct_app_execution_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];
            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_other_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];

            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
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

            let missing_args = vec![ctcore::ct_util_name()];
            let result = command.try_get_matches_from(missing_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_parents_long() {
            let file_name = "test_ct_app_parents_long";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--parents", file_name];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_parents_short() {
            let file_name = "test_ct_app_parents_short";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-p", file_name];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_ignore_fail_on_non_empty_long() {
            let file_name = "test_ct_app_ignore_fail_on_non_empty_long";
            let command = ct_app();

            let args = vec![
                ctcore::ct_util_name(),
                "--ignore-fail-on-non-empty",
                file_name,
            ];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_verbose_long() {
            let file_name = "test_ct_app_verbose_long";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--verbose", file_name];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_verbose_short() {
            let file_name = "test_ct_app_verbose_short";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-v", file_name];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }
    }
}
