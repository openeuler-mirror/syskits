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
use std::cmp::Ordering;

/// Compares the non-digit parts of a version.
/// Special cases: ~ are before everything else, even ends ("a~" < "a")
/// Letters are before non-letters
fn version_non_digit_cmp(a: &str, b: &str) -> Ordering {
    let mut a_chars = a.chars();
    let mut b_chars = b.chars();
    loop {
        match (a_chars.next(), b_chars.next()) {
            (Some(c1), Some(c2)) if c1 == c2 => {}
            (None, None) => return Ordering::Equal,
            (_, Some('~')) => return Ordering::Greater,
            (Some('~'), _) => return Ordering::Less,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c1), Some(c2)) if c1.is_ascii_alphabetic() && !c2.is_ascii_alphabetic() => {
                return Ordering::Less
            }
            (Some(c1), Some(c2)) if !c1.is_ascii_alphabetic() && c2.is_ascii_alphabetic() => {
                return Ordering::Greater
            }
            (Some(c1), Some(c2)) => return c1.cmp(&c2),
        }
    }
}

/// Remove file endings matching the regex (\.[A-Za-z~][A-Za-z0-9~]*)*$
fn remove_file_ending(a: &str) -> &str {
    let mut ending_start = None;
    let mut prev_was_dot = false;
    for (idx, char) in a.char_indices() {
        if char == '.' {
            if ending_start.is_none() || prev_was_dot {
                ending_start = Some(idx);
            }
            prev_was_dot = true;
        } else if prev_was_dot {
            prev_was_dot = false;
            if !char.is_ascii_alphabetic() && char != '~' {
                ending_start = None;
            }
        } else if !char.is_ascii_alphanumeric() && char != '~' {
            ending_start = None;
        }
    }
    if prev_was_dot {
        ending_start = None;
    }
    if let Some(ending_start) = ending_start {
        &a[..ending_start]
    } else {
        a
    }
}

pub fn version_cmp(mut a: &str, mut b: &str) -> Ordering {
    let str_cmp = a.cmp(b);
    if str_cmp == Ordering::Equal {
        return str_cmp;
    }

    // Special cases:
    // 1. Empty strings
    match (a.is_empty(), b.is_empty()) {
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        (true, true) => unreachable!(),
        (false, false) => {}
    }
    // 2. Dots
    match (a == ".", b == ".") {
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        (true, true) => unreachable!(),
        (false, false) => {}
    }
    // 3. Two Dots
    match (a == "..", b == "..") {
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        (true, true) => unreachable!(),
        (false, false) => {}
    }
    // 4. Strings starting with a dot
    match (a.starts_with('.'), b.starts_with('.')) {
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        (true, true) => {
            // Strip the leading dot for later comparisons
            a = &a[1..];
            b = &b[1..];
        }
        _ => {}
    }

    // Try to strip file extensions
    let (mut a, mut b) = match (remove_file_ending(a), remove_file_ending(b)) {
        (a_stripped, b_stripped) if a_stripped == b_stripped => {
            // If both would be the same after stripping file extensions, don't strip them.
            (a, b)
        }
        stripped => stripped,
    };

    // 1. Compare leading non-numerical part
    // 2. Compare leading numerical part
    // 3. Repeat
    while !a.is_empty() || !b.is_empty() {
        let a_numerical_start = a.find(|c: char| c.is_ascii_digit()).unwrap_or(a.len());
        let b_numerical_start = b.find(|c: char| c.is_ascii_digit()).unwrap_or(b.len());

        let a_str = &a[..a_numerical_start];
        let b_str = &b[..b_numerical_start];

        match version_non_digit_cmp(a_str, b_str) {
            Ordering::Equal => {}
            ord => return ord,
        }

        a = &a[a_numerical_start..];
        b = &b[a_numerical_start..];

        let a_numerical_end = a.find(|c: char| !c.is_ascii_digit()).unwrap_or(a.len());
        let b_numerical_end = b.find(|c: char| !c.is_ascii_digit()).unwrap_or(b.len());

        let a_str = a[..a_numerical_end].trim_start_matches('0');
        let b_str = b[..b_numerical_end].trim_start_matches('0');

        match a_str.len().cmp(&b_str.len()) {
            Ordering::Equal => {}
            ord => return ord,
        }

        match a_str.cmp(b_str) {
            Ordering::Equal => {}
            ord => return ord,
        }

        a = &a[a_numerical_end..];
        b = &b[b_numerical_end..];
    }

    Ordering::Equal
}

#[cfg(test)]
mod tests {
    use crate::version_cmp::remove_file_ending;
    use crate::version_cmp::version_cmp;
    use crate::version_cmp::version_non_digit_cmp;

    use std::cmp::Ordering;
    #[test]
    fn test_version_cmp() {
        // Identical strings
        assert_eq!(version_cmp("hello", "hello"), Ordering::Equal);

        assert_eq!(version_cmp("file12", "file12"), Ordering::Equal);

        assert_eq!(
            version_cmp("file12-suffix", "file12-suffix"),
            Ordering::Equal
        );

        assert_eq!(
            version_cmp("file12-suffix24", "file12-suffix24"),
            Ordering::Equal
        );

        // Shortened names
        assert_eq!(version_cmp("world", "wo"), Ordering::Greater,);

        assert_eq!(version_cmp("hello10wo", "hello10world"), Ordering::Less,);

        // Simple names
        assert_eq!(version_cmp("world", "hello"), Ordering::Greater,);

        assert_eq!(version_cmp("hello", "world"), Ordering::Less);

        assert_eq!(version_cmp("apple", "ant"), Ordering::Greater);

        assert_eq!(version_cmp("ant", "apple"), Ordering::Less);

        // Uppercase letters
        assert_eq!(
            version_cmp("Beef", "apple"),
            Ordering::Less,
            "Uppercase letters are sorted before all lowercase letters"
        );

        assert_eq!(version_cmp("Apple", "apple"), Ordering::Less);

        assert_eq!(version_cmp("apple", "aPple"), Ordering::Greater);

        // Numbers
        assert_eq!(
            version_cmp("100", "20"),
            Ordering::Greater,
            "Greater numbers are greater even if they start with a smaller digit",
        );

        assert_eq!(
            version_cmp("20", "20"),
            Ordering::Equal,
            "Equal numbers are equal"
        );

        assert_eq!(
            version_cmp("15", "200"),
            Ordering::Less,
            "Small numbers are smaller"
        );

        // Comparing numbers with other characters
        assert_eq!(
            version_cmp("1000", "apple"),
            Ordering::Less,
            "Numbers are sorted before other characters"
        );

        assert_eq!(
            // spell-checker:disable-next-line
            version_cmp("file1000", "fileapple"),
            Ordering::Less,
            "Numbers in the middle of the name are sorted before other characters"
        );

        // Leading zeroes
        assert_eq!(
            version_cmp("012", "12"),
            Ordering::Equal,
            "A single leading zero does not make a difference"
        );

        assert_eq!(
            version_cmp("000800", "0000800"),
            Ordering::Equal,
            "Multiple leading zeros do not make a difference"
        );

        // Numbers and other characters combined
        assert_eq!(version_cmp("ab10", "aa11"), Ordering::Greater);

        assert_eq!(
            version_cmp("aa10", "aa11"),
            Ordering::Less,
            "Numbers after other characters are handled correctly."
        );

        assert_eq!(
            version_cmp("aa2", "aa100"),
            Ordering::Less,
            "Numbers after alphabetical characters are handled correctly."
        );

        assert_eq!(
            version_cmp("aa10bb", "aa11aa"),
            Ordering::Less,
            "Number is used even if alphabetical characters after it differ."
        );

        assert_eq!(
            version_cmp("aa10aa0010", "aa11aa1"),
            Ordering::Less,
            "Second number is ignored if the first number differs."
        );

        assert_eq!(
            version_cmp("aa10aa0010", "aa10aa1"),
            Ordering::Greater,
            "Second number is used if the rest is equal."
        );

        assert_eq!(
            version_cmp("aa10aa0010", "aa00010aa1"),
            Ordering::Greater,
            "Second number is used if the rest is equal up to leading zeroes of the first number."
        );

        assert_eq!(
            version_cmp("aa10aa0022", "aa010aa022"),
            Ordering::Equal,
            "Test multiple numeric values with leading zeros"
        );

        assert_eq!(
            version_cmp("file-1.4", "file-1.13"),
            Ordering::Less,
            "Periods are handled as normal text, not as a decimal point."
        );

        // Greater than u64::Max
        // u64 == 18446744073709551615 so this should be plenty:
        //        20000000000000000000000
        assert_eq!(
            version_cmp("aa2000000000000000000000bb", "aa002000000000000000000001bb"),
            Ordering::Less,
            "Numbers larger than u64::MAX are handled correctly without crashing"
        );

        assert_eq!(
            version_cmp("aa2000000000000000000000bb", "aa002000000000000000000000bb"),
            Ordering::Equal,
            "Leading zeroes for numbers larger than u64::MAX are \
            handled correctly without crashing"
        );

        assert_eq!(
            version_cmp("  a", "a"),
            Ordering::Greater,
            "Whitespace is after letters because letters are before non-letters"
        );

        assert_eq!(
            version_cmp("a~", "ab"),
            Ordering::Less,
            "A tilde is before other letters"
        );

        assert_eq!(
            version_cmp("a~", "a"),
            Ordering::Less,
            "A tilde is before the line end"
        );
        assert_eq!(
            version_cmp("~", ""),
            Ordering::Greater,
            "A tilde is after the empty string"
        );
        assert_eq!(
            version_cmp(".f", ".1"),
            Ordering::Greater,
            "if both start with a dot it is ignored for the comparison"
        );

        // The following tests are incompatible with GNU as of 2021/06.
        // I think that's because of a bug in GNU, reported as https://lists.gnu.org/archive/html/bug-coreutils/2021-06/msg00045.html
        assert_eq!(
            version_cmp("a..a", "a.+"),
            Ordering::Less,
            ".a is stripped before the comparison"
        );
        assert_eq!(
            version_cmp("a.", "a+"),
            Ordering::Greater,
            ". is not stripped before the comparison"
        );
        assert_eq!(
            version_cmp("a\0a", "a"),
            Ordering::Greater,
            "NULL bytes are handled comparison"
        );
    }
    #[test]
    fn test_remove_file_ending_no_dot() {
        let input = "filename";
        assert_eq!(remove_file_ending(input), "filename");
    }

    #[test]
    fn test_remove_single_dot_at_end() {
        let input = "filename.";
        assert_eq!(remove_file_ending(input), "filename.");
    }

    #[test]
    fn test_remove_multiple_dots_at_end() {
        let input = "filename...";
        assert_eq!(remove_file_ending(input), "filename...");
    }

    #[test]
    fn test_remove_extension_with_dot() {
        let input = "filename.txt";
        assert_eq!(remove_file_ending(input), "filename");
    }

    #[test]
    fn test_remove_extension_with_special_chars() {
        let input = "filename.tar.gz";
        assert_eq!(remove_file_ending(input), "filename");
    }

    #[test]
    fn test_remove_extension_with_tilde() {
        let input = "filename~backup.txt";
        assert_eq!(remove_file_ending(input), "filename~backup");
    }

    #[test]
    fn test_keep_special_characters_in_name() {
        let input = "file-name.123!";
        assert_eq!(remove_file_ending(input), "file-name.123!");
    }

    #[test]
    fn test_remove_file_ending_when_dot_not_followed_by_valid_chars() {
        let input = "filename...!";
        assert_eq!(remove_file_ending(input), "filename...!");
    }

    #[test]
    fn test_no_change_when_dot_inside_filename() {
        let input = "file.name.txt";
        assert_eq!(remove_file_ending(input), "file");
    }

    #[test]
    fn test_empty_string() {
        let input = "";
        assert_eq!(remove_file_ending(input), "");
    }

    #[test]
    fn test_version_cmp_equal() {
        assert_eq!(version_cmp("1.2.3", "1.2.3"), Ordering::Equal);
        assert_eq!(version_cmp("alpha1", "alpha1"), Ordering::Equal);
        assert_eq!(
            version_cmp("alpha.beta.gamma", "alpha.beta.gamma"),
            Ordering::Equal
        );
        assert_eq!(
            version_cmp("1alpha.2beta.3gamma", "1alpha.2beta.3gamma"),
            Ordering::Equal
        );
    }

    #[test]
    fn test_version_cmp_less() {
        assert_eq!(version_cmp("1.0", "1.1"), Ordering::Less);
        assert_eq!(version_cmp("alpha", "beta"), Ordering::Less);
        assert_eq!(version_cmp("alpha.beta", "alpha.beta.1"), Ordering::Less);
        assert_eq!(version_cmp("1.alpha.2", "1.alpha.3"), Ordering::Less);
        assert_eq!(version_cmp("1.2.3a", "1.2.3b"), Ordering::Less);
    }

    #[test]
    fn test_version_cmp_greater() {
        assert_eq!(version_cmp("1.1", "1.0"), Ordering::Greater);
        assert_eq!(version_cmp("beta", "alpha"), Ordering::Greater);
        assert_eq!(version_cmp("alpha.beta.1", "alpha.beta"), Ordering::Greater);
        assert_eq!(version_cmp("1.alpha.3", "1.alpha.2"), Ordering::Greater);
        assert_eq!(version_cmp("1.2.3b", "1.2.3a"), Ordering::Greater);
    }

    #[test]
    fn test_special_cases() {
        assert_eq!(version_cmp("", "1"), Ordering::Less);
        assert_eq!(version_cmp("1", ""), Ordering::Greater);
        assert_eq!(version_cmp(".", ""), Ordering::Greater);
        assert_eq!(version_cmp("", "."), Ordering::Less);
        assert_eq!(version_cmp("..", "."), Ordering::Greater);
        assert_eq!(version_cmp(".", ".."), Ordering::Less);

        // Leading dots
        assert_eq!(version_cmp(".1", "1"), Ordering::Less);
        assert_eq!(version_cmp("1", ".1"), Ordering::Greater);
        assert_eq!(version_cmp(".alpha", "alpha"), Ordering::Less);
        assert_eq!(version_cmp("alpha", ".alpha"), Ordering::Greater);
    }

    #[test]
    fn test_file_extension_stripping() {
        let a = "file-1.0.0.tar.gz";
        let b = "file-1.0.1.tar.gz";
        assert_eq!(version_cmp(a, b), Ordering::Less);
        assert_eq!(
            version_cmp(remove_file_ending(a), remove_file_ending(b)),
            Ordering::Less
        );

        // No stripping when extensions are not equal
        let a = "file-1.0.0.tar";
        let b = "file-1.0.1.gz";
        assert_eq!(
            version_cmp(a, b),
            version_cmp(remove_file_ending(a), remove_file_ending(b))
        );
    }

    #[test]
    fn test_leading_zeros_comparison() {
        assert_eq!(version_cmp("1.01", "1.1"), Ordering::Equal);
        assert_eq!(version_cmp("1.010", "1.1"), Ordering::Greater);
        assert_eq!(version_cmp("1.001", "1.01"), Ordering::Equal);
    }

    #[test]
    fn test_version_non_digit_cmp_equal() {
        assert_eq!(version_non_digit_cmp("alpha", "alpha"), Ordering::Equal);
        assert_eq!(version_non_digit_cmp("abc123", "abc123"), Ordering::Equal);
        assert_eq!(version_non_digit_cmp("~", "~"), Ordering::Equal);
    }

    #[test]
    fn test_version_non_digit_cmp_greater() {
        assert_eq!(version_non_digit_cmp("beta", "alpha"), Ordering::Greater);
        assert_eq!(version_non_digit_cmp("abc", "aaa"), Ordering::Greater);
        assert_eq!(version_non_digit_cmp("~", "any_character"), Ordering::Less);
        assert_eq!(
            version_non_digit_cmp("any_character", "~"),
            Ordering::Greater
        );
    }

    #[test]
    fn test_version_non_digit_cmp_special_cases() {
        assert_eq!(version_non_digit_cmp("any", ""), Ordering::Greater);
        assert_eq!(version_non_digit_cmp("", "any"), Ordering::Less);
        assert_eq!(version_non_digit_cmp("abc", "def~"), Ordering::Less);
        assert_eq!(version_non_digit_cmp("abc~", "def"), Ordering::Less);
    }

    #[test]
    fn test_version_non_digit_cmp_mixed_case() {
        assert_eq!(version_non_digit_cmp("AbC", "abc"), Ordering::Less);
        assert_eq!(version_non_digit_cmp("abC", "ABC"), Ordering::Greater);
        assert_eq!(version_non_digit_cmp("ABcd", "abcd"), Ordering::Less);
    }

}
