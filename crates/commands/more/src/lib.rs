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

//! More command implementation - modular architecture
//!
//! This module provides a pager for viewing text files, strictly aligned with util-linux more behavior.
//!
//! # Architecture
//!
//! The implementation is organized into layers:
//!
//! - **TTY Layer** (`tty`): Raw terminal I/O, key reading, terminal control sequences
//! - **Command Layer** (`command`): Parse input into semantic actions
//! - **Pager Layer** (`pager`): State machine, core paging logic
//! - **Render Layer** (`render`): Terminal rendering, prompt formatting
//!
//! This layered design ensures clear separation of concerns and makes TTY behavior
//! alignment with util-linux more explicit and testable.

extern crate rust_i18n;
rust_i18n::i18n!("locales", fallback = "zh-CN");

pub mod command;
pub mod more;
pub mod pager;
pub mod render;
pub mod tty;

pub use more::{More, more_main};
