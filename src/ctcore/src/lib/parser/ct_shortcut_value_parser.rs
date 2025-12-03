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
pub struct ShortcutValueParser(Vec<PossibleValue>);

/// `ShortcutValueParser` is similar to clap's `PossibleValuesParser`: it verifies that the value is
/// from an enumerated set of `PossibleValue`.
///
/// Whereas `PossibleValuesParser` only accepts exact matches, `ShortcutValueParser` also accepts
/// shortcuts as long as they are unambiguous.
impl ShortcutValueParser {
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

        // If an argument is specified, add it to the error context as `InvalidArg`.
        if let Some(arg) = argument {
            error.insert(
                ContextKind::InvalidArg,
                ContextValue::String(arg.to_string()),
            );
        }

        // Add the invalid input value to the error context.
        error.insert(
            ContextKind::InvalidValue,
            ContextValue::String(input_value.to_string()),
        );

        // Collect and add all valid values to the error context.
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

impl TypedValueParser for ShortcutValueParser {
    type Value = String;

    fn parse_ref(
        &self,
        command: &clap::Command,
        argument: Option<&clap::Arg>,
        input: &std::ffi::OsStr,
    ) -> Result<Self::Value, clap::Error> {
        // let value = value
        //     .to_str()
        //     .ok_or(clap::Error::new(ErrorKind::InvalidUtf8))?;
        //
        // let matched_values: Vec<_> = self
        //     .0
        //     .iter()
        //     .filter(|x| x.get_name().starts_with(value))
        //     .collect();
        //
        // match matched_values.len() {
        //     0 => Err(self.generate_clap_error(cmd, arg, value)),
        //     1 => Ok(matched_values[0].get_name().to_string()),
        //     _ => {
        //         if let Some(direct_match) = matched_values.iter().find(|x| x.get_name() == value) {
        //             Ok(direct_match.get_name().to_string())
        //         } else {
        //             Err(self.generate_clap_error(cmd, arg, value))
        //         }
        //     }
        // }
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

impl<I, T> From<I> for ShortcutValueParser
where
    I: IntoIterator<Item = T>,
    T: Into<PossibleValue>,
{
    fn from(values: I) -> Self {
        Self(values.into_iter().map(|t| t.into()).collect())
    }
}

