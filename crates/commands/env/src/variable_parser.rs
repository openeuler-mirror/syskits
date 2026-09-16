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

use crate::{
    native_int_str::NativeIntStr, parse_error::EnvParseError, string_parser::StringParser,
};

pub struct VariableParser<'a, 'b> {
    pub parser: &'b mut StringParser<'a>,
}

impl<'a> VariableParser<'a, '_> {
    /// 获取当前字符。
    fn get_current_char(&self) -> Option<char> {
        self.parser.peek().ok()
    }

    /// GNU env accepts only ASCII letters or underscore as the first name character.
    fn check_variable_name_start(&self) -> Result<(), EnvParseError> {
        match self.get_current_char() {
            Some(c) if c.is_ascii_alphabetic() || c == '_' => Ok(()),
            _ => Err(EnvParseError::ParsingOfVariableNameFailed {
                pos: self.parser.get_peek_position(),
                msg: "expected an ASCII letter or underscore".into(),
            }),
        }
    }

    /// 跳过一个字符。
    fn skip_one(&mut self) -> Result<(), EnvParseError> {
        self.parser.consume_chunk()?;
        Ok(())
    }

    /// Parse GNU env -S's ${VARNAME} syntax.
    fn parse_braced_variable_name(&mut self) -> Result<&'a NativeIntStr, EnvParseError> {
        let position_start = self.parser.get_peek_position();

        self.check_variable_name_start()?;

        loop {
            match self.get_current_char() {
                None => {
                    return Err(EnvParseError::ParsingOfVariableNameFailed {
                        pos: self.parser.get_peek_position(),
                        msg: "Missing closing brace".into(),
                    });
                }
                Some(c) if c.is_ascii_alphanumeric() || c == '_' => {
                    self.skip_one()?;
                }
                Some('}') => {
                    let var_name_end = self.parser.get_peek_position();
                    self.skip_one()?;
                    return Ok(self.parser.substring(&(position_start..var_name_end)));
                }
                Some(c) => {
                    return Err(EnvParseError::ParsingOfVariableNameFailed {
                        pos: self.parser.get_peek_position(),
                        msg: format!("unexpected character: '{c}'"),
                    });
                }
            };
        }
    }

    /// GNU env -S仅支持${VARNAME}形式的变量展开。
    pub fn parse_variable(
        &mut self,
    ) -> Result<(&'a NativeIntStr, Option<&'a NativeIntStr>), EnvParseError> {
        self.skip_one()?;

        let (name, default) = match self.get_current_char() {
            None => {
                return Err(EnvParseError::ParsingOfVariableNameFailed {
                    pos: self.parser.get_peek_position(),
                    msg: "missing variable name".into(),
                });
            }
            Some('{') => {
                self.skip_one()?;
                (self.parse_braced_variable_name()?, None)
            }
            Some(_) => {
                return Err(EnvParseError::ParsingOfVariableNameFailed {
                    pos: self.parser.get_peek_position(),
                    msg: "only ${VARNAME} expansion is supported".into(),
                });
            }
        };

        Ok((name, default))
    }
}
