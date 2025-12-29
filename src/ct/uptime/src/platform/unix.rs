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

use std::path::Path;

use chrono::Local;

#[cfg(unix)]
use ctcore::libc::getloadavg;
use ctcore::libc::time_t;

#[cfg(unix)]
pub fn print_loadavg() -> String {
    use ctcore::libc::c_double;

    let mut avg: [c_double; 3] = [0.0; 3];
    let loads: i32 = unsafe { getloadavg(avg.as_mut_ptr(), 3) };

    if loads == -1 {
        String::new()
    } else {
        let mut result = "load average: ".to_string();
        for n in 0..loads {
            let separator = if n == loads - 1 { "\n" } else { ", " };
            result.push_str(&format!("{:.2}{}", avg[n as usize], separator));
        }
        result
    }
}

#[cfg(unix)]
pub fn process_utmpx() -> (Option<time_t>, usize) {
    use ctcore::ct_utmpx::*;

    let mut n_users = 0;
    let mut boot_time = None;

    for record in CtUtmpx::iter_all_records() {
        match record.record_type() {
            USER_PROCESS => n_users += 1,
            BOOT_TIME => {
                let date_time = record.login_time();
                if date_time.unix_timestamp() > 0 {
                    boot_time = Some(date_time.unix_timestamp() as time_t);
                }
            }
            _ => continue,
        }
    }
    (boot_time, n_users)
}

#[cfg(unix)]
pub fn get_uptime(boot_time: Option<time_t>) -> i64 {
    get_uptime_by_proc(boot_time, "/proc/uptime")
}

#[cfg(unix)]
fn get_uptime_by_proc<P: AsRef<Path>>(boot_time: Option<time_t>, path: P) -> i64 {
    use std::fs::File;
    use std::io::Read;

    let mut proc_uptime_s = String::new();

    let proc_uptime = File::open(path)
        .ok()
        .and_then(|mut f| f.read_to_string(&mut proc_uptime_s).ok())
        .and_then(|_| proc_uptime_s.split_whitespace().next())
        .and_then(|s| s.split('.').next().unwrap_or("0").parse().ok());

    proc_uptime.unwrap_or_else(|| match boot_time {
        Some(t) => {
            let now = Local::now().timestamp();
            #[cfg(target_pointer_width = "64")]
            let boot_time: i64 = t;
            #[cfg(not(target_pointer_width = "64"))]
            let boot_time: i64 = t.into();
            now - boot_time
        }
        None => -1,
    })
}

