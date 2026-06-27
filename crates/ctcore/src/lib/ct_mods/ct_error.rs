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
#![allow(rustdoc::broken_intra_doc_links)]
//! 所有 utils 在退出时都会返回一个退出码。通常，会遵循以下编码规则：
//! * 0: 成功
//! * 1: 小问题
//! * 2: 大问题
//!
//! 本模块提供了与 Rust 错误处理习惯相统一的类型，以便处理这些退出码。相比手动使用 [std::process::exit]，这种方式有以下几个优势：
//! 1. 允许在 main 中使用 ?、map_err、unwrap_or 等操作符。
//! 1. 鼓励在 utils 的函数中使用 [CTResult]/[Result] 类型。
//! 1. 使得 utils 之间的错误消息格式标准化。
//! 1. 可以从外部结果类型（如：[std::io::Result] 和 clap::ClapResult）创建标准化的错误消息。
//! 1. 使用 [set_ct_exit_code] 函数可以减轻非致命错误时手动跟踪退出码的负担。
//!
//! # 使用方式

//! 一个典型的 util 签名应该是：
//! ```ignore
//! fn xxx_main(args: impl ctcore::Args) -> UResult<()> {
//!     ...
//! }
//! ```
//! [CTResult]是围绕[Result]的一个简单封装，带有一个自定义错误特征：[CTError]。与实现了[std::error::Error]的类型相比，最重要的区别在于当从xxx_main返回时，[CTError]可以指定程序的退出码：
//! * 当返回Ok时，使用通过[set_ct_exit_code]设置的代码作为退出码。如果未使用[set_ct_exit_code]，则使用0。
//! * 当返回Err时，使用与错误对应的代码作为退出码，并显示错误消息。
//!
//! 此外，还可以使用[show]和[show_if_err]宏手动显示错误：
//! ```ignore
//! let res = Err(USimpleError::new(1, "Error!!"));
//! show_if_err!(res);
//! // or
//! if let Err(e) = res {
//!    show!(e);
//! }
//! ```
//!
//! 注意：show 和 show_if_err 宏通过调用 set_exit_code 函数设置程序的退出码。有关更多信息，请参见该函数的文档。
//!
//! # 指导原则
//! * 尽可能使用来自 ctcore 的错误类型。
//! * 如果一个错误出现在多个 utils 中，请将其添加到 ctcore。
//! * 优先使用适当的自定义错误类型，而不是 ExitCode 和 USimpleError。
//! * 对于具有简单错误处理的小型 utils，可以使用 USimpleError。
//! * 虽不推荐使用 ExitCode，但在将 utils 转换为使用 UResult 时可能会有用。

//拼写检查器：忽略 uioerror rustdoc

use std::{
    error::Error,
    fmt::{Display, Formatter},
    sync::atomic::{AtomicI32, Ordering},
};

static EXIT_CODE: AtomicI32 = AtomicI32::new(0);

/// 获取最后一次使用[set_ct_exit_code]设置的退出码。
/// 默认值为 0。
pub fn get_ct_exit_code() -> i32 {
    EXIT_CODE.load(Ordering::SeqCst)
}

/// 如果xxx_main返回Ok(())，则为程序设置退出码。
///
/// 本函数对于非致命错误最有用，例如在对多个文件应用操作时：
///
/// ```ignore
/// use ctcore::ct_error::{UResult, set_exit_code};
///
/// fn xxx_main(args: impl ctcore::Args) -> UResult<()> {
///     ...
///     for file in files {
///         let res = some_operation_that_might_fail(file);
///         match res {
///             Ok() => {},
///             Err(_) => set_exit_code(1),
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

/// 由 utils 和 ctcore 定义的自定义错误。
///
/// 所有错误应实现 [std::error::Error], [std::fmt::Display] 和 /// [std::fmt::Debug] 特征，
/// 并提供一个额外的 code 方法，用于指定当该错误从 ctmain 返回时程序的退出码。
///
/// 来自 ls 工具的一个自定义错误示例：
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
/// 主程序看起来像这样：
///
/// ```ignore
///
/// pub fn xxx_main(args: impl ctcore::Args) -> UResult<()> {
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
    ///
    /// 为枚举类型每个变体设置一个返回值，将错误码（返回给系统外壳）与错误变体关联起来。
    ///
    ///
    /// # 示例
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

    /// 将使用帮助打印到自定义错误中。
    ///
    /// 返回true或false来控制是否在错误消息下方打印简短的使用帮助。使用帮助的格式为：“试试{name} --help以获取更多信息。”仅当返回true时才会打印。
    ///
    /// # 示例
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

/// 包装类型，围绕着[std::io::Error]。
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
                    message = strip_errno(&self.inner);
                    &message
                }
            }
            None => {
                // 这些消息不需要过多规范化，而且上述
                // 消息并不总是合适的替代品。
                // 例如，ErrorKind::NotFound 并不一定意味着找不到文件。
                // 还有一些错误带有完全自定义的消息。
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

/// 从 IO 错误字符串中移除尾部的 " (os error XX)"。
pub fn strip_errno(err: &std::io::Error) -> String {
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

/// 允许从Result<T, nix::Error>转换到UResult<T>。
///
/// # 示例
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

/// 构造 UIoError 实例的简写。
///
/// 本宏作为一个便捷调用，可快速构造 UIoError 实例。它接受：
///
/// - 一个 std::io::Error 实例
/// - 一个与 format! 兼容的字符串以及
/// - 格式字符串所需的任意数量参数
///
/// 严格按照此顺序。它等同于示例中所展示的更冗长的代码。
///
/// # 示例
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
/// 如果您只想显示包含在UIoError中的std::io::ErrorKind的错误消息，请将第二个参数传递为空字符串：
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
/// ```
/// use ctcore::ct_error::{ExitCode, CTResult};
/// // Explicit
/// let res: CTResult<()> = Err(ExitCode(1).into());
///
/// // Using into on `i32`:
/// let res: CTResult<()> = Err(1.into());
/// ```
/// 此类型对于从返回i32的实用程序到返回UResult的简单转换特别有用。
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
/// 包含一个自定义错误码。当对这个结构体调用Display::fmt时，会直接将[clap::Error]打印到stdout或stderr。
/// 这是因为clap仅在直接打印时支持彩色输出。
///
/// 通常通过在[clap::Error]上调用[UClapError::with_exit_code]方法，
/// 或使用从[clap::Error]到Box<dyn UError>的[From]实现来创建[ClapErrorWrapper]，
/// 后者会创建一个退出码为1的ClapErrorWrapper实例。
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

/// 用于clap::Error调整退出码的扩展特性。
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
        match self.error.kind() {
            clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => 0,
            _ => self.code,
        }
    }
}

impl Error for ClapErrorWrapper {}

// 这是对Display特性的滥用
impl Display for ClapErrorWrapper {
    fn fmt(&self, _f: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
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
        assert_eq!(get_ct_exit_code(), 0);
    }

    #[test]
    fn test_set_exit_code() {
        // 测试设置退出码是否成功
        set_ct_exit_code(1);
        assert_eq!(get_ct_exit_code(), 1);
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
        assert_eq!(get_ct_exit_code(), 5);
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
