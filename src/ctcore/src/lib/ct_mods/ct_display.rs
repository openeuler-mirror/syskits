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

/// 字符和无效的Unicode。
///
/// 在信息性消息中显示路径时，请使用Quotable::quote。这将在文件名周围添加引号，并添加必要的转义，使其可复制粘贴到shell中。
///
/// 当输出不应被引号或转义时，使用println_verbatim将原始路径写入stdout。这将保留无效的Unicode。
///
/// # 示例
/// ```
/// use std::path::Path;
/// use ctcore::ct_display::{Quotable, ct_println_verbatim};
///
/// let path = Path::new("foo/bar.baz");
///
/// println!("Found file {}", path.quote()); // Prints "Found file 'foo/bar.baz'"
/// ct_println_verbatim(path)?; // Prints "foo/bar.baz"
/// # Ok::<(), std::io::Error>(())
/// ```
use std::ffi::OsStr;
use std::io::{self, Write as IoWrite};

#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(target_os = "wasi")]
use std::os::wasi::ffi::OsStrExt;

// 这些原本在这里定义，但现在它们有自己的 crate。
pub use os_display::{Quotable, Quoted};

/// 直接将路径（或类似OsStr的对象）打印到stdout，后面带一个换行符，即使其编码无效也不会丢失任何信息。
///
/// 该函数适用于打印路径是重点且输出可能被捕获的命令，如pwd和basename。对于信息性输出，请使用Quotable::quote。
///
/// FIXME：在Windows上这会丢失数据。它可能可以使用一些接受UTF-16的低级API来实现，而无需经过io::Write。这不是优先考虑的大事，因为在Windows上损坏的文件名比在Unix上罕见得多。
pub fn ct_println_verbatim<S: AsRef<OsStr>>(text: S) -> io::Result<()> {
    let output = io::stdout();
    let mut stdout = output.lock();
    #[cfg(any(unix, target_os = "wasi"))]
    {
        stdout.write_all(text.as_ref().as_bytes())?;
        stdout.write_all(b"\n")?;
    }
    #[cfg(not(any(unix, target_os = "wasi")))]
    {
        writeln!(stdout, "{}", std::path::Path::new(text.as_ref()).display())?;
    }
    Ok(())
}

/// 类似于 ct_println_verbatim，但不带尾部换行符。
pub fn ct_print_verbatim<S: AsRef<OsStr>>(text: S) -> io::Result<()> {
    let mut stdout = io::stdout();
    #[cfg(any(unix, target_os = "wasi"))]
    {
        stdout.write_all(text.as_ref().as_bytes())
    }
    #[cfg(not(any(unix, target_os = "wasi")))]
    {
        write!(stdout, "{}", std::path::Path::new(text.as_ref()).display())
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
            assert!(ct_println_verbatim(path).is_ok());
        }
    }
    /// 测试 `println_verbatim` 能否正确处理空路径。
    #[test]
    fn test_println_verbatim_with_empty_path() {
        let path = Path::new("");
        // 验证空路径不会导致错误
        assert!(ct_println_verbatim(path).is_ok());
    }

    /// 测试 `println_verbatim` 能否处理非常长的路径。
    #[test]
    fn test_println_verbatim_with_long_path() {
        // 生成一个非常长的路径字符串
        let long_path_str = std::iter::repeat("a").take(10000).collect::<String>();
        let path = Path::new(&long_path_str);
        // 验证长路径不会导致错误
        assert!(ct_println_verbatim(path).is_ok());
    }

    /// 测试 `println_verbatim` 在处理包含特殊文件系统字符的路径时的行为。
    #[test]
    fn test_println_verbatim_with_special_characters() {
        // 根据操作系统可能需要不同的测试路径
        #[cfg(unix)]
        let special_chars_path = Path::new("特殊路径/\n\t");
        // 验证特殊字符路径不会导致错误
        assert!(ct_println_verbatim(special_chars_path).is_ok());
    }

    /// 测试 `println_verbatim` 在处理仅包含特殊文件系统字符的路径时的行为。
    #[test]
    fn test_println_verbatim_with_only_special_characters() {
        // 根据操作系统可能需要不同的测试路径
        #[cfg(unix)]
        let path = Path::new("\n\t");
        // 验证路径仅包含特殊字符时不会导致错误
        assert!(ct_println_verbatim(path).is_ok());
    }
    /// 测试 `print_verbatim` 能否正确处理空路径。
    #[test]
    fn test_print_verbatim_with_empty_path() {
        let path = Path::new("");
        // 验证空路径不会导致错误
        assert!(ct_print_verbatim(path).is_ok());
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
        assert!(ct_print_verbatim(special_chars_path).is_ok());
    }

    /// 测试 `print_verbatim` 能否处理非常长的路径。
    #[test]
    fn test_print_verbatim_with_long_path() {
        let long_path_str = "a".repeat(10000);
        let path = Path::new(&long_path_str);
        // 验证长路径不会导致错误
        assert!(ct_print_verbatim(path).is_ok());
    }
}
