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
use clap::builder::ValueParser;
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use clap::parser::ValuesRef;
use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use ctcore::Tool;
#[cfg(not(windows))]
use ctcore::ct_error::FromIo;
use ctcore::ct_error::{CTResult, CtSimpleError};
#[cfg(not(windows))]
use ctcore::ct_mode;
use ctcore::ct_show_if_err;
use ctcore::{ct_display::Quotable, ct_fs::dir_strip_dot_for_creation};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
use ctcore::libc;
#[cfg(target_os = "linux")]
use selinux::SecurityContext;
#[cfg(target_os = "linux")]
use selinux::label::{Labeler, back_end::File as FileBackEnd};
#[cfg(target_os = "linux")]
use std::ffi::{CString, OsStr};
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStrExt;

const MKDIR_DEFAULT_PERM: u32 = 0o777;

mod mkdir_flags {
    pub const MODE: &str = "mode";
    pub const PARENTS: &str = "parents";
    pub const VERBOSE: &str = "verbose";
    pub const CTX: &str = "ctx";
    pub const CONTEXT: &str = "context";
    pub const DIRS: &str = "dirs";
}

#[cfg(windows)]
fn mkdir_get_mode(
    _arg_matches: &ArgMatches,
    _is_mode_had_minus_prefix: bool,
) -> Result<u32, String> {
    Ok(MKDIR_DEFAULT_PERM)
}

#[cfg(not(windows))]
fn mkdir_get_mode(arg_matches: &ArgMatches, is_mode_had_minus_prefix: bool) -> Result<u32, String> {
    // 未在 Windows 上测试
    let mut new_mode = MKDIR_DEFAULT_PERM;

    if let Some(m) = arg_matches.get_one::<String>(mkdir_flags::MODE) {
        for mode in m.split(',') {
            if mode.chars().any(|c| c.is_ascii_digit()) {
                new_mode = ct_mode::parse_numeric(new_mode, m, true)?;
            } else {
                let c_mode = match is_mode_had_minus_prefix {
                    true => {
                        // clap 解析完成，现在加回前缀
                        format!("-{mode}")
                    }
                    false => mode.to_string(),
                };
                new_mode = ct_mode::parse_symbolic(new_mode, &c_mode, ct_mode::get_umask(), true)?;
            }
        }
        Ok(new_mode)
    } else {
        // 如果未指定模式参数，则返回从 umask 派生的模式
        Ok(!ct_mode::get_umask() & 0o0777)
    }
}

#[cfg(windows)]
fn mkdir_strip_minus_from_mode(_args: &mut [String]) -> bool {
    false
}

#[cfg(not(windows))]
fn mkdir_strip_minus_from_mode(args: &mut [String]) -> bool {
    ct_mode::strip_minus_from_mode(args)
}

#[derive(Default)]
pub struct Mkdir;
impl Tool for Mkdir {
    fn name(&self) -> &'static str {
        "mkdir"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        mkdir_main(args.iter().cloned())
    }
}

pub fn mkdir_main(args: impl ctcore::Args) -> CTResult<()> {
    // 从环境变量读取 locale,符合 GNU coreutils 行为
    let lang_code = std::env::var("LANG")
        .ok()
        .map(|lang| {
            // 处理 LANG=C 或 LANG=C.UTF-8 等情况,使用英文
            if lang.starts_with("C.") || lang == "C" || lang == "POSIX" {
                String::from("en-US")
            } else if lang.starts_with("zh") {
                String::from("zh-CN")
            } else if lang.starts_with("en") {
                String::from("en-US")
            } else {
                // 其他情况尝试解析
                lang.replace('_', "-")
            }
        })
        .unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let mut args = args.collect_lossy();

    // 在我们能用 clap（以及以前的 getopts）解析 'args' 之前，
    // 可能需要移除 MODE 前缀 '-'（例如 "chmod -x FILE"）。
    let is_mode_had_minus_prefix = mkdir_strip_minus_from_mode(&mut args);

    // Linux 特有的选项，未实现
    // opts.optflag("Z", "context", "set SELinux security context" +
    // " of each created directory to CTX"),
    let matches = ct_app().try_get_matches_from(args)?;

    let dirs = matches
        .get_many::<OsString>(mkdir_flags::DIRS)
        .unwrap_or_default();
    if dirs.len() == 0 {
        return Err(CtSimpleError::new(1, "missing operand"));
    }
    let is_verbose = matches.get_flag(mkdir_flags::VERBOSE);
    let is_recursive = matches.get_flag(mkdir_flags::PARENTS);
    let has_z_flag = matches.get_flag(mkdir_flags::CTX);
    let has_context_flag =
        matches.value_source(mkdir_flags::CONTEXT) == Some(clap::parser::ValueSource::CommandLine);
    let context = matches
        .get_one::<OsString>(mkdir_flags::CONTEXT)
        .filter(|v| !v.is_empty());
    let set_context = has_z_flag || has_context_flag;
    let warn_on_unsupported = context.is_some();

    match mkdir_get_mode(&matches, is_mode_had_minus_prefix) {
        Ok(mode) => mkdir_exec(
            dirs,
            is_recursive,
            mode,
            is_verbose,
            context,
            set_context,
            warn_on_unsupported,
        ),
        Err(f) => Err(CtSimpleError::new(1, f)),
    }
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("mkdir.about");
    let usage_description = t!("mkdir.usage");
    let args = vec![
        Arg::new("help")
            .short('h')
            .long("help")
            .help(t!("mkdir.clap.help"))
            .action(ArgAction::Help),
        Arg::new("version")
            .short('V')
            .long("version")
            .help(t!("mkdir.clap.version"))
            .action(ArgAction::Version),
        Arg::new(mkdir_flags::MODE)
            .short('m')
            .long(mkdir_flags::MODE)
            .help(t!("mkdir.clap.mode")),
        Arg::new(mkdir_flags::PARENTS)
            .short('p')
            .long(mkdir_flags::PARENTS)
            .help(t!("mkdir.clap.parents"))
            .action(ArgAction::SetTrue),
        Arg::new(mkdir_flags::VERBOSE)
            .short('v')
            .long(mkdir_flags::VERBOSE)
            .help(t!("mkdir.clap.verbose"))
            .action(ArgAction::SetTrue),
        Arg::new(mkdir_flags::CTX)
            .short('Z')
            .action(ArgAction::SetTrue)
            .help("set the default SELinux security context"),
        Arg::new(mkdir_flags::CONTEXT)
            .long(mkdir_flags::CONTEXT)
            .value_name("CTX")
            .help("if CTX is specified then set the SELinux security context to CTX")
            .value_parser(ValueParser::os_string())
            .num_args(0..=1)
            .require_equals(true)
            .default_missing_value(""),
        Arg::new(mkdir_flags::DIRS)
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
        .disable_help_flag(true)
        .disable_version_flag(true)
        .args(args)
        .after_help(t!("mkdir.after_help"))
}

/**
 * 创建新的目录列表
 */
fn mkdir_exec(
    dirs: ValuesRef<OsString>,
    is_recursive: bool,
    mode: u32,
    is_verbose: bool,
    context: Option<&OsString>,
    set_context: bool,
    warn_on_unsupported: bool,
) -> CTResult<()> {
    for d in dirs {
        let p_buf = PathBuf::from(d);
        let p = p_buf.as_path();

        ct_show_if_err!(mkdir(
            p,
            is_recursive,
            mode,
            is_verbose,
            context,
            set_context,
            warn_on_unsupported
        ));
    }
    Ok(())
}

/// 在给定的 `path` 处创建目录。
///
/// ## 选项
///
/// * `recursive` --- 创建 `path` 的父目录（如果不存在）。
/// * `mode` --- 目录的文件模式（在 windows 上未实现）。
/// * `verbose` --- 为每个创建的目录打印一条消息。
///
/// ## 尾随点
///
/// 为匹配 GNU 的行为，路径的最后一个目录是单个点（如 `some/path/to/.`）的情况会创建（并去除点）。
pub fn mkdir(
    path: &Path,
    is_recursive: bool,
    mode: u32,
    is_verbose: bool,
    context: Option<&OsString>,
    set_context: bool,
    warn_on_unsupported: bool,
) -> CTResult<()> {
    // 特殊情况匹配 GNU 的行为：
    // mkdir -p foo/. 应该工作并只创建 foo/
    // std::fs::create_dir("foo/."); 在纯 Rust 中失败
    let path_buf = dir_strip_dot_for_creation(path);
    let path = path_buf.as_path();

    if path.exists() && path.is_dir() {
        if !is_recursive {
            let err_message = format!("cannot create directory {}: File exists", path.quote());
            return Err(CtSimpleError::new(1, err_message));
        }
        return Ok(());
    }

    mkdir_create_dir(
        path,
        is_recursive,
        is_verbose,
        false,
        mode,
        context,
        set_context,
        warn_on_unsupported,
    )?;
    mkdir_chmod(path, mode)
}

#[cfg(unix)]
fn mkdir_chmod(path: &Path, mode: u32) -> CTResult<()> {
    use std::os::unix::fs::PermissionsExt;

    let mode = std::fs::Permissions::from_mode(mode);

    std::fs::set_permissions(path, mode)
        .map_err_context(|| format!("cannot set permissions {}", path.quote()))
}

#[cfg(windows)]
fn mkdir_chmod(_path: &Path, _mode: u32) -> CTResult<()> {
    // 在 Windows 上，chmod 仅设置只读标志，该标志甚至不适用于目录
    Ok(())
}

// `is_parent` 参数在 windows 上不使用
#[allow(unused_variables)]
#[allow(clippy::too_many_arguments)]
fn mkdir_create_dir(
    path: &Path,
    is_recursive: bool,
    is_verbose: bool,
    is_parent: bool,
    mode: u32,
    context: Option<&OsString>,
    set_context: bool,
    warn_on_unsupported: bool,
) -> CTResult<()> {
    if path == Path::new("") {
        return Ok(());
    }

    if path.exists() && !is_recursive {
        let err_message = format!("cannot create directory {}: File exists", path.quote());
        return Err(CtSimpleError::new(1, err_message));
    }

    if is_recursive {
        if let Some(p) = path.parent() {
            mkdir_create_dir(
                p,
                is_recursive,
                is_verbose,
                true,
                mode,
                context,
                set_context,
                warn_on_unsupported,
            )?;
        } else {
            CtSimpleError::new(1, "failed to create whole tree");
        }
    }

    if let Err(e) = std::fs::create_dir(path) {
        if path.is_dir() {
            Ok(())
        } else {
            let msg = format!("cannot create directory {}: {}", path.quote(), e);
            Err(CtSimpleError::new(1, msg))
        }
    } else {
        if set_context {
            #[cfg(target_os = "linux")]
            {
                if let Err(e) = mkdir_set_security_context(context, path, mode, warn_on_unsupported)
                {
                    return Err(CtSimpleError::new(1, e));
                }
            }
        }
        if is_verbose {
            println!(
                "{}: created directory {}",
                ctcore::ct_util_name(),
                path.quote()
            );
        }
        #[cfg(not(windows))]
        if is_parent {
            // 由 -p 创建的目录权限位设置为 '=rwx,u+wx'，
            // 即 umask 修改后的 'u+wx'
            mkdir_chmod(path, (!ct_mode::get_umask() & 0o0777) | 0o0300)?;
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn mkdir_set_security_context(
    context: Option<&OsString>,
    path: &Path,
    mode: u32,
    warn_on_unsupported: bool,
) -> Result<(), String> {
    if selinux::kernel_support() == selinux::KernelSupport::Unsupported {
        if warn_on_unsupported {
            eprintln!(
                "mkdir: warning: ignoring --context; it requires an SELinux/SMACK-enabled kernel"
            );
        }
        return Ok(());
    }

    match context {
        Some(ctx) => {
            let c_context = os_str_to_c_string(ctx);
            SecurityContext::from_c_str(&c_context, false)
                .set_for_new_file_system_objects(false)
                .map_err(|e| format!("failed to set default file creation context: {e}"))
        }
        None => {
            let labeler = Labeler::<FileBackEnd>::restorecon_default(false).map_err(|e| {
                eprintln!("mkdir: warning: cannot create SELinux labeler: {e}");
                String::new()
            })?;
            let file_mode = libc::S_IFDIR | mode;
            let file_access_mode =
                selinux::FileAccessMode::new(file_mode).expect("mode should be non-zero");
            let default_context = labeler
                .look_up_by_path(path, Some(file_access_mode))
                .map_err(|e| {
                    eprintln!(
                        "mkdir: warning: cannot look up default SELinux context for {}: {e}",
                        path.display()
                    );
                    String::new()
                })?;
            default_context
                .set_for_new_file_system_objects(false)
                .map_err(|e| {
                    eprintln!("mkdir: warning: cannot set default file creation context: {e}");
                    String::new()
                })?;
            Ok(())
        }
    }
}

#[cfg(target_os = "linux")]
fn os_str_to_c_string(os_str: &OsStr) -> CString {
    CString::new(os_str.as_bytes()).expect("Failed to convert OsStr to CString")
}

#[cfg(test)]
mod tests_tool_implementation {
    use crate::Mkdir;
    use ctcore::Tool;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Mkdir;

        // 测试 name 方法
        assert_eq!(tool.name(), "mkdir");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("mkdir"));

        // 测试 execute 方法
        let args = vec![OsString::from("mkdir"), OsString::from("--help")];
        assert!(tool.execute(&args).is_err()); // --help参数通常会返回错误
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(test)]
    mod get_mode_tests {
        use ctcore::libc;

        use super::*;

        fn get_test_matches(args: Vec<&str>) -> ArgMatches {
            ct_app().try_get_matches_from(args).unwrap()
        }

        #[cfg(not(windows))]
        #[test]
        fn test_get_mode_default() {
            let matches = get_test_matches(vec![ctcore::ct_util_name()]);
            let mode = mkdir_get_mode(&matches, false).unwrap();
            assert_eq!(mode, !ct_mode::get_umask() & 0o0777);
        }

        #[cfg(not(windows))]
        #[test]
        fn test_get_mode_numeric() {
            let matches = get_test_matches(vec![ctcore::ct_util_name(), "-m", "0755"]);
            let mode = mkdir_get_mode(&matches, false).unwrap();
            assert_eq!(mode, 0o755);
        }

        #[cfg(not(windows))]
        #[test]
        fn test_get_mode_symbolic() {
            let matches = get_test_matches(vec![ctcore::ct_util_name(), "-m", "u+rwx,go-w"]);
            let mode = mkdir_get_mode(&matches, false).unwrap();
            assert_eq!(mode, 0o755);
        }

        #[cfg(not(windows))]
        #[test]
        fn test_get_mode_mixed() {
            let matches = get_test_matches(vec![ctcore::ct_util_name(), "-m", "0755,u+s"]);
            let mode = mkdir_get_mode(&matches, false);

            assert!(mode.is_err());

            assert_eq!(mode.unwrap_err(), "invalid digit found in string");
        }

        #[cfg(not(windows))]
        #[test]
        fn test_get_mode_invalid() {
            let matches = get_test_matches(vec![ctcore::ct_util_name(), "-m", "invalid"]);
            let mode = mkdir_get_mode(&matches, false);
            assert!(mode.is_err());
        }

        #[cfg(not(windows))]
        #[test]
        fn test_get_mode_combined_symbolic() {
            let matches = get_test_matches(vec![ctcore::ct_util_name(), "-m", "u+rwx,g-w,o+r"]);
            let mode = mkdir_get_mode(&matches, false).unwrap();
            assert_eq!(mode, 0o757);
        }

        #[cfg(not(windows))]
        #[test]
        fn test_get_mode_empty_mode_string() {
            let matches = get_test_matches(vec![ctcore::ct_util_name(), "-m", ""]);
            let mode = mkdir_get_mode(&matches, false);
            assert!(mode.is_err());
        }

        #[cfg(not(windows))]
        #[test]
        fn test_get_mode_partial_mode_string() {
            let matches = get_test_matches(vec![ctcore::ct_util_name(), "-m", "0755,"]);
            let mode = mkdir_get_mode(&matches, false);

            assert!(mode.is_err());
            assert_eq!(mode.unwrap_err(), "invalid digit found in string");
        }

        #[cfg(not(windows))]
        #[test]
        fn test_get_mode_complex_symbolic() {
            let matches = get_test_matches(vec![ctcore::ct_util_name(), "-m", "u+rw,go-rw"]);
            let mode = mkdir_get_mode(&matches, false).unwrap();
            assert_eq!(mode, 0o711);
        }

        #[cfg(not(windows))]
        #[test]
        fn test_get_mode_with_umask() {
            // Set a specific umask for the test and restore afterwards
            let original_umask = unsafe { libc::umask(0o027) };
            let matches = get_test_matches(vec![ctcore::ct_util_name()]);
            let mode = mkdir_get_mode(&matches, false);
            unsafe {
                libc::umask(original_umask);
            }
            let mode = mode.unwrap();
            assert_eq!(mode, 0o750);
        }
        #[cfg(windows)]
        #[test]
        fn test_get_mode_default_windows() {
            let matches = get_test_matches(vec![ctcore::ct_util_name()]);
            let mode = mkdir_get_mode(&matches, false).unwrap();
            assert_eq!(mode, MKDIR_DEFAULT_PERM);
        }

        #[cfg(windows)]
        #[test]
        fn test_get_mode_numeric_windows() {
            let matches = get_test_matches(vec![ctcore::ct_util_name(), "-m", "0755"]);
            let mode = mkdir_get_mode(&matches, false).unwrap();
            assert_eq!(mode, MKDIR_DEFAULT_PERM);
        }

        #[cfg(windows)]
        #[test]
        fn test_get_mode_invalid_windows() {
            let matches = get_test_matches(vec![ctcore::ct_util_name(), "-m", "invalid"]);
            let mode = mkdir_get_mode(&matches, false);
            assert!(mode.is_ok()); // On Windows, invalid mode should return DEFAULT_PERM
            assert_eq!(mode.unwrap(), MKDIR_DEFAULT_PERM);
        }
    }

    #[cfg(test)]
    mod strip_minus_from_mode_tests {
        use super::*;

        #[cfg(not(windows))]
        #[test]
        fn test_strip_minus_from_mode_no_change() {
            let mut args = vec![ctcore::ct_util_name().to_string(), "dir".to_string()];
            let result = mkdir_strip_minus_from_mode(&mut args);
            assert!(!result);
            assert_eq!(
                args,
                vec![ctcore::ct_util_name().to_string(), "dir".to_string()]
            );
        }

        #[cfg(not(windows))]
        #[test]
        fn test_strip_minus_from_mode_with_minus() {
            let mut args = vec![
                ctcore::ct_util_name().to_string(),
                "-m".to_string(),
                "-rw-r--r--".to_string(),
                "dir".to_string(),
            ];
            let result = mkdir_strip_minus_from_mode(&mut args);
            assert!(result);
            assert_eq!(
                args,
                vec![
                    ctcore::ct_util_name().to_string(),
                    "-m".to_string(),
                    "rw-r--r--".to_string(),
                    "dir".to_string()
                ]
            );
        }

        #[cfg(not(windows))]
        #[test]
        fn test_strip_minus_from_mode_with_multiple_minus() {
            let mut args = vec![
                ctcore::ct_util_name().to_string(),
                "-m".to_string(),
                "-rw-r--r--".to_string(),
                "-m".to_string(),
                "-rwxr-xr-x".to_string(),
                "dir".to_string(),
            ];
            let result = mkdir_strip_minus_from_mode(&mut args);
            assert!(result);
            assert_eq!(
                args,
                vec![
                    ctcore::ct_util_name().to_string(),
                    "-m".to_string(),
                    "rw-r--r--".to_string(),
                    "-m".to_string(),
                    "-rwxr-xr-x".to_string(),
                    "dir".to_string()
                ]
            );
        }

        #[cfg(not(windows))]
        #[test]
        fn test_strip_minus_from_mode_no_mode() {
            let mut args = vec![ctcore::ct_util_name().to_string(), "dir".to_string()];
            let result = mkdir_strip_minus_from_mode(&mut args);
            assert!(!result);
            assert_eq!(
                args,
                vec![ctcore::ct_util_name().to_string(), "dir".to_string()]
            );
        }

        #[cfg(not(windows))]
        #[test]
        fn test_strip_minus_from_mode_mixed_args() {
            let mut args = vec![
                ctcore::ct_util_name().to_string(),
                "-v".to_string(),
                "-m".to_string(),
                "-rw-r--r--".to_string(),
                "dir".to_string(),
            ];
            let result = mkdir_strip_minus_from_mode(&mut args);
            assert!(result);
            assert_eq!(
                args,
                vec![
                    ctcore::ct_util_name().to_string(),
                    "-v".to_string(),
                    "-m".to_string(),
                    "rw-r--r--".to_string(),
                    "dir".to_string()
                ]
            );
        }

        #[cfg(not(windows))]
        #[test]
        fn test_strip_minus_from_mode_only_minus() {
            let mut args = vec![
                ctcore::ct_util_name().to_string(),
                "-".to_string(),
                "dir".to_string(),
            ];
            let result = mkdir_strip_minus_from_mode(&mut args);
            assert!(!result);
            assert_eq!(
                args,
                vec![
                    ctcore::ct_util_name().to_string(),
                    "-".to_string(),
                    "dir".to_string()
                ]
            );
        }

        #[cfg(not(windows))]
        #[test]
        fn test_strip_minus_from_mode_mode_at_end() {
            let mut args = vec![
                ctcore::ct_util_name().to_string(),
                "dir".to_string(),
                "-m".to_string(),
                "-rw-r--r--".to_string(),
            ];
            let result = mkdir_strip_minus_from_mode(&mut args);
            assert!(result);
            assert_eq!(
                args,
                vec![
                    ctcore::ct_util_name().to_string(),
                    "dir".to_string(),
                    "-m".to_string(),
                    "rw-r--r--".to_string()
                ]
            );
        }

        #[cfg(not(windows))]
        #[test]
        fn test_strip_minus_from_mode_multiple_dirs() {
            let mut args = vec![
                ctcore::ct_util_name().to_string(),
                "-m".to_string(),
                "-rw-r--r--".to_string(),
                "dir1".to_string(),
                "dir2".to_string(),
            ];
            let result = mkdir_strip_minus_from_mode(&mut args);
            assert!(result);
            assert_eq!(
                args,
                vec![
                    ctcore::ct_util_name().to_string(),
                    "-m".to_string(),
                    "rw-r--r--".to_string(),
                    "dir1".to_string(),
                    "dir2".to_string()
                ]
            );
        }

        #[cfg(not(windows))]
        #[test]
        fn test_strip_minus_from_mode_no_minus_prefix() {
            let mut args = vec![
                ctcore::ct_util_name().to_string(),
                "-m".to_string(),
                "rw-r--r--".to_string(),
                "dir".to_string(),
            ];
            let result = mkdir_strip_minus_from_mode(&mut args);
            assert!(!result);
            assert_eq!(
                args,
                vec![
                    ctcore::ct_util_name().to_string(),
                    "-m".to_string(),
                    "rw-r--r--".to_string(),
                    "dir".to_string()
                ]
            );
        }

        #[cfg(not(windows))]
        #[test]
        fn test_strip_minus_from_mode_no_dirs() {
            let mut args = vec![
                ctcore::ct_util_name().to_string(),
                "-m".to_string(),
                "-rw-r--r--".to_string(),
            ];
            let result = mkdir_strip_minus_from_mode(&mut args);
            assert!(result);
            assert_eq!(
                args,
                vec![
                    ctcore::ct_util_name().to_string(),
                    "-m".to_string(),
                    "rw-r--r--".to_string()
                ]
            );
        }

        #[cfg(not(windows))]
        #[test]
        fn test_strip_minus_from_mode_empty_args() {
            let mut args: Vec<String> = vec![];
            let result = mkdir_strip_minus_from_mode(&mut args);
            assert!(!result);
            assert!(args.is_empty());
        }
    }

    #[cfg(test)]
    mod exec_tests {
        use std::ffi::OsString;
        use std::fs;

        use clap::ArgMatches;

        use tempfile::tempdir;

        use super::*;

        fn get_test_matches(args: Vec<&str>) -> ArgMatches {
            ct_app().try_get_matches_from(args).unwrap()
        }

        #[test]
        fn test_exec_single_dir() {
            let temp_dir = tempdir().unwrap();
            let test_path = temp_dir.path().join("testdir");

            let matches =
                get_test_matches(vec![ctcore::ct_util_name(), test_path.to_str().unwrap()]);
            let dirs = matches
                .get_many::<OsString>(mkdir_flags::DIRS)
                .unwrap_or_default();

            let result = mkdir_exec(dirs, false, 0o755, false, None, false, false);
            assert!(result.is_ok());
            assert!(test_path.exists());
            assert!(test_path.is_dir());
        }

        #[test]
        fn test_exec_multiple_dirs() {
            let temp_dir = tempdir().unwrap();
            let test_path1 = temp_dir.path().join("dir1");
            let test_path2 = temp_dir.path().join("dir2");

            let matches = get_test_matches(vec![
                ctcore::ct_util_name(),
                test_path1.to_str().unwrap(),
                test_path2.to_str().unwrap(),
            ]);
            let dirs = matches
                .get_many::<OsString>(mkdir_flags::DIRS)
                .unwrap_or_default();

            let result = mkdir_exec(dirs, false, 0o755, false, None, false, false);
            assert!(result.is_ok());
            assert!(test_path1.exists());
            assert!(test_path1.is_dir());
            assert!(test_path2.exists());
            assert!(test_path2.is_dir());
        }

        #[test]
        fn test_exec_recursive() {
            let temp_dir = tempdir().unwrap();
            let test_path = temp_dir.path().join("dir1/dir2/dir3");

            let matches = get_test_matches(vec![
                ctcore::ct_util_name(),
                "-p",
                test_path.to_str().unwrap(),
            ]);
            let dirs = matches
                .get_many::<OsString>(mkdir_flags::DIRS)
                .unwrap_or_default();

            let result = mkdir_exec(dirs, true, 0o755, false, None, false, false);
            assert!(result.is_ok());
            assert!(test_path.exists());
            assert!(test_path.is_dir());
        }

        #[test]
        fn test_exec_verbose() {
            let temp_dir = tempdir().unwrap();
            let test_path = temp_dir.path().join("testdir");

            let matches = get_test_matches(vec![
                ctcore::ct_util_name(),
                "-v",
                test_path.to_str().unwrap(),
            ]);
            let dirs = matches
                .get_many::<OsString>(mkdir_flags::DIRS)
                .unwrap_or_default();

            let result = mkdir_exec(dirs, false, 0o755, true, None, false, false);
            assert!(result.is_ok());
            assert!(test_path.exists());
            assert!(test_path.is_dir());
        }

        #[test]
        fn test_exec_mkdir_error() {
            let matches = get_test_matches(vec![ctcore::ct_util_name(), "testdir"]);
            let dirs = matches
                .get_many::<OsString>(mkdir_flags::DIRS)
                .unwrap_or_default();

            let result = mkdir_exec(dirs, false, 0o755, false, None, false, false);
            assert!(result.is_ok());
        }

        #[test]
        fn test_exec_with_real_mkdir() {
            let temp_dir = tempdir().unwrap();
            let test_path = temp_dir.path().join("testdir");

            let matches =
                get_test_matches(vec![ctcore::ct_util_name(), test_path.to_str().unwrap()]);
            let dirs = matches
                .get_many::<OsString>(mkdir_flags::DIRS)
                .unwrap_or_default();

            let result = mkdir_exec(dirs, false, 0o755, false, None, false, false);
            assert!(result.is_ok());
            assert!(test_path.exists());
            assert!(test_path.is_dir());
        }

        #[test]
        fn test_exec_existing_dir() {
            let temp_dir = tempdir().unwrap();
            let test_path = temp_dir.path().join("test_exec_existing_dir");
            fs::create_dir(&test_path).unwrap();

            let matches =
                get_test_matches(vec![ctcore::ct_util_name(), test_path.to_str().unwrap()]);
            let dirs = matches
                .get_many::<OsString>(mkdir_flags::DIRS)
                .unwrap_or_default();

            let result = mkdir_exec(dirs, false, 0o755, false, None, false, false);
            assert!(result.is_ok());
            assert!(test_path.exists());
            assert!(test_path.is_dir());
        }

        #[test]
        fn test_exec_nested_dirs() {
            let temp_dir = tempdir().unwrap();
            let test_path = temp_dir.path().join("dir1/dir2/dir3");

            let matches = get_test_matches(vec![
                ctcore::ct_util_name(),
                "-p",
                test_path.to_str().unwrap(),
            ]);
            let dirs = matches
                .get_many::<OsString>(mkdir_flags::DIRS)
                .unwrap_or_default();

            let result = mkdir_exec(dirs, true, 0o755, false, None, false, false);
            assert!(result.is_ok());
            assert!(test_path.exists());
            assert!(test_path.is_dir());
        }

        #[test]
        fn test_exec_empty_dirs() {
            let matches = get_test_matches(vec![ctcore::ct_util_name()]);
            let dirs = matches
                .get_many::<OsString>(mkdir_flags::DIRS)
                .unwrap_or_default();

            let result = mkdir_exec(dirs, false, 0o755, false, None, false, false);
            assert!(result.is_ok());
        }

        #[test]
        fn test_exec_invalid_path() {
            let matches = get_test_matches(vec![ctcore::ct_util_name(), "\0invalid"]);
            let dirs = matches
                .get_many::<OsString>(mkdir_flags::DIRS)
                .unwrap_or_default();

            let result = mkdir_exec(dirs, false, 0o755, false, None, false, false);
            assert!(result.is_ok());
        }

        #[test]
        fn test_exec_recursive_existing_path() {
            let temp_dir = tempdir().unwrap();
            let test_path = temp_dir.path().join("test_exec_recursive_existing_path");
            fs::create_dir(&test_path).unwrap();

            let nested_path = test_path.join("nested");

            let matches = get_test_matches(vec![
                ctcore::ct_util_name(),
                "-p",
                nested_path.to_str().unwrap(),
            ]);
            let dirs = matches
                .get_many::<OsString>(mkdir_flags::DIRS)
                .unwrap_or_default();

            let result = mkdir_exec(dirs, true, 0o755, false, None, false, false);
            assert!(result.is_ok());
            assert!(nested_path.exists());
            assert!(nested_path.is_dir());
        }

        #[test]
        fn test_exec_verbose_existing_dir() {
            let temp_dir = tempdir().unwrap();
            let test_path = temp_dir.path().join("test_exec_verbose_existing_dir");
            fs::create_dir(&test_path).unwrap();

            let matches = get_test_matches(vec![
                ctcore::ct_util_name(),
                "-v",
                test_path.to_str().unwrap(),
            ]);
            let dirs = matches
                .get_many::<OsString>(mkdir_flags::DIRS)
                .unwrap_or_default();

            let result = mkdir_exec(dirs, false, 0o755, true, None, false, false);
            assert!(result.is_ok());
            assert!(test_path.exists());
            assert!(test_path.is_dir());
        }

        #[test]
        fn test_exec_with_different_modes() {
            let temp_dir = tempdir().unwrap();
            let test_path = temp_dir.path().join("test_exec_with_different_modes");

            let matches = get_test_matches(vec![
                ctcore::ct_util_name(),
                "-m",
                "0700",
                test_path.to_str().unwrap(),
            ]);
            let dirs = matches
                .get_many::<OsString>(mkdir_flags::DIRS)
                .unwrap_or_default();

            let result = mkdir_exec(dirs, false, 0o700, false, None, false, false);
            assert!(result.is_ok());
            assert!(test_path.exists());
            assert!(test_path.is_dir());

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let metadata = fs::metadata(&test_path).unwrap();
                let permissions = metadata.permissions();
                assert_eq!(permissions.mode() & 0o777, 0o700);
            }
        }
    }

    #[cfg(test)]
    mod mkdir_tests {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        use std::path::Path;

        use tempfile::tempdir;

        use super::*;

        #[test]
        fn test_mkdir_create_single_directory() {
            let test_dir = Path::new("test_mkdir_single_dir");
            if test_dir.exists() {
                fs::remove_dir_all(test_dir).unwrap();
            }

            assert!(mkdir(test_dir, false, 0o755, false, None, false, false).is_ok());
            assert!(test_dir.exists());
            assert!(test_dir.is_dir());

            let metadata = fs::metadata(test_dir).unwrap();
            let permissions = metadata.permissions();
            assert_eq!(permissions.mode() & 0o777, 0o755);

            fs::remove_dir_all(test_dir).unwrap();
        }

        #[test]
        fn test_mkdir_create_nested_directories() {
            let test_dir = Path::new("test_mkdir_nested_dir/subdir1/subdir2");
            if test_dir.exists() {
                fs::remove_dir_all(test_dir).unwrap();
            }

            assert!(mkdir(test_dir, true, 0o755, false, None, false, false).is_ok());
            assert!(test_dir.exists());
            assert!(test_dir.is_dir());

            let metadata = fs::metadata(test_dir).unwrap();
            let permissions = metadata.permissions();
            assert_eq!(permissions.mode() & 0o777, 0o755);

            fs::remove_dir_all(test_dir.parent().unwrap().parent().unwrap()).unwrap();
        }

        #[test]
        fn test_mkdir_directory_already_exists() {
            let test_dir = Path::new("test_mkdir_existing_dir");
            if !test_dir.exists() {
                fs::create_dir(test_dir).unwrap();
            }

            assert!(mkdir(test_dir, false, 0o755, false, None, false, false).is_err());

            fs::remove_dir_all(test_dir).unwrap();
        }

        #[test]
        fn test_mkdir_with_trailing_dot() {
            let test_dir = Path::new("test_mkdir_with_dot/.");
            if test_dir.parent().unwrap().exists() {
                fs::remove_dir_all(test_dir.parent().unwrap()).unwrap();
            }

            assert!(mkdir(test_dir, true, 0o755, false, None, false, false).is_ok());
            assert!(!test_dir.parent().unwrap().exists());
            assert!(!test_dir.parent().unwrap().is_dir());
            let remove_dir = Path::new("test_mkdir_with_dot");
            if remove_dir.exists() {
                fs::remove_dir_all(remove_dir).unwrap();
            }
        }

        #[test]
        fn test_mkdir_verbose() {
            let test_dir = Path::new("test_mkdir_verbose_dir");
            if test_dir.exists() {
                fs::remove_dir_all(test_dir).unwrap();
            }

            let output = std::panic::catch_unwind(|| {
                mkdir(test_dir, false, 0o755, true, None, false, false).unwrap();
            });
            assert!(output.is_ok());
            assert!(test_dir.exists());
            assert!(test_dir.is_dir());

            fs::remove_dir_all(test_dir).unwrap();
        }

        #[test]
        fn test_mkdir_create_multiple_directories() {
            let dirs = vec![
                Path::new("multi_dir1"),
                Path::new("multi_dir2"),
                Path::new("multi_dir3"),
            ];

            for dir in &dirs {
                if dir.exists() {
                    fs::remove_dir_all(dir).unwrap();
                }
            }

            for dir in &dirs {
                assert!(mkdir(dir, false, 0o755, false, None, false, false).is_ok());
                assert!(dir.exists());
                assert!(dir.is_dir());
            }

            for dir in &dirs {
                fs::remove_dir_all(dir).unwrap();
            }
        }

        #[test]
        fn test_mkdir_with_existing_file() {
            let test_file = Path::new("test_mkdir_existing_file");
            if !test_file.exists() {
                fs::File::create(test_file).unwrap();
            }

            assert!(mkdir(test_file, false, 0o755, false, None, false, false).is_err());

            fs::remove_file(test_file).unwrap();
        }

        #[test]
        fn test_mkdir_with_no_permissions() {
            let temp_dir = tempdir().unwrap();
            let parent_dir = temp_dir.path().join("test_mkdir_no_permission_dir");
            fs::create_dir(&parent_dir).unwrap();
            let test_dir = parent_dir.join("child_dir");

            let is_root = ctcore::ct_process::geteuid() == 0;
            let original_permissions = fs::metadata(&parent_dir).unwrap().permissions();
            let mut no_write_permissions = original_permissions.clone();
            no_write_permissions.set_mode(original_permissions.mode() & !0o222);
            fs::set_permissions(&parent_dir, no_write_permissions).unwrap();

            let result = mkdir(&test_dir, false, 0o755, false, None, false, false);

            fs::set_permissions(&parent_dir, original_permissions).unwrap();
            if !is_root {
                assert!(result.is_err());
            }
        }

        #[test]
        fn test_mkdir_with_non_existent_parent() {
            let test_dir = Path::new("test_mkdir_non_existent_parent/child_dir");
            let parent_dir = test_dir.parent().unwrap();
            if parent_dir.exists() {
                fs::remove_dir_all(parent_dir).unwrap();
            }

            assert!(mkdir(test_dir, false, 0o755, false, None, false, false).is_err());

            assert!(mkdir(parent_dir, false, 0o755, false, None, false, false).is_ok());
            assert!(mkdir(test_dir, false, 0o755, false, None, false, false).is_ok());

            fs::remove_dir_all(parent_dir).unwrap();
        }

        #[test]
        fn test_mkdir_recursive_with_partial_existing_parents() {
            let parent_dir = Path::new("test_mkdir_partial_existing_parents");
            let child_dir = parent_dir.join("child_dir1/child_dir2");

            if parent_dir.exists() {
                fs::remove_dir_all(parent_dir).unwrap();
            }
            fs::create_dir(parent_dir).unwrap();
            fs::create_dir(parent_dir.join("child_dir1")).unwrap();

            assert!(mkdir(&child_dir, true, 0o755, false, None, false, false).is_ok());
            assert!(child_dir.exists());

            fs::remove_dir_all(parent_dir).unwrap();
        }

        #[test]
        fn test_mkdir_with_different_permissions() {
            let test_dir = Path::new("test_mkdir_diff_permissions");
            if test_dir.exists() {
                fs::remove_dir_all(test_dir).unwrap();
            }

            assert!(mkdir(test_dir, false, 0o700, false, None, false, false).is_ok());
            assert!(test_dir.exists());
            assert!(test_dir.is_dir());

            let metadata = fs::metadata(test_dir).unwrap();
            let permissions = metadata.permissions();
            assert_eq!(permissions.mode() & 0o777, 0o700);

            fs::remove_dir_all(test_dir).unwrap();
        }

        #[test]
        fn test_mkdir_recursive_with_trailing_dot() {
            let test_dir = Path::new("test_mkdir_recursive_with_dot/.");
            if test_dir.parent().unwrap().exists() {
                fs::remove_dir_all(test_dir.parent().unwrap()).unwrap();
            }

            assert!(mkdir(test_dir, true, 0o755, false, None, false, false).is_ok());
            assert!(!test_dir.parent().unwrap().exists());
            assert!(!test_dir.parent().unwrap().is_dir());

            let remove = Path::new("test_mkdir_recursive_with_dot");
            if remove.exists() {
                fs::remove_dir_all(remove).unwrap();
            }
        }

        #[test]
        fn test_mkdir_with_special_characters() {
            let test_dir = Path::new("test_mkdir_special_!@#$%^&*()_+{}[];'.dir");
            if test_dir.exists() {
                fs::remove_dir_all(test_dir).unwrap();
            }

            assert!(mkdir(test_dir, false, 0o755, false, None, false, false).is_ok());
            assert!(test_dir.exists());
            assert!(test_dir.is_dir());

            fs::remove_dir_all(test_dir).unwrap();
        }

        #[test]
        fn test_mkdir_with_empty_string() {
            let test_dir = Path::new("");
            assert!(mkdir(test_dir, false, 0o755, false, None, false, false).is_err());
        }

        #[test]
        fn test_mkdir_with_relative_path() {
            let test_dir = Path::new("relative/test_mkdir_dir");
            if test_dir.exists() {
                fs::remove_dir_all(test_dir).unwrap();
            }

            assert!(mkdir(test_dir, true, 0o755, false, None, false, false).is_ok());
            assert!(test_dir.exists());
            assert!(test_dir.is_dir());

            fs::remove_dir_all(test_dir.parent().unwrap()).unwrap();
        }

        #[test]
        fn test_mkdir_with_absolute_path() {
            let test_dir = Path::new("/tmp/test_mkdir_absolute_dir");
            if test_dir.exists() {
                fs::remove_dir_all(test_dir).unwrap();
            }

            assert!(mkdir(test_dir, false, 0o755, false, None, false, false).is_ok());
            assert!(test_dir.exists());
            assert!(test_dir.is_dir());

            fs::remove_dir_all(test_dir).unwrap();
        }

        #[test]
        fn test_mkdir_with_symlink_as_parent() {
            let parent_dir = Path::new("test_symlink_parent_dir");
            let test_symlink = Path::new("test_symlink_parent");
            let child_dir = test_symlink.join("child_dir");

            if parent_dir.exists() {
                fs::remove_dir_all(parent_dir).unwrap();
            }
            if test_symlink.exists() {
                fs::remove_dir_all(test_symlink).unwrap();
            }

            fs::create_dir(parent_dir).unwrap();
            std::os::unix::fs::symlink(parent_dir, test_symlink).unwrap();

            assert!(mkdir(&child_dir, true, 0o755, false, None, false, false).is_ok());
            assert!(child_dir.exists());

            fs::remove_dir_all(test_symlink).unwrap();
            fs::remove_dir_all(parent_dir).unwrap();
        }
    }

    #[cfg(test)]
    mod chmod_tests {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        use std::path::Path;

        use super::*;

        #[test]
        fn test_chmod_set_permissions() {
            let test_dir = Path::new("test_chmod_dir");
            if test_dir.exists() {
                fs::remove_dir_all(test_dir).unwrap();
            }

            fs::create_dir(test_dir).unwrap();
            assert!(mkdir_chmod(test_dir, 0o755).is_ok());

            let metadata = fs::metadata(test_dir).unwrap();
            let permissions = metadata.permissions();
            assert_eq!(permissions.mode() & 0o777, 0o755);

            fs::remove_dir_all(test_dir).unwrap();
        }

        #[test]
        fn test_chmod_set_different_permissions() {
            let test_dir = Path::new("test_chmod_different_dir");
            if test_dir.exists() {
                fs::remove_dir_all(test_dir).unwrap();
            }

            fs::create_dir(test_dir).unwrap();
            assert!(mkdir_chmod(test_dir, 0o700).is_ok());

            let metadata = fs::metadata(test_dir).unwrap();
            let permissions = metadata.permissions();
            assert_eq!(permissions.mode() & 0o777, 0o700);

            fs::remove_dir_all(test_dir).unwrap();
        }

        #[test]
        fn test_chmod_on_non_existent_dir() {
            let test_dir = Path::new("test_non_existent_dir");
            if test_dir.exists() {
                fs::remove_dir_all(test_dir).unwrap();
            }

            assert!(mkdir_chmod(test_dir, 0o755).is_err());
        }

        #[test]
        fn test_chmod_on_file() {
            let test_file = Path::new("test_chmod_file");
            if test_file.exists() {
                fs::remove_file(test_file).unwrap();
            }

            fs::File::create(test_file).unwrap();
            assert!(mkdir_chmod(test_file, 0o644).is_ok());

            let metadata = fs::metadata(test_file).unwrap();
            let permissions = metadata.permissions();
            assert_eq!(permissions.mode() & 0o777, 0o644);

            fs::remove_file(test_file).unwrap();
        }

        #[test]
        fn test_chmod_preserves_existing_permissions() {
            let test_file = Path::new("test_chmod_preserve_permissions_file");
            if test_file.exists() {
                fs::remove_file(test_file).unwrap();
            }

            fs::File::create(test_file).unwrap();
            let original_metadata = fs::metadata(test_file).unwrap();
            let original_permissions = original_metadata.permissions();

            assert!(mkdir_chmod(test_file, 0o644).is_ok());
            let metadata = fs::metadata(test_file).unwrap();
            let permissions = metadata.permissions();
            assert_eq!(permissions.mode() & 0o777, 0o644);

            // Restore original permissions
            fs::set_permissions(test_file, original_permissions.clone()).unwrap();
            let restored_metadata = fs::metadata(test_file).unwrap();
            let restored_permissions = restored_metadata.permissions();
            assert_eq!(restored_permissions.mode(), original_permissions.mode());

            fs::remove_file(test_file).unwrap();
        }

        #[test]
        fn test_chmod_on_symbolic_link() {
            let test_file = Path::new("test_chmod_symlink_target");
            let test_symlink = Path::new("test_chmod_symlink");
            if test_file.exists() {
                fs::remove_file(test_file).unwrap();
            }
            if test_symlink.exists() {
                fs::remove_file(test_symlink).unwrap();
            }

            fs::File::create(test_file).unwrap();
            std::os::unix::fs::symlink(test_file, test_symlink).unwrap();
            assert!(mkdir_chmod(test_file, 0o644).is_ok());

            let metadata = fs::metadata(test_file).unwrap();
            let permissions = metadata.permissions();
            assert_eq!(permissions.mode() & 0o777, 0o644);

            fs::remove_file(test_symlink).unwrap();
            fs::remove_file(test_file).unwrap();
        }

        #[test]
        fn test_chmod_no_effect_on_directory() {
            let test_dir = Path::new("test_chmod_no_effect_dir");
            if test_dir.exists() {
                //fs::remove_dir_all(test_dir).unwrap();
            } else {
                fs::create_dir(test_dir).unwrap();
            }

            assert!(mkdir_chmod(test_dir, 0o755).is_ok());
            let metadata = fs::metadata(test_dir).unwrap();
            let permissions = metadata.permissions();
            assert_eq!(permissions.mode() & 0o777, 0o755);

            assert!(mkdir_chmod(test_dir, 0o644).is_ok());
            let metadata = fs::metadata(test_dir).unwrap();
            let permissions = metadata.permissions();
            assert_eq!(permissions.mode() & 0o777, 0o644);

            //fs::remove_dir_all(test_dir).unwrap();
        }
    }

    #[cfg(test)]
    mod create_dir_tests {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        use std::path::Path;

        use tempfile::tempdir;

        use super::*;

        #[test]
        fn test_create_single_dir() {
            let test_dir = Path::new("test_single_dir");
            if test_dir.exists() {
                fs::remove_dir_all(test_dir).unwrap();
            }

            assert!(
                mkdir_create_dir(test_dir, false, false, false, 0o777, None, false, false).is_ok()
            );
            assert!(test_dir.exists());
            assert!(test_dir.is_dir());

            fs::remove_dir_all(test_dir).unwrap();
        }

        #[test]
        fn test_create_nested_dir() {
            let test_dir = Path::new("test_nested_dir/subdir1/subdir2");
            if test_dir.exists() {
                fs::remove_dir_all(test_dir).unwrap();
            }

            assert!(
                mkdir_create_dir(test_dir, true, false, false, 0o777, None, false, false).is_ok()
            );
            assert!(test_dir.exists());
            assert!(test_dir.is_dir());

            fs::remove_dir_all(Path::new("test_nested_dir")).unwrap();
        }

        #[test]
        fn test_create_dir_already_exists() {
            let test_dir = Path::new("test_existing_dir");
            if !test_dir.exists() {
                fs::create_dir(test_dir).unwrap();
            }

            assert!(
                mkdir_create_dir(test_dir, false, false, false, 0o777, None, false, false).is_err()
            );

            fs::remove_dir_all(test_dir).unwrap();
        }

        #[test]
        fn test_create_dir_with_recursive_false() {
            let parent_dir = Path::new("parent_dir");
            let test_dir = parent_dir.join("child_dir");
            if test_dir.exists() {
                fs::remove_dir_all(test_dir.as_path()).unwrap();
            }

            assert!(
                mkdir_create_dir(
                    test_dir.as_path(),
                    false,
                    false,
                    false,
                    0o777,
                    None,
                    false,
                    false
                )
                .is_err()
            );
        }

        #[test]
        fn test_create_dir_with_permissions() {
            let test_dir = Path::new("test_permission_dir");
            if test_dir.exists() {
                fs::remove_dir_all(test_dir).unwrap();
            }

            assert!(
                mkdir_create_dir(test_dir, false, false, false, 0o777, None, false, false).is_ok()
            );
            assert!(mkdir_chmod(test_dir, 0o755).is_ok());

            let metadata = fs::metadata(test_dir).unwrap();
            let permissions = metadata.permissions();
            assert_eq!(permissions.mode() & 0o777, 0o755);

            fs::remove_dir_all(test_dir).unwrap();
        }

        #[test]
        fn test_create_dir_verbose() {
            let test_dir = Path::new("test_verbose_dir");
            if test_dir.exists() {
                fs::remove_dir_all(test_dir).unwrap();
            }

            assert!(
                mkdir_create_dir(test_dir, false, true, false, 0o777, None, false, false).is_ok()
            );
            assert!(test_dir.exists());

            fs::remove_dir_all(test_dir).unwrap();
        }

        #[test]
        fn test_create_recursive_with_trailing_dot() {
            let test_dir = Path::new("test_recursive_dir_with_dot/.");
            if test_dir.parent().unwrap().exists() {
                fs::remove_dir_all(test_dir.parent().unwrap()).unwrap();
            }

            assert!(
                mkdir_create_dir(test_dir, true, false, false, 0o777, None, false, false).is_err()
            );
            assert!(!test_dir.parent().unwrap().exists());
            assert!(!test_dir.parent().unwrap().is_dir());
        }

        #[test]
        fn test_create_dir_with_invalid_path() {
            let test_dir = Path::new("");
            assert!(
                mkdir_create_dir(test_dir, false, false, false, 0o777, None, false, false).is_ok()
            );
        }

        #[test]
        fn test_create_multiple_directories() {
            let dirs = vec![
                Path::new("multi_dir1"),
                Path::new("multi_dir2"),
                Path::new("multi_dir3"),
            ];

            for dir in &dirs {
                if dir.exists() {
                    fs::remove_dir_all(dir).unwrap();
                }
            }

            for dir in &dirs {
                assert!(
                    mkdir_create_dir(dir, false, false, false, 0o777, None, false, false).is_ok()
                );
                assert!(dir.exists());
                assert!(dir.is_dir());
            }

            for dir in &dirs {
                fs::remove_dir_all(dir).unwrap();
            }
        }

        #[test]
        fn test_create_dir_with_existing_file() {
            let test_file = Path::new("test_existing_file");
            if !test_file.exists() {
                fs::File::create(test_file).unwrap();
            }

            assert!(
                mkdir_create_dir(test_file, false, false, false, 0o777, None, false, false)
                    .is_err()
            );

            fs::remove_file(test_file).unwrap();
        }

        #[test]
        fn test_create_dir_with_no_permissions() {
            let temp_dir = tempdir().unwrap();
            let parent_dir = temp_dir.path().join("test_no_permission_dir");
            fs::create_dir(&parent_dir).unwrap();
            let test_dir = parent_dir.join("child_dir");

            let is_root = ctcore::ct_process::geteuid() == 0;
            let original_permissions = fs::metadata(&parent_dir).unwrap().permissions();
            let mut no_write_permissions = original_permissions.clone();
            no_write_permissions.set_mode(original_permissions.mode() & !0o222);
            fs::set_permissions(&parent_dir, no_write_permissions).unwrap();

            let result =
                mkdir_create_dir(&test_dir, false, false, false, 0o777, None, false, false);

            fs::set_permissions(&parent_dir, original_permissions).unwrap();
            if !is_root {
                assert!(result.is_err());
            }
        }
    }
    #[cfg(test)]
    mod ct_main_tests {
        use tempfile::tempdir;

        use super::*;

        #[test]
        fn test_ct_main_execution_version() {
            let args = [ctcore::ct_util_name(), "--version"];
            let result = mkdir_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_execution_other_version() {
            let args = [ctcore::ct_util_name(), "-V"];
            let result = mkdir_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_execution_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = mkdir_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_execution_help_short() {
            let args = [ctcore::ct_util_name(), "-h"];
            let result = mkdir_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_execution_unsupport_help() {
            let args = [ctcore::ct_util_name(), "-H"];
            let result = mkdir_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_invalid_argument() {
            let args = [ctcore::ct_util_name(), "--invalid-argument"];
            let result = mkdir_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_parents_long() {
            let args = [ctcore::ct_util_name(), "--parents"];
            let result = mkdir_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_parents_short() {
            let args = [ctcore::ct_util_name(), "-p"];
            let result = mkdir_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_mode_long() {
            let args = [ctcore::ct_util_name(), "--mode"];
            let result = mkdir_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_mode_short() {
            let args = [ctcore::ct_util_name(), "-m"];
            let result = mkdir_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_verbose_long() {
            let args = [ctcore::ct_util_name(), "--verbose"];
            let result = mkdir_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_verbose_short() {
            let args = [ctcore::ct_util_name(), "-v"];
            let result = mkdir_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_support_missing_argument() {
            let args = [ctcore::ct_util_name()];
            let result = mkdir_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_mode_u() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test_ct_main_mode_long_u");

            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-m", "u+rwx,go-w", file_name];
            let result = mkdir_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_mode_long_u() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test_ct_main_mode_long_u");

            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--mode", "u+rwx,go-w", file_name];
            let result = mkdir_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_mode_long_r() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test_ct_main_mode_long_r");

            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--mode", "+rwx", file_name];
            let result = mkdir_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_mode_long_u_s_755() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test_ct_main_mode_long_r");

            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--mode", "0755,u+s", file_name];
            let result = mkdir_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_main_mode_long_u0755() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("test_ct_main_mode_long_u0755");

            let file_name = file_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-m", "0755", file_name];
            let result = mkdir_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
    }
    #[cfg(test)]
    mod ct_app_tests {
        use clap::error::ErrorKind;

        use super::*;

        // mkdir 接口: mkdir [OPTION]... DIRECTORY...
        //
        // Arguments:
        //   [dirs]...
        //
        // Options:
        //   -m, --mode <mode>  set file mode (not implemented on windows)
        //   -p, --parents      make parent directories as needed
        //   -v, --verbose      print a message for each printed directory
        //   -h, --help         Print help
        //   -V, --version      Print version
        //
        //   Each MODE is of the form '[ugoa]*([-+=]([rwxXst]*|[ugo]))+|[-+=]?[0-7]+'.

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
        fn test_ct_app_parents_long() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "--parents"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_parents_short() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-p"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_mode_long() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "--mode"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_mode_short() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-m"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ct_app_verbose_long() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "--verbose"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_verbose_short() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-v"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_ok());
        }
        #[test]
        fn test_ct_app_support_missing_argument() {
            let command = ct_app();

            let missing_args = vec![ctcore::ct_util_name()];
            let result = command.try_get_matches_from(missing_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_version() {
            let app = ct_app();
            assert_eq!(app.get_version(), Some(crate_version!()));
        }

        #[test]
        fn test_ct_app_args() {
            let app = ct_app();

            let mode_arg = app
                .get_arguments()
                .find(|a| a.get_id() == mkdir_flags::MODE);
            assert!(mode_arg.is_some());
            let mode_arg = mode_arg.unwrap();
            assert_eq!(mode_arg.get_short(), Some('m'));
            assert_eq!(mode_arg.get_long(), Some(mkdir_flags::MODE));

            let parents_arg = app
                .get_arguments()
                .find(|a| a.get_id() == mkdir_flags::PARENTS);
            assert!(parents_arg.is_some());
            let parents_arg = parents_arg.unwrap();
            assert_eq!(parents_arg.get_short(), Some('p'));
            assert_eq!(parents_arg.get_long(), Some(mkdir_flags::PARENTS));

            let verbose_arg = app
                .get_arguments()
                .find(|a| a.get_id() == mkdir_flags::VERBOSE);
            assert!(verbose_arg.is_some());
            let verbose_arg = verbose_arg.unwrap();
            assert_eq!(verbose_arg.get_short(), Some('v'));
            assert_eq!(verbose_arg.get_long(), Some(mkdir_flags::VERBOSE));

            let dirs_arg = app
                .get_arguments()
                .find(|a| a.get_id() == mkdir_flags::DIRS);
            assert!(dirs_arg.is_some());
            let dirs_arg = dirs_arg.unwrap();
            assert_eq!(dirs_arg.get_help(), None);
            assert_eq!(dirs_arg.get_value_hint(), clap::ValueHint::DirPath);
        }

        #[test]
        fn test_ct_app_args_parsing() {
            let app = ct_app();

            let matches = app.try_get_matches_from(vec![
                ctcore::ct_util_name(),
                "-m",
                "0755",
                "-p",
                "-v",
                "dir1",
                "dir2",
            ]);
            assert!(matches.is_ok());
            let matches = matches.unwrap();

            assert_eq!(
                matches.get_one::<String>(mkdir_flags::MODE),
                Some(&"0755".to_string())
            );
            assert!(matches.get_flag(mkdir_flags::PARENTS));
            assert!(matches.get_flag(mkdir_flags::VERBOSE));
            let dirs: Vec<&OsString> = matches
                .get_many::<OsString>(mkdir_flags::DIRS)
                .unwrap()
                .collect();
            assert_eq!(dirs, vec![&OsString::from("dir1"), &OsString::from("dir2")]);
        }

        #[test]
        fn test_ct_app_mode_long_parsing() {
            let app = ct_app();
            let matches = app.try_get_matches_from(vec![
                ctcore::ct_util_name(),
                "--mode",
                "u+rwx,go-w",
                "dir",
            ]);
            assert!(matches.is_ok());

            let matches = matches.unwrap();
            assert_eq!(
                matches.get_one::<String>(mkdir_flags::MODE),
                Some(&"u+rwx,go-w".to_string())
            );
        }

        #[test]
        fn test_ct_app_invalid_mode_parsing() {
            let app = ct_app();
            let result =
                app.try_get_matches_from(vec![ctcore::ct_util_name(), "-m", "invalidmode", "dir"]);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_dirs_parsing() {
            let app = ct_app();
            let matches =
                app.try_get_matches_from(vec![ctcore::ct_util_name(), "dir1", "dir2", "dir3"]);
            assert!(matches.is_ok());
            let matches = matches.unwrap();

            let dirs: Vec<&OsString> = matches
                .get_many::<OsString>(mkdir_flags::DIRS)
                .unwrap()
                .collect();
            assert_eq!(
                dirs,
                vec![
                    &OsString::from("dir1"),
                    &OsString::from("dir2"),
                    &OsString::from("dir3")
                ]
            );
        }

        #[test]
        fn test_ct_app_no_dirs() {
            let app = ct_app();
            let result = app.try_get_matches_from(vec![ctcore::ct_util_name()]);
            assert!(result.is_ok());
            let matches = result.unwrap();

            let dirs: Vec<&OsString> = matches
                .get_many::<OsString>(mkdir_flags::DIRS)
                .unwrap_or_default()
                .collect();
            assert!(dirs.is_empty());
        }
        #[test]
        fn test_ct_app_numeric_mode() {
            let app = ct_app();
            let matches =
                app.try_get_matches_from(vec![ctcore::ct_util_name(), "-m", "0755", "dir"]);
            assert!(matches.is_ok());
            let matches = matches.unwrap();

            assert_eq!(
                matches.get_one::<String>(mkdir_flags::MODE),
                Some(&"0755".to_string())
            );
        }

        #[test]
        fn test_ct_app_symbolic_mode() {
            let app = ct_app();
            let matches =
                app.try_get_matches_from(vec![ctcore::ct_util_name(), "-m", "u+rwx,go-w", "dir"]);
            assert!(matches.is_ok());
            let matches = matches.unwrap();

            assert_eq!(
                matches.get_one::<String>(mkdir_flags::MODE),
                Some(&"u+rwx,go-w".to_string())
            );
        }

        #[test]
        fn test_ct_app_combined_mode() {
            let app = ct_app();
            let matches =
                app.try_get_matches_from(vec![ctcore::ct_util_name(), "-m", "0755,u+s", "dir"]);
            assert!(matches.is_ok());
            let matches = matches.unwrap();

            assert_eq!(
                matches.get_one::<String>(mkdir_flags::MODE),
                Some(&"0755,u+s".to_string())
            );
        }

        #[test]
        fn test_ct_app_empty_mode() {
            let app = ct_app();
            let result = app.try_get_matches_from(vec![ctcore::ct_util_name(), "-m", "", "dir"]);
            assert!(result.is_ok());
            let matches = result.unwrap();

            assert_eq!(
                matches.get_one::<String>(mkdir_flags::MODE),
                Some(&"".to_string())
            );
        }

        #[test]
        fn test_ct_app_invalid_mode() {
            let app = ct_app();
            let matches =
                app.try_get_matches_from(vec![ctcore::ct_util_name(), "-m", "invalidmode", "dir"]);
            assert!(matches.is_ok());
            let matches = matches.unwrap();

            assert_eq!(
                matches.get_one::<String>(mkdir_flags::MODE),
                Some(&"invalidmode".to_string())
            );
        }

        #[test]
        fn test_ct_app_no_mode_specified() {
            let app = ct_app();
            let matches = app.try_get_matches_from(vec![ctcore::ct_util_name(), "dir"]);
            assert!(matches.is_ok());
            let matches = matches.unwrap();

            assert_eq!(matches.get_one::<String>(mkdir_flags::MODE), None);
        }

        #[test]
        fn test_ct_app_symbolic_mode_no_users() {
            let app = ct_app();
            let matches =
                app.try_get_matches_from(vec![ctcore::ct_util_name(), "-m", "+rwx", "dir"]);
            assert!(matches.is_ok());
            let matches = matches.unwrap();

            assert_eq!(
                matches.get_one::<String>(mkdir_flags::MODE),
                Some(&"+rwx".to_string())
            );
        }

        #[test]
        fn test_ct_app_symbolic_mode_only_permission() {
            let app = ct_app();
            let matches =
                app.try_get_matches_from(vec![ctcore::ct_util_name(), "-m", "a+r", "dir"]);
            assert!(matches.is_ok());
            let matches = matches.unwrap();

            assert_eq!(
                matches.get_one::<String>(mkdir_flags::MODE),
                Some(&"a+r".to_string())
            );
        }

        #[test]
        fn test_ct_app_numeric_mode_with_leading_zero() {
            let app = ct_app();
            let matches =
                app.try_get_matches_from(vec![ctcore::ct_util_name(), "-m", "0777", "dir"]);
            assert!(matches.is_ok());
            let matches = matches.unwrap();

            assert_eq!(
                matches.get_one::<String>(mkdir_flags::MODE),
                Some(&"0777".to_string())
            );
        }
    }
}
