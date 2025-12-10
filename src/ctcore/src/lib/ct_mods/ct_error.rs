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
#![allow(rustdoc::broken_intra_doc_links)]
//! All utils return exit with an exit code. Usually, the following scheme is used:
//! * `0`: succeeded
//! * `1`: minor problems
//! * `2`: major problems
//!
//! 本模块提供了与 Rust 错误处理习惯相统一的类型，以便处理这些退出码。相比手动使用 [std::process::exit]，这种方式有以下几个优势：
//! 1. 允许在 ctmain 中使用 ?、map_err、unwrap_or 等操作符。
//! 1. 鼓励在 utils 的函数中使用 [CTResult]/[Result] 类型。
//! 1. 使得 utils 之间的错误消息格式标准化。
//! 1. 可以从外部结果类型（如：[std::io::Result] 和 clap::ClapResult）创建标准化的错误消息。
//! 1. 使用 [set_ct_exit_code] 函数可以减轻非致命错误时手动跟踪退出码的负担。
//!
//! # Usage
//! The signature of a typical util should be:
//! ```ignore
//! fn ctmain(args: impl ctcore::Args) -> UResult<()> {
//!     ...
//! }
//! ```
//! [CTResult]是围绕[Result]的一个简单封装，带有一个自定义错误特征：[CTError]。与实现了[std::error::Error]的类型相比，最重要的区别在于当从ctmain返回时，[CTError]可以指定程序的退出码：
//! * 当返回Ok时，使用通过[set_ct_exit_code]设置的代码作为退出码。如果未使用[set_ct_exit_code]，则使用0。
//! * 当返回Err时，使用与错误对应的代码作为退出码，并显示错误消息。
//!
//! Additionally, the errors can be displayed manually with the [`show`] and [`show_if_err`] macros:
//! ```ignore
//! let res = Err(USimpleError::new(1, "Error!!"));
//! show_if_err!(res);
//! // or
//! if let Err(e) = res {
//!    ct_show!(e);
//! }
//! ```
//!
//! **Note**: The [`show`] and [`show_if_err`] macros set the exit code of the program using
//! [`set_ct_exit_code`]. See the documentation on that function for more information.
//!
//! # Guidelines
//! * Use error types from `ctcore` where possible.
//! * Add error types to `ctcore` if an error appears in multiple utils.
//! * Prefer proper custom error types over [`ExitCode`] and [`USimpleError`].
//! * [`USimpleError`] may be used in small utils with simple error handling.
//! * Using [`ExitCode`] is not recommended but can be useful for converting utils to use
//!   [`UResult`].

// spell-checker:ignore uioerror rustdoc

use std::{
    error::Error,
    fmt::{Display, Formatter},
    sync::atomic::{AtomicI32, Ordering},
};

static EXIT_CODE: AtomicI32 = AtomicI32::new(0);

/// Get the last exit code set with [`set_ct_exit_code`].
/// The default value is `0`.
pub fn get_exit_code() -> i32 {
    EXIT_CODE.load(Ordering::SeqCst)
}

/// Set the exit code for the program if `ctmain` returns `Ok(())`.
///
/// This function is most useful for non-fatal errors, for example when applying an operation to
/// multiple files:
/// ```ignore
/// use ctcore::ct_error::{UResult, set_ct_exit_code};
///
/// fn ctmain(args: impl ctcore::Args) -> UResult<()> {
///     ...
///     for file in files {
///         let res = some_operation_that_might_fail(file);
///         match res {
///             Ok() => {},
///             Err(_) => set_ct_exit_code(1),
///         }
///     }
///     Ok(()) // If any of the operations failed, 1 is returned.
/// }
/// ```
pub fn set_ct_exit_code(code: i32) {
    EXIT_CODE.store(code, Ordering::SeqCst);
}

/// Result type that should be returned by all utils.
pub type CTResult<T> = Result<T, Box<dyn CTError>>;

/// Custom errors defined by the utils and `ctcore`.
///
/// All errors should implement [`std::error::Error`], [`std::fmt::Display`] and
/// [`std::fmt::Debug`] and have an additional `code` method that specifies the
/// exit code of the program if the error is returned from `ctmain`.
///
/// An example of a custom error from `ls`:
///
/// ```
/// use ctcore::{
///     ct_display::Quotable,
///     ct_error::{CTError, CTResult}
/// };
/// use std::{
///     error::Error,
///     fmt::{Display, Debug},
///     path::PathBuf
/// };
///
/// #[derive(Debug)]
/// enum LsError {
///     InvalidLineWidth(String),
///     NoMetadata(PathBuf),
/// }
///
/// impl CTError for LsError {
///     fn code(&self) -> i32 {
///         match self {
///             LsError::InvalidLineWidth(_) => 2,
///             LsError::NoMetadata(_) => 1,
///         }
///     }
/// }
///
/// impl Error for LsError {}
///
/// impl Display for LsError {
///     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
///         match self {
///             LsError::InvalidLineWidth(s) => write!(f, "invalid line width: {}", s.quote()),
///             LsError::NoMetadata(p) => write!(f, "could not open file: {}", p.quote()),
///         }
///     }
/// }
/// ```
///
/// The main routine would look like this:
///
/// ```ignore
/// #[ctcore::main]
/// pub fn ctmain(args: impl ctcore::Args) -> UResult<()> {
///     // Perform computations here ...
///     return Err(LsError::InvalidLineWidth(String::from("test")).into())
/// }
/// ```
///
/// /// 调用into()是为了将LsError转换为
/// [`Box<dyn CTError>`]. From 的实现会自动提供。
///
/// 类似于 quick_error 这样的 crate 也可使用，
/// 但仍然需要为 code 方法提供 impl 实现。
pub trait CTError: Error + Send {
    /// 自定义错误的错误码。
    ///
    /// Set a return value for each variant of an enum-type to associate an
    /// error code (which is returned to the system shell) with an error
    /// variant.
    ///
    /// # Example
    ///
    /// ```
    /// use ctcore::{
    ///     ct_display::Quotable,
    ///     ct_error::CTError
    /// };
    /// use std::{
    ///     error::Error,
    ///     fmt::{Display, Debug},
    ///     path::PathBuf
    /// };
    ///
    /// #[derive(Debug)]
    /// enum MyError {
    ///     Foo(String),
    ///     Bar(PathBuf),
    ///     Bing(),
    /// }
    ///
    /// impl CTError for MyError {
    ///     fn code(&self) -> i32 {
    ///         match self {
    ///             MyError::Foo(_) => 2,
    ///             // All other errors yield the same error code, there's no
    ///             // need to list them explicitly.
    ///             _ => 1,
    ///         }
    ///     }
    /// }
    ///
    /// impl Error for MyError {}
    ///
    /// impl Display for MyError {
    ///     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    ///         use MyError as ME;
    ///         match self {
    ///             ME::Foo(s) => write!(f, "Unknown Foo: {}", s.quote()),
    ///             ME::Bar(p) => write!(f, "Couldn't find Bar: {}", p.quote()),
    ///             ME::Bing() => write!(f, "Exterminate!"),
    ///         }
    ///     }
    /// }
    /// ```
    fn code(&self) -> i32 {
        1
    }

    /// Print usage help to a custom error.
    ///
    /// Return true or false to control whether a short usage help is printed
    /// below the error message. The usage help is in the format: "Try `{name}
    /// --help` for more information." and printed only if `true` is returned.
    ///
    /// # Example
    ///
    /// ```
    /// use ctcore::{
    ///     ct_display::Quotable,
    ///     ct_error::CTError
    /// };
    /// use std::{
    ///     error::Error,
    ///     fmt::{Display, Debug},
    ///     path::PathBuf
    /// };
    ///
    /// #[derive(Debug)]
    /// enum MyError {
    ///     Foo(String),
    ///     Bar(PathBuf),
    ///     Bing(),
    /// }
    ///
    /// impl CTError for MyError {
    ///     fn usage(&self) -> bool {
    ///         match self {
    ///             // This will have a short usage help appended
    ///             MyError::Bar(_) => true,
    ///             // These matches won't have a short usage help appended
    ///             _ => false,
    ///         }
    ///     }
    /// }
    ///
    /// impl Error for MyError {}
    ///
    /// impl Display for MyError {
    ///     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    ///         use MyError as ME;
    ///         match self {
    ///             ME::Foo(s) => write!(f, "Unknown Foo: {}", s.quote()),
    ///             ME::Bar(p) => write!(f, "Couldn't find Bar: {}", p.quote()),
    ///             ME::Bing() => write!(f, "Exterminate!"),
    ///         }
    ///     }
    /// }
    /// ```
    fn usage(&self) -> bool {
        false
    }
}

impl<T> From<T> for Box<dyn CTError>
where
    T: CTError + 'static,
{
    fn from(t: T) -> Self {
        Box::new(t)
    }
}

/// 一个包含退出码和消息的简单错误类型，实现了 [CTError] 特征。
///
/// ```
/// use ctcore::ct_error::{CTResult, CtSimpleError};
/// let err = CtSimpleError { code: 1, message: "error!".into()};
/// let res: CTResult<()> = Err(err.into());
/// // or using the `new` method:
/// let res: CTResult<()> = Err(CtSimpleError::new(1, "error!"));
/// ```
#[derive(Debug)]
pub struct CtSimpleError {
    pub code: i32,
    pub message: String,
}

impl CtSimpleError {
    #[allow(clippy::new_ret_no_self)]
    pub fn new<S: Into<String>>(code: i32, message: S) -> Box<dyn CTError> {
        Box::new(Self {
            code,
            message: message.into(),
        })
    }
}

impl Error for CtSimpleError {}

impl Display for CtSimpleError {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        self.message.fmt(f)
    }
}

impl CTError for CtSimpleError {
    fn code(&self) -> i32 {
        self.code
    }
}

#[derive(Debug)]
pub struct CTsageError {
    pub code: i32,
    pub message: String,
}

impl CTsageError {
    #[allow(clippy::new_ret_no_self)]
    pub fn new<S: Into<String>>(code: i32, message: S) -> Box<dyn CTError> {
        Box::new(Self {
            code,
            message: message.into(),
        })
    }
}

impl Error for CTsageError {}

impl Display for CTsageError {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        self.message.fmt(f)
    }
}

impl CTError for CTsageError {
    fn code(&self) -> i32 {
        self.code
    }

    fn usage(&self) -> bool {
        true
    }
}

/// Wrapper type around [`std::io::Error`].
///
/// 显示由[CTIoError]的错误消息应与GNU coreutils显示的错误消息相匹配。
///
/// 有两种构造此类型的方法：使用[CTIoError::new]或在[std::io::Result]或[std::io::Error]上调用[FromIo::map_err_context]方法。
/// ```
/// use ctcore::{
///     ct_display::Quotable,
///     ct_error::{FromIo, CTResult, CTIoError, CTError}
/// };
/// use std::fs::File;
/// use std::path::Path;
/// let path = Path::new("test.txt");
///
/// // Manual construction
/// let e: Box<dyn CTError> = CTIoError::new(
///     std::io::ErrorKind::NotFound,
///     format!("cannot access {}", path.quote())
/// );
/// let res: CTResult<()> = Err(e.into());
///
/// // Converting from an `std::io::Error`.
/// let res: CTResult<File> = File::open(path).map_err_context(|| format!("cannot access {}", path.quote()));
/// ```
#[derive(Debug)]
pub struct CTIoError {
    context: Option<String>,
    inner: std::io::Error,
}

impl CTIoError {
    #[allow(clippy::new_ret_no_self)]
    pub fn new<S: Into<String>>(kind: std::io::ErrorKind, context: S) -> Box<dyn CTError> {
        Box::new(Self {
            context: Some(context.into()),
            inner: kind.into(),
        })
    }
}

impl CTError for CTIoError {}

impl Error for CTIoError {}

impl Display for CTIoError {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        use std::io::ErrorKind::*;

        let message;
        // let message = if self.inner.raw_os_error().is_some() {
        //     // These are errors that come directly from the OS.
        //     // We want to normalize their messages across systems,
        //     // and we want to strip the "(os error X)" suffix.
        //     match self.inner.kind() {
        //         NotFound => "No such file or directory",
        //         PermissionDenied => "Permission denied",
        //         ConnectionRefused => "Connection refused",
        //         ConnectionReset => "Connection reset",
        //         ConnectionAborted => "Connection aborted",
        //         NotConnected => "Not connected",
        //         AddrInUse => "Address in use",
        //         AddrNotAvailable => "Address not available",
        //         BrokenPipe => "Broken pipe",
        //         AlreadyExists => "Already exists",
        //         WouldBlock => "Would block",
        //         InvalidInput => "Invalid input",
        //         InvalidData => "Invalid data",
        //         TimedOut => "Timed out",
        //         WriteZero => "Write zero",
        //         Interrupted => "Interrupted",
        //         UnexpectedEof => "Unexpected end of file",
        //         _ => {
        //             // TODO: When the new error variants
        //             // (https://github.com/rust-lang/rust/issues/86442)
        //             // are stabilized, we should add them to the match statement.
        //             message = strip_errno(&self.inner);
        //             &message
        //         }
        //     }
        // } else {
        //     // These messages don't need as much normalization, and the above
        //     // messages wouldn't always be a good substitute.
        //     // For example, ErrorKind::NotFound doesn't necessarily mean it was
        //     // a file that was not found.
        //     // There are also errors with entirely custom messages.
        //     message = self.inner.to_string();
        //     &message
        // };
        // if let Some(ctx) = &self.context {
        //     write!(f, "{ctx}: {message}")
        // } else {
        //     write!(f, "{message}")
        // }

        let message = match self.inner.raw_os_error() {
            Some(_) => {
                if self.inner.kind() == NotFound {
                    "No such file or directory"
                } else if self.inner.kind() == PermissionDenied {
                    "Permission denied"
                } else if self.inner.kind() == ConnectionRefused {
                    "Connection refused"
                } else if self.inner.kind() == ConnectionReset {
                    "Connection reset"
                } else if self.inner.kind() == ConnectionAborted {
                    "Connection aborted"
                } else if self.inner.kind() == NotConnected {
                    "Not connected"
                } else if self.inner.kind() == AddrInUse {
                    "Address in use"
                } else if self.inner.kind() == AddrNotAvailable {
                    "Address not available"
                } else if self.inner.kind() == BrokenPipe {
                    "Broken pipe"
                } else if self.inner.kind() == AlreadyExists {
                    "Already exists"
                } else if self.inner.kind() == WouldBlock {
                    "Would block"
                } else if self.inner.kind() == InvalidInput {
                    "Invalid input"
                } else if self.inner.kind() == InvalidData {
                    "Invalid data"
                } else if self.inner.kind() == TimedOut {
                    "Timed out"
                } else if self.inner.kind() == WriteZero {
                    "Write zero"
                } else if self.inner.kind() == Interrupted {
                    "Interrupted"
                } else if self.inner.kind() == UnexpectedEof {
                    "Unexpected end of file"
                } else {
                    // TODO: When the new error variants
                    // (https://github.com/rust-lang/rust/issues/86442)
                    // are stabilized, we should add them to the if-else chain.
                    message = strip_errno(&self.inner);
                    &message
                }
            }
            None => {
                // These messages don't need as much normalization, and the above
                // messages wouldn't always be a good substitute.
                // For example, ErrorKind::NotFound doesn't necessarily mean it was
                // a file that was not found.
                // There are also errors with entirely custom messages.
                message = self.inner.to_string();
                &message
            }
        };

        match &self.context {
            Some(ctx) => write!(f, "{}: {}", ctx, message),
            None => write!(f, "{}", message),
        }
    }
}

/// Strip the trailing " (os error XX)" from io error strings.
pub fn strip_errno(err: &std::io::Error) -> String {
    // let mut msg = err.to_string();
    // if let Some(pos) = msg.find(" (os error ") {
    //     msg.truncate(pos);
    // }
    // msg

    let mut msg = err.to_string();
    match msg.find(" (os error ") {
        Some(pos) => {
            msg.truncate(pos);
            msg
        }
        None => msg,
    }
}

/// 启用从[std::io::Error]到[CTError]的转换，以及从[std::io::Result]到[CTResult]的转换。
pub trait FromIo<T> {
    fn map_err_context(self, context: impl FnOnce() -> String) -> T;
}

impl FromIo<Box<CTIoError>> for std::io::Error {
    fn map_err_context(self, context: impl FnOnce() -> String) -> Box<CTIoError> {
        Box::new(CTIoError {
            context: Some((context)()),
            inner: self,
        })
    }
}

impl<T> FromIo<CTResult<T>> for std::io::Result<T> {
    fn map_err_context(self, context: impl FnOnce() -> String) -> CTResult<T> {
        self.map_err(|e| e.map_err_context(context) as Box<dyn CTError>)
    }
}

impl FromIo<Box<CTIoError>> for std::io::ErrorKind {
    fn map_err_context(self, context: impl FnOnce() -> String) -> Box<CTIoError> {
        Box::new(CTIoError {
            context: Some((context)()),
            inner: std::io::Error::new(self, ""),
        })
    }
}

impl From<std::io::Error> for CTIoError {
    fn from(f: std::io::Error) -> Self {
        Self {
            context: None,
            inner: f,
        }
    }
}

impl From<std::io::Error> for Box<dyn CTError> {
    fn from(f: std::io::Error) -> Self {
        let u_error: CTIoError = f.into();
        Box::new(u_error) as Self
    }
}

/// Enables the conversion from [`Result<T, nix::Error>`] to [`UResult<T>`].
///
/// # Examples
///
/// ```
/// use ctcore::ct_error::FromIo;
/// use nix::errno::Errno;
///
/// let nix_err = Err::<(), nix::Error>(Errno::EACCES);
/// let uio_result = nix_err.map_err_context(|| String::from("fix me please!"));
///
/// // prints "fix me please!: Permission denied"
/// println!("{}", uio_result.unwrap_err());
/// ```
#[cfg(unix)]
impl<T> FromIo<CTResult<T>> for Result<T, nix::Error> {
    fn map_err_context(self, context: impl FnOnce() -> String) -> CTResult<T> {
        self.map_err(|e| {
            Box::new(CTIoError {
                context: Some((context)()),
                inner: std::io::Error::from_raw_os_error(e as i32),
            }) as Box<dyn CTError>
        })
    }
}

#[cfg(unix)]
impl<T> FromIo<CTResult<T>> for nix::Error {
    fn map_err_context(self, context: impl FnOnce() -> String) -> CTResult<T> {
        Err(Box::new(CTIoError {
            context: Some((context)()),
            inner: std::io::Error::from_raw_os_error(self as i32),
        }) as Box<dyn CTError>)
    }
}

#[cfg(unix)]
impl From<nix::Error> for CTIoError {
    fn from(f: nix::Error) -> Self {
        Self {
            context: None,
            inner: std::io::Error::from_raw_os_error(f as i32),
        }
    }
}

#[cfg(unix)]
impl From<nix::Error> for Box<dyn CTError> {
    fn from(f: nix::Error) -> Self {
        let u_error: CTIoError = f.into();
        Box::new(u_error) as Self
    }
}

/// Shorthand to construct [`UIoError`]-instances.
///
/// This macro serves as a convenience call to quickly construct instances of
/// [`UIoError`]. It takes:
///
/// - An instance of [`std::io::Error`]
/// - A `format!`-compatible string and
/// - An arbitrary number of arguments to the format string
///
/// In exactly this order. It is equivalent to the more verbose code seen in the
/// example.
///
/// # Examples
///
/// ```
/// use ctcore::ct_error::CTIoError;
/// use ctcore::uio_error;
///
/// let io_err = std::io::Error::new(
///     std::io::ErrorKind::PermissionDenied, "fix me please!"
/// );
///
/// let uio_err = CTIoError::new(
///     io_err.kind(),
///     format!("Error code: {}", 2)
/// );
///
/// let other_uio_err = uio_error!(io_err, "Error code: {}", 2);
///
/// // prints "fix me please!: Permission denied"
/// println!("{}", uio_err);
/// // prints "Error code: 2: Permission denied"
/// println!("{}", other_uio_err);
/// ```
///
/// [CTIoError]的std::fmt::Display实现将确保与std::io::Error实际错误类型相关的适当错误消息被追加到任何附加定义的错误消息（作为第二个参数）之后。
///
/// If you want to show only the error message for the [`std::io::ErrorKind`]
/// that's contained in [`UIoError`], pass the second argument as empty string:
///
/// ```
/// use ctcore::ct_error::CTIoError;
/// use ctcore::uio_error;
///
/// let io_err = std::io::Error::new(
///     std::io::ErrorKind::PermissionDenied, "fix me please!"
/// );
///
/// let other_uio_err = uio_error!(io_err, "");
///
/// // prints: ": Permission denied"
/// println!("{}", other_uio_err);
/// ```
//#[macro_use]
#[macro_export]
macro_rules! uio_error(
    ($err:expr, $($args:tt)+) => ({
        CTIoError::new(
            $err.kind(),
            format!($($args)+)
        )
    })
);

/// 一种特殊错误类型，当从 ctmain 返回时不会打印任何消息。对于将实用程序移植为使用 [CTResult] 特别有用。
///
/// 可以通过以下两种方式构造 ExitCode：
///
/// There are two ways to construct an [`ExitCode`]:
/// ```
/// use ctcore::ct_error::{ExitCode, CTResult};
/// // Explicit
/// let res: CTResult<()> = Err(ExitCode(1).into());
///
/// // Using into on `i32`:
/// let res: CTResult<()> = Err(1.into());
/// ```
/// This type is especially useful for a trivial conversion from utils returning [`i32`] to
/// returning [`UResult`].
#[derive(Debug)]
pub struct ExitCode(pub i32);

impl ExitCode {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(code: i32) -> Box<dyn CTError> {
        Box::new(Self(code))
    }
}

impl Error for ExitCode {}

impl Display for ExitCode {
    fn fmt(&self, _: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        Ok(())
    }
}

impl CTError for ExitCode {
    fn code(&self) -> i32 {
        self.0
    }
}

impl From<i32> for Box<dyn CTError> {
    fn from(i: i32) -> Self {
        ExitCode::new(i)
    }
}

/// clap::Error的封装器，实现了[CTError] trait
///
/// Contains a custom error code. When `Display::fmt` is called on this struct
/// the [`clap::Error`] will be printed _directly to `stdout` or `stderr`_.
/// This is because `clap` only supports colored output when it prints directly.
///
/// [`ClapErrorWrapper`] is generally created by calling the
/// [`UClapError::with_exit_code`] method on [`clap::Error`] or using the [`From`]
/// implementation from [`clap::Error`] to `Box<dyn CTError>`, which constructs
/// a [`ClapErrorWrapper`] with an exit code of `1`.
///
/// ```rust
/// use ctcore::ct_error::{ClapErrorWrapper, CTError, UClapError};
/// let command = clap::Command::new("test");
/// let result: Result<_, ClapErrorWrapper> = command.try_get_matches().with_exit_code(125);
///
/// let command = clap::Command::new("test");
/// let result: Result<_, Box<dyn CTError>> = command.try_get_matches().map_err(Into::into);
/// ```
#[derive(Debug)]
pub struct ClapErrorWrapper {
    code: i32,
    error: clap::Error,
}

/// Extension trait for `clap::Error` to adjust the exit code.
pub trait UClapError<T> {
    fn with_exit_code(self, code: i32) -> T;
}

impl From<clap::Error> for Box<dyn CTError> {
    fn from(e: clap::Error) -> Self {
        Box::new(ClapErrorWrapper { code: 1, error: e })
    }
}

impl UClapError<ClapErrorWrapper> for clap::Error {
    fn with_exit_code(self, code: i32) -> ClapErrorWrapper {
        ClapErrorWrapper { code, error: self }
    }
}

impl UClapError<Result<clap::ArgMatches, ClapErrorWrapper>>
    for Result<clap::ArgMatches, clap::Error>
{
    fn with_exit_code(self, code: i32) -> Result<clap::ArgMatches, ClapErrorWrapper> {
        self.map_err(|e| e.with_exit_code(code))
    }
}

impl CTError for ClapErrorWrapper {
    fn code(&self) -> i32 {
        // If the error is a DisplayHelp or DisplayVersion variant,
        // we don't want to apply the custom error code, but leave
        // it 0.
        // if let clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion =
        //     self.error.kind()
        // {
        //     0
        // } else {
        //     self.code
        // }
        match self.error.kind() {
            clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => 0,
            _ => self.code,
        }
    }
}

impl Error for ClapErrorWrapper {}

// This is abuse of the Display trait
impl Display for ClapErrorWrapper {
    fn fmt(&self, _f: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        // self.error.print().unwrap();
        // Ok(())

        match self.error.print() {
            Ok(_) => Ok(()),
            Err(e) => panic!("{}", e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

   #[test]
    fn test_get_exit_code() {
        // 测试默认退出码是否为0
        set_ct_exit_code(0);
        assert_eq!(get_exit_code(), 0);
    }

    #[test]
    fn test_set_exit_code() {
        // 测试设置退出码是否成功
        set_ct_exit_code(1);
        assert_eq!(get_exit_code(), 1);
    }

    #[test]
    fn test_ui_error_code() {
        // 测试自定义错误类型的错误码
        let error = CtSimpleError {
            code: 42,
            message: String::from("Error message"),
        };
        assert_eq!(error.code(), 42);
    }

    #[test]
    fn test_ui_error_display() {
        // 测试自定义错误类型的显示
        let error = CtSimpleError {
            code: 42,
            message: String::from("Error message"),
        };
        assert_eq!(error.to_string(), "Error message");
    }

    #[test]
    fn test_ui_io_error_display() {
        // 测试IO错误类型的显示
        let error = CTIoError {
            context: None,
            inner: std::io::Error::from(std::io::ErrorKind::NotFound),
        };
        assert_eq!(error.to_string(), "entity not found");
    }

    #[test]
    fn test_usimple_error() {
        let err = CtSimpleError::new(2, "Test error");
        assert_eq!(err.code(), 2);
        assert_eq!(format!("{}", err), "Test error");
    }

    #[test]
    fn test_uusage_error_usage_flag() {
        let err = CTsageError::new(1, "Usage needed");
        assert!(err.usage());
        assert_eq!(format!("{}", err), "Usage needed");
    }

    #[test]
    fn test_uio_error_message() {
        let err = CTIoError::new(std::io::ErrorKind::NotFound, "File not found");
        assert_eq!(format!("{}", err), "File not found: entity not found");
    }

    #[test]
    fn test_exit_code() {
        let err = ExitCode::new(3);
        assert_eq!(err.code(), 3);
        assert_eq!(format!("{}", err), "");
    }

    #[test]
    fn test_set_get_exit_code() {
        set_ct_exit_code(5);
        assert_eq!(get_exit_code(), 5);
    }

    #[test]
    fn test_from_io_error_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "Access denied");
        let custom_err: Box<dyn CTError> = Box::new(CTIoError::from(io_err));
        assert_eq!(format!("{}", custom_err), "Access denied");
    }

    #[test]
    #[cfg(unix)]
    fn test_base_nix_error_conversion() {
        use super::{CTIoError, FromIo};
        use nix::errno::Errno;
        use std::io::ErrorKind;

        for (nix_error, expected_error_kind) in [
            (Errno::EACCES, ErrorKind::PermissionDenied),
            (Errno::ENOENT, ErrorKind::NotFound),
            (Errno::EEXIST, ErrorKind::AlreadyExists),
        ] {
            let error = CTIoError::from(nix_error);
            assert_eq!(expected_error_kind, error.inner.kind());
        }
        assert_eq!(
            "test: Permission denied",
            Err::<(), nix::Error>(Errno::EACCES)
                .map_err_context(|| String::from("test"))
                .unwrap_err()
                .to_string()
        );
    }

}