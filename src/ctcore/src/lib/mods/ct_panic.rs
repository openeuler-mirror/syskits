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
//! Custom panic hooks that allow silencing certain types of errors.
//!
//! Use the [`mute_sigpipe_panic`] function to silence panics caused by
//! broken pipe errors. This can happen when a process is still
//! producing data when the consuming process terminates and closes the
//! pipe. For example,
//!
//! ```sh
//! $ seq inf | head -n 1
//! ```
//!
use std::panic;
use std::panic::PanicInfo;

/// Decide whether a panic was caused by a broken pipe (SIGPIPE) error.
fn is_broken_pipe(info: &PanicInfo) -> bool {
    // if let Some(res) = info.payload().downcast_ref::<String>() {
    //     if res.contains("BrokenPipe") || res.contains("Broken pipe") {
    //         return true;
    //     }
    // }
    // false
    info.payload()
        .downcast_ref::<String>()
        .map_or(false, |message| {
            message.contains("BrokenPipe") || message.contains("Broken pipe")
        })
}

/// Terminate without error on panics that occur due to broken pipe errors.
///
/// For background discussions on `SIGPIPE` handling, see
///
/// * `<https://github.com/cttils/coreutils/issues/374>`
/// * `<https://github.com/cttils/coreutils/pull/1106>`
/// * `<https://github.com/rust-lang/rust/issues/62569>`
/// * `<https://github.com/BurntSushi/ripgrep/issues/200>`
/// * `<https://github.com/crev-dev/cargo-crev/issues/287>`
///
pub fn mute_sigpipe_panic() {
    // let hook = panic::take_hook();
    // panic::set_hook(Box::new(move |info| {
    //     if !is_broken_pipe(info) {
    //         hook(info);
    //     }
    // }));

    // Take the current global panic hook
    let previous_hook = panic::take_hook();

    // Create a new panic hook that ignores 'broken pipe' panics
    let new_hook = Box::new(move |info: &PanicInfo| {
        if !is_broken_pipe(info) {
            // Call the original hook if it's not a broken pipe panic
            previous_hook(info);
        }
    });

    // Set the new hook as the global panic hook
    panic::set_hook(new_hook);
}

