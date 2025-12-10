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

// TODO fix broken links
#![allow(rustdoc::broken_intra_doc_links)]
//! Macros for the ctcore utilities.
//!
//! This module bundles all macros used across the ctcore utilities. These
//! include macros for reporting errors in various formats, aborting program
//! execution and more.
//!
//! To make use of all macros in this module, they must be imported like so:
//!
//! ```ignore
//! #[macro_use]
//! extern crate ctcore;
//! ```
//!
//! Alternatively, you can import single macros by importing them through their
//! fully qualified name like this:
//!
//! ```no_run
//! use ctcore::{ct_show, crash};
//! ```
//!
//! Here's an overview of the macros sorted by purpose
//!
//! - Print errors
//!   - From types implementing [`crate::ct_error::CTError`]: [`ct_show!`],
//!     [`show_if_err!`]
//!   - From custom messages: [`ct_show_error!`]
//! - Print warnings: [`ct_show_warning!`]
//! - Terminate util execution
//!   - Crash program: [`crash!`], [`ct_crash_if_err!`]

// spell-checker:ignore sourcepath targetpath rustdoc

use std::sync::atomic::AtomicBool;

// This file is part of the cttils coreutils package.
//
// For the full copyright and license information, please view the LICENSE
// file that was distributed with this source code.

/// Whether we were called as a multicall binary (`coreutils <utility>`)
pub static UTILITY_IS_SECOND_ARG: AtomicBool = AtomicBool::new(false);

//====

/// Display a [`crate::ct_error::CTError`] and set global exit code.
///
/// Prints the error message contained in an [`crate::ct_error::CTError`] to stderr
/// and sets the exit code through [`crate::ct_error::set_ct_exit_code`]. The printed
/// error message is prepended with the calling utility's name. A call to this
/// macro will not finish program execution.
///
/// # Examples
///
/// The following example would print a message "Some error occurred" and set
/// the utility's exit code to 2.
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
///     assert_eq!(ct_error::get_exit_code(), 2);
/// }
/// ```
///
/// If not using [`crate::ct_error::CTError`], one may achieve the same behavior
/// like this:
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
macro_rules! ct_show(
    ($err:expr) => ({
        use $crate::ct_error::CTError;
        let e = $err;
        $crate::ct_error::set_ct_exit_code(e.code());
        eprintln!("{}: {}", $crate::ct_util_name(), e);
    })
);

/// Display an error and set global exit code in error case.
///
/// Wraps around [`ct_show!`] and takes a [`crate::ct_error::UResult`] instead of a
/// [`crate::ct_error::CTError`] type. This macro invokes [`ct_show!`] if the
/// [`crate::ct_error::UResult`] is an `Err`-variant. This can be invoked directly
/// on the result of a function call, like in the `install` utility:
///
/// ```ignore
/// show_if_err!(copy(sourcepath, &targetpath, b));
/// ```
///
/// # Examples
///
/// ```ignore
/// # #[macro_use]
/// # extern crate ctcore;
/// # use ctcore::ct_error::{CTError, UIoError, UResult, USimpleError};
///
/// # fn main() {
/// let is_ok = Ok(1);
/// // This does nothing at all
/// show_if_err!(is_ok);
///
/// let is_err = Err(USimpleError::new(1, "I'm an error").into());
/// // Calls `ct_show!` on the contained USimpleError
/// show_if_err!(is_err);
/// # }
/// ```
///
///
#[macro_export]
macro_rules! show_if_err(
    ($res:expr) => ({
        if let Err(e) = $res {
            $crate::ct_show!(e);
        }
    })
);

/// Show an error to stderr in a similar style to GNU coreutils.
///
/// Takes a [`format!`]-like input and prints it to stderr. The output is
/// prepended with the current utility's name.
///
/// # Examples
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

/// Print a warning message to stderr.
///
/// Takes [`format!`]-compatible input and prepends it with the current
/// utility's name and "warning: " before printing to stderr.
///
/// # Examples
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

/// Display an error and [`std::process::exit`]
///
/// Displays the provided error message using [`ct_show_error!`], then invokes
/// [`std::process::exit`] with the provided exit code.
///
/// # Examples
///
/// ```should_panic
/// # #[macro_use]
/// # extern crate ctcore;
/// # fn main() {
/// // outputs <name>: Couldn't apply foo to bar
/// // and terminates execution
/// crash!(1, "Couldn't apply {} to {}", "foo", "bar");
/// # }
/// ```
#[macro_export]
macro_rules! crash(
    ($exit_code:expr, $($args:tt)+) => ({
        $crate::ct_show_error!($($args)+);
        std::process::exit($exit_code);
    })
);

/// Unwrap a [`std::result::Result`], crashing instead of panicking.
///
/// If the result is an `Ok`-variant, returns the value contained inside. If it
/// is an `Err`-variant, invokes [`crash!`] with the formatted error instead.
///
/// # Examples
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
#[macro_export]
macro_rules! ct_crash_if_err {
    ($exit_code:expr, $exp:expr) => {
        if let Err(f) = $exp {
            $crate::crash!($exit_code, "{}", f);
        } else {
            $exp.expect("Expected Ok, found Err")
        }
    };
}
