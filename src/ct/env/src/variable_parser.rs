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

use std::ops::Range;

use crate::{
    native_int_str::NativeIntStr, parse_error::EnvParseError, string_parser::StringParser,
};

pub struct VariableParser<'a, 'b> {
    pub parser: &'b mut StringParser<'a>,
}

impl<'a, 'b> VariableParser<'a, 'b> {
    /// 获取当前字符。
    fn get_current_char(&self) -> Option<char> {
        self.parser.peek().ok()
    }

    /// 检查变量名是否以数字开头，这是不允许的。
    fn check_variable_name_start(&self) -> Result<(), EnvParseError> {
        if let Some(c) = self.get_current_char() {
            if c.is_ascii_digit() {
                return Err(EnvParseError::ParsingOfVariableNameFailed {
                    pos: self.parser.get_peek_position(),
                    msg: format!("Unexpected character: '{}', expected variable name must not start with 0..9", c) });
            }
        }
        Ok(())
    }

    /// 跳过一个字符。
    fn skip_one(&mut self) -> Result<(), EnvParseError> {
        self.parser.consume_chunk()?;
        Ok(())
    }

    /// 解析用花括号包围的变量名及其可能的默认值。
    fn parse_braced_variable_name(
        &mut self,
    ) -> Result<(&'a NativeIntStr, Option<&'a NativeIntStr>), EnvParseError> {
        let position_start = self.parser.get_peek_position();

        self.check_variable_name_start()?;

        let (var_name_end, default_end);
        loop {
            match self.get_current_char() {
                None => {
                    return Err(EnvParseError::ParsingOfVariableNameFailed {
                        pos: self.parser.get_peek_position(), msg: "Missing closing brace".into() })
                },
                Some(c) if !c.is_ascii() || c.is_ascii_alphanumeric() || c == '_' => {
                    self.skip_one()?;
                }
                Some(':') => {
                    var_name_end = self.parser.get_peek_position();
                    loop {
                        match self.get_current_char() {
                            None => {
                                return Err(EnvParseError::ParsingOfVariableNameFailed {
                                    pos: self.parser.get_peek_position(),
                                    msg: "Missing closing brace after default value".into() })
                            },
                            Some('}') => {
                                default_end = Some(self.parser.get_peek_position());
                                self.skip_one()?;
                                break
                            },
                            Some(_) => {
                                self.skip_one()?;
                            },
                        }
                    }
                    break;
                },
                Some('}') => {
                    var_name_end = self.parser.get_peek_position();
                    default_end = None;
                    self.skip_one()?;
                    break;
                },
                Some(c) => {
                    return Err(EnvParseError::ParsingOfVariableNameFailed {
                        pos: self.parser.get_peek_position(),
                        msg: format!("Unexpected character: '{}', expected a closing brace ('}}') or colon (':')", c)
                    })
                },
            };
        }

        // 根据是否有默认值结束位置，来决定是否解析默认值。
        let default_option = if let Some(default_end) = default_end {
            Some(self.parser.substring(&Range {
                start: var_name_end + 1,
                end: default_end,
            }))
        } else {
            None
        };

        let varname = self.parser.substring(&Range {
            start: position_start,
            end: var_name_end,
        });

        Ok((varname, default_option))
    }

    /// 解析未用花括号包围的变量名。
    fn parse_unbraced_variable_name(&mut self) -> Result<&'a NativeIntStr, EnvParseError> {
        let position_start = self.parser.get_peek_position();

        self.check_variable_name_start()?;

        loop {
            match self.get_current_char() {
                None => break,
                Some(c) if c.is_ascii_alphanumeric() || c == '_' => {
                    self.skip_one()?;
                }
                Some(_) => break,
            };
        }

        let pos_end = self.parser.get_peek_position();

        if pos_end == position_start {
            return Err(EnvParseError::ParsingOfVariableNameFailed {
                pos: position_start,
                msg: "Missing variable name".into(),
            });
        }

        let var_name = self.parser.substring(&Range {
            start: position_start,
            end: pos_end,
        });

        Ok(var_name)
    }

    /// 解析变量，支持带花括号或不带花括号的变量名。
    pub fn parse_variable(
        &mut self,
    ) -> Result<(&'a NativeIntStr, Option<&'a NativeIntStr>), EnvParseError> {
        self.skip_one()?;

        let (name, default) = match self.get_current_char() {
            None => {
                return Err(EnvParseError::ParsingOfVariableNameFailed {
                    pos: self.parser.get_peek_position(),
                    msg: "missing variable name".into(),
                })
            }
            Some('{') => {
                self.skip_one()?;
                self.parse_braced_variable_name()?
            }
            Some(_) => (self.parse_unbraced_variable_name()?, None),
        };

        Ok((name, default))
    }
}
