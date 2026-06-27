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
use crate::test_case::{FileType, TestCase, TestFile};
use crate::{Result, TestError};
use nix::sys::resource::{self, Resource};
use nix::sys::signal::{self};
use rand::Rng;
use std::collections::HashMap;
use std::fs::{self, File, Permissions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH, Duration};
use tempfile::TempDir;


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
                TestError::ExecutionError(format!("Failed to register signal handler: {}", e))
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
                TestError::ExecutionError(format!("Failed to set resource limit: {}", e))
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
    /// 当前工作目录
    current_dir: PathBuf,
    /// 当前 umask
    umask: u32,
    /// 上一个命令的退出码
    exit_code: i32,
    /// 是否启用调试输出
    debug: bool,
}

impl IsolatedSandbox {
    /// 创建新的隔离沙箱
    pub fn new(debug: bool) -> Result<Self> {
        let temp_dir = TempDir::new()
            .map_err(|e| TestError::ExecutionError(format!("Failed to create sandbox: {}", e)))?;
        let temp_path = temp_dir.path().to_path_buf();

        // 生成唯一ID：时间戳 + 随机数
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let random_num = rand::thread_rng().gen_range(0..1000000);
        let id = format!("{:x}-{:x}", timestamp, random_num);

        Ok(Self {
            id,
            temp_dir: Some(temp_dir),
            resource_limiter: Some(ResourceLimiter::new()),
            current_env: std::env::vars().collect(),
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

        // 创建测试所需的文件和目录
        for file in &test_case.environment.files {
            self.debug_fmt(format_args!("Creating test file: {:?}", file));
            self.debug_fmt(format_args!("File type: {:?}", file.file_type));
            self.create_test_file(file)?;
        }

        // 设置工作目录
        if let Some(ref working_dir) = test_case.environment.working_dir {
            let work_dir = self.path().join(working_dir);
            self.debug_fmt(format_args!(
                "Setting specified working directory: {:?}",
                work_dir
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
                    debug_msgs.push(format!("Setting CPU time limit: {}", cpu_time));
                }
                if let Some(file_size) = limits.file_size {
                    limiter.add_limit(Resource::RLIMIT_FSIZE, file_size, file_size);
                    debug_msgs.push(format!("Setting file size limit: {}", file_size));
                }
                if let Some(memory_size) = limits.memory_size {
                    limiter.add_limit(Resource::RLIMIT_AS, memory_size, memory_size);
                    debug_msgs.push(format!("Setting memory size limit: {}", memory_size));
                }
                if let Some(open_files) = limits.open_files {
                    limiter.add_limit(Resource::RLIMIT_NOFILE, open_files, open_files);
                    debug_msgs.push(format!("Setting open files limit: {}", open_files));
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
        self.debug_fmt(format_args!("Creating file: {:?}", path));
        self.debug_fmt(format_args!("File type: {:?}", file.file_type));

        match file.file_type {
            FileType::Directory => {
                self.debug_fmt(format_args!("Creating directory: {:?}", path));
                fs::create_dir_all(&path)?;
                self.debug_fmt(format_args!("Directory created successfully"));
            }
            FileType::Regular => {
                if let Some(parent) = path.parent() {
                    self.debug_fmt(format_args!("Creating parent directory: {:?}", parent));
                    fs::create_dir_all(parent)?;
                }
                self.debug_fmt(format_args!("Creating file: {:?}", path));
                let mut file_handle = File::create(&path)?;
                if let Some(ref content) = file.content {
                    self.debug_fmt(format_args!(
                        "Writing file content, length: {}",
                        content.len()
                    ));
                    file_handle.write_all(content.as_bytes())?;
                }
                if let Some(ref perms) = file.permissions {
                    self.debug_fmt(format_args!("Setting file permissions: {}", perms));
                    let mode = u32::from_str_radix(perms, 8).map_err(|e| {
                        TestError::ExecutionError(format!("Invalid permissions: {}", e))
                    })?;
                    fs::set_permissions(&path, Permissions::from_mode(mode))?;
                }
                self.debug_fmt(format_args!("File created successfully"));
            }
            FileType::Symlink => {
                if let Some(ref target) = file.symlink_target {
                    self.debug_fmt(format_args!("Creating symlink: {:?} -> {:?}", path, target));
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
                stderr: format!("cd: {}: No such file or directory\n", new_dir),
                exit_code: 1,
            })
        }
    }

    /// 处理export命令
    fn builtin_export(&mut self, command: &str) -> Result<CommandResult> {
        let args: Vec<&str> = command.split_whitespace().skip(1).collect();
        for arg in args {
            if let Some((key, value)) = arg.split_once('=') {
                self.current_env.insert(key.to_string(), value.to_string());
            }
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
                    stderr: format!("umask: invalid mode: {}\n", mode),
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

    /// 执行命令
    pub fn execute_command(
        &mut self,
        cmd: &str,
        args: &[String],
        stdin_content: Option<&str>,
        is_record_result: bool,
        timeout: Option<u64>,
    ) -> Result<CommandResult> {

        self.debug_fmt(format_args!("Executing command: {} {:?}", cmd, args));
        self.debug_fmt(format_args!(
            "Current working directory: {:?}",
            self.current_dir
        ));

        let mut command = Command::new(cmd);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .args(args)
            .current_dir(&self.current_dir)
            .envs(&self.current_env);

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                self.debug_fmt(format_args!("Command execution failed: {}", e));
                self.debug_fmt(format_args!("Error type: {:?}", e.kind()));
                return Ok(CommandResult {
                    stdout: String::new(),
                    stderr: format!("Failed to execute command: {}", e),
                    exit_code: 127, // Common error code for command not found
                });
            }
        };

        // 启动命令
        if let Some(content) = stdin_content {
            if !content.is_empty() {
                if let Some(stdin) = child.stdin.as_mut() {
                    if let Err(e) = stdin.write_all(content.as_bytes()) {
                        self.debug_fmt(format_args!("Failed to write to stdin: {}", e));
                        return Ok(CommandResult {
                            stdout: String::new(),
                            stderr: format!("Failed to write to stdin: {}", e),
                            exit_code: 1,
                        });
                    }
                }
            }
        }

        let output;
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
                        return Ok(CommandResult {
                            stdout: String::new(),
                            stderr: format!("Failed to wait for command: {}", e),
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
                    self.debug_fmt(format_args!("Failed to wait for command: {}", e));
                    return Ok(CommandResult {
                        stdout: String::new(),
                        stderr: format!("Failed to wait for command: {}", e),
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
                    self.debug_fmt(format_args!("Failed to wait for command: {}", e));
                    return Ok(CommandResult {
                        stdout: String::new(),
                        stderr: format!("Failed to wait for command: {}", e),
                        exit_code: 1,
                    });
                }
            };
        }

        let result = CommandResult::from(output);

        self.debug_fmt(format_args!("Command execution results:"));
        self.debug_fmt(format_args!("exit_code: {}", result.exit_code));
        self.debug_fmt(format_args!("stdout: {}", result.stdout));
        self.debug_fmt(format_args!("stderr: {}", result.stderr));

        // Check if stdout contains null bytes
        if result.stdout.contains('\0') {
            self.debug("Warning: stdout contains null bytes");
            if self.debug {
                println!("DEBUG: stdout hex representation:");
                for (i, byte) in result.stdout.as_bytes().iter().enumerate().take(100) {
                    print!("{:02x} ", byte);
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
            if result.stdout.contains('\0') {
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

    /// 执行外部命令
    fn execute_external_command(&mut self, command: &str) -> Result<CommandResult> {
        self.debug_fmt(format_args!("Executing external command: {}", command));
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
                self.debug_fmt(format_args!("Command execution failed: {}", e));
                self.debug_fmt(format_args!("Error type: {:?}", e.kind()));
                CommandResult {
                    stdout: String::new(),
                    stderr: format!("Failed to execute command: {}", e),
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
    use std::fs;
    use std::io::Write;

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

    /*#[test]
    fn test_resource_limits() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;

        // 测试设置 CPU 时间限制
        sandbox.set_cpu_time_limit(10)?;

        // 测试设置内存限制
        sandbox.set_memory_limit(1024 * 1024)?;

        // 测试设置打开文件数限制
        sandbox.set_open_files_limit(100)?;
        Ok(())
    }*/

    #[test]
    fn test_environment_variables() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;

        // 测试添加环境变量
        sandbox.add_env("TEST_VAR", "test_value");
        assert_eq!(sandbox.get_env("TEST_VAR"), Some("test_value"));

        // 测试获取不存在的环境变量
        assert_eq!(sandbox.get_env("NONEXISTENT_VAR"), None);
        Ok(())
    }

    #[test]
    fn test_command_execution_with_null_bytes() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;

        // 创建包含空字节的文件
        let test_file = sandbox.path().join("test.bin");
        let mut file = File::create(&test_file)?;
        file.write_all(&[0, 1, 2, 0, 3, 4])?;

        // 读取包含空字节的文件
        let result = sandbox.execute_command(
            "cat",
            &[test_file.to_str().unwrap().to_string()],
            None,
            true,
            None,
        )?;
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains('\0'));
        Ok(())
    }

    #[test]
    fn test_command_execution_with_large_output() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;

        // 生成大量输出的命令
        let result = sandbox.execute_command(
            "bash",
            &[
                "-c".to_string(),
                "for i in {1..1000}; do echo $i; done".to_string(),
            ],
            None,
            true,
            None,
        )?;

        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.lines().count() == 1000);
        Ok(())
    }

    #[test]
    fn test_command_execution_timeout() -> Result<()> {
        let mut sandbox = IsolatedSandbox::new(false)?;

        // 设置 CPU 时间限制为 1 秒
        sandbox.set_cpu_time_limit(1)?;

        // 执行一个耗时的命令
        let result = sandbox.execute_command(
            "bash",
            &["-c".to_string(), "while true; do : ; done".to_string()],
            None,
            true,
            None,
        )?;

        // 命令应该因为超时而被终止
        assert_ne!(result.exit_code, 0);
        Ok(())
    }
}
