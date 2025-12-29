/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO NON-INFRINGEMENT,
 * MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

use ctcore::ct_crash;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTError, CTResult};
use ctcore::ct_wide::{CtFromWide, CtToWide};
use nix::errno::Errno;
use nix::fcntl::{open, OFlag};
use nix::sys::stat::Mode;
use std::fs::OpenOptions;
#[cfg(target_os = "windows")]
use std::os::windows::prelude::*;
use std::path::Path;

#[cfg(windows)]
use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_NO_MORE_FILES, HANDLE, INVALID_HANDLE_VALUE, MAX_PATH,
};
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    FindFirstVolumeW, FindNextVolumeW, FindVolumeClose, FlushFileBuffers, GetDriveTypeW,
};
#[cfg(windows)]
use windows_sys::Win32::System::WindowsProgramming::DRIVE_FIXED;

#[cfg(windows)]
unsafe fn flush_volume(name: &str) {
    let name_wide = name.to_wide_null();
    if GetDriveTypeW(name_wide.as_ptr()) == DRIVE_FIXED {
        let sliced_name = &name[..name.len() - 1]; // 去掉末尾的反斜杠
        match OpenOptions::new().write(true).open(sliced_name) {
            Ok(file) => {
                if FlushFileBuffers(file.as_raw_handle() as HANDLE) == 0 {
                    ct_crash!(GetLastError() as i32, "failed to flush file buffer");
                }
            }
            Err(e) => ct_crash!(
                e.raw_os_error().unwrap_or(1),
                "failed to create volume handle"
            ),
        }
    }
}

#[cfg(windows)]
unsafe fn find_first_volume() -> (String, HANDLE) {
    let mut name: [u16; MAX_PATH as usize] = [0; MAX_PATH as usize];
    let handle = FindFirstVolumeW(name.as_mut_ptr(), name.len() as u32);
    if handle == INVALID_HANDLE_VALUE {
        ct_crash!(GetLastError() as i32, "failed to find first volume");
    }
    (String::from_wide_null(&name), handle)
}

#[cfg(windows)]
unsafe fn find_all_volumes() -> Vec<String> {
    let (first_volume, next_volume_handle) = find_first_volume();
    let mut volumes = vec![first_volume];
    loop {
        let mut name: [u16; MAX_PATH as usize] = [0; MAX_PATH as usize];
        match FindNextVolumeW(next_volume_handle, name.as_mut_ptr(), name.len() as u32) {
            0 => match GetLastError() {
                ERROR_NO_MORE_FILES => {
                    FindVolumeClose(next_volume_handle);
                    return volumes;
                }
                err => ct_crash!(err as i32, "failed to find next volume"),
            },
            _ => {
                volumes.push(String::from_wide_null(&name));
            }
        }
    }
}

#[cfg(windows)]
pub unsafe fn do_sync() -> isize {
    let volumes = find_all_volumes();
    for volume in &volumes {
        flush_volume(volume);
    }
    0
}

#[cfg(windows)]
pub unsafe fn do_syncfs(files_vec: Vec<String>) -> isize {
    for p in files_vec {
        flush_volume(
            Path::new(&p)
                .components()
                .next()
                .unwrap()
                .as_os_str()
                .to_str()
                .unwrap(),
        );
    }
    0
}

#[cfg(windows)]
pub fn check_files(f: &String) -> CTResult<()> {
    let err_message = format!("error opening {}: No such file or directory", f.quote());
    if !Path::new(&f).exists() {
        return Err(CtSimpleError::new(1, err_message));
    }
    Ok(())
}
