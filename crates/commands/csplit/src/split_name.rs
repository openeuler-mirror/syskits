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

use ctcore::ct_format::{Format, FormatError, num_format::UnsignedInt};
use std::ffi::OsString;
use std::num::IntErrorKind;
use std::path::PathBuf;

use crate::csplit_error::CsplitError;

/// Computes the filename of a split, taking into consideration a possible user-defined suffix
/// format.
pub struct SplitName {
    prefix: Vec<u8>,
    format: Format<UnsignedInt>,
}

impl SplitName {
    /// Creates a new SplitName with the given user-defined options:
    /// - `prefix_opt` specifies a prefix for all splits.
    /// - `format_opt` specifies a custom format for the suffix part of the filename, using the
    ///   `sprintf` format notation.
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
    #[cfg(test)]
    pub fn new(
        prefix_opt_strs: Option<String>,
        format_opt_strs: Option<String>,
        n_digits_opt_strs: Option<String>,
    ) -> Result<Self, CsplitError> {
        Self::new_with_prefix(
            prefix_opt_strs.map(OsString::from),
            format_opt_strs.map(OsString::from),
            n_digits_opt_strs,
        )
    }

    pub fn new_with_prefix(
        prefix_opt: Option<OsString>,
        format_opt: Option<OsString>,
        n_digits_opt_strs: Option<String>,
    ) -> Result<Self, CsplitError> {
        let prefix = prefix_opt.unwrap_or_else(|| OsString::from("xx"));

        let n_digits = parse_digits_option(n_digits_opt_strs.as_deref())?.unwrap_or(2);

        let format = match format_opt {
            Some(format) => os_string_to_bytes(&format),
            None => format!("%0{n_digits}u").into_bytes(),
        };

        validate_suffix_format(&format)?;
        let format = match Format::<UnsignedInt>::parse(&format) {
            Ok(format) => Ok(format),
            Err(FormatError::TooManySpecs(_)) => Err(CsplitError::SuffixFormatTooManyPercents),
            Err(_) => Err(CsplitError::SuffixFormatIncorrect),
        }?;

        Ok(Self {
            prefix: os_string_to_bytes(&prefix),
            format,
        })
    }

    /// Returns the filename of the i-th split.
    #[cfg(test)]
    pub fn get(&self, n: usize) -> String {
        self.get_path(n).to_string_lossy().into_owned()
    }

    pub fn get_path(&self, n: usize) -> PathBuf {
        let mut v = self.prefix.clone();
        self.format.fmt(&mut v, n as u64).unwrap();
        bytes_to_path(v)
    }
}

fn os_string_to_bytes(value: &OsString) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;

        value.as_os_str().as_bytes().to_owned()
    }

    #[cfg(not(unix))]
    value.to_string_lossy().as_bytes().to_owned()
}

fn bytes_to_path(bytes: Vec<u8>) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;

        PathBuf::from(OsString::from_vec(bytes))
    }

    #[cfg(not(unix))]
    PathBuf::from(String::from_utf8_lossy(&bytes).into_owned())
}

fn parse_digits(option: String) -> Result<usize, CsplitError> {
    let value = option.trim_start_matches(|character: char| character.is_ascii_whitespace());
    let digits = match value.parse::<i64>() {
        Ok(digits) => digits,
        Err(error)
            if matches!(
                error.kind(),
                IntErrorKind::PosOverflow | IntErrorKind::NegOverflow
            ) =>
        {
            return Err(CsplitError::InvalidNumberTooLarge(option));
        }
        Err(_) => return Err(CsplitError::InvalidNumber(option)),
    };

    if digits < 0 {
        // This mirrors GNU xdectoimax's ERANGE versus EOVERFLOW threshold.
        return Err(if digits < i64::from(i32::MIN) / 2 {
            CsplitError::InvalidNumberTooLarge(option)
        } else {
            CsplitError::InvalidNumberOutOfRange(option)
        });
    }
    if digits > i64::from(i32::MAX) {
        return Err(CsplitError::InvalidNumberTooLarge(option));
    }

    Ok(digits as usize)
}

pub(crate) fn parse_digits_option(option: Option<&str>) -> Result<Option<usize>, CsplitError> {
    option
        .map(|option| parse_digits(option.to_owned()))
        .transpose()
}

fn validate_suffix_format(format: impl AsRef<[u8]>) -> Result<(), CsplitError> {
    let bytes = format.as_ref();
    let mut position = 0;
    let mut conversion_found = false;

    while position < bytes.len() {
        if bytes[position] != b'%' {
            position += 1;
            continue;
        }
        position += 1;

        if position == bytes.len() {
            return Err(CsplitError::SuffixFormatMissingSpecifier);
        }
        if bytes[position] == b'%' {
            position += 1;
            continue;
        }
        if conversion_found {
            return Err(CsplitError::SuffixFormatTooManyPercents);
        }
        conversion_found = true;

        let mut alternative = false;
        let mut thousands = false;
        while let Some(flag) = bytes.get(position) {
            match flag {
                b'-' | b'0' => position += 1,
                b'#' => {
                    alternative = true;
                    position += 1;
                }
                b'\'' => {
                    thousands = true;
                    position += 1;
                }
                _ => break,
            }
        }
        while bytes.get(position).is_some_and(u8::is_ascii_digit) {
            position += 1;
        }
        if bytes.get(position) == Some(&b'.') {
            position += 1;
            while bytes.get(position).is_some_and(u8::is_ascii_digit) {
                position += 1;
            }
        }

        let Some(&conversion) = bytes.get(position) else {
            return Err(CsplitError::SuffixFormatMissingSpecifier);
        };
        let allowed_flags = match conversion {
            b'd' | b'i' | b'u' => thousands && !alternative,
            b'o' | b'x' | b'X' => alternative && !thousands,
            _ => {
                return Err(CsplitError::SuffixFormatInvalidSpecifier(
                    suffix_conversion_description(conversion),
                ));
            }
        };
        if (alternative || thousands) && !allowed_flags {
            let invalid_flag = if alternative { '#' } else { '\'' };
            return Err(CsplitError::SuffixFormatInvalidFlags(format!(
                "%{invalid_flag}{}",
                conversion as char
            )));
        }
        position += 1;
    }

    if conversion_found {
        Ok(())
    } else {
        Err(CsplitError::SuffixFormatMissingConversion)
    }
}

fn suffix_conversion_description(conversion: u8) -> String {
    if conversion.is_ascii_graphic() || conversion == b' ' {
        (conversion as char).to_string()
    } else {
        format!("\\{conversion:03o}")
    }
}

#[cfg(test)]
mod tests {
    use crate::csplit_error::CsplitError;
    use crate::split_name::{SplitName, validate_suffix_format};

    #[test]
    fn test_split_name_new_default_prefix_and_format() {
        let split_name = SplitName::new(None, None, None).unwrap();
        assert_eq!("xx", String::from_utf8(split_name.prefix).unwrap());
    }

    #[test]
    fn test_split_name_new_custom_prefix_no_format() {
        let split_name = SplitName::new(Some("abc".to_string()), None, None).unwrap();
        assert_eq!("abc", String::from_utf8(split_name.prefix).unwrap());
    }

    #[test]
    fn test_split_name_new_no_prefix_custom_format() {
        let split_name = SplitName::new(None, Some("%03u".to_string()), None).unwrap();
        assert_eq!("xx", String::from_utf8(split_name.prefix).unwrap());
    }

    #[test]
    fn test_split_name_new_no_prefix_no_format_custom_n_digits() {
        let split_name = SplitName::new(None, None, Some("3".to_string())).unwrap();
        assert_eq!("xx", String::from_utf8(split_name.prefix).unwrap());
    }

    #[test]
    fn test_split_name_new_custom_prefix_format_and_n_digits() {
        let split_name = SplitName::new(
            Some("def".to_string()),
            Some("ghi-%04d".to_string()),
            Some("5".to_string()),
        )
        .unwrap();
        assert_eq!("def", String::from_utf8(split_name.prefix).unwrap());
    }

    #[test]
    fn test_split_name_get_i_equals_0() {
        let split_name = SplitName::new(None, None, None).unwrap();
        let filename = split_name.get(0);
        assert_eq!("xx00", filename);
    }

    #[test]
    fn test_split_name_get_i_equals_10() {
        let split_name = SplitName::new(None, None, None).unwrap();
        let filename = split_name.get(10);
        assert_eq!("xx10", filename);
    }

    #[test]
    fn test_split_name_get_i_equals_100() {
        let split_name = SplitName::new(None, None, None).unwrap();
        let filename = split_name.get(100);
        assert_eq!("xx100", filename);
    }

    #[test]
    fn test_split_name_get_i_equals_42_with_custom_format() {
        let split_name = SplitName::new(None, Some(String::from("%03d")), None).unwrap();
        let filename = split_name.get(42);
        assert_eq!("xx042", filename);
    }

    #[test]
    fn invalid_number() {
        let split_name = SplitName::new(None, None, Some(String::from("bad")));
        match split_name {
            Err(CsplitError::InvalidNumber(_)) => (),
            _ => panic!("should fail with InvalidNumber"),
        };
    }

    #[test]
    fn digits_match_gnu_range_rules() {
        assert!(SplitName::new(None, None, Some("-0".into())).is_ok());
        assert!(SplitName::new(None, None, Some("+2".into())).is_ok());
        assert!(matches!(
            SplitName::new(None, None, Some("-1".into())),
            Err(CsplitError::InvalidNumberOutOfRange(_))
        ));
        assert!(matches!(
            SplitName::new(None, None, Some("-2147483648".into())),
            Err(CsplitError::InvalidNumberTooLarge(_))
        ));
        assert!(matches!(
            SplitName::new(None, None, Some("2147483648".into())),
            Err(CsplitError::InvalidNumberTooLarge(_))
        ));
    }

    #[test]
    fn digits_allow_leading_whitespace() {
        let split_name = SplitName::new(None, None, Some(String::from(" \t+3"))).unwrap();
        assert_eq!(split_name.get(2), "xx002");
    }

    #[test]
    fn invalid_suffix_format1() {
        let split_name = SplitName::new(None, Some(String::from("no conversion string")), None);
        match split_name {
            Err(CsplitError::SuffixFormatMissingConversion) => (),
            _ => panic!("should fail with SuffixFormatMissingConversion"),
        };
    }

    #[test]
    fn invalid_suffix_format2() {
        let split_name = SplitName::new(None, Some(String::from("%042a")), None);
        match split_name {
            Err(CsplitError::SuffixFormatInvalidSpecifier(specifier)) if specifier == "a" => (),
            _ => panic!("should fail with SuffixFormatInvalidSpecifier"),
        };
    }

    #[test]
    fn suffix_format_validation_matches_gnu_categories() {
        assert!(matches!(
            validate_suffix_format("%%"),
            Err(CsplitError::SuffixFormatMissingConversion)
        ));
        assert!(matches!(
            validate_suffix_format("%"),
            Err(CsplitError::SuffixFormatMissingSpecifier)
        ));
        assert!(matches!(
            validate_suffix_format("%llx"),
            Err(CsplitError::SuffixFormatInvalidSpecifier(specifier)) if specifier == "l"
        ));
        assert!(matches!(
            validate_suffix_format("%#d"),
            Err(CsplitError::SuffixFormatInvalidFlags(flags)) if flags == "%#d"
        ));
        assert!(matches!(
            validate_suffix_format("%'x"),
            Err(CsplitError::SuffixFormatInvalidFlags(flags)) if flags == "%'x"
        ));
        assert!(validate_suffix_format("%%-%0#6.3x").is_ok());
    }

    #[test]
    fn default_formatter() {
        let split_name = SplitName::new(None, None, None).unwrap();
        assert_eq!(split_name.get(2), "xx02");
    }

    #[test]
    fn default_formatter_with_prefix() {
        let split_name = SplitName::new(Some(String::from("aaa")), None, None).unwrap();
        assert_eq!(split_name.get(2), "aaa02");
    }

    #[test]
    fn default_formatter_with_width() {
        let split_name = SplitName::new(None, None, Some(String::from("5"))).unwrap();
        assert_eq!(split_name.get(2), "xx00002");
    }

    #[test]
    fn no_padding_decimal() {
        let split_name = SplitName::new(None, Some(String::from("cst-%d-")), None).unwrap();
        assert_eq!(split_name.get(2), "xxcst-2-");
    }

    #[test]
    fn zero_padding_decimal1() {
        let split_name = SplitName::new(None, Some(String::from("cst-%03d-")), None).unwrap();
        assert_eq!(split_name.get(2), "xxcst-002-");
    }

    #[test]
    fn zero_padding_decimal2() {
        let split_name = SplitName::new(
            Some(String::from("pre-")),
            Some(String::from("cst-%03d-post")),
            None,
        )
        .unwrap();
        assert_eq!(split_name.get(2), "pre-cst-002-post");
    }

    #[test]
    fn zero_padding_decimal3() {
        let split_name = SplitName::new(
            None,
            Some(String::from("cst-%03d-")),
            Some(String::from("42")),
        )
        .unwrap();
        assert_eq!(split_name.get(2), "xxcst-002-");
    }

    #[test]
    fn zero_padding_decimal4() {
        let split_name = SplitName::new(None, Some(String::from("cst-%03i-")), None).unwrap();
        assert_eq!(split_name.get(2), "xxcst-002-");
    }

    #[test]
    fn zero_padding_decimal5() {
        let split_name = SplitName::new(None, Some(String::from("cst-%03u-")), None).unwrap();
        assert_eq!(split_name.get(2), "xxcst-002-");
    }

    #[test]
    fn zero_padding_octal() {
        let split_name = SplitName::new(None, Some(String::from("cst-%03o-")), None).unwrap();
        assert_eq!(split_name.get(42), "xxcst-052-");
    }

    #[test]
    fn zero_padding_lower_hex() {
        let split_name = SplitName::new(None, Some(String::from("cst-%03x-")), None).unwrap();
        assert_eq!(split_name.get(42), "xxcst-02a-");
    }

    #[test]
    fn zero_padding_upper_hex() {
        let split_name = SplitName::new(None, Some(String::from("cst-%03X-")), None).unwrap();
        assert_eq!(split_name.get(42), "xxcst-02A-");
    }

    #[test]
    fn alternate_form_octal() {
        let split_name = SplitName::new(None, Some(String::from("cst-%#10o-")), None).unwrap();
        assert_eq!(split_name.get(42), "xxcst-       052-");
    }

    #[test]
    fn alternate_form_lower_hex() {
        let split_name = SplitName::new(None, Some(String::from("cst-%#10x-")), None).unwrap();
        assert_eq!(split_name.get(42), "xxcst-      0x2a-");
    }

    #[test]
    fn alternate_form_upper_hex() {
        let split_name = SplitName::new(None, Some(String::from("cst-%#10X-")), None).unwrap();
        assert_eq!(split_name.get(42), "xxcst-      0X2A-");
    }

    #[test]
    fn left_adjusted_decimal1() {
        let split_name = SplitName::new(None, Some(String::from("cst-%-10d-")), None).unwrap();
        assert_eq!(split_name.get(42), "xxcst-42        -");
    }

    #[test]
    fn left_adjusted_decimal2() {
        let split_name = SplitName::new(None, Some(String::from("cst-%-10i-")), None).unwrap();
        assert_eq!(split_name.get(42), "xxcst-42        -");
    }

    #[test]
    fn left_adjusted_decimal3() {
        let split_name = SplitName::new(None, Some(String::from("cst-%-10u-")), None).unwrap();
        assert_eq!(split_name.get(42), "xxcst-42        -");
    }

    #[test]
    fn left_adjusted_octal() {
        let split_name = SplitName::new(None, Some(String::from("cst-%-10o-")), None).unwrap();
        assert_eq!(split_name.get(42), "xxcst-52        -");
    }

    #[test]
    fn left_adjusted_lower_hex() {
        let split_name = SplitName::new(None, Some(String::from("cst-%-10x-")), None).unwrap();
        assert_eq!(split_name.get(42), "xxcst-2a        -");
    }

    #[test]
    fn left_adjusted_upper_hex() {
        let split_name = SplitName::new(None, Some(String::from("cst-%-10X-")), None).unwrap();
        assert_eq!(split_name.get(42), "xxcst-2A        -");
    }

    #[test]
    fn too_many_percent() {
        let split_name = SplitName::new(None, Some(String::from("%02d-%-3x")), None);
        match split_name {
            Err(CsplitError::SuffixFormatTooManyPercents) => (),
            _ => panic!("should fail with SuffixFormatTooManyPercents"),
        };
    }
}
