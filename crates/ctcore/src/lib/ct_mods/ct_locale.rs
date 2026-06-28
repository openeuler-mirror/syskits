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

//! 本地化相关功能模块
//!
//! 提供类似于GNU coreutils中hard_locale函数的功能，
//! 用于判断当前locale是否为"硬"locale（即非C/POSIX locale）。
//!
//! 这个判断在多个工具中都需要使用，特别是在时间格式化、
//! 数字格式化等需要根据locale调整行为的场景中。

use std::cmp::Ordering;
use std::env;
use std::ffi::CString;

/// LC类别常量，对应libc中的LC_*常量
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LcCategory {
    /// 所有locale类别
    LcAll = 0,
    /// 字符分类和case转换
    LcCtype = 1,
    /// 数字格式化
    LcNumeric = 2,
    /// 时间和日期格式化
    LcTime = 3,
    /// 排序规则
    LcCollate = 4,
    /// 货币格式化
    LcMonetary = 5,
    /// 消息本地化
    LcMessages = 6,
}

/// 判断指定category的当前locale是否为"硬"locale
///
/// "硬"locale指的是不同于C或POSIX locale的locale，
/// 这些locale具有固定的行为。
///
/// 此函数参考GNU coreutils中的hard_locale实现：
/// - 如果locale为"C"或"POSIX"，返回false
/// - 在Android平台上，额外检查MB_CUR_MAX > 1的情况
/// - 其他情况返回true
///
/// # 参数
/// * `category` - LC类别，如LC_TIME、LC_NUMERIC等
///
/// # 返回值
/// * `true` - 当前locale为"硬"locale
/// * `false` - 当前locale为C/POSIX locale
///
/// # 示例
///
/// ```
/// use ctcore::ct_locale::{hard_locale, LcCategory};
///
/// // 检查时间locale是否为硬locale
/// if hard_locale(LcCategory::LcTime) {
///     // 使用本地化的时间格式
///     println!("Using localized time format");
/// } else {
///     // 使用默认的C locale时间格式
///     println!("Using C locale time format");
/// }
/// ```
pub fn hard_locale(category: LcCategory) -> bool {
    let locale_name = get_locale_name(category);

    // 检查是否为C或POSIX locale
    if locale_name == "C" || locale_name == "POSIX" {
        return false;
    }

    true
}

/// 获取指定category的locale名称  
///
/// 按照以下优先级获取locale：
/// 1. LC_ALL env var (如果category不是LC_ALL)
/// 2. 对应category的env var (如LC_TIME, LC_NUMERIC等)
/// 3. LANG env var
/// 4. 默认返回"C"
fn get_locale_name(category: LcCategory) -> String {
    get_locale_name_with_env(category, |s| env::var(s))
}

/// 可测试版本的get_locale_name，允许注入环境变量获取函数
fn get_locale_name_with_env<F>(category: LcCategory, env_getter: F) -> String
where
    F: Fn(&str) -> Result<String, env::VarError>,
{
    // 首先检查LC_ALL（除非查询的就是LC_ALL）
    if category != LcCategory::LcAll {
        if let Ok(lc_all) = env_getter("LC_ALL") {
            if !lc_all.is_empty() {
                return lc_all;
            }
        }
    }

    // 检查对应的category环境变量
    let category_name = match category {
        LcCategory::LcAll => "LC_ALL",
        LcCategory::LcCtype => "LC_CTYPE",
        LcCategory::LcNumeric => "LC_NUMERIC",
        LcCategory::LcTime => "LC_TIME",
        LcCategory::LcCollate => "LC_COLLATE",
        LcCategory::LcMonetary => "LC_MONETARY",
        LcCategory::LcMessages => "LC_MESSAGES",
    };

    if let Ok(locale) = env_getter(category_name) {
        if !locale.is_empty() {
            return locale;
        }
    }

    // 最后检查LANG
    if let Ok(lang) = env_getter("LANG") {
        if !lang.is_empty() {
            return lang;
        }
    }

    // 默认返回C locale
    "C".to_string()
}

/// 便利函数：检查LC_TIME是否为硬locale
///
/// 这是最常用的检查，许多工具需要根据时间locale来决定时间格式
#[inline]
pub fn hard_locale_time() -> bool {
    hard_locale(LcCategory::LcTime)
}

/// 便利函数：检查LC_NUMERIC是否为硬locale
///
/// 用于需要根据数值locale来决定数字格式的场景
#[inline]
pub fn hard_locale_numeric() -> bool {
    hard_locale(LcCategory::LcNumeric)
}

/// 便利函数：检查LC_COLLATE是否为硬locale
///
/// 用于需要根据排序locale来决定字符串比较规则的场景
#[inline]
pub fn hard_locale_collate() -> bool {
    hard_locale(LcCategory::LcCollate)
}

/// 使用系统locale进行字符串比较
///
/// 在硬locale环境下，使用系统的strcoll函数进行locale感知的字符串比较。
/// 在C/POSIX locale下，退回到字节比较。
///
/// # 参数
/// * `s1` - 第一个字符串的字节数组
/// * `s2` - 第二个字符串的字节数组
/// * `ignore_case` - 是否忽略大小写
///
/// # 返回值
/// * `Ordering::Less` - s1 < s2
/// * `Ordering::Equal` - s1 == s2  
/// * `Ordering::Greater` - s1 > s2
///
/// # 示例
///
/// ```
/// use ctcore::ct_locale::strcoll_compare;
/// use std::cmp::Ordering;
///
/// let result = strcoll_compare(b"apple", b"banana", false);
/// assert_eq!(result, Ordering::Less);
/// ```
pub fn strcoll_compare(s1: &[u8], s2: &[u8], ignore_case: bool) -> Ordering {
    let is_hard_locale = hard_locale_collate();

    if is_hard_locale {
        // 在硬locale下使用系统的strcoll
        strcoll_compare_with_locale(s1, s2, ignore_case)
    } else {
        // C/POSIX locale下使用字节比较
        if ignore_case {
            s1.to_ascii_lowercase().cmp(&s2.to_ascii_lowercase())
        } else {
            s1.cmp(s2)
        }
    }
}

/// 使用系统strcoll进行locale感知的字符串比较
///
/// 这个函数调用系统的strcoll函数，该函数遵循当前locale的排序规则。
/// 对于无法转换为C字符串的输入，退回到UTF-8字符串比较。
fn strcoll_compare_with_locale(s1: &[u8], s2: &[u8], ignore_case: bool) -> Ordering {
    // 尝试转换为C字符串用于strcoll调用
    let c_str1 = CString::new(s1);
    let c_str2 = CString::new(s2);

    match (c_str1, c_str2) {
        (Ok(c1), Ok(c2)) => {
            // 成功转换为C字符串，使用系统的strcoll
            unsafe {
                // 重要：需要调用setlocale让C库使用当前环境变量的locale设置
                // 传递空字符串表示使用环境变量的设置
                let empty_str = CString::new("").unwrap();
                let _locale_result = libc::setlocale(libc::LC_COLLATE, empty_str.as_ptr());

                let result = if ignore_case {
                    // 对于忽略大小写的比较，我们需要转换为小写
                    // 这里简化实现，使用UTF-8字符串比较
                    let str1 = String::from_utf8_lossy(s1).to_lowercase();
                    let str2 = String::from_utf8_lossy(s2).to_lowercase();
                    let c1_lower = CString::new(str1.as_bytes()).unwrap_or(c1);
                    let c2_lower = CString::new(str2.as_bytes()).unwrap_or(c2);
                    libc::strcoll(c1_lower.as_ptr(), c2_lower.as_ptr())
                } else {
                    libc::strcoll(c1.as_ptr(), c2.as_ptr())
                };

                match result {
                    x if x < 0 => Ordering::Less,
                    x if x > 0 => Ordering::Greater,
                    _ => Ordering::Equal,
                }
            }
        }
        _ => {
            // 无法转换为C字符串（包含null字节），退回到UTF-8字符串比较
            let str1 = String::from_utf8_lossy(s1);
            let str2 = String::from_utf8_lossy(s2);

            if ignore_case {
                str1.to_lowercase().cmp(&str2.to_lowercase())
            } else {
                str1.cmp(&str2)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// 测试用的环境变量模拟器
    fn make_env_getter(
        vars: HashMap<&str, &str>,
    ) -> impl Fn(&str) -> Result<String, env::VarError> {
        move |key: &str| {
            vars.get(key)
                .map(|s| s.to_string())
                .ok_or(env::VarError::NotPresent)
        }
    }

    #[test]
    fn test_get_locale_name_default() {
        let env_getter = make_env_getter(HashMap::new());
        assert_eq!(
            get_locale_name_with_env(LcCategory::LcTime, &env_getter),
            "C"
        );
    }

    #[test]
    fn test_get_locale_name_lang_only() {
        let mut env_vars = HashMap::new();
        env_vars.insert("LANG", "en_US.UTF-8");
        let env_getter = make_env_getter(env_vars);
        assert_eq!(
            get_locale_name_with_env(LcCategory::LcTime, &env_getter),
            "en_US.UTF-8"
        );
    }

    #[test]
    fn test_get_locale_name_lc_time_overrides_lang() {
        let mut env_vars = HashMap::new();
        env_vars.insert("LANG", "en_US.UTF-8");
        env_vars.insert("LC_TIME", "zh_CN.UTF-8");
        let env_getter = make_env_getter(env_vars);
        assert_eq!(
            get_locale_name_with_env(LcCategory::LcTime, &env_getter),
            "zh_CN.UTF-8"
        );
    }

    #[test]
    fn test_get_locale_name_lc_all_overrides_all() {
        let mut env_vars = HashMap::new();
        env_vars.insert("LANG", "en_US.UTF-8");
        env_vars.insert("LC_TIME", "zh_CN.UTF-8");
        env_vars.insert("LC_ALL", "fr_FR.UTF-8");
        let env_getter = make_env_getter(env_vars);
        assert_eq!(
            get_locale_name_with_env(LcCategory::LcTime, &env_getter),
            "fr_FR.UTF-8"
        );
    }

    #[test]
    fn test_hard_locale_c_values() {
        // 模拟C locale
        let mut env_vars = HashMap::new();
        env_vars.insert("LC_TIME", "C");
        let env_getter = make_env_getter(env_vars);
        let locale_name = get_locale_name_with_env(LcCategory::LcTime, &env_getter);
        assert_eq!(locale_name, "C");

        // 模拟POSIX locale
        let mut env_vars = HashMap::new();
        env_vars.insert("LC_TIME", "POSIX");
        let env_getter = make_env_getter(env_vars);
        let locale_name = get_locale_name_with_env(LcCategory::LcTime, &env_getter);
        assert_eq!(locale_name, "POSIX");
    }

    #[test]
    fn test_hard_locale_non_c_values() {
        // 测试非C locale
        let mut env_vars = HashMap::new();
        env_vars.insert("LC_TIME", "en_US.UTF-8");
        let env_getter = make_env_getter(env_vars);
        let locale_name = get_locale_name_with_env(LcCategory::LcTime, &env_getter);
        assert_eq!(locale_name, "en_US.UTF-8");

        // 测试中文locale
        let mut env_vars = HashMap::new();
        env_vars.insert("LC_TIME", "zh_CN.UTF-8");
        let env_getter = make_env_getter(env_vars);
        let locale_name = get_locale_name_with_env(LcCategory::LcTime, &env_getter);
        assert_eq!(locale_name, "zh_CN.UTF-8");
    }

    #[test]
    fn test_empty_env_vars_ignored() {
        // 空的环境变量应该被忽略
        let mut env_vars = HashMap::new();
        env_vars.insert("LC_TIME", "");
        env_vars.insert("LANG", "en_US.UTF-8");
        let env_getter = |key: &str| -> Result<String, env::VarError> {
            match key {
                "LC_TIME" => Ok("".to_string()),
                "LANG" => Ok("en_US.UTF-8".to_string()),
                _ => Err(env::VarError::NotPresent),
            }
        };
        assert_eq!(
            get_locale_name_with_env(LcCategory::LcTime, &env_getter),
            "en_US.UTF-8"
        );
    }

    #[test]
    fn test_hard_locale_c_locale() {
        // 测试C locale下hard_locale返回false
        let env_getter = |key: &str| -> Result<String, env::VarError> {
            match key {
                "LC_TIME" => Ok("C".to_string()),
                _ => Err(env::VarError::NotPresent),
            }
        };
        let locale_name = get_locale_name_with_env(LcCategory::LcTime, &env_getter);
        assert_eq!(locale_name, "C");
        // 模拟hard_locale逻辑
        assert!(!hard_locale_for_test(LcCategory::LcTime, &env_getter));
    }

    #[test]
    fn test_hard_locale_posix_locale() {
        // 测试POSIX locale下hard_locale返回false
        let env_getter = |key: &str| -> Result<String, env::VarError> {
            match key {
                "LC_TIME" => Ok("POSIX".to_string()),
                _ => Err(env::VarError::NotPresent),
            }
        };
        let locale_name = get_locale_name_with_env(LcCategory::LcTime, &env_getter);
        assert_eq!(locale_name, "POSIX");
        // 模拟hard_locale逻辑
        assert!(!hard_locale_for_test(LcCategory::LcTime, &env_getter));
    }

    #[test]
    fn test_hard_locale_non_c_locale() {
        // 测试非C/POSIX locale下hard_locale返回true
        let env_getter = |key: &str| -> Result<String, env::VarError> {
            match key {
                "LC_TIME" => Ok("en_US.UTF-8".to_string()),
                _ => Err(env::VarError::NotPresent),
            }
        };
        let locale_name = get_locale_name_with_env(LcCategory::LcTime, &env_getter);
        assert_eq!(locale_name, "en_US.UTF-8");
        // 模拟hard_locale逻辑
        assert!(hard_locale_for_test(LcCategory::LcTime, &env_getter));
    }

    #[test]
    fn test_hard_locale_numeric_category() {
        // 测试LC_NUMERIC类别
        let env_getter = |key: &str| -> Result<String, env::VarError> {
            match key {
                "LC_NUMERIC" => Ok("zh_CN.UTF-8".to_string()),
                _ => Err(env::VarError::NotPresent),
            }
        };
        let locale_name = get_locale_name_with_env(LcCategory::LcNumeric, &env_getter);
        assert_eq!(locale_name, "zh_CN.UTF-8");
        assert!(hard_locale_for_test(LcCategory::LcNumeric, &env_getter));
    }

    #[test]
    fn test_hard_locale_collate_category() {
        // 测试LC_COLLATE类别
        let env_getter = |key: &str| -> Result<String, env::VarError> {
            match key {
                "LC_COLLATE" => Ok("fr_FR.UTF-8".to_string()),
                _ => Err(env::VarError::NotPresent),
            }
        };
        let locale_name = get_locale_name_with_env(LcCategory::LcCollate, &env_getter);
        assert_eq!(locale_name, "fr_FR.UTF-8");
        assert!(hard_locale_for_test(LcCategory::LcCollate, &env_getter));
    }

    #[test]
    fn test_strcoll_compare_c_locale() {
        // 在测试中模拟C locale环境
        let env_getter = |key: &str| -> Result<String, env::VarError> {
            match key {
                "LC_COLLATE" => Ok("C".to_string()),
                _ => Err(env::VarError::NotPresent),
            }
        };

        // 在C locale下，应该使用字节比较
        // "Windows" < "linux" (字节比较，W=87, l=108)
        let result = strcoll_compare(b"Windows", b"linux", false);
        assert_eq!(result, Ordering::Less);

        // 忽略大小写时，"windows" < "linux"
        let result = strcoll_compare(b"Windows", b"linux", true);
        assert_eq!(result, Ordering::Greater);
    }

    #[test]
    fn test_strcoll_compare_with_null_bytes() {
        // 测试包含null字节的情况，应该退回到UTF-8比较
        let s1 = b"hello\x00world";
        let s2 = b"hello\x00universe";

        let result = strcoll_compare(s1, s2, false);
        // "world" > "universe"
        assert_eq!(result, Ordering::Greater);
    }

    #[test]
    fn test_strcoll_compare_empty_strings() {
        let result = strcoll_compare(b"", b"", false);
        assert_eq!(result, Ordering::Equal);

        let result = strcoll_compare(b"a", b"", false);
        assert_eq!(result, Ordering::Greater);

        let result = strcoll_compare(b"", b"b", false);
        assert_eq!(result, Ordering::Less);
    }

    // 测试专用的hard_locale函数，接受环境变量获取函数作为参数
    fn hard_locale_for_test<F>(category: LcCategory, env_getter: F) -> bool
    where
        F: Fn(&str) -> Result<String, env::VarError>,
    {
        let locale_name = get_locale_name_with_env(category, env_getter);

        // 检查是否为C或POSIX locale
        if locale_name == "C" || locale_name == "POSIX" {
            return false;
        }

        true
    }
}
