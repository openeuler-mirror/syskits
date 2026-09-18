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

use libc::uid_t;
use std::ffi::{CStr, OsString};
use std::io;
use std::os::unix::ffi::OsStringExt;

fn whoami_geteuid() -> uid_t {
    unsafe { libc::geteuid() }
}

pub fn get_username() -> io::Result<OsString> {
    clear_errno();
    let uid = whoami_geteuid();
    let passwd = unsafe { libc::getpwuid(uid) };
    if passwd.is_null() {
        return Err(getpwuid_error(uid));
    }

    // SAFETY: getpwuid returned a non-null passwd entry whose pw_name is a C string.
    Ok(unsafe { username_from_passwd_name((*passwd).pw_name) })
}

fn clear_errno() {
    unsafe {
        *libc::__errno_location() = 0;
    }
}

fn getpwuid_error(uid: uid_t) -> io::Error {
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        Some(0) | None => io::Error::new(io::ErrorKind::NotFound, format!("No such id: {uid}")),
        Some(_) => error,
    }
}

unsafe fn username_from_passwd_name(name: *const libc::c_char) -> OsString {
    OsString::from_vec(unsafe { CStr::from_ptr(name) }.to_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;

    #[test]
    fn username_from_passwd_name_preserves_non_utf8_bytes() {
        let entry = [0xff, b'u', b's', b'e', b'r', 0];

        assert_eq!(
            unsafe { username_from_passwd_name(entry.as_ptr().cast()) }.as_bytes(),
            &[0xff, b'u', b's', b'e', b'r']
        );
    }
}
