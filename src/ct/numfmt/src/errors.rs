/*
 *    Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved
 *     syskits is licensed under Mulan PSL v2.
 *    You can use this software according to the terms and conditions of the Mulan PSL V2
 *    You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 *    THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 *    KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 *    NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 *    See the Mulan PSL v2 for more details.
 *
 */

use ctcore::ct_error::CTError;
use std::{
    error::Error,
    fmt::{Debug, Display},
};

#[derive(Debug)]
pub enum NumfmtError {
    NumfmtIoError(String),
    NumfmtIllegalArgument(String),
    NumfmtFormattingError(String),
}

impl CTError for NumfmtError {
    fn code(&self) -> i32 {
        match self {
            NumfmtError::NumfmtIoError(_) => 1,
            NumfmtError::NumfmtIllegalArgument(_) => 1,
            NumfmtError::NumfmtFormattingError(_) => 2,
        }
    }
}

impl Error for NumfmtError {}

impl Display for NumfmtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NumfmtError::NumfmtIoError(err)
            | NumfmtError::NumfmtIllegalArgument(err)
            | NumfmtError::NumfmtFormattingError(err) => {
                write!(f, "{err}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(test)]
    mod ct_error_tests {
        use super::*;
        #[test]
        fn test_numfmt_error_code() {
            // NumfmtIoError
            let error1 = NumfmtError::NumfmtIoError("IO error".to_string());
            assert_eq!(error1.code(), 1);

            // NumfmtIllegalArgument
            let error2 = NumfmtError::NumfmtIllegalArgument("Illegal argument".to_string());
            assert_eq!(error2.code(), 1);

            // NumfmtFormattingError
            let error3 = NumfmtError::NumfmtFormattingError("Formatting error".to_string());
            assert_eq!(error3.code(), 2);
        }

        #[test]
        fn test_numfmt_error_code_additional_cases() {
            // Ensure that different instances of the same variant return the same code
            let error4a = NumfmtError::NumfmtIoError("First IO error".to_string());
            let error4b = NumfmtError::NumfmtIoError("Second IO error".to_string());
            assert_eq!(error4a.code(), error4b.code());

            // Check that a different variant returns a different code
            let error5 = NumfmtError::NumfmtIllegalArgument("Different error".to_string());
            let error6 = NumfmtError::NumfmtFormattingError("Another error".to_string());
            assert_ne!(error5.code(), error6.code());
        }
    }
    #[cfg(test)]
    mod display_for_error_tests {
        use super::*;
        #[test]
        fn test_numfmt_error_display() {
            let io_error = std::io::Error::new(std::io::ErrorKind::Other, "IO error");
            let illegal_argument = NumfmtError::NumfmtIllegalArgument("Invalid input".to_string());
            let formatting_error = NumfmtError::NumfmtFormattingError("Invalid format".to_string());

            let io_error_str = format!("{}", io_error);
            let illegal_argument_str = format!("{}", illegal_argument);
            let formatting_error_str = format!("{}", formatting_error);

            assert_eq!("IO error", io_error_str);
            assert_eq!("Invalid input", illegal_argument_str);
            assert_eq!("Invalid format", formatting_error_str);
        }

        #[test]
        fn test_numfmt_io_error_display() {
            // Test with different kinds of IO errors
            let permission_denied = "Permission Denied".to_string();
            let not_found = "File not found".to_string();

            let permission_denied_str =
                format!("{}", NumfmtError::NumfmtIoError(permission_denied));
            let not_found_str = format!("{}", NumfmtError::NumfmtIoError(not_found));

            assert_eq!("Permission Denied", permission_denied_str);
            assert_eq!("File not found", not_found_str);
        }

        #[test]
        fn test_numfmt_illegal_argument_display() {
            // Test with various illegal argument messages
            let empty_arg = NumfmtError::NumfmtIllegalArgument("".to_string());
            let complex_arg =
                NumfmtError::NumfmtIllegalArgument("Complex illegal argument string!".to_string());

            let empty_arg_str = format!("{}", empty_arg);
            let complex_arg_str = format!("{}", complex_arg);

            assert_eq!("", empty_arg_str);
            assert_eq!("Complex illegal argument string!", complex_arg_str);
        }

        #[test]
        fn test_numfmt_formatting_error_display() {
            // Test with different formatting error scenarios
            let generic_format_err =
                NumfmtError::NumfmtFormattingError("General formatting issue".to_string());
            let specific_format_err =
                NumfmtError::NumfmtFormattingError("Number too large".to_string());

            let generic_format_err_str = format!("{}", generic_format_err);
            let specific_format_err_str = format!("{}", specific_format_err);

            assert_eq!("General formatting issue", generic_format_err_str);
            assert_eq!("Number too large", specific_format_err_str);
        }
    }
}
