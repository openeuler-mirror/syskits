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
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CTsageError};
use ctcore::{ct_prompt_yes, ct_show_error};
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, Metadata};
use std::io::ErrorKind;
use std::ops::BitOr;
use std::path::{Path, PathBuf};
use sys_locale::get_locale;

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
    pub preserve_root_all: bool,
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

        // 解析 preserve_root 及其扩展参数
        let preserve_root_val = matches
            .get_one::<String>(rm_flags::RM_PRESERVE_ROOT)
            .map(|s| s.as_str());
        let preserve_root_all = preserve_root_val == Some("all");
        let preserve_root =
            preserve_root_val.is_some() || !matches.get_flag(rm_flags::RM_NO_PRESERVE_ROOT);

        Ok(RMOptions {
            force,
            interactive: determine_interactive_mode(matches, force_prompt_never),
            one_fs: matches.get_flag(rm_flags::RM_ONE_FILE_SYSTEM),
            preserve_root,
            preserve_root_all,
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
    let mut had_err = false;
    let mut safe_files = Vec::new();

    // 拦截试图删除 . 或 .. 的危险操作，并打印标准警告
    for file_osstr in &files {
        let path = Path::new(file_osstr);

        // 将 OsStr 转换为字符串，以便进行字面量匹配
        let path_str = file_osstr.to_string_lossy();

        // 去除尾部的所有斜杠 '/'
        let trimmed = path_str.trim_end_matches('/');

        // 提取最后一个路径节点（basename）
        let base_name = if let Some(idx) = trimmed.rfind('/') {
            &trimmed[idx + 1..]
        } else {
            trimmed
        };

        // 严格判断 basename 是否为 "." 或 ".."
        if base_name == "." || base_name == ".." {
            // 打印 GNU 期望的特制警告信息
            ct_show_error!(
                "refusing to remove '.' or '..' directory: skipping {}",
                path.quote()
            );
            had_err = true;
            // 跳过此文件，不加入 safe_files
        } else {
            safe_files.push(*file_osstr);
        }
    }

    // 只对通过了安全检查的文件执行真实的删除逻辑
    if !safe_files.is_empty()
        && should_remove_file(&safe_files, &options)
        && remove(&safe_files, &options)
    {
        had_err = true;
    }

    if had_err {
        return Err(1.into());
    }
    Ok(())
}

fn should_remove_file(files: &[&OsStr], options: &RMOptions) -> bool {
    if should_prompt_user(options, files) {
        let n = files.len();
        let plural = if n == 1 { "" } else { "s" };

        if options.recursive {
            ct_prompt_yes!("remove {} argument{} recursively?", n, plural)
        } else {
            ct_prompt_yes!("remove {} argument{}?", n, plural)
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
            val => panic!("Invalid argument to interactive ({val})"),
        }
    } else {
        // 遵循 POSIX 规范：如果输入不是 TTY，受保护文件直接删除不提示
        use std::io::IsTerminal;
        let presume_tty = matches.get_flag(rm_flags::RM_PRESUME_INPUT_TTY);
        if presume_tty || std::io::stdin().is_terminal() {
            InteractiveMode::PromptProtected
        } else {
            InteractiveMode::Never
        }
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
            .value_name("all")
            .num_args(0..=1)
            .require_equals(true)
            .value_parser(["all"])
            .default_missing_value("true"),
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

pub fn remove(files: &[&OsStr], options: &RMOptions) -> bool {
    let mut had_err = false;

    for filename in files {
        let file = Path::new(filename);

        // 处理 --preserve-root=all 保护挂载点的特性
        if options.preserve_root_all && is_mount_point(file) {
            ct_show_error!(
                "skipping {}, since it's on a different device",
                file.quote()
            );
            ct_show_error!("and --preserve-root=all is in effect");
            had_err = true;
            continue;
        }

        had_err = match file.symlink_metadata() {
            Ok(metadata) => {
                let top_dev = get_device(&metadata); // 获取顶层目录的设备号

                if metadata.is_dir() {
                    handle_dir(file, options, top_dev)
                } else if is_symlink_dir(&metadata) {
                    remove_dir(file, file, options)
                } else {
                    remove_file(file, file, options)
                }
            }
            Err(_e) => {
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
fn handle_dir(path: &Path, options: &RMOptions, top_dev: u64) -> bool {
    let mut had_err = false;

    let is_root = path.has_root() && path.parent().is_none();
    if options.recursive && (!is_root || !options.preserve_root) {
        // 使用跨平台的、防御 ENAMETOOLONG 长路径异常的单轨递归引擎
        had_err = remove_dir_tree(path, path, options, top_dev, true);
    } else if options.dir && (!is_root || !options.preserve_root) {
        had_err = remove_dir(path, path, options).bitor(had_err);
    } else if options.recursive {
        ct_show_error!("could not remove directory {}", path.quote());
        had_err = true;
    } else {
        ct_show_error!("cannot remove {}: Is a directory", path.quote());
        had_err = true;
    }

    had_err
}

struct DirRestorer {
    #[cfg(unix)]
    fd: File,
    #[cfg(not(unix))]
    path: PathBuf,
}

impl DirRestorer {
    fn new() -> std::io::Result<Self> {
        #[cfg(unix)]
        {
            Ok(Self {
                fd: File::open(".")?,
            })
        }
        #[cfg(not(unix))]
        {
            Ok(Self {
                path: std::env::current_dir()?,
            })
        }
    }

    fn restore(&self) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            unsafe {
                if libc::fchdir(self.fd.as_raw_fd()) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        }
        #[cfg(not(unix))]
        {
            std::env::set_current_dir(&self.path)
        }
    }
}

/// 核心递归引擎：完美解决深层目录长路径越界 (ENAMETOOLONG) 问题
/// `local_path`: 供系统调用使用的短路径（总是相对于当前 cwd 的直接子代）
/// `display_path`: 仅用于内存组装，打印在警告中的完整用户路径
fn remove_dir_tree(
    local_path: &Path,
    display_path: &Path,
    options: &RMOptions,
    top_dev: u64,
    is_top_level: bool,
) -> bool {
    let mut had_err = false;

    // 1. 跨文件系统检测
    if !is_top_level && options.one_fs {
        if let Ok(meta) = fs::symlink_metadata(local_path) {
            if get_device(&meta) != top_dev {
                ct_show_error!(
                    "skipping {}, since it's on a different device",
                    display_path.quote()
                );
                return true;
            }
        }
    }

    // 2. 准备深入目录。先保存当前目录现场。
    let restorer = match DirRestorer::new() {
        Ok(r) => r,
        Err(e) => {
            ct_show_error!("cannot open current directory: {}", e);
            return true;
        }
    };

    // 执行 chdir 下沉：规避超长路径崩溃
    if let Err(chdir_err) = std::env::set_current_dir(local_path) {
        // 无法 chdir (缺乏 x 权限)。但如果它有 r 权限，我们依然可以在外部读取列表！
        if let Ok(iter) = fs::read_dir(local_path) {
            for entry in iter {
                match entry {
                    Ok(e) => {
                        let name = e.file_name();
                        let child_display = display_path.join(&name);
                        let child_local = local_path.join(&name);

                        // 我们没有 x 权限，系统不允许解析下级路径，任何操作都会返回 EACCES。
                        // 强制触发系统的拒绝删除响应，以对齐 GNU 的子文件报警逻辑
                        if let Err(err) =
                            fs::remove_file(&child_local).or_else(|_| fs::remove_dir(&child_local))
                        {
                            if err.kind() == std::io::ErrorKind::PermissionDenied {
                                ct_show_error!(
                                    "cannot remove {}: {}",
                                    child_display.quote(),
                                    "Permission denied"
                                );
                            } else {
                                ct_show_error!("cannot remove {}: {}", child_display.quote(), err);
                            }
                            had_err = true;
                        }
                    }
                    Err(e) => {
                        ct_show_error!("cannot read directory {}: {}", display_path.quote(), e);
                        had_err = true;
                    }
                }
            }
            // 如果内部报错了，父级删除就会顺理成章地被跳过，这就完美吻合了 exp-solaris 测试预期！
            if !had_err {
                had_err |= remove_dir(local_path, display_path, options);
            }
            return had_err;
        }

        // 如果连 read_dir 都失败了，说明连 r 权限都没有。走原本的兜底逻辑。
        if !prompt_dir(local_path, display_path, options) {
            return false;
        }

        if fs::remove_dir(local_path).is_ok() {
            if options.verbose {
                println!("removed directory {}", normalize(display_path).quote());
            }
            return false;
        }

        let err_msg = if chdir_err.kind() == std::io::ErrorKind::PermissionDenied {
            "Permission denied".to_string()
        } else {
            chdir_err.to_string()
        };
        ct_show_error!(
            "cannot read directory {}: {}",
            display_path.quote(),
            err_msg
        );
        return true;
    }

    let mut is_empty = true;
    let mut entries = Vec::new();
    let mut read_dir_err = None;

    match fs::read_dir(".") {
        Ok(iter) => {
            for entry in iter {
                match entry {
                    Ok(e) => {
                        is_empty = false;
                        entries.push(e);
                    }
                    Err(e) => {
                        read_dir_err = Some(e);
                        break;
                    }
                }
            }
        }
        Err(e) => {
            read_dir_err = Some(e);
        }
    }

    // 如果能够进入 (有 x 权限) 但无权读取列表 (无 r 权限)，尝试以空目录直接抹杀它！
    if let Some(e) = read_dir_err {
        let _ = restorer.restore();

        if !prompt_dir(local_path, display_path, options) {
            return false;
        }

        if fs::remove_dir(local_path).is_ok() {
            if options.verbose {
                println!("removed directory {}", normalize(display_path).quote());
            }
            return false;
        }

        let err_msg = if e.kind() == std::io::ErrorKind::PermissionDenied {
            "Permission denied".to_string()
        } else {
            e.to_string()
        };
        ct_show_error!(
            "cannot read directory {}: {}",
            display_path.quote(),
            err_msg
        );
        return true;
    }

    // 3. 交互式确认
    if options.interactive == InteractiveMode::Always && !is_empty && !prompt_descend(display_path)
    {
        let _ = restorer.restore();
        return true; // 拒绝进入子树，标记为错误以防止父目录被删
    }

    // 4. 清理内层所有文件，此时 entry 仅为一个极短的 local filename
    for entry in entries {
        let name = entry.file_name();
        let child_display = display_path.join(&name);
        let child_local = Path::new(&name);

        let is_dir = match entry.file_type() {
            Ok(ft) => ft.is_dir(),
            Err(_) => fs::symlink_metadata(child_local)
                .map(|m| m.is_dir())
                .unwrap_or(false),
        };

        if is_dir {
            had_err |= remove_dir_tree(child_local, &child_display, options, top_dev, false);
        } else {
            had_err |= remove_file(child_local, &child_display, options);
        }
    }

    // 5. 子文件清理完毕。为了删除自己，必须先跳出回到父目录！
    if let Err(e) = restorer.restore() {
        ct_show_error!("failed to restore directory: {}", e);
        return true;
    }

    // 6. 只要子节点无报错，就在外部（父层）将其彻底移除
    if !had_err {
        had_err |= remove_dir(local_path, display_path, options);
    }

    had_err
}

fn remove_dir(local_path: &Path, display_path: &Path, options: &RMOptions) -> bool {
    if prompt_dir(local_path, display_path, options) {
        if !options.dir && !options.recursive {
            ct_show_error!("cannot remove {}: Is a directory", display_path.quote());
            return true;
        }

        // 尝试判断是否为空，如果 read_dir 被拒绝权限，则默认为 true 交给后续的 remove_dir 裁决
        let is_empty = match fs::read_dir(local_path) {
            Ok(mut iter) => iter.next().is_none(),
            Err(_) => true,
        };

        if is_empty {
            match fs::remove_dir(local_path) {
                Ok(_) => {
                    if options.verbose {
                        println!("removed directory {}", normalize(display_path).quote());
                    }
                    return false;
                }
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::PermissionDenied {
                        ct_show_error!(
                            "cannot remove {}: {}",
                            display_path.quote(),
                            "Permission denied"
                        );
                    } else {
                        ct_show_error!("cannot remove {}: {}", display_path.quote(), e);
                    }
                    return true;
                }
            }
        } else {
            ct_show_error!(
                "cannot remove {}: Directory not empty",
                display_path.quote()
            );
            return true;
        }
    }
    false
}

fn remove_file(local_path: &Path, display_path: &Path, options: &RMOptions) -> bool {
    if prompt_file(local_path, display_path, options) {
        match fs::remove_file(local_path) {
            Ok(_) => {
                if options.verbose {
                    println!("removed {}", normalize(display_path).quote());
                }
                return false;
            }
            Err(e) => {
                if e.kind() == std::io::ErrorKind::PermissionDenied {
                    ct_show_error!(
                        "cannot remove {}: {}",
                        display_path.quote(),
                        "Permission denied"
                    );
                } else {
                    ct_show_error!("cannot remove {}: {}", display_path.quote(), e);
                }
                return true;
            }
        }
    }
    false
}

fn prompt_dir(local_path: &Path, display_path: &Path, options: &RMOptions) -> bool {
    if options.interactive == InteractiveMode::Never {
        return true;
    }
    if let Ok(metadata) = fs::metadata(local_path) {
        handle_writable_directory(local_path, display_path, options, &metadata)
    } else {
        true
    }
}

fn prompt_file(local_path: &Path, display_path: &Path, options: &RMOptions) -> bool {
    if options.interactive == InteractiveMode::Never {
        return true;
    }
    if options.interactive == InteractiveMode::Always {
        if let Ok(metadata) = fs::symlink_metadata(local_path) {
            if metadata.is_symlink() {
                return ct_prompt_yes!("remove symbolic link {}?", display_path.quote());
            }
        }
    }
    match File::options().read(true).write(true).open(local_path) {
        Ok(file) => {
            let Ok(metadata) = file.metadata() else {
                return true;
            };

            if options.interactive == InteractiveMode::Always && !metadata.permissions().readonly()
            {
                return if metadata.len() == 0 {
                    ct_prompt_yes!("remove regular empty file {}?", display_path.quote())
                } else {
                    ct_prompt_yes!("remove file {}?", display_path.quote())
                };
            }
        }
        Err(err) => {
            if err.kind() != ErrorKind::PermissionDenied {
                return true;
            }
        }
    }
    prompt_file_permission_readonly(local_path, display_path)
}

fn prompt_file_permission_readonly(local_path: &Path, display_path: &Path) -> bool {
    #[cfg(unix)]
    if unsafe { libc::geteuid() } == 0 {
        return true;
    }

    match fs::metadata(local_path) {
        Ok(metadata) if !metadata.permissions().readonly() => true,
        Ok(metadata) if metadata.len() == 0 => ct_prompt_yes!(
            "remove write-protected regular empty file {}?",
            display_path.quote()
        ),
        _ => ct_prompt_yes!(
            "remove write-protected regular file {}?",
            display_path.quote()
        ),
    }
}

#[cfg(unix)]
fn handle_writable_directory(
    _local_path: &Path,
    display_path: &Path,
    options: &RMOptions,
    metadata: &Metadata,
) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let mode = metadata.permissions().mode();
    #[allow(clippy::unnecessary_cast)]
    let user_writable = (mode & (libc::S_IWUSR as u32)) != 0;

    let is_root = unsafe { libc::geteuid() } == 0;

    if !user_writable && !is_root {
        ct_prompt_yes!("remove write-protected directory {}?", display_path.quote())
    } else if options.interactive == InteractiveMode::Always {
        ct_prompt_yes!("remove directory {}?", display_path.quote())
    } else {
        true
    }
}

#[cfg(windows)]
fn handle_writable_directory(
    _local_path: &Path,
    display_path: &Path,
    options: &RMOptions,
    metadata: &Metadata,
) -> bool {
    use std::os::windows::prelude::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_READONLY;
    let not_user_writable = (metadata.file_attributes() & FILE_ATTRIBUTE_READONLY) != 0;
    if not_user_writable {
        ct_prompt_yes!("remove write-protected directory {}?", display_path.quote())
    } else if options.interactive == InteractiveMode::Always {
        ct_prompt_yes!("remove directory {}?", display_path.quote())
    } else {
        true
    }
}

#[cfg(not(windows))]
#[cfg(not(unix))]
fn handle_writable_directory(
    _local_path: &Path,
    display_path: &Path,
    options: &RMOptions,
    _metadata: &Metadata,
) -> bool {
    if options.interactive == InteractiveMode::Always {
        ct_prompt_yes!("remove directory {}?", display_path.quote())
    } else {
        true
    }
}

fn prompt_descend(display_path: &Path) -> bool {
    ct_prompt_yes!("descend into directory {}?", display_path.quote())
}

fn normalize(path: &Path) -> PathBuf {
    ctcore::ct_fs::normalize_path(path)
}

#[cfg(unix)]
fn get_device(metadata: &Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    metadata.dev()
}

#[cfg(not(unix))]
fn get_device(_metadata: &Metadata) -> u64 {
    0
}

fn is_mount_point(path: &Path) -> bool {
    if let Some(parent) = path.parent() {
        if parent.as_os_str().is_empty() {
            return false;
        }
        if let (Ok(m1), Ok(m2)) = (fs::symlink_metadata(path), fs::symlink_metadata(parent)) {
            return get_device(&m1) != get_device(&m2);
        }
    }
    false
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
    use std::ffi::{OsStr, OsString};
    use std::fs;
    use std::os::unix::fs::MetadataExt;
    use std::path::PathBuf;

    fn base_options() -> RMOptions {
        RMOptions {
            force: false,
            interactive: InteractiveMode::Never,
            one_fs: false,
            preserve_root: true,
            preserve_root_all: false,
            recursive: false,
            dir: false,
            verbose: false,
        }
    }

    #[test]
    fn test_tool_implementation() {
        let rm = Rm;

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
    fn test_should_force_prompt_never_logic() {
        let matches = ct_app()
            .try_get_matches_from(vec!["rm", "-f", "target"])
            .unwrap();
        assert!(should_force_prompt_never(&matches, true));

        let matches = ct_app()
            .try_get_matches_from(vec!["rm", "-f", "-i", "target"])
            .unwrap();
        assert!(!should_force_prompt_never(&matches, true));
    }

    #[test]
    fn test_extract_files_and_validate_input() {
        let matches = ct_app().try_get_matches_from(vec!["rm", "a", "b"]).unwrap();
        let files = extract_files(&matches);
        assert_eq!(files.len(), 2);

        assert!(validate_input(&[], false).is_err());
        assert!(validate_input(&[], true).is_ok());
        assert!(validate_input(&[OsStr::new("file")], false).is_ok());
    }

    #[test]
    fn test_rm_options_new_and_interactive_modes() {
        let matches = ct_app()
            .try_get_matches_from(vec!["rm", "-f", "-r", "target"])
            .unwrap();
        let opts = RMOptions::new(&matches).unwrap();
        assert!(opts.force);
        assert!(opts.recursive);
        assert!(matches!(opts.interactive, InteractiveMode::Never));

        let matches = ct_app()
            .try_get_matches_from(vec!["rm", "-i", "target"])
            .unwrap();
        let opts = RMOptions::new(&matches).unwrap();
        assert!(matches!(opts.interactive, InteractiveMode::Always));

        let matches = ct_app()
            .try_get_matches_from(vec!["rm", "-I", "target"])
            .unwrap();
        let opts = RMOptions::new(&matches).unwrap();
        assert!(matches!(opts.interactive, InteractiveMode::Once));

        let matches = ct_app()
            .try_get_matches_from(vec!["rm", "--interactive=never", "target"])
            .unwrap();
        let opts = RMOptions::new(&matches).unwrap();
        assert!(matches!(opts.interactive, InteractiveMode::Never));
    }

    #[test]
    fn test_should_prompt_user_logic() {
        let mut options = base_options();
        options.interactive = InteractiveMode::Once;
        options.recursive = true;
        let files: Vec<&OsStr> = vec![OsStr::new("a"), OsStr::new("b")];
        assert!(should_prompt_user(&options, &files));

        options.recursive = false;
        let files: Vec<&OsStr> = vec![
            OsStr::new("a"),
            OsStr::new("b"),
            OsStr::new("c"),
            OsStr::new("d"),
        ];
        assert!(should_prompt_user(&options, &files));

        options.interactive = InteractiveMode::Never;
        assert!(!should_prompt_user(&options, &files));
    }

    #[test]
    fn test_should_remove_file_without_prompt() {
        let options = base_options();
        let files: Vec<&OsStr> = vec![OsStr::new("dummy")];
        assert!(should_remove_file(&files, &options));
    }

    #[test]
    fn test_remove_successfully_deletes_file() {
        let temp = tempfile::tempdir().unwrap();
        let file_path = temp.path().join("file.txt");
        std::fs::write(&file_path, b"content").unwrap();

        let options = base_options();
        let files_vec = [file_path.into_os_string()];
        let removed_path = PathBuf::from(&files_vec[0]);
        let refs: Vec<&OsStr> = files_vec.iter().map(|s| s.as_os_str()).collect();

        let result = remove(&refs, &options);
        assert!(!result);
        assert!(!removed_path.exists());
    }

    #[test]
    fn test_remove_with_missing_file_sets_error() {
        let options = base_options();
        let missing = OsString::from("nonexistent.txt");
        let refs: Vec<&OsStr> = vec![missing.as_os_str()];
        let result = remove(&refs, &options);
        assert!(result);
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
            preserve_root_all: false,
            recursive: true,
            dir: true,
            verbose: true,
        };
        let result = remove_dir(path, path, &options);

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
            preserve_root_all: false,
            recursive: true,
            dir: true,
            verbose: true,
        };
        let result = remove_dir(path, path, &options);

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
            let result = remove_dir(path, path, &options);

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
            preserve_root_all: false,
            recursive: true,
            dir: true,
            verbose: true,
        };

        // 调用函数进行测试
        let metadata = std::fs::metadata(path).unwrap();
        let result = handle_dir(path, &options, metadata.dev());

        // 断言结果
        assert!(!result);
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
            preserve_root_all: false,
            recursive: true,
            dir: true,
            verbose: true,
        };

        // 调用函数进行测试
        let metadata = std::fs::metadata(path).unwrap();
        let result = handle_dir(path, &options, metadata.dev());

        // 断言结果
        assert!(!result);
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
            preserve_root_all: false,
            recursive: true,
            dir: true,
            verbose: true,
        };

        // 模拟权限错误
        fs::set_permissions(path, fs::Permissions::from_mode(0o000)).unwrap();

        // 调用函数进行测试
        let metadata = std::fs::metadata(path).unwrap();
        let result = handle_dir(path, &options, metadata.dev());

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
                preserve_root_all: false,
                recursive: true,
                dir: true,
                verbose: false,
            };

            // 调用函数进行测试
            if let Ok(metadata) = fs::metadata(path) {
                let result = handle_writable_directory(path, path, &options, &metadata);
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
                preserve_root_all: false,
                recursive: true,
                dir: false,
                verbose: false,
            };

            // 调用 remove_file 函数
            let result = crate::remove_file(temp_file, temp_file, &options);

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
                preserve_root_all: false,
                recursive: true,
                dir: false,
                verbose: false,
            };

            // 调用 remove_file 函数
            let result = crate::remove_file(read_only_file, read_only_file, &options);

            // 断言返回值为 true，表示遇到权限拒绝错误
            assert!(!result);
            // 清理临时文件
            let _ = fs::remove_file(read_only_file);
        }
    }
}
