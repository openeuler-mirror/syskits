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

// spell-checker:ignore (ToDO) stdlib, ISCHR, GETFD
// spell-checker:ignore (options) EPERM, ENOSYS

use std::io::Error;

pub type Pid = libc::pid_t;

pub struct ProcessChecker {
    pid: self::Pid,
}

impl ProcessChecker {
    pub fn new(process_id: self::Pid) -> Self {
        Self { pid: process_id }
    }

    // Borrowing mutably to be aligned with Windows implementation
    #[allow(clippy::wrong_self_convention)]
    pub fn is_dead(&mut self) -> bool {
        unsafe { libc::kill(self.pid, 0) != 0 && get_errno() != libc::EPERM }
    }
}

impl Drop for ProcessChecker {
    fn drop(&mut self) {}
}

pub fn supports_pid_checks(pid: self::Pid) -> bool {
    unsafe { !(libc::kill(pid, 0) != 0 && get_errno() == libc::ENOSYS) }
}

#[inline]
fn get_errno() -> i32 {
    Error::last_os_error().raw_os_error().unwrap()
}

//pub fn stdin_is_bad_fd() -> bool {
// FIXME: Detect a closed file descriptor, e.g.: `tail <&-`
// this is never `true`, even with `<&-` because Rust's stdlib is reopening fds as /dev/null
// see also: https://github.com/ctutils/coreutils/issues/2873
// (gnu/tests/tail-2/follow-stdin.sh fails because of this)
// unsafe { libc::fcntl(fd, libc::F_GETFD) == -1 && get_errno() == libc::EBADF }
//false
//}
