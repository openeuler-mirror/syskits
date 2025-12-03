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
//! Provides consistent newline/zero terminator handling for `-z`/`--zero` flags.
//!
//! See the [`LineEnding`] struct for more information.
use std::fmt::Display;

/// Line ending of either `\n` or `\0`
///
/// Used by various utilities that have the option to separate lines by nul
/// characters instead of `\n`. Usually, this is specified with the `-z` or
/// `--zero` flag.
///
/// The [`Display`] implementation writes the character corresponding to the
/// variant to the formatter.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum LineEnding {
    #[default]
    Newline = b'\n',
    Nul = 0,
}

impl Display for LineEnding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if *self == LineEnding::Newline {
            writeln!(f) // Writes a newline character to the formatter
        } else {
            write!(f, "\0") // Writes a null character to the formatter
        }
    }
}

impl From<LineEnding> for u8 {
    fn from(line_ending: LineEnding) -> Self {
        line_ending as Self
    }
}

impl LineEnding {
    /// Create a [`LineEnding`] from a `-z`/`--zero` flag
    ///
    /// If `is_zero_terminated` is true, [`LineEnding::Nul`] is returned,
    /// otherwise [`LineEnding::Newline`].
    pub fn from_zero_flag(is_zero_terminated: bool) -> Self {
        match is_zero_terminated {
            true => Self::Nul,
            false => Self::Newline,
        }
    }
}

