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
use std::ffi::OsString;
use std::fmt::Write;
use std::io;

use ctcore::ct_display::Quotable;

pub(crate) type Result<T> = std::result::Result<T, Error>;

#[derive(thiserror::Error, Debug)]
pub(crate) enum Error {
    #[error("No context is specified")]
    MissingContext,

    #[error("No files are specified")]
    MissingFiles,

    #[error("Data is out of range")]
    OutOfRange,

    #[error("{0}")]
    ArgumentsMismatch(String),

    #[error(transparent)]
    CommandLine(#[from] clap::Error),

    #[error("{operation} failed")]
    SELinux {
        operation: &'static str,
        source: selinux::errors::Error,
    },

    #[error("{operation} failed")]
    Io {
        operation: &'static str,
        source: io::Error,
    },

    #[error("{operation} failed on {}", .operand1.quote())]
    Io1 {
        operation: &'static str,
        operand1: OsString,
        source: io::Error,
    },
}

impl Error {
    pub(crate) fn from_io(operation: &'static str, source: io::Error) -> Self {
        Self::Io { operation, source }
    }

    pub(crate) fn from_io1(
        operation: &'static str,
        operand1: impl Into<OsString>,
        source: io::Error,
    ) -> Self {
        Self::Io1 {
            operation,
            operand1: operand1.into(),
            source,
        }
    }

    pub(crate) fn from_selinux(operation: &'static str, source: selinux::errors::Error) -> Self {
        Self::SELinux { operation, source }
    }
}

pub(crate) fn report_full_error(mut err: &dyn std::error::Error) -> String {
    let mut desc = String::with_capacity(256);
    write!(desc, "{err}").unwrap();
    while let Some(source) = err.source() {
        err = source;
        write!(desc, ". {err}").unwrap();
    }
    desc
}

