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

use windows_sys::Win32::Foundation::{BOOL, CloseHandle, HANDLE, WAIT_FAILED, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::OpenProcess;
use windows_sys::Win32::System::Threading::PROCESS_SYNCHRONIZE;
use windows_sys::Win32::System::Threading::WaitForSingleObject;

pub type Pid = u32;

pub struct ProcessChecker {
    dead: bool,
    handle: HANDLE,
}

impl ProcessChecker {
    pub fn new(process_id: self::Pid) -> Self {
        #[allow(non_snake_case)]
        let FALSE: BOOL = 0;
        let h = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, FALSE, process_id) };
        Self {
            dead: h == 0,
            handle: h,
        }
    }

    #[allow(clippy::wrong_self_convention)]
    pub fn is_dead(&mut self) -> bool {
        if !self.dead {
            self.dead = unsafe {
                let status = WaitForSingleObject(self.handle, 0);
                status == WAIT_OBJECT_0 || status == WAIT_FAILED
            }
        }

        self.dead
    }
}

impl Drop for ProcessChecker {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.handle);
        }
    }
}

pub fn supports_pid_checks(_pid: self::Pid) -> bool {
    true
}
