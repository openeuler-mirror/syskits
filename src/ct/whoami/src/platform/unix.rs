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

use ctcore::ct_entries::uid2usr;
use libc::uid_t;
use std::ffi::OsString;
use std::io;

fn whoami_geteuid() -> uid_t {
    unsafe { libc::geteuid() }
}

pub fn get_username() -> io::Result<OsString> {
    // uid2usr 应该返回一个 OsString，但目前没有返回
    uid2usr(whoami_geteuid()).map(Into::into)
}
