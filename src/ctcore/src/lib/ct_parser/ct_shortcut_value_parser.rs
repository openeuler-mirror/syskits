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
use clap::{
    builder::{PossibleValue, TypedValueParser},
    error::{ContextKind, ContextValue, ErrorKind},
};

#[derive(Clone)]
pub struct CtShortcutValueParser(Vec<PossibleValue>);

/// ShortcutValueParser类似于clap库中的PossibleValuesParser，
/// 其作用是验证给定值是否来自于一组枚举的PossibleValue集合。
/// 与仅接受精确匹配值的PossibleValuesParser不同，
/// ShortcutValueParser还接受无歧义的快捷方式表示。
impl CtShortcutValueParser {
    pub fn new(values: impl Into<Self>) -> Self {
        values.into()
    }

    fn generate_clap_error(
        &self,
        command: &clap::Command,
        argument: Option<&clap::Arg>,
        input_value: &str,
    ) -> clap::Error {
        let mut error = clap::Error::new(ErrorKind::InvalidValue).with_cmd(command);

        // 如果指定了参数，将其作为 InvalidArg 添加到错误上下文中。
        if let Some(arg) = argument {
            error.insert(
                ContextKind::InvalidArg,
                ContextValue::String(arg.to_string()),
            );
        }

        // 将无效输入值添加到错误上下文中。
        error.insert(
            ContextKind::InvalidValue,
            ContextValue::String(input_value.to_string()),
        );

        // 收集并添加所有有效值到错误上下文中。
        error.insert(
            ContextKind::ValidValue,
            ContextValue::Strings(
                self.0
                    .iter()
                    .map(|value| value.get_name().to_string())
                    .collect(),
            ),
        );

        error
    }
}

impl TypedValueParser for CtShortcutValueParser {
    type Value = String;

    fn parse_ref(
        &self,
        command: &clap::Command,
        argument: Option<&clap::Arg>,
        input: &std::ffi::OsStr,
    ) -> Result<Self::Value, clap::Error> {
        let input_str = input
            .to_str()
            .ok_or_else(|| clap::Error::new(ErrorKind::InvalidUtf8))?;

        let filtered_values: Vec<_> = self
            .0
            .iter()
            .filter(|&possible_value| possible_value.get_name().starts_with(input_str))
            .collect();

        if filtered_values.is_empty() {
            Err(self.generate_clap_error(command, argument, input_str))
        } else if filtered_values.len() == 1 {
            Ok(filtered_values[0].get_name().to_string())
        } else if let Some(direct_match) = filtered_values
            .iter()
            .find(|&&value| value.get_name() == input_str)
        {
            Ok(direct_match.get_name().to_string())
        } else {
            Err(self.generate_clap_error(command, argument, input_str))
        }
    }

    fn possible_values(&self) -> Option<Box<dyn Iterator<Item = PossibleValue> + '_>> {
        Some(Box::new(self.0.iter().cloned()))
    }
}

impl<I, T> From<I> for CtShortcutValueParser
where
    I: IntoIterator<Item = T>,
    T: Into<PossibleValue>,
{
    fn from(values: I) -> Self {
        Self(values.into_iter().map(|t| t.into()).collect())
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use clap::{builder::TypedValueParser, error::ErrorKind, Command};

    use super::CtShortcutValueParser;

    #[cfg(test)]
    mod tests {
        use super::*;
        use clap::{builder::PossibleValue, Arg, Command};

        use clap::error::ErrorKind;

        #[test]
        fn test_generate_clap_error_basic() {
            let parser = CtShortcutValueParser(vec![
                PossibleValue::new("option1"),
                PossibleValue::new("option2"),
            ]);

            let cmd = Command::new("test");
            let arg = Arg::new("test_arg");

            let error = parser.generate_clap_error(&cmd, Some(&arg), "option3");

            assert_eq!(error.kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_ambiguous_values() {
            let parser = CtShortcutValueParser(vec![
                PossibleValue::new("start"),
                PossibleValue::new("stop"),
                PossibleValue::new("state"),
            ]);

            let cmd = Command::new("test");
            let arg = Arg::new("test_arg");

            let error = parser.generate_clap_error(&cmd, Some(&arg), "st");

            assert_eq!(error.kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_exact_match_but_not_in_list() {
            let parser = CtShortcutValueParser(vec![
                PossibleValue::new("start"),
                PossibleValue::new("restart"),
            ]);

            let cmd = Command::new("test");
            let arg = Arg::new("test_arg");

            let error = parser.generate_clap_error(&cmd, Some(&arg), "stop");

            assert_eq!(error.kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_empty_input() {
            let parser = CtShortcutValueParser(vec![
                PossibleValue::new("start"),
                PossibleValue::new("stop"),
            ]);

            let cmd = Command::new("test");

            let error = parser.generate_clap_error(&cmd, None, "");

            assert_eq!(error.kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_very_long_input() {
            let parser = CtShortcutValueParser(vec![
                PossibleValue::new("start"),
                PossibleValue::new("stop"),
            ]);

            let cmd = Command::new("test");
            let long_input = "a".repeat(1000); // Arbitrary long input

            let error = parser.generate_clap_error(&cmd, None, &long_input);

            assert_eq!(error.kind(), ErrorKind::InvalidValue);
        }

        #[test]
        fn test_input_with_special_characters() {
            let parser = CtShortcutValueParser(vec![
                PossibleValue::new("start"),
                PossibleValue::new("stop"),
            ]);

            let cmd = Command::new("test");

            let error = parser.generate_clap_error(&cmd, None, "st@rt");

            assert_eq!(error.kind(), ErrorKind::InvalidValue);
        }
    }

    #[test]
    fn test_parse_ref() {
        let cmd = Command::new("cmd");
        let parser = CtShortcutValueParser::new(["abcd"]);
        let values = ["a", "ab", "abc", "abcd"];

        for value in values {
            let result = parser.parse_ref(&cmd, None, OsStr::new(value));
            assert_eq!("abcd", result.unwrap());
        }
    }

    #[test]
    fn test_parse_ref_with_invalid_value() {
        let cmd = Command::new("cmd");
        let parser = CtShortcutValueParser::new(["abcd"]);
        let invalid_values = ["e", "abe", "abcde"];

        for invalid_value in invalid_values {
            let result = parser.parse_ref(&cmd, None, OsStr::new(invalid_value));
            assert_eq!(ErrorKind::InvalidValue, result.unwrap_err().kind());
        }
    }

    #[test]
    fn test_parse_ref_with_ambiguous_value() {
        let cmd = Command::new("cmd");
        let parser = CtShortcutValueParser::new(["abcd", "abef"]);
        let ambiguous_values = ["a", "ab"];

        for ambiguous_value in ambiguous_values {
            let result = parser.parse_ref(&cmd, None, OsStr::new(ambiguous_value));
            assert_eq!(ErrorKind::InvalidValue, result.unwrap_err().kind());
        }

        let result = parser.parse_ref(&cmd, None, OsStr::new("abc"));
        assert_eq!("abcd", result.unwrap());

        let result = parser.parse_ref(&cmd, None, OsStr::new("abe"));
        assert_eq!("abef", result.unwrap());
    }

    #[test]
    fn test_parse_ref_with_ambiguous_value_that_is_a_possible_value() {
        let cmd = Command::new("cmd");
        let parser = CtShortcutValueParser::new(["abcd", "abcdefgh"]);
        let result = parser.parse_ref(&cmd, None, OsStr::new("abcd"));
        assert_eq!("abcd", result.unwrap());
    }

    #[test]
    #[cfg(unix)]
    fn test_parse_ref_with_invalid_utf8() {
        use std::os::unix::prelude::OsStrExt;

        let parser = CtShortcutValueParser::new(["abcd"]);
        let cmd = Command::new("cmd");

        let result = parser.parse_ref(&cmd, None, OsStr::from_bytes(&[0xc3, 0x28]));
        assert_eq!(ErrorKind::InvalidUtf8, result.unwrap_err().kind());
    }
}
