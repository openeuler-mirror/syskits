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
//! **ONLY** support linux for the time being
//!
//! # Examples:
//!
//! ```
//! use ctcore::ct_utmpx::CtUtmpx;
//! for ut in CtUtmpx::iter_all_records() {
//!     if ut.is_user_process() {
//!         println!("{}: {}", ut.host(), ut.user())
//!     }
//! }
//! ```
//!
//! Specifying the path to login record:
//!
//! ```
//! use ctcore::ct_utmpx::CtUtmpx;
//! for ut in CtUtmpx::iter_all_records_from("/some/where/else") {
//!     if ut.is_user_process() {
//!         println!("{}: {}", ut.host(), ut.user())
//!     }
//! }
//! ```

pub extern crate time;

use std::ffi::CString;
use std::io::Result as IOResult;
use std::marker::PhantomData;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::ptr;
use std::sync::{Mutex, MutexGuard};

pub use self::ct_ut::*;
pub use libc::endutxent;
pub use libc::getutxent;
pub use libc::setutxent;
use libc::utmpx;
#[cfg(target_os = "linux")]
pub use libc::utmpxname;

use crate::*;
// import macros from `../../ct_macros`

// In case the c_char array doesn't end with NULL
macro_rules! chars2string {
    ($arr:expr) => {
        $arr.iter()
            .take_while(|i| **i > 0)
            .map(|&i| i as u8 as char)
            .collect::<String>()
    };
}

#[cfg(target_os = "linux")]
mod ct_ut {
    pub static DEFAULT_FILE: &str = "/var/run/utmp";

    pub use libc::__UT_HOSTSIZE as UT_HOSTSIZE;
    pub use libc::__UT_LINESIZE as UT_LINESIZE;
    pub use libc::__UT_NAMESIZE as UT_NAMESIZE;
    pub const UT_IDSIZE: usize = 4;

    pub use libc::ACCOUNTING;
    pub use libc::BOOT_TIME;
    pub use libc::DEAD_PROCESS;
    pub use libc::EMPTY;
    pub use libc::INIT_PROCESS;
    pub use libc::LOGIN_PROCESS;
    pub use libc::NEW_TIME;
    pub use libc::OLD_TIME;
    pub use libc::RUN_LVL;
    pub use libc::USER_PROCESS;
}

#[cfg(target_vendor = "apple")]
mod ut {
    pub static DEFAULT_FILE: &str = "/var/run/utmpx";

    pub use libc::_UTX_HOSTSIZE as UT_HOSTSIZE;
    pub use libc::_UTX_IDSIZE as UT_IDSIZE;
    pub use libc::_UTX_LINESIZE as UT_LINESIZE;
    pub use libc::_UTX_USERSIZE as UT_NAMESIZE;

    pub use libc::ACCOUNTING;
    pub use libc::BOOT_TIME;
    pub use libc::DEAD_PROCESS;
    pub use libc::EMPTY;
    pub use libc::INIT_PROCESS;
    pub use libc::LOGIN_PROCESS;
    pub use libc::NEW_TIME;
    pub use libc::OLD_TIME;
    pub use libc::RUN_LVL;
    pub use libc::SHUTDOWN_TIME;
    pub use libc::SIGNATURE;
    pub use libc::USER_PROCESS;
}

pub struct CtUtmpx {
    inner: utmpx,
}

impl CtUtmpx {
    /// A.K.A. ut.ut_type
    pub fn record_type(&self) -> i16 {
        self.inner.ut_type
    }
    /// A.K.A. ut.ut_pid
    pub fn pid(&self) -> i32 {
        self.inner.ut_pid
    }
    /// A.K.A. ut.ut_id
    pub fn terminal_suffix(&self) -> String {
        chars2string!(self.inner.ut_id)
    }
    /// A.K.A. ut.ut_user
    pub fn user(&self) -> String {
        chars2string!(self.inner.ut_user)
    }
    /// A.K.A. ut.ut_host
    pub fn host(&self) -> String {
        chars2string!(self.inner.ut_host)
    }
    /// A.K.A. ut.ut_line
    pub fn tty_device(&self) -> String {
        chars2string!(self.inner.ut_line)
    }
    /// A.K.A. ut.ut_tv
    pub fn login_time(&self) -> time::OffsetDateTime {
        #[allow(clippy::unnecessary_cast)]
        let ts_nanos: i128 = (1_000_000_000_i64 * self.inner.ut_tv.tv_sec as i64
            + 1_000_i64 * self.inner.ut_tv.tv_usec as i64)
            .into();
        let local_offset = time::OffsetDateTime::now_local().unwrap().offset();
        time::OffsetDateTime::from_unix_timestamp_nanos(ts_nanos)
            .unwrap()
            .to_offset(local_offset)
    }
    /// A.K.A. ut.ut_exit
    ///
    /// Return (e_termination, e_exit)
    #[cfg(target_os = "linux")]
    pub fn exit_status(&self) -> (i16, i16) {
        (self.inner.ut_exit.e_termination, self.inner.ut_exit.e_exit)
    }
    /// A.K.A. ut.ut_exit
    ///
    /// Return (0, 0) on Non-Linux platform
    #[cfg(target_os = "windows")]
    pub fn exit_status(&self) -> (i16, i16) {
        (0, 0)
    }
    /// Consumes the `Utmpx`, returning the underlying C struct utmpx
    pub fn into_inner(self) -> utmpx {
        self.inner
    }
    pub fn is_user_process(&self) -> bool {
        !self.user().is_empty() && self.record_type() == USER_PROCESS
    }

    /// Canonicalize host name using DNS
    pub fn canon_host(&self) -> IOResult<String> {
        let host = self.host();

        let (hostname, display) = host.split_once(':').unwrap_or((&host, ""));

        if !hostname.is_empty() {
            use dns_lookup::{AddrInfoHints, getaddrinfo};

            const AI_CANONNAME: i32 = 0x2;
            let hints = AddrInfoHints {
                flags: AI_CANONNAME,
                ..AddrInfoHints::default()
            };
            if let Ok(sockets) = getaddrinfo(Some(hostname), None, Some(hints)) {
                let sockets = sockets.collect::<IOResult<Vec<_>>>()?;
                for socket in sockets {
                    if let Some(ai_canonname) = socket.canonname {
                        return Ok(if display.is_empty() {
                            ai_canonname
                        } else {
                            format!("{ai_canonname}:{display}")
                        });
                    }
                }
            } else {
                // GNU coreutils具有这种行为
                return Ok(hostname.to_string());
            }
        }

        Ok(host.to_string())
    }

    /// Iterate through all the utmp records.
    ///
    /// This will use the default location, or the path [`CtUtmpx::iter_all_records_from`]
    /// was most recently called with.
    ///
    /// Only one instance of [`CtUtmpxIter`] may be active at a time. This
    /// function will block as long as one is still active. Beware!
    pub fn iter_all_records() -> CtUtmpxIter {
        let iter = CtUtmpxIter::new();
        unsafe {
            // 从技术上讲，这可能会失败，检测到这一点会很好，但它什么也不返回，所以我们不得不对errno做一些讨厌的事情。
            setutxent();
        }
        iter
    }

    /// Iterate through all the utmp records from a specific file.
    ///
    /// No failure is reported or detected.
    ///
    /// This function affects subsequent calls to [`CtUtmpx::iter_all_records`].
    ///
    /// The same caveats as for [`CtUtmpx::iter_all_records`] apply.
    pub fn iter_all_records_from<P: AsRef<Path>>(path: P) -> CtUtmpxIter {
        let iter = CtUtmpxIter::new();
        let path = CString::new(path.as_ref().as_os_str().as_bytes()).unwrap();
        unsafe {
            // 在 glibc 中，utmpxname() 只有在内存不足以复制字符串时才会失败。
            // Solaris 成功时返回 1 而不是 0。据说还有一些系统返回 void。
            // 在 Debian 上的 GNU who 如果指定了无效的文件名似乎什么也不输出，没有警告或其他内容。
            // 所以这个函数非常疯狂，我们不尝试检测错误。
            // 除了祈祷外，我们无能为力。
            utmpxname(path.as_ptr());
            setutxent();
        }
        iter
    }
}

// 在某些系统上，这些函数不是线程安全的。在其他系统上，它们是线程局部的。
// 因此，我们使用互斥锁来确保一次只能存在一个guard，并确保UtmpxIter不能跨线程发送。
//
// 我认为唯一可能的技术内存不安全性是在从getutxent()返回的指针处复制数据时发生数据竞争，但普通的竞态条件也很可能发生。
static LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// Iterator of login records
pub struct CtUtmpxIter {
    #[allow(dead_code)]
    guard: MutexGuard<'static, ()>,
    /// Ensure UtmpxIter is !Send. Technically redundant because MutexGuard
    /// is also !Send.
    phantom: PhantomData<std::rc::Rc<()>>,
}

impl CtUtmpxIter {
    fn new() -> Self {
        // PoisonErrors可以安全地被忽略
        let guard = LOCK.lock().unwrap_or_else(|err| err.into_inner());
        Self {
            guard,
            phantom: PhantomData,
        }
    }
}

impl Iterator for CtUtmpxIter {
    type Item = CtUtmpx;
    fn next(&mut self) -> Option<Self::Item> {
        unsafe {
            let res = getutxent();
            if res.is_null() {
                None
            } else {
                // 此指针后面的data将在下一次调用getutxent()时被替换，所以我们现在必须读取它。
                // 所有字符串作为数组内联在结构体中，这使得事情变得更容易。
                Some(CtUtmpx {
                    inner: ptr::read(res as *const _),
                })
            }
        }
    }
}

impl Drop for CtUtmpxIter {
    fn drop(&mut self) {
        unsafe {
            endutxent();
        }
    }
}
