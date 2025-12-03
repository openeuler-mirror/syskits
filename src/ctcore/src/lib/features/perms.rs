/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved
 *   syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

//! Common functions to manage permissions

// spell-checker:ignore (jargon) TOCTOU

use crate::ct_display::Quotable;
use crate::ct_error::{strip_errno, UResult, USimpleError};
pub use crate::features::entries;
use crate::show_error;
use clap::{Arg, ArgMatches, Command};
use libc::{gid_t, uid_t};
use walkdir::WalkDir;

use std::io::Error as IOError;
use std::io::Result as IOResult;

use std::ffi::CString;
use std::fs::Metadata;
use std::os::unix::fs::MetadataExt;

use std::os::unix::ffi::OsStrExt;
use std::path::{Path, MAIN_SEPARATOR_STR};

/// The various level of verbosity
#[derive(PartialEq, Eq, Clone, Debug)]
pub enum VerbosityLevel {
    Silent,
    Changes,
    Verbose,
    Normal,
}
#[derive(PartialEq, Eq, Clone, Debug)]
pub struct Verbosity {
    pub groups_only: bool,
    pub level: VerbosityLevel,
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
            VerbosityLevel::Silent => (),
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
                if level == VerbosityLevel::Verbose {
                    out = if verbosity.groups_only {
                        let gid = meta.gid();
                        format!(
                            "{}\nfailed to change group of {} from {} to {}",
                            out,
                            path.quote(),
                            entries::gid2grp(gid).unwrap_or_else(|_| gid.to_string()),
                            entries::gid2grp(dest_gid).unwrap_or_else(|_| dest_gid.to_string())
                        )
                    } else {
                        let uid = meta.uid();
                        let gid = meta.gid();
                        format!(
                            "{}\nfailed to change ownership of {} from {}:{} to {}:{}",
                            out,
                            path.quote(),
                            entries::uid2usr(uid).unwrap_or_else(|_| uid.to_string()),
                            entries::gid2grp(gid).unwrap_or_else(|_| gid.to_string()),
                            entries::uid2usr(dest_uid).unwrap_or_else(|_| dest_uid.to_string()),
                            entries::gid2grp(dest_gid).unwrap_or_else(|_| dest_gid.to_string())
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
                VerbosityLevel::Changes | VerbosityLevel::Verbose => {
                    let gid = meta.gid();
                    out = if verbosity.groups_only {
                        format!(
                            "changed group of {} from {} to {}",
                            path.quote(),
                            entries::gid2grp(gid).unwrap_or_else(|_| gid.to_string()),
                            entries::gid2grp(dest_gid).unwrap_or_else(|_| dest_gid.to_string())
                        )
                    } else {
                        let gid = meta.gid();
                        let uid = meta.uid();
                        format!(
                            "changed ownership of {} from {}:{} to {}:{}",
                            path.quote(),
                            entries::uid2usr(uid).unwrap_or_else(|_| uid.to_string()),
                            entries::gid2grp(gid).unwrap_or_else(|_| gid.to_string()),
                            entries::uid2usr(dest_uid).unwrap_or_else(|_| dest_uid.to_string()),
                            entries::gid2grp(dest_gid).unwrap_or_else(|_| dest_gid.to_string())
                        )
                    };
                }
                _ => (),
            };
        } else if verbosity.level == VerbosityLevel::Verbose {
            out = if verbosity.groups_only {
                format!(
                    "group of {} retained as {}",
                    path.quote(),
                    entries::gid2grp(dest_gid).unwrap_or_default()
                )
            } else {
                format!(
                    "ownership of {} retained as {}:{}",
                    path.quote(),
                    entries::uid2usr(dest_uid).unwrap_or_else(|_| dest_uid.to_string()),
                    entries::gid2grp(dest_gid).unwrap_or_else(|_| dest_gid.to_string())
                )
            };
        }
    }
    Ok(out)
}

pub enum IfFrom {
    All,
    User(u32),
    Group(u32),
    UserGroup(u32, u32),
}

#[derive(PartialEq, Eq)]
pub enum TraverseSymlinks {
    None,
    First,
    All,
}

pub struct ChownExecutor {
    pub dest_uid: Option<u32>,
    pub dest_gid: Option<u32>,
    pub raw_owner: String, // The owner of the file as input by the user in the command line.
    pub traverse_symlinks: TraverseSymlinks,
    pub verbosity: Verbosity,
    pub filter: IfFrom,
    pub files: Vec<String>,
    pub recursive: bool,
    pub preserve_root: bool,
    pub dereference: bool,
}

