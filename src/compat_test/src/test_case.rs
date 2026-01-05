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

//! 测试用例管理器

use crate::CommandResult;
use crate::Result;
use crate::TestError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// 表示测试环境中要创建的文件或目录
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TestFile {
    /// 相对于测试目录的路径
    pub path: String,
    /// 文件内容（目录为None）
    pub content: Option<String>,
    /// 文件权限（八进制格式，如 "644"）
    pub permissions: Option<String>,
    /// 所有者（用户名）
    pub owner: Option<String>,
    /// 用户组（组名）
    pub group: Option<String>,
    /// 文件类型（普通文件、目录、符号链接等）
    pub file_type: FileType,
    /// 符号链接的目标路径
    pub symlink_target: Option<String>,
    /// 特殊情况下的文件大小（如稀疏文件）
    pub size: Option<u64>,
    /// 文件时间戳（从纪元开始的秒数）
    pub timestamp: Option<i64>,
}

/// 表示不同的文件类型
#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum FileType {
    Regular,     // 普通文件
    Directory,   // 目录
    Symlink,     // 符号链接
    CharDevice,  // 字符设备
    BlockDevice, // 块设备
    Fifo,        // 命名管道
    Socket,      // 套接字
}

/// 表示测试的环境设置
#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct TestEnvironment {
    /// 测试前要创建的文件和目录
    pub files: Vec<TestFile>,
    /// 要设置的环境变量
    pub env_vars: HashMap<String, String>,
    /// 当前工作目录（相对于测试目录）
    pub working_dir: Option<String>,
    /// 以指定用户身份运行测试（如果支持）
    pub run_as_user: Option<String>,
    /// 以指定用户组运行测试（如果支持）
    pub run_as_group: Option<String>,
    /// 测试的umask设置
    pub umask: Option<String>,
    /// 资源限制设置
    pub resource_limits: Option<ResourceLimits>,
}

/// Resource limits for the test environment
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ResourceLimits {
    /// Maximum file size (bytes)
    pub file_size: Option<u64>,
    /// Maximum CPU time (seconds)
    pub cpu_time: Option<u64>,
    /// Maximum memory size (bytes)
    pub memory_size: Option<u64>,
    /// Maximum number of open files
    pub open_files: Option<u64>,
}

/// 命令执行结果验证
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CommandExecution {
    /// 退出码
    pub exit_code: i32,
    /// 标准输出
    pub stdout: Option<String>,
    /// 标准错误
    pub stderr: Option<String>,
}

/// 命令功能验证
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FunctionalVerification {
    /// 验证命令
    pub command: String,
    /// 预期退出码
    pub expected_exit: i32,
    /// 预期标准输出
    pub expected_stdout: Option<String>,
    /// 预期标准错误
    pub expected_stderr: Option<String>,
}

/// 测试期望结果
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TestExpectation {
    /// 命令执行结果验证
    pub execution: CommandExecution,
    /// 功能验证列表
    #[serde(default)]
    pub verifications: Vec<FunctionalVerification>,
    /// 是否使用模式匹配
    #[serde(default = "default_use_patterns")]
    pub use_patterns: bool,
    /// 环境变量变化
    #[serde(default)]
    pub env_changes: HashMap<String, String>,
    /// 文件变化
    #[serde(default)]
    pub file_changes: Vec<FileChange>,
    /// 忽略的比较字段
    #[serde(default)]
    pub ignore_fields: IgnoreFields,
}

/// 可以在比较时忽略的字段
#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct IgnoreFields {
    /// 忽略退出码差异
    #[serde(default)]
    pub ignore_exit_code: bool,
    /// 忽略标准输出差异
    #[serde(default)]
    pub ignore_stdout: bool,
    /// 忽略标准错误差异
    #[serde(default)]
    pub ignore_stderr: bool,
    /// 忽略验证差异
    #[serde(default)]
    pub ignore_verifications: bool,
}

/// 表示预期的文件变化
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileChange {
    /// 要变化的文件路径
    pub path: String,
    /// 预期的文件内容
    pub content: Option<String>,
    /// 预期的文件权限
    pub permissions: Option<String>,
    /// 预期的文件所有者
    pub owner: Option<String>,
    /// 预期的文件用户组
    pub group: Option<String>,
    /// 文件是否应该存在
    pub should_exist: bool,
}

/// 测试用例集合
#[derive(Debug, Serialize, Deserialize)]
pub struct TestSuite {
    pub tests: Vec<TestCase>,
}

/// 单个测试用例
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TestCase {
    /// 命令名称
    pub command: String,
    /// 测试描述
    pub description: String,
    /// 命令参数
    pub args: Vec<String>,
    /// 期望结果
    pub expectation: TestExpectation,
    /// 环境准备命令
    #[serde(default)]
    pub setup_commands: Vec<String>,
    /// 环境清理命令
    #[serde(default)]
    pub cleanup_commands: Vec<String>,
    /// 是否需要root权限
    #[serde(default)]
    pub requires_root: bool,
    /// 超时时间（秒）
    #[serde(default = "default_timeout")]
    pub timeout: u64,
    /// 测试标签
    #[serde(default)]
    pub tags: Vec<String>,
    /// 沙箱环境配置
    #[serde(default)]
    pub environment: TestEnvironment,
}

fn default_timeout() -> u64 {
    30
}

fn default_use_patterns() -> bool {
    false
}

/// 测试用例管理器
pub struct TestCaseManager {
    /// 测试用例文件目录
    pub test_cases_dir: std::path::PathBuf,
}

impl TestCaseManager {
    /// 创建新的测试用例管理器
    pub fn new<P: AsRef<Path>>(test_cases_dir: P) -> Self {
        Self {
            test_cases_dir: test_cases_dir.as_ref().to_path_buf(),
        }
    }

    /// 加载指定命令的所有测试用例
    pub fn load_test_cases(&self, command: &str) -> Result<Vec<TestCase>> {
        let path = self.test_cases_dir.join(format!("{}.json", command));
        println!("Looking for test cases at: {:?}", path);

        if !path.exists() {
            println!("Test case file not found");
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(&path)?;
        println!(
            "Found test case file with content length: {}",
            content.len()
        );

        let test_suite: TestSuite = serde_json::from_str(&content)
            .map_err(|e| TestError::TestCaseError(format!("Failed to parse test cases: {}", e)))?;

        println!("Loaded {} test cases", test_suite.tests.len());
        Ok(test_suite.tests)
    }

    /// 保存指定命令的测试用例
    pub fn save_test_cases(&self, command: &str, test_cases: &[TestCase]) -> Result<()> {
        let path = self.test_cases_dir.join(format!("{}.json", command));
        let test_suite = TestSuite {
            tests: test_cases.to_vec(),
        };
        let content = serde_json::to_string_pretty(&test_suite).map_err(|e| {
            TestError::TestCaseError(format!("Failed to serialize test cases: {}", e))
        })?;

        fs::write(path, content)?;
        Ok(())
    }

    /// 获取所有可用的测试命令
    pub fn get_available_commands(&self) -> Result<Vec<String>> {
        let mut commands = Vec::new();
        for entry in fs::read_dir(&self.test_cases_dir)? {
            let entry = entry?;
            if let Some(file_name) = entry.path().file_stem() {
                if let Some(name) = file_name.to_str() {
                    commands.push(name.to_string());
                }
            }
        }
        Ok(commands)
    }
}

impl From<CommandExecution> for CommandResult {
    fn from(execution: CommandExecution) -> Self {
        CommandResult {
            stdout: execution.stdout.unwrap_or_default(),
            stderr: execution.stderr.unwrap_or_default(),
            exit_code: execution.exit_code,
        }
    }
}

