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

use ctcore::ct_entries::{CtPasswd, Locate};
use libc::uid_t;
use std::ffi::OsString;
use std::io;
use std::os::unix::ffi::OsStringExt;

fn whoami_geteuid() -> uid_t {
    unsafe { libc::geteuid() }
}

pub fn get_username() -> io::Result<OsString> {
    CtPasswd::locate(whoami_geteuid()).map(username_from_passwd)
}

fn username_from_passwd(entry: CtPasswd) -> OsString {
    OsString::from_vec(entry.name_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;

    #[test]
    fn username_from_passwd_preserves_non_utf8_name_bytes() {
        let entry = CtPasswd {
            name: "replacement".to_owned(),
            raw_name: Some(vec![0xff, b'u', b's', b'e', b'r']),
            ..Default::default()
        };

        assert_eq!(
            username_from_passwd(entry).as_bytes(),
            &[0xff, b'u', b's', b'e', b'r']
        );
    }
}
