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

pub fn is_wsl_1() -> bool {
    // #[cfg(target_os = "linux")]
    // {
    //     if is_wsl_2() {
    //         return false;
    //     }
    //     if let Ok(b) = std::fs::read("/proc/sys/kernel/osrelease") {
    //         if let Ok(s) = std::str::from_utf8(&b) {
    //             let a = s.to_ascii_lowercase();
    //             return a.contains("microsoft") || a.contains("wsl");
    //         }
    //     }
    // }
    // false

    #[cfg(target_os = "linux")]
    {
        if is_wsl_2() {
            return false;
        }

        match std::fs::read("/proc/sys/kernel/osrelease") {
            Ok(b) => match std::str::from_utf8(&b) {
                Ok(s) => {
                    let v = s.to_ascii_lowercase();
                    v.contains("wsl") || v.contains("microsoft")
                }
                Err(_) => false, // 处理 UTF-8 转换失败的情况
            },
            Err(_) => false, // 处理文件读取失败的情况
        }
    }
    #[cfg(not(target_os = "linux"))]
    false
}

pub fn is_wsl_2() -> bool {
    #[cfg(target_os = "linux")]
    {
        // if let Ok(b) = std::fs::read("/proc/sys/kernel/osrelease") {
        //     if let Ok(s) = std::str::from_utf8(&b) {
        //         let a = s.to_ascii_lowercase();
        //         return a.contains("wsl2");
        //     }
        // }
        match std::fs::read("/proc/sys/kernel/osrelease") {
            Ok(b) => match std::str::from_utf8(&b) {
                Ok(s) => s.to_ascii_lowercase().contains("wsl2"),
                Err(_) => false, // 处理 UTF-8 转换失败的情况
            },
            Err(_) => false, // 处理文件读取失败的情况
        }
    }

    #[cfg(not(target_os = "linux"))]
    false
}
