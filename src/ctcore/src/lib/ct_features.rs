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

#[cfg(feature = "backup-control")]
pub mod ct_backup_control;
#[cfg(feature = "colors")]
pub mod ct_colors;
#[cfg(feature = "encoding")]
pub mod ct_encoding;
#[cfg(feature = "format")]
pub mod ct_format;
#[cfg(feature = "fs")]
pub mod ct_fs;
#[cfg(feature = "fsext")]
pub mod ct_fsext;
#[cfg(feature = "lines")]
pub mod ct_lines;
#[cfg(feature = "quoting-style")]
pub mod ct_quoting_style;
#[cfg(feature = "ranges")]
pub mod ct_ranges;
#[cfg(feature = "ringbuffer")]
pub mod ct_ringbuffer;
#[cfg(feature = "sum")]
pub mod ct_sum;
#[cfg(feature = "update-control")]
pub mod ct_update_control;
#[cfg(feature = "version-cmp")]
pub mod ct_version_cmp;

// * （平台相关）特性门控模块

// ** 非Linux类（即Unix与Fuchsia）
#[cfg(all(not(likelinux), feature = "mode"))]
pub mod ct_mode;

// ** 仅unix
#[cfg(all(unix, feature = "entries"))]
pub mod ct_entries;
#[cfg(all(unix, feature = "perms"))]
pub mod ct_perms;
#[cfg(all(unix, feature = "pipes"))]
pub mod ct_pipes;
#[cfg(all(unix, feature = "process"))]
pub mod ct_process;

#[cfg(all(unix, not(target_os = "macos"), feature = "fsxattr"))]
pub mod ct_fsxattr;
#[cfg(all(unix, not(target_os = "fuchsia"), feature = "signals"))]
pub mod ct_signals;
#[cfg(all(
    unix,
    not(target_os = "android"),
    not(target_os = "fuchsia"),
    not(target_os = "openbsd"),
    not(target_os = "redox"),
    not(target_env = "musl"),
    feature = "utmpx"
))]
pub mod ct_utmpx;
// ** likelinux-only
#[cfg(all(likelinux, feature = "wide"))]
pub mod ct_wide;
