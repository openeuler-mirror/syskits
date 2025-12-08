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
/// Utilities for printing paths, with special attention paid to special
/// characters and invalid unicode.
///
/// For displaying paths in informational messages use `Quotable::quote`. This
/// will wrap quotes around the filename and add the necessary escapes to make
/// it copy/paste-able into a shell.
///
/// For writing raw paths to stdout when the output should not be quoted or escaped,
/// use `println_verbatim`. This will preserve invalid unicode.
///
/// # Examples
/// ```
/// use std::path::Path;
/// use ctcore::ct_display::{Quotable, println_verbatim};
///
/// let path = Path::new("foo/bar.baz");
///
/// println!("Found file {}", path.quote()); // Prints "Found file 'foo/bar.baz'"
/// println_verbatim(path)?; // Prints "foo/bar.baz"
/// # Ok::<(), std::io::Error>(())
/// ```
use std::ffi::OsStr;
use std::io::{self, Write as IoWrite};

#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(target_os = "wasi")]
use std::os::wasi::ffi::OsStrExt;

// These used to be defined here, but they live in their own crate now.
pub use os_display::{Quotable, Quoted};

/// Print a path (or `OsStr`-like object) directly to stdout, with a trailing newline,
/// without losing any information if its encoding is invalid.
///
/// This function is appropriate for commands where printing paths is the point and the
/// output is likely to be captured, like `pwd` and `basename`. For informational output
/// use `Quotable::quote`.
///
/// FIXME: This is lossy on Windows. It could probably be implemented using some low-level
/// API that takes UTF-16, without going through io::Write. This is not a big priority
/// because broken filenames are much rarer on Windows than on Unix.
pub fn println_verbatim<S: AsRef<OsStr>>(text: S) -> io::Result<()> {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    #[cfg(any(unix, target_os = "wasi"))]
    {
        stdout.write_all(text.as_ref().as_bytes())?;
        stdout.write_all(b"\n")?;
    }

    Ok(())
}

/// Like `println_verbatim`, without the trailing newline.
pub fn print_verbatim<S: AsRef<OsStr>>(text: S) -> io::Result<()> {
    let mut stdout = io::stdout();
    #[cfg(any(unix, target_os = "wasi"))]
    {
        stdout.write_all(text.as_ref().as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // use std::ffi::OsString;
    use std::path::Path;

    /// 测试 `println_verbatim` 函数能否在不同的输入下成功执行。
    #[test]
    fn test_println_verbatim_execution() {
        let paths = vec![
            "正常路径/foo.txt",
            "特殊字符路径/特?殊*字符|.txt",
            "路径含有空格/foo bar.txt",
        ];

        for path_str in paths {
            let path = Path::new(&path_str);
            // 注意：这里我们只能检查函数是否成功执行，而不能直接验证输出内容
            assert!(println_verbatim(path).is_ok());
        }
    }
    /// 测试 `println_verbatim` 能否正确处理空路径。
    #[test]
    fn test_println_verbatim_with_empty_path() {
        let path = Path::new("");
        // 验证空路径不会导致错误
        assert!(println_verbatim(path).is_ok());
    }

    /// 测试 `println_verbatim` 能否处理非常长的路径。
    #[test]
    fn test_println_verbatim_with_long_path() {
        // 生成一个非常长的路径字符串
        let long_path_str = std::iter::repeat("a").take(10000).collect::<String>();
        let path = Path::new(&long_path_str);
        // 验证长路径不会导致错误
        assert!(println_verbatim(path).is_ok());
    }

    /// 测试 `println_verbatim` 在处理包含特殊文件系统字符的路径时的行为。
    #[test]
    fn test_println_verbatim_with_special_characters() {
        // 根据操作系统可能需要不同的测试路径
        #[cfg(unix)]
        let special_chars_path = Path::new("特殊路径/\n\t");
        // 验证特殊字符路径不会导致错误
        assert!(println_verbatim(special_chars_path).is_ok());
    }

    /// 测试 `println_verbatim` 在处理仅包含特殊文件系统字符的路径时的行为。
    #[test]
    fn test_println_verbatim_with_only_special_characters() {
        // 根据操作系统可能需要不同的测试路径
        #[cfg(unix)]
        let path = Path::new("\n\t");
        // 验证路径仅包含特殊字符时不会导致错误
        assert!(println_verbatim(path).is_ok());
    }
    /// 测试 `print_verbatim` 能否正确处理空路径。
    #[test]
    fn test_print_verbatim_with_empty_path() {
        let path = Path::new("");
        // 验证空路径不会导致错误
        assert!(print_verbatim(path).is_ok());
    }

    /// 测试 `print_verbatim` 能否处理包含特殊字符的路径。
    #[test]
    fn test_print_verbatim_with_special_characters() {
        let special_chars_path = if cfg!(unix) {
            Path::new("特殊路径/\n\t")
        } else {
            // 注意：Windows 路径中不能包含以下字符，因此该测试仅适用于 Unix
            Path::new("特殊路径/<>:\"\\|?*")
        };

        // 验证特殊字符路径不会导致错误
        assert!(print_verbatim(special_chars_path).is_ok());
    }

    /// 测试 `print_verbatim` 能否处理非常长的路径。
    #[test]
    fn test_print_verbatim_with_long_path() {
        let long_path_str = "a".repeat(10000);
        let path = Path::new(&long_path_str);
        // 验证长路径不会导致错误
        assert!(print_verbatim(path).is_ok());
    }
}