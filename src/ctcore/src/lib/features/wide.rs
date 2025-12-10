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

use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
pub trait ToWide {
    fn to_wide(&self) -> Vec<u16>;
    fn to_wide_null(&self) -> Vec<u16>;
}
impl<T> ToWide for T
where
    T: AsRef<OsStr>,
{
    fn to_wide(&self) -> Vec<u16> {
        self.as_ref().encode_wide().collect()
    }
    fn to_wide_null(&self) -> Vec<u16> {
        self.as_ref().encode_wide().chain(Some(0)).collect()
    }
}
pub trait FromWide {
    fn from_wide(wide: &[u16]) -> Self;
    fn from_wide_null(wide: &[u16]) -> Self;
}

impl FromWide for String {
    fn from_wide(wide: &[u16]) -> Self {
        if wide.is_empty() {
            return String::new();
        }

        match OsString::from_wide(wide).to_string_lossy().to_string() {
            Ok(s) => s,
            Err(_) => String::new(),
        }
    }

    fn from_wide_null(wide: &[u16]) -> Self {
        let len = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
        let os_string = OsString::from_wide(&wide[..len])
            .to_string_lossy()
            .to_string();

        match os_string {
            Ok(s) => s,
            Err(_) => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_wide() {
        // 测试非空字符串转宽字符
        let test_str = "Test string";
        let wide_vec = test_str.to_wide();
        assert_ne!(wide_vec, Vec::<u16>::new());
        // 验证转换后的内容可以通过 OsString 还原为原始字符串
        let os_string: OsString = OsString::from_wide(&wide_vec);
        assert_eq!(test_str, os_string.to_string_lossy());

        // 测试空字符串转宽字符
        let empty_str = "";
        let empty_wide_vec = empty_str.to_wide();
        assert_eq!(empty_wide_vec, Vec::<u16>::new());
    }

    #[test]
    fn test_to_wide_null() {
        // 测试非空字符串转带null终止符的宽字符
        let test_str = "Test string";
        let wide_null_vec = test_str.to_wide_null();
        // 验证末尾有无null终止符
        assert_eq!(wide_null_vec.last(), Some(&0));
        // 验证转换后的内容可以通过 OsString 还原为原始字符串
        let os_string: OsString = OsString::from_wide(&wide_null_vec);
        assert_eq!(test_str, os_string.to_string_lossy());

        // 测试空字符串转带null终止符的宽字符
        let empty_str = "";
        let empty_wide_null_vec = empty_str.to_wide_null();
        assert_eq!(empty_wide_null_vec, vec![0]);
    }

    #[test]
    fn test_from_wide() {
        // 测试从宽字符数组转回字符串
        let wide_vec: Vec<u16> = OsStr::new("テスト文字列").encode_wide().collect();
        let restored_str: String = FromWide::from_wide(&wide_vec);
        assert_eq!(restored_str, "テスト文字列");

        // 测试包含无效Unicode序列的情况
        let invalid_wide_vec = vec![0x1234, 0x5678];
        let restored_invalid_str: String = FromWide::from_wide(&invalid_wide_vec);
        // 此处断言取决于具体的转码策略，通常会转码为问号或其他占位符
        assert!(restored_invalid_str.contains("?"));
    }

    #[test]
    fn test_from_wide_null() {
        // 测试从带null终止符的宽字符数组转回字符串
        let wide_null_vec: Vec<u16> = OsStr::new("テスト文字列\0")
            .encode_wide()
            .chain(Some(0))
            .collect();
        let restored_str: String = FromWide::from_wide_null(&wide_null_vec);
        assert_eq!(restored_str, "テスト文字列");

        // 测试包含多个null终止符的情况
        let multiple_nulls_wide_vec: Vec<u16> = [0x65E5, 0x672C, 0x8A9E, 0x0000, 0x0000].to_vec();
        let restored_multiple_nulls_str: String =
            FromWide::from_wide_null(&multiple_nulls_wide_vec);
        assert_eq!(restored_multiple_nulls_str, "日本語");
    }
}
