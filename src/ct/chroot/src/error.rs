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
use ctcore::ct_display::Quotable;
use ctcore::ct_error::CTError;
use std::fmt::Display;
use std::io::Error;

/// Errors that can happen while executing chroot.
#[derive(Debug)]
pub enum ChrootError {
    /// Failed to enter the specified directory.
    CannotEnter(String, Error),

    /// Failed to execute the specified command.
    CommandFailed(String, Error),

    /// Failed to find the specified command.
    CommandNotFound(String, Error),

    /// The given user and group specification was invalid.
    InvalidUserspec(String),

    /// The new root directory was not given.
    MissingNewRoot,

    /// Failed to find the specified group.
    NoSuchGroup(String),

    /// The given directory does not exist.
    NoSuchDirectory(String),

    /// The call to `setgid()` failed.
    SetGidFailed(String, Error),

    /// The call to `setgroups()` failed.
    SetGroupsFailed(Error),

    /// The call to `setuid()` failed.
    SetUserFailed(String, Error),
}

impl std::error::Error for ChrootError {}

impl CTError for ChrootError {
    // 125：如果chroot操作本身失败
    // 126：若命令已找到但无法执行
    // 127：若命令无法找到

    fn code(&self) -> i32 {
        if let Self::CommandFailed(..) = self {
            126
        } else if let Self::CommandNotFound(..) = self {
            127
        } else {
            125
        }
    }
}

impl Display for ChrootError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::CannotEnter(s, e) => write!(f, "cannot chroot to {}: {}", s.quote(), e,),
            Self::CommandFailed(s, e) | Self::CommandNotFound(s, e) => {
                write!(f, "failed to run command {}: {}", s.to_string().quote(), e,)
            }
            Self::InvalidUserspec(s) => write!(f, "invalid userspec: {}", s.quote(),),
            Self::MissingNewRoot => write!(
                f,
                "Missing operand: NEWROOT\nTry '{} --help' for more information.",
                ctcore::ct_execute_phrase(),
            ),
            Self::NoSuchGroup(s) => write!(f, "no such group: {}", s.maybe_quote(),),
            Self::NoSuchDirectory(s) => write!(
                f,
                "cannot change root directory to {}: no such directory",
                s.quote(),
            ),
            Self::SetGidFailed(s, e) => write!(f, "cannot set gid to {s}: {e}"),
            Self::SetGroupsFailed(e) => write!(f, "cannot set groups: {e}"),
            Self::SetUserFailed(s, e) => {
                write!(f, "cannot set user to {}: {}", s.maybe_quote(), e)
            }
        }
    }
}

