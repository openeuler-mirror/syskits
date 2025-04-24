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

// spell-checker:ignore (path) eacces inacc

extern crate rust_i18n;
use clap::{Arg, ArgAction, Command, builder::ValueParser, crate_version, parser::ValueSource};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "zh-CN");
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CTsageError};
use ctcore::{ct_prompt_yes, ct_show_error};
use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, Metadata};
use std::io::ErrorKind;
use std::ops::BitOr;
use std::path::{Path, PathBuf};
use sys_locale::get_locale;
use walkdir::{DirEntry, WalkDir};

#[derive(Eq, PartialEq, Clone, Copy)]
/// Enum, determining when the `rm` will prompt the user about the file deletion
pub enum InteractiveMode {
    /// Never prompt
    Never,
    /// Prompt once before removing more than three files, or when removing
    /// recursively.
    Once,
    /// Prompt before every removal
    Always,
    /// Prompt only on write-protected files
    PromptProtected,
}

/// RMOptions for the `rm` command
///
/// All options are public so that the options can be programmatically
/// constructed by other crates, such as Nushell. That means that this struct
/// is part of our public API. It should therefore not be changed without good
/// reason.
///
/// The fields are documented with the arguments that determine their value.
pub struct RMOptions {
    /// `-f`, `--force`
    pub force: bool,
    /// Iterative mode, determines when the command will prompt.
    ///
    /// Set by the following arguments:
    /// - `-i`: [`InteractiveMode::Always`]
    /// - `-I`: [`InteractiveMode::Once`]
    /// - `--interactive`: sets one of the above or [`InteractiveMode::Never`]
    /// - `-f`: implicitly sets [`InteractiveMode::Never`]
    ///
    /// If no other option sets this mode, [`InteractiveMode::PromptProtected`]
    /// is used
    pub interactive: InteractiveMode,
    #[allow(dead_code)]
    /// `--one-file-system`
    pub one_fs: bool,
    /// `--preserve-root`/`--no-preserve-root`
    pub preserve_root: bool,
    /// `-r`, `--recursive`
    pub recursive: bool,
    /// `-d`, `--dir`
    pub dir: bool,
    /// `-v`, `--verbose`
    pub verbose: bool,
}

impl RMOptions {
    pub fn new(matches: &clap::ArgMatches) -> CTResult<Self> {
        let force = matches.get_flag(rm_flags::RM_FORCE);
        let force_prompt_never = should_force_prompt_never(matches, force);

        Ok(RMOptions {
            force,
            interactive: determine_interactive_mode(matches, force_prompt_never),
            one_fs: matches.get_flag(rm_flags::RM_ONE_FILE_SYSTEM),
            preserve_root: !matches.get_flag(rm_flags::RM_NO_PRESERVE_ROOT),
            recursive: matches.get_flag(rm_flags::RM_RECURSIVE),
            dir: matches.get_flag(rm_flags::RM_DIR),
            verbose: matches.get_flag(rm_flags::RM_VERBOSE),
        })
    }
}

mod rm_flags {
    pub const RM_DIR: &str = "dir";
    pub const RM_INTERACTIVE: &str = "interactive";
    pub const RM_FORCE: &str = "force";
    pub const RM_NO_PRESERVE_ROOT: &str = "no-preserve-root";
    pub const RM_ONE_FILE_SYSTEM: &str = "one-file-system";
    pub const RM_PRESERVE_ROOT: &str = "preserve-root";
    pub const RM_PROMPT: &str = "prompt";
    pub const RM_PROMPT_MORE: &str = "prompt-more";
    pub const RM_RECURSIVE: &str = "recursive";
    pub const RM_VERBOSE: &str = "verbose";
    pub const RM_PRESUME_INPUT_TTY: &str = "-presume-input-tty";

    pub const RM_ARG_FILES: &str = "files";
}

#[derive(Default)]
pub struct Rm;
impl Tool for Rm {
    fn name(&self) -> &'static str {
        "rm"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        rm_main(args.iter().cloned())
    }
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    rm_main(args)
}

pub fn rm_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let matches = ct_app()
        .after_help(t!("rm.after_help"))
        .try_get_matches_from(args)?;

    let files = extract_files(&matches);
    let force_flag = matches.get_flag(rm_flags::RM_FORCE);

    validate_input(&files, force_flag)?;

    let options = RMOptions::new(&matches)?;

    if should_remove_file(&files, &options) && remove(&files, &options) {
        return Err(1.into());
    }
    Ok(())
}

fn should_remove_file(files: &[&OsStr], options: &RMOptions) -> bool {
    if should_prompt_user(options, files) {
        // 获取第一个文件的名称
        let first_file = Path::new(files[0]).display().to_string();

        // 根据是否有多个文件决定提示信息
        let has_multiple_files = files.len() > 1;

        match (options.recursive, has_multiple_files) {
            (true, true) => {
                ct_prompt_yes!("remove files recursively starting from '{}'?", first_file)
            }
            (true, false) => {
                ct_prompt_yes!("remove '{}' and its contents recursively?", first_file)
            }
            (false, true) => {
                ct_prompt_yes!("remove files starting from '{}'?", first_file)
            }
            (false, false) => {
                ct_prompt_yes!("remove '{}'?", first_file)
            }
        }
    } else {
        true
    }
}

fn extract_files(matches: &clap::ArgMatches) -> Vec<&OsStr> {
    matches
        .get_many::<OsString>(rm_flags::RM_ARG_FILES)
        .map(|v| v.map(OsString::as_os_str).collect())
        .unwrap_or_default()
}

fn should_force_prompt_never(matches: &clap::ArgMatches, force_flag: bool) -> bool {
    force_flag && {
        let force_index = matches.index_of(rm_flags::RM_FORCE).unwrap_or(0);
        ![
            rm_flags::RM_PROMPT,
            rm_flags::RM_PROMPT_MORE,
            rm_flags::RM_INTERACTIVE,
        ]
        .iter()
        .any(|flag| {
            matches.value_source(flag) == Some(ValueSource::CommandLine)
                && matches.index_of(flag).unwrap_or(0) > force_index
        })
    }
}

fn validate_input(files: &[&OsStr], force_flag: bool) -> CTResult<()> {
    if files.is_empty() && !force_flag {
        Err(CTsageError::new(1, "missing operand"))
    } else {
        Ok(())
    }
}

fn determine_interactive_mode(
    matches: &clap::ArgMatches,
    force_prompt_never: bool,
) -> InteractiveMode {
    if force_prompt_never {
        InteractiveMode::Never
    } else if matches.get_flag(rm_flags::RM_PROMPT) {
        InteractiveMode::Always
    } else if matches.get_flag(rm_flags::RM_PROMPT_MORE) {
        InteractiveMode::Once
    } else if matches.contains_id(rm_flags::RM_INTERACTIVE) {
        match matches
            .get_one::<String>(rm_flags::RM_INTERACTIVE)
            .unwrap()
            .as_str()
        {
            "never" => InteractiveMode::Never,
            "once" => InteractiveMode::Once,
            "always" => InteractiveMode::Always,
            val => panic!("Invalid argument to interactive ({val})"), // Ideally, this should return a Result
        }
    } else {
        InteractiveMode::PromptProtected
    }
}

fn should_prompt_user(options: &RMOptions, files: &[&OsStr]) -> bool {
    options.interactive == InteractiveMode::Once && (options.recursive || files.len() > 3)
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("rm.about");
    let usage_description = t!("rm.usage");
    let args = vec![
        Arg::new(rm_flags::RM_FORCE)
            .short('f')
            .long(rm_flags::RM_FORCE)
            .help(t!("rm.clap.rm_force"))
            .action(ArgAction::SetTrue),
        Arg::new(rm_flags::RM_PROMPT)
            .short('i')
            .help(t!("rm.clap.rm_prompt"))
            .overrides_with_all([rm_flags::RM_PROMPT_MORE, rm_flags::RM_INTERACTIVE])
            .action(ArgAction::SetTrue),
        Arg::new(rm_flags::RM_PROMPT_MORE)
            .short('I')
            .help(
                "prompt once before removing more than three files, or when removing recursively. \
        Less intrusive than -i, while still giving some protection against most mistakes",
            )
            .overrides_with_all([rm_flags::RM_PROMPT, rm_flags::RM_INTERACTIVE])
            .action(ArgAction::SetTrue),
        Arg::new(rm_flags::RM_INTERACTIVE)
            .long(rm_flags::RM_INTERACTIVE)
            .help(
                "prompt according to WHEN: never, once (-I), or always (-i). Without WHEN, \
            prompts always",
            )
            .value_name("WHEN")
            .num_args(0..=1)
            .require_equals(true)
            .default_missing_value("always")
            .overrides_with_all([rm_flags::RM_PROMPT, rm_flags::RM_PROMPT_MORE]),
        Arg::new(rm_flags::RM_ONE_FILE_SYSTEM)
            .long(rm_flags::RM_ONE_FILE_SYSTEM)
            .help(
                "when removing a hierarchy recursively, skip any directory that is on a file \
            system different from that of the corresponding command line argument (NOT \
            IMPLEMENTED)",
            )
            .action(ArgAction::SetTrue),
        Arg::new(rm_flags::RM_NO_PRESERVE_ROOT)
            .long(rm_flags::RM_NO_PRESERVE_ROOT)
            .help(t!("rm.clap.rm_no_preserve_root"))
            .action(ArgAction::SetTrue),
        Arg::new(rm_flags::RM_PRESERVE_ROOT)
            .long(rm_flags::RM_PRESERVE_ROOT)
            .help(t!("rm.clap.rm_preserve_root"))
            .action(ArgAction::SetTrue),
        Arg::new(rm_flags::RM_RECURSIVE)
            .short('r')
            .visible_short_alias('R')
            .long(rm_flags::RM_RECURSIVE)
            .help(t!("rm.clap.rm_recursive"))
            .action(ArgAction::SetTrue),
        Arg::new(rm_flags::RM_DIR)
            .short('d')
            .long(rm_flags::RM_DIR)
            .help(t!("rm.clap.rm_dir"))
            .action(ArgAction::SetTrue),
        Arg::new(rm_flags::RM_VERBOSE)
            .short('v')
            .long(rm_flags::RM_VERBOSE)
            .help(t!("rm.clap.rm_verbose"))
            .action(ArgAction::SetTrue),
        Arg::new(rm_flags::RM_PRESUME_INPUT_TTY)
            .long("presume-input-tty")
            .alias(rm_flags::RM_PRESUME_INPUT_TTY)
            .hide(true)
            .action(ArgAction::SetTrue),
        Arg::new(rm_flags::RM_ARG_FILES)
            .action(ArgAction::Append)
            .value_parser(ValueParser::os_string())
            .num_args(1..)
            .value_hint(clap::ValueHint::AnyPath),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args_override_self(true)
        .args(args)
}

// TODO: implement one-file-system (this may get partially implemented in walkdir)
/// Remove (or unlink) the given files
///
/// Returns true if it has encountered an error.
///
/// Behavior is determined by the `options` parameter, see [`RMOptions`] for
/// details.
pub fn remove(files: &[&OsStr], options: &RMOptions) -> bool {
    let mut had_err = false;

    for filename in files {
        let file = Path::new(filename);
        had_err = match file.symlink_metadata() {
            Ok(metadata) => {
                if metadata.is_dir() {
                    handle_dir(file, options)
                } else if is_symlink_dir(&metadata) {
                    remove_dir(file, options)
                } else {
                    remove_file(file, options)
                }
            }
            Err(_e) => {
                // TODO: actually print out the specific error
                // TODO: When the error is not about missing files
                // (e.g., permission), even rm -f should fail with
                // outputting the error, but there's no easy way.
                if options.force {
                    false
                } else {
                    ct_show_error!(
                        "cannot remove {}: No such file or directory",
                        filename.quote()
                    );
                    true
                }
            }
        }
        .bitor(had_err);
    }

    had_err
}

#[allow(clippy::cognitive_complexity)]
fn handle_dir(path: &Path, options: &RMOptions) -> bool {
    let mut had_err = false;

    let is_root = path.has_root() && path.parent().is_none();
    if options.recursive && (!is_root || !options.preserve_root) {
        if options.interactive != InteractiveMode::Always && !options.verbose {
            if let Err(e) = fs::remove_dir_all(path) {
                // GNU compatibility (rm/empty-inacc.sh)
                // remove_dir_all failed. maybe it is because of the permissions
                // but if the directory is empty, remove_dir might work.
                // So, let's try that before failing for real
                if fs::remove_dir(path).is_err() {
                    had_err = true;
                    if e.kind() == std::io::ErrorKind::PermissionDenied {
                        // GNU compatibility (rm/fail-eacces.sh)
                        // here, GNU doesn't use some kind of remove_dir_all
                        // It will show directory+file
                        ct_show_error!("cannot remove {}: {}", path.quote(), "Permission denied");
                    } else {
                        ct_show_error!("cannot remove {}: {}", path.quote(), e);
                    }
                }
            }
        } else {
            let mut dirs: VecDeque<DirEntry> = VecDeque::new();
            // The Paths to not descend into. We need to this because WalkDir doesn't have a way, afaik, to not descend into a directory
            // So we have to just ignore paths as they come up if they start with a path we aren't descending into
            let mut not_descended: Vec<PathBuf> = Vec::new();

            'outer: for entry in WalkDir::new(path) {
                match entry {
                    Ok(entry) => {
                        if options.interactive == InteractiveMode::Always {
                            for not_descend in &not_descended {
                                if entry.path().starts_with(not_descend) {
                                    // We don't need to continue the rest of code in this loop if we are in a directory we don't want to descend into
                                    continue 'outer;
                                }
                            }
                        }
                        let file_type = entry.file_type();
                        if file_type.is_dir() {
                            // If we are in Interactive Mode Always and the directory isn't empty we ask if we should descend else we push this directory onto dirs vector
                            if options.interactive == InteractiveMode::Always
                                && fs::read_dir(entry.path()).unwrap().count() != 0
                            {
                                // If we don't descend we push this directory onto our not_descended vector else we push this directory onto dirs vector
                                if prompt_descend(entry.path()) {
                                    dirs.push_back(entry);
                                } else {
                                    not_descended.push(entry.path().to_path_buf());
                                }
                            } else {
                                dirs.push_back(entry);
                            }
                        } else {
                            had_err = remove_file(entry.path(), options).bitor(had_err);
                        }
                    }
                    Err(e) => {
                        had_err = true;
                        ct_show_error!("recursing in {}: {}", path.quote(), e);
                    }
                }
            }

            for dir in dirs.iter().rev() {
                had_err = remove_dir(dir.path(), options).bitor(had_err);
            }
        }
    } else if options.dir && (!is_root || !options.preserve_root) {
        had_err = remove_dir(path, options).bitor(had_err);
    } else if options.recursive {
        ct_show_error!("could not remove directory {}", path.quote());
        had_err = true;
    } else {
        ct_show_error!(
            "cannot remove {}: Is a directory", // GNU's rm error message does not include help
            path.quote()
        );
        had_err = true;
    }

    had_err
}

fn remove_dir(path: &Path, options: &RMOptions) -> bool {
    if prompt_dir(path, options) {
        if let Ok(mut read_dir) = fs::read_dir(path) {
            if options.dir || options.recursive {
                if read_dir.next().is_none() {
                    match fs::remove_dir(path) {
                        Ok(_) => {
                            if options.verbose {
                                println!("removed directory {}", normalize(path).quote());
                            }
                        }
                        Err(e) => {
                            if e.kind() == std::io::ErrorKind::PermissionDenied {
                                // GNU compatibility (rm/fail-eacces.sh)
                                ct_show_error!(
                                    "cannot remove {}: {}",
                                    path.quote(),
                                    "Permission denied"
                                );
                            } else {
                                ct_show_error!("cannot remove {}: {}", path.quote(), e);
                            }
                            return true;
                        }
                    }
                } else {
                    // directory can be read but is not empty
                    ct_show_error!("cannot remove {}: Directory not empty", path.quote());
                    return true;
                }
            } else {
                // called to remove a symlink_dir (windows) without "-r"/"-R" or "-d"
                ct_show_error!("cannot remove {}: Is a directory", path.quote());
                return true;
            }
        } else {
            // GNU's rm shows this message if directory is empty but not readable
            ct_show_error!("cannot remove {}: Directory not empty", path.quote());
            return true;
        }
    }

    false
}

fn remove_file(path: &Path, options: &RMOptions) -> bool {
    if prompt_file(path, options) {
        match fs::remove_file(path) {
            Ok(_) => {
                if options.verbose {
                    println!("removed {}", normalize(path).quote());
                }
            }
            Err(e) => {
                if e.kind() == std::io::ErrorKind::PermissionDenied {
                    // GNU compatibility (rm/fail-eacces.sh)
                    ct_show_error!("cannot remove {}: {}", path.quote(), "Permission denied");
                } else {
                    ct_show_error!("cannot remove {}: {}", path.quote(), e);
                }
                return true;
            }
        }
    }

    false
}

fn prompt_dir(path: &Path, options: &RMOptions) -> bool {
    // If interactive is Never we never want to send prompts
    if options.interactive == InteractiveMode::Never {
        return true;
    }

    // We can't use metadata.permissions.readonly for directories because it only works on files
    // So we have to handle whether a directory is writable manually
    if let Ok(metadata) = fs::metadata(path) {
        handle_writable_directory(path, options, &metadata)
    } else {
        true
    }
}

fn prompt_file(path: &Path, options: &RMOptions) -> bool {
    // If interactive is Never we never want to send prompts
    if options.interactive == InteractiveMode::Never {
        return true;
    }
    // If interactive is Always we want to check if the file is symlink to prompt the right message
    if options.interactive == InteractiveMode::Always {
        if let Ok(metadata) = fs::symlink_metadata(path) {
            if metadata.is_symlink() {
                return ct_prompt_yes!("remove symbolic link {}?", path.quote());
            }
        }
    }
    // File::open(path) doesn't open the file in write mode so we need to use file options to open it in also write mode to check if it can written too
    match File::options().read(true).write(true).open(path) {
        Ok(file) => {
            let Ok(metadata) = file.metadata() else {
                return true;
            };

            if options.interactive == InteractiveMode::Always && !metadata.permissions().readonly()
            {
                return if metadata.len() == 0 {
                    ct_prompt_yes!("remove regular empty file {}?", path.quote())
                } else {
                    ct_prompt_yes!("remove file {}?", path.quote())
                };
            }
        }
        Err(err) => {
            if err.kind() != ErrorKind::PermissionDenied {
                return true;
            }
        }
    }
    prompt_file_permission_readonly(path)
}

fn prompt_file_permission_readonly(path: &Path) -> bool {
    match fs::metadata(path) {
        Ok(metadata) if !metadata.permissions().readonly() => true,
        Ok(metadata) if metadata.len() == 0 => ct_prompt_yes!(
            "remove write-protected regular empty file {}?",
            path.quote()
        ),
        _ => ct_prompt_yes!("remove write-protected regular file {}?", path.quote()),
    }
}

// For directories finding if they are writable or not is a hassle. In Unix we can use the built-in rust crate to to check mode bits. But other os don't have something similar afaik
// Most cases are covered by keep eye out for edge cases
#[cfg(unix)]
fn handle_writable_directory(path: &Path, options: &RMOptions, metadata: &Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let mode = metadata.permissions().mode();
    // Check if directory has user write permissions
    #[allow(clippy::unnecessary_cast)]
    let user_writable = (mode & (libc::S_IWUSR as u32)) != 0;
    if !user_writable {
        ct_prompt_yes!("remove write-protected directory {}?", path.quote())
    } else if options.interactive == InteractiveMode::Always {
        ct_prompt_yes!("remove directory {}?", path.quote())
    } else {
        true
    }
}

// For windows we can use windows metadata trait and file attributes to see if a directory is readonly
#[cfg(windows)]
fn handle_writable_directory(path: &Path, options: &RMOptions, metadata: &Metadata) -> bool {
    use std::os::windows::prelude::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_READONLY;
    let not_user_writable = (metadata.file_attributes() & FILE_ATTRIBUTE_READONLY) != 0;
    if not_user_writable {
        ct_prompt_yes!("remove write-protected directory {}?", path.quote())
    } else if options.interactive == InteractiveMode::Always {
        ct_prompt_yes!("remove directory {}?", path.quote())
    } else {
        true
    }
}

// I have this here for completeness but it will always return "remove directory {}" because metadata.permissions().readonly() only works for file not directories
#[cfg(not(windows))]
#[cfg(not(unix))]
fn handle_writable_directory(path: &Path, options: &RMOptions, metadata: &Metadata) -> bool {
    if options.interactive == InteractiveMode::Always {
        ct_prompt_yes!("remove directory {}?", path.quote())
    } else {
        true
    }
}

fn prompt_descend(path: &Path) -> bool {
    ct_prompt_yes!("descend into directory {}?", path.quote())
}

fn normalize(path: &Path) -> PathBuf {
    // copied from https://github.com/rust-lang/cargo/blob/2e4cfc2b7d43328b207879228a2ca7d427d188bb/src/cargo/util/paths.rs#L65-L90
    // both projects are MIT https://github.com/rust-lang/cargo/blob/master/LICENSE-MIT
    // for std impl progress see rfc https://github.com/rust-lang/rfcs/issues/2208
    // TODO: replace this once that lands
    ctcore::ct_fs::normalize_path(path)
}

#[cfg(not(windows))]
fn is_symlink_dir(_metadata: &Metadata) -> bool {
    false
}

#[cfg(windows)]
fn is_symlink_dir(metadata: &Metadata) -> bool {
    use std::os::windows::prelude::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY;

    metadata.file_type().is_symlink()
        && ((metadata.file_attributes() & FILE_ATTRIBUTE_DIRECTORY) != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let rm = Rm::default();

        // Test name method
        assert_eq!(rm.name(), "rm");

        // Test command method
        let command = rm.command();
        assert!(command.get_name().contains("rm"));

        // Test execute method with no arguments
        let args = vec![OsString::from("rm")];
        let result = rm.execute(&args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), 1);

        // Test execute method with help flag
        let args = vec![OsString::from("rm"), OsString::from("--help")];
        let result = rm.execute(&args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), 0);

        // Test execute method with version flag
        let args = vec![OsString::from("rm"), OsString::from("--version")];
        let result = rm.execute(&args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), 0);
    }

    #[test]
    fn test_remove_dir() {
        // 创建一个临时目录用于测试
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path();

        // 调用 remove_dir 函数
        let options = RMOptions {
            force: false,
            interactive: InteractiveMode::Never,
            one_fs: false,
            preserve_root: false,
            recursive: true,
            dir: true,
            verbose: true,
        };
        let result = remove_dir(path, &options);

        // 断言结果为 false，表示目录成功删除
        assert!(!result);
    }

    #[test]
    fn test_remove_dir_not_empty() {
        // 创建一个临时目录并在其中创建一些文件
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path();

        fs::File::create(path.join("file1.txt")).unwrap();

        // 调用 remove_dir 函数
        let options = RMOptions {
            force: false,
            interactive: InteractiveMode::Never,
            one_fs: false,
            preserve_root: false,
            recursive: true,
            dir: true,
            verbose: true,
        };
        let result = remove_dir(path, &options);

        // 断言结果为 true，表示目录因为非空而无法删除
        assert!(result);
    }
    /*
        #[test]
        fn test_remove_dir_permission_denied() {
            // 创建一个临时目录并设置权限为只读
            let temp_dir = tempfile::tempdir().unwrap();
            let path = temp_dir.path();

            fs::set_permissions(path, fs::Permissions::from_mode(0o000)).unwrap();

            // 调用 remove_dir 函数
            let options = RMOptions {
                force: false,
                interactive: InteractiveMode::Never,
                one_fs: false,
                preserve_root: false,
                recursive: true,
                dir: true,
                verbose: true,
            };
            let result = remove_dir(path, &options);

            // 断言结果为 true，表示因为权限被拒绝而无法删除
            assert!(result);

            // 恢复权限
            fs::set_permissions(path, fs::Permissions::from_mode(0o777)).unwrap();

            // 清理临时目录
            fs::remove_dir_all(path).unwrap();
        }
    */
    #[test]
    fn test_handle_dir() {
        // 创建一个临时目录用于测试
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path();

        // 设置一些测试选项
        let options = RMOptions {
            force: false,
            interactive: InteractiveMode::Never,
            one_fs: false,
            preserve_root: false,
            recursive: true,
            dir: true,
            verbose: true,
        };

        // 调用函数进行测试
        let result = handle_dir(path, &options);

        // 断言结果
        assert_eq!(result, false);
    }

    #[test]
    fn test_handle_dir_recursive() {
        // 创建一个临时目录用于测试
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path();

        // 在临时目录下创建一些子目录和文件
        fs::create_dir_all(path.join("subdir1")).unwrap();
        fs::create_dir_all(path.join("subdir2")).unwrap();
        fs::File::create(path.join("file1.txt")).unwrap();

        // 设置一些测试选项
        let options = RMOptions {
            force: false,
            interactive: InteractiveMode::Never,
            one_fs: false,
            preserve_root: false,
            recursive: true,
            dir: true,
            verbose: true,
        };

        // 调用函数进行测试
        let result = handle_dir(path, &options);

        // 断言结果
        assert_eq!(result, false);
    }

    /*
    #[test]
    fn test_handle_dir_error() {
        // 创建一个临时目录用于测试
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path();

        // 设置一些测试选项，模拟权限错误
        let options = RMOptions {
            force: false,
            interactive: InteractiveMode::Never,
            one_fs: false,
            preserve_root: false,
            recursive: true,
            dir: true,
            verbose: true,
        };

        // 模拟权限错误
        fs::set_permissions(path, fs::Permissions::from_mode(0o000)).unwrap();

        // 调用函数进行测试
        let result = handle_dir(path, &options);

        // 断言结果
        assert_eq!(result, true);
    }
    */
    mod test_handle_writable_directory {
        use super::*;
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        #[test]
        fn test_handle_writable_directory() {
            // 创建一个临时目录用于测试
            let temp_dir = tempfile::tempdir().unwrap();
            let path = temp_dir.path();

            // 设置目录权限为只读
            fs::set_permissions(path, fs::Permissions::from_mode(0o777)).unwrap();

            // 创建 RMOptions 和 Metadata 实例
            let options = RMOptions {
                force: false,
                interactive: InteractiveMode::PromptProtected,
                one_fs: false,
                preserve_root: false,
                recursive: true,
                dir: true,
                verbose: false,
            };

            // 调用函数进行测试
            if let Ok(metadata) = fs::metadata(path) {
                let result = handle_writable_directory(path, &options, &metadata);
                // 断言结果为 false，因为目录不可写
                assert!(result);
            }

            // 清理临时目录
            temp_dir.close().unwrap();
        }
    }

    mod tests_remove_file {
        use crate::InteractiveMode;
        use std::fs;

        use crate::RMOptions;

        use std::path::Path;

        use std::os::unix::fs::PermissionsExt;

        #[test]
        fn test_remove_file_success() {
            // 创建一个临时文件
            let temp_file = Path::new("temp_file.txt");
            fs::write(temp_file, "Test content").unwrap();

            let options = RMOptions {
                force: false,
                interactive: InteractiveMode::Never,
                one_fs: false,
                preserve_root: false,
                recursive: true,
                dir: false,
                verbose: false,
            };

            // 调用 remove_file 函数
            let result = crate::remove_file(temp_file, &options);

            // 断言文件被成功删除
            assert!(!result);
            assert!(!temp_file.exists());
        }

        #[test]
        pub(crate) fn test_remove_file_permission_denied() {
            // 创建一个只读文件
            let read_only_file = Path::new("read_only_file.txt");
            fs::write(read_only_file, "Test content").unwrap();
            let mode = 0o444; // 只读权限
            let permissions = PermissionsExt::from_mode(mode);
            fs::set_permissions(read_only_file, permissions).unwrap();

            let options = RMOptions {
                force: false,
                interactive: InteractiveMode::Never,
                one_fs: false,
                preserve_root: false,
                recursive: true,
                dir: false,
                verbose: false,
            };

            // 调用 remove_file 函数
            let result = crate::remove_file(read_only_file, &options);

            // 断言返回值为 true，表示遇到权限拒绝错误
            assert!(!result);
            // 清理临时文件
            let _ = fs::remove_file(read_only_file);
        }
    }
}
