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
//!
//! See the [`copy_directory`] function for more information.
#[cfg(windows)]
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf, StripPrefixError};

use ctcore::ct_display::Quotable;
use ctcore::ct_error::CTIoError;
use ctcore::ct_fs::{
    CtFileInformation, MissingHandling, ResolveMode, canonicalize, path_ends_with_terminator,
};
use ctcore::ct_show;
use ctcore::ct_show_error;
use ctcore::uio_error;
use filetime::FileTime;
use indicatif::ProgressBar;
use walkdir::{DirEntry, WalkDir};

use crate::{
    CopyResult, CpError, CpOptions, copy_attributes, copy_attributes_with_deref, copy_file,
    copy_recursive_link, cp_aligned_ancestors, cp_print_verbose_output, show_error_if_needed,
};

/// Ensure a Windows path starts with a `\\?`.
#[cfg(target_os = "windows")]
fn adjust_canonicalization(p: &Path) -> Cow<Path> {
    // In some cases, \\? can be missing on some Windows paths.  Add it at the
    // beginning unless the path is prefixed with a device namespace.
    const VERBATIM_PREFIX: &str = r"\\?";
    const DEVICE_NS_PREFIX: &str = r"\\.";

    let has_prefix = p
        .components()
        .next()
        .and_then(|comp| comp.as_os_str().to_str())
        .map(|p_str| p_str.starts_with(VERBATIM_PREFIX) || p_str.starts_with(DEVICE_NS_PREFIX))
        .unwrap_or_default();

    if has_prefix {
        p.into()
    } else {
        Path::new(VERBATIM_PREFIX).join(p).into()
    }
}

/// Get a descendant path relative to the given parent directory.
///
/// If `root_parent` is `None`, then this just returns the `path`
/// itself. Otherwise, this function strips the parent prefix from the
/// given `path`, leaving only the portion of the path relative to the
/// parent.
fn get_local_to_root_parent(
    path: &Path,
    root_parent: Option<&Path>,
) -> Result<PathBuf, StripPrefixError> {
    match root_parent {
        Some(parent) => {
            // On Windows, some paths are starting with \\?
            // but not always, so, make sure that we are consistent for strip_prefix
            // See https://docs.microsoft.com/en-us/windows/win32/fileio/naming-a-file for more info
            #[cfg(windows)]
            let (path, parent) = (
                adjust_canonicalization(path),
                adjust_canonicalization(parent),
            );
            let path = path.strip_prefix(parent)?;
            Ok(path.to_path_buf())
        }
        None => Ok(path.to_path_buf()),
    }
}

/// Paths that are invariant throughout the traversal when copying a directory.
struct Context<'a> {
    /// The current working directory at the time of starting the traversal.
    current_dir: PathBuf,

    /// The path to the parent of the source directory, if any.
    root_parent: Option<PathBuf>,

    /// The target path to which the directory will be copied.
    target: &'a Path,

    /// The source path from which the directory will be copied.
    root: &'a Path,
}

impl<'a> Context<'a> {
    fn new(root: &'a Path, target: &'a Path) -> std::io::Result<Self> {
        let current_dir = env::current_dir()?;
        let root_path = current_dir.join(root);
        let root_is_dot = root == Path::new(".") || root == Path::new("./");
        let root_parent = if target.exists() && !ends_with_slash_dot(root) && !root_is_dot {
            root_path.parent().map(|p| p.to_path_buf())
        } else {
            Some(root_path)
        };
        Ok(Self {
            current_dir,
            root_parent,
            target,
            root,
        })
    }
}

/// Data needed to perform a single copy operation while traversing a directory.
///
/// For convenience while traversing a directory, the [`Entry::new`]
/// function allows creating an entry from a [`Context`] and a
/// [`walkdir::DirEntry`].
///
/// # Examples
///
/// For example, if the source directory structure is `a/b/c`, the
/// target is `d/`, a directory that already exists, and the copy
/// command is `cp -r a/b/c d`, then the overall set of copy
/// operations could be represented as three entries,
///
/// ```rust,ignore
/// let operations = [
///     Entry {
///         source_absolute: "/tmp/a".into(),
///         source_relative: "a".into(),
///         local_to_target: "d/a".into(),
///         target_is_file: false,
///     }
///     Entry {
///         source_absolute: "/tmp/a/b".into(),
///         source_relative: "a/b".into(),
///         local_to_target: "d/a/b".into(),
///         target_is_file: false,
///     }
///     Entry {
///         source_absolute: "/tmp/a/b/c".into(),
///         source_relative: "a/b/c".into(),
///         local_to_target: "d/a/b/c".into(),
///         target_is_file: false,
///     }
/// ];
/// ```
struct Entry {
    /// The absolute path to file or directory to copy.
    source_absolute: PathBuf,

    /// The relative path to file or directory to copy.
    source_relative: PathBuf,

    /// The path to the destination, relative to the target.
    local_to_target: PathBuf,

    /// Whether the destination is a file.
    target_is_file: bool,

    /// Depth of the direntry from the walkdir traversal root.
    depth: usize,
}

impl Entry {
    fn new(
        context: &Context,
        direntry: &DirEntry,
        no_target_dir: bool,
    ) -> Result<Self, StripPrefixError> {
        let source_relative = direntry.path().to_path_buf();
        let source_absolute = context.current_dir.join(&source_relative);
        let mut descendant =
            get_local_to_root_parent(&source_absolute, context.root_parent.as_deref())?;
        if no_target_dir {
            let source_is_dir = direntry.path().is_dir();
            if path_ends_with_terminator(context.target) && source_is_dir {
                if let Err(e) = std::fs::create_dir_all(context.target) {
                    eprintln!("Failed to create directory: {e}");
                }
            } else {
                descendant = descendant.strip_prefix(context.root)?.to_path_buf();
            }
        }

        let local_to_target = context.target.join(descendant);
        let target_is_file = context.target.is_file();
        let depth = direntry.depth();
        Ok(Self {
            source_absolute,
            source_relative,
            local_to_target,
            target_is_file,
            depth,
        })
    }
}

/// Decide whether the given path ends with `/.`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(ends_with_slash_dot("/."));
/// assert!(ends_with_slash_dot("./."));
/// assert!(ends_with_slash_dot("a/."));
///
/// assert!(!ends_with_slash_dot("."));
/// assert!(!ends_with_slash_dot("./"));
/// assert!(!ends_with_slash_dot("a/.."));
/// ```
fn ends_with_slash_dot<P>(path: P) -> bool
where
    P: AsRef<Path>,
{
    // `path.ends_with(".")` does not seem to work
    path.as_ref().display().to_string().ends_with("/.")
}

#[cfg(unix)]
fn directory_creation_modes(source_path: &Path, options: &CpOptions) -> CopyResult<(u32, u32)> {
    use crate::CpPreserve;
    use std::os::unix::fs::PermissionsExt;

    let source_mode = fs::metadata(source_path)?.permissions().mode() & 0o7777;
    let (preserve_mode, explicit_no_preserve_mode) = options.cp_preserve_mode();
    let final_mode = if preserve_mode {
        source_mode
    } else if explicit_no_preserve_mode {
        0o777 & !ctcore::ct_mode::get_umask()
    } else {
        source_mode & !ctcore::ct_mode::get_umask()
    };

    let temporary_mode = if matches!(options.attributes.ownership, CpPreserve::Yes { .. }) {
        0o700
    } else {
        final_mode | 0o700
    };
    Ok((temporary_mode, final_mode))
}

/// Copy a single entry during a directory traversal.
fn copy_direntry(
    progress_bar: &Option<ProgressBar>,
    entry: Entry,
    options: &CpOptions,
    symlinked_files: &mut HashSet<CtFileInformation>,
    preserve_hard_links: bool,
    copied_files: &mut HashMap<CtFileInformation, PathBuf>,
    source_in_command_line: bool,
) -> CopyResult<()> {
    let Entry {
        source_absolute,
        source_relative,
        local_to_target,
        target_is_file,
        depth,
    } = entry;

    let dereference = if depth == 0 {
        options.cp_dereference(source_in_command_line)
    } else {
        options.dereference
    };

    // If the source is a symbolic link and the options tell us not to
    // dereference the link, then copy the link object itself.
    if source_absolute.is_symlink() && !dereference {
        return copy_recursive_link(
            &source_absolute,
            &source_relative,
            &local_to_target,
            options,
            symlinked_files,
            copied_files,
        );
    }

    // If the source is a directory and the destination does not
    // exist, ...
    if source_absolute.is_dir()
        && !ends_with_slash_dot(&source_absolute)
        && !local_to_target.exists()
    {
        if target_is_file {
            return Err("cannot overwrite non-directory with directory".into());
        } else {
            fs::create_dir_all(&local_to_target)?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;

                let (temporary_mode, _) = directory_creation_modes(&source_absolute, options)?;
                fs::set_permissions(&local_to_target, fs::Permissions::from_mode(temporary_mode))?;
            }

            if options.verbose {
                cp_print_verbose_output(false, progress_bar, &source_relative, &local_to_target)?;
            }
            return Ok(());
        }
    }

    // If the source is not a directory, then we need to copy the file.
    if !source_absolute.is_dir() {
        if preserve_hard_links {
            match copy_file(
                progress_bar,
                &source_absolute,
                &source_relative,
                local_to_target.as_path(),
                options,
                symlinked_files,
                copied_files,
                false,
            ) {
                Ok(_) => Ok(()),
                Err(err) => {
                    if source_absolute.is_symlink() {
                        // silent the error with a symlink
                        // In case we do --archive, we might copy the symlink
                        // before the file itself
                        Ok(())
                    } else {
                        Err(err)
                    }
                }
            }?;
        } else {
            // At this point, `path` is just a plain old file.
            // Terminate this function immediately if there is any
            // kind of error *except* a "permission denied" error.
            //
            // TODO What other kinds of errors, if any, should
            // cause us to continue walking the directory?
            match copy_file(
                progress_bar,
                &source_absolute,
                &source_relative,
                local_to_target.as_path(),
                options,
                symlinked_files,
                copied_files,
                false,
            ) {
                Ok(_) => {}
                Err(CpError::IoErrContext(e, _))
                    if e.kind() == std::io::ErrorKind::PermissionDenied =>
                {
                    ct_show!(uio_error!(
                        e,
                        "cannot open {} for reading",
                        source_relative.quote(),
                    ));
                }
                Err(e) => return Err(e),
            }
        }
    }

    // In any other case, there is nothing to do, so we just return to
    // continue the traversal.
    Ok(())
}

/// Read the contents of the directory `root` and recursively copy the
/// contents to `target`.
///
/// Any errors encountered copying files in the tree will be logged but
/// will not cause a short-circuit.
pub(crate) fn copy_directory(
    progress_bar: &Option<ProgressBar>,
    root: &Path,
    target: &Path,
    options: &CpOptions,
    symlinked_files: &mut HashSet<CtFileInformation>,
    copied_files: &mut HashMap<CtFileInformation, PathBuf>,
    source_in_command_line: bool,
) -> CopyResult<()> {
    let mut copy_error = false;

    if !options.recursive {
        return Err(format!("-r not specified; omitting directory {}", root.quote()).into());
    }

    // if no-dereference is enabled and this is a symlink, copy it as a file
    if !options.cp_dereference(source_in_command_line) && root.is_symlink() {
        return copy_file(
            progress_bar,
            root,
            root,
            target,
            options,
            symlinked_files,
            copied_files,
            source_in_command_line,
        );
    }

    // check if root is a prefix of target
    if path_has_prefix(target, root)? {
        if !target.exists() {
            fs::create_dir_all(target)?;
        }
        return Err(format!(
            "cannot copy a directory, {}, into itself, {}",
            root.quote(),
            target.quote()
        )
        .into());
    }

    // If in `--parents` mode, create all the necessary ancestor directories.
    //
    // For example, if the command is `cp --parents a/b/c d`, that
    // means we need to copy the two ancestor directories first:
    //
    // a -> d/a
    // a/b -> d/a/b
    //
    let tmp = if options.parents {
        if let Some(parent) = root.parent() {
            let new_target = target.join(parent);
            std::fs::create_dir_all(&new_target)?;

            if options.verbose {
                // For example, if copying file `a/b/c` and its parents
                // to directory `d/`, then print
                //
                //     a -> d/a
                //     a/b -> d/a/b
                //
                for (x, y) in cp_aligned_ancestors(root, &target.join(root)) {
                    if cp_print_verbose_output(false, &None, x, y).is_err() {
                        copy_error = true;
                    }
                }
            }

            new_target
        } else {
            target.to_path_buf()
        }
    } else {
        target.to_path_buf()
    };
    let target = tmp.as_path();

    let preserve_hard_links = options.cp_preserve_hard_links();

    // Collect some paths here that are invariant during the traversal
    // of the given directory, like the current working directory and
    // the target directory.
    let context = match Context::new(root, target) {
        Ok(c) => c,
        Err(e) => return Err(format!("failed to get current directory {e}").into()),
    };

    // Capture the root metadata before WalkDir reads the directory.  Reading a
    // directory updates its atime on filesystems mounted with atime semantics,
    // while GNU cp's archive mode preserves the timestamp from before traversal.
    let root_dereference = options.cp_dereference(source_in_command_line);
    let root_metadata = if root_dereference {
        fs::metadata(root).ok()
    } else {
        fs::symlink_metadata(root).ok()
    };
    let mut dirs_to_preserve = Vec::new();
    // Traverse the contents of the directory, copying each one.
    for direntry_result in WalkDir::new(root)
        .same_file_system(options.one_file_system)
        .follow_links(options.dereference)
        .follow_root_links(options.cp_dereference(source_in_command_line))
    {
        match direntry_result {
            Ok(direntry) => {
                let entry = Entry::new(&context, &direntry, options.no_target_dir)?;
                let dereference = if entry.depth == 0 {
                    root_dereference
                } else {
                    options.dereference
                };
                let source_metadata = if entry.depth == 0 {
                    root_metadata.clone()
                } else if dereference {
                    fs::metadata(&entry.source_absolute).ok()
                } else {
                    fs::symlink_metadata(&entry.source_absolute).ok()
                };
                if source_metadata
                    .as_ref()
                    .map(|metadata| metadata.is_dir())
                    .unwrap_or(false)
                    && !ends_with_slash_dot(&entry.source_absolute)
                {
                    dirs_to_preserve.push((
                        entry.source_absolute.clone(),
                        entry.local_to_target.clone(),
                        !entry.local_to_target.exists(),
                        source_metadata,
                        dereference,
                    ));
                }
                if let Err(error) = copy_direntry(
                    progress_bar,
                    entry,
                    options,
                    symlinked_files,
                    preserve_hard_links,
                    copied_files,
                    source_in_command_line,
                ) {
                    show_error_if_needed(&error);
                    copy_error = true;
                }
            }
            // Print an error message, but continue traversing the directory.
            Err(e) => {
                ct_show_error!("{}", e);
                copy_error = true;
            }
        }
    }

    // Apply attributes to all directories we traversed in post-order (deepest first).
    // This perfectly emulates GNU cp behavior and avoids read-only parent issues.
    for (src, dest, created, source_metadata, dereference) in dirs_to_preserve.into_iter().rev() {
        if dest.exists() && dest.is_dir() {
            copy_attributes_with_deref(&src, &dest, &options.attributes, dereference)?;
            if matches!(options.attributes.timestamps, crate::CpPreserve::Yes { .. }) {
                if let Some(source_metadata) = source_metadata {
                    let destination_metadata = fs::metadata(&dest)?;
                    filetime::set_file_times(
                        &dest,
                        FileTime::from_last_access_time(&source_metadata),
                        FileTime::from_last_modification_time(&destination_metadata),
                    )?;
                }
            }
            #[cfg(unix)]
            if created
                && !dest.is_symlink()
                && !matches!(options.attributes.mode, crate::CpPreserve::Yes { .. })
            {
                use std::os::unix::fs::PermissionsExt;

                let (_, final_mode) = directory_creation_modes(&src, options)?;
                fs::set_permissions(&dest, fs::Permissions::from_mode(final_mode))?;
            }
        }
    }

    // Ancestor directories created by `--parents` were not yielded by WalkDir
    if options.parents {
        let dest = target.join(root.file_name().unwrap());
        for (x, y) in cp_aligned_ancestors(root, dest.as_path()) {
            copy_attributes(x, y, &options.attributes)?;
        }
    }

    if copy_error {
        Err(CpError::NotAllFilesCopied)
    } else {
        Ok(())
    }
}

/// Decide whether the second path is a prefix of the first.
///
/// This function canonicalizes the paths via
/// [`ctcore::ct_fs::canonicalize`] before comparing.
///
/// # Errors
///
/// If there is an error determining the canonical, absolute form of
/// either path.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(path_has_prefix(Path::new("/usr/bin"), Path::new("/usr")))
/// assert!(!path_has_prefix(Path::new("/usr"), Path::new("/usr/bin")))
/// assert!(!path_has_prefix(Path::new("/usr/bin"), Path::new("/var/log")))
/// ```
pub fn path_has_prefix(p1: &Path, p2: &Path) -> io::Result<bool> {
    let pathbuf1 = canonicalize(p1, MissingHandling::Normal, ResolveMode::Logical)?;
    let pathbuf2 = canonicalize(p2, MissingHandling::Normal, ResolveMode::Logical)?;

    Ok(pathbuf1.starts_with(pathbuf2))
}

#[cfg(test)]
mod tests {
    use super::ends_with_slash_dot;
    use super::path_has_prefix;

    #[test]
    #[allow(clippy::cognitive_complexity)]
    fn test_ends_with_slash_dot() {
        assert!(ends_with_slash_dot("/."));
        assert!(ends_with_slash_dot("./."));
        assert!(ends_with_slash_dot("../."));
        assert!(ends_with_slash_dot("a/."));
        assert!(ends_with_slash_dot("/a/."));

        assert!(!ends_with_slash_dot(""));
        assert!(!ends_with_slash_dot("."));
        assert!(!ends_with_slash_dot("./"));
        assert!(!ends_with_slash_dot(".."));
        assert!(!ends_with_slash_dot("/.."));
        assert!(!ends_with_slash_dot("a/.."));
        assert!(!ends_with_slash_dot("/a/.."));
        assert!(!ends_with_slash_dot("/"));
    }

    #[test]
    fn test_copy_directory() {}

    use std::path::PathBuf;

    #[test]
    fn test_path_has_prefix() {
        // Test case 1: p2 is a prefix of p1
        let p1 = PathBuf::from("/usr/bin");
        let p2 = PathBuf::from("/usr");
        assert!(path_has_prefix(&p1, &p2).is_ok() && path_has_prefix(&p1, &p2).unwrap());

        // Test case 2: p2 is not a prefix of p1
        let p1 = PathBuf::from("/usr");
        let p2 = PathBuf::from("/usr/bin");
        assert!(path_has_prefix(&p1, &p2).is_ok() && !path_has_prefix(&p1, &p2).unwrap());

        // Test case 3: p2 is not a prefix of p1
        let p1 = PathBuf::from("/usr/bin");
        let p2 = PathBuf::from("/var/log");
        assert!(path_has_prefix(&p1, &p2).is_ok() && !path_has_prefix(&p1, &p2).unwrap());
        assert!(path_has_prefix(&p2, &p1).is_ok() && !path_has_prefix(&p2, &p1).unwrap());
    }

    // use std::fs;
    // use std::path::Path;
    //
    // #[test]
    // fn test_entry_new() {
    //     // Create a temporary directory for testing
    //     let temp_dir = fs::create_dir_all("/tmp/test_dir").unwrap();
    //     let root = Path::new("/tmp/test_dir");
    //     let context = Context {
    //         current_dir: PathBuf::from("/tmp"),
    //         root_parent: Some(PathBuf::from("/tmp/test_dir")),
    //         target: &*PathBuf::from("/tmp/target"),
    //         root: &root,
    //     };
    //
    //     // Create a dummy DirEntry
    //     let dir_entry = Path::new("/tmp/test_dir").to_str();
    //     let direntry = fs::DirEntry::path(dir_entry);
    //
    //     // Create an Entry using the new function
    //     let entry = Entry::new(&context, &direntry, false).unwrap();
    //
    //     // Assert the values of the created Entry
    //     assert_eq!(entry.source_absolute, PathBuf::from("/tmp/test_dir/file.txt"));
    //     assert_eq!(entry.source_relative, PathBuf::from("file.txt"));
    //     assert_eq!(entry.local_to_target, PathBuf::from("/tmp/target/file.txt"));
    //     assert!(!entry.target_is_file);
    //
    //     // Clean up the temporary directory
    //     fs::remove_dir_all("/tmp/test_dir").unwrap();
    // }
}
