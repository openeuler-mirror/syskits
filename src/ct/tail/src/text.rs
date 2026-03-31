/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

pub const TAIL_DASH: &str = "-";
pub const TAIL_DEV_STDIN: &str = "/dev/stdin";
pub const TAIL_STDIN_HEADER: &str = "standard input";
pub const TAIL_NO_FILES_REMAINING: &str = "no files remaining";
pub const TAIL_NO_SUCH_FILE: &str = "No such file or directory";
pub const TAIL_BECOME_INACCESSIBLE: &str = "has become inaccessible";
pub const TAIL_BAD_FD: &str = "Bad file descriptor";
#[cfg(target_os = "linux")]
pub const TAIL_BACKEND: &str = "inotify";
#[cfg(all(unix, not(target_os = "linux")))]
pub const TAIL_BACKEND: &str = "kqueue";
#[cfg(target_os = "windows")]
pub const TAIL_BACKEND: &str = "ReadDirectoryChanges";
pub const TAIL_FD0: &str = "/dev/fd/0";
pub const TAIL_IS_A_DIRECTORY: &str = "Is a directory";
pub const TAIL_DEV_TTY: &str = "/dev/tty";
pub const TAIL_DEV_PTMX: &str = "/dev/ptmx";
