/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

use crate::text;
use ctcore::ct_error::CTResult;
use std::ffi::OsStr;
use std::fs::{File, Metadata};
use std::io::{Seek, SeekFrom};
#[cfg(unix)]
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub enum TailInputKind {
    File(PathBuf),
    Stdin,
}

#[cfg(unix)]
impl From<&OsStr> for TailInputKind {
    fn from(value: &OsStr) -> Self {
        if value == OsStr::new("-") {
            Self::Stdin
        } else {
            Self::File(PathBuf::from(value))
        }
    }
}

#[cfg(not(unix))]
impl From<&OsStr> for TailInputKind {
    fn from(value: &OsStr) -> Self {
        if value == OsStr::new(text::TAIL_DASH) {
            Self::Stdin
        } else {
            Self::File(PathBuf::from(value))
        }
    }
}

#[derive(Debug, Clone)]
pub struct TailInput {
    kind: TailInputKind,
    pub display_name: String,
}

impl TailInput {
    pub fn from<T: AsRef<OsStr>>(string: T) -> Self {
        let string = string.as_ref();

        let kind = string.into();
        let display_name = match kind {
            TailInputKind::File(_) => string.to_string_lossy().to_string(),
            TailInputKind::Stdin => text::TAIL_STDIN_HEADER.to_string(),
        };

        Self { kind, display_name }
    }

    pub fn kind(&self) -> &TailInputKind {
        &self.kind
    }

    pub fn is_stdin(&self) -> bool {
        match self.kind {
            TailInputKind::File(_) => false,
            TailInputKind::Stdin => true,
        }
    }

    pub fn resolve(&self) -> Option<PathBuf> {
        match &self.kind {
            TailInputKind::File(path) if path != &PathBuf::from(text::TAIL_DEV_STDIN) => {
                path.canonicalize().ok()
            }
            TailInputKind::File(_) | TailInputKind::Stdin => {
                if cfg!(unix) {
                    match PathBuf::from(text::TAIL_DEV_STDIN).canonicalize().ok() {
                        Some(path) if path != PathBuf::from(text::TAIL_FD0) => Some(path),
                        Some(_) | None => None,
                    }
                } else {
                    None
                }
            }
        }
    }

    pub fn is_tailable(&self) -> bool {
        match &self.kind {
            TailInputKind::File(path) => path_is_tailable(path),
            TailInputKind::Stdin => self.resolve().map_or(false, |path| path_is_tailable(&path)),
        }
    }
}

impl Default for TailInput {
    fn default() -> Self {
        Self {
            kind: TailInputKind::Stdin,
            display_name: String::from(text::TAIL_STDIN_HEADER),
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TailHeaderPrinter {
    verbose: bool,
    first_header: bool,
}

impl TailHeaderPrinter {
    pub fn new(verbose: bool, first_header: bool) -> Self {
        Self {
            verbose,
            first_header,
        }
    }

    pub fn print_input(&mut self, input: &TailInput) {
        self.print(input.display_name.as_str());
    }

    pub fn print(&mut self, string: &str) {
        if self.verbose {
            println!(
                "{}==> {} <==",
                if self.first_header { "" } else { "\n" },
                string,
            );
            self.first_header = false;
        }
    }
}
pub trait TailFileExtTail {
    #[allow(clippy::wrong_self_convention)]
    fn is_seekable(&mut self, current_offset: u64) -> bool;
}

impl TailFileExtTail for File {
    /// Test if File is seekable.
    /// Set the current position offset to `current_offset`.
    fn is_seekable(&mut self, current_offset: u64) -> bool {
        self.stream_position().is_ok()
            && self.seek(SeekFrom::End(0)).is_ok()
            && self.seek(SeekFrom::Start(current_offset)).is_ok()
    }
}

pub trait TailMetadataExt {
    fn is_tailable(&self) -> bool;
    fn got_truncated(&self, other: &Metadata) -> CTResult<bool>;
    fn get_block_size(&self) -> u64;
    fn file_id_eq(&self, other: &Metadata) -> bool;
}

impl TailMetadataExt for Metadata {
    fn is_tailable(&self) -> bool {
        let ft = self.file_type();
        #[cfg(unix)]
        {
            ft.is_file() || ft.is_char_device() || ft.is_fifo()
        }
        #[cfg(not(unix))]
        {
            ft.is_file()
        }
    }

    /// Return true if the file was modified and is now shorter
    fn got_truncated(&self, other: &Metadata) -> CTResult<bool> {
        Ok(other.len() < self.len() && other.modified()? != self.modified()?)
    }

    fn get_block_size(&self) -> u64 {
        #[cfg(unix)]
        {
            self.blocks()
        }
        #[cfg(not(unix))]
        {
            self.len()
        }
    }

    fn file_id_eq(&self, _other: &Metadata) -> bool {
        #[cfg(unix)]
        {
            self.ino().eq(&_other.ino())
        }
        #[cfg(windows)]
        {
            // TODO: `file_index` requires unstable library feature `windows_by_handle`
            // use std::os::windows::prelude::*;
            // if let Some(self_id) = self.file_index() {
            //     if let Some(other_id) = other.file_index() {
            //     // TODO: not sure this is the equivalent of comparing inode numbers
            //
            //         return self_id.eq(&other_id);
            //     }
            // }
            false
        }
    }
}

pub trait TailPathExt {
    fn is_stdin(&self) -> bool;
    fn is_orphan(&self) -> bool;
    fn is_tailable(&self) -> bool;
}

impl TailPathExt for Path {
    fn is_stdin(&self) -> bool {
        self.eq(Self::new(text::TAIL_DASH))
            || self.eq(Self::new(text::TAIL_DEV_STDIN))
            || self.eq(Self::new(text::TAIL_STDIN_HEADER))
    }

    /// Return true if `path` does not have an existing parent directory
    fn is_orphan(&self) -> bool {
        !matches!(self.parent(), Some(parent) if parent.is_dir())
    }

    /// Return true if `path` is is a file type that can be tailed
    fn is_tailable(&self) -> bool {
        path_is_tailable(self)
    }
}

pub fn path_is_tailable(path: &Path) -> bool {
    path.is_file() || path.exists() && path.metadata().map_or(false, |meta| meta.is_tailable())
}

#[inline]
pub fn tail_stdin_is_bad_fd() -> bool {
    // FIXME : Rust's stdlib is reopening fds as /dev/null
    // see also: https://github.com/ctutils/coreutils/issues/2873
    // (gnu/tests/tail-2/follow-stdin.sh fails because of this)
    //#[cfg(unix)]
    {
        //platform::stdin_is_bad_fd()
    }
    //#[cfg(not(unix))]
    false
}
