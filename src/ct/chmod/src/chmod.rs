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

use clap::{crate_version, Arg, ArgAction, Command};
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{set_ct_exit_code, CTResult, CTsageError, CtSimpleError, ExitCode};
use ctcore::ct_fs::display_permissions_unix;
#[cfg(not(windows))]
use ctcore::ct_mode;
use ctcore::libc::mode_t;
use ctcore::{
    ct_format_usage, ct_help_about, ct_help_section, ct_help_usage, ct_show, ct_show_error,
};
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

const CHMOD_ABOUT: &str = ct_help_about!("chmod.md");
const CHMOD_USAGE: &str = ct_help_usage!("chmod.md");
const CHMOD_LONG_USAGE: &str = ct_help_section!("after help", "chmod.md");

mod chmod_flags {
    pub const CHANGES: &str = "changes";
    pub const QUIET: &str = "quiet"; // 可见别名("silent")
    pub const VERBOSE: &str = "verbose";
    pub const NO_PRESERVE_ROOT: &str = "no-preserve-root";
    pub const PRESERVE_ROOT: &str = "preserve-root";
    pub const REFERENCE: &str = "RFILE";
    pub const RECURSIVE: &str = "recursive";
    pub const MODE: &str = "MODE";
    pub const FILE: &str = "FILE";
}

/// Extract negative modes (starting with '-') from the rest of the arguments.
///
/// This is mainly required for GNU compatibility, where "non-positional negative" modes are used
/// as the actual positional MODE. Some examples of these cases are:
/// * "chmod -w -r file", which is the same as "chmod -w,-r file"
/// * "chmod -w file -r", which is the same as "chmod -w,-r file"
///
/// These can currently not be handled by clap.
/// Therefore it might be possible that a pseudo MODE is inserted to pass clap parsing.
/// The pseudo MODE is later replaced by the extracted (and joined) negative modes.
fn extract_negative_modes(mut extr_args: impl ctcore::Args) -> (Option<String>, Vec<OsString>) {
    // 我们查找参数直到找到“--”
    // “-mode”将被提取到parsed_cmode_vec中
    let (parsed_chmod_vec, pre_double_hyphen_args): (Vec<OsString>, Vec<OsString>) = extr_args
        .by_ref()
        .take_while(|a| a != "--")
        .partition(|arg| {
            let arg = if let Some(arg) = arg.to_str() {
                arg.to_string()
            } else {
                return false;
            };
            arg.len() >= 2
                && arg.starts_with('-')
                && matches!(
                    arg.chars().nth(1).unwrap(),
                    'r' | 'w' | 'x' | 'X' | 's' | 't' | 'u' | 'g' | 'o' | '0'..='7'
                )
        });

    let mut clean_chmod_args = Vec::new();
    if !parsed_chmod_vec.is_empty() {
        // 我们需要为clap提供一个伪cmode，后续不会使用它。
        // 这是因为clap需要遵循默认的“chmod MODE FILE”模式。
        clean_chmod_args.push("w".into());
    }
    clean_chmod_args.extend(pre_double_hyphen_args);

    if let Some(arg) = extr_args.next() {
        // 由于迭代器中仍有剩余项，我们先前已消费了“--”
        // -> 将其再次添加到args中
        clean_chmod_args.push("--".into());
        clean_chmod_args.push(arg);
    }
    clean_chmod_args.extend(extr_args);

    let parsed_chmod = Some(
        parsed_chmod_vec
            .iter()
            .map(|s| s.to_str().unwrap())
            .collect::<Vec<&str>>()
            .join(","),
    )
    .filter(|s| !s.is_empty());
    (parsed_chmod, clean_chmod_args)
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    let (parsed_cmode, args) = extract_negative_modes(args.skip(1)); // 跳过二进制名称

    let args_match = ct_app()
        .after_help(CHMOD_LONG_USAGE)
        .try_get_matches_from(args)?;

    let chmod_flag_changes = args_match.get_flag(chmod_flags::CHANGES);
    let chmod_flags_quite = args_match.get_flag(chmod_flags::QUIET);
    let chmod_flags_verbose = args_match.get_flag(chmod_flags::VERBOSE);
    let chmod_flags_preserve_root = args_match.get_flag(chmod_flags::PRESERVE_ROOT);
    let chmod_flags_recursive = args_match.get_flag(chmod_flags::RECURSIVE);
    let mode_reference = match args_match.get_one::<String>(chmod_flags::REFERENCE) {
        Some(reference) => match fs::metadata(reference) {
            Ok(metra) => Some(metra.mode() & 0o7777),
            Err(e) => {
                return Err(CtSimpleError::new(
                    1,
                    format!("cannot stat attributes of {}: {}", reference.quote(), e),
                ))
            }
        },
        None => None,
    };

    let chmod_flags_modes = args_match.get_one::<String>(chmod_flags::MODE);
    let chmod = if let Some(parsed_cmode) = parsed_cmode {
        parsed_cmode
    } else {
        chmod_flags_modes.unwrap().to_string() // modes 是必需的
    };
    // FIXME: 支持非UTF-8路径
    let mut f: Vec<String> = args_match
        .get_many::<String>(chmod_flags::FILE)
        .map(|v| v.map(ToString::to_string).collect())
        .unwrap_or_default();
    let cmode = if mode_reference.is_some() {
        // "--reference" 和 MODE 互斥
        // 若使用了 "--reference"，则需要将 MODE 解释为另一个 FILE
        // 直接用 clap 实现这种行为不可行
        f.push(chmod);
        None
    } else {
        Some(chmod)
    };

    if f.is_empty() {
        return Err(CTsageError::new(1, "missing operand".to_string()));
    }

    let chmoder_info = Chmoder {
        changes: chmod_flag_changes,
        quiet: chmod_flags_quite,
        verbose: chmod_flags_verbose,
        preserve_root: chmod_flags_preserve_root,
        recursive: chmod_flags_recursive,
        fmode: mode_reference,
        cmode,
    };

    chmoder_info.chmod(&f)
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = CHMOD_ABOUT;
    let usage_description = ct_format_usage(CHMOD_USAGE);

    let args = chmod_args_init();

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .args_override_self(true)
        .infer_long_args(true)
        .no_binary_name(true)
        .args(&args)
}

fn chmod_args_init() -> Vec<Arg> {
    let args = vec![
        Arg::new(chmod_flags::CHANGES)
            .long(chmod_flags::CHANGES)
            .short('c')
            .help("like verbose but report only when a change is made")
            .action(ArgAction::SetTrue),
        Arg::new(chmod_flags::QUIET)
            .long(chmod_flags::QUIET)
            .visible_alias("silent")
            .short('f')
            .help("suppress most error messages")
            .action(ArgAction::SetTrue),
        Arg::new(chmod_flags::VERBOSE)
            .long(chmod_flags::VERBOSE)
            .short('v')
            .help("output a diagnostic for every file processed")
            .action(ArgAction::SetTrue),
        Arg::new(chmod_flags::NO_PRESERVE_ROOT)
            .long(chmod_flags::NO_PRESERVE_ROOT)
            .help("do not treat '/' specially (the default)")
            .action(ArgAction::SetTrue),
        Arg::new(chmod_flags::PRESERVE_ROOT)
            .long(chmod_flags::PRESERVE_ROOT)
            .help("fail to operate recursively on '/'")
            .action(ArgAction::SetTrue),
        Arg::new(chmod_flags::RECURSIVE)
            .long(chmod_flags::RECURSIVE)
            .short('R')
            .help("change files and directories recursively")
            .action(ArgAction::SetTrue),
        Arg::new(chmod_flags::REFERENCE)
            .long("reference")
            .value_hint(clap::ValueHint::FilePath)
            .help("use RFILE's mode instead of MODE values"),
        Arg::new(chmod_flags::MODE).required_unless_present(chmod_flags::REFERENCE),
        Arg::new(chmod_flags::FILE)
            .required_unless_present(chmod_flags::MODE)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::AnyPath),
    ];
    args
}

struct Chmoder {
    changes: bool,
    quiet: bool,
    verbose: bool,
    preserve_root: bool,
    recursive: bool,
    fmode: Option<u32>,
    cmode: Option<String>,
}

impl Chmoder {
    fn chmod(&self, files: &[String]) -> CTResult<()> {
        let mut r = Ok(());

        for name in files {
            let file_name = &name[..];
            let file = Path::new(file_name);
            if !file.exists() {
                if file.is_symlink() {
                    if !self.quiet {
                        ct_show!(CtSimpleError::new(
                            1,
                            format!("cannot operate on dangling symlink {}", file_name.quote()),
                        ));
                    }
                    if self.verbose {
                        println!(
                            "failed to change mode of {} from 0000 (---------) to 1500 (r-x-----T)",
                            file_name.quote()
                        );
                    }
                } else if !self.quiet {
                    ct_show!(CtSimpleError::new(
                        1,
                        format!(
                            "cannot access {}: No such file or directory",
                            file_name.quote()
                        )
                    ));
                }
                // 即使传递了 -q 或 --quiet，GNU 仍以退出码 1 退出
                // 因此我们设置退出码，因为在 `self.quiet` 为真时它尚未被设置
                set_ct_exit_code(1);
                continue;
            }
            if self.recursive && self.preserve_root && file_name == "/" {
                return Err(CtSimpleError::new(
                    1,
                    format!(
                        "it is dangerous to operate recursively on {}\nchmod: use --no-preserve-root to override this failsafe",
                        file_name.quote()
                    )
                ));
            }
            match self.recursive {
                true => {
                    r = self.chmod_walk_dir(file);
                }
                false => {
                    r = self.chmod_file(file).and(r);
                }
            }
        }
        r
    }

    fn chmod_walk_dir(&self, path: &Path) -> CTResult<()> {
        let mut r = self.chmod_file(path);
        if !path.is_symlink() && path.is_dir() {
            for dir_entry in path.read_dir()? {
                let path = dir_entry?.path();
                if !path.is_symlink() {
                    r = self.chmod_walk_dir(path.as_path());
                }
            }
        }
        r
    }

    #[cfg(unix)]
    fn chmod_file(&self, file_path: &Path) -> CTResult<()> {
        use ctcore::ct_mode::get_umask;

        let file_perms = match fs::metadata(file_path) {
            Ok(meta) => meta.mode() & 0o7777,
            Err(err) => {
                if file_path.is_symlink() {
                    if self.verbose {
                        println!(
                            "neither symbolic link {} nor referent has been changed",
                            file_path.quote()
                        );
                    }
                    return Ok(());
                } else if err.kind() == std::io::ErrorKind::PermissionDenied {
                    // 这两个文件名通常会被条件性地加上引号，
                    // 但 GNU 的测试期望它们总是被加上引号
                    return Err(CtSimpleError::new(
                        1,
                        format!("{}: Permission denied", file_path.quote()),
                    ));
                } else {
                    return Err(CtSimpleError::new(
                        1,
                        format!("{}: {}", file_path.quote(), err),
                    ));
                }
            }
        };
        match self.fmode {
            Some(mode) => self.change_file(file_perms, mode, file_path)?,
            None => {
                let chmod_unwrapped = self.cmode.clone().unwrap();
                let mut new_mode = file_perms;
                let mut naively_expected_new_mode = new_mode;
                for mode in chmod_unwrapped.split(',') {
                    let result = if mode.chars().any(|c| c.is_ascii_digit()) {
                        ct_mode::parse_numeric(new_mode, mode, file_path.is_dir()).map(|v| (v, v))
                    } else {
                        ct_mode::parse_symbolic(new_mode, mode, get_umask(), file_path.is_dir())
                            .map(|m| {
                                // 假设umask为0来计算新的模式
                                let naive_mode = ct_mode::parse_symbolic(
                                    naively_expected_new_mode,
                                    mode,
                                    0,
                                    file_path.is_dir(),
                                )
                                .unwrap(); // 我们知道mode必须是有效的，因此这不可能失败
                                (m, naive_mode)
                            })
                    };

                    match result {
                        Ok((mode_value, naive_mode_value)) => {
                            new_mode = mode_value;
                            naively_expected_new_mode = naive_mode_value;
                        }
                        Err(f) => {
                            return if self.quiet {
                                Err(ExitCode::new(1))
                            } else {
                                Err(CtSimpleError::new(1, f))
                            };
                        }
                    }
                }

                self.change_file(file_perms, new_mode, file_path)?;
                // 如果在umask为0的情况下某个权限本应被移除，但由于umask不是0而实际上并未移除，则打印错误并失败
                if (new_mode & !naively_expected_new_mode) != 0 {
                    return Err(CtSimpleError::new(
                        1,
                        format!(
                            "{}: new permissions are {}, not {}",
                            file_path.maybe_quote(),
                            display_permissions_unix(new_mode as mode_t, false),
                            display_permissions_unix(naively_expected_new_mode as mode_t, false)
                        ),
                    ));
                }
            }
        }

        Ok(())
    }

    #[cfg(unix)]
    fn change_file(&self, file_perms: u32, mode: u32, file_path: &Path) -> Result<(), i32> {
        if file_perms == mode {
            if self.verbose && !self.changes {
                println!(
                    "mode of {} retained as {:04o} ({})",
                    file_path.quote(),
                    file_perms,
                    display_permissions_unix(file_perms as mode_t, false),
                );
            }
            Ok(())
        } else if let Err(err) = fs::set_permissions(file_path, fs::Permissions::from_mode(mode)) {
            if !self.quiet {
                ct_show_error!("{}", err);
            }
            if self.verbose {
                println!(
                    "failed to change mode of file {} from {:04o} ({}) to {:04o} ({})",
                    file_path.quote(),
                    file_perms,
                    display_permissions_unix(file_perms as mode_t, false),
                    mode,
                    display_permissions_unix(mode as mode_t, false)
                );
            }
            Err(1)
        } else {
            if self.verbose || self.changes {
                println!(
                    "mode of {} changed from {:04o} ({}) to {:04o} ({})",
                    file_path.quote(),
                    file_perms,
                    display_permissions_unix(file_perms as mode_t, false),
                    mode,
                    display_permissions_unix(mode as mode_t, false)
                );
            }
            Ok(())
        }
    }
}

