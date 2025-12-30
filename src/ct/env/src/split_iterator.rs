/*
 *    Copyright(c) 2022-2024 China Telecom Cloud Technologies co., Ltd. All rights reserved
 *     syskits is licensed under Mulan PSL v2.
 *    You can use this software according to the terms and conditions of the Mulan PSL V2
 *    You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 *    THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 *    KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 *    NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 *    See the Mulan PSL v2 for more details.
 *
 */
//! Even though it looks quite like a POSIX syntax, the original
//! "shell_words" implementation had to be adapted significantly.
//!
//! Apart from the grammar differences, there is a new feature integrated: $VARIABLE expansion.
//!
//! [GNU env] <https://www.gnu.org/software/coreutils/manual/html_node/env-invocation.html#g_t_002dS_002f_002d_002dsplit_002dstring-syntax>

#![forbid(unsafe_code)]

use std::borrow::Cow;

use crate::native_int_str::from_native_int_representation;
use crate::native_int_str::NativeCharInt;
use crate::native_int_str::NativeIntStr;
use crate::native_int_str::NativeIntString;
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
                    })
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
                    })
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

