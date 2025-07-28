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

// * 特性门控外部crate（重新作为公共内部模块共享）
#[cfg(feature = "libc")]
pub extern crate libc;
#[cfg(all(feature = "windows-sys", target_os = "windows"))]
pub extern crate windows_sys;

//## 内部模块
// 模块结构说明：
//
// ct_features: 特性门控代码模块。这部分代码依赖于特定的Rust语言特性，只有当项目启用相应特性时才会编译和使用。
//
// macros: 框架宏模块。包含使用macro_rules!宏定义的、供crate内部或外部使用者调用的宏，可通过crate::...路径进行访问。
//
// ct_mods: 核心跨平台模块。封装了适用于多种操作系统平台的通用功能，确保代码具有良好的平台兼容性。
//
// ct_parser: 字符串解析模块。包含用于解析和处理字符串数据的相关逻辑和算法，可能是专门针对某种特定格式或结构的字符串解析器。

mod ct_features;
mod ct_macros;
mod ct_mods;
mod ct_parser;
mod tool;

pub use tool::Tool;

// * cross-platform modules
pub use crate::ct_mods::ct_display;
pub use crate::ct_mods::ct_error;
pub use crate::ct_mods::ct_io;
pub use crate::ct_mods::ct_line_ending;
pub use crate::ct_mods::ct_locale;
pub use crate::ct_mods::ct_os;
pub use crate::ct_mods::ct_panic;
pub use crate::ct_mods::ct_posix;

// * string parsing modules
#[cfg(feature = "datetime")]
pub use crate::ct_parser::ct_parse_datetime;
pub use crate::ct_parser::ct_parse_glob;
pub use crate::ct_parser::ct_parse_size;
pub use crate::ct_parser::ct_parse_time;
pub use crate::ct_parser::ct_shortcut_value_parser;

// * feature-gated modules
#[cfg(feature = "backup-control")]
pub use crate::ct_features::ct_backup_control;
#[cfg(feature = "colors")]
pub use crate::ct_features::ct_colors;
#[cfg(feature = "encoding")]
pub use crate::ct_features::ct_encoding;
#[cfg(feature = "format")]
pub use crate::ct_features::ct_format;
#[cfg(feature = "fs")]
pub use crate::ct_features::ct_fs;
#[cfg(feature = "lines")]
pub use crate::ct_features::ct_lines;
#[cfg(feature = "quoting-style")]
pub use crate::ct_features::ct_quoting_style;
#[cfg(feature = "ranges")]
pub use crate::ct_features::ct_ranges;
#[cfg(feature = "ringbuffer")]
pub use crate::ct_features::ct_ringbuffer;
#[cfg(feature = "sum")]
pub use crate::ct_features::ct_sum;
#[cfg(feature = "update-control")]
pub use crate::ct_features::ct_update_control;
#[cfg(feature = "version-cmp")]
pub use crate::ct_features::ct_version_cmp;

// ** unix-only
#[cfg(all(unix, feature = "entries"))]
pub use crate::ct_features::ct_entries;
// * (platform-specific) feature-gated modules
// ** non-windows (i.e. Unix + Fuchsia)
#[cfg(all(not(windows), feature = "mode"))]
pub use crate::ct_features::ct_mode;
#[cfg(all(unix, feature = "perms"))]
pub use crate::ct_features::ct_perms;
#[cfg(all(unix, feature = "pipes"))]
pub use crate::ct_features::ct_pipes;
#[cfg(all(unix, feature = "process"))]
pub use crate::ct_features::ct_process;
#[cfg(all(target_os = "linux", feature = "signals"))]
pub use crate::ct_features::ct_signals;
#[cfg(all(target_os = "linux", feature = "utmpx"))]
pub use crate::ct_features::ct_utmpx;
// ** windows-only
#[cfg(all(windows, feature = "wide"))]
pub use crate::ct_features::ct_wide;

#[cfg(feature = "fsext")]
pub use crate::ct_features::ct_fsext;

#[cfg(all(target_os = "linux", feature = "fsxattr"))]
pub use crate::ct_features::ct_fsxattr;

//## core functions

use std::ffi::OsString;
use std::sync::atomic::Ordering;

use once_cell::sync::Lazy;

/// 执行util的实用程序代码。
///
/// 该宏扩展为一个主函数，调用util中的ctmain函数，以ctmain返回的代码退出。
#[macro_export]
macro_rules! ct_bin {
    ($crate_name:ident, $struct_name:ident) => {
        pub fn main() {
            use ctcore::Tool;
            use std::io::Write;
            // 对SIGPIPE失败/恐慌抑制冗余错误输出
            ctcore::ct_panic::ct_mute_set_panic_hook();

            // 执行实用工具代码
            let args = ctcore::ct_os_args().collect::<Vec<_>>();
            let tool = $crate_name::$struct_name::default();
            let result = tool.execute(&args);
            let code = match result {
                Ok(()) => ctcore::ct_error::get_ct_exit_code(),
                Err(err) => {
                    let s_err = format!("{}", err);
                    if !s_err.is_empty() {
                        ctcore::ct_show_error!("{}", s_err);
                    }
                    if err.usage() {
                        eprintln!(
                            "Try '{} --help' for more information.",
                            ctcore::ct_execute_phrase()
                        );
                    }
                    err.code()
                }
            };

            if let Err(e) = std::io::stdout().flush() {
                eprintln!("Error flushing stdout: {}", e);
            }

            std::process::exit(code);
        }
    };
}

/// 为 clap 生成使用说明字符串。
///
/// 本函数执行两件事。它缩进除首行之外的所有行以保持对齐，因为 clap 会在首行添加 "Usage: "
/// 。然后，它将所有出现的 {} 替换为执行短语，并返回生成的 String。
/// 它不支持更高级的格式化功能，如 {0}。
pub fn ct_format_usage(s: &str) -> String {
    s.lines() // 分割为行，以处理换行
        .enumerate() // 为每行添加索引
        .map(|(i, line)| {
            if i == 0 {
                // 第一行不添加空格
                line.to_string()
            } else {
                // 后续行添加7个空格的缩进
                format!("       {}", line)
            }
        })
        .collect::<Vec<_>>() // 将处理过的行收集回Vec
        .join("\n") // 重新连接为单个字符串
        .replace("{}", crate::ct_execute_phrase()) // 替换{}
}

pub fn ct_get_utility_is_second_arg() -> bool {
    crate::ct_macros::UTILITY_IS_SECOND_ARG.load(Ordering::SeqCst)
}

pub fn ct_set_utility_is_second_arg() {
    crate::ct_macros::UTILITY_IS_SECOND_ARG.store(true, Ordering::SeqCst);
}

// 调用args_os()可能代价较高，因为它会在迭代前复制整个argv。
// 因此，如果我们只需要第一个参数左右的信息，这样做就有些过分了。所以我们将其缓存起来。
static ARGV: Lazy<Vec<OsString>> = Lazy::new(|| wild::args_os().collect());

static UTIL_NAME: Lazy<String> = Lazy::new(|| {
    let base_index = if ct_get_utility_is_second_arg() { 1 } else { 0 };
    ARGV.get(base_index)
        .or_else(|| ARGV.get(base_index + 1))
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned())
});

/// Derive the utility name.
pub fn ct_util_name() -> &'static str {
    &UTIL_NAME
}

static EXECUTION_PHRASE: Lazy<String> = Lazy::new(|| {
    ARGV.get(..=usize::from(ct_get_utility_is_second_arg()))
        .unwrap_or_else(|| &ARGV[..1]) // 默认使用第一个元素
        .iter()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
});

/// 为"usage"派生完整的执行短语
pub fn ct_execute_phrase() -> &'static str {
    &EXECUTION_PHRASE
}

pub trait Args: Iterator<Item = OsString> + Sized {
    /// 将迭代器收集到一个Vec<String>中，将OsStrings有损地转换为Strings。
    fn collect_lossy(self) -> Vec<String> {
        self.map(|s| s.to_string_lossy().into_owned()).collect()
    }

    /// 将迭代器收集到一个Vec<String>中，同时移除其中包含无效编码的任何元素。
    fn collect_ignore(self) -> Vec<String> {
        self.filter_map(|s| s.into_string().ok()).collect()
    }
}

impl<T: Iterator<Item = OsString> + Sized> Args for T {}

pub fn ct_os_args() -> impl Iterator<Item = OsString> {
    ARGV.iter().cloned()
}

/// 从标准输入读取一行，并检查首字符是否为 'y' 或 'Y'
pub fn ct_read_true() -> bool {
    let mut s = String::new();

    match std::io::stdin().read_line(&mut s) {
        Ok(_) => s.trim().starts_with('y') || s.trim().starts_with('Y'),
        Err(_) => false,
    }
}

/// 使用格式化的字符串提示用户，并在他们回复'y'或'Y'时返回true。
///
/// 该宏函数接受与format!相同的语法。提示写入stderr。还会在末尾打印一个空格，以便在提示和用户输入之间有适当的间距。任何以'y'或'Y'开头的输入都被解释为'yes'。
///
/// # 示例
/// ```
/// use ctcore::ct_prompt_yes;
/// let file = "foo.rs";
/// ct_prompt_yes!("Do you want to delete '{}'?", file);
/// ```
/// 将如下内容打印到stderr（其中util_name将被实际的util名称替换），并等待用户输入。
///
/// ```txt
/// ```
#[macro_export]
macro_rules! ct_prompt_yes (
    ($($args:tt)+) => ({
        use std::io::Write;
        eprint!("{}: ", ctcore::ct_util_name());
        eprint!($($args)+);
        eprint!(" ");
        ctcore::ct_crash_if_err!(1, std::io::stderr().flush());
        ctcore::ct_read_true()
    })
);

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    fn make_os_vec(os_str: &OsStr) -> Vec<OsString> {
        vec![
            OsString::from("test"),
            OsString::from("สวัสดี"), // 关闭拼写检查：本行
            os_str.to_os_string(),
        ]
    }

    #[cfg(target_os = "linux")]
    fn test_invalid_utf8_args_lossy(os_str: &OsStr) {
        // 断言我们的字符串为无效UTF-8
        assert!(os_str.to_os_string().into_string().is_err());
        let test_vec = make_os_vec(os_str);
        let collected_to_str = test_vec.clone().into_iter().collect_lossy();

        // 长度不变 - 接受有损转换时不得丢弃任何参数
        assert_eq!(collected_to_str.len(), test_vec.len());
        // 首个索引相同
        for index in 0..2 {
            assert_eq!(collected_to_str[index], test_vec[index].to_str().unwrap());
        }
        // 对具有非法编码的字符串完成有损转换
        assert_eq!(
            *collected_to_str[2],
            os_str.to_os_string().to_string_lossy()
        );
    }

    #[cfg(target_os = "linux")]
    fn test_invalid_utf8_args_ignore(os_str: &OsStr) {
        // 断言我们的字符串为无效UTF-8
        assert!(os_str.to_os_string().into_string().is_err());
        let test_vec = make_os_vec(os_str);
        let collected_to_str = test_vec.clone().into_iter().collect_ignore();
        // 断言已过滤掉损坏的条目
        assert_eq!(collected_to_str.len(), test_vec.len() - 1);
        // 断言未损坏的索引按预期转换
        for index in 0..2 {
            assert_eq!(
                collected_to_str.get(index).unwrap(),
                test_vec.get(index).unwrap().to_str().unwrap()
            );
        }
    }

    #[test]
    fn valid_utf8_encoding_args() {
        // 创建仅包含正确编码的向量
        let test_vec = make_os_vec(&OsString::from("test2"));
        // 即使接受有损转换，也期望实现无损失的完全转换
        let _ = test_vec.into_iter().collect_lossy();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn invalid_utf8_args_unix() {
        use std::os::unix::ffi::OsStrExt;

        let source = [0x66, 0x6f, 0x80, 0x6f];
        let os_str = OsStr::from_bytes(&source[..]);
        test_invalid_utf8_args_lossy(os_str);
        test_invalid_utf8_args_ignore(os_str);
    }

    #[test]
    fn test_format_usage() {
        assert_eq!(ct_format_usage("expr EXPRESSION"), "expr EXPRESSION");
        assert_eq!(
            ct_format_usage("expr EXPRESSION\nexpr OPTION"),
            "expr EXPRESSION\n       expr OPTION"
        );
    }
}
