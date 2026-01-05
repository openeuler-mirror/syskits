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

// TODO fix broken links
#![allow(rustdoc::broken_intra_doc_links)]
//! ctcore实用工具的宏。
//!
//! 本模块捆绑了ctcore实用工具中使用的所有宏。这些宏包括用于以各种格式报告错误、终止程序执行等的宏。
//!
//! 要使用本模块中的所有宏，必须像下面这样导入它们：
//!
//! ```ignore
//! #[macro_use]
//! extern crate ctcore;
//! ```
//!
//! 或者，你可以通过它们的全限定名单独导入宏，如下所示：
//!
//! ```no_run
//! use ctcore::{ct_show, ct_crash};
//! ```
//!
//! 以下是按用途排序的宏概述
//!
//! - 打印错误
//! - 来自实现crate::ct_error::UError的类型：[show!],[show_if_err!]
//! - 来自自定义消息：[show_error!]
//! - 打印警告：[show_warning!]
//! - 终止util执行
//! - 异常终止程序：[crash!],[crash_if_err!]`

// 忽略拼写检查器对以下单词的检查：sourcepath、targetpath、rustdoc

use std::sync::atomic::AtomicBool;

/// 是否以多调用二进制形式被调用(coreutils <utility>)
pub static UTILITY_IS_SECOND_ARG: AtomicBool = AtomicBool::new(false);

///
/// 显示一个 crate::ct_error::UError 并设置全局退出码。
///
/// 将 crate::ct_error::UError 中包含的错误消息打印到 stderr，并通过 crate::ct_error::set_exit_code 设置退出码。打印的错误消息前会附加调用工具的名称。调用此宏不会完成程序执行。
///
/// # 示例
///
/// 下面示例会打印一条消息 "Some error occurred" 并将工具的退出码设置为 2。
///
/// ```
/// # #[macro_use]
/// # extern crate ctcore;
///
/// use ctcore::ct_error::{self, CtSimpleError};
///
/// fn main() {
///     let err = CtSimpleError::new(2, "Some error occurred.");
///     ct_show!(err);
///     assert_eq!(ct_error::get_ct_exit_code(), 2);
/// }
/// ```
///
/// 如果不使用crate::ct_error::UError，可以通过以下方式实现相同的行为：
///
/// ```
/// # #[macro_use]
/// # extern crate ctcore;
///
/// use ctcore::ct_error::set_ct_exit_code;
///
/// fn main() {
///     set_ct_exit_code(2);
///     ct_show_error!("Some error occurred.");
/// }
/// ```
#[macro_export]
macro_rules! ct_show (
    ($err:expr) => ({
        use $crate::ct_error::CTError;
        let e = $err;
        $crate::ct_error::set_ct_exit_code(e.code());
        eprintln!("{}: {}", $crate::ct_util_name(), e);
    })
);

/// 在错误情况下显示错误并设置全局退出码。
///
/// 该宏围绕着 show! 宏，并接受一个 crate::ct_error::UResult 类型而非 crate::ct_error::UError 类型。
/// 如果 crate::ct_error::UResult 是 Err 变体，
/// 该宏将调用 show!。可以直接在函数调用的结果上使用它，就像在 install 工具中那样：
///
/// ```ignore
/// show_if_err!(copy(sourcepath, &targetpath, b));
/// ```
///
/// # 示例
///
/// ```ignore
/// # #[macro_use]
/// # extern crate ctcore;
/// # use ctcore::ct_error::{UError, UIoError, UResult, USimpleError};
///
/// # fn main() {
/// let is_ok = Ok(1);
/// // This does nothing at all
/// show_if_err!(is_ok);
///
/// let is_err = Err(USimpleError::new(1, "I'm an error").into());
/// // Calls `show!` on the contained USimpleError
/// show_if_err!(is_err);
/// # }
/// ```
///
///
#[macro_export]
macro_rules! ct_show_if_err(
    ($res:expr) => ({
        if let Err(e) = $res {
            $crate::ct_show!(e);
        }
    })
);

/// 以类似于GNU coreutils的方式向stderr显示错误。
///
/// 接受类似format!的输入并将其打印到stderr。输出前面带有当前实用程序的名称。
///
/// # 示例
///
/// ```
/// # #[macro_use]
/// # extern crate ctcore;
/// # fn main() {
/// ct_show_error!("Couldn't apply {} to {}", "foo", "bar");
/// # }
/// ```
#[macro_export]
macro_rules! ct_show_error(
    ($($args:tt)+) => ({
        eprint!("{}: ", $crate::ct_util_name());
        eprintln!($($args)+);
    })
);

/// 将警告消息打印到 stderr。
///
/// 接受与 format! 兼容的输入，在将其打印到 stderr 之前，会在消息前加上当前工具的名称和 "warning: "。
///
/// # 示例
///
/// ```
/// # #[macro_use]
/// # extern crate ctcore;
/// # fn main() {
/// // outputs <name>: warning: Couldn't apply foo to bar
/// ct_show_warning!("Couldn't apply {} to {}", "foo", "bar");
/// # }
/// ```
#[macro_export]
macro_rules! ct_show_warning(
    ($($args:tt)+) => ({
        eprint!("{}: warning: ", $crate::ct_util_name());
        eprintln!($($args)+);
    })
);

/// 显示错误并调用std::process::exit
///
/// 使用show_error!显示提供的错误消息，然后使用提供的退出代码调用std::process::exit。
///
/// # 示例
///
/// ```should_panic
/// # #[macro_use]
/// # extern crate ctcore;
/// # fn main() {
/// // outputs <name>: Couldn't apply foo to bar
/// // and terminates execution
/// ct_crash!(1, "Couldn't apply {} to {}", "foo", "bar");
/// # }
/// ```
#[macro_export]
macro_rules! ct_crash(
    ($exit_code:expr, $($args:tt)+) => ({
         $crate::ct_show_error!($($args)+);
        std::process::exit($exit_code);
    })
);

/// 解包一个 std::result::Result，遇到错误时崩溃而非引发恐慌。
///
/// 如果结果是 Ok 变体，则返回其中包含的值。如果是 Err 变体，则使用格式化的错误信息调用 crash!。
///
/// # 示例
///
/// ```should_panic
/// # #[macro_use]
/// # extern crate ctcore;
/// # fn main() {
/// let is_ok: Result<u32, &str> = Ok(1);
/// // Does nothing
/// ct_crash_if_err!(1, is_ok);
///
/// let is_err: Result<u32, &str> = Err("This didn't work...");
/// // Calls `crash!`
/// ct_crash_if_err!(1, is_err);
/// # }
/// ```
///
#[macro_export]
macro_rules! ct_crash_if_err {
    ($exit_code:expr, $exp:expr) => {
        match $exp {
            Ok(v) => v,
            Err(f) => $crate::ct_crash!($exit_code, "{}", f),
        }
    };
}
