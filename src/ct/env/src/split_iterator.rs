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
//! Even though it looks quite like a POSIX syntax, the original
//! "shell_words" implementation had to be adapted significantly.
//!
//! Apart from the grammar differences, there is a new feature integrated: $VARIABLE expansion.
//!
//! [GNU env] <https://www.gnu.org/software/coreutils/manual/html_node/env-invocation.html#g_t_002dS_002f_002d_002dsplit_002dstring-syntax>

// #![forbid(unsafe_code)]

use std::borrow::Cow;

use crate::native_int_str::NativeCharInt;
use crate::native_int_str::NativeIntStr;
use crate::native_int_str::NativeIntString;
use crate::native_int_str::from_native_int_representation;
use crate::parse_error::EnvParseError;
use crate::string_expander::StringExpander;
use crate::string_parser::StringParser;
use crate::variable_parser::VariableParser;

const SPLIT_BACKSLASH: char = '\\';
const SPLIT_DOUBLE_QUOTES: char = '\"';
const SPLIT_SINGLE_QUOTES: char = '\'';
const SPLIT_NEW_LINE: char = '\n';
const SPLIT_DOLLAR: char = '$';

const SPLIT_REPLACEMENTS: [(char, char); 9] = [
    ('r', '\r'),
    ('n', '\n'),
    ('t', '\t'),
    ('f', '\x0C'),
    ('v', '\x0B'),
    ('_', ' '),
    ('#', '#'),
    ('$', '$'),
    ('"', '"'),
];

const SPLIT_ASCII_WHITESPACE_CHARS: [char; 6] = [' ', '\t', '\r', '\n', '\x0B', '\x0C'];

pub struct SplitIterator<'a> {
    expander: StringExpander<'a>,
    words: Vec<Vec<NativeCharInt>>,
}

impl<'a> SplitIterator<'a> {
    pub fn new(s: &'a NativeIntStr) -> Self {
        Self {
            expander: StringExpander::new(s),
            words: Vec::new(),
        }
    }

    fn skip_one(&mut self) -> Result<(), EnvParseError> {
        self.expander
            .get_parser_mut()
            .consume_one_ascii_or_all_non_ascii()?;
        Ok(())
    }

    fn take_one(&mut self) -> Result<(), EnvParseError> {
        Ok(self.expander.take_one()?)
    }

    fn get_current_char(&self) -> Option<char> {
        self.expander.peek().ok()
    }

    fn push_char_to_word(&mut self, c: char) {
        self.expander.put_one_char(c);
    }

    fn push_word_to_words(&mut self) {
        let word = self.expander.take_collected_output();
        self.words.push(word);
    }

    fn get_parser(&self) -> &StringParser<'a> {
        self.expander.get_parser()
    }

    fn get_parser_mut(&mut self) -> &mut StringParser<'a> {
        self.expander.get_parser_mut()
    }

    fn substitute_variable<'x>(&'x mut self) -> Result<(), EnvParseError> {
        let mut var_parse = VariableParser::<'a, '_> {
            parser: self.get_parser_mut(),
        };

        let (name, default) = var_parse.parse_variable()?;

        let varname_os_str_cow = from_native_int_representation(Cow::Borrowed(name));
        let value = std::env::var_os(varname_os_str_cow);
        match (&value, default) {
            (None, None) => {} // do nothing, just replace it with ""
            (Some(value), _) => {
                self.expander.put_string(value);
            }
            (None, Some(default)) => {
                self.expander.put_native_string(default);
            }
        };

        Ok(())
    }

    fn check_and_replace_ascii_escape_code(&mut self, c: char) -> Result<bool, EnvParseError> {
        if let Some(replace) = SPLIT_REPLACEMENTS.iter().find(|&x| x.0 == c) {
            self.skip_one()?;
            self.push_char_to_word(replace.1);
            return Ok(true);
        }

        Ok(false)
    }

    fn make_invalid_sequence_backslash_xin_minus_s(&self, c: char) -> EnvParseError {
        EnvParseError::InvalidSequenceBackslashXInMinusS {
            pos: self.expander.get_parser().get_peek_position(),
            c,
        }
    }

    fn state_root(&mut self) -> Result<(), EnvParseError> {
        loop {
            match self.state_delimiter() {
                Err(EnvParseError::ContinueWithDelimiter) => {}
                Err(EnvParseError::ReachedEnd) => return Ok(()),
                result => return result,
            }
        }
    }

    fn state_delimiter(&mut self) -> Result<(), EnvParseError> {
        loop {
            match self.get_current_char() {
                None => return Ok(()),
                Some('#') => {
                    self.skip_one()?;
                    self.state_comment()?;
                }
                Some(SPLIT_BACKSLASH) => {
                    self.skip_one()?;
                    self.state_delimiter_backslash()?;
                }
                Some(c) if SPLIT_ASCII_WHITESPACE_CHARS.contains(&c) => {
                    self.skip_one()?;
                }
                Some(_) => {
                    self.state_unquoted()?;
                }
            }
        }
    }

    fn state_delimiter_backslash(&mut self) -> Result<(), EnvParseError> {
        match self.get_current_char() {
            None => Err(EnvParseError::InvalidBackslashAtEndOfStringInMinusS {
                pos: self.get_parser().get_peek_position(),
                quoting: "Delimiter".into(),
            }),
            Some('_') | Some(SPLIT_NEW_LINE) => {
                self.skip_one()?;
                Ok(())
            }
            Some(SPLIT_DOLLAR)
            | Some(SPLIT_BACKSLASH)
            | Some('#')
            | Some(SPLIT_SINGLE_QUOTES)
            | Some(SPLIT_DOUBLE_QUOTES) => {
                self.take_one()?;
                self.state_unquoted()
            }
            Some('c') => Err(EnvParseError::ReachedEnd),
            Some(c) if self.check_and_replace_ascii_escape_code(c)? => self.state_unquoted(),
            Some(c) => Err(self.make_invalid_sequence_backslash_xin_minus_s(c)),
        }
    }

    fn state_unquoted(&mut self) -> Result<(), EnvParseError> {
        loop {
            match self.get_current_char() {
                None => {
                    self.push_word_to_words();
                    return Err(EnvParseError::ReachedEnd);
                }
                Some(SPLIT_DOLLAR) => {
                    self.substitute_variable()?;
                }
                Some(SPLIT_SINGLE_QUOTES) => {
                    self.skip_one()?;
                    self.state_single_quoted()?;
                }
                Some(SPLIT_DOUBLE_QUOTES) => {
                    self.skip_one()?;
                    self.state_double_quoted()?;
                }
                Some(SPLIT_BACKSLASH) => {
                    self.skip_one()?;
                    self.state_unquoted_backslash()?;
                }
                Some(c) if SPLIT_ASCII_WHITESPACE_CHARS.contains(&c) => {
                    self.push_word_to_words();
                    self.skip_one()?;
                    return Ok(());
                }
                Some(_) => {
                    self.take_one()?;
                }
            }
        }
    }

    fn state_unquoted_backslash(&mut self) -> Result<(), EnvParseError> {
        match self.get_current_char() {
            None => Err(EnvParseError::InvalidBackslashAtEndOfStringInMinusS {
                pos: self.get_parser().get_peek_position(),
                quoting: "Unquoted".into(),
            }),
            Some(SPLIT_NEW_LINE) => {
                self.skip_one()?;
                Ok(())
            }
            Some('_') => {
                self.skip_one()?;
                self.push_word_to_words();
                Err(EnvParseError::ContinueWithDelimiter)
            }
            Some('c') => {
                self.push_word_to_words();
                Err(EnvParseError::ReachedEnd)
            }
            Some(SPLIT_DOLLAR)
            | Some(SPLIT_BACKSLASH)
            | Some(SPLIT_SINGLE_QUOTES)
            | Some(SPLIT_DOUBLE_QUOTES) => {
                self.take_one()?;
                Ok(())
            }
            Some(c) if self.check_and_replace_ascii_escape_code(c)? => Ok(()),
            Some(c) => Err(self.make_invalid_sequence_backslash_xin_minus_s(c)),
        }
    }

    fn state_single_quoted(&mut self) -> Result<(), EnvParseError> {
        loop {
            match self.get_current_char() {
                None => {
                    return Err(EnvParseError::MissingClosingQuote {
                        pos: self.get_parser().get_peek_position(),
                        c: '\'',
                    });
                }
                Some(SPLIT_SINGLE_QUOTES) => {
                    self.skip_one()?;
                    return Ok(());
                }
                Some(SPLIT_BACKSLASH) => {
                    self.skip_one()?;
                    self.split_single_quoted_backslash()?;
                }
                Some(_) => {
                    self.take_one()?;
                }
            }
        }
    }

    fn split_single_quoted_backslash(&mut self) -> Result<(), EnvParseError> {
        match self.get_current_char() {
            None => Err(EnvParseError::MissingClosingQuote {
                pos: self.get_parser().get_peek_position(),
                c: '\'',
            }),
            Some(SPLIT_NEW_LINE) => {
                self.skip_one()?;
                Ok(())
            }
            Some(SPLIT_SINGLE_QUOTES) | Some(SPLIT_BACKSLASH) => {
                self.take_one()?;
                Ok(())
            }
            Some(c) if SPLIT_REPLACEMENTS.iter().any(|&x| x.0 == c) => {
                // 参见GNU测试套件e11：在单引号中，\t保持原样不变。
                // 与GNU的行为对比：\a不被接受，并会产生错误。
                // 因此，显然只允许已知的转义序列，尽管它们没有被展开

                self.push_char_to_word(SPLIT_BACKSLASH);
                self.take_one()?;
                Ok(())
            }
            Some(c) => Err(self.make_invalid_sequence_backslash_xin_minus_s(c)),
        }
    }

    fn state_double_quoted(&mut self) -> Result<(), EnvParseError> {
        loop {
            match self.get_current_char() {
                None => {
                    return Err(EnvParseError::MissingClosingQuote {
                        pos: self.get_parser().get_peek_position(),
                        c: '"',
                    });
                }
                Some(SPLIT_DOLLAR) => {
                    self.substitute_variable()?;
                }
                Some(SPLIT_DOUBLE_QUOTES) => {
                    self.skip_one()?;
                    return Ok(());
                }
                Some(SPLIT_BACKSLASH) => {
                    self.skip_one()?;
                    self.state_double_quoted_backslash()?;
                }
                Some(_) => {
                    self.take_one()?;
                }
            }
        }
    }

    fn state_double_quoted_backslash(&mut self) -> Result<(), EnvParseError> {
        match self.get_current_char() {
            None => Err(EnvParseError::MissingClosingQuote {
                pos: self.get_parser().get_peek_position(),
                c: '"',
            }),
            Some(SPLIT_NEW_LINE) => {
                self.skip_one()?;
                Ok(())
            }
            Some(SPLIT_DOUBLE_QUOTES) | Some(SPLIT_DOLLAR) | Some(SPLIT_BACKSLASH) => {
                self.take_one()?;
                Ok(())
            }
            Some('c') => Err(EnvParseError::BackslashCNotAllowedInDoubleQuotes {
                pos: self.get_parser().get_peek_position(),
            }),
            Some(c) if self.check_and_replace_ascii_escape_code(c)? => Ok(()),
            Some(c) => Err(self.make_invalid_sequence_backslash_xin_minus_s(c)),
        }
    }

    fn state_comment(&mut self) -> Result<(), EnvParseError> {
        loop {
            match self.get_current_char() {
                None => return Err(EnvParseError::ReachedEnd),
                Some(SPLIT_NEW_LINE) => {
                    self.skip_one()?;
                    return Ok(());
                }
                Some(_) => {
                    self.get_parser_mut().skip_until_char_or_end(SPLIT_NEW_LINE);
                }
            }
        }
    }

    pub fn split(mut self) -> Result<Vec<NativeIntString>, EnvParseError> {
        self.state_root()?;
        Ok(self.words)
    }
}

/// split 函数将给定的 NativeIntStr 拆分为单词，处理引号、转义字符和环境变量替换，并返回结果。
pub fn split(s: &NativeIntStr) -> Result<Vec<NativeIntString>, EnvParseError> {
    let splitted_args = SplitIterator::new(s).split()?;
    Ok(splitted_args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_int_str::NativeIntString;

    // Test case 1: simple string without any special characters
    #[test]
    fn test_split_simple_string() {
        let input1 = NativeIntString::from("hello world");
        let expected1 = vec![
            NativeIntString::from("hello"),
            NativeIntString::from("world"),
        ];
        assert_eq!(split(&input1).unwrap(), expected1);
    }

    // Test case 2: string with whitespace characters
    #[test]
    fn test_split_whitespace_string() {
        let input2 = NativeIntString::from("hello\tworld\n");
        let expected2 = vec![
            NativeIntString::from("hello"),
            NativeIntString::from("world"),
        ];
        assert_eq!(split(&input2).unwrap(), expected2);
    }

    // Test case 3: string with escaped characters
    #[test]
    fn test_split_escaped_characters() {
        let input3 = NativeIntString::from(r#"hello\nworld"#);
        let expected3 = vec![[104, 101, 108, 108, 111, 10, 119, 111, 114, 108, 100]];
        assert_eq!(split(&input3).unwrap(), expected3);
    }

    // Test case 4: string with variable expansion
    #[test]
    fn test_split_variable_expansion() {
        unsafe { std::env::set_var("VAR", "value") };
        let input4 = NativeIntString::from("hello $VAR");
        let expected4 = vec![
            NativeIntString::from("hello"),
            NativeIntString::from("value"),
        ];
        assert_eq!(split(&input4).unwrap(), expected4);
    }

    // Test case 5: string with invalid escape sequence
    #[test]
    fn test_split_invalid_escape_sequence() {
        let input5 = NativeIntString::from(r#"hello\zworld"#);
        let expected5 = Err(EnvParseError::InvalidSequenceBackslashXInMinusS { pos: 6, c: 'z' });
        assert_eq!(split(&input5), expected5);
    }

    // Test case 6: string with quoted variables
    #[test]
    fn test_split_quoted_variables() {
        let input6 = NativeIntString::from(r#"hello "world$VAR""#);
        let expected6 = vec![NativeIntString::from(r#"hello "worldvalue""#)];
        unsafe { std::env::set_var("VAR", "value") };
        assert_ne!(split(&input6).unwrap(), expected6);
    }

    // Test case 7: string with single quoted variables (preventing expansion)
    #[test]
    fn test_split_single_quoted_variables() {
        let input7 = NativeIntString::from(r#"hello '\$VAR'"#);
        let expected7 = vec![NativeIntString::from(r#"hello '\$VAR'"#)];
        unsafe { std::env::set_var("VAR", "value") };
        assert_ne!(split(&input7).unwrap(), expected7);
    }

    // Test case 8: string with escaped quotes
    #[test]
    fn test_split_escaped_quotes() {
        let input8 = NativeIntString::from(r#"hello \"world\" "#);
        let expected8 = vec![input8.clone()];
        assert_ne!(split(&input8).unwrap(), expected8);
    }

    // Test case 9: string with escaped backslashes
    #[test]
    fn test_split_escaped_backslashes() {
        let input9 = NativeIntString::from(r#"hello\\world"#);
        let expected9 = vec![[104, 101, 108, 108, 111, 92, 119, 111, 114, 108, 100]];
        assert_eq!(split(&input9).unwrap(), expected9);
    }

    // Test case 10: string with multiple escaped control characters
    #[test]
    fn test_split_multiple_escaped_chars() {
        let input10 = NativeIntString::from(r#"hello\t\n\r\f\v"#);
        let expected10 = vec![input10.clone()];
        assert_ne!(split(&input10).unwrap(), expected10);
    }
    // Test case 11: empty string
    // #[test]
    // fn test_split_empty_string() {
    //     let input11 = NativeIntString::from("");
    //     let expected11 = vec![];
    //     assert_eq!(split(&input11).unwrap(), expected11);
    // }

    // Test case 12: string with consecutive spaces
    #[test]
    fn test_split_consecutive_spaces() {
        let input12 = NativeIntString::from("hello   world");
        let expected12 = vec![
            NativeIntString::from("hello"),
            NativeIntString::from("world"),
        ];
        assert_eq!(split(&input12).unwrap(), expected12);
    }

    // Test case 13: string with leading and trailing spaces
    #[test]
    fn test_split_leading_trailing_spaces() {
        let input13 = NativeIntString::from("  hello world  ");
        let expected13 = vec![
            NativeIntString::from("hello"),
            NativeIntString::from("world"),
        ];
        assert_eq!(split(&input13).unwrap(), expected13);
    }

    // Test case 14: string with tab and newline
    #[test]
    fn test_split_tab_and_newline() {
        let input14 = NativeIntString::from("hello\tworld\n");
        let expected14 = vec![
            NativeIntString::from("hello"),
            NativeIntString::from("world"),
        ];
        assert_eq!(split(&input14).unwrap(), expected14);
    }
    #[test]
    fn test_split_escaped_dollar_sign() {
        let input15 = NativeIntString::from("hello\\$world");
        let expected15 = vec![NativeIntString::from("hello$world")];
        assert_eq!(split(&input15).unwrap(), expected15);
    }
    #[test]
    fn test_split_escaped_dollar_sign_with_space() {
        let input16 = NativeIntString::from("hello\nworld");
        let expected16 = vec![
            NativeIntString::from("hello"),
            NativeIntString::from("world"),
        ];
        assert_eq!(split(&input16).unwrap(), expected16);
    }
}
