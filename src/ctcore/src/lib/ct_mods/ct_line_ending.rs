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
//! See the [`CtLineEnding`] struct for more information.
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
pub enum CtLineEnding {
    #[default]
    Newline = b'\n',
    Nul = 0,
}

impl Display for CtLineEnding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if *self == CtLineEnding::Newline {
            writeln!(f) // Writes a newline character to the formatter
        } else {
            write!(f, "\0") // Writes a null character to the formatter
        }
    }
}

impl From<CtLineEnding> for u8 {
    fn from(line_ending: CtLineEnding) -> Self {
        line_ending as Self
    }
}

impl CtLineEnding {
    /// Create a [`CtLineEnding`] from a `-z`/`--zero` flag
    ///
    /// If `is_zero_terminated` is true, [`CtLineEnding::Nul`] is returned,
    /// otherwise [`CtLineEnding::Newline`].
    pub fn from_zero_flag(is_zero_terminated: bool) -> Self {
        match is_zero_terminated {
            true => Self::Nul,
            false => Self::Newline,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write;
    #[test]
    fn test_default_line_ending() {
        assert_eq!(CtLineEnding::default(), CtLineEnding::Newline);
    }

    #[test]
    fn test_display_newline() {
        let newline = CtLineEnding::Newline;
        assert_eq!(format!("{}", newline), "\n");
    }

    #[test]
    fn test_display_nul() {
        let nul = CtLineEnding::Nul;
        assert_eq!(format!("{}", nul), "\0");
    }

    #[test]
    fn test_from_u8_newline() {
        assert_eq!(u8::from(CtLineEnding::Newline), b'\n');
    }

    #[test]
    fn test_from_u8_nul() {
        assert_eq!(u8::from(CtLineEnding::Nul), 0);
    }

    #[test]
    fn test_from_zero_flag_true() {
        assert_eq!(CtLineEnding::from_zero_flag(true), CtLineEnding::Nul);
    }

    #[test]
    fn test_from_zero_flag_false() {
        assert_eq!(CtLineEnding::from_zero_flag(false), CtLineEnding::Newline);
    }

    #[test]
    fn test_display_formats_correctly_newline() {
        let newline = CtLineEnding::Newline;
        let mut output = String::new();
        write!(output, "{}", newline).expect("Failed to write to string");
        assert_eq!(output, "\n");
    }

    #[test]
    fn test_display_formats_correctly_nul() {
        let nul = CtLineEnding::Nul;
        let mut output = String::new();
        write!(output, "{}", nul).expect("Failed to write to string");
        assert_eq!(output, "\0");
    }

    #[test]
    fn line_ending_equality_checks() {
        assert_eq!(CtLineEnding::Newline, CtLineEnding::Newline);
        assert_eq!(CtLineEnding::Nul, CtLineEnding::Nul);
        assert_ne!(CtLineEnding::Newline, CtLineEnding::Nul);
    }

    #[test]
    fn test_display_implementation_consistency() {
        assert_eq!(format!("{}", CtLineEnding::Newline), "\n");
        assert_eq!(format!("{}", CtLineEnding::Nul), "\0");
    }

    #[test]
    fn test_conversion_to_u8_and_back() {
        let newline = CtLineEnding::Newline;
        let nul = CtLineEnding::Nul;
        let newline_byte: u8 = newline.into();
        let nul_byte: u8 = nul.into();

        assert_eq!(
            CtLineEnding::from_zero_flag(newline_byte == 0),
            CtLineEnding::Newline
        );
        assert_eq!(
            CtLineEnding::from_zero_flag(nul_byte == 0),
            CtLineEnding::Nul
        );
        assert_eq!(
            CtLineEnding::from_zero_flag(newline_byte != 0),
            CtLineEnding::Nul
        );
    }
}
