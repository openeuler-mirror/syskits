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

use std::path::Path;

use chrono::Local;

#[cfg(unix)]
use ctcore::libc::getloadavg;
use ctcore::libc::time_t;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UptimeSource {
    ProcUptime,
    BootTime,
    TickCount,
    Unknown,
}

#[cfg(unix)]
pub fn uptime_source_kind(source: UptimeSource) -> &'static str {
    match source {
        UptimeSource::ProcUptime => "proc_uptime",
        UptimeSource::BootTime => "boot_time",
        UptimeSource::TickCount => "tick_count",
        UptimeSource::Unknown => "unknown",
    }
}

#[cfg(unix)]
pub fn get_loadavg_values() -> Vec<f64> {
    use ctcore::libc::c_double;

    let mut avg: [c_double; 3] = [0.0; 3];
    let loads: i32 = unsafe { getloadavg(avg.as_mut_ptr(), 3) };

    if loads == -1 {
        Vec::new()
    } else {
        avg[..usize::try_from(loads).unwrap_or(0)].to_vec()
    }
}

#[cfg(test)]
pub fn print_loadavg() -> String {
    let loads = get_loadavg_values();
    if loads.is_empty() {
        String::new()
    } else {
        let mut result = "load average: ".to_string();
        for (index, value) in loads.iter().enumerate() {
            let separator = if index + 1 == loads.len() { "\n" } else { ", " };
            result.push_str(&format!("{value:.2}{separator}"));
        }
        result
    }
}

#[cfg(unix)]
pub fn process_utmpx(path: Option<&str>) -> (Option<time_t>, usize, Option<std::io::Error>) {
    use ctcore::ct_utmpx::*;

    let mut n_users = 0;
    let mut boot_time = None;
    let records = if let Some(path) = path {
        if let Err(err) = std::fs::File::open(path) {
            return (None, 0, Some(err));
        }
        CtUtmpx::iter_all_records_from(path)
    } else {
        CtUtmpx::iter_all_records()
    };

    for record in records {
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
    (boot_time, n_users, None)
}

pub fn get_uptime_from_boot_time(boot_time: Option<time_t>) -> i64 {
    match boot_time {
        Some(t) => {
            let now = Local::now().timestamp();
            #[cfg(target_pointer_width = "64")]
            let boot_time: i64 = t;
            #[cfg(not(target_pointer_width = "64"))]
            let boot_time: i64 = t.into();
            now - boot_time
        }
        None => -1,
    }
}

#[cfg(unix)]
pub fn get_uptime_with_source(boot_time: Option<time_t>) -> (i64, UptimeSource) {
    get_uptime_by_proc_with_source(boot_time, "/proc/uptime")
}

#[cfg(unix)]
#[cfg(test)]
fn get_uptime_by_proc<P: AsRef<Path>>(boot_time: Option<time_t>, path: P) -> i64 {
    get_uptime_by_proc_with_source(boot_time, path).0
}

#[cfg(unix)]
fn get_uptime_by_proc_with_source<P: AsRef<Path>>(
    boot_time: Option<time_t>,
    path: P,
) -> (i64, UptimeSource) {
    use std::fs::File;
    use std::io::Read;

    let mut proc_uptime_s = String::new();

    let proc_uptime = File::open(path)
        .ok()
        .and_then(|mut f| f.read_to_string(&mut proc_uptime_s).ok())
        .and_then(|_| proc_uptime_s.split_whitespace().next())
        .and_then(|s| s.split('.').next().unwrap_or("0").parse().ok());

    if let Some(value) = proc_uptime {
        (value, UptimeSource::ProcUptime)
    } else {
        match boot_time {
            Some(t) => {
                let now = Local::now().timestamp();
                #[cfg(target_pointer_width = "64")]
                let boot_time: i64 = t;
                #[cfg(not(target_pointer_width = "64"))]
                let boot_time: i64 = t.into();
                (now - boot_time, UptimeSource::BootTime)
            }
            None => (-1, UptimeSource::Unknown),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(test)]
    mod print_loadavg_tests {
        #[cfg(unix)]
        use super::print_loadavg;

        #[test]
        #[cfg(unix)]
        fn test_print_loadavg() {
            let result = print_loadavg();
            assert!(result.contains(":"));
        }
    }

    #[cfg(test)]
    #[cfg(unix)]
    mod get_uptime_by_proc_tests {
        use std::fs;
        use std::io::Write;
        use std::time::{SystemTime, UNIX_EPOCH};

        use tempfile::tempdir;

        use super::*;

        #[test]
        fn test_get_uptime_by_proc_with_proc_uptime() {
            let dir = tempdir().unwrap();
            let proc_uptime_path = dir.path().join("uptime");
            let mut file = fs::File::create(&proc_uptime_path).unwrap();
            writeln!(file, "12345.67 67890.12").unwrap();

            let result = get_uptime_by_proc(None, proc_uptime_path);
            assert_eq!(result, 12345);
        }

        #[test]
        fn test_get_uptime_by_proc_without_proc_uptime_with_boot_time() {
            let boot_time = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64
                - 12345;
            let result = get_uptime_by_proc(Some(boot_time), "/nonexistent/path");
            assert!(result >= 12345);
        }

        #[test]
        fn test_get_uptime_by_proc_without_proc_uptime_without_boot_time() {
            let result = get_uptime_by_proc(None, "/nonexistent/path");
            assert_eq!(result, -1);
        }

        #[test]
        fn test_get_uptime_by_proc_with_proc_uptime2() {
            let dir = tempdir().unwrap();
            let proc_uptime_path = dir.path().join("uptime");
            let mut file = fs::File::create(&proc_uptime_path).unwrap();
            writeln!(file, "12345.67 67890.12").unwrap();

            let result = get_uptime_by_proc(None, proc_uptime_path);
            assert_eq!(result, 12345);
        }

        #[test]
        fn test_get_uptime_by_proc_without_proc_uptime_with_boot_time2() {
            let boot_time = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64
                - 12345;
            let result = get_uptime_by_proc(Some(boot_time), "/nonexistent/path");
            assert!(result >= 12345);
        }

        #[test]
        fn test_get_uptime_by_proc_without_proc_uptime_without_boot_time2() {
            let result = get_uptime_by_proc(None, "/nonexistent/path");
            assert_eq!(result, -1);
        }

        #[test]
        fn test_get_uptime_by_proc_with_empty_proc_uptime2() {
            let dir = tempdir().unwrap();
            let proc_uptime_path = dir.path().join("uptime");
            let mut file = fs::File::create(&proc_uptime_path).unwrap();
            writeln!(file).unwrap();

            let result = get_uptime_by_proc(None, proc_uptime_path);
            assert_eq!(result, -1);
        }

        #[test]
        fn test_get_uptime_by_proc_with_invalid_proc_uptime2() {
            let dir = tempdir().unwrap();
            let proc_uptime_path = dir.path().join("uptime");
            let mut file = fs::File::create(&proc_uptime_path).unwrap();
            writeln!(file, "invalid_data").unwrap();

            let result = get_uptime_by_proc(None, proc_uptime_path);
            assert_eq!(result, -1);
        }

        #[test]
        fn test_get_uptime_by_proc_with_proc_uptime_with_decimal() {
            let dir = tempdir().unwrap();
            let proc_uptime_path = dir.path().join("uptime");
            let mut file = fs::File::create(&proc_uptime_path).unwrap();
            writeln!(file, "12345.67").unwrap();

            let result = get_uptime_by_proc(None, proc_uptime_path);
            assert_eq!(result, 12345);
        }

        #[test]
        fn test_get_uptime_by_proc_with_boot_time_32bit() {
            let boot_time = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as u32
                - 12345;
            let result = get_uptime_by_proc(Some(boot_time as time_t), "/nonexistent/path");
            assert!(result >= 12345);
        }

        #[test]
        fn test_get_uptime_by_proc_with_boot_time_64bit() {
            let boot_time = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64
                - 12345;
            let result = get_uptime_by_proc(Some(boot_time), "/nonexistent/path");
            assert!(result >= 12345);
        }

        #[test]
        fn test_process_utmpx_with_missing_file() {
            let (_boot_time, user_count, err) = process_utmpx(Some("/nonexistent/utmp-file"));
            assert_eq!(user_count, 0);
            assert!(err.is_some());
        }
    }
}
