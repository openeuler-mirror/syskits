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

//! 沙箱环境实现模块
//! 提供命令执行的隔离环境，支持资源限制和环境变量管理

use crate::CommandResult;
use crate::test_case::{
    FileType, InputStream, OutputStream, SignalDisposition, StandardStreams, TestCase, TestFile,
};
use crate::{Result, TestError};
use hex;
use nix::fcntl::{FcntlArg, OFlag, fcntl};
use nix::pty::{Winsize, openpty};
use nix::sys::resource::{self, Resource};
use nix::sys::signal::{self};
use nix::unistd::{dup, setsid};
use nix::{errno::Errno, libc};
use rand::Rng;
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::{self, File, Permissions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tempfile::TempDir;

fn tty_stdin_needs_eof(stdin_content: Option<&[u8]>) -> bool {
    stdin_content.is_some_and(|content| !content.is_empty())
}

pub(crate) struct CommandStreamOptions<'a> {
    pub output_hex: bool,
    pub streams: &'a StandardStreams,
}

struct PipeOutputCapture {
    reader: OwnedFd,
    prefixed_bytes: usize,
}

type ConfiguredOutput = (
    Stdio,
    Option<OwnedFd>,
    Option<OwnedFd>,
    Option<PipeOutputCapture>,
);

impl PipeOutputCapture {
    fn read_output(self) -> Result<Vec<u8>> {
        let mut reader = File::from(self.reader);
        let mut output = Vec::new();
        reader.read_to_end(&mut output)?;
        if output.len() < self.prefixed_bytes {
            return Err(TestError::ExecutionError(
                "Nonblocking pipe output was shorter than its prefilled prefix".to_string(),
            ));
        }
        Ok(output.split_off(self.prefixed_bytes))
    }
}

fn configured_output(mode: OutputStream) -> Result<ConfiguredOutput> {
    match mode {
        OutputStream::Capture => Ok((Stdio::piped(), None, None, None)),
        OutputStream::Inherit => Ok((Stdio::inherit(), None, None, None)),
        OutputStream::Null => Ok((Stdio::null(), None, None, None)),
        OutputStream::Full => {
            let full = fs::OpenOptions::new().write(true).open("/dev/full")?;
            Ok((Stdio::from(full), None, None, None))
        }
        OutputStream::ReadOnlyNull => {
            let null = fs::OpenOptions::new().read(true).open("/dev/null")?;
            Ok((Stdio::from(null), None, None, None))
        }
        OutputStream::Closed => Ok((Stdio::null(), None, None, None)),
        OutputStream::ClosedPipe => {
            let (read_end, write_end) = nix::unistd::pipe()
                .map_err(|e| TestError::ExecutionError(format!("Failed to create pipe: {e}")))?;
            drop(read_end);
            Ok((Stdio::from(write_end), None, None, None))
        }
        OutputStream::NonblockingFullPipe => {
            let (read_end, write_end) = nix::unistd::pipe()
                .map_err(|e| TestError::ExecutionError(format!("Failed to create pipe: {e}")))?;
            let mut flags = OFlag::from_bits_truncate(
                fcntl(write_end.as_raw_fd(), FcntlArg::F_GETFL).map_err(|e| {
                    TestError::ExecutionError(format!("Failed to read pipe flags: {e}"))
                })?,
            );
            flags.insert(OFlag::O_NONBLOCK);
            fcntl(write_end.as_raw_fd(), FcntlArg::F_SETFL(flags))
                .map_err(|e| TestError::ExecutionError(format!("Failed to set pipe flags: {e}")))?;
            let fill = [0_u8; 8192];
            loop {
                match nix::unistd::write(&write_end, &fill) {
                    Ok(_) => continue,
                    Err(Errno::EAGAIN) => break,
                    Err(e) => {
                        return Err(TestError::ExecutionError(format!(
                            "Failed to fill nonblocking pipe: {e}"
                        )));
                    }
                }
            }
            Ok((Stdio::from(write_end), None, Some(read_end), None))
        }
        OutputStream::NonblockingPartialPipe => {
            let (read_end, write_end) = nix::unistd::pipe()
                .map_err(|e| TestError::ExecutionError(format!("Failed to create pipe: {e}")))?;
            let mut flags = OFlag::from_bits_truncate(
                fcntl(write_end.as_raw_fd(), FcntlArg::F_GETFL).map_err(|e| {
                    TestError::ExecutionError(format!("Failed to read pipe flags: {e}"))
                })?,
            );
            flags.insert(OFlag::O_NONBLOCK);
            fcntl(write_end.as_raw_fd(), FcntlArg::F_SETFL(flags))
                .map_err(|e| TestError::ExecutionError(format!("Failed to set pipe flags: {e}")))?;

            let fill = [0_u8; 8192];
            let mut prefixed_bytes = 0;
            loop {
                match nix::unistd::write(&write_end, &fill) {
                    Ok(written) => prefixed_bytes += written,
                    Err(Errno::EAGAIN) => break,
                    Err(e) => {
                        return Err(TestError::ExecutionError(format!(
                            "Failed to fill nonblocking partial pipe: {e}"
                        )));
                    }
                }
            }

            const AVAILABLE_BYTES: usize = 4096;
            let mut released_bytes = 0;
            while released_bytes < AVAILABLE_BYTES {
                let mut buffer = [0_u8; AVAILABLE_BYTES];
                let read = nix::unistd::read(
                    read_end.as_raw_fd(),
                    &mut buffer[..AVAILABLE_BYTES - released_bytes],
                )
                .map_err(|e| {
                    TestError::ExecutionError(format!(
                        "Failed to release space in nonblocking partial pipe: {e}"
                    ))
                })?;
                if read == 0 {
                    return Err(TestError::ExecutionError(
                        "Nonblocking partial pipe ended before releasing PIPE_BUF bytes"
                            .to_string(),
                    ));
                }
                released_bytes += read;
            }

            Ok((
                Stdio::from(write_end),
                None,
                None,
                Some(PipeOutputCapture {
                    reader: read_end,
                    prefixed_bytes: prefixed_bytes - released_bytes,
                }),
            ))
        }
        OutputStream::NonblockingFullSocket => {
            let (reader, write_end) = UnixStream::pair().map_err(|e| {
                TestError::ExecutionError(format!("Failed to create Unix socket pair: {e}"))
            })?;
            let mut flags = OFlag::from_bits_truncate(
                fcntl(write_end.as_raw_fd(), FcntlArg::F_GETFL).map_err(|e| {
                    TestError::ExecutionError(format!("Failed to read socket flags: {e}"))
                })?,
            );
            flags.insert(OFlag::O_NONBLOCK);
            fcntl(write_end.as_raw_fd(), FcntlArg::F_SETFL(flags)).map_err(|e| {
                TestError::ExecutionError(format!("Failed to set socket flags: {e}"))
            })?;

            let fill = [0_u8; 8192];
            let mut prefixed_bytes = 0;
            loop {
                match nix::unistd::write(&write_end, &fill) {
                    Ok(written) => prefixed_bytes += written,
                    Err(Errno::EAGAIN) => break,
                    Err(e) => {
                        return Err(TestError::ExecutionError(format!(
                            "Failed to fill nonblocking socket: {e}"
                        )));
                    }
                }
            }

            Ok((
                Stdio::from(OwnedFd::from(write_end)),
                None,
                None,
                Some(PipeOutputCapture {
                    reader: OwnedFd::from(reader),
                    prefixed_bytes,
                }),
            ))
        }
        OutputStream::Tty => {
            let pty = openpty(None, None)
                .map_err(|e| TestError::ExecutionError(format!("Failed to create pty: {e}")))?;
            let master_fd = pty.master.as_raw_fd();
            let mut flags =
                OFlag::from_bits_truncate(fcntl(master_fd, FcntlArg::F_GETFL).map_err(|e| {
                    TestError::ExecutionError(format!("Failed to read pty flags: {e}"))
                })?);
            flags.insert(OFlag::O_NONBLOCK);
            fcntl(master_fd, FcntlArg::F_SETFL(flags))
                .map_err(|e| TestError::ExecutionError(format!("Failed to set pty flags: {e}")))?;
            Ok((Stdio::from(pty.slave), Some(pty.master), None, None))
        }
    }
}

fn read_pty_output(master: OwnedFd, process_done: Arc<AtomicBool>) -> Result<Vec<u8>> {
    let mut master = File::from(master);
    let mut output = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        match master.read(&mut buffer) {
            Ok(0) => break,
            Ok(size) => output.extend_from_slice(&buffer[..size]),
            Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if process_done.load(Ordering::Acquire) {
                    break;
                }
                thread::sleep(Duration::from_millis(2));
            }
            Err(error) => return Err(TestError::IoError(error)),
        }
    }

    let mut normalized = Vec::with_capacity(output.len());
    let mut index = 0;
    while index < output.len() {
        if output[index..].starts_with(b"\r\n") {
            normalized.push(b'\n');
            index += 2;
        } else {
            normalized.push(output[index]);
            index += 1;
        }
    }
    Ok(normalized)
}

/// 信号处理器
/// 用于处理测试过程中的信号（如 SIGTERM、SIGINT 等）
pub struct SignalHandler {
    /// 终止标志
    terminate: Arc<AtomicBool>,
    /// 需要处理的信号列表
    signals: Vec<signal::Signal>,
}

impl SignalHandler {
    /// 创建新的信号处理器
    pub fn new() -> Result<Self> {
        let terminate = Arc::new(AtomicBool::new(false));
        let signals = vec![
            signal::Signal::SIGTERM,
            signal::Signal::SIGINT,
            signal::Signal::SIGQUIT,
        ];

        let handler = Self { terminate, signals };

        handler.setup()?;
        Ok(handler)
    }

    /// 设置信号处理器
    fn setup(&self) -> Result<()> {
        let terminate = Arc::clone(&self.terminate);

        for &sig in &self.signals {
            let flag = Arc::clone(&terminate);
            signal_hook::flag::register(sig as i32, flag).map_err(|e| {
                TestError::ExecutionError(format!("Failed to register signal handler: {e}"))
            })?;
        }

        Ok(())
    }

    /// 检查是否应该终止执行
    pub fn should_terminate(&self) -> bool {
        self.terminate.load(Ordering::SeqCst)
    }
}

/// 资源限制器
/// 用于限制测试过程中的资源使用（CPU、内存、文件等）
pub struct ResourceLimiter {
    /// 资源限制映射表
    limits: HashMap<Resource, (u64, u64)>,
}

impl Default for ResourceLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl ResourceLimiter {
    /// 创建新的资源限制器
    pub fn new() -> Self {
        Self {
            limits: HashMap::new(),
        }
    }

    /// 添加资源限制
    pub fn add_limit(&mut self, resource: Resource, soft: u64, hard: u64) {
        self.limits.insert(resource, (soft, hard));
    }

    /// 应用资源限制
    pub fn apply_limits(&self) -> Result<()> {
        for (&resource, &(soft, hard)) in &self.limits {
            resource::setrlimit(resource, soft, hard).map_err(|e| {
                TestError::ExecutionError(format!("Failed to set resource limit: {e}"))
            })?;
        }
        Ok(())
    }
}

/// 增强的隔离沙箱
/// 提供命令执行的隔离环境，支持文件系统隔离、环境变量管理等
pub struct IsolatedSandbox {
    /// 沙箱唯一ID
    id: String,
    /// 临时目录
    temp_dir: Option<TempDir>,
    /// 资源限制器
    resource_limiter: Option<ResourceLimiter>,
    /// 当前环境变量
    current_env: HashMap<String, String>,
    /// 以原始字节传递的环境变量，覆盖同名的 UTF-8 环境变量。
    raw_env: HashMap<OsString, OsString>,
    /// 子进程是否不继承compat_test进程的环境变量。
    clear_environment: bool,
    /// 当前工作目录
    current_dir: PathBuf,
    /// 当前 umask
    umask: u32,
    /// 上一个命令的退出码
    exit_code: i32,
    /// 是否启用调试输出
    debug: bool,
}

fn split_shell_words(input: &str) -> std::result::Result<Vec<String>, &'static str> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;

    for character in input.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        match quote {
            Some('\'') => {
                if character == '\'' {
                    quote = None;
                } else {
                    current.push(character);
                }
            }
            Some('"') => match character {
                '"' => quote = None,
                '\\' => escaped = true,
                _ => current.push(character),
            },
            _ => match character {
                '\'' | '"' => quote = Some(character),
                '\\' => escaped = true,
                character if character.is_whitespace() => {
                    if !current.is_empty() {
                        words.push(std::mem::take(&mut current));
                    }
                }
                _ => current.push(character),
            },
        }
    }
    if escaped || quote.is_some() {
        return Err("unterminated quoted value");
    }
    if !current.is_empty() {
        words.push(current);
    }
    Ok(words)
}

impl IsolatedSandbox {
    /// 创建新的隔离沙箱
    pub fn new(debug: bool) -> Result<Self> {
        let temp_dir = TempDir::new()
            .map_err(|e| TestError::ExecutionError(format!("Failed to create sandbox: {e}")))?;
        let temp_path = temp_dir.path().to_path_buf();

        // 生成唯一ID：时间戳 + 随机数
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let random_num = rand::thread_rng().gen_range(0..1000000);
        let id = format!("{timestamp:x}-{random_num:x}");

        Ok(Self {
            id,
            temp_dir: Some(temp_dir),
            resource_limiter: Some(ResourceLimiter::new()),
            current_env: std::env::vars().collect(),
            raw_env: HashMap::new(),
            clear_environment: false,
            current_dir: temp_path,
            umask: 0o022,
            exit_code: 0,
            debug,
        })
    }

    /// 获取沙箱根路径
    pub fn path(&self) -> &Path {
        self.temp_dir.as_ref().unwrap().path()
    }

    /// 设置沙箱环境
    pub fn setup(&mut self, test_case: &TestCase) -> Result<()> {
        self.debug_fmt(format_args!("Starting sandbox environment setup"));
        self.debug_fmt(format_args!("Sandbox root directory: {:?}", self.path()));

        if test_case.environment.clear_env {
            self.current_env.clear();
            self.raw_env.clear();
        }
        self.clear_environment = test_case.environment.clear_env;
        self.current_env
            .extend(test_case.environment.env_vars.clone());
        for (name, value) in &test_case.environment.env_bytes {
            let value = hex::decode(value).map_err(|error| {
                TestError::TestCaseError(format!(
                    "Invalid hexadecimal value for environment variable {name}: {error}"
                ))
            })?;
            self.current_env.remove(name);
            self.raw_env
                .insert(OsString::from(name), OsString::from_vec(value));
        }

        // 创建测试所需的文件和目录
        for file in &test_case.environment.files {
            self.debug_fmt(format_args!("Creating test file: {file:?}"));
            self.debug_fmt(format_args!("File type: {:?}", file.file_type));
            self.create_test_file(file)?;
        }

        // 设置工作目录
        if let Some(ref working_dir) = test_case.environment.working_dir {
            let work_dir = self.path().join(working_dir);
            self.debug_fmt(format_args!(
                "Setting specified working directory: {work_dir:?}"
            ));
            std::env::set_current_dir(&work_dir)?;
            self.current_dir = work_dir;
        } else {
            self.debug_fmt(format_args!(
                "Using default working directory: {:?}",
                self.path()
            ));
            std::env::set_current_dir(self.path())?;
            self.current_dir = self.path().to_path_buf();
        }

        self.debug_fmt(format_args!(
            "Current working directory set to: {:?}",
            self.current_dir
        ));

        // 应用资源限制
        if let Some(ref limits) = test_case.environment.resource_limits {
            // 收集所有调试信息
            let mut debug_msgs = Vec::new();

            if let Some(ref mut limiter) = self.resource_limiter.as_mut() {
                if let Some(cpu_time) = limits.cpu_time {
                    limiter.add_limit(Resource::RLIMIT_CPU, cpu_time, cpu_time);
                    debug_msgs.push(format!("Setting CPU time limit: {cpu_time}"));
                }
                if let Some(file_size) = limits.file_size {
                    limiter.add_limit(Resource::RLIMIT_FSIZE, file_size, file_size);
                    debug_msgs.push(format!("Setting file size limit: {file_size}"));
                }
                if let Some(memory_size) = limits.memory_size {
                    limiter.add_limit(Resource::RLIMIT_AS, memory_size, memory_size);
                    debug_msgs.push(format!("Setting memory size limit: {memory_size}"));
                }
                if let Some(open_files) = limits.open_files {
                    limiter.add_limit(Resource::RLIMIT_NOFILE, open_files, open_files);
                    debug_msgs.push(format!("Setting open files limit: {open_files}"));
                }

                limiter.apply_limits()?;
            }

            // 完成可变借用后，统一输出调试信息
            for msg in debug_msgs {
                self.debug(&msg);
            }
        }

        self.debug_fmt(format_args!("Sandbox environment setup completed"));
        Ok(())
    }

    /// 创建测试文件
    fn create_test_file(&self, file: &TestFile) -> Result<()> {
        let path = self.path().join(&file.path);
        self.debug_fmt(format_args!("Creating file: {path:?}"));
        self.debug_fmt(format_args!("File type: {:?}", file.file_type));

        match file.file_type {
            FileType::Directory => {
                self.debug_fmt(format_args!("Creating directory: {path:?}"));
                fs::create_dir_all(&path)?;
                self.debug_fmt(format_args!("Directory created successfully"));
            }
            FileType::Regular => {
                if let Some(parent) = path.parent() {
                    self.debug_fmt(format_args!("Creating parent directory: {parent:?}"));
                    fs::create_dir_all(parent)?;
                }
                self.debug_fmt(format_args!("Creating file: {path:?}"));
                let mut file_handle = File::create(&path)?;
                if let Some(ref content) = file.content {
                    self.debug_fmt(format_args!(
                        "Writing file content, length: {}",
                        content.len()
                    ));
                    file_handle.write_all(content.as_bytes())?;
                }
                if let Some(ref perms) = file.permissions {
                    self.debug_fmt(format_args!("Setting file permissions: {perms}"));
                    let mode = u32::from_str_radix(perms, 8).map_err(|e| {
                        TestError::ExecutionError(format!("Invalid permissions: {e}"))
                    })?;
                    fs::set_permissions(&path, Permissions::from_mode(mode))?;
                }
                self.debug_fmt(format_args!("File created successfully"));
            }
            FileType::Symlink => {
                if let Some(ref target) = file.symlink_target {
                    self.debug_fmt(format_args!("Creating symlink: {path:?} -> {target:?}"));
                    std::os::unix::fs::symlink(target, &path)?;
                    self.debug_fmt(format_args!("Symlink created successfully"));
                }
            }
            _ => {
                return Err(TestError::ExecutionError(format!(
                    "File type {:?} is not supported in the sandbox",
                    file.file_type
                )));
            }
        }

        Ok(())
    }

    /// 在隔离环境中执行函数
    pub fn execute_isolated<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce() -> Result<T>,
    {
        let result = f()?;
        Ok(result)
    }

    /// 清理沙箱环境
    pub fn cleanup(&self) -> Result<()> {
        std::env::set_current_dir(self.path().parent().unwrap())?;
        Ok(())
    }

    /// 设置CPU时间限制
    pub fn set_cpu_time_limit(&mut self, limit: u64) -> Result<()> {
        if let Some(ref mut limiter) = self.resource_limiter {
            limiter.add_limit(Resource::RLIMIT_CPU, limit, limit);
            limiter.apply_limits()?;
        }
        Ok(())
    }

    /// 设置内存限制
    pub fn set_memory_limit(&mut self, limit: u64) -> Result<()> {
        if let Some(ref mut limiter) = self.resource_limiter {
            limiter.add_limit(Resource::RLIMIT_AS, limit, limit);
            limiter.apply_limits()?;
        }
        Ok(())
    }

    /// 设置打开文件数限制
    pub fn set_open_files_limit(&mut self, limit: u64) -> Result<()> {
        if let Some(ref mut limiter) = self.resource_limiter {
            limiter.add_limit(Resource::RLIMIT_NOFILE, limit, limit);
            limiter.apply_limits()?;
        }
        Ok(())
    }

    /// 执行shell命令
    pub fn execute_shell_command(&mut self, command: &str) -> Result<CommandResult> {
        // 解析命令，处理shell内建命令
        match command.split_whitespace().next() {
            Some("cd") => self.builtin_cd(command),
            Some("export") => self.builtin_export(command),
            Some("unset") => self.builtin_unset(command),
            Some("umask") => self.builtin_umask(command),
            _ => self.execute_external_command(command),
        }
    }

    /// 处理cd命令
    fn builtin_cd(&mut self, command: &str) -> Result<CommandResult> {
        let args: Vec<&str> = command.split_whitespace().collect();
        let new_dir = args.get(1).copied().unwrap_or("~");

        let target_dir = match new_dir {
            "~" => dirs::home_dir().ok_or_else(|| {
                TestError::ExecutionError("Cannot get home directory".to_string())
            })?,
            "-" => self.current_dir.clone(), // TODO: 实现 OLDPWD
            _ => {
                if new_dir.starts_with('/') {
                    PathBuf::from(new_dir)
                } else {
                    self.current_dir.join(new_dir)
                }
            }
        };

        if target_dir.exists() && target_dir.is_dir() {
            self.current_dir = target_dir.clone();
            Ok(CommandResult::default())
        } else {
            Ok(CommandResult {
                stdout: String::new(),
                stderr: format!("cd: {new_dir}: No such file or directory\n"),
                exit_code: 1,
            })
        }
    }

    /// 处理export命令
    fn builtin_export(&mut self, command: &str) -> Result<CommandResult> {
        let args = match split_shell_words(command.strip_prefix("export").unwrap_or_default()) {
            Ok(args) => args,
            Err(message) => {
                return Ok(CommandResult {
                    stdout: String::new(),
                    stderr: format!("export: {message}\n"),
                    exit_code: 1,
                });
            }
        };
        for arg in args {
            if let Some((key, value)) = arg.split_once('=') {
                self.current_env.insert(key.to_string(), value.to_string());
                self.raw_env.remove(&OsString::from(key));
            }
        }
        Ok(CommandResult::default())
    }

    /// 处理unset命令
    fn builtin_unset(&mut self, command: &str) -> Result<CommandResult> {
        let args = match split_shell_words(command.strip_prefix("unset").unwrap_or_default()) {
            Ok(args) => args,
            Err(message) => {
                return Ok(CommandResult {
                    stdout: String::new(),
                    stderr: format!("unset: {message}\n"),
                    exit_code: 1,
                });
            }
        };
        for key in args {
            self.current_env.remove(&key);
            self.raw_env.remove(&OsString::from(key));
        }
        Ok(CommandResult::default())
    }

    /// 获取环境变量
    pub fn get_env(&self, key: &str) -> Option<&str> {
        self.current_env.get(key).map(|s| s.as_str())
    }

    /// 添加环境变量
    pub fn add_env(&mut self, key: &str, value: &str) {
        self.current_env.insert(key.to_string(), value.to_string());
        self.raw_env.remove(&OsString::from(key));
    }

    /// 获取当前环境变量集合
    pub fn get_current_env(&self) -> &HashMap<String, String> {
        &self.current_env
    }

    /// 获取当前工作目录
    pub fn get_current_dir(&self) -> &PathBuf {
        &self.current_dir
    }

    /// 更新命令执行状态
    pub fn update_status(&mut self, result: &CommandResult) {
        self.exit_code = result.exit_code;
    }

    /// 处理umask命令
    fn builtin_umask(&mut self, command: &str) -> Result<CommandResult> {
        let args: Vec<&str> = command.split_whitespace().collect();
        if let Some(mode) = args.get(1) {
            if let Ok(new_umask) = u32::from_str_radix(mode, 8) {
                self.umask = new_umask;
                Ok(CommandResult::default())
            } else {
                Ok(CommandResult {
                    stdout: String::new(),
                    stderr: format!("umask: invalid mode: {mode}\n"),
                    exit_code: 1,
                })
            }
        } else {
            // 显示当前umask
            Ok(CommandResult {
                stdout: format!("{:03o}\n", self.umask),
                stderr: String::new(),
                exit_code: 0,
            })
        }
    }

    /// 输出调试信息
    fn debug(&self, msg: &str) {
        if self.debug {
            eprintln!("DEBUG [{}]: {}", self.id, msg);
        }
    }

    /// 输出调试信息（带格式化）
    fn debug_fmt(&self, fmt: std::fmt::Arguments<'_>) {
        if self.debug {
            eprintln!("DEBUG [{}]: {}", self.id, fmt);
        }
    }

    /// 执行命令（字符串参数）
    pub fn execute_command(
        &mut self,
        cmd: &str,
        args: &[String],
        stdin_content: Option<&str>,
        is_record_result: bool,
        timeout: Option<u64>,
    ) -> Result<CommandResult> {
        let os_args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let stdin_bytes = stdin_content.map(|s| s.as_bytes());
        self.execute_command_bytes(cmd, &os_args, stdin_bytes, is_record_result, timeout, false)
    }

    /// 执行命令，并按用例配置连接标准输出、标准错误和 SIGPIPE。
    pub fn execute_command_with_streams(
        &mut self,
        cmd: &str,
        args: &[String],
        stdin_content: Option<&str>,
        is_record_result: bool,
        timeout: Option<u64>,
        streams: &StandardStreams,
    ) -> Result<CommandResult> {
        let os_args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let stdin_bytes = stdin_content.map(str::as_bytes);
        self.execute_command_bytes_with_streams(
            cmd,
            &os_args,
            stdin_bytes,
            is_record_result,
            timeout,
            CommandStreamOptions {
                output_hex: false,
                streams,
            },
        )
    }

    /// 执行命令（字符串参数，伪终端模式）
    pub fn execute_command_tty(
        &mut self,
        cmd: &str,
        args: &[String],
        stdin_content: Option<&str>,
        is_record_result: bool,
        timeout: Option<u64>,
    ) -> Result<CommandResult> {
        let os_args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let stdin_bytes = stdin_content.map(|s| s.as_bytes());
        self.execute_command_bytes_tty(cmd, &os_args, stdin_bytes, is_record_result, timeout, false)
    }

    /// 执行命令（原始字节参数）
    pub fn execute_command_bytes(
        &mut self,
        cmd: &str,
        args: &[OsString],
        stdin_content: Option<&[u8]>,
        is_record_result: bool,
        timeout: Option<u64>,
        output_hex: bool,
    ) -> Result<CommandResult> {
        self.execute_command_bytes_with_streams(
            cmd,
            args,
            stdin_content,
            is_record_result,
            timeout,
            CommandStreamOptions {
                output_hex,
                streams: &StandardStreams::default(),
            },
        )
    }

    /// 执行命令（原始字节参数及可配置标准流）。
    pub(crate) fn execute_command_bytes_with_streams(
        &mut self,
        cmd: &str,
        args: &[OsString],
        stdin_content: Option<&[u8]>,
        is_record_result: bool,
        timeout: Option<u64>,
        options: CommandStreamOptions<'_>,
    ) -> Result<CommandResult> {
        self.debug_fmt(format_args!("Executing command: {cmd} {args:?}"));
        self.debug_fmt(format_args!(
            "Current working directory: {:?}",
            self.current_dir
        ));

        let encode_if_hex = |value: &str| {
            if options.output_hex {
                hex::encode(value.as_bytes())
            } else {
                value.to_string()
            }
        };

        let streams = options.streams;
        let close_stdin = streams.stdin_file.is_none() && streams.stdin == InputStream::Closed;
        let keep_stdin_open =
            streams.stdin_file.is_none() && streams.stdin == InputStream::OpenPipe;
        let close_stdout = streams.stdout == OutputStream::Closed;
        let close_stderr = streams.stderr == OutputStream::Closed;
        let stdin = match streams.stdin_file.as_deref() {
            Some(path) => {
                let path = Path::new(path);
                let path = if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    self.current_dir.join(path)
                };
                let mut file = File::open(path)?;
                if let Some(offset) = streams.stdin_offset {
                    file.seek(SeekFrom::Start(offset))?;
                }
                Stdio::from(file)
            }
            None if close_stdin => Stdio::null(),
            None => Stdio::piped(),
        };
        let (stdout, stdout_tty, _stdout_pipe_keepalive, stdout_pipe_capture) =
            configured_output(streams.stdout)?;
        let (stderr, stderr_tty, _stderr_pipe_keepalive, stderr_pipe_capture) =
            configured_output(streams.stderr)?;
        let mut command = if streams.use_bash {
            let mut command = Command::new("bash");
            command
                .args(["-c", "\"$@\"; status=$?; :; exit \"$status\"", "bash", cmd])
                .args(args);
            command
        } else {
            let mut command = Command::new(cmd);
            command.args(args);
            command
        };
        if self.clear_environment {
            command.env_clear();
        }
        command
            .stdin(stdin)
            .stdout(stdout)
            .stderr(stderr)
            .current_dir(&self.current_dir)
            .envs(&self.current_env)
            .envs(&self.raw_env);

        let sigpipe = streams.sigpipe;
        unsafe {
            command.pre_exec(move || {
                let handler = match sigpipe {
                    SignalDisposition::Default => signal::SigHandler::SigDfl,
                    SignalDisposition::Ignore => signal::SigHandler::SigIgn,
                };
                signal::signal(signal::Signal::SIGPIPE, handler).map_err(std::io::Error::other)?;
                if close_stdin && libc::close(libc::STDIN_FILENO) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if close_stdout && libc::close(libc::STDOUT_FILENO) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if close_stderr && libc::close(libc::STDERR_FILENO) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                self.debug_fmt(format_args!("Command execution failed: {e}"));
                self.debug_fmt(format_args!("Error type: {:?}", e.kind()));
                let stderr = encode_if_hex(&format!("Failed to execute command: {e}"));
                return Ok(CommandResult {
                    stdout: String::new(),
                    stderr,
                    exit_code: 127, // Common error code for command not found
                });
            }
        };
        drop(command);
        let stdin_keepalive = if keep_stdin_open {
            child
                .stdin
                .as_ref()
                .map(|stdin| -> Result<OwnedFd> {
                    let fd = dup(stdin.as_raw_fd())?;
                    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
                })
                .transpose()?
        } else {
            None
        };
        let pty_readers_done = Arc::new(AtomicBool::new(false));
        let stdout_tty_reader = stdout_tty.map(|master| {
            let process_done = Arc::clone(&pty_readers_done);
            thread::spawn(move || read_pty_output(master, process_done))
        });
        let stderr_tty_reader = stderr_tty.map(|master| {
            let process_done = Arc::clone(&pty_readers_done);
            thread::spawn(move || read_pty_output(master, process_done))
        });

        // 启动命令
        if !keep_stdin_open {
            if let Some(content) = stdin_content {
                if let Some(stdin) = child.stdin.as_mut() {
                    if !content.is_empty() {
                        if let Err(e) = stdin.write_all(content) {
                            let _ = child.kill();
                            pty_readers_done.store(true, Ordering::Release);
                            self.debug_fmt(format_args!("Failed to write to stdin: {e}"));
                            let stderr = encode_if_hex(&format!("Failed to write to stdin: {e}"));
                            return Ok(CommandResult {
                                stdout: String::new(),
                                stderr,
                                exit_code: 1,
                            });
                        }
                    }
                    // Always close stdin so the child can observe EOF.
                    drop(child.stdin.take());
                }
            }
        }

        let mut output;
        let timeout_args;
        // 等待命令执行完成并获取输出
        if let Some(timeout_secs) = timeout {
            timeout_args = Duration::from_secs(timeout_secs);
            let start = std::time::Instant::now();

            loop {
                if start.elapsed() >= timeout_args {
                    child.kill().unwrap();
                    break;
                }

                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) => thread::sleep(Duration::from_millis(100)),
                    Err(e) => {
                        let _ = child.kill();
                        pty_readers_done.store(true, Ordering::Release);
                        let stderr = encode_if_hex(&format!("Failed to wait for command: {e}"));
                        return Ok(CommandResult {
                            stdout: String::new(),
                            stderr,
                            exit_code: 1,
                        });
                    }
                }
            }

            output = match child.wait_with_output() {
                Ok(output) => {
                    self.debug_fmt(format_args!("Command executed successfully"));
                    output
                }
                Err(e) => {
                    pty_readers_done.store(true, Ordering::Release);
                    self.debug_fmt(format_args!("Failed to wait for command: {e}"));
                    let stderr = encode_if_hex(&format!("Failed to wait for command: {e}"));
                    return Ok(CommandResult {
                        stdout: String::new(),
                        stderr,
                        exit_code: 1,
                    });
                }
            };
        } else {
            output = match child.wait_with_output() {
                Ok(output) => {
                    self.debug_fmt(format_args!("Command executed successfully"));
                    output
                }
                Err(e) => {
                    pty_readers_done.store(true, Ordering::Release);
                    self.debug_fmt(format_args!("Failed to wait for command: {e}"));
                    let stderr = encode_if_hex(&format!("Failed to wait for command: {e}"));
                    return Ok(CommandResult {
                        stdout: String::new(),
                        stderr,
                        exit_code: 1,
                    });
                }
            };
        }

        drop(stdin_keepalive);

        pty_readers_done.store(true, Ordering::Release);
        if let Some(reader) = stdout_tty_reader {
            output.stdout = reader.join().map_err(|_| {
                TestError::ExecutionError("stdout PTY reader thread panicked".to_string())
            })??;
        }
        if let Some(reader) = stderr_tty_reader {
            output.stderr = reader.join().map_err(|_| {
                TestError::ExecutionError("stderr PTY reader thread panicked".to_string())
            })??;
        }
        if let Some(capture) = stdout_pipe_capture {
            output.stdout = capture.read_output()?;
        }
        if let Some(capture) = stderr_pipe_capture {
            output.stderr = capture.read_output()?;
        }

        let result = if options.output_hex {
            CommandResult::from_output_hex(output)
        } else {
            CommandResult::from(output)
        };

        self.debug_fmt(format_args!("Command execution results:"));
        self.debug_fmt(format_args!("exit_code: {}", result.exit_code));
        self.debug_fmt(format_args!("stdout: {}", result.stdout));
        self.debug_fmt(format_args!("stderr: {}", result.stderr));

        // Check if stdout contains null bytes
        if !options.output_hex && result.stdout.contains('\0') {
            self.debug("Warning: stdout contains null bytes");
            if self.debug {
                println!("DEBUG: stdout hex representation:");
                for (i, byte) in result.stdout.as_bytes().iter().enumerate().take(100) {
                    print!("{byte:02x} ");
                    if (i + 1) % 16 == 0 {
                        println!();
                    }
                }
                println!("...");
            }
        }

        // Save command execution results to environment variables for verification
        if is_record_result {
            self.debug_fmt(format_args!(
                "Setting environment variable CMD_EXIT_CODE={}",
                result.exit_code
            ));
            self.add_env("CMD_EXIT_CODE", &result.exit_code.to_string());

            // Check for null bytes in stdout before setting environment variable
            if !options.output_hex && result.stdout.contains('\0') {
                self.debug("Warning: Found null bytes when setting CMD_STDOUT");
                // Replace null bytes with visible characters to avoid environment variable issues
                let safe_stdout = result.stdout.replace('\0', "\\0");
                self.add_env("CMD_STDOUT", &safe_stdout);
            } else {
                self.add_env("CMD_STDOUT", &result.stdout);
            }

            self.add_env("CMD_STDERR", &result.stderr);
        }

        self.update_status(&result);
        Ok(result)
    }

    /// 执行命令（伪终端模式）
    pub fn execute_command_bytes_tty(
        &mut self,
        cmd: &str,
        args: &[OsString],
        stdin_content: Option<&[u8]>,
        is_record_result: bool,
        timeout: Option<u64>,
        output_hex: bool,
    ) -> Result<CommandResult> {
        self.debug_fmt(format_args!("Executing command (tty): {cmd} {args:?}"));
        self.debug_fmt(format_args!(
            "Current working directory: {:?}",
            self.current_dir
        ));

        let encode_if_hex = |value: &str| {
            if output_hex {
                hex::encode(value.as_bytes())
            } else {
                value.to_string()
            }
        };

        let winsize = Winsize {
            ws_row: 200,
            ws_col: 80,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let pty = openpty(Some(&winsize), None)
            .map_err(|e| TestError::ExecutionError(format!("Failed to create pty: {e}")))?;
        let master = pty.master;
        let master_fd = master.as_raw_fd();
        let mut flags = OFlag::from_bits_truncate(fcntl(master_fd, FcntlArg::F_GETFL).unwrap_or(0));
        flags.insert(OFlag::O_NONBLOCK);
        let _ = fcntl(master_fd, FcntlArg::F_SETFL(flags));

        let slave_fd = pty.slave.into_raw_fd();
        let stdin_fd = dup(slave_fd)
            .map_err(|e| TestError::ExecutionError(format!("Failed to dup pty slave: {e}")))?;
        let stdout_fd = dup(slave_fd)
            .map_err(|e| TestError::ExecutionError(format!("Failed to dup pty slave: {e}")))?;
        let stderr_fd = dup(slave_fd)
            .map_err(|e| TestError::ExecutionError(format!("Failed to dup pty slave: {e}")))?;
        let slave_fd_for_ioctl = slave_fd;

        let mut command = Command::new(cmd);
        unsafe {
            command
                .stdin(Stdio::from_raw_fd(stdin_fd))
                .stdout(Stdio::from_raw_fd(stdout_fd))
                .stderr(Stdio::from_raw_fd(stderr_fd));
        }
        if self.clear_environment {
            command.env_clear();
        }
        command
            .args(args)
            .current_dir(&self.current_dir)
            .envs(&self.current_env)
            .envs(&self.raw_env);

        unsafe {
            command.pre_exec(move || {
                setsid().map_err(std::io::Error::other)?;
                let rc = libc::ioctl(slave_fd_for_ioctl, libc::TIOCSCTTY as libc::c_ulong, 0);
                if rc < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                self.debug_fmt(format_args!("Command execution failed: {e}"));
                self.debug_fmt(format_args!("Error type: {:?}", e.kind()));
                let stderr = encode_if_hex(&format!("Failed to execute command: {e}"));
                return Ok(CommandResult {
                    stdout: String::new(),
                    stderr,
                    exit_code: 127,
                });
            }
        };

        let mut output = Vec::new();

        if let Some(content) = stdin_content
            && !content.is_empty()
        {
            unsafe {
                let mut written = 0;
                while written < content.len() {
                    let rc = libc::write(
                        master_fd,
                        content[written..].as_ptr() as *const libc::c_void,
                        content.len() - written,
                    );
                    if rc < 0 {
                        let err = std::io::Error::last_os_error();
                        if err.kind() == std::io::ErrorKind::WouldBlock {
                            thread::sleep(Duration::from_millis(10));
                            continue;
                        }
                        let stderr =
                            encode_if_hex(&format!("Failed to write to pty master: {err}"));
                        return Ok(CommandResult {
                            stdout: String::new(),
                            stderr,
                            exit_code: 1,
                        });
                    }
                    written += rc as usize;
                }
            }

            let echo_deadline = std::time::Instant::now() + Duration::from_millis(20);
            loop {
                let mut buf = [0u8; 4096];
                let rc = unsafe {
                    libc::read(master_fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len())
                };
                if rc > 0 {
                    output.extend_from_slice(&buf[..rc as usize]);
                    continue;
                }
                if rc == 0 {
                    break;
                }

                let err = Errno::last();
                if err == Errno::EAGAIN {
                    if std::time::Instant::now() >= echo_deadline {
                        break;
                    }
                    thread::sleep(Duration::from_millis(2));
                    continue;
                }
                if err == Errno::EIO {
                    break;
                }

                let stderr = encode_if_hex(&format!(
                    "Failed to read from pty: {}",
                    std::io::Error::last_os_error()
                ));
                return Ok(CommandResult {
                    stdout: String::new(),
                    stderr,
                    exit_code: 1,
                });
            }
        }

        if tty_stdin_needs_eof(stdin_content) {
            unsafe {
                let eof = b"\x04";
                libc::write(master_fd, eof.as_ptr() as *const libc::c_void, 1);
            }
        }

        // Close the slave fd in the parent to allow EOF on master when the child exits.
        unsafe {
            libc::close(slave_fd);
        }

        let start = std::time::Instant::now();
        let mut child_exited = false;
        let mut child_status = None;
        let mut child_exit_at = None;

        loop {
            if let Some(timeout_secs) = timeout {
                if start.elapsed() >= Duration::from_secs(timeout_secs) {
                    let _ = child.kill();
                    break;
                }
            }

            let mut buf = [0u8; 4096];
            let rc =
                unsafe { libc::read(master_fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if rc == 0 {
                break;
            } else if rc < 0 {
                let err = Errno::last();
                if err == Errno::EAGAIN {
                    if !child_exited {
                        if let Ok(Some(status)) = child.try_wait() {
                            child_exited = true;
                            child_status = Some(status);
                            child_exit_at = Some(std::time::Instant::now());
                        }
                    }
                    if child_exited {
                        if child_exit_at.is_some_and(|t| t.elapsed() >= Duration::from_millis(50)) {
                            break;
                        }
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    thread::sleep(Duration::from_millis(10));
                } else {
                    let stderr = encode_if_hex(&format!(
                        "Failed to read from pty: {}",
                        std::io::Error::last_os_error()
                    ));
                    return Ok(CommandResult {
                        stdout: String::new(),
                        stderr,
                        exit_code: 1,
                    });
                }
            } else {
                output.extend_from_slice(&buf[..rc as usize]);
            }
        }

        let status = if let Some(status) = child_status {
            status
        } else {
            match child.wait() {
                Ok(status) => status,
                Err(e) => {
                    let stderr = encode_if_hex(&format!("Failed to wait for command: {e}"));
                    return Ok(CommandResult {
                        stdout: String::new(),
                        stderr,
                        exit_code: 1,
                    });
                }
            }
        };

        let exit_code = if let Some(signal) = status.signal() {
            128 + signal
        } else {
            status.code().unwrap_or(-1)
        };

        let stdout = if output_hex {
            hex::encode(&output)
        } else {
            String::from_utf8_lossy(&output).into_owned()
        };

        let result = CommandResult {
            stdout,
            stderr: String::new(),
            exit_code,
        };

        self.debug_fmt(format_args!("Command execution results (tty):"));
        self.debug_fmt(format_args!("exit_code: {}", result.exit_code));
        self.debug_fmt(format_args!("stdout: {}", result.stdout));

        if is_record_result {
            self.debug_fmt(format_args!(
                "Setting environment variable CMD_EXIT_CODE={}",
                result.exit_code
            ));
            self.add_env("CMD_EXIT_CODE", &result.exit_code.to_string());

            if !output_hex && result.stdout.contains('\0') {
                self.debug("Warning: Found null bytes when setting CMD_STDOUT");
                let safe_stdout = result.stdout.replace('\0', "\\0");
                self.add_env("CMD_STDOUT", &safe_stdout);
            } else {
                self.add_env("CMD_STDOUT", &result.stdout);
            }

            self.add_env("CMD_STDERR", &result.stderr);
        }

        self.update_status(&result);
        Ok(result)
    }

    /// 执行外部命令
    fn execute_external_command(&mut self, command: &str) -> Result<CommandResult> {
        self.debug_fmt(format_args!("Executing external command: {command}"));
        self.debug_fmt(format_args!(
            "Current working directory: {:?}",
            self.current_dir
        ));

        // Use /bin/sh -c to execute command to support shell features
        let mut shell_cmd = std::process::Command::new("/bin/sh");
        shell_cmd
            .arg("-c")
            .arg(command)
            .current_dir(&self.current_dir)
            .envs(&self.current_env)
            .envs(&self.raw_env)
            // 设置标准输入/输出/错误
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        self.debug_fmt(format_args!(
            "Full command: cd {:?} && /bin/sh -c {:?}",
            self.current_dir, command
        ));
        self.debug_fmt(format_args!(
            "Environment variables: {:?}",
            self.current_env
        ));

        // Use output() result regardless of success or failure
        let result = match shell_cmd.output() {
            Ok(output) => {
                self.debug_fmt(format_args!("Command executed successfully"));
                CommandResult::from(output)
            }
            Err(e) => {
                self.debug_fmt(format_args!("Command execution failed: {e}"));
                self.debug_fmt(format_args!("Error type: {:?}", e.kind()));
                CommandResult {
                    stdout: String::new(),
                    stderr: format!("Failed to execute command: {e}"),
                    exit_code: 127, // Common error code for command not found
                }
            }
        };

        self.debug_fmt(format_args!("External command execution results:"));
        self.debug_fmt(format_args!("exit_code: {}", result.exit_code));
        self.debug_fmt(format_args!("stdout: {}", result.stdout));
        self.debug_fmt(format_args!("stderr: {}", result.stderr));

        self.update_status(&result);
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_case::{
        CommandExecution, IgnoreFields, OutputStream, SignalDisposition, StandardStreams, TestCase,
        TestEnvironment, TestExpectation,
    };
    use std::ffi::OsString;
    use std::fs;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    #[test]
    fn test_sandbox_new() -> Result<()> {
        let sandbox = IsolatedSandbox::new(true)?;
        assert!(sandbox.path().exists());
        assert!(sandbox.path().is_dir());
        Ok(())
    }

    #[test]
    fn test_execute_command_simple() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;
        let result = sandbox.execute_command("echo", &["hello".to_string()], None, true, None)?;
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout.trim(), "hello");
        assert_eq!(result.stderr, "");
        Ok(())
    }

    #[test]
    fn test_execute_command_with_stdin() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;
        let result = sandbox.execute_command("cat", &[], Some("test input"), true, None)?;
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "test input");
        assert_eq!(result.stderr, "");
        Ok(())
    }

    #[test]
    fn test_execute_command_with_open_stdin_pipe_keeps_child_waiting() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;
        let streams = StandardStreams {
            stdin: InputStream::OpenPipe,
            ..StandardStreams::default()
        };

        let result = sandbox.execute_command_with_streams(
            "cat",
            &[],
            Some("ignored input"),
            true,
            Some(1),
            &streams,
        )?;

        assert_eq!(result.exit_code, 137);
        assert!(result.stdout.is_empty());
        assert!(result.stderr.is_empty());
        Ok(())
    }

    #[test]
    fn test_execute_command_with_regular_file_stdin() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;
        fs::write(sandbox.path().join("stdin.fixture"), b"regular input")?;
        let streams = StandardStreams {
            stdin_file: Some("stdin.fixture".to_string()),
            ..StandardStreams::default()
        };
        let result = sandbox.execute_command_with_streams(
            "cat",
            &[],
            Some("ignored pipe input"),
            true,
            None,
            &streams,
        )?;

        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "regular input");
        assert_eq!(result.stderr, "");
        Ok(())
    }

    #[test]
    fn test_execute_command_with_regular_file_stdin_offset() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;
        fs::write(sandbox.path().join("stdin.fixture"), b"skip:read this")?;
        let streams = StandardStreams {
            stdin_file: Some("stdin.fixture".to_string()),
            stdin_offset: Some(5),
            ..StandardStreams::default()
        };
        let result = sandbox.execute_command_with_streams(
            "cat",
            &[],
            Some("ignored pipe input"),
            true,
            None,
            &streams,
        )?;

        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "read this");
        assert_eq!(result.stderr, "");
        Ok(())
    }

    #[test]
    fn test_execute_command_with_stdout_full_reports_write_error() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;
        let streams = StandardStreams {
            stdout: OutputStream::Full,
            ..StandardStreams::default()
        };
        let result = sandbox.execute_command_with_streams(
            "/usr/bin/printf",
            &["output".to_string()],
            None,
            true,
            None,
            &streams,
        )?;

        assert_eq!(result.exit_code, 1);
        assert!(result.stdout.is_empty());
        assert!(result.stderr.contains("write error"));
        Ok(())
    }

    #[test]
    fn test_execute_command_with_closed_stdout_closes_fd_before_exec() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;
        let streams = StandardStreams {
            stdout: OutputStream::Closed,
            ..StandardStreams::default()
        };
        let result = sandbox.execute_command_with_streams(
            "sh",
            &["-c".to_string(), "test ! -e /proc/self/fd/1".to_string()],
            None,
            true,
            None,
            &streams,
        )?;

        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.is_empty());
        assert!(result.stderr.is_empty());
        Ok(())
    }

    #[test]
    fn test_execute_command_with_closed_stdout_uses_default_sigpipe() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;
        let streams = StandardStreams {
            stdout: OutputStream::ClosedPipe,
            ..StandardStreams::default()
        };
        let result = sandbox.execute_command_with_streams(
            "seq",
            &["1".to_string(), "100000".to_string()],
            None,
            true,
            None,
            &streams,
        )?;

        assert_eq!(result.exit_code, 141);
        assert!(result.stdout.is_empty());
        assert!(result.stderr.is_empty());
        Ok(())
    }

    #[test]
    fn test_execute_command_with_closed_stdout_can_ignore_sigpipe() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;
        let streams = StandardStreams {
            stdout: OutputStream::ClosedPipe,
            sigpipe: SignalDisposition::Ignore,
            ..StandardStreams::default()
        };
        let result = sandbox.execute_command_with_streams(
            "seq",
            &["1".to_string(), "100000".to_string()],
            None,
            true,
            None,
            &streams,
        )?;

        assert_eq!(result.exit_code, 1);
        assert!(result.stdout.is_empty());
        assert!(result.stderr.contains("write error: Broken pipe"));
        Ok(())
    }

    #[test]
    fn test_execute_command_with_nonblocking_full_stdout_reports_eagain() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;
        let streams = StandardStreams {
            stdout: OutputStream::NonblockingFullPipe,
            ..StandardStreams::default()
        };
        let result = sandbox.execute_command_with_streams(
            "/usr/bin/yes",
            &["x".to_string()],
            None,
            true,
            Some(1),
            &streams,
        )?;

        assert_eq!(result.exit_code, 1);
        assert!(result.stdout.is_empty());
        assert_eq!(
            result.stderr,
            "/usr/bin/yes: standard output: Resource temporarily unavailable\n"
        );
        Ok(())
    }

    #[test]
    fn test_execute_command_with_bash_keeps_bash_as_parent() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;
        let streams = StandardStreams {
            use_bash: true,
            ..StandardStreams::default()
        };
        let result = sandbox.execute_command_with_streams(
            "sh",
            &["-c".to_string(), "cat /proc/$PPID/comm".to_string()],
            None,
            true,
            None,
            &streams,
        )?;

        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "bash\n");
        assert!(result.stderr.is_empty());
        Ok(())
    }

    #[test]
    fn test_execute_command_can_attach_only_stderr_to_tty() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;
        let streams = StandardStreams {
            stderr: OutputStream::Tty,
            ..StandardStreams::default()
        };
        let result = sandbox.execute_command_with_streams(
            "sh",
            &[
                "-c".to_string(),
                "test -t 2 && printf 'tty\\n' >&2".to_string(),
            ],
            None,
            true,
            None,
            &streams,
        )?;

        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.is_empty());
        assert_eq!(result.stderr, "tty\n");
        Ok(())
    }

    #[test]
    fn test_execute_command_drains_large_stderr_pty_while_child_runs() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;
        let streams = StandardStreams {
            stderr: OutputStream::Tty,
            ..StandardStreams::default()
        };
        let result = sandbox.execute_command_with_streams(
            "sh",
            &[
                "-c".to_string(),
                "/usr/bin/yes x | /usr/bin/head -c 131072 >&2".to_string(),
            ],
            None,
            true,
            Some(2),
            &streams,
        )?;

        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.is_empty());
        assert_eq!(result.stderr.len(), 131_072);
        Ok(())
    }

    #[test]
    fn test_tty_eof_is_only_sent_for_non_empty_stdin() {
        assert!(!tty_stdin_needs_eof(None));
        assert!(!tty_stdin_needs_eof(Some(b"")));
        assert!(tty_stdin_needs_eof(Some(b"data")));
    }

    #[test]
    fn test_execute_command_not_found() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;
        let result = sandbox.execute_command("nonexistent_command", &[], None, true, None)?;
        assert_eq!(result.exit_code, 127);
        assert!(result.stderr.contains("Failed to execute command"));
        Ok(())
    }

    #[test]
    fn test_execute_shell_command() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;
        let result = sandbox.execute_shell_command("echo 'hello world'")?;
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout.trim(), "hello world");
        assert_eq!(result.stderr, "");
        Ok(())
    }

    #[test]
    fn test_builtin_cd() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;

        // 创建测试目录
        fs::create_dir_all(sandbox.path().join("test_dir"))?;

        // 测试切换到存在的目录
        let result = sandbox.builtin_cd("cd test_dir")?;
        assert_eq!(result.exit_code, 0);
        assert_eq!(sandbox.get_current_dir(), &sandbox.path().join("test_dir"));

        // 测试切换到不存在的目录
        let result = sandbox.builtin_cd("cd nonexistent_dir")?;
        assert_eq!(result.exit_code, 1);
        assert!(result.stderr.contains("No such file or directory"));
        Ok(())
    }

    #[test]
    fn test_builtin_export() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;

        // 测试设置环境变量
        let result = sandbox.builtin_export("export TEST_VAR=test_value")?;
        assert_eq!(result.exit_code, 0);
        assert_eq!(sandbox.get_env("TEST_VAR"), Some("test_value"));
        Ok(())
    }

    #[test]
    fn test_builtin_umask() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;

        // 测试设置 umask
        let result = sandbox.builtin_umask("umask 022")?;
        assert_eq!(result.exit_code, 0);

        // 测试获取 umask
        let result = sandbox.builtin_umask("umask")?;
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout.trim(), "022");

        // 测试无效的 umask
        let result = sandbox.builtin_umask("umask invalid")?;
        assert_eq!(result.exit_code, 1);
        assert!(result.stderr.contains("invalid mode"));
        Ok(())
    }

    #[test]
    fn test_signal_handler() -> Result<()> {
        let handler = SignalHandler::new()?;
        assert!(!handler.should_terminate());

        // 注意：我们不能真正发送信号，但我们可以测试基本结构
        let terminate = Arc::clone(&handler.terminate);
        terminate.store(true, Ordering::SeqCst);

        assert!(handler.should_terminate());
        Ok(())
    }

    #[test]
    fn test_resource_limiter() -> Result<()> {
        let mut limiter = ResourceLimiter::new();

        // 添加一些限制
        limiter.add_limit(Resource::RLIMIT_NOFILE, 1000, 1000);
        limiter.add_limit(Resource::RLIMIT_CPU, 10, 10);

        // 验证限制已添加（通过检查内部结构）
        assert_eq!(limiter.limits.len(), 2);
        assert_eq!(
            limiter.limits.get(&Resource::RLIMIT_NOFILE),
            Some(&(1000, 1000))
        );
        assert_eq!(limiter.limits.get(&Resource::RLIMIT_CPU), Some(&(10, 10)));

        // 注意：我们不能真正应用限制，因为它可能会限制测试进程
        // limiter.apply_limits()?;

        Ok(())
    }

    #[test]
    fn test_isolated_sandbox_id_generation() -> Result<()> {
        // 创建多个沙箱并验证它们的ID不同
        let sandbox1 = IsolatedSandbox::new(false)?;
        let sandbox2 = IsolatedSandbox::new(false)?;

        assert_ne!(sandbox1.id, sandbox2.id);
        Ok(())
    }

    #[test]
    fn test_isolated_sandbox_current_dir() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;

        // 创建测试目录
        let test_dir = "test_dir";
        let test_dir_path = sandbox.path().join(test_dir);
        fs::create_dir_all(&test_dir_path)?;

        // 执行cd命令
        sandbox.execute_shell_command(&format!("cd {test_dir}"))?;

        // 验证当前目录已更改 - 检查路径的最后一部分
        let current_dir_name = sandbox
            .get_current_dir()
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();

        assert_eq!(current_dir_name, test_dir);

        Ok(())
    }

    #[test]
    fn test_isolated_sandbox_environment_variables() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;

        // 添加环境变量
        sandbox.add_env("TEST_VAR", "test_value");

        // 验证环境变量已添加
        assert_eq!(sandbox.get_env("TEST_VAR"), Some("test_value"));

        // 执行export命令
        sandbox.execute_shell_command("export TEST_VAR2=another_value")?;

        // 验证通过命令添加的环境变量
        assert_eq!(sandbox.get_env("TEST_VAR2"), Some("another_value"));

        // 验证在命令中使用环境变量
        let result = sandbox.execute_shell_command("echo $TEST_VAR")?;
        assert_eq!(result.stdout.trim(), "test_value");

        Ok(())
    }

    #[test]
    fn test_create_test_files() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;

        // 创建测试用例
        let mut test_case = TestCase {
            tstdin: "".to_string(),
            byte_mode: false,
            tty: false,
            compare_use_bash: false,
            command: "test".to_string(),
            description: "Test with files".to_string(),
            args: vec![],
            expectation: TestExpectation {
                execution: CommandExecution {
                    exit_code: Some(0),
                    stdout: Some("".to_string()),
                    stderr: Some("".to_string()),
                },
                verifications: vec![],
                use_patterns: false,
                env_changes: HashMap::new(),
                file_changes: vec![],
                ignore_fields: IgnoreFields::default(),
            },
            setup_commands: vec![],
            cleanup_commands: vec![],
            requires_root: false,
            timeout: None,
            tags: vec![],
            environment: TestEnvironment::default(),
        };

        // 添加测试文件
        test_case.environment.files.push(TestFile {
            path: "test_file.txt".to_string(),
            content: Some("Test content".to_string()),
            permissions: Some("644".to_string()),
            owner: None,
            group: None,
            file_type: FileType::Regular,
            symlink_target: None,
            size: None,
            timestamp: None,
        });

        // 添加测试目录
        test_case.environment.files.push(TestFile {
            path: "test_dir".to_string(),
            content: None,
            permissions: Some("755".to_string()),
            owner: None,
            group: None,
            file_type: FileType::Directory,
            symlink_target: None,
            size: None,
            timestamp: None,
        });

        // 添加测试符号链接
        test_case.environment.files.push(TestFile {
            path: "test_link".to_string(),
            content: None,
            permissions: None,
            owner: None,
            group: None,
            file_type: FileType::Symlink,
            symlink_target: Some("test_file.txt".to_string()),
            size: None,
            timestamp: None,
        });

        // 设置沙箱环境
        sandbox.setup(&test_case)?;

        // 验证文件是否创建
        assert!(sandbox.path().join("test_file.txt").exists());
        assert!(sandbox.path().join("test_dir").exists());
        assert!(sandbox.path().join("test_dir").is_dir());
        assert!(sandbox.path().join("test_link").exists());

        // 验证文件内容
        let content = fs::read_to_string(sandbox.path().join("test_file.txt"))?;
        assert_eq!(content, "Test content");

        // 验证符号链接
        assert!(
            fs::symlink_metadata(sandbox.path().join("test_link"))?
                .file_type()
                .is_symlink()
        );

        Ok(())
    }

    #[test]
    fn test_command_execution_with_env_vars() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;

        // 设置环境变量
        sandbox.add_env("TEST_VAR", "test_value");

        // 执行使用环境变量的命令
        let result = sandbox.execute_command(
            "sh",
            &["-c".to_string(), "echo $TEST_VAR".to_string()],
            None,
            true,
            None,
        )?;

        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout.trim(), "test_value");

        Ok(())
    }

    #[test]
    fn test_shell_environment_builtins_persist_quoted_values_and_unsets() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;

        sandbox.execute_shell_command("export LC_ALL='C.UTF-8'")?;
        assert_eq!(sandbox.get_env("LC_ALL"), Some("C.UTF-8"));

        sandbox.add_env("POSIXLY_CORRECT", "1");
        sandbox.execute_shell_command("unset POSIXLY_CORRECT")?;
        assert_eq!(sandbox.get_env("POSIXLY_CORRECT"), None);

        Ok(())
    }

    #[test]
    fn test_command_execution_with_working_dir() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;

        // 创建测试目录
        let test_dir = sandbox.path().join("test_dir");
        fs::create_dir_all(&test_dir)?;

        // 创建测试文件
        let test_file = test_dir.join("test_file.txt");
        fs::write(&test_file, "Test content")?;

        // 更改当前工作目录
        sandbox.execute_shell_command("cd test_dir")?;

        // 执行依赖工作目录的命令
        let result = sandbox.execute_command("ls", &[], None, true, None)?;

        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("test_file.txt"));

        Ok(())
    }

    #[test]
    fn test_execute_command_with_timeout() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;

        // 执行一个会运行很长时间的命令，但设置超时
        let result = sandbox.execute_command("sleep", &["10".to_string()], None, true, Some(1))?;

        // 命令应该被中断，不会运行10秒
        assert_ne!(result.exit_code, 0);

        Ok(())
    }

    #[test]
    fn test_command_exit_code() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;

        // 执行会成功的命令
        let result = sandbox.execute_command("true", &[], None, true, None)?;

        assert_eq!(result.exit_code, 0);

        // 执行会失败的命令
        let result = sandbox.execute_command("false", &[], None, true, None)?;

        assert_eq!(result.exit_code, 1);

        Ok(())
    }

    #[test]
    fn test_sandbox_cleanup() -> Result<()> {
        let sandbox = IsolatedSandbox::new(false)?;

        // 创建一个文件
        let test_file = sandbox.path().join("test_file.txt");
        fs::write(&test_file, "Test content")?;

        // 验证文件是否创建
        assert!(test_file.exists());

        // 清理沙箱
        sandbox.cleanup()?;

        // 注意：cleanup()不会删除文件，它只会更改当前目录
        // 要测试文件仍然存在
        assert!(test_file.exists());

        Ok(())
    }

    #[test]
    fn test_sandbox_with_test_environment() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;

        // 创建测试用例
        let mut test_case = TestCase {
            tstdin: "".to_string(),
            byte_mode: false,
            tty: false,
            compare_use_bash: false,
            command: "test".to_string(),
            description: "Test with environment".to_string(),
            args: vec![],
            expectation: TestExpectation {
                execution: CommandExecution {
                    exit_code: Some(0),
                    stdout: Some("".to_string()),
                    stderr: Some("".to_string()),
                },
                verifications: vec![],
                use_patterns: false,
                env_changes: HashMap::new(),
                file_changes: vec![],
                ignore_fields: IgnoreFields::default(),
            },
            setup_commands: vec![],
            cleanup_commands: vec![],
            requires_root: false,
            timeout: None,
            tags: vec![],
            environment: TestEnvironment::default(),
        };

        // 设置环境变量
        test_case
            .environment
            .env_vars
            .insert("TEST_ENV_VAR".to_string(), "test_value".to_string());
        test_case
            .environment
            .env_bytes
            .insert("TEST_RAW_ENV".to_string(), "7261772dff".to_string());
        test_case.environment.clear_env = true;

        // 设置工作目录
        let work_dir = "work_dir";
        test_case.environment.working_dir = Some(work_dir.to_string());

        // 添加文件
        test_case.environment.files.push(TestFile {
            path: format!("{work_dir}/test_file.txt"),
            content: Some("Test content".to_string()),
            permissions: Some("644".to_string()),
            owner: None,
            group: None,
            file_type: FileType::Regular,
            symlink_target: None,
            size: None,
            timestamp: None,
        });

        // 设置沙箱环境
        sandbox.setup(&test_case)?;

        assert_eq!(sandbox.get_env("TEST_ENV_VAR"), Some("test_value"));

        // 验证工作目录
        assert_eq!(
            sandbox
                .get_current_dir()
                .file_name()
                .unwrap()
                .to_str()
                .unwrap(),
            work_dir
        );

        // 验证环境变量
        assert_eq!(sandbox.get_env("TEST_ENV_VAR"), Some("test_value"));
        assert_eq!(sandbox.get_env("PATH"), None);

        let raw_environment = sandbox.execute_command_bytes(
            "sh",
            &[
                OsString::from("-c"),
                OsString::from("printf %s \"$TEST_RAW_ENV\""),
            ],
            None,
            false,
            None,
            true,
        )?;
        assert_eq!(raw_environment.exit_code, 0);
        assert_eq!(raw_environment.stdout, "7261772dff");

        let printed_environment =
            sandbox.execute_command_bytes("/usr/bin/env", &[], None, false, None, true)?;
        assert_eq!(printed_environment.exit_code, 0);
        assert!(
            printed_environment
                .stdout
                .contains("544553545f454e565f5641523d746573745f76616c75650a")
        );
        assert!(
            printed_environment
                .stdout
                .contains("544553545f5241575f454e563d7261772dff0a")
        );
        assert!(!printed_environment.stdout.contains("504154483d"));

        // 验证文件是否创建
        assert!(sandbox.path().join(work_dir).join("test_file.txt").exists());

        Ok(())
    }

    #[test]
    fn test_isolated_execution() -> Result<()> {
        let sandbox = IsolatedSandbox::new(false)?;

        // 定义在隔离环境中执行的函数
        let result = sandbox.execute_isolated(|| {
            // 执行一些操作
            let value = 42;
            Ok(value)
        })?;

        assert_eq!(result, 42);

        Ok(())
    }

    #[test]
    fn test_sandbox_get_current_env() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;

        // 添加环境变量
        sandbox.add_env("TEST_VAR1", "value1");
        sandbox.add_env("TEST_VAR2", "value2");

        // 获取所有环境变量
        let env = sandbox.get_current_env();

        // 验证我们添加的环境变量存在
        assert_eq!(env.get("TEST_VAR1"), Some(&"value1".to_string()));
        assert_eq!(env.get("TEST_VAR2"), Some(&"value2".to_string()));

        Ok(())
    }

    #[test]
    fn test_sandbox_update_status() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;

        // 创建CommandResult
        let result = CommandResult {
            stdout: "output".to_string(),
            stderr: "error".to_string(),
            exit_code: 42,
        };

        // 更新状态
        sandbox.update_status(&result);

        // 验证退出码已更新
        assert_eq!(sandbox.exit_code, 42);

        Ok(())
    }

    #[test]
    fn byte_mode_captures_stderr_as_hex() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;
        let result = sandbox.execute_command_bytes(
            "/bin/sh",
            &[
                OsString::from("-c"),
                OsString::from("printf diagnostic >&2"),
            ],
            None,
            false,
            None,
            true,
        )?;

        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "");
        assert_eq!(result.stderr, "646961676e6f73746963");
        Ok(())
    }
}
