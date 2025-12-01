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
//! Use the [`from_str`] function to parse a [`Pattern`] from a string.

// cSpell:words fnmatch

use glob::{Pattern, PatternError};

fn fix_negation(input: &str) -> String {
    // let mut chars = input.chars().collect::<Vec<_>>();
    //
    // let mut i = 0;
    // // Add 3 to prevent out of bounds in loop
    // while i + 3 < chars.len() {
    //     if chars[i] == '[' && chars[i + 1] == '^' {
    //         match chars[i + 3..].iter().position(|x| *x == ']') {
    //             None => {
    //                 // if closing square bracket not found, stop looking for it
    //                 // again
    //                 break;
    //             }
    //             Some(j) => {
    //                 chars[i + 1] = '!';
    //                 i += j + 4;
    //                 continue;
    //             }
    //         }
    //     }
    //
    //     i += 1;
    // }
    //
    // chars.into_iter().collect::<String>()

    let mut characters = input.chars().collect::<Vec<_>>();
    let mut position = 0;

    while position + 3 < characters.len() {
        if characters[position] == '[' && characters[position + 1] == '^' {
            if let Some(match_index) = characters[position + 3..].iter().position(|&ch| ch == ']') {
                characters[position + 1] = '!'; // Change '^' to '!'
                position += match_index + 4; // Skip to the character after the closing ']'
                continue;
            } else {
                // if closing square bracket not found, stop looking for it
                // again
                break;
            }
        }
        position += 1;
    }

    characters.into_iter().collect()
}

/// Parse a glob Pattern from a string.
///
/// This function amends the input string to replace any caret or circumflex
/// character (^) used to negate a set of characters with an exclamation mark
/// (!), which adapts rust's glob matching to function the way the GNU utils'
/// fnmatch does.
///
/// # Examples
///
/// ```rust
/// use std::time::Duration;
/// use ctcore::ct_parse_glob::from_str;
/// assert!(!from_str("[^abc]").unwrap().matches("a"));
/// assert!(from_str("[^abc]").unwrap().matches("x"));
/// ```
pub fn from_str(glob: &str) -> Result<Pattern, PatternError> {
    Pattern::new(&fix_negation(glob))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base_from_str() {
        assert_eq!(Pattern::new("[!abc]").unwrap(), from_str("[^abc]").unwrap());
    }

    #[test]
    fn basic_negation_fix() {
        assert_eq!(fix_negation("[^abc]"), "[!abc]");
        assert_eq!(fix_negation("test[^xyz]more"), "test[!xyz]more");
    }

    #[test]
    fn multiple_negations() {
        assert_eq!(fix_negation("[^a][^b][^c]"), "[!a][!b][!c]");
    }

    #[test]
    fn edge_cases_with_special_characters() {
        assert_eq!(fix_negation("[^\\]^[]]"), "[!\\]^[]]");
        assert_eq!(fix_negation("abc[^]def"), "abc[^]def"); // Invalid negation not to be replaced
        assert_eq!(fix_negation("nothing_special"), "nothing_special");
    }

    #[test]
    fn escaped_characters() {
        assert_eq!(fix_negation("[^\\^abc]"), "[!\\^abc]");
        assert_eq!(fix_negation("[^\\]]"), "[!\\]]");
    }

    #[test]
    fn complex_patterns() {
        assert_eq!(fix_negation("foo[^]bar[^abc]baz"), "foo[!]bar[^abc]baz");
        assert_eq!(fix_negation("[^abc][def][^ghi]"), "[!abc][def][!ghi]");
    }

    #[test]
    fn nested_negations() {
        assert_eq!(fix_negation("[^[^abc]]"), "[![^abc]]");
    }

    #[test]
    fn no_change_needed() {
        assert_eq!(fix_negation("abc"), "abc");
        assert_eq!(fix_negation("[abc]"), "[abc]");
    }

    #[test]
    fn already_correct_negation() {
        assert_eq!(fix_negation("[!abc]"), "[!abc]");
    }

    #[test]
    fn stress_test_with_large_input() {
        let large_input = "[^abc]".repeat(10000);
        let expected_output = "[!abc]".repeat(10000);
        assert_eq!(fix_negation(&large_input), expected_output);
    }

    #[test]
    fn invalid_inputs() {
        assert_eq!(fix_negation("["), "[");
        assert_eq!(fix_negation("]"), "]");
        assert_eq!(fix_negation("[]"), "[]");
        assert_eq!(fix_negation("[^]"), "[^]"); // No valid characters to negate
    }

    #[test]
    fn test_base_fix_negation() {
        // Happy/Simple case
        assert_eq!("[!abc]", fix_negation("[^abc]"));

        // Should fix negations in a long regex
        assert_eq!("foo[abc]  bar[!def]", fix_negation("foo[abc]  bar[^def]"));

        // Should fix multiple negations in a regex
        assert_eq!("foo[!abc]bar[!def]", fix_negation("foo[^abc]bar[^def]"));

        // Should fix negation of the single character ]
        assert_eq!("[!]]", fix_negation("[^]]"));

        // Should fix negation of the single character ^
        assert_eq!("[!^]", fix_negation("[^^]"));

        // Should fix negation of the space character
        assert_eq!("[! ]", fix_negation("[^ ]"));

        // Complicated patterns
        assert_eq!("[!][]", fix_negation("[^][]"));
        assert_eq!("[![]]", fix_negation("[^[]]"));

        // More complex patterns that should be replaced
        assert_eq!("[[]] [!a]", fix_negation("[[]] [^a]"));
        assert_eq!("[[] [!a]", fix_negation("[[] [^a]"));
        assert_eq!("[]] [!a]", fix_negation("[]] [^a]"));

        // test that we don't look for closing square brackets unnecessarily
        // Verifies issue #5584
        let chars = "^[".repeat(174571);
        assert_eq!(chars, fix_negation(chars.as_str()));
    }

    #[test]
    fn test_base_fix_negation_should_not_amend() {
        assert_eq!("abc", fix_negation("abc"));

        // Regex specifically matches either [ or ^
        assert_eq!("[[^]", fix_negation("[[^]"));

        // Regex that specifically matches either space or ^
        assert_eq!("[ ^]", fix_negation("[ ^]"));

        // Regex that specifically matches either [, space or ^
        assert_eq!("[[ ^]", fix_negation("[[ ^]"));
        assert_eq!("[ [^]", fix_negation("[ [^]"));

        // Invalid globs (according to rust's glob implementation) will remain unamended
        assert_eq!("[^]", fix_negation("[^]"));
        assert_eq!("[^", fix_negation("[^"));
        assert_eq!("[][^]", fix_negation("[][^]"));

        // Issue #4479
        assert_eq!("ààà[^", fix_negation("ààà[^"));
    }
}
