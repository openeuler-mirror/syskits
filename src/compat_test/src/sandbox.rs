/*
 *  Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved
 *    syskits is licensed under Mulan PSL v2.
 *  You can use this software according to the terms and conditions of the Mulan PSL V2
 *  You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 *  THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 *  KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 *  NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 *  See the Mulan PSL v2 for more details.
 */

//! 沙箱环境实现模块
//! 提供命令执行的隔离环境，支持资源限制和环境变量管理

use crate::CommandResult;
use crate::test_case::{FileType, TestCase, TestFile};
use crate::{Result, TestError};
use nix::sys::resource::{self, Resource};
use nix::sys::signal::{self};
use std::collections::HashMap;
use std::fs::{self, File, Permissions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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
}

impl IsolatedSandbox {
    /// 创建新的隔离沙箱
    pub fn new() -> Result<Self> {
        let temp_dir = TempDir::new()
            .map_err(|e| TestError::ExecutionError(format!("Failed to create sandbox: {}", e)))?;
        let temp_path = temp_dir.path().to_path_buf();

        Ok(Self {
            temp_dir: Some(temp_dir),
            resource_limiter: Some(ResourceLimiter::new()),
            current_env: std::env::vars().collect(),
            current_dir: temp_path,
            umask: 0o022,
            exit_code: 0,
        })
    }

    /// 获取沙箱根路径
    pub fn path(&self) -> &Path {
        self.temp_dir.as_ref().unwrap().path()
    }

    /// 设置沙箱环境
    pub fn setup(&mut self, test_case: &TestCase) -> Result<()> {
        // 创建测试所需的文件和目录
        for file in &test_case.environment.files {
            self.create_test_file(file)?;
        }

        // 设置工作目录
        if let Some(ref working_dir) = test_case.environment.working_dir {
            let work_dir = self.path().join(working_dir);
            std::env::set_current_dir(&work_dir)?;
            self.current_dir = work_dir;
        } else {
            std::env::set_current_dir(self.path())?;
            self.current_dir = self.path().to_path_buf();
        }

        // 应用资源限制
        if let Some(ref limits) = test_case.environment.resource_limits {
            if let Some(ref mut limiter) = self.resource_limiter.as_mut() {
                if let Some(cpu_time) = limits.cpu_time {
                    limiter.add_limit(Resource::RLIMIT_CPU, cpu_time, cpu_time);
                }
                if let Some(file_size) = limits.file_size {
                    limiter.add_limit(Resource::RLIMIT_FSIZE, file_size, file_size);
                }
                if let Some(memory_size) = limits.memory_size {
                    limiter.add_limit(Resource::RLIMIT_AS, memory_size, memory_size);
                }
                if let Some(open_files) = limits.open_files {
                    limiter.add_limit(Resource::RLIMIT_NOFILE, open_files, open_files);
                }

                limiter.apply_limits()?;
            }
        }

        Ok(())
    }

    /// 创建测试文件
    fn create_test_file(&self, file: &TestFile) -> Result<()> {
        let path = self.path().join(&file.path);

        match file.file_type {
            FileType::Directory => {
                fs::create_dir_all(&path)?;
            }
            FileType::Regular => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut file_handle = File::create(&path)?;
                if let Some(ref content) = file.content {
                    file_handle.write_all(content.as_bytes())?;
                }
                if let Some(ref perms) = file.permissions {
                    let mode = u32::from_str_radix(perms, 8).map_err(|e| {
                        TestError::ExecutionError(format!("Invalid permissions: {}", e))
                    })?;
                    fs::set_permissions(&path, Permissions::from_mode(mode))?;
                }
            }
            FileType::Symlink => {
                if let Some(ref target) = file.symlink_target {
                    std::os::unix::fs::symlink(target, &path)?;
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

    /// 执行命令
    pub fn execute_command(&mut self, cmd: &str, args: &[String]) -> Result<CommandResult> {
        let mut command = std::process::Command::new(cmd);
        command
            .args(args)
            .current_dir(&self.current_dir)
            .envs(&self.current_env);

        let output = command
            .output()
            .map_err(|e| TestError::ExecutionError(format!("Failed to execute command: {}", e)))?;

        let result = CommandResult::from(output);
        self.update_status(&result);
        Ok(result)
    }

    /// 执行外部命令
    fn execute_external_command(&mut self, command: &str) -> Result<CommandResult> {
        let parts: Vec<String> = command.split_whitespace().map(String::from).collect();
        if let Some((cmd, args)) = parts.split_first() {
            let mut command = std::process::Command::new(cmd);
            command
                .args(args)
                .current_dir(&self.current_dir)
                .envs(&self.current_env);

            let output = command.output().map_err(|e| {
                TestError::ExecutionError(format!("Failed to execute command: {}", e))
            })?;

            let result = CommandResult::from(output);
            self.update_status(&result);
            Ok(result)
        } else {
            Err(TestError::ExecutionError("Empty command".to_string()))
        }
    }
}
