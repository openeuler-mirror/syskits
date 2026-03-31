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

//! Set of functions to manage IDs

use libc::{gid_t, pid_t, uid_t};
use std::io;
use std::process::Child;
use std::process::ExitStatus;
use std::thread;
use std::time::{Duration, Instant};

// 安全性：这些函数总是成功并返回简单的整数。

/// `geteuid()` returns the effective user ID of the calling process.
pub fn geteuid() -> uid_t {
    unsafe { libc::geteuid() }
}

/// `getegid()` returns the effective group ID of the calling process.
pub fn getegid() -> gid_t {
    unsafe { libc::getegid() }
}

/// `getgid()` returns the real group ID of the calling process.
pub fn getgid() -> gid_t {
    unsafe { libc::getgid() }
}

/// `getuid()` returns the real user ID of the calling process.
pub fn getuid() -> uid_t {
    unsafe { libc::getuid() }
}

/// Missing methods for Child objects
pub trait CtChildExt {
    /// Send a signal to a Child process.
    ///
    /// Caller beware: if the process already exited then you may accidentally
    /// send the signal to an unrelated process that recycled the PID.
    fn send_signal(&mut self, signal: usize) -> io::Result<()>;

    /// Send a signal to a process group.
    fn send_signal_group(&mut self, signal: usize) -> io::Result<()>;

    /// Wait for a process to finish or return after the specified duration.
    /// A `timeout` of zero disables the timeout.
    fn wait_or_timeout(&mut self, timeout: Duration) -> io::Result<Option<ExitStatus>>;
}

impl CtChildExt for Child {
    fn send_signal(&mut self, signal: usize) -> io::Result<()> {
        if unsafe { libc::kill(self.id() as pid_t, signal as i32) } == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    fn send_signal_group(&mut self, signal: usize) -> io::Result<()> {
        let ignore_signal = unsafe { libc::signal(signal as libc::c_int, libc::SIG_IGN) };
        if ignore_signal == libc::SIG_ERR {
            return Err(io::Error::last_os_error());
        }

        let result = unsafe { libc::kill(0, signal as libc::c_int) };
        match result {
            0 => Ok(()),
            _ => Err(io::Error::last_os_error()),
        }
    }

    fn wait_or_timeout(&mut self, timeout: Duration) -> io::Result<Option<ExitStatus>> {
        if timeout == Duration::from_micros(0) {
            return self.wait().map(Some);
        }
        // .try_wait()不会放弃stdin，所以我们手动放弃
        drop(self.stdin.take());

        let start = Instant::now();
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(Some(status));
            }

            if start.elapsed() >= timeout {
                break;
            }

            // XXX: 这有点恶心，但它比只是为了等待而启动一个线程（这是之前的解决方案）更干净。
            // 我们可能也想在这里使用不同的持续时间
            thread::sleep(Duration::from_millis(100));
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::time::Duration;
    #[test]
    fn test_get_uid_and_gid_functions() {
        // 我们不能直接模拟这些系统调用，但我们可以确保它们是可调用的。
        assert_ne!(getegid(), 1);
        assert_ne!(getgid(), 1);
        assert_ne!(getuid(), 1);
    }

    #[test]
    fn test_geteuid() {
        let euid = geteuid(); // 假设第一个元素是您关心的 `u32` 类型
        assert_ne!(euid, 1);
    }

    #[test]
    fn test_getegid() {
        let egid = getegid();
        assert_ne!(egid, 1);
    }

    #[test]
    fn test_getgid() {
        let gid = getgid();
        assert_ne!(gid, 1);
    }

    #[test]
    fn test_getuid() {
        let uid = getuid();
        assert_ne!(uid, 1);
    }
    // 这里会kill进程，影响测试，仅单元测试时打开
    // #[test]
    // fn test_send_signal() {
    //     let mut child = Command::new("sleep").arg("10").spawn().unwrap();
    //     let pid = child.id();
    //
    //     // Send SIGINT to the child process
    //     assert!(child.send_signal(libc::SIGINT as usize).is_ok());
    //
    //     // Check if the child process still exists
    //     assert!(unsafe { libc::kill(pid as pid_t, 0) } == 0);
    // }
    //
    // #[test]
    // fn test_send_signal_group() {
    //     let mut child = Command::new("sleep").arg("10").spawn().unwrap();
    //
    //     // Send SIGINT to the process group
    //     assert!(child.send_signal_group(libc::SIGINT as usize).is_ok());
    //
    //     // Wait for a short duration to ensure the process group receives the signal
    //     thread::sleep(Duration::from_secs(1));
    //
    //     // Check if the child process still exists
    //     assert!(unsafe { libc::kill(0, 0) } == 0);
    // }
    #[test]
    fn test_wait_or_timeout() {
        let mut child = Command::new("sleep").arg("5").spawn().unwrap();
        let start = Instant::now();

        // Wait for the child process to finish or timeout after 2 seconds
        let result = child.wait_or_timeout(Duration::from_secs(2));
        assert!(result.is_ok());

        // Ensure the function returns None due to timeout
        assert_eq!(result.unwrap(), None);

        // Ensure the elapsed time is greater than or equal to the timeout duration
        assert!(start.elapsed() >= Duration::from_secs(2));
    }
}
