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

//! 封装了操作系统之间关于文件句柄/描述符访问的差异。
//! 这在处理低级别的stdin/stdout访问时非常有用。
//!
//! 具体来说：
//! 在类Unix操作系统上，使用文件描述符。
//! 在Windows操作系统上，使用文件句柄。
//!
//! 尽管它们是不同的类，但它们共享共同的功能。
//! 对这种共同功能的访问在`OwnedFileDescriptorOrHandle`中提供。

#[cfg(not(windows))]
use std::os::fd::{AsFd, OwnedFd};
#[cfg(unix)]
use std::os::fd::{AsRawFd, OwnedFd as UnixOwnedFd};
#[cfg(windows)]
use std::os::windows::io::{AsHandle, OwnedHandle};
use std::{
    fs::{File, OpenOptions},
    io,
    path::Path,
    process::Stdio,
};

#[cfg(windows)]
type NativeType = OwnedHandle;
#[cfg(not(windows))]
type NativeType = OwnedFd;

/// 用于封装原生文件句柄/文件描述符的抽象层
pub struct CtOwnedFileDescriptorOrHandle {
    fx: NativeType,
}

impl CtOwnedFileDescriptorOrHandle {
    /// 从底层原生类型创建
    pub fn new(new_native_type: NativeType) -> Self {
        Self {
            fx: new_native_type,
        }
    }

    /// 通过打开文件来创建
    pub fn open_file(options: &OpenOptions, path: &Path) -> io::Result<Self> {
        let f = options.open(path)?;
        Self::from(f)
    }

    /// 借用原生类型的转换
    ///
    /// e.g. `std::io::stdout()`, `std::fs::File`, ...
    #[cfg(windows)]
    pub fn from<T: AsHandle>(t: T) -> io::Result<Self> {
        Ok(Self {
            fx: t.as_handle().try_clone_to_owned()?,
        })
    }

    /// conversion from borrowed native type
    ///
    /// e.g. `std::io::stdout()`, `std::fs::File`, ...
    #[cfg(not(windows))]
    pub fn from<T: AsFd>(t: T) -> io::Result<Self> {
        Ok(Self {
            fx: t.as_fd().try_clone_to_owned()?,
        })
    }

    /// 实例化相应的File
    pub fn into_file(self) -> File {
        File::from(self.fx)
    }

    /// 实例化相应的Stdio
    pub fn into_stdio(self) -> Stdio {
        Stdio::from(self.fx)
    }

    /// 克隆自身。当需要对同一文件的另一个拥有引用时有用
    pub fn try_clone(&self) -> io::Result<Self> {
        self.fx.try_clone().map(Self::new)
    }

    /// 提供用于直接与操作系统特定函数交互而不经过抽象层的原生类型
    pub fn as_raw(&self) -> &NativeType {
        &self.fx
    }
}

/// 实例化相应的Stdio
impl From<CtOwnedFileDescriptorOrHandle> for Stdio {
    fn from(value: CtOwnedFileDescriptorOrHandle) -> Self {
        value.into_stdio()
    }
}

#[cfg(unix)]
struct CtRawFdGuard {
    fd: i32,
}

#[cfg(unix)]
impl CtRawFdGuard {
    fn new(fd: i32) -> Self {
        Self { fd }
    }

    fn as_raw_fd(&self) -> i32 {
        self.fd
    }
}

#[cfg(unix)]
impl Drop for CtRawFdGuard {
    fn drop(&mut self) {
        let _ = nix::unistd::close(self.fd);
    }
}

#[cfg(unix)]
fn io_other(label: &str, stage: &str, err: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("{label}: {stage}: {err}"))
}

#[cfg(unix)]
pub fn with_stdin_writer<T, E, F, W, M>(
    label: &str,
    write_stdin: W,
    run: F,
    map_io_err: M,
) -> Result<T, E>
where
    F: FnOnce() -> Result<T, E>,
    W: FnOnce(File) -> io::Result<()> + Send + 'static,
    M: Copy + Fn(io::Error) -> E,
{
    let (stdin_read_fd, stdin_write_fd): (UnixOwnedFd, UnixOwnedFd) =
        nix::unistd::pipe().map_err(|e| map_io_err(io_other(label, "stdin pipe failed", e)))?;

    let saved_stdin = nix::unistd::dup(std::io::stdin().as_raw_fd())
        .map_err(|e| map_io_err(io_other(label, "dup stdin failed", e)))?;
    let saved_stdin = CtRawFdGuard::new(saved_stdin);

    let writer_thread = std::thread::spawn(move || {
        let file = File::from(stdin_write_fd);
        write_stdin(file)
    });

    if let Err(e) = nix::unistd::dup2(stdin_read_fd.as_raw_fd(), std::io::stdin().as_raw_fd()) {
        drop(stdin_read_fd);
        let _ = writer_thread.join();
        return Err(map_io_err(io_other(label, "redirect stdin failed", e)));
    }
    drop(stdin_read_fd);

    let run_result = run();
    let restore_stdin = nix::unistd::dup2(saved_stdin.as_raw_fd(), std::io::stdin().as_raw_fd());
    let writer_result = writer_thread.join();

    if let Err(e) = restore_stdin {
        return Err(map_io_err(io_other(label, "restore stdin failed", e)));
    }

    match writer_result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(map_io_err(io_other(label, "write stdin failed", e))),
        Err(_) => {
            return Err(map_io_err(io::Error::other(format!(
                "{label}: stdin writer thread panicked"
            ))));
        }
    }

    run_result
}

#[cfg(unix)]
pub fn with_stdin_writer_io<T, F, W>(label: &str, write_stdin: W, run: F) -> io::Result<T>
where
    F: FnOnce() -> io::Result<T>,
    W: FnOnce(File) -> io::Result<()> + Send + 'static,
{
    with_stdin_writer(label, write_stdin, run, |e| e)
}

#[cfg(not(unix))]
pub fn with_stdin_writer<T, E, F, W, M>(
    label: &str,
    _write_stdin: W,
    _run: F,
    map_io_err: M,
) -> Result<T, E>
where
    F: FnOnce() -> Result<T, E>,
    W: FnOnce(File) -> io::Result<()> + Send + 'static,
    M: Copy + Fn(io::Error) -> E,
{
    Err(map_io_err(io::Error::other(format!(
        "{label}: stdin redirection is unsupported on non-unix targets"
    ))))
}

#[cfg(not(unix))]
pub fn with_stdin_writer_io<T, F, W>(label: &str, _write_stdin: W, _run: F) -> io::Result<T>
where
    F: FnOnce() -> io::Result<T>,
    W: FnOnce(File) -> io::Result<()> + Send + 'static,
{
    Err(io::Error::other(format!(
        "{label}: stdin redirection is unsupported on non-unix targets"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::{self, Write};
    use tempfile::tempdir;

    #[test]
    fn test_open_file() -> io::Result<()> {
        let dir = tempdir()?;
        let file_path = dir.path().join("test_file.txt");

        let mut options_builder = OpenOptions::new();
        let options = options_builder.create(true).write(true).read(true);

        let handle = CtOwnedFileDescriptorOrHandle::open_file(options, &file_path)?;
        let mut file = handle.into_file();
        write!(file, "Hello, world!")?;
        file.sync_all()?;

        assert!(file_path.exists());
        Ok(())
    }

    #[test]
    fn test_into_stdio() -> io::Result<()> {
        let dir = tempdir()?;
        let file_path = dir.path().join("test_file.txt");
        let file = File::create(&file_path)?;
        let handle = CtOwnedFileDescriptorOrHandle::from(file)?;

        let stdio = handle.into_stdio();

        let mut echo_cmd = if cfg!(target_os = "windows") {
            let mut cmd = std::process::Command::new("cmd");
            cmd.args(["/C", "echo", "hello"]);
            cmd
        } else {
            let mut cmd = std::process::Command::new("echo");
            cmd.arg("hello");
            cmd
        };

        let output = echo_cmd.stdout(stdio).output()?;
        assert!(
            output.status.success(),
            "The command failed to execute properly"
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "",
            "Unexpected command output"
        );

        Ok(())
    }
    #[test]
    fn test_open_file_error_handling() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("non_existent_file.txt");
        let mut binding = OpenOptions::new();
        let options = binding.read(true); // Not creating the file

        assert!(CtOwnedFileDescriptorOrHandle::open_file(options, &file_path).is_err());
    }

    #[test]
    fn test_failed_conversion_due_to_state() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test_file.txt");
        let file = File::create(&file_path).unwrap();
        let file_clone = file.try_clone().unwrap(); // Use try_clone() to duplicate the file descriptor
        drop(file); // Close the file by dropping it

        // Now, the original file descriptor is closed, and attempting to create a handle from the cloned file descriptor
        let result = CtOwnedFileDescriptorOrHandle::from(file_clone);

        // We check if the operation is successful, which it should be, since try_clone() keeps the file accessible
        assert!(
            result.is_ok(),
            "Expected success as the file clone should still be open"
        );
    }

    #[test]
    fn test_try_clone_independence() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test_file.txt");
        let mut file = File::create(&file_path).unwrap();
        writeln!(file, "Original content").unwrap();

        let original = CtOwnedFileDescriptorOrHandle::from(&file).unwrap();
        let clone = original.try_clone().unwrap();

        let mut original_file = original.into_file();
        let mut clone_file = clone.into_file();

        writeln!(original_file, "Additional content from original").unwrap();
        writeln!(clone_file, "Additional content from clone").unwrap();

        original_file.sync_all().unwrap();
        clone_file.sync_all().unwrap();

        let contents = std::fs::read_to_string(file_path).unwrap();
        assert!(contents.contains("Original content"));
        assert!(contents.contains("Additional content from original"));
        assert!(contents.contains("Additional content from clone"));
    }
    #[test]
    fn test_as_raw_consistency() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test_file.txt");
        let file = File::create(&file_path).unwrap();
        let handle = CtOwnedFileDescriptorOrHandle::from(file).unwrap();

        let raw_before = handle.as_raw() as *const _; // Take a pointer for comparison
        let cloned_handle = handle.try_clone().unwrap();
        let raw_after = cloned_handle.as_raw() as *const _;

        assert_ne!(
            raw_before, raw_after,
            "Cloned handle should have a different raw pointer"
        );
    }
}
