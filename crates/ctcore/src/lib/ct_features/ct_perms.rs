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

//! Common functions to manage permissions

use crate::ct_display::Quotable;
use crate::ct_error::{CTResult, CtSimpleError, strip_errno};
pub use crate::ct_features::ct_entries;
use crate::ct_show_error;
use clap::{Arg, ArgMatches, Command};
use libc::{gid_t, uid_t};
use walkdir::WalkDir;

use std::io::Error as IOError;
use std::io::Result as IOResult;

use std::ffi::CString;
use std::fs::Metadata;
use std::os::unix::fs::MetadataExt;

use std::os::unix::ffi::OsStrExt;
use std::path::{MAIN_SEPARATOR_STR, Path};

/// The various level of verbosity
#[derive(PartialEq, Eq, Clone, Debug)]
pub enum CtVerbosityLevel {
    Silent,
    Changes,
    Verbose,
    Normal,
}
#[derive(PartialEq, Eq, Clone, Debug)]
pub struct Verbosity {
    pub groups_only: bool,
    pub level: CtVerbosityLevel,
}

/// Actually perform the change of owner on a path
fn chown<P: AsRef<Path>>(path: P, uid: uid_t, gid: gid_t, follow: bool) -> IOResult<()> {
    let path = path.as_ref();
    let s = CString::new(path.as_os_str().as_bytes()).unwrap();
    let ret = unsafe {
        if follow {
            libc::chown(s.as_ptr(), uid, gid)
        } else {
            libc::lchown(s.as_ptr(), uid, gid)
        }
    };
    if ret == 0 {
        Ok(())
    } else {
        Err(IOError::last_os_error())
    }
}

/// Perform the change of owner on a path
/// with the various options
/// and error messages management
pub fn wrap_chown<P: AsRef<Path>>(
    path: P,
    meta: &Metadata,
    dest_uid: Option<u32>,
    dest_gid: Option<u32>,
    follow: bool,
    verbosity: Verbosity,
) -> Result<String, String> {
    let dest_uid = dest_uid.unwrap_or_else(|| meta.uid());
    let dest_gid = dest_gid.unwrap_or_else(|| meta.gid());
    let path = path.as_ref();
    let mut out: String = String::new();

    if let Err(e) = chown(path, dest_uid, dest_gid, follow) {
        match verbosity.level {
            CtVerbosityLevel::Silent => (),
            level => {
                out = format!(
                    "changing {} of {}: {}",
                    if verbosity.groups_only {
                        "group"
                    } else {
                        "ownership"
                    },
                    path.quote(),
                    e
                );
                if level == CtVerbosityLevel::Verbose {
                    out = if verbosity.groups_only {
                        let gid = meta.gid();
                        format!(
                            "{}\nfailed to change group of {} from {} to {}",
                            out,
                            path.quote(),
                            ct_entries::gid2grp(gid).unwrap_or_else(|_| gid.to_string()),
                            ct_entries::gid2grp(dest_gid).unwrap_or_else(|_| dest_gid.to_string())
                        )
                    } else {
                        let uid = meta.uid();
                        let gid = meta.gid();
                        format!(
                            "{}\nfailed to change ownership of {} from {}:{} to {}:{}",
                            out,
                            path.quote(),
                            ct_entries::uid2usr(uid).unwrap_or_else(|_| uid.to_string()),
                            ct_entries::gid2grp(gid).unwrap_or_else(|_| gid.to_string()),
                            ct_entries::uid2usr(dest_uid).unwrap_or_else(|_| dest_uid.to_string()),
                            ct_entries::gid2grp(dest_gid).unwrap_or_else(|_| dest_gid.to_string())
                        )
                    };
                };
            }
        }
        return Err(out);
    } else {
        let changed = dest_uid != meta.uid() || dest_gid != meta.gid();
        if changed {
            match verbosity.level {
                CtVerbosityLevel::Changes | CtVerbosityLevel::Verbose => {
                    let gid = meta.gid();
                    out = if verbosity.groups_only {
                        format!(
                            "changed group of {} from {} to {}",
                            path.quote(),
                            ct_entries::gid2grp(gid).unwrap_or_else(|_| gid.to_string()),
                            ct_entries::gid2grp(dest_gid).unwrap_or_else(|_| dest_gid.to_string())
                        )
                    } else {
                        let gid = meta.gid();
                        let uid = meta.uid();
                        format!(
                            "changed ownership of {} from {}:{} to {}:{}",
                            path.quote(),
                            ct_entries::uid2usr(uid).unwrap_or_else(|_| uid.to_string()),
                            ct_entries::gid2grp(gid).unwrap_or_else(|_| gid.to_string()),
                            ct_entries::uid2usr(dest_uid).unwrap_or_else(|_| dest_uid.to_string()),
                            ct_entries::gid2grp(dest_gid).unwrap_or_else(|_| dest_gid.to_string())
                        )
                    };
                }
                _ => (),
            };
        } else if verbosity.level == CtVerbosityLevel::Verbose {
            out = if verbosity.groups_only {
                format!(
                    "group of {} retained as {}",
                    path.quote(),
                    ct_entries::gid2grp(dest_gid).unwrap_or_default()
                )
            } else {
                format!(
                    "ownership of {} retained as {}:{}",
                    path.quote(),
                    ct_entries::uid2usr(dest_uid).unwrap_or_else(|_| dest_uid.to_string()),
                    ct_entries::gid2grp(dest_gid).unwrap_or_else(|_| dest_gid.to_string())
                )
            };
        }
    }
    Ok(out)
}

pub enum CtIfFrom {
    All,
    User(u32),
    Group(u32),
    UserGroup(u32, u32),
}

#[derive(PartialEq, Eq)]
pub enum CtTraverseSymlinks {
    None,
    First,
    All,
}

pub struct CtChownExecutor {
    pub dest_uid: Option<u32>,
    pub dest_gid: Option<u32>,
    pub raw_owner: String, //如果第二个字符有效，则移除减号并返回true
    pub traverse_symlinks: CtTraverseSymlinks,
    pub verbosity: Verbosity,
    pub filter: CtIfFrom,
    pub files: Vec<String>,
    pub recursive: bool,
    pub preserve_root: bool,
    pub dereference: bool,
}

#[cfg(test)]
pub fn check_root(path: &Path, would_recurse_symlink: bool) -> bool {
    is_root(path, would_recurse_symlink)
}

/// In the context of chown and chgrp, check whether we are in a "preserve-root" scenario.
///
/// In particular, we want to prohibit further traversal only if:
///     (--preserve-root and -R present) &&
///     (path canonicalizes to "/") &&
///     (
///         (path is a symlink && would traverse/recurse this symlink) ||
///         (path is not a symlink)
///     )
/// The first clause is checked by the caller, the second and third clause is checked here.
/// The caller has to evaluate -P/-H/-L into 'would_recurse_symlink'.
/// Recall that canonicalization resolves both relative paths (e.g. "..") and symlinks.
fn is_root(path: &Path, would_traverse_symlink: bool) -> bool {
    // 第三个子句可以在没有任何系统调用的情况下进行评估，所以我们先这样做。
    // 如果would_recurse_symlink为真，那么无论路径是否为符号链接，该子句都为真。
    // 否则，我们只需要在这里检查路径在语法上是否可以是符号链接：
    if !would_traverse_symlink {
        // 我们不能在这里检查 path.is_dir()，因为这会解析符号链接，这是我们这里需要避免的。
        // 所有类似目录的路径都匹配“/”，除了“.”，“..”，“/.”和“*/..”。
        let looks_like_dir = match path.as_os_str().to_str() {
            // 如果它包含特殊字符，出于安全考虑，倾向于禁止chown操作：
            None => false,
            Some(".") | Some("..") => true,
            Some(path_str) => {
                (path_str.ends_with(MAIN_SEPARATOR_STR))
                    || (path_str.ends_with(&format!("{MAIN_SEPARATOR_STR}.")))
                    || (path_str.ends_with(&format!("{MAIN_SEPARATOR_STR}..")))
            }
        };

        if !looks_like_dir {
            return false;
        }
    }

    // FIXME: TOCTOU漏洞！canonicalize()运行的时间与WalkDir的递归决策时间不同。
    // 然而，我们被迫在甚至试图chown路径之前（更不用说在WalkDir内部做stat）就决定是否警告--preserve-root
    if let Ok(p) = path.canonicalize() {
        let path_buf = path.to_path_buf();
        if p.parent().is_none() {
            if path_buf.as_os_str() == "/" {
                ct_show_error!("it is dangerous to operate recursively on '/'");
            } else {
                ct_show_error!(
                    "it is dangerous to operate recursively on {} (same as '/')",
                    path_buf.quote()
                );
            }
            ct_show_error!("use --no-preserve-root to override this failsafe");
            return true;
        }
    }

    false
}

impl CtChownExecutor {
    pub fn exec(&self) -> CTResult<()> {
        let mut ret = 0;
        for f in &self.files {
            ret |= self.traverse(f);
        }
        if ret != 0 {
            return Err(ret.into());
        }
        Ok(())
    }

    #[allow(clippy::cognitive_complexity)]
    fn traverse<P: AsRef<Path>>(&self, root: P) -> i32 {
        let path = root.as_ref();
        let meta = match self.obtain_meta(path, self.dereference) {
            Some(m) => m,
            _ => {
                if self.verbosity.level == CtVerbosityLevel::Verbose {
                    println!(
                        "failed to change ownership of {} to {}",
                        path.quote(),
                        self.raw_owner
                    );
                }
                return 1;
            }
        };

        if self.recursive
            && self.preserve_root
            && is_root(path, self.traverse_symlinks != CtTraverseSymlinks::None)
        {
            //快速失败，不尝试递归。
            return 1;
        }

        let ret = if self.matched(meta.uid(), meta.gid()) {
            match wrap_chown(
                path,
                &meta,
                self.dest_uid,
                self.dest_gid,
                self.dereference,
                self.verbosity.clone(),
            ) {
                Ok(n) => {
                    if !n.is_empty() {
                        ct_show_error!("{}", n);
                    }
                    0
                }
                Err(e) => {
                    if self.verbosity.level != CtVerbosityLevel::Silent {
                        ct_show_error!("{}", e);
                    }
                    1
                }
            }
        } else {
            self.print_verbose_ownership_retained_as(
                path,
                meta.uid(),
                self.dest_gid.map(|_| meta.gid()),
            );
            0
        };

        if self.recursive {
            ret | self.dive_into(&root)
        } else {
            ret
        }
    }

    #[allow(clippy::cognitive_complexity)]
    fn dive_into<P: AsRef<Path>>(&self, root: P) -> i32 {
        let root = root.as_ref();

        //walkdir总是解析根目录，所以我们必须自己检查
        if self.traverse_symlinks == CtTraverseSymlinks::None && root.is_symlink() {
            return 0;
        }

        let mut ret = 0;
        let mut iterator = WalkDir::new(root)
            .follow_links(self.traverse_symlinks == CtTraverseSymlinks::All)
            .min_depth(1)
            .into_iter();
        // 我们不能使用 for 循环，因为在循环内部我们需要操作迭代器。
        while let Some(entry) = iterator.next() {
            let entry = match entry {
                Err(e) => {
                    ret = 1;
                    if let Some(path) = e.path() {
                        ct_show_error!(
                            "cannot access '{}': {}",
                            path.display(),
                            if let Some(error) = e.io_error() {
                                strip_errno(error)
                            } else {
                                "Too many levels of symbolic links".into()
                            }
                        );
                    } else {
                        ct_show_error!("{}", e);
                    }
                    continue;
                }
                Ok(entry) => entry,
            };
            let path = entry.path();
            let meta = match self.obtain_meta(path, self.dereference) {
                Some(m) => m,
                _ => {
                    ret = 1;
                    if entry.file_type().is_dir() {
                        // 指示walkdir跳过此目录，以避免walkdir尝试查询此目录的子目录时再次出现错误。
                        iterator.skip_current_dir();
                    }
                    continue;
                }
            };

            if self.preserve_root
                && is_root(path, self.traverse_symlinks == CtTraverseSymlinks::All)
            {
                // 快速失败，不再递归深入。
                return 1;
            }

            if !self.matched(meta.uid(), meta.gid()) {
                self.print_verbose_ownership_retained_as(
                    path,
                    meta.uid(),
                    self.dest_gid.map(|_| meta.gid()),
                );
                continue;
            }

            ret = match wrap_chown(
                path,
                &meta,
                self.dest_uid,
                self.dest_gid,
                self.dereference,
                self.verbosity.clone(),
            ) {
                Ok(n) => {
                    if !n.is_empty() {
                        ct_show_error!("{}", n);
                    }
                    0
                }
                Err(e) => {
                    if self.verbosity.level != CtVerbosityLevel::Silent {
                        ct_show_error!("{}", e);
                    }
                    1
                }
            }
        }
        ret
    }

    fn obtain_meta<P: AsRef<Path>>(&self, path: P, follow: bool) -> Option<Metadata> {
        let path = path.as_ref();
        let meta = if follow {
            path.metadata()
        } else {
            path.symlink_metadata()
        };
        match meta {
            Err(e) => {
                match self.verbosity.level {
                    CtVerbosityLevel::Silent => (),
                    _ => ct_show_error!(
                        "cannot {} {}: {}",
                        if follow { "dereference" } else { "access" },
                        path.quote(),
                        strip_errno(&e)
                    ),
                }
                None
            }
            Ok(meta) => Some(meta),
        }
    }

    #[inline]
    fn matched(&self, uid: uid_t, gid: gid_t) -> bool {
        match self.filter {
            CtIfFrom::All => true,
            CtIfFrom::User(u) => u == uid,
            CtIfFrom::Group(g) => g == gid,
            CtIfFrom::UserGroup(u, g) => u == uid && g == gid,
        }
    }

    fn print_verbose_ownership_retained_as(&self, path: &Path, uid: u32, gid: Option<u32>) {
        if self.verbosity.level == CtVerbosityLevel::Verbose {
            match (self.dest_uid, self.dest_gid, gid) {
                (Some(_), Some(_), Some(gid)) => {
                    println!(
                        "ownership of {} retained as {}:{}",
                        path.quote(),
                        ct_entries::uid2usr(uid).unwrap_or_else(|_| uid.to_string()),
                        ct_entries::gid2grp(gid).unwrap_or_else(|_| gid.to_string()),
                    );
                }
                (None, Some(_), Some(gid)) => {
                    println!(
                        "ownership of {} retained as {}",
                        path.quote(),
                        ct_entries::gid2grp(gid).unwrap_or_else(|_| gid.to_string()),
                    );
                }
                (_, _, _) => {
                    println!(
                        "ownership of {} retained as {}",
                        path.quote(),
                        ct_entries::uid2usr(uid).unwrap_or_else(|_| uid.to_string()),
                    );
                }
            }
        }
    }
}

pub mod opt_flags {
    pub const HELP: &str = "help";
    pub mod verbosity {
        pub const CHANGES: &str = "changes";
        pub const QUIET: &str = "quiet";
        pub const SILENT: &str = "silent";
        pub const VERBOSE: &str = "verbose";
    }
    pub mod preserve_root {
        pub const PRESERVE: &str = "preserve-root";
        pub const NO_PRESERVE: &str = "no-preserve-root";
    }
    pub mod dereference {
        pub const DEREFERENCE: &str = "dereference";
        pub const NO_DEREFERENCE: &str = "no-dereference";
    }
    pub const FROM: &str = "from";
    pub const RECURSIVE: &str = "recursive";
    pub mod traverse {
        pub const TRAVERSE: &str = "H";
        pub const NO_TRAVERSE: &str = "P";
        pub const EVERY: &str = "L";
    }
    pub const REFERENCE: &str = "reference";
    pub const ARG_OWNER: &str = "OWNER";
    pub const ARG_GROUP: &str = "GROUP";
    pub const ARG_FILES: &str = "FILE";
}

pub struct CtGidUidOwnerFilter {
    pub dest_gid: Option<u32>,
    pub dest_uid: Option<u32>,
    pub raw_owner: String,
    pub filter: CtIfFrom,
}
type GidUidFilterOwnerParser = fn(&ArgMatches) -> CTResult<CtGidUidOwnerFilter>;

/// Base implementation for `chgrp` and `chown`.
///
/// An argument called `add_arg_if_not_reference` will be added to `command` if
/// `args` does not contain the `--reference` option.
/// `parse_gid_uid_and_filter` will be called to obtain the target gid and uid, and the filter,
/// from `ArgMatches`.
/// `groups_only` determines whether verbose output will only mention the group.
#[allow(clippy::cognitive_complexity)]
pub fn chown_base(
    mut command: Command,
    args: impl crate::Args,
    add_arg_if_not_reference: &'static str,
    parse_gid_uid_and_filter: GidUidFilterOwnerParser,
    groups_only: bool,
) -> CTResult<()> {
    let args: Vec<_> = args.collect();
    let mut reference = false;
    let mut help = false;
    // stop processing options on --
    for arg in args.iter().take_while(|s| *s != "--") {
        if arg.to_string_lossy().starts_with("--reference=") || arg == "--reference" {
            reference = true;
        } else if arg == "--help" {
            // we stop processing once we see --help,
            // as it doesn't matter if we've seen reference or not
            help = true;
            break;
        }
    }

    if help || !reference {
        // add both positional arguments
        // arg_group is only required if
        command = command.arg(
            Arg::new(add_arg_if_not_reference)
                .value_name(add_arg_if_not_reference)
                .required(true),
        );
    }
    command = command.arg(
        Arg::new(opt_flags::ARG_FILES)
            .value_name(opt_flags::ARG_FILES)
            .value_hint(clap::ValueHint::FilePath)
            .action(clap::ArgAction::Append)
            .required(true)
            .num_args(1..),
    );
    let matches = command.try_get_matches_from(args)?;

    let files: Vec<String> = matches
        .get_many::<String>(opt_flags::ARG_FILES)
        .map(|v| v.map(ToString::to_string).collect())
        .unwrap_or_default();

    let preserve_root = matches.get_flag(opt_flags::preserve_root::PRESERVE);

    let mut dereference = if matches.get_flag(opt_flags::dereference::DEREFERENCE) {
        Some(true)
    } else if matches.get_flag(opt_flags::dereference::NO_DEREFERENCE) {
        Some(false)
    } else {
        None
    };

    let mut traverse_symlinks = if matches.get_flag(opt_flags::traverse::TRAVERSE) {
        CtTraverseSymlinks::First
    } else if matches.get_flag(opt_flags::traverse::EVERY) {
        CtTraverseSymlinks::All
    } else {
        CtTraverseSymlinks::None
    };

    let recursive = matches.get_flag(opt_flags::RECURSIVE);
    if recursive {
        if traverse_symlinks == CtTraverseSymlinks::None {
            if dereference == Some(true) {
                return Err(CtSimpleError::new(1, "-R --dereference requires -H or -L"));
            }
            dereference = Some(false);
        }
    } else {
        traverse_symlinks = CtTraverseSymlinks::None;
    }

    let verbosity_level = if matches.get_flag(opt_flags::verbosity::CHANGES) {
        CtVerbosityLevel::Changes
    } else if matches.get_flag(opt_flags::verbosity::SILENT)
        || matches.get_flag(opt_flags::verbosity::QUIET)
    {
        CtVerbosityLevel::Silent
    } else if matches.get_flag(opt_flags::verbosity::VERBOSE) {
        CtVerbosityLevel::Verbose
    } else {
        CtVerbosityLevel::Normal
    };
    let CtGidUidOwnerFilter {
        dest_gid,
        dest_uid,
        raw_owner,
        filter,
    } = parse_gid_uid_and_filter(&matches)?;

    let executor = CtChownExecutor {
        traverse_symlinks,
        dest_gid,
        dest_uid,
        raw_owner,
        verbosity: Verbosity {
            groups_only,
            level: verbosity_level,
        },
        recursive,
        dereference: dereference.unwrap_or(true),
        preserve_root,
        files,
        filter,
    };
    executor.exec()
}

#[cfg(test)]
mod tests {
    // Note this useful idiom: importing names from outer (for mod tests) scope.
    use super::*;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix;
    use std::path::{Component, PathBuf};
    #[cfg(unix)]
    use tempfile::tempdir;
    #[test]
    fn test_empty_string() {
        let path = PathBuf::new();
        assert_eq!(path.to_str(), Some(""));
        // 这里要测试的主要点是我们不会崩溃。结果应该是'false'，以避免不必要的和令人困惑的警告。
        assert!(!is_root(&path, false));
        assert!(!is_root(&path, true));
    }

    #[allow(clippy::needless_borrow)]
    #[cfg(unix)]
    #[test]
    fn test_literal_root() {
        let component = Component::RootDir;
        let path: &Path = component.as_ref();
        assert_eq!(
            path.to_str(),
            Some("/"),
            "cfg(unix) but using non-unix path delimiters?!"
        );

        // 必须返回 true，这是 --preserve-root 应阻止的主要场景。
        assert!(is_root(&path, false));
        assert!(is_root(&path, true));
    }

    #[cfg(unix)]
    #[test]
    fn test_symlink_slash() {
        let temp_dir = tempdir().unwrap();
        let symlink_path = temp_dir.path().join("symlink");
        unix::fs::symlink(PathBuf::from("/"), symlink_path).unwrap();
        let symlink_path_slash = temp_dir.path().join("symlink/");
        // 必须返回 true，因为我们即将“意外地”对 "/" 进行递归操作，
        // 因为 "symlink/" 总是被视为已进入的目录 // 来自 GNU 的输出：
        //   $ chown --preserve-root -RH --dereference $(id -u) slink-to-root/
        //   chown: it is dangerous to operate recursively on 'slink-to-root/' (same as '/')
        //   chown: use --no-preserve-root to override this failsafe
        //   [$? = 1]
        //   $ chown --preserve-root -RH --no-dereference $(id -u) slink-to-root/
        //   chown: it is dangerous to operate recursively on 'slink-to-root/' (same as '/')
        //   chown: use --no-preserve-root to override this failsafe
        //   [$? = 1]
        assert!(is_root(&symlink_path_slash, false));
        assert!(is_root(&symlink_path_slash, true));
    }

    #[cfg(unix)]
    #[test]
    fn test_symlink_no_slash() {
        // 这涵盖了命令行参数情况和递归情况。
        let temp_dir = tempdir().unwrap();
        let symlink_path = temp_dir.path().join("symlink");
        unix::fs::symlink(PathBuf::from("/"), &symlink_path).unwrap();
        // 仅当我们将要“意外地”对 "/" 进行递归操作时才返回 true。
        assert!(!is_root(&symlink_path, false));
        assert!(is_root(&symlink_path, true));
    }
    #[test]
    fn test_check_root_valid_cases() {
        // Test case 1: root path is "/", would_traverse_symlink is true
        let result1 = check_root(Path::new("/"), true);
        assert!(result1);

        // Test case 2: root path is "/", would_traverse_symlink is false
        let result2 = check_root(Path::new("/"), false);
        assert!(result2);

        // Test case 3: root path is not "/", would_traverse_symlink is true
        let result3 = check_root(Path::new("/test"), true);
        assert!(!result3);

        // Test case 4: root path is not "/", would_traverse_symlink is false
        let result4 = check_root(Path::new("/test"), false);
        assert!(!result4);
    }

    #[test]
    fn test_check_root_invalid_cases() {
        // Test case 5: Invalid path (non-existent), would_traverse_symlink is true
        let non_existent_path = Path::new("non_existent");
        let result5 = check_root(non_existent_path, true);
        // 根据实际情况判断此处应抛出错误还是返回特定值
        assert!(!result5);

        // Test case 6: Invalid path (non-existent), would_traverse_symlink is false
        let result6 = check_root(non_existent_path, false);
        // 同上，根据实际情况进行断言
        assert!(!result6);

        // Test case 7: Handling symbolic links (if applicable)
        // 如果函数应该处理符号链接，请添加相应的测试用例
        // 注意：在大多数情况下，仅路径字符串并不足以模拟符号链接行为
    }

    #[test]
    fn test_chown() {
        // Prepare test data
        let test_path = "/tmp/test_file";
        fs::create_dir_all("/tmp").expect("create_dir_all FAIL");
        fs::write(test_path, "test data").expect("write FAIL");

        // Change ownership of the file
        let uid = 0; // Replace with the desired UID
        let gid = 0; // Replace with the desired GID
        let follow = true; // Set to true if following symbolic links
        chown(test_path, uid, gid, follow).expect("chown FAIL");

        // Verify ownership change
        let metadata = fs::metadata(test_path);
        assert_eq!(metadata.expect("REASON").clone().uid(), uid);
        //assert_eq!(metadata.expect("REASON").clone().gid(), gid as u32);

        // Clean up test data
        fs::remove_file(test_path).expect("Tremove_file FAIL");
    }
}
