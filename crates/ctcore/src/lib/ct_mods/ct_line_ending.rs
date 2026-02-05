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

use std::fmt::Display;

/// 行尾字符，可以是\n或\0
/// 该枚举由一些具有使用nul字符而非\n分隔行选项的工具使用。通常，这是通过-z或--zero标志指定的。
/// 实现了[Display] trait，会将枚举变体对应的字符写入格式化器中。
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
            writeln!(f) // 向格式化器写入换行符
        } else {
            write!(f, "\0") //向格式化器写入空字符
        }
    }
}

impl From<CtLineEnding> for u8 {
    fn from(line_ending: CtLineEnding) -> Self {
        line_ending as Self
    }
}

impl CtLineEnding {
    /// 从-z/--zero标志创建一个[CtLineEnding]实例
    ///
    /// 若`is_zero_terminated`为真，则返回表示以`NUL`字符结尾的行的枚举值[`CtLineEnding::Nul`]；
    /// 否则返回表示以换行符`\n`结尾的行的枚举值[`CtLineEnding::Newline`]。
    pub fn from_zero_flag(flag_zero: bool) -> Self {
        match flag_zero {
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
        assert_eq!(format!("{newline}"), "\n");
    }

    #[test]
    fn test_display_nul() {
        let nul = CtLineEnding::Nul;
        assert_eq!(format!("{nul}"), "\0");
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
        write!(output, "{newline}").expect("Failed to write to string");
        assert_eq!(output, "\n");
    }

    #[test]
    fn test_display_formats_correctly_nul() {
        let nul = CtLineEnding::Nul;
        let mut output = String::new();
        write!(output, "{nul}").expect("Failed to write to string");
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
