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

//! 自定义字符串比较。
//!
//! 目标是在不转换字符串的情况下（即不分配新字符串）比较字符串。)

use ctcore::ct_locale::strcoll_compare;
use std::cmp::Ordering;

fn custom_filter_char(
    c: char,
    is_ignore_non_printing: bool,
    is_ignore_non_dictionary: bool,
) -> bool {
    if is_ignore_non_dictionary && !(c.is_ascii_alphanumeric() || c.is_ascii_whitespace()) {
        return false;
    }
    if is_ignore_non_printing && (c.is_ascii_control() || !c.is_ascii()) {
        return false;
    }
    true
}

fn custom_cmp_chars(a: char, b: char, is_ignore_case: bool) -> Ordering {
    match is_ignore_case {
        true => a.to_ascii_uppercase().cmp(&b.to_ascii_uppercase()),
        false => a.cmp(&b),
    }
}

pub fn custom_cmp_str(
    a: &str,
    b: &str,
    is_ignore_non_printing: bool,
    is_ignore_non_dictionary: bool,
    ignore_case: bool,
) -> Ordering {
    if !(ignore_case || is_ignore_non_dictionary || is_ignore_non_printing) {
        // 没有自定义设置。使用locale感知的字符串比较，速度更快。
        return strcoll_compare(a.as_bytes(), b.as_bytes(), false);
    }
    let mut a_chars = a
        .chars()
        .filter(|&c| custom_filter_char(c, is_ignore_non_printing, is_ignore_non_dictionary));
    let mut b_chars = b
        .chars()
        .filter(|&c| custom_filter_char(c, is_ignore_non_printing, is_ignore_non_dictionary));
    loop {
        let a_char = a_chars.next();
        let b_char = b_chars.next();
        match (a_char, b_char) {
            (None, None) => return Ordering::Equal,
            (Some(_), None) => return Ordering::Greater,
            (None, Some(_)) => return Ordering::Less,
            (Some(a_char), Some(b_char)) => {
                let ordering = custom_cmp_chars(a_char, b_char, ignore_case);
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;
    use std::env;

    use super::{custom_cmp_chars, custom_cmp_str, custom_filter_char};

    #[test]
    fn test_filter_char_no_ignore() {
        assert!(custom_filter_char('a', false, false));
        assert!(custom_filter_char('1', false, false));
        assert!(custom_filter_char(' ', false, false));
        assert!(custom_filter_char('\n', false, false));
        assert!(custom_filter_char('\u{263A}', false, false)); // Unicode smiley face

        assert!(custom_filter_char('\u{0}', false, false)); // ASCII NUL
        assert!(custom_filter_char('\u{FFFF}', false, false)); // Non-ASCII character

        assert!(custom_filter_char('!', false, false));
        assert!(custom_filter_char('@', false, false));
        assert!(custom_filter_char('#', false, false));
        assert!(custom_filter_char('$', false, false));
        assert!(custom_filter_char('%', false, false));
        assert!(custom_filter_char('^', false, false));
        assert!(custom_filter_char('&', false, false));
        assert!(custom_filter_char('*', false, false));
        assert!(custom_filter_char('(', false, false));
        assert!(custom_filter_char(')', false, false));
        assert!(custom_filter_char('-', false, false));
        assert!(custom_filter_char('_', false, false));
        assert!(custom_filter_char('=', false, false));
        assert!(custom_filter_char('+', false, false));
        assert!(custom_filter_char('{', false, false));
        assert!(custom_filter_char('}', false, false));
        assert!(custom_filter_char('|', false, false));
        assert!(custom_filter_char(':', false, false));
        assert!(custom_filter_char(';', false, false));
        assert!(custom_filter_char('\'', false, false));
        assert!(custom_filter_char('"', false, false));
        assert!(custom_filter_char('<', false, false));
        assert!(custom_filter_char('>', false, false));
        assert!(custom_filter_char(',', false, false));
        assert!(custom_filter_char('.', false, false));
        assert!(custom_filter_char('/', false, false));
        assert!(custom_filter_char('?', false, false));

        assert!(custom_filter_char('\u{00A0}', false, false)); // Non-breaking space
        assert!(custom_filter_char('\u{00A9}', false, false)); // Copyright symbol
        assert!(custom_filter_char('\u{00AE}', false, false)); // Registered trademark symbol
        assert!(custom_filter_char('\u{00B0}', false, false)); // Degree sign
        assert!(custom_filter_char('\u{00B7}', false, false)); // Middle dot
        assert!(custom_filter_char('\u{00BB}', false, false)); // Right-pointing double angle quotation mark
        assert!(custom_filter_char('\u{00BF}', false, false)); // Inverted question mark
        assert!(custom_filter_char('\u{2013}', false, false)); // En dash
        assert!(custom_filter_char('\u{2014}', false, false)); // Em dash
        assert!(custom_filter_char('\u{2018}', false, false)); // Left single quotation mark
        assert!(custom_filter_char('\u{2019}', false, false)); // Right single quotation mark
        assert!(custom_filter_char('\u{201C}', false, false)); // Left double quotation mark
        assert!(custom_filter_char('\u{201D}', false, false)); // Right double quotation mark
        assert!(custom_filter_char('\u{2026}', false, false)); // Horizontal ellipsis
        assert!(custom_filter_char('\u{2122}', false, false)); // Trademark symbol
        assert!(custom_filter_char('\u{2212}', false, false)); // Minus sign
        assert!(custom_filter_char('\u{2605}', false, false)); // Black star
    }

    #[test]
    fn test_filter_char_ignore_non_printing() {
        assert!(custom_filter_char('a', true, false));
        assert!(custom_filter_char('1', true, false));
        assert!(custom_filter_char(' ', true, false));
        assert!(!custom_filter_char('\n', true, false)); // ASCII control character
        assert!(!custom_filter_char('\u{263A}', true, false)); // Unicode smiley face

        assert!(!custom_filter_char('\u{0}', true, false)); // ASCII NUL
        assert!(!custom_filter_char('\u{FFFF}', true, false)); // Non-ASCII character

        assert!(custom_filter_char('!', true, false));
        assert!(custom_filter_char('@', true, false));
        assert!(custom_filter_char('#', true, false));
        assert!(custom_filter_char('$', true, false));
        assert!(custom_filter_char('%', true, false));
        assert!(custom_filter_char('^', true, false));
        assert!(custom_filter_char('&', true, false));
        assert!(custom_filter_char('*', true, false));
        assert!(custom_filter_char('(', true, false));
        assert!(custom_filter_char(')', true, false));
        assert!(custom_filter_char('-', true, false));
        assert!(custom_filter_char('_', true, false));
        assert!(custom_filter_char('=', true, false));
        assert!(custom_filter_char('+', true, false));
        assert!(custom_filter_char('{', true, false));
        assert!(custom_filter_char('}', true, false));
        assert!(custom_filter_char('|', true, false));
        assert!(custom_filter_char(':', true, false));
        assert!(custom_filter_char(';', true, false));
        assert!(custom_filter_char('\'', true, false));
        assert!(custom_filter_char('"', true, false));
        assert!(custom_filter_char('<', true, false));
        assert!(custom_filter_char('>', true, false));
        assert!(custom_filter_char(',', true, false));
        assert!(custom_filter_char('.', true, false));
        assert!(custom_filter_char('/', true, false));
        assert!(custom_filter_char('?', true, false));

        assert!(!custom_filter_char('\u{00A0}', true, false)); // Non-breaking space
        assert!(!custom_filter_char('\u{00A9}', true, false)); // Copyright symbol
        assert!(!custom_filter_char('\u{00AE}', true, false)); // Registered trademark symbol
        assert!(!custom_filter_char('\u{00B0}', true, false)); // Degree sign
        assert!(!custom_filter_char('\u{00B7}', true, false)); // Middle dot
        assert!(!custom_filter_char('\u{00BB}', true, false)); // Right-pointing double angle quotation mark
        assert!(!custom_filter_char('\u{00BF}', true, false)); // Inverted question mark
        assert!(!custom_filter_char('\u{2013}', true, false)); // En dash
        assert!(!custom_filter_char('\u{2014}', true, false)); // Em dash
        assert!(!custom_filter_char('\u{2018}', true, false)); // Left single quotation mark
        assert!(!custom_filter_char('\u{2019}', true, false)); // Right single quotation mark
        assert!(!custom_filter_char('\u{201C}', true, false)); // Left double quotation mark
        assert!(!custom_filter_char('\u{201D}', true, false)); // Right double quotation mark
        assert!(!custom_filter_char('\u{2026}', true, false)); // Horizontal ellipsis
        assert!(!custom_filter_char('\u{2122}', true, false)); // Trademark symbol
        assert!(!custom_filter_char('\u{2212}', true, false)); // Minus sign
        assert!(!custom_filter_char('\u{2605}', true, false)); // Black star
    }

    #[test]
    fn test_filter_char_ignore_non_dictionary() {
        assert!(custom_filter_char('a', false, true));
        assert!(custom_filter_char('1', false, true));
        assert!(custom_filter_char(' ', false, true));
        assert!(custom_filter_char('\n', false, true)); // ASCII control character
        assert!(!custom_filter_char('\u{263A}', false, true)); // Non-alphanumeric, non-whitespace Unicode character

        assert!(!custom_filter_char('\u{0}', false, true)); // ASCII NUL
        assert!(!custom_filter_char('\u{FFFF}', false, true)); // Non-ASCII character

        assert!(!custom_filter_char('!', false, true));
        assert!(!custom_filter_char('@', false, true));
        assert!(!custom_filter_char('#', false, true));
        assert!(!custom_filter_char('$', false, true));
        assert!(!custom_filter_char('%', false, true));
        assert!(!custom_filter_char('^', false, true));
        assert!(!custom_filter_char('&', false, true));
        assert!(!custom_filter_char('*', false, true));
        assert!(!custom_filter_char('(', false, true));
        assert!(!custom_filter_char(')', false, true));
        assert!(!custom_filter_char('-', false, true));
        assert!(!custom_filter_char('_', false, true));
        assert!(!custom_filter_char('=', false, true));
        assert!(!custom_filter_char('+', false, true));
        assert!(!custom_filter_char('{', false, true));
        assert!(!custom_filter_char('}', false, true));
        assert!(!custom_filter_char('|', false, true));
        assert!(!custom_filter_char(':', false, true));
        assert!(!custom_filter_char(';', false, true));
        assert!(!custom_filter_char('\'', false, true));
        assert!(!custom_filter_char('"', false, true));
        assert!(!custom_filter_char('<', false, true));
        assert!(!custom_filter_char('>', false, true));
        assert!(!custom_filter_char(',', false, true));
        assert!(!custom_filter_char('.', false, true));
        assert!(!custom_filter_char('/', false, true));
        assert!(!custom_filter_char('?', false, true));

        assert!(!custom_filter_char('\u{00A0}', false, true)); // Non-breaking space
        assert!(!custom_filter_char('\u{00A9}', false, true)); // Copyright symbol
        assert!(!custom_filter_char('\u{00AE}', false, true)); // Registered trademark symbol
        assert!(!custom_filter_char('\u{00B0}', false, true)); // Degree sign
        assert!(!custom_filter_char('\u{00B7}', false, true)); // Middle dot
        assert!(!custom_filter_char('\u{00BB}', false, true)); // Right-pointing double angle quotation mark
        assert!(!custom_filter_char('\u{00BF}', false, true)); // Inverted question mark
        assert!(!custom_filter_char('\u{2013}', false, true)); // En dash
        assert!(!custom_filter_char('\u{2014}', false, true)); // Em dash
        assert!(!custom_filter_char('\u{2018}', false, true)); // Left single quotation mark
        assert!(!custom_filter_char('\u{2019}', false, true)); // Right single quotation mark
        assert!(!custom_filter_char('\u{201C}', false, true)); // Left double quotation mark
        assert!(!custom_filter_char('\u{201D}', false, true)); // Right double quotation mark
        assert!(!custom_filter_char('\u{2026}', false, true)); // Horizontal ellipsis
        assert!(!custom_filter_char('\u{2122}', false, true)); // Trademark symbol
        assert!(!custom_filter_char('\u{2212}', false, true)); // Minus sign
        assert!(!custom_filter_char('\u{2605}', false, true)); // Black star
    }

    #[test]
    fn test_filter_char_ignore_both() {
        assert!(custom_filter_char('a', true, true));
        assert!(custom_filter_char('1', true, true));
        assert!(custom_filter_char(' ', true, true));
        assert!(!custom_filter_char('\n', true, true)); // ASCII control character
        assert!(!custom_filter_char('\u{263A}', true, true)); // Non-alphanumeric, non-whitespace Unicode character

        assert!(!custom_filter_char('\u{0}', true, true)); // ASCII NUL
        assert!(!custom_filter_char('\u{FFFF}', true, true)); // Non-ASCII character

        assert!(!custom_filter_char('!', true, true));
        assert!(!custom_filter_char('@', true, true));
        assert!(!custom_filter_char('#', true, true));
        assert!(!custom_filter_char('$', true, true));
        assert!(!custom_filter_char('%', true, true));
        assert!(!custom_filter_char('^', true, true));
        assert!(!custom_filter_char('&', true, true));
        assert!(!custom_filter_char('*', true, true));
        assert!(!custom_filter_char('(', true, true));
        assert!(!custom_filter_char(')', true, true));
        assert!(!custom_filter_char('-', true, true));
        assert!(!custom_filter_char('_', true, true));
        assert!(!custom_filter_char('=', true, true));
        assert!(!custom_filter_char('+', true, true));
        assert!(!custom_filter_char('{', true, true));
        assert!(!custom_filter_char('}', true, true));
        assert!(!custom_filter_char('|', true, true));
        assert!(!custom_filter_char(':', true, true));
        assert!(!custom_filter_char(';', true, true));
        assert!(!custom_filter_char('\'', true, true));
        assert!(!custom_filter_char('"', true, true));
        assert!(!custom_filter_char('<', true, true));
        assert!(!custom_filter_char('>', true, true));
        assert!(!custom_filter_char(',', true, true));
        assert!(!custom_filter_char('.', true, true));
        assert!(!custom_filter_char('/', true, true));
        assert!(!custom_filter_char('?', true, true));

        assert!(!custom_filter_char('\u{00A0}', true, true)); // Non-breaking space
        assert!(!custom_filter_char('\u{00A9}', true, true)); // Copyright symbol
        assert!(!custom_filter_char('\u{00AE}', true, true)); // Registered trademark symbol
        assert!(!custom_filter_char('\u{00B0}', true, true)); // Degree sign
        assert!(!custom_filter_char('\u{00B7}', true, true)); // Middle dot
        assert!(!custom_filter_char('\u{00BB}', true, true)); // Right-pointing double angle quotation mark
        assert!(!custom_filter_char('\u{00BF}', true, true)); // Inverted question mark
        assert!(!custom_filter_char('\u{2013}', true, true)); // En dash
        assert!(!custom_filter_char('\u{2014}', true, true)); // Em dash
        assert!(!custom_filter_char('\u{2018}', true, true)); // Left single quotation mark
        assert!(!custom_filter_char('\u{2019}', true, true)); // Right single quotation mark
        assert!(!custom_filter_char('\u{201C}', true, true)); // Left double quotation mark
        assert!(!custom_filter_char('\u{201D}', true, true)); // Right double quotation mark
        assert!(!custom_filter_char('\u{2026}', true, true)); // Horizontal ellipsis
        assert!(!custom_filter_char('\u{2122}', true, true)); // Trademark symbol
        assert!(!custom_filter_char('\u{2212}', true, true)); // Minus sign
        assert!(!custom_filter_char('\u{2605}', true, true)); // Black star
    }

    #[test]
    fn test_filter_char_ignore_both_with_special_cases() {
        assert!(!custom_filter_char('\u{00AD}', true, true)); // Soft hyphen
        assert!(!custom_filter_char('\u{200B}', true, true)); // Zero-width space
        assert!(!custom_filter_char('\u{2028}', true, true)); // Line separator
        assert!(!custom_filter_char('\u{2029}', true, true)); // Paragraph separator
        assert!(!custom_filter_char('\u{FEFF}', true, true)); // Byte Order Mark (BOM)

        assert!(!custom_filter_char('\u{000C}', true, true)); // Form feed (ASCII control character)
        assert!(!custom_filter_char('\u{007F}', true, true)); // Delete (ASCII control character)
    }

    #[test]
    fn test_cmp_chars_no_ignore_case() {
        assert_eq!(custom_cmp_chars('a', 'b', false), Ordering::Less);
        assert_eq!(custom_cmp_chars('b', 'a', false), Ordering::Greater);
        assert_eq!(custom_cmp_chars('a', 'a', false), Ordering::Equal);
    }

    #[test]
    fn test_cmp_chars_ignore_case() {
        assert_eq!(custom_cmp_chars('a', 'B', true), Ordering::Less);
        assert_eq!(custom_cmp_chars('b', 'A', true), Ordering::Greater);
        assert_eq!(custom_cmp_chars('A', 'a', true), Ordering::Equal);

        assert_eq!(custom_cmp_chars('a', 'c', true), Ordering::Less);
        assert_eq!(custom_cmp_chars('c', 'a', true), Ordering::Greater);
        assert_eq!(custom_cmp_chars('A', 'A', true), Ordering::Equal);
    }

    #[test]
    fn test_cmp_chars_special_chars() {
        assert_eq!(
            custom_cmp_chars('\u{00E9}', '\u{00C9}', true),
            Ordering::Greater
        ); // é (U+00E9) vs É (U+00C9)
        assert_eq!(
            custom_cmp_chars('\u{00FC}', '\u{00DC}', true),
            Ordering::Greater
        ); // ü (U+00FC) vs Ü (U+00DC)

        assert_eq!(
            custom_cmp_chars('\u{00E9}', '\u{00C9}', false),
            Ordering::Greater
        ); // é (U+00E9) vs É (U+00C9)
        assert_eq!(
            custom_cmp_chars('\u{00FC}', '\u{00DC}', false),
            Ordering::Greater
        );
        // ü (U+00FC) vs Ü (U+00DC)
    }

    #[test]
    fn test_cmp_chars_numbers() {
        assert_eq!(custom_cmp_chars('1', '2', false), Ordering::Less);
        assert_eq!(custom_cmp_chars('2', '1', false), Ordering::Greater);
        assert_eq!(custom_cmp_chars('1', '1', false), Ordering::Equal);

        assert_eq!(custom_cmp_chars('1', '2', true), Ordering::Less);
        assert_eq!(custom_cmp_chars('2', '1', true), Ordering::Greater);
        assert_eq!(custom_cmp_chars('1', '1', true), Ordering::Equal);
    }

    #[test]
    fn test_cmp_chars_punctuation() {
        assert_eq!(custom_cmp_chars('.', ',', false), Ordering::Greater);
        assert_eq!(custom_cmp_chars(',', '.', false), Ordering::Less);
        assert_eq!(custom_cmp_chars('.', '.', false), Ordering::Equal);
        assert_eq!(custom_cmp_chars(',', ',', false), Ordering::Equal);

        assert_eq!(custom_cmp_chars('.', ',', true), Ordering::Greater);
        assert_eq!(custom_cmp_chars(',', '.', true), Ordering::Less);
        assert_eq!(custom_cmp_chars('.', '.', true), Ordering::Equal);
        assert_eq!(custom_cmp_chars(',', ',', true), Ordering::Equal);
    }

    #[test]
    fn test_cmp_chars_whitespace() {
        assert_eq!(custom_cmp_chars(' ', '\t', false), Ordering::Greater);
        assert_eq!(custom_cmp_chars('\t', ' ', false), Ordering::Less);
        assert_eq!(custom_cmp_chars(' ', ' ', false), Ordering::Equal);
        assert_eq!(custom_cmp_chars('\t', '\t', false), Ordering::Equal);

        assert_eq!(custom_cmp_chars(' ', '\t', true), Ordering::Greater);
        assert_eq!(custom_cmp_chars('\t', ' ', true), Ordering::Less);
        assert_eq!(custom_cmp_chars(' ', ' ', true), Ordering::Equal);
        assert_eq!(custom_cmp_chars('\t', '\t', true), Ordering::Equal);
    }

    #[test]
    fn test_cmp_chars_symbols() {
        assert_eq!(custom_cmp_chars('+', '-', false), Ordering::Less);
        assert_eq!(custom_cmp_chars('-', '+', false), Ordering::Greater);
        assert_eq!(custom_cmp_chars('+', '+', false), Ordering::Equal);
        assert_eq!(custom_cmp_chars('-', '-', false), Ordering::Equal);

        assert_eq!(custom_cmp_chars('+', '-', true), Ordering::Less);
        assert_eq!(custom_cmp_chars('-', '+', true), Ordering::Greater);
        assert_eq!(custom_cmp_chars('+', '+', true), Ordering::Equal);
        assert_eq!(custom_cmp_chars('-', '-', true), Ordering::Equal);
    }

    #[test]
    fn test_cmp_chars_unicode_characters() {
        assert_eq!(
            custom_cmp_chars('\u{1F600}', '\u{1F601}', false),
            Ordering::Less
        );
        assert_eq!(
            custom_cmp_chars('\u{1F601}', '\u{1F600}', false),
            Ordering::Greater
        );
        assert_eq!(
            custom_cmp_chars('\u{1F600}', '\u{1F600}', false),
            Ordering::Equal
        );

        assert_eq!(
            custom_cmp_chars('\u{1F600}', '\u{1F601}', true),
            Ordering::Less
        );
        assert_eq!(
            custom_cmp_chars('\u{1F601}', '\u{1F600}', true),
            Ordering::Greater
        );
        assert_eq!(
            custom_cmp_chars('\u{1F600}', '\u{1F600}', true),
            Ordering::Equal
        );
    }

    #[test]
    fn test_cmp_chars_edge_cases() {
        assert_eq!(custom_cmp_chars('\u{0}', '\u{1}', false), Ordering::Less);
        assert_eq!(custom_cmp_chars('\u{1}', '\u{0}', false), Ordering::Greater);
        assert_eq!(custom_cmp_chars('\u{0}', '\u{0}', false), Ordering::Equal);

        assert_eq!(custom_cmp_chars('\u{0}', '\u{1}', true), Ordering::Less);
        assert_eq!(custom_cmp_chars('\u{1}', '\u{0}', true), Ordering::Greater);
        assert_eq!(custom_cmp_chars('\u{0}', '\u{0}', true), Ordering::Equal);

        assert_eq!(
            custom_cmp_chars('\u{FFFF}', '\u{10000}', false),
            Ordering::Less
        );
        assert_eq!(
            custom_cmp_chars('\u{10000}', '\u{FFFF}', false),
            Ordering::Greater
        );
        assert_eq!(
            custom_cmp_chars('\u{FFFF}', '\u{FFFF}', false),
            Ordering::Equal
        );

        assert_eq!(
            custom_cmp_chars('\u{FFFF}', '\u{10000}', true),
            Ordering::Less
        );
        assert_eq!(
            custom_cmp_chars('\u{10000}', '\u{FFFF}', true),
            Ordering::Greater
        );
        assert_eq!(
            custom_cmp_chars('\u{FFFF}', '\u{FFFF}', true),
            Ordering::Equal
        );
    }

    #[test]
    fn test_cmp_chars_unusual_unicode_characters() {
        assert_eq!(
            custom_cmp_chars('\u{1D49E}', '\u{1D49F}', false),
            Ordering::Less
        );
        assert_eq!(
            custom_cmp_chars('\u{1D49F}', '\u{1D49E}', false),
            Ordering::Greater
        );
        assert_eq!(
            custom_cmp_chars('\u{1D49E}', '\u{1D49E}', false),
            Ordering::Equal
        );

        assert_eq!(
            custom_cmp_chars('\u{1D49E}', '\u{1D49F}', true),
            Ordering::Less
        );
        assert_eq!(
            custom_cmp_chars('\u{1D49F}', '\u{1D49E}', true),
            Ordering::Greater
        );
        assert_eq!(
            custom_cmp_chars('\u{1D49E}', '\u{1D49E}', true),
            Ordering::Equal
        );

        assert_eq!(
            custom_cmp_chars('\u{1F1E6}', '\u{1F1E7}', false),
            Ordering::Less
        );
        assert_eq!(
            custom_cmp_chars('\u{1F1E7}', '\u{1F1E6}', false),
            Ordering::Greater
        );
        assert_eq!(
            custom_cmp_chars('\u{1F1E6}', '\u{1F1E6}', false),
            Ordering::Equal
        );

        assert_eq!(
            custom_cmp_chars('\u{1F1E6}', '\u{1F1E7}', true),
            Ordering::Less
        );
        assert_eq!(
            custom_cmp_chars('\u{1F1E7}', '\u{1F1E6}', true),
            Ordering::Greater
        );
        assert_eq!(
            custom_cmp_chars('\u{1F1E6}', '\u{1F1E6}', true),
            Ordering::Equal
        );
    }

    #[test]
    fn test_custom_str_cmp_no_custom_settings() {
        let result = custom_cmp_str("abc", "def", false, false, false);
        assert_eq!(result, "abc".cmp("def"));
    }

    #[test]
    fn test_custom_str_cmp_ignore_non_printing() {
        let result = custom_cmp_str("abc\x08\x00def", "abcdef", true, false, false);
        assert_eq!(result, "abcdef".cmp("abcdef"));
    }

    #[test]
    fn test_custom_str_cmp_ignore_non_dictionary() {
        let result = custom_cmp_str("abc$%def", "abcdef", false, true, false);
        assert_eq!(result, "abcdef".cmp("abcdef"));
    }

    #[test]
    fn test_custom_str_cmp_ignore_case() {
        let result = custom_cmp_str("AbCdEf", "abcdef", false, false, true);
        assert_eq!(result, "abcdef".cmp("abcdef"));
    }

    #[test]
    fn test_custom_str_cmp_ignore_non_printing_and_non_dictionary() {
        let result = custom_cmp_str("abc\x08\x00$%def", "abcdef", true, true, false);
        assert_eq!(result, "abcdef".cmp("abcdef"));
    }

    #[test]
    fn test_custom_str_cmp_ignore_case_and_non_printing() {
        let result = custom_cmp_str("AbCdEf\x08\x00", "abcdef", true, false, true);
        assert_eq!(result, "abcdef".cmp("abcdef"));
    }

    #[test]
    fn test_custom_str_cmp_ignore_case_and_non_dictionary() {
        let result = custom_cmp_str("AbCdEf$", "abcdef", false, true, true);
        assert_eq!(result, "abcdef".cmp("abcdef"));
    }

    #[test]
    fn test_custom_str_cmp_ignore_all() {
        let result = custom_cmp_str("AbCdEf\x08\x00$%gHiJkL", "abcdefghijkl", true, true, true);
        assert_eq!(result, "abcdefghijkl".cmp("abcdefghijkl"));
    }

    #[test]
    fn test_custom_str_cmp_empty_strings() {
        let result = custom_cmp_str("", "", false, false, false);
        assert_eq!(result, Ordering::Equal);

        let result = custom_cmp_str("", "abc", false, false, false);
        assert_eq!(result, Ordering::Less);

        let result = custom_cmp_str("abc", "", false, false, false);
        assert_eq!(result, Ordering::Greater);
    }

    #[test]
    fn test_custom_str_cmp_equal_strings() {
        let result = custom_cmp_str("abcdef", "abcdef", false, false, false);
        assert_eq!(result, Ordering::Equal);

        let result = custom_cmp_str("aBcDeF", "abcdef", true, false, false);
        assert_eq!(result, Ordering::Less);

        let result = custom_cmp_str("abc$%def", "abcdef", false, true, false);
        assert_eq!(result, Ordering::Equal);

        let result = custom_cmp_str("AbCdEf\x08\x00$%def", "abcdef", true, true, false);
        assert_eq!(result, Ordering::Less);
    }

    #[test]
    fn test_custom_str_cmp_different_length() {
        let result = custom_cmp_str("abc", "abcd", false, false, false);
        assert_eq!(result, Ordering::Less);

        let result = custom_cmp_str("abcde", "abcd", false, false, false);
        assert_eq!(result, Ordering::Greater);
    }

    #[test]
    fn test_custom_str_cmp_complex_cases() {
        let result = custom_cmp_str(
            "AbC123\x08\x00$%dEf",
            "aBc456\x08\x00$%deF",
            true,
            true,
            true,
        );
        assert_eq!(result, "abc123def".cmp("abc456def"));

        let result = custom_cmp_str("ABCDEF", "abcdef", false, false, true);
        assert_eq!(result, Ordering::Equal);

        // 在C locale下测试字节比较行为
        unsafe {
            env::set_var("LC_COLLATE", "C");
        }
        let result = custom_cmp_str("ABCDEF", "abcdef", false, false, false);
        assert_eq!(result, Ordering::Less);

        // 恢复原始locale设置
        unsafe {
            env::remove_var("LC_COLLATE");
        }

        let result = custom_cmp_str(
            "abc123\x08\x00$%def",
            "abc123\x08\x00$%def",
            true,
            true,
            false,
        );
        assert_eq!(result, Ordering::Equal);
    }

    #[test]
    fn test_custom_str_cmp_single_character_strings() {
        let result = custom_cmp_str("a", "b", false, false, false);
        assert_eq!(result, Ordering::Less);

        let result = custom_cmp_str("b", "a", false, false, false);
        assert_eq!(result, Ordering::Greater);

        let result = custom_cmp_str("A", "a", true, false, false);
        assert_eq!(result, Ordering::Less);

        let result = custom_cmp_str("!", "@", false, true, false);
        assert_eq!(result, Ordering::Equal);

        let result = custom_cmp_str("\x00", "\x01", false, false, true);
        assert_eq!(result, Ordering::Less);
    }

    #[test]
    fn test_custom_str_cmp_large_strings() {
        let a = "a".repeat(1000);
        let b = "b".repeat(1000);

        let result = custom_cmp_str(&a, &b, false, false, false);
        assert_eq!(result, Ordering::Less);

        let result = custom_cmp_str(&b, &a, false, false, false);
        assert_eq!(result, Ordering::Greater);

        let result = custom_cmp_str(&a, &b, true, false, false);
        assert_eq!(result, Ordering::Less);

        let result = custom_cmp_str(&b, &a, true, false, false);
        assert_eq!(result, Ordering::Greater);

        let result = custom_cmp_str(&a, &b, false, true, false);
        assert_eq!(result, Ordering::Less);

        let result = custom_cmp_str(&b, &a, false, true, false);
        assert_eq!(result, Ordering::Greater);

        let result = custom_cmp_str(&a, &b, false, false, true);
        assert_eq!(result, Ordering::Less);

        let result = custom_cmp_str(&b, &a, false, false, true);
        assert_eq!(result, Ordering::Greater);
    }
}
