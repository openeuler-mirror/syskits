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
use clap::{Arg, ArgMatches, Command, builder::OsStringValueParser};
use libc::{gid_t, uid_t};
use rust_i18n::t;
use walkdir::WalkDir;

use std::io::Error as IOError;
use std::io::Result as IOResult;

use std::ffi::{CString, OsString};
use std::fs::{File, Metadata};
use std::os::fd::{FromRawFd, IntoRawFd};
use std::os::unix::fs::MetadataExt;

use std::collections::HashSet;
use std::os::unix::ffi::OsStrExt;
use std::path::{MAIN_SEPARATOR_STR, Path, PathBuf};

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
    pub force_silent: bool,
    pub level: CtVerbosityLevel,
}

/// Names to use in ownership-change diagnostics.
#[derive(Default)]
pub struct CtChownOutputNames<'a> {
    pub user: Option<&'a str>,
    pub group: Option<&'a str>,
}

struct CtChownRequest<'a> {
    dest_uid: Option<u32>,
    dest_gid: Option<u32>,
    output_names: CtChownOutputNames<'a>,
    filter: &'a CtIfFrom,
    follow: bool,
    verbosity: Verbosity,
}

static ALL_FILTER: CtIfFrom = CtIfFrom::All;

#[derive(Debug)]
struct ChownFailure {
    stderr: Option<String>,
    stdout: Option<String>,
}

impl ChownFailure {
    fn into_error_message(self) -> String {
        match (self.stderr, self.stdout) {
            (Some(stderr), Some(stdout)) => format!("{stderr}\n{stdout}"),
            (Some(stderr), None) => stderr,
            (None, Some(stdout)) => stdout,
            (None, None) => String::new(),
        }
    }
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

#[derive(Debug, PartialEq, Eq)]
enum RestrictedChownResult {
    Applied,
    Skipped,
}

fn matches_filter(filter: &CtIfFrom, uid: uid_t, gid: gid_t) -> bool {
    match filter {
        CtIfFrom::All => true,
        CtIfFrom::User(user) => *user == uid,
        CtIfFrom::Group(group) => *group == gid,
        CtIfFrom::UserGroup(user, group) => *user == uid && *group == gid,
    }
}

fn restricted_chown<P: AsRef<Path>>(
    path: P,
    original_meta: &Metadata,
    uid: uid_t,
    gid: gid_t,
    filter: &CtIfFrom,
) -> IOResult<RestrictedChownResult> {
    let path = path.as_ref();
    if !original_meta.is_file() && !original_meta.is_dir() {
        chown(path, uid, gid, true)?;
        return Ok(RestrictedChownResult::Applied);
    }

    let path_c = CString::new(path.as_os_str().as_bytes()).unwrap();
    let mut flags = libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOCTTY | libc::O_CLOEXEC;
    if original_meta.is_dir() {
        flags |= libc::O_DIRECTORY;
    }
    let mut fd = unsafe { libc::open(path_c.as_ptr(), flags) };
    if fd < 0
        && IOError::last_os_error().raw_os_error() == Some(libc::EACCES)
        && original_meta.is_file()
    {
        flags |= libc::O_WRONLY;
        fd = unsafe { libc::open(path_c.as_ptr(), flags) };
    }

    if fd < 0 {
        let error = IOError::last_os_error();
        if error.raw_os_error() == Some(libc::EACCES) {
            chown(path, uid, gid, true)?;
            return Ok(RestrictedChownResult::Applied);
        }
        return Err(error);
    }

    let file = unsafe { File::from_raw_fd(fd) };
    let current_meta = file.metadata()?;
    if current_meta.dev() != original_meta.dev() || current_meta.ino() != original_meta.ino() {
        return Ok(RestrictedChownResult::Skipped);
    }
    if !matches_filter(filter, current_meta.uid(), current_meta.gid()) {
        return Ok(RestrictedChownResult::Skipped);
    }

    let fd = file.into_raw_fd();
    let chown_result = unsafe { libc::fchown(fd, uid, gid) };
    let chown_error = (chown_result < 0).then(IOError::last_os_error);
    let close_result = unsafe { libc::close(fd) };
    if let Some(error) = chown_error {
        return Err(error);
    }
    if close_result < 0 {
        return Err(IOError::last_os_error());
    }
    Ok(RestrictedChownResult::Applied)
}

fn chown_ids_for_syscall(dest_uid: Option<u32>, dest_gid: Option<u32>) -> (uid_t, gid_t) {
    (
        dest_uid.unwrap_or(uid_t::MAX),
        dest_gid.unwrap_or(gid_t::MAX),
    )
}

/// Perform the change of owner on a path
/// with the various options
/// and error messages management
pub fn wrap_chown<P: AsRef<Path>>(
    path: P,
    meta: &Metadata,
    dest_uid: Option<u32>,
    dest_gid: Option<u32>,
    output_names: CtChownOutputNames<'_>,
    follow: bool,
    verbosity: Verbosity,
) -> Result<String, String> {
    wrap_chown_with_diagnostics(
        path,
        meta,
        CtChownRequest {
            dest_uid,
            dest_gid,
            output_names,
            filter: &ALL_FILTER,
            follow,
            verbosity,
        },
    )
    .map_err(ChownFailure::into_error_message)
}

fn wrap_chown_with_diagnostics<P: AsRef<Path>>(
    path: P,
    meta: &Metadata,
    request: CtChownRequest<'_>,
) -> Result<String, ChownFailure> {
    let CtChownRequest {
        dest_uid,
        dest_gid,
        output_names,
        filter,
        follow,
        verbosity,
    } = request;
    let dest_uid_val = dest_uid.unwrap_or_else(|| meta.uid());
    let dest_gid_val = dest_gid.unwrap_or_else(|| meta.gid());
    let (uid_for_syscall, gid_for_syscall) = chown_ids_for_syscall(dest_uid, dest_gid);
    let path = path.as_ref();
    let mut out: String = String::new();

    let uid = meta.uid();
    let gid = meta.gid();

    let group_only = verbosity.groups_only
        || (dest_uid.is_none() && output_names.user.is_none() && dest_gid.is_some());
    let (old_str, new_str) = if group_only {
        (
            ct_entries::gid2grp(gid).unwrap_or_else(|_| gid.to_string()),
            output_names
                .group
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| dest_gid_val.to_string()),
        )
    } else {
        match (dest_uid, dest_gid) {
            (Some(_), Some(_)) => (
                format!(
                    "{}:{}",
                    ct_entries::uid2usr(uid).unwrap_or_else(|_| uid.to_string()),
                    ct_entries::gid2grp(gid).unwrap_or_else(|_| gid.to_string())
                ),
                format!(
                    "{}:{}",
                    output_names
                        .user
                        .map(ToOwned::to_owned)
                        .unwrap_or_else(|| dest_uid_val.to_string()),
                    output_names
                        .group
                        .map(ToOwned::to_owned)
                        .unwrap_or_else(|| dest_gid_val.to_string())
                ),
            ),
            (Some(_), None) => (
                ct_entries::uid2usr(uid).unwrap_or_else(|_| uid.to_string()),
                output_names
                    .user
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| dest_uid_val.to_string()),
            ),
            (None, Some(_)) => (
                format!(
                    "{}:{}",
                    ct_entries::uid2usr(uid).unwrap_or_else(|_| uid.to_string()),
                    ct_entries::gid2grp(gid).unwrap_or_else(|_| gid.to_string())
                ),
                format!(
                    ":{}",
                    output_names
                        .group
                        .map(ToOwned::to_owned)
                        .unwrap_or_else(|| dest_gid_val.to_string())
                ),
            ),
            (None, None) => (
                ct_entries::uid2usr(uid).unwrap_or_else(|_| uid.to_string()),
                ct_entries::uid2usr(dest_uid_val).unwrap_or_else(|_| dest_uid_val.to_string()),
            ),
        }
    };
    let path_str = path.quote().to_string();

    let chown_result = if follow && !matches!(filter, CtIfFrom::All) {
        restricted_chown(path, meta, uid_for_syscall, gid_for_syscall, filter)
    } else {
        chown(path, uid_for_syscall, gid_for_syscall, follow)
            .map(|_| RestrictedChownResult::Applied)
    };

    match chown_result {
        Err(e) => {
            let verbose_output = (verbosity.level == CtVerbosityLevel::Verbose).then(|| {
                if group_only {
                    t!(
                        "ctcore.chgrp.failed_change",
                        file = path_str,
                        old = old_str,
                        new = new_str
                    )
                    .to_string()
                } else {
                    t!(
                        "ctcore.chown.failed_change",
                        file = path_str,
                        old = old_str,
                        new = new_str
                    )
                    .to_string()
                }
            });
            let stderr = (!verbosity.force_silent && verbosity.level != CtVerbosityLevel::Silent)
                .then(|| {
                    format!(
                        "changing {} of {}: {}",
                        if group_only { "group" } else { "ownership" },
                        path_str,
                        strip_errno(&e)
                    )
                });

            return Err(ChownFailure {
                stderr,
                stdout: verbose_output,
            });
        }
        Ok(RestrictedChownResult::Skipped) => {
            let stdout = (verbosity.level == CtVerbosityLevel::Verbose).then(|| {
                if group_only {
                    t!(
                        "ctcore.chgrp.failed_change",
                        file = path_str,
                        old = old_str,
                        new = new_str
                    )
                    .to_string()
                } else {
                    t!(
                        "ctcore.chown.failed_change",
                        file = path_str,
                        old = old_str,
                        new = new_str
                    )
                    .to_string()
                }
            });
            return Err(ChownFailure {
                stderr: None,
                stdout,
            });
        }
        Ok(RestrictedChownResult::Applied) => {
            let changed = dest_uid_val != meta.uid() || dest_gid_val != meta.gid();
            if changed {
                match verbosity.level {
                    CtVerbosityLevel::Changes | CtVerbosityLevel::Verbose => {
                        out = if group_only {
                            t!(
                                "ctcore.chgrp.changed_group",
                                file = path_str,
                                old = old_str,
                                new = new_str
                            )
                            .to_string()
                        } else {
                            t!(
                                "ctcore.chown.changed_ownership",
                                file = path_str,
                                old = old_str,
                                new = new_str
                            )
                            .to_string()
                        };
                    }
                    _ => (),
                };
            } else if verbosity.level == CtVerbosityLevel::Verbose {
                out = if group_only {
                    t!(
                        "ctcore.chgrp.retained_group",
                        file = path_str,
                        old = old_str
                    )
                    .to_string()
                } else if dest_uid.is_none() && dest_gid.is_none() {
                    t!("ctcore.chown.retained_ownership_no_change", file = path_str).to_string()
                } else {
                    t!(
                        "ctcore.chown.retained_ownership",
                        file = path_str,
                        old = old_str
                    )
                    .to_string()
                };
            }
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
    pub dest_user_name: Option<String>,
    pub dest_group_name: Option<String>,
    pub raw_owner: String, //如果第二个字符有效，则移除减号并返回true
    pub traverse_symlinks: CtTraverseSymlinks,
    pub verbosity: Verbosity,
    pub filter: CtIfFrom,
    pub files: Vec<OsString>,
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

fn metadata_failure_action(path: &Path, dereference: bool) -> &'static str {
    if dereference && path.is_symlink() {
        "dereference"
    } else {
        "access"
    }
}

fn traversal_failure_action(path: &Path, dereference: bool) -> &'static str {
    if path.is_dir() {
        "read directory"
    } else {
        metadata_failure_action(path, dereference)
    }
}

fn traversal_walk(root: &Path, traverse_symlinks: &CtTraverseSymlinks) -> WalkDir {
    WalkDir::new(root)
        .follow_links(traverse_symlinks == &CtTraverseSymlinks::All)
        .min_depth(1)
        // Followed directories must be yielded before their contents so
        // --preserve-root can reject a link to / before reading its hierarchy.
        .contents_first(traverse_symlinks != &CtTraverseSymlinks::All)
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

        if self.recursive {
            self.dive_into(path) | self.change_path(path, &meta)
        } else {
            self.change_path(path, &meta)
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
        let mut iterator = traversal_walk(root, &self.traverse_symlinks).into_iter();
        let mut unreadable_directories = HashSet::<PathBuf>::new();
        let mut deferred_directories = Vec::<(usize, PathBuf, Metadata)>::new();
        let defer_directories = self.traverse_symlinks == CtTraverseSymlinks::All;
        // 我们不能使用 for 循环，因为在循环内部我们需要操作迭代器。
        while let Some(entry) = iterator.next() {
            if defer_directories {
                let depth = entry
                    .as_ref()
                    .map_or_else(|error| error.depth(), |entry| entry.depth());
                ret |= self.change_deferred_directories(
                    &mut deferred_directories,
                    &unreadable_directories,
                    depth,
                );
            }
            let entry = match entry {
                Err(e) => {
                    // GNU FTS skips symlink-induced directory cycles.
                    if e.loop_ancestor().is_some() {
                        continue;
                    }
                    ret = 1;
                    let path = e.path();
                    let action = path.map(|path| {
                        let action = traversal_failure_action(path, self.dereference);
                        if action == "read directory" {
                            unreadable_directories.insert(path.to_path_buf());
                        }
                        action
                    });
                    if !self.verbosity.force_silent {
                        if let Some(path) = path {
                            ct_show_error!(
                                "cannot {} {}: {}",
                                action.expect("path action is available"),
                                path.quote(),
                                if let Some(error) = e.io_error() {
                                    strip_errno(error)
                                } else {
                                    "Too many levels of symbolic links".into()
                                }
                            );
                        } else {
                            ct_show_error!("{}", e);
                        }
                    }
                    continue;
                }
                Ok(entry) => entry,
            };
            let path = entry.path();
            if unreadable_directories.contains(path) {
                continue;
            }
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
                // Pre-order traversal lets us avoid entering a symlinked /.
                iterator.skip_current_dir();
                ret = 1;
                continue;
            }

            if defer_directories && entry.file_type().is_dir() {
                deferred_directories.push((entry.depth(), path.to_path_buf(), meta));
            } else {
                ret |= self.change_path(path, &meta);
            }
        }
        if defer_directories {
            ret |= self.change_deferred_directories(
                &mut deferred_directories,
                &unreadable_directories,
                0,
            );
        }
        ret
    }

    fn change_deferred_directories(
        &self,
        deferred_directories: &mut Vec<(usize, PathBuf, Metadata)>,
        unreadable_directories: &HashSet<PathBuf>,
        next_depth: usize,
    ) -> i32 {
        let mut ret = 0;
        while deferred_directories
            .last()
            .is_some_and(|(depth, _, _)| *depth >= next_depth)
        {
            let (_, path, meta) = deferred_directories.pop().unwrap();
            if !unreadable_directories.contains(&path) {
                ret |= self.change_path(&path, &meta);
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
                if !self.verbosity.force_silent {
                    ct_show_error!(
                        "cannot {} {}: {}",
                        metadata_failure_action(path, follow),
                        path.quote(),
                        strip_errno(&e)
                    );
                }
                None
            }
            Ok(meta) => Some(meta),
        }
    }

    #[inline]
    fn matched(&self, uid: uid_t, gid: gid_t) -> bool {
        matches_filter(&self.filter, uid, gid)
    }

    fn change_path(&self, path: &Path, meta: &Metadata) -> i32 {
        if self.matched(meta.uid(), meta.gid()) {
            match wrap_chown_with_diagnostics(
                path,
                meta,
                CtChownRequest {
                    dest_uid: self.dest_uid,
                    dest_gid: self.dest_gid,
                    output_names: CtChownOutputNames {
                        user: self.dest_user_name.as_deref(),
                        group: self.dest_group_name.as_deref(),
                    },
                    filter: &self.filter,
                    follow: self.dereference,
                    verbosity: self.verbosity.clone(),
                },
            ) {
                Ok(output) => {
                    if !output.is_empty() {
                        println!("{output}");
                    }
                    0
                }
                Err(error) => {
                    if let Some(stderr) = error.stderr {
                        ct_show_error!("{stderr}");
                    }
                    if let Some(stdout) = error.stdout {
                        println!("{stdout}");
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
        }
    }

    fn print_verbose_ownership_retained_as(&self, path: &Path, uid: u32, gid: Option<u32>) {
        if self.verbosity.level == CtVerbosityLevel::Verbose {
            let path_str = path.quote().to_string();
            if self.group_only_output() {
                let gid_val = gid.unwrap_or(0);
                let old_str = ct_entries::gid2grp(gid_val).unwrap_or_else(|_| gid_val.to_string());
                println!(
                    "{}",
                    t!(
                        "ctcore.chgrp.retained_group",
                        file = path_str,
                        old = old_str
                    )
                );
            } else {
                let old_str = match (self.dest_uid, self.dest_gid) {
                    (Some(_), Some(_)) | (None, Some(_)) => {
                        let gid_val = gid.unwrap_or(0);
                        format!(
                            "{}:{}",
                            ct_entries::uid2usr(uid).unwrap_or_else(|_| uid.to_string()),
                            ct_entries::gid2grp(gid_val).unwrap_or_else(|_| gid_val.to_string())
                        )
                    }
                    (Some(_), None) | (None, None) => {
                        ct_entries::uid2usr(uid).unwrap_or_else(|_| uid.to_string())
                    }
                };
                println!(
                    "{}",
                    if self.dest_uid.is_none() && self.dest_gid.is_none() {
                        t!("ctcore.chown.retained_ownership_no_change", file = path_str)
                    } else {
                        t!(
                            "ctcore.chown.retained_ownership",
                            file = path_str,
                            old = old_str
                        )
                    }
                );
            }
        }
    }

    fn group_only_output(&self) -> bool {
        self.verbosity.groups_only || (self.dest_uid.is_none() && self.dest_gid.is_some())
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
    pub dest_user_name: Option<String>,
    pub dest_group_name: Option<String>,
    pub raw_owner: String,
    pub filter: CtIfFrom,
}
type GidUidFilterOwnerParser = fn(&ArgMatches) -> CTResult<CtGidUidOwnerFilter>;

fn matching_long_option<'a>(command: &'a Command, argument: &OsString) -> Option<(&'a Arg, bool)> {
    let argument = argument.to_string_lossy();
    let option = argument.strip_prefix("--")?;
    let (option, has_inline_value) = option
        .split_once('=')
        .map_or((option, false), |(name, _)| (name, true));
    if option.is_empty() {
        return None;
    }

    let mut matches = command
        .get_arguments()
        .filter(|arg| arg.get_long().is_some_and(|long| long.starts_with(option)));
    let matching = matches.next()?;
    matches
        .next()
        .is_none()
        .then_some((matching, has_inline_value))
}

fn is_reference_option(command: &Command, argument: &OsString) -> bool {
    matching_long_option(command, argument)
        .is_some_and(|(arg, _)| arg.get_long() == Some(opt_flags::REFERENCE))
}

fn long_option_takes_next_value(command: &Command, argument: &OsString) -> bool {
    matching_long_option(command, argument).is_some_and(|(arg, has_inline_value)| {
        !has_inline_value && arg.get_num_args().is_some_and(|range| range.takes_values())
    })
}

fn is_operand(argument: &OsString) -> bool {
    let argument = argument.to_string_lossy();
    argument == "-" || !argument.starts_with('-')
}

fn has_option_before_operand(
    command: &Command,
    args: &[OsString],
    matches_option: impl Fn(&OsString) -> bool,
) -> bool {
    let posixly_correct = crate::ct_posix::posixly_correct();
    let mut skip_next_option_value = false;

    for argument in args.iter().skip(1) {
        if argument == "--" {
            break;
        }
        if skip_next_option_value {
            skip_next_option_value = false;
            continue;
        }
        if matches_option(argument) {
            return true;
        }
        if long_option_takes_next_value(command, argument) {
            skip_next_option_value = true;
        } else if posixly_correct && is_operand(argument) {
            break;
        }
    }

    false
}

fn has_reference_option(command: &Command, args: &[OsString]) -> bool {
    has_option_before_operand(command, args, |argument| {
        is_reference_option(command, argument)
    })
}

fn has_help_option(command: &Command, args: &[OsString]) -> bool {
    has_option_before_operand(command, args, |argument| argument == "--help")
}

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
    let help = has_help_option(&command, &args);
    let reference = has_reference_option(&command, &args);

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
            .value_parser(OsStringValueParser::new())
            .action(clap::ArgAction::Append)
            .required(true)
            .num_args(1..),
    );
    let matches = command.try_get_matches_from(args)?;

    let files: Vec<OsString> = matches
        .get_many::<OsString>(opt_flags::ARG_FILES)
        .map(|v| v.cloned().collect())
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
                return Err(CtSimpleError::new(
                    1,
                    "-R --dereference requires either -H or -L",
                ));
            }
            dereference = Some(false);
        }
    } else {
        traverse_symlinks = CtTraverseSymlinks::None;
    }

    let verbosity = verbosity_from_matches(&matches, groups_only);
    let CtGidUidOwnerFilter {
        dest_gid,
        dest_uid,
        dest_user_name,
        dest_group_name,
        raw_owner,
        filter,
    } = parse_gid_uid_and_filter(&matches)?;

    let executor = CtChownExecutor {
        traverse_symlinks,
        dest_gid,
        dest_uid,
        dest_user_name,
        dest_group_name,
        raw_owner,
        verbosity,
        recursive,
        dereference: dereference.unwrap_or(true),
        preserve_root,
        files,
        filter,
    };
    executor.exec()
}

fn verbosity_from_matches(matches: &ArgMatches, groups_only: bool) -> Verbosity {
    let level = if matches.get_flag(opt_flags::verbosity::CHANGES) {
        CtVerbosityLevel::Changes
    } else if matches.get_flag(opt_flags::verbosity::VERBOSE) {
        CtVerbosityLevel::Verbose
    } else {
        CtVerbosityLevel::Normal
    };

    Verbosity {
        groups_only,
        force_silent: matches.get_flag(opt_flags::verbosity::SILENT)
            || matches.get_flag(opt_flags::verbosity::QUIET),
        level,
    }
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

    #[cfg(unix)]
    #[test]
    fn test_metadata_failure_action_only_dereferences_symlinks() {
        let temp_dir = tempdir().unwrap();
        let dangling = temp_dir.path().join("dangling");
        unix::fs::symlink("missing", &dangling).unwrap();
        let missing = temp_dir.path().join("missing");

        assert_eq!(metadata_failure_action(&dangling, true), "dereference");
        assert_eq!(metadata_failure_action(&dangling, false), "access");
        assert_eq!(metadata_failure_action(&missing, true), "access");
    }

    #[cfg(unix)]
    #[test]
    fn test_recursive_failure_is_not_hidden_by_later_success() {
        let temp_dir = tempdir().unwrap();
        let tree = temp_dir.path().join("tree");
        fs::create_dir(&tree).unwrap();
        unix::fs::symlink("missing", tree.join("dangle")).unwrap();
        fs::write(tree.join("regular"), b"").unwrap();

        let executor = CtChownExecutor {
            dest_uid: Some(unsafe { libc::geteuid() }),
            dest_gid: Some(unsafe { libc::getegid() }),
            dest_user_name: None,
            dest_group_name: None,
            raw_owner: "current".to_string(),
            traverse_symlinks: CtTraverseSymlinks::All,
            verbosity: Verbosity {
                groups_only: false,
                force_silent: false,
                level: CtVerbosityLevel::Normal,
            },
            filter: CtIfFrom::All,
            files: vec![tree.into_os_string()],
            recursive: true,
            preserve_root: false,
            dereference: true,
        };

        assert!(executor.exec().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn test_recursive_followed_symlink_cycle_is_skipped() {
        let temp_dir = tempdir().unwrap();
        let tree = temp_dir.path().join("tree");
        fs::create_dir(&tree).unwrap();
        unix::fs::symlink("../tree", tree.join("loop")).unwrap();

        let executor = CtChownExecutor {
            dest_uid: Some(unsafe { libc::geteuid() }),
            dest_gid: Some(unsafe { libc::getegid() }),
            dest_user_name: None,
            dest_group_name: None,
            raw_owner: "current".to_string(),
            traverse_symlinks: CtTraverseSymlinks::All,
            verbosity: Verbosity {
                groups_only: false,
                force_silent: false,
                level: CtVerbosityLevel::Normal,
            },
            filter: CtIfFrom::All,
            files: vec![tree.into_os_string()],
            recursive: true,
            preserve_root: false,
            dereference: true,
        };

        assert!(executor.exec().is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn test_recursive_walk_visits_directory_after_its_contents() {
        let temp_dir = tempdir().unwrap();
        let tree = temp_dir.path().join("tree");
        let directory = tree.join("directory");
        fs::create_dir_all(&directory).unwrap();
        let child = directory.join("child");
        fs::write(&child, b"").unwrap();

        let paths: Vec<_> = traversal_walk(&tree, &CtTraverseSymlinks::None)
            .into_iter()
            .map(|entry| entry.unwrap().into_path())
            .collect();

        assert_eq!(paths, vec![child, directory]);
    }

    #[cfg(unix)]
    #[test]
    fn test_recursive_walk_visits_followed_directory_before_its_contents() {
        let temp_dir = tempdir().unwrap();
        let tree = temp_dir.path().join("tree");
        let target = temp_dir.path().join("target");
        let child = target.join("child");
        let link = tree.join("link");
        fs::create_dir(&tree).unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(&child, b"").unwrap();
        unix::fs::symlink(&target, &link).unwrap();

        let paths: Vec<_> = traversal_walk(&tree, &CtTraverseSymlinks::All)
            .into_iter()
            .map(|entry| entry.unwrap().into_path())
            .collect();

        assert_eq!(paths, vec![link.clone(), link.join("child")]);
    }

    #[cfg(unix)]
    #[test]
    fn test_verbose_no_owner_or_group_change_reports_retained_ownership() {
        let temp_dir = tempdir().unwrap();
        let file = temp_dir.path().join("file");
        fs::write(&file, b"").unwrap();
        let meta = file.metadata().unwrap();

        let output = wrap_chown(
            &file,
            &meta,
            None,
            None,
            CtChownOutputNames::default(),
            true,
            Verbosity {
                groups_only: false,
                force_silent: false,
                level: CtVerbosityLevel::Verbose,
            },
        )
        .unwrap();

        assert_eq!(output, format!("ownership of {} retained", file.quote()));
    }

    #[cfg(unix)]
    #[test]
    fn test_verbose_chown_failure_keeps_diagnostics_on_separate_streams() {
        let temp_dir = tempdir().unwrap();
        let file = temp_dir.path().join("file");
        fs::write(&file, b"").unwrap();
        let meta = file.metadata().unwrap();
        fs::remove_file(&file).unwrap();

        let failure = wrap_chown_with_diagnostics(
            &file,
            &meta,
            CtChownRequest {
                dest_uid: Some(1),
                dest_gid: None,
                output_names: CtChownOutputNames::default(),
                filter: &ALL_FILTER,
                follow: true,
                verbosity: Verbosity {
                    groups_only: false,
                    force_silent: false,
                    level: CtVerbosityLevel::Verbose,
                },
            },
        )
        .unwrap_err();

        let old_owner = ct_entries::uid2usr(meta.uid()).unwrap_or_else(|_| meta.uid().to_string());
        assert_eq!(
            failure.stderr,
            Some(format!(
                "changing ownership of {}: No such file or directory",
                file.quote()
            ))
        );
        assert_eq!(
            failure.stdout,
            Some(format!(
                "failed to change ownership of {} from {old_owner} to 1",
                file.quote()
            ))
        );
    }

    #[test]
    fn test_filtered_group_change_uses_group_diagnostic() {
        let executor = CtChownExecutor {
            dest_uid: None,
            dest_gid: Some(0),
            dest_user_name: None,
            dest_group_name: None,
            raw_owner: ":0".to_string(),
            traverse_symlinks: CtTraverseSymlinks::None,
            verbosity: Verbosity {
                groups_only: false,
                force_silent: false,
                level: CtVerbosityLevel::Verbose,
            },
            filter: CtIfFrom::UserGroup(u32::MAX, u32::MAX),
            files: Vec::new(),
            recursive: false,
            preserve_root: false,
            dereference: true,
        };

        assert!(executor.group_only_output());
    }

    #[test]
    fn test_force_silent_preserves_verbose_diagnostics() {
        let command = Command::new("chown")
            .arg(
                Arg::new(opt_flags::verbosity::CHANGES)
                    .short('c')
                    .action(clap::ArgAction::SetTrue),
            )
            .arg(
                Arg::new(opt_flags::verbosity::SILENT)
                    .short('f')
                    .action(clap::ArgAction::SetTrue),
            )
            .arg(
                Arg::new(opt_flags::verbosity::QUIET)
                    .long(opt_flags::verbosity::QUIET)
                    .action(clap::ArgAction::SetTrue),
            )
            .arg(
                Arg::new(opt_flags::verbosity::VERBOSE)
                    .short('v')
                    .action(clap::ArgAction::SetTrue),
            );
        let matches = command.try_get_matches_from(["chown", "-f", "-v"]).unwrap();

        let verbosity = verbosity_from_matches(&matches, false);

        assert!(verbosity.force_silent);
        assert_eq!(verbosity.level, CtVerbosityLevel::Verbose);
    }

    #[test]
    fn test_traversal_failure_uses_read_directory_for_directories() {
        let temp_dir = tempfile::tempdir().unwrap();
        let directory = temp_dir.path().join("directory");
        let file = temp_dir.path().join("file");
        fs::create_dir(&directory).unwrap();
        fs::write(&file, b"").unwrap();

        assert_eq!(traversal_failure_action(&directory, true), "read directory");
        assert_eq!(traversal_failure_action(&file, true), "access");
    }

    #[test]
    fn test_unspecified_ownership_uses_chown_sentinel_values() {
        assert_eq!(chown_ids_for_syscall(Some(1000), None), (1000, gid_t::MAX));
        assert_eq!(chown_ids_for_syscall(None, Some(1000)), (uid_t::MAX, 1000));
    }

    #[cfg(unix)]
    #[test]
    fn test_restricted_chown_rejects_replaced_symlink_referent() {
        let temp_dir = tempdir().unwrap();
        let safe = temp_dir.path().join("safe");
        let protected = temp_dir.path().join("protected");
        let victim = temp_dir.path().join("victim");
        fs::write(&safe, b"").unwrap();
        fs::write(&protected, b"").unwrap();
        unix::fs::symlink("safe", &victim).unwrap();

        let safe_meta = safe.metadata().unwrap();
        let protected_uid = protected.metadata().unwrap().uid();
        fs::remove_file(&victim).unwrap();
        unix::fs::symlink("protected", &victim).unwrap();

        assert_eq!(
            restricted_chown(
                &victim,
                &safe_meta,
                uid_t::MAX,
                gid_t::MAX,
                &CtIfFrom::User(safe_meta.uid()),
            )
            .unwrap(),
            RestrictedChownResult::Skipped
        );
        assert_eq!(protected.metadata().unwrap().uid(), protected_uid);
    }

    #[cfg(unix)]
    #[test]
    fn test_filtered_chown_rejects_replaced_symlink_referent() {
        let temp_dir = tempdir().unwrap();
        let safe = temp_dir.path().join("safe");
        let protected = temp_dir.path().join("protected");
        let victim = temp_dir.path().join("victim");
        fs::write(&safe, b"").unwrap();
        fs::write(&protected, b"").unwrap();
        unix::fs::symlink("safe", &victim).unwrap();

        let safe_meta = safe.metadata().unwrap();
        let protected_uid = protected.metadata().unwrap().uid();
        fs::remove_file(&victim).unwrap();
        unix::fs::symlink("protected", &victim).unwrap();

        let executor = CtChownExecutor {
            dest_uid: Some(safe_meta.uid()),
            dest_gid: None,
            dest_user_name: None,
            dest_group_name: None,
            raw_owner: safe_meta.uid().to_string(),
            traverse_symlinks: CtTraverseSymlinks::None,
            verbosity: Verbosity {
                groups_only: false,
                force_silent: false,
                level: CtVerbosityLevel::Normal,
            },
            filter: CtIfFrom::User(safe_meta.uid()),
            files: Vec::new(),
            recursive: false,
            preserve_root: false,
            dereference: true,
        };

        assert_eq!(executor.change_path(&victim, &safe_meta), 1);
        assert_eq!(protected.metadata().unwrap().uid(), protected_uid);
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
        use crate::ct_process;

        if ct_process::geteuid() != 0 {
            println!("Skipping test_chown: requires root privileges");
            return;
        }
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
