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

use ctcore::ct_format::{num_format::UnsignedInt, Format, FormatError};

use crate::csplit_error::CsplitError;

/// Computes the filename of a split, taking into consideration a possible user-defined suffix
/// ct_format.
pub struct SplitName {
    prefix: Vec<u8>,
    format: Format<UnsignedInt>,
}

impl SplitName {
    /// Creates a new SplitName with the given user-defined options:
    /// - `prefix_opt` specifies a prefix for all splits.
    /// - `format_opt` specifies a custom ct_format for the suffix part of the filename, using the
    /// `sprintf` ct_format notation.
    /// - `n_digits_opt` defines the width of the split number.
    ///
    /// # Caveats
    ///
    /// If `prefix_opt` and `format_opt` are defined, and the `format_opt` has some string appearing
    /// before the conversion pattern (e.g., "here-%05d"), then it is appended to the passed prefix
    /// via `prefix_opt`.
    ///
    /// If `n_digits_opt` and `format_opt` are defined, then width defined in `format_opt` is
    /// taken.
    pub fn new(
        prefix_opt_strs: Option<String>,
        format_opt_strs: Option<String>,
        n_digits_opt_strs: Option<String>,
    ) -> Result<Self, CsplitError> {
        // 获取前缀
        let prefix_opt_str = prefix_opt_strs.unwrap_or_else(|| "xx".to_string());

        let n_digits = n_digits_opt_strs
            .map(|opt| {
                opt.parse::<usize>()
                    .map_err(|_| CsplitError::InvalidNumber(opt))
            })
            .transpose()?
            .unwrap_or(2);

        let format_str = match format_opt_strs {
            Some(f) => f,
            None => format!("%0{n_digits}u"),
        };

        let format = match Format::<UnsignedInt>::parse(format_str) {
            Ok(format) => Ok(format),
            Err(FormatError::TooManySpecs(_)) => Err(CsplitError::SuffixFormatTooManyPercents),
            Err(_) => Err(CsplitError::SuffixFormatIncorrect),
        }?;

        Ok(Self {
            prefix: prefix_opt_str.as_bytes().to_owned(),
            format,
        })
    }

    /// Returns the filename of the i-th split.
    pub fn get(&self, n: usize) -> String {
        let mut v = self.prefix.clone();
        self.format.fmt(&mut v, n as u64).unwrap();
        String::from_utf8_lossy(&v).to_string()
    }
}

