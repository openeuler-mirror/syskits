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
//!Implement GNU-style backup functionality.
//!
//! This module implements the backup functionality as described in the [GNU
//! manual][1]. It provides
//!
//! - pre-defined [`clap`-Arguments][2] for inclusion in utilities that
//!   implement backups
//! - determination of the [backup mode][3]
//! - determination of the [backup suffix][4]
//! - [backup target path construction][5]
//! - [Error types][6] for backup-related errors
//! - GNU-compliant [help texts][7] for backup-related errors
//!
//! Backup-functionality is implemented by the following utilities:
//!
//! - `cp`
//! - `install`
//! - `ln`
//! - `mv`
//!
//!
//! [1]: https://www.gnu.org/software/coreutils/manual/html_node/Backup-options.html
//! [2]: arguments
//! [3]: `determine_backup_mode()`
//! [4]: `determine_backup_suffix()`
//! [5]: `get_backup_path()`
//! [6]: `CtBackupError`
//! [7]: `CT_BACKUP_CONTROL_LONG_HELP`
//!
//!
//! # Usage example
//!
//!```
//! #[macro_use]
//! extern crate ctcore;
//!
//! use clap::{Command, Arg, ArgMatches};
//! use std::path::{Path, PathBuf};
//! use ctcore::ct_backup_control::{self, CtBackupMode};
//! use ctcore::ct_error::{CTError, CTResult};
//!
//! fn main() {
//!     let usage = String::from("command [OPTION]... ARG");
//!     let long_usage = String::from("And here's a detailed explanation");
//!
//!     let matches = Command::new("command")
//!         .arg(ct_backup_control::arguments::backup())
//!         .arg(ct_backup_control::arguments::backup_no_args())
//!         .arg(ct_backup_control::arguments::suffix())
//!         .override_usage(usage)
//!         .after_help(format!(
//!             "{}\n{}",
//!             long_usage,
//!             ct_backup_control::CT_BACKUP_CONTROL_LONG_HELP
//!         ))
//!         .get_matches_from(vec![
//!             "command", "--backup=t", "--suffix=bak~"
//!         ]);
//!
//!     let backup_mode = match ct_backup_control::determine_backup_mode(&matches) {
//!         Err(e) => {
//!             ct_show!(e);
//!             return;
//!         }
//!         Ok(mode) => mode,
//!     };
//!     let backup_suffix = ct_backup_control::determine_backup_suffix(&matches);
//!     let target_path = Path::new("/tmp/example");
//!
//!     let backup_path = ct_backup_control::get_backup_path(
//!         backup_mode, target_path, &backup_suffix
//!     );
//!
//!     // 在此处执行备份。
//!
//! }
//! ```

use crate::{
    ct_display::Quotable,
    ct_error::{CTError, CTResult},
};
use clap::ArgMatches;
use std::{
    env,
    error::Error,
    fmt::{Debug, Display},
    path::{Path, PathBuf},
};

pub static CT_BACKUP_CONTROL_VALUES: &[&str] = &[
    "simple", "never", "numbered", "t", "existing", "nil", "none", "off",
];

pub const CT_BACKUP_CONTROL_LONG_HELP: &str =
    "The backup suffix is '~', unless set with --suffix or SIMPLE_BACKUP_SUFFIX.
The version control method may be selected via the --backup option or through
the VERSION_CONTROL environment variable.  Here are the values:

  none, off       never make backups (even if --backup is given)
  numbered, t     make numbered backups
  existing, nil   numbered if numbered backups exist, simple otherwise
  simple, never   always make simple backups";

static CT_VALID_ARGS_HELP: &str = "Valid arguments are:
  - 'none', 'off'
  - 'simple', 'never'
  - 'existing', 'nil'
  - 'numbered', 't'";

/// Available backup modes.
///
/// The mapping of the backup modes to the CLI arguments is annotated on the
/// enum variants.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum CtBackupMode {
    /// Argument 'none', 'off'
    NoBackup,
    /// Argument 'simple', 'never'
    SimpleBackup,
    /// Argument 'numbered', 't'
    NumberedBackup,
    /// Argument 'existing', 'nil'
    ExistingBackup,
}

/// Backup error types.
///
/// Errors are currently raised by [`determine_backup_mode`] only. All errors
/// are implemented as [`CTError`] for uniform handling across utilities.
#[derive(Debug, Eq, PartialEq)]
pub enum CtBackupError {
    /// An invalid argument (e.g. 'foo') was given as backup type. First
    /// parameter is the argument, second is the arguments origin (CLI or
    /// ENV-var)
    InvalidArgument(String, String),
    /// An ambiguous argument (e.g. 'n') was given as backup type. First
    /// parameter is the argument, second is the arguments origin (CLI or
    /// ENV-var)
    AmbiguousArgument(String, String),
    /// Currently unused
    BackupImpossible(),
}

impl CTError for CtBackupError {
    fn code(&self) -> i32 {
        match self {
            Self::BackupImpossible() => 2,
            _ => 1,
        }
    }

    fn usage(&self) -> bool {
        matches!(
            self,
            Self::InvalidArgument(_, _) | Self::AmbiguousArgument(_, _)
        )
    }
}

impl Error for CtBackupError {}

impl Display for CtBackupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidArgument(arg, origin) => write!(
                f,
                "invalid argument {} for '{}'\n{}",
                arg.quote(),
                origin,
                CT_VALID_ARGS_HELP
            ),
            Self::AmbiguousArgument(arg, origin) => write!(
                f,
                "ambiguous argument {} for '{}'\n{}",
                arg.quote(),
                origin,
                CT_VALID_ARGS_HELP
            ),
            Self::BackupImpossible() => write!(f, "cannot create backup"),
        }
    }
}

/// Arguments for backup-related functionality.
///
/// Rather than implementing the `clap`-Arguments for every utility, it is
/// recommended to include the `clap` arguments via the functions provided here.
/// This way the backup-specific arguments are handled uniformly across
/// utilities and can be maintained in one central place.
pub mod arguments {
    use clap::ArgAction;

    pub static OPT_BACKUP: &str = "backupopt_backup";
    pub static OPT_BACKUP_NO_ARG: &str = "backupopt_b";
    pub static OPT_SUFFIX: &str = "backupopt_suffix";

    /// '--backup' 字段
    pub fn backup() -> clap::Arg {
        clap::Arg::new(OPT_BACKUP)
            .long("backup")
            .help("make a backup of each existing destination file")
            .action(clap::ArgAction::Set)
            .require_equals(true)
            .num_args(0..=1)
            .value_name("CONTROL")
    }

    /// '-b' 字段
    pub fn backup_no_args() -> clap::Arg {
        clap::Arg::new(OPT_BACKUP_NO_ARG)
            .short('b')
            .help("like --backup but does not accept an argument")
            .action(ArgAction::SetTrue)
    }

    /// '-S, --suffix' 字段
    pub fn suffix() -> clap::Arg {
        clap::Arg::new(OPT_SUFFIX)
            .short('S')
            .long("suffix")
            .help("override the usual backup suffix")
            .action(clap::ArgAction::Set)
            .value_name("SUFFIX")
            .allow_hyphen_values(true)
    }
}

/// Obtain the suffix to use for a backup.
///
/// In order of precedence, this function obtains the backup suffix
///
/// 1. From the '-S' or '--suffix' CLI argument, if present
/// 2. From the "SIMPLE_BACKUP_SUFFIX" environment variable, if present
/// 3. By using the default '~' if none of the others apply
///
/// This function directly takes [`clap::ArgMatches`] as argument and looks for
/// the '-S' and '--suffix' arguments itself.
pub fn determine_backup_suffix(matches: &ArgMatches) -> String {
    let supplied_suffix = matches.get_one::<String>(arguments::OPT_SUFFIX);
    if let Some(suffix) = supplied_suffix {
        String::from(suffix)
    } else {
        env::var("SIMPLE_BACKUP_SUFFIX").unwrap_or_else(|_| "~".to_owned())
    }
}

/// Determine the "mode" for the backup operation to perform, if any.
///
/// Parses the backup options according to the [GNU manual][1], and converts
/// them to an instance of `BackupMode` for further processing.
///
/// Takes [`clap::ArgMatches`] as argument which **must** contain the options
/// from [`arguments::backup()`] and [`arguments::backup_no_args()`]. Otherwise
/// the `NoBackup` mode is returned unconditionally.
///
/// It is recommended for anyone who would like to implement the
/// backup-functionality to use the arguments prepared in the `arguments`
/// submodule (see examples)
///
/// [1]: https://www.gnu.org/software/coreutils/manual/html_node/Backup-options.html
///
///
/// # Errors
///
/// If an argument supplied directly to the long `backup` option, or read in
/// through the `VERSION CONTROL` env var is ambiguous (i.e. may resolve to
/// multiple backup modes) or invalid, an [`InvalidArgument`][10] or
/// [`AmbiguousArgument`][11] error is returned, respectively.
///
/// [10]: CtBackupError::InvalidArgument
/// [11]: CtBackupError::AmbiguousArgument
///
///
/// # Examples
///
/// Here's how one would integrate the backup mode determination into an
/// application.
///
/// ```
/// #[macro_use]
/// extern crate ctcore;
/// use ctcore::ct_backup_control::{self, CtBackupMode};
/// use clap::{Command, Arg, ArgMatches};
///
/// fn main() {
///     let matches = Command::new("command")
///         .arg(ct_backup_control::arguments::backup())
///         .arg(ct_backup_control::arguments::backup_no_args())
///         .get_matches_from(vec![
///             "command", "-b", "--backup=t"
///         ]);
///
///     let backup_mode = ct_backup_control::determine_backup_mode(&matches).unwrap();
///     assert_eq!(backup_mode, CtBackupMode::NumberedBackup)
/// }
/// ```
///
/// This example shows an ambiguous input, as 'n' may resolve to 4 different
/// backup modes.
///
///
/// ```
/// #[macro_use]
/// extern crate ctcore;
/// use ctcore::ct_backup_control::{self, CtBackupMode, CtBackupError};
/// use clap::{Command, Arg, ArgMatches};
///
/// fn main() {
///     let matches = Command::new("command")
///         .arg(ct_backup_control::arguments::backup())
///         .arg(ct_backup_control::arguments::backup_no_args())
///         .get_matches_from(vec![
///             "command", "-b", "--backup=n"
///         ]);
///
///     let backup_mode = ct_backup_control::determine_backup_mode(&matches);
///
///     assert!(backup_mode.is_err());
///     let err = backup_mode.unwrap_err();
///
///     // 使用ctcore功能向用户显示错误
///     ct_show!(err);
/// }
/// ```
pub fn determine_backup_mode(matches: &ArgMatches) -> CTResult<CtBackupMode> {
    if matches.contains_id(arguments::OPT_BACKUP) {
        // 使用方法来确定要创建的备份类型。
        // 当使用此选项但未指定方法时，将使用 VERSION_CONTROL 环境变量的值。
        // 如果 VERSION_CONTROL 未设置，则默认备份类型为 'existing'。
        if let Some(method) = matches.get_one::<String>(arguments::OPT_BACKUP) {
            // 第二个参数用于返回的错误字符串。
            match_method(method, "backup type")
        } else if let Ok(method) = env::var("VERSION_CONTROL") {
            // 第二个参数是用于返回的错误字符串
            match_method(&method, "$VERSION_CONTROL")
        } else {
            // 如果未向 --backup 提供参数时的默认值
            Ok(CtBackupMode::ExistingBackup)
        }
    } else if matches.get_flag(arguments::OPT_BACKUP_NO_ARG) {
        // 该选项的短形式 -b 不接受任何参数。
        // 使用 -b 相当于使用 --backup=existing。
        Ok(CtBackupMode::ExistingBackup)
    } else {
        // 完全没有出现任何选项
        Ok(CtBackupMode::NoBackup)
    }
}

/// Match a backup option string to a `BackupMode`.
///
/// The GNU manual specifies that abbreviations to options are valid as long as
/// they aren't ambiguous. This function matches the given `method` argument
/// against all valid backup options (via `starts_with`), and returns a valid
/// [`CtBackupMode`] if exactly one backup option matches the `method` given.
///
/// `origin` is required in order to format the generated error message
/// properly, when an error occurs.
///
///
/// # Errors
///
/// If `method` is invalid or ambiguous (i.e. may resolve to multiple backup
/// modes), an [`InvalidArgument`][10] or [`AmbiguousArgument`][11] error is
/// returned, respectively.
///
/// [10]: CtBackupError::InvalidArgument
/// [11]: CtBackupError::AmbiguousArgument
fn match_method(method: &str, origin: &str) -> CTResult<CtBackupMode> {
    let matches: Vec<&&str> = CT_BACKUP_CONTROL_VALUES
        .iter()
        .filter(|val| val.starts_with(method))
        .collect();
    if matches.len() == 1 {
        match *matches[0] {
            "simple" | "never" => Ok(CtBackupMode::SimpleBackup),
            "numbered" | "t" => Ok(CtBackupMode::NumberedBackup),
            "existing" | "nil" => Ok(CtBackupMode::ExistingBackup),
            "none" | "off" => Ok(CtBackupMode::NoBackup),
            _ => unreachable!(), // 由于上面的列表必须恰好有一个匹配项，所以这种情况不会发生。
        }
    } else if matches.is_empty() {
        Err(CtBackupError::InvalidArgument(method.to_string(), origin.to_string()).into())
    } else {
        Err(CtBackupError::AmbiguousArgument(method.to_string(), origin.to_string()).into())
    }
}

pub fn get_backup_path(
    backup_mode: CtBackupMode,
    backup_path: &Path,
    suffix: &str,
) -> Option<PathBuf> {
    match backup_mode {
        CtBackupMode::NoBackup => None,
        CtBackupMode::SimpleBackup => Some(simple_backup_path(backup_path, suffix)),
        CtBackupMode::NumberedBackup => Some(numbered_backup_path(backup_path)),
        CtBackupMode::ExistingBackup => Some(existing_backup_path(backup_path, suffix)),
    }
}

fn simple_backup_path(path: &Path, suffix: &str) -> PathBuf {
    let mut p = path.to_string_lossy().into_owned();
    p.push_str(suffix);
    PathBuf::from(p)
}

fn numbered_backup_path(path: &Path) -> PathBuf {
    for i in 1_u64.. {
        let path_str = &format!("{}.~{}~", path.to_string_lossy(), i);
        let path = Path::new(path_str);
        if !path.exists() {
            return path.to_path_buf();
        }
    }
    panic!("cannot create backup")
}

fn existing_backup_path(path: &Path, suffix: &str) -> PathBuf {
    let test_path_str = &format!("{}.~1~", path.to_string_lossy());
    let test_path = Path::new(test_path_str);
    if test_path.exists() {
        numbered_backup_path(path)
    } else {
        simple_backup_path(path, suffix)
    }
}

/// Returns true if the source file is likely to be the simple backup file for the target file.
///
/// # Arguments
///
/// * `source` - A Path reference that holds the source (backup) file path.
/// * `target` - A Path reference that holds the target file path.
/// * `suffix` - Str that holds the backup suffix.
///
/// # Examples
///
/// ```
/// use std::path::Path;
/// use ctcore::ct_backup_control::source_is_target_backup;
/// let source = Path::new("data.txt~");
/// let target = Path::new("data.txt");
/// let suffix = String::from("~");
///
/// assert_eq!(source_is_target_backup(&source, &target, &suffix), true);
/// ```
///
pub fn source_is_target_backup(source: &Path, target: &Path, suffix: &str) -> bool {
    let source_filename = source.to_string_lossy();
    let target_backup_filename = format!("{}{suffix}", target.to_string_lossy());
    source_filename == target_backup_filename
}

//
// 本模块的测试
//
#[cfg(test)]
mod tests {
    use super::*;
    // Required to instantiate mutex in shared context
    use clap::Command;
    use once_cell::sync::Lazy;
    use std::fs;
    use std::io::Write;
    use std::sync::Mutex;

    // 在此处需要互斥锁，因为默认情况下所有测试都是作为同一个父进程下的单独线程运行的。
    // 由于环境变量是特定于进程的（因此在多个线程间共享），
    // 如果不采取预防措施，就一定会发生数据竞争。
    // 因此，我们让所有依赖于环境变量的测试都锁定这个空互斥锁，以确保它们不会并发访问它。
    static TEST_MUTEX: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    // “VERSION_CONTROL”的环境变量
    static ENV_VERSION_CONTROL: &str = "VERSION_CONTROL";

    fn make_app() -> clap::Command {
        Command::new("command")
            .arg(arguments::backup())
            .arg(arguments::backup_no_args())
            .arg(arguments::suffix())
    }

    // 默认为 --backup=existing
    #[test]
    fn test_backup_mode_short_only() {
        let _dummy = TEST_MUTEX.lock().unwrap();
        let matches = make_app().get_matches_from(vec!["command", "-b"]);

        let result = determine_backup_mode(&matches).unwrap();

        assert_eq!(result, CtBackupMode::ExistingBackup);
    }

    // --backup 选项优先于 -b 选项
    #[test]
    fn test_backup_mode_long_preferred_over_short() {
        let _dummy = TEST_MUTEX.lock().unwrap();
        let matches = make_app().get_matches_from(vec!["command", "-b", "--backup=none"]);

        let result = determine_backup_mode(&matches).unwrap();

        assert_eq!(result, CtBackupMode::NoBackup);
    }

    // --backup 选项可以不带参数传入
    #[test]
    fn test_backup_mode_long_without_args_no_env() {
        let _dummy = TEST_MUTEX.lock().unwrap();
        let matches = make_app().get_matches_from(vec!["command", "--backup"]);

        let result = determine_backup_mode(&matches).unwrap();

        assert_eq!(result, CtBackupMode::ExistingBackup);
    }

    // --backup只能带有参数一起使用
    #[test]
    fn test_backup_mode_long_with_args() {
        let _dummy = TEST_MUTEX.lock().unwrap();
        let matches = make_app().get_matches_from(vec!["command", "--backup=simple"]);

        let result = determine_backup_mode(&matches).unwrap();

        assert_eq!(result, CtBackupMode::SimpleBackup);
    }

    // --backup对于无效参数报错
    #[test]
    fn test_backup_mode_long_with_args_invalid() {
        let _dummy = TEST_MUTEX.lock().unwrap();
        let matches = make_app().get_matches_from(vec!["command", "--backup=foobar"]);

        let result = determine_backup_mode(&matches);

        assert!(result.is_err());
        let text = format!("{}", result.unwrap_err());
        assert!(text.contains("invalid argument 'foobar' for 'backup type'"));
    }

    // --backup 在遇到模糊参数时报错
    #[test]
    fn test_backup_mode_long_with_args_ambiguous() {
        let _dummy = TEST_MUTEX.lock().unwrap();
        let matches = make_app().get_matches_from(vec!["command", "--backup=n"]);

        let result = determine_backup_mode(&matches);

        assert!(result.is_err());
        let text = format!("{}", result.unwrap_err());
        assert!(text.contains("ambiguous argument 'n' for 'backup type'"));
    }

    // --backup接受缩写的参数（si表示simple）
    #[test]
    fn test_backup_mode_long_with_arg_shortened() {
        let _dummy = TEST_MUTEX.lock().unwrap();
        let matches = make_app().get_matches_from(vec!["command", "--backup=si"]);

        let result = determine_backup_mode(&matches).unwrap();

        assert_eq!(result, CtBackupMode::SimpleBackup);
    }

    // -b 选项忽略 “VERSION_CONTROL” 环境变量
    #[test]
    fn test_backup_mode_short_only_ignore_env() {
        let _dummy = TEST_MUTEX.lock().unwrap();
        unsafe { env::set_var(ENV_VERSION_CONTROL, "none") };
        let matches = make_app().get_matches_from(vec!["command", "-b"]);

        let result = determine_backup_mode(&matches).unwrap();

        assert_eq!(result, CtBackupMode::ExistingBackup);
        unsafe { env::remove_var(ENV_VERSION_CONTROL) };
    }

    // --backup可以不带参数传入，但如果存在则读取环境变量
    #[test]
    fn test_backup_mode_long_without_args_with_env() {
        let _dummy = TEST_MUTEX.lock().unwrap();
        unsafe { env::set_var(ENV_VERSION_CONTROL, "none") };
        let matches = make_app().get_matches_from(vec!["command", "--backup"]);

        let result = determine_backup_mode(&matches).unwrap();

        assert_eq!(result, CtBackupMode::NoBackup);
        unsafe { env::remove_var(ENV_VERSION_CONTROL) };
    }

    // --backup 在遇到无效的 VERSION_CONTROL 环境变量时报错
    #[test]
    fn test_backup_mode_long_with_env_var_invalid() {
        let _dummy = TEST_MUTEX.lock().unwrap();
        unsafe { env::set_var(ENV_VERSION_CONTROL, "foobar") };
        let matches = make_app().get_matches_from(vec!["command", "--backup"]);

        let result = determine_backup_mode(&matches);

        assert!(result.is_err());
        let text = format!("{}", result.unwrap_err());
        assert!(text.contains("invalid argument 'foobar' for '$VERSION_CONTROL'"));
        unsafe { env::remove_var(ENV_VERSION_CONTROL) };
    }

    // --backup对于模糊的VERSION_CONTROL环境变量报错
    #[test]
    fn test_backup_mode_long_with_env_var_ambiguous() {
        let _dummy = TEST_MUTEX.lock().unwrap();
        unsafe { env::set_var(ENV_VERSION_CONTROL, "n") };
        let matches = make_app().get_matches_from(vec!["command", "--backup"]);

        let result = determine_backup_mode(&matches);

        assert!(result.is_err());
        let text = format!("{}", result.unwrap_err());
        assert!(text.contains("ambiguous argument 'n' for '$VERSION_CONTROL'"));
        unsafe { env::remove_var(ENV_VERSION_CONTROL) };
    }

    // --backup 接受简写的环境变量（如 si 代表 simple）
    #[test]
    fn test_backup_mode_long_with_env_var_shortened() {
        let _dummy = TEST_MUTEX.lock().unwrap();
        unsafe { env::set_var(ENV_VERSION_CONTROL, "si") };
        let matches = make_app().get_matches_from(vec!["command", "--backup"]);

        let result = determine_backup_mode(&matches).unwrap();

        assert_eq!(result, CtBackupMode::SimpleBackup);
        unsafe { env::remove_var(ENV_VERSION_CONTROL) };
    }

    #[test]
    fn test_suffix_takes_hyphen_value() {
        let _dummy = TEST_MUTEX.lock().unwrap();
        let matches = make_app().get_matches_from(vec!["command", "-b", "--suffix", "-v"]);

        let result = determine_backup_suffix(&matches);
        assert_eq!(result, "-v");
    }
    #[test]
    fn test_source_is_target_backup() {
        let source = Path::new("data.txt.bak");
        let target = Path::new("data.txt");
        let suffix = String::from(".bak");

        assert!(source_is_target_backup(source, target, &suffix));
    }

    #[test]
    fn test_source_is_not_target_backup() {
        let source = Path::new("data.txt");
        let target = Path::new("backup.txt");
        let suffix = String::from(".bak");

        assert!(!source_is_target_backup(source, target, &suffix));
    }

    #[test]
    fn test_source_is_target_backup_with_tilde_suffix() {
        let source = Path::new("example~");
        let target = Path::new("example");
        let suffix = String::from("~");

        assert!(source_is_target_backup(source, target, &suffix));
    }

    #[test]
    fn test_invalid_argument_display() {
        let error = CtBackupError::InvalidArgument("arg".to_string(), "origin".to_string());
        let expected = "invalid argument 'arg' for 'origin'\nValid arguments are:\n  - 'none', 'off'\n  - 'simple', 'never'\n  - 'existing', 'nil'\n  - 'numbered', 't'";
        print!("{error}");
        print!("{expected}");
        assert_eq!(expected, format!("{error}"));
    }

    #[test]
    fn test_ambiguous_argument_display() {
        let error = CtBackupError::AmbiguousArgument("arg".to_string(), "origin".to_string());
        let expected = "ambiguous argument 'arg' for 'origin'\nValid arguments are:\n  - 'none', 'off'\n  - 'simple', 'never'\n  - 'existing', 'nil'\n  - 'numbered', 't'";
        assert_eq!(expected, format!("{error}"));
    }

    #[test]
    fn test_existing_backup_path_with_existing_path() {
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join("test_file.txt");
        let suffix = ".bak";

        // 创建一个用于测试的虚拟文件
        let _ = fs::File::create(&path).unwrap();

        // 使用给定的后缀创建备份文件
        let backup_path = existing_backup_path(&path, suffix);
        let except_path: PathBuf = temp_dir.join("test_file.txt.bak");

        assert_eq!(backup_path, except_path);

        // 清理虚拟文件和备份文件
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn test_existing_backup_path_with_non_existing_path() {
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join("test_file.txt");
        let suffix = ".bak";

        // 使用给定的后缀创建备份文件
        let backup_path = existing_backup_path(&path, suffix);

        // Check if the backup file does not exist
        assert!(!backup_path.exists());

        // Clean up the backup file (if it was created)
        //fs::remove_file(&backup_path).unwrap();
    }
    #[test]
    fn test_numbered_backup_path() {
        // Create a temporary directory for testing
        let temp_dir = tempfile::Builder::new()
            .prefix("backup_test")
            .tempdir()
            .unwrap();
        let temp_path = temp_dir.path();

        // Create a file inside the temporary directory
        let file_path = temp_path.join("test.txt");
        let mut file = fs::File::create(&file_path).unwrap();
        file.write_all(b"test content").unwrap();

        // Call the `numbered_backup_path` function and assert the result
        let backup_path = numbered_backup_path(&file_path);
        assert!(!backup_path.exists());
        assert!(backup_path.starts_with(temp_path));
        //assert!(backup_path.ends_with(".~1~"));

        // Clean up the temporary directory
        temp_dir.close().unwrap();
    }
    #[test]
    fn test_simple_backup_path() {
        let path = Path::new("/var/tmp/file.txt");
        let suffix = ".bak";
        let expected = PathBuf::from("/var/tmp/file.txt.bak");

        let result = simple_backup_path(path, suffix);

        assert_eq!(result, expected);
    }

    #[test]
    fn test_match_method_invalid_argument() {
        let result = match_method("invalid", "test");
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap().to_string(),
            "invalid argument 'invalid' for 'test'\nValid arguments are:\n  - 'none', 'off'\n  - 'simple', 'never'\n  - 'existing', 'nil'\n  - 'numbered', 't'"
        );
    }

    #[test]
    fn test_match_method_ambiguous_argument() {
        let result = match_method("ambig", "test");
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap().to_string(),
            "invalid argument 'ambig' for 'test'\nValid arguments are:\n  - 'none', 'off'\n  - 'simple', 'never'\n  - 'existing', 'nil'\n  - 'numbered', 't'"
        );
    }

    #[test]
    fn test_backup_mode_with_backup_option() {
        // Arrange
        let mut args = std::collections::HashMap::new();
        args.insert("backup", "incremental");
        let _matches = <ArgMatches as std::default::Default>::default();
    }
}
