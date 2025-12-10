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
//! Parsing a glob Pattern from a string.
//!
//! 使用from_str函数从字符串解析Pattern。

use glob::{Pattern, PatternError};

fn ct_fix_negation(ct_input: &str) -> String {
    let mut ct_characters = ct_input.chars().collect::<Vec<_>>();
    let mut ct_position = 0;

    while ct_position + 3 < ct_characters.len() {
        if ct_characters[ct_position] == '[' && ct_characters[ct_position + 1] == '^' {
            if let Some(match_index) = ct_characters[ct_position + 3..]
                .iter()
                .position(|&ch| ch == ']')
            {
                ct_characters[ct_position + 1] = '!'; // 改变 '^' 为 '!'
                ct_position += match_index + 4; // 跳过闭合']'后的字符
                continue;
            } else {
                // 如果未找到右方括号，停止再次寻找
                break;
            }
        }
        ct_position += 1;
    }

    ct_characters.into_iter().collect()
}

/// 从字符串解析 glob 模式。
///
/// 该函数修改输入字符串，将用于否定一组字符的尖括号（^）替换为感叹号（!），从而调整 Rust 的 glob 匹配行为以与 GNU 工具的 fnmatch 行为一致。
///
/// # 示例
///
/// ```rust
/// use std::time::Duration;
/// use ctcore::ct_parse_glob::ct_from_str;
/// assert!(!ct_from_str("[^abc]").unwrap().matches("a"));
/// assert!(ct_from_str("[^abc]").unwrap().matches("x"));
/// ```
pub fn ct_from_str(glob: &str) -> Result<Pattern, PatternError> {
    Pattern::new(&ct_fix_negation(glob))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base_from_str() {
        assert_eq!(
            Pattern::new("[!abc]").unwrap(),
            ct_from_str("[^abc]").unwrap()
        );
    }

    #[test]
    fn basic_negation_fix() {
        assert_eq!(ct_fix_negation("[^abc]"), "[!abc]");
        assert_eq!(ct_fix_negation("test[^xyz]more"), "test[!xyz]more");
    }

    #[test]
    fn multiple_negations() {
        assert_eq!(ct_fix_negation("[^a][^b][^c]"), "[!a][!b][!c]");
    }

    #[test]
    fn edge_cases_with_special_characters() {
        assert_eq!(ct_fix_negation("[^\\]^[]]"), "[!\\]^[]]");
        assert_eq!(ct_fix_negation("abc[^]def"), "abc[^]def"); // Invalid negation not to be replaced
        assert_eq!(ct_fix_negation("nothing_special"), "nothing_special");
    }

    #[test]
    fn escaped_characters() {
        assert_eq!(ct_fix_negation("[^\\^abc]"), "[!\\^abc]");
        assert_eq!(ct_fix_negation("[^\\]]"), "[!\\]]");
    }

    #[test]
    fn complex_patterns() {
        assert_eq!(ct_fix_negation("foo[^]bar[^abc]baz"), "foo[!]bar[^abc]baz");
        assert_eq!(ct_fix_negation("[^abc][def][^ghi]"), "[!abc][def][!ghi]");
    }

    #[test]
    fn nested_negations() {
        assert_eq!(ct_fix_negation("[^[^abc]]"), "[![^abc]]");
    }

    #[test]
    fn no_change_needed() {
        assert_eq!(ct_fix_negation("abc"), "abc");
        assert_eq!(ct_fix_negation("[abc]"), "[abc]");
    }

    #[test]
    fn already_correct_negation() {
        assert_eq!(ct_fix_negation("[!abc]"), "[!abc]");
    }

    #[test]
    fn stress_test_with_large_input() {
        let large_input = "[^abc]".repeat(10000);
        let expected_output = "[!abc]".repeat(10000);
        assert_eq!(ct_fix_negation(&large_input), expected_output);
    }

    #[test]
    fn invalid_inputs() {
        assert_eq!(ct_fix_negation("["), "[");
        assert_eq!(ct_fix_negation("]"), "]");
        assert_eq!(ct_fix_negation("[]"), "[]");
        assert_eq!(ct_fix_negation("[^]"), "[^]"); // No valid characters to negate
    }

    #[test]
    fn test_base_fix_negation() {
        // Happy/Simple case
        assert_eq!("[!abc]", ct_fix_negation("[^abc]"));

        // Should fix negations in a long regex
        assert_eq!(
            "foo[abc]  bar[!def]",
            ct_fix_negation("foo[abc]  bar[^def]")
        );

        // Should fix multiple negations in a regex
        assert_eq!("foo[!abc]bar[!def]", ct_fix_negation("foo[^abc]bar[^def]"));

        // Should fix negation of the single character ]
        assert_eq!("[!]]", ct_fix_negation("[^]]"));

        // Should fix negation of the single character ^
        assert_eq!("[!^]", ct_fix_negation("[^^]"));

        // Should fix negation of the space character
        assert_eq!("[! ]", ct_fix_negation("[^ ]"));

        // Complicated patterns
        assert_eq!("[!][]", ct_fix_negation("[^][]"));
        assert_eq!("[![]]", ct_fix_negation("[^[]]"));

        // More complex patterns that should be replaced
        assert_eq!("[[]] [!a]", ct_fix_negation("[[]] [^a]"));
        assert_eq!("[[] [!a]", ct_fix_negation("[[] [^a]"));
        assert_eq!("[]] [!a]", ct_fix_negation("[]] [^a]"));

        // test that we don't look for closing square brackets unnecessarily
        // Verifies issue #5584
        let chars = "^[".repeat(174571);
        assert_eq!(chars, ct_fix_negation(chars.as_str()));
    }

    #[test]
    fn test_base_fix_negation_should_not_amend() {
        assert_eq!("abc", ct_fix_negation("abc"));

        // Regex specifically matches either [ or ^
        assert_eq!("[[^]", ct_fix_negation("[[^]"));

        // Regex that specifically matches either space or ^
        assert_eq!("[ ^]", ct_fix_negation("[ ^]"));

        // Regex that specifically matches either [, space or ^
        assert_eq!("[[ ^]", ct_fix_negation("[[ ^]"));
        assert_eq!("[ [^]", ct_fix_negation("[ [^]"));

        // Invalid globs (according to rust's glob implementation) will remain unamended
        assert_eq!("[^]", ct_fix_negation("[^]"));
        assert_eq!("[^", ct_fix_negation("[^"));
        assert_eq!("[][^]", ct_fix_negation("[][^]"));

        // Issue #4479
        assert_eq!("ààà[^", ct_fix_negation("ààà[^"));
    }

}