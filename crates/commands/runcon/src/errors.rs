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

//! runcon 命令的错误处理模块
//!
//! # 功能概述
//! 该模块提供了 runcon 命令的错误类型定义和错误处理功能。
//!
//! # 主要组件
//! - `DefaultError`: 基础错误类型
//! - `RunconError`: 包装了错误码的错误类型
//! - `error_exit_status`: 错误退出状态码常量
//!
//! # 错误处理流程
//! 1. 底层错误被封装为 `DefaultError`
//! 2. `DefaultError` 被包装为 `RunconError` 并添加错误码
//! 3. 最终转换为 `CTError` 返回给用户

use std::ffi::OsString;
use std::fmt::{Display, Formatter, Write};
use std::io;
use std::str::Utf8Error;

use ctcore::ct_display::Quotable;
use ctcore::ct_error::CTError;

/// runcon 命令的结果类型
pub(crate) type Result<T> = std::result::Result<T, DefaultError>;

/// 错误退出状态码
///
/// 注意：这个列表并不完整。当命令通过 `execvp()` 执行其他程序时，
/// 进程的退出状态将是该程序的退出状态。
pub(crate) mod error_exit_status {
    /// 命令未找到
    pub const RUNCON_NOT_FOUND: i32 = 127;
    /// 无法执行命令
    pub const RUNCON_COULD_NOT_EXECUTE: i32 = 126;
    /// 其他错误
    pub const RUNCON_ANOTHER_ERROR: i32 = libc::EXIT_FAILURE;
}

/// runcon 命令的基础错误类型
#[derive(thiserror::Error, Debug)]
pub(crate) enum DefaultError {
    /// 未指定要执行的命令
    #[error("No command is specified")]
    MissingCommand,

    /// SELinux 未启用
    #[error("runcon may be used only on a SELinux kernel")]
    SELinuxNotEnabled,

    /// UTF-8 转换错误
    #[error(transparent)]
    NotUTF8(#[from] Utf8Error),

    /// 命令行参数解析错误
    #[error(transparent)]
    CommandLine(#[from] clap::Error),

    /// SELinux 无效上下文
    #[error("Invalid security context: {}", operand1.quote())]
    InvalidSecurityContext {
        /// 操作对象
        operand1: OsString,
        /// 错误来源
        source: io::Error,
    },

    /// SELinux 操作错误
    #[error("{operation} failed")]
    SELinux {
        /// 操作描述
        operation: &'static str,
        /// 错误来源
        source: selinux::errors::Error,
    },

    /// IO 操作错误
    #[error("{operation} failed")]
    Io {
        /// 操作描述
        operation: &'static str,
        /// 错误来源
        source: io::Error,
    },

    /// 带操作对象的 IO 错误
    #[error("{operation} failed on {}", .operand1.quote())]
    Io1 {
        /// 操作描述
        operation: &'static str,
        /// 操作对象
        operand1: OsString,
        /// 错误来源
        source: io::Error,
    },
}

impl DefaultError {
    /// 创建 IO 错误
    pub(crate) fn from_io(operation: &'static str, source: io::Error) -> Self {
        Self::Io { operation, source }
    }

    /// 创建带操作对象的 IO 错误
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

    /// 创建 InvalidSecurityContext 错误
    pub(crate) fn from_invalid_security_context(operand1: impl Into<OsString>, source: io::Error) -> Self {
        Self::InvalidSecurityContext { operand1: operand1.into(), source }
    }

    /// 创建 SELinux 错误
    pub(crate) fn from_selinux(operation: &'static str, source: selinux::errors::Error) -> Self {
        Self::SELinux { operation, source }
    }
}

/// 写入完整的错误信息，包括错误链
///
/// # 参数
/// * `writer` - 写入目标
/// * `err` - 错误对象
///
/// # 返回值
/// 写入成功返回 Ok(())，失败返回格式化错误
pub(crate) fn write_full_error<W>(writer: &mut W, err: &dyn std::error::Error) -> std::fmt::Result
where
    W: Write,
{
    write!(writer, "{err}")?;
    let mut err = err;
    while let Some(source) = err.source() {
        err = source;
        write!(writer, ": {err}")?;
    }
    Ok(())
}

/// runcon 命令的错误类型
///
/// 包装了基础错误和错误码
#[derive(Debug)]
pub(crate) struct RunconError {
    /// 内部错误
    inner: DefaultError,
    /// 错误码
    code: i32,
}

impl RunconError {
    /// 使用默认错误码创建错误
    pub(crate) fn new(e: DefaultError) -> Self {
        Self::with_code(error_exit_status::RUNCON_ANOTHER_ERROR, e)
    }

    /// 使用指定错误码创建错误
    pub(crate) fn with_code(code: i32, e: DefaultError) -> Self {
        Self { inner: e, code }
    }
}

impl std::error::Error for RunconError {}

impl CTError for RunconError {
    fn code(&self) -> i32 {
        self.code
    }
}

impl Display for RunconError {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        write_full_error(f, &self.inner)
    }
}
