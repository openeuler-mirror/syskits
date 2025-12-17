/*
 *    Copyright(c) 2022-2024 China Telecom Cloud Technologies co., Ltd. All rights reserved
 *     syskits is licensed under Mulan PSL v2.
 *    You can use this software according to the terms and conditions of the Mulan PSL V2
 *    You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 *    THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 *    KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 *    NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 *    See the Mulan PSL v2 for more details.
 *
 */

use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::OsStringExt;

use windows_sys::Win32::NetworkManagement::NetManagement::UNLEN;
use windows_sys::Win32::System::WindowsProgramming::GetUserNameW;

pub fn get_username() -> io::Result<OsString> {
    const CT_BUF_LEN: u32 = UNLEN + 1;
    let mut buffer = [0_u16; CT_BUF_LEN as usize];
    let mut len = CT_BUF_LEN;
    // SAFETY: buffer.len() == len
    if unsafe { GetUserNameW(buffer.as_mut_ptr(), &mut len) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(OsString::from_wide(&buffer[..len as usize - 1]))
}
