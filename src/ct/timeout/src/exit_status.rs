/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */
use ctcore::ct_error::CTError;

/// Enumerates the exit statuses produced by `timeout`.
///
/// Use [`Into::into`] (or [`From::from`]) to convert an enumeration
/// member into a numeric status code. You can also convert into a
/// [`CTError`].
///
/// # Examples
///
/// Convert into an [`i32`]:
///
/// ```rust,ignore
/// assert_eq!(i32::from(ExitStatus::CommandTimedOut), 124);
/// ```
pub(crate) enum ExitStatus {
    /// 当命令超时且未设置 --preserve-status 时返回 124
    CommandTimedOut,

    /// timeout 程序本身失败时返回 125
    TimeoutFailed,

    /// 命令找到但无法执行时返回 126
    CommandNotExecutable,

    /// 命令未找到时返回 127
    CommandNotFound,

    /// 进程被信号终止时返回 128+n，n 为信号值
    SignalTerminated(i32),
}

impl From<ExitStatus> for i32 {
    fn from(exit_status: ExitStatus) -> Self {
        match exit_status {
            ExitStatus::CommandTimedOut => 124,
            ExitStatus::TimeoutFailed => 125,
            ExitStatus::CommandNotExecutable => 126,
            ExitStatus::CommandNotFound => 127,
            ExitStatus::SignalTerminated(signal) => 128 + signal,
        }
    }
}

impl From<ExitStatus> for Box<dyn CTError> {
    fn from(exit_status: ExitStatus) -> Self {
        Box::from(i32::from(exit_status))
    }
}
