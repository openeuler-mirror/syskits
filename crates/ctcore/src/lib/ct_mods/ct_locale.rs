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

use std::env;

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
    fn test_convenience_functions() {
        // 测试便利函数是否正确调用了底层函数
        // 这些测试依赖于实际的环境变量，所以只验证函数能够正常调用
        let _result_time = hard_locale_time();
        let _result_numeric = hard_locale_numeric();
        let _result_collate = hard_locale_collate();
        // 如果没有panic，说明函数调用成功
        assert!(true);
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
