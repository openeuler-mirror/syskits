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

#[cfg(unix)]
pub use self::unix::{
    Pid,
    //stdin_is_bad_fd, stdin_is_pipe_or_fifo, supports_pid_checks, Pid, ProcessChecker,
    ProcessChecker,
    supports_pid_checks,
};

#[cfg(windows)]
pub use self::windows::{Pid, ProcessChecker, supports_pid_checks};

#[cfg(unix)]
mod unix;

#[cfg(windows)]
mod windows;
