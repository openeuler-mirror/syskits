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

