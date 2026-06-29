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

//! 测试用例管理器

use crate::CommandResult;
use crate::Result;
use crate::TestError;
use hex;
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
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
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
    pub exit_code: Option<i32>,
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
    pub expected_exit: Option<i32>,
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
    ///标准输入
    #[serde(default)]
    pub tstdin: String,
    /// 是否按十六进制字节模式解析本用例
    #[serde(default, rename = "byteMode", alias = "byte_mode")]
    pub byte_mode: bool,
    /// 是否使用伪终端执行
    #[serde(default, rename = "tty", alias = "tty_mode")]
    pub tty: bool,
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
    pub timeout: Option<u64>,
    /// 测试标签
    #[serde(default)]
    pub tags: Vec<String>,
    /// 沙箱环境配置
    #[serde(default)]
    pub environment: TestEnvironment,
}

fn default_timeout() -> Option<u64> {
    None
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
        let path = self.test_cases_dir.join(format!("{command}.json"));
        println!("Looking for test cases at: {path:?}");

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
            .map_err(|e| TestError::TestCaseError(format!("Failed to parse test cases: {e}")))?;

        println!("Loaded {} test cases", test_suite.tests.len());
        Ok(test_suite.tests)
    }

    /// 保存指定命令的测试用例
    pub fn save_test_cases(&self, command: &str, test_cases: &[TestCase]) -> Result<()> {
        let path = self.test_cases_dir.join(format!("{command}.json"));
        let test_suite = TestSuite {
            tests: test_cases.to_vec(),
        };
        let content = serde_json::to_string_pretty(&test_suite).map_err(|e| {
            TestError::TestCaseError(format!("Failed to serialize test cases: {e}"))
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

impl CommandExecution {
    pub fn to_command_result(&self, byte_mode: bool) -> Result<CommandResult> {
        let stdout = if byte_mode {
            normalize_hex_opt("stdout", self.stdout.as_deref())?
        } else {
            self.stdout.clone().unwrap_or_default()
        };
        let stderr = if byte_mode {
            normalize_hex_opt("stderr", self.stderr.as_deref())?
        } else {
            self.stderr.clone().unwrap_or_default()
        };
        Ok(CommandResult {
            stdout,
            stderr,
            exit_code: self.exit_code.unwrap_or(0),
        })
    }
}

impl From<&FunctionalVerification> for CommandResult {
    fn from(verification: &FunctionalVerification) -> Self {
        CommandResult {
            stdout: verification.expected_stdout.clone().unwrap_or_default(),
            stderr: verification.expected_stderr.clone().unwrap_or_default(),
            exit_code: verification.expected_exit.unwrap_or(0),
        }
    }
}

fn decode_hex_field(field: &str, value: &str) -> Result<Vec<u8>> {
    hex::decode(value).map_err(|e| TestError::TestCaseError(format!("Invalid hex in {field}: {e}")))
}

fn normalize_hex_field(field: &str, value: &str) -> Result<String> {
    let bytes = decode_hex_field(field, value)?;
    Ok(hex::encode(bytes))
}

fn normalize_hex_opt(field: &str, value: Option<&str>) -> Result<String> {
    match value {
        Some(v) => normalize_hex_field(field, v),
        None => Ok(String::new()),
    }
}

impl TestCase {
    pub fn args_bytes(&self) -> Result<Vec<Vec<u8>>> {
        if self.byte_mode {
            let mut decoded = Vec::with_capacity(self.args.len());
            for hex_str in &self.args {
                decoded.push(decode_hex_field("args", hex_str)?);
            }
            Ok(decoded)
        } else {
            Ok(self
                .args
                .iter()
                .map(|arg| arg.as_bytes().to_vec())
                .collect())
        }
    }

    pub fn tstdin_bytes(&self) -> Result<Vec<u8>> {
        if self.byte_mode {
            decode_hex_field("tstdin", &self.tstdin)
        } else {
            Ok(self.tstdin.as_bytes().to_vec())
        }
    }

    pub fn args_display(&self) -> Vec<String> {
        if self.byte_mode {
            return self
                .args
                .iter()
                .map(|hex_str| format!("0x{hex_str}"))
                .collect();
        }
        self.args.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_deserialize_test_case() {
        let json = r#"{
            "tstdin": "",
            "command": "echo",
            "description": "测试基本的 echo 命令",
            "args": ["Hello, World!"],
            "expectation": {
                "execution": {
                    "exit_code": 0,
                    "stdout": "Hello, World!\n",
                    "stderr": ""
                },
                "verifications": [],
                "use_patterns": false,
                "env_changes": {},
                "file_changes": [],
                "ignore_fields": {
                    "ignore_exit_code": false,
                    "ignore_stdout": false,
                    "ignore_stderr": false
                }
            },
            "setup_commands": [],
            "cleanup_commands": [],
            "requires_root": false,
            "timeout": 5,
            "tags": ["basic"],
            "environment": {
                "files": [],
                "env_vars": {},
                "working_dir": null,
                "run_as_user": null,
                "run_as_group": null,
                "umask": null,
                "resource_limits": null
            }
        }"#;

        let test_case: TestCase = serde_json::from_str(json).unwrap();
        assert_eq!(test_case.command, "echo");
        assert_eq!(test_case.args, vec!["Hello, World!"]);
        assert_eq!(test_case.expectation.execution.exit_code, Some(0));
        assert_eq!(
            test_case.expectation.execution.stdout,
            Some("Hello, World!\n".to_string())
        );
    }

    #[test]
    fn test_deserialize_test_suite() {
        let json = r#"{
            "tests": [
                {
                    "tstdin": "",
                    "command": "ls",
                    "description": "测试文件列表功能",
                    "args": ["-l", "test_dir"],
                    "setup_commands": [
                        "mkdir -p test_dir",
                        "touch test_dir/file1"
                    ],
                    "expectation": {
                        "execution": {
                            "exit_code": 0,
                            "stdout": "",
                            "stderr": ""
                        },
                        "verifications": [
                            {
                                "command": "test -d test_dir",
                                "expected_exit": 0,
                                "expected_stdout": "",
                                "expected_stderr": ""
                            }
                        ],
                        "use_patterns": false,
                        "env_changes": {},
                        "file_changes": [],
                        "ignore_fields": {
                            "ignore_exit_code": false,
                            "ignore_stdout": false,
                            "ignore_stderr": false
                        }
                    },
                    "cleanup_commands": [
                        "rm -rf test_dir"
                    ],
                    "requires_root": false,
                    "timeout": 5,
                    "tags": ["basic"],
                    "environment": {
                        "files": [],
                        "env_vars": {},
                        "working_dir": null,
                        "run_as_user": null,
                        "run_as_group": null,
                        "umask": null,
                        "resource_limits": null
                    }
                }
            ]
        }"#;

        let test_suite: TestSuite = serde_json::from_str(json).unwrap();
        assert_eq!(test_suite.tests.len(), 1);
        assert_eq!(test_suite.tests[0].command, "ls");
        assert_eq!(test_suite.tests[0].setup_commands.len(), 2);
        assert_eq!(test_suite.tests[0].cleanup_commands.len(), 1);
    }

    #[test]
    fn test_deserialize_null_fields() {
        let json = r#"{
            "tstdin": "",
            "command": "echo",
            "description": "测试带null字段的命令",
            "args": ["Hello, World!"],
            "expectation": {
                "execution": {
                    "exit_code": null,
                    "stdout": null,
                    "stderr": null
                },
                "verifications": [],
                "use_patterns": false,
                "env_changes": {},
                "file_changes": [],
                "ignore_fields": {
                    "ignore_exit_code": false,
                    "ignore_stdout": false,
                    "ignore_stderr": false
                }
            },
            "setup_commands": [],
            "cleanup_commands": [],
            "requires_root": false,
            "timeout": 5,
            "tags": ["basic"],
            "environment": {
                "files": [],
                "env_vars": {},
                "working_dir": null,
                "run_as_user": null,
                "run_as_group": null,
                "umask": null,
                "resource_limits": null
            }
        }"#;

        let test_case: TestCase = serde_json::from_str(json).unwrap();
        assert_eq!(test_case.command, "echo");
        assert_eq!(test_case.args, vec!["Hello, World!"]);
        assert_eq!(test_case.expectation.execution.exit_code, None);
        assert_eq!(test_case.expectation.execution.stdout, None);
        assert_eq!(test_case.expectation.execution.stderr, None);
    }

    #[test]
    fn test_deserialize_mixed_null_fields() {
        let json = r#"{
            "tstdin": "",
            "command": "echo",
            "description": "测试部分null字段的命令",
            "args": ["Hello, World!"],
            "expectation": {
                "execution": {
                    "exit_code": 0,
                    "stdout": null,
                    "stderr": ""
                },
                "verifications": [
                    {
                        "command": "echo test",
                        "expected_exit": 0,
                        "expected_stdout": null,
                        "expected_stderr": null
                    }
                ],
                "use_patterns": false,
                "env_changes": {},
                "file_changes": [],
                "ignore_fields": {
                    "ignore_exit_code": false,
                    "ignore_stdout": false,
                    "ignore_stderr": false
                }
            },
            "setup_commands": [],
            "cleanup_commands": [],
            "requires_root": false,
            "timeout": 5,
            "tags": ["basic"],
            "environment": {
                "files": [],
                "env_vars": {},
                "working_dir": null,
                "run_as_user": null,
                "run_as_group": null,
                "umask": null,
                "resource_limits": null
            }
        }"#;

        let test_case: TestCase = serde_json::from_str(json).unwrap();
        assert_eq!(test_case.command, "echo");
        assert_eq!(test_case.args, vec!["Hello, World!"]);
        assert_eq!(test_case.expectation.execution.exit_code, Some(0));
        assert_eq!(test_case.expectation.execution.stdout, None);
        assert_eq!(test_case.expectation.execution.stderr, Some("".to_string()));

        // 验证命令的null字段
        assert_eq!(test_case.expectation.verifications.len(), 1);
        assert_eq!(
            test_case.expectation.verifications[0].expected_exit,
            Some(0)
        );
        assert_eq!(test_case.expectation.verifications[0].expected_stdout, None);
        assert_eq!(test_case.expectation.verifications[0].expected_stderr, None);
    }

    #[test]
    fn test_command_execution_to_command_result() {
        // 测试正常值转换
        let execution = CommandExecution {
            exit_code: Some(0),
            stdout: Some("output".to_string()),
            stderr: Some("error".to_string()),
        };

        let result: CommandResult = execution.to_command_result(false).unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "output");
        assert_eq!(result.stderr, "error");

        // 测试null值转换
        let execution = CommandExecution {
            exit_code: None,
            stdout: None,
            stderr: None,
        };

        let result: CommandResult = execution.to_command_result(false).unwrap();
        assert_eq!(result.exit_code, 0); // 默认值为0
        assert_eq!(result.stdout, ""); // 默认值为空字符串
        assert_eq!(result.stderr, ""); // 默认值为空字符串

        // 测试混合值转换
        let execution = CommandExecution {
            exit_code: Some(1),
            stdout: None,
            stderr: Some("error".to_string()),
        };

        let result: CommandResult = execution.to_command_result(false).unwrap();
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.stdout, "");
        assert_eq!(result.stderr, "error");
    }

    #[test]
    fn test_test_case_manager_creation() {
        let temp_dir = TempDir::new().unwrap();
        let test_cases_dir = temp_dir.path().to_path_buf();

        let manager = TestCaseManager::new(&test_cases_dir);

        assert_eq!(manager.test_cases_dir, test_cases_dir);
    }

    #[test]
    fn test_test_case_manager_save_and_load() -> Result<()> {
        // 创建临时目录
        let temp_dir = TempDir::new().unwrap();
        let test_cases_dir = temp_dir.path().to_path_buf();

        // 创建测试用例管理器
        let manager = TestCaseManager::new(&test_cases_dir);

        // 创建测试用例
        let test_cases = vec![TestCase {
            tstdin: "".to_string(),
            byte_mode: false,
            tty: false,
            command: "echo".to_string(),
            description: "Test echo command".to_string(),
            args: vec!["Hello".to_string()],
            expectation: TestExpectation {
                execution: CommandExecution {
                    exit_code: Some(0),
                    stdout: Some("Hello\n".to_string()),
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
            tags: vec!["basic".to_string()],
            environment: TestEnvironment::default(),
        }];

        // 保存测试用例
        manager.save_test_cases("echo", &test_cases)?;

        // 验证文件是否创建
        let test_file = test_cases_dir.join("echo.json");
        assert!(test_file.exists());

        // 加载测试用例
        let loaded_cases = manager.load_test_cases("echo")?;

        // 验证加载的测试用例
        assert_eq!(loaded_cases.len(), 1);
        assert_eq!(loaded_cases[0].command, "echo");
        assert_eq!(loaded_cases[0].description, "Test echo command");
        assert_eq!(loaded_cases[0].args, vec!["Hello"]);

        Ok(())
    }

    #[test]
    fn test_test_case_manager_get_available_commands() -> Result<()> {
        // 创建临时目录
        let temp_dir = TempDir::new().unwrap();
        let test_cases_dir = temp_dir.path().to_path_buf();

        // 创建测试用例管理器
        let manager = TestCaseManager::new(&test_cases_dir);

        // 创建测试文件
        fs::write(test_cases_dir.join("cmd1.json"), "{\"tests\":[]}")?;
        fs::write(test_cases_dir.join("cmd2.json"), "{\"tests\":[]}")?;
        fs::write(test_cases_dir.join("cmd3.json"), "{\"tests\":[]}")?;

        // 获取可用命令
        let commands = manager.get_available_commands()?;

        // 验证命令列表
        assert_eq!(commands.len(), 3);
        assert!(commands.contains(&"cmd1".to_string()));
        assert!(commands.contains(&"cmd2".to_string()));
        assert!(commands.contains(&"cmd3".to_string()));

        Ok(())
    }

    #[test]
    fn test_test_case_manager_load_nonexistent_file() -> Result<()> {
        // 创建临时目录
        let temp_dir = TempDir::new().unwrap();
        let test_cases_dir = temp_dir.path().to_path_buf();

        // 创建测试用例管理器
        let manager = TestCaseManager::new(&test_cases_dir);

        // 加载不存在的测试用例文件
        let cases = manager.load_test_cases("nonexistent")?;

        // 验证返回空列表
        assert!(cases.is_empty());

        Ok(())
    }

    #[test]
    fn test_test_case_manager_invalid_json() -> Result<()> {
        // 创建临时目录
        let temp_dir = TempDir::new().unwrap();
        let test_cases_dir = temp_dir.path().to_path_buf();

        // 创建测试用例管理器
        let manager = TestCaseManager::new(&test_cases_dir);

        // 创建无效的测试文件
        fs::write(
            test_cases_dir.join("invalid.json"),
            "{ this is not valid json }",
        )?;

        // 尝试加载无效的测试用例文件
        let result = manager.load_test_cases("invalid");

        // 验证返回错误
        assert!(result.is_err());

        Ok(())
    }

    #[test]
    fn test_test_environment_default() {
        // 测试 TestEnvironment 默认值
        let env = TestEnvironment::default();

        assert!(env.files.is_empty());
        assert!(env.env_vars.is_empty());
        assert_eq!(env.working_dir, None);
        assert_eq!(env.run_as_user, None);
        assert_eq!(env.run_as_group, None);
        assert_eq!(env.umask, None);
        assert_eq!(env.resource_limits, None);
    }

    #[test]
    fn test_ignore_fields_default() {
        // 测试 IgnoreFields 默认值
        let ignore = IgnoreFields::default();

        assert!(!ignore.ignore_exit_code);
        assert!(!ignore.ignore_stdout);
        assert!(!ignore.ignore_stderr);
        assert!(!ignore.ignore_verifications);
    }

    #[test]
    fn test_file_type_serialization() {
        // 测试 FileType 序列化和反序列化
        let types = vec![
            FileType::Regular,
            FileType::Directory,
            FileType::Symlink,
            FileType::CharDevice,
            FileType::BlockDevice,
            FileType::Fifo,
            FileType::Socket,
        ];

        for file_type in types {
            let serialized = serde_json::to_string(&file_type).unwrap();
            let deserialized: FileType = serde_json::from_str(&serialized).unwrap();

            // 检查反序列化结果与原始值匹配
            match (file_type, deserialized) {
                (FileType::Regular, FileType::Regular) => (),
                (FileType::Directory, FileType::Directory) => (),
                (FileType::Symlink, FileType::Symlink) => (),
                (FileType::CharDevice, FileType::CharDevice) => (),
                (FileType::BlockDevice, FileType::BlockDevice) => (),
                (FileType::Fifo, FileType::Fifo) => (),
                (FileType::Socket, FileType::Socket) => (),
                _ => panic!("FileType serialization/deserialization failed"),
            }
        }
    }

    #[test]
    fn test_test_file_serialization() {
        // 创建 TestFile
        let test_file = TestFile {
            path: "test_file.txt".to_string(),
            content: Some("Test content".to_string()),
            permissions: Some("644".to_string()),
            owner: Some("user".to_string()),
            group: Some("group".to_string()),
            file_type: FileType::Regular,
            symlink_target: None,
            size: Some(100),
            timestamp: Some(1234567890),
        };

        // 序列化
        let serialized = serde_json::to_string(&test_file).unwrap();

        // 反序列化
        let deserialized: TestFile = serde_json::from_str(&serialized).unwrap();

        // 验证字段
        assert_eq!(deserialized.path, test_file.path);
        assert_eq!(deserialized.content, test_file.content);
        assert_eq!(deserialized.permissions, test_file.permissions);
        assert_eq!(deserialized.owner, test_file.owner);
        assert_eq!(deserialized.group, test_file.group);
        assert_eq!(deserialized.size, test_file.size);
        assert_eq!(deserialized.timestamp, test_file.timestamp);

        // 验证枚举类型
        match (test_file.file_type, deserialized.file_type) {
            (FileType::Regular, FileType::Regular) => (),
            _ => panic!("FileType mismatch after serialization/deserialization"),
        }
    }

    #[test]
    fn test_file_change_serialization() {
        // 创建 FileChange
        let file_change = FileChange {
            path: "test_file.txt".to_string(),
            content: Some("New content".to_string()),
            permissions: Some("755".to_string()),
            owner: Some("new_user".to_string()),
            group: Some("new_group".to_string()),
            should_exist: true,
        };

        // 序列化
        let serialized = serde_json::to_string(&file_change).unwrap();

        // 反序列化
        let deserialized: FileChange = serde_json::from_str(&serialized).unwrap();

        // 验证字段
        assert_eq!(deserialized.path, file_change.path);
        assert_eq!(deserialized.content, file_change.content);
        assert_eq!(deserialized.permissions, file_change.permissions);
        assert_eq!(deserialized.owner, file_change.owner);
        assert_eq!(deserialized.group, file_change.group);
        assert_eq!(deserialized.should_exist, file_change.should_exist);
    }

    #[test]
    fn test_resource_limits_serialization() {
        // 创建 ResourceLimits
        let limits = ResourceLimits {
            file_size: Some(1024),
            cpu_time: Some(10),
            memory_size: Some(1_000_000),
            open_files: Some(100),
        };

        // 序列化
        let serialized = serde_json::to_string(&limits).unwrap();

        // 反序列化
        let deserialized: ResourceLimits = serde_json::from_str(&serialized).unwrap();

        // 验证字段
        assert_eq!(deserialized.file_size, limits.file_size);
        assert_eq!(deserialized.cpu_time, limits.cpu_time);
        assert_eq!(deserialized.memory_size, limits.memory_size);
        assert_eq!(deserialized.open_files, limits.open_files);
    }

    #[test]
    fn test_functional_verification_conversion() {
        // 创建 FunctionalVerification
        let verification = FunctionalVerification {
            command: "test command".to_string(),
            expected_exit: Some(0),
            expected_stdout: Some("expected output".to_string()),
            expected_stderr: Some("expected error".to_string()),
        };

        // 转换为 CommandResult
        let result: CommandResult = (&verification).into();

        // 验证转换结果
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "expected output");
        assert_eq!(result.stderr, "expected error");
    }

    #[test]
    fn test_functional_verification_null_fields() {
        // 创建具有null字段的 FunctionalVerification
        let verification = FunctionalVerification {
            command: "test command".to_string(),
            expected_exit: None,
            expected_stdout: None,
            expected_stderr: None,
        };

        // 转换为 CommandResult
        let result: CommandResult = (&verification).into();

        // 验证转换结果（应使用默认值）
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "");
        assert_eq!(result.stderr, "");
    }

    #[test]
    fn test_test_case_timeout_default() {
        // 验证 TestCase 的 timeout 默认值
        assert_eq!(default_timeout(), None);
    }

    #[test]
    fn test_use_patterns_default() {
        // 验证 TestExpectation 的 use_patterns 默认值
        assert!(!default_use_patterns());
    }
}
