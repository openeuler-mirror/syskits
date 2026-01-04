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

#[cfg(windows)]
extern "C" {
    fn GetTickCount() -> ctcore::libc::uint32_t;
}

#[cfg(windows)]
pub fn print_loadavg() -> String {
    // XXX: currently this is a noop as Windows does not seem to have anything comparable to
    //      getloadavg()
    String::new()
}

#[cfg(windows)]
pub fn process_utmpx() -> (Option<time_t>, usize) {
    (None, 0) // TODO: change 0 to number of users
}

#[cfg(windows)]
pub fn get_uptime(_boot_time: Option<time_t>) -> i64 {
    unsafe { GetTickCount() as i64 }
}
