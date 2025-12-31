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

//! syskits 与 GNU coreutils 的兼容性测试框架
//! 本 crate 提供了测试 syskits 命令与其 GNU coreutils 对应命令兼容性的功能

pub mod config;
pub mod executor;
pub mod reporter;
pub mod sandbox;
pub mod test_case;

use crate::config::Config;
use crate::executor::{CommandExecutor, ParallelTestExecutor};
use crate::test_case::TestCaseManager;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::Output;

/// 命令执行结果
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandResult {
    /// UTF-8 格式的标准输出
    pub stdout: String,
    /// UTF-8 格式的标准错误
    pub stderr: String,
    /// 退出状态码
    pub exit_code: i32,
}

impl From<Output> for CommandResult {
    fn from(output: Output) -> Self {
        let exit_code = if let Some(signal) = output.status.signal() {
            128 + signal // 标准 Unix 做法：信号退出码为 128 + 信号编号
        } else {
            output.status.code().unwrap_or(-1)
        };

        CommandResult {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            exit_code,
        }
    }
}

/// syskits 与 GNU coreutils 的比较结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonResult {
    /// 被测试的命令
    pub command: String,
    /// 测试用例描述
    pub description: String,
    /// 使用的参数
    pub args: Vec<String>,
    /// 期望的结果
    pub expected: CommandResult,
    /// syskits 的实际结果
    pub actual: CommandResult,
    /// 测试是否通过
    pub passed: bool,
    /// 如果测试失败，记录详细的差异
    pub differences: Vec<String>,
}

/// 测试环境配置
#[derive(Debug, Clone)]
pub struct TestConfig {
    /// syskits 可执行文件路径
    pub syskits_path: PathBuf,
    /// GNU coreutils 可执行文件路径
    pub coreutils_path: Option<PathBuf>,
    /// 测试用例目录
    pub test_cases_dir: PathBuf,
    /// 是否显示测试进度
    pub show_progress: bool,
    /// 是否清理测试临时文件
    pub cleanup: bool,
    /// 报告格式（text、json、html）
    pub report_format: String,
    /// 测试报告目录
    pub report_dir: PathBuf,
    /// 测试默认超时时间（秒）
    pub default_timeout: u64,
    /// 是否显示测试失败的详细差异
    pub show_diff: bool,
    /// syskits 执行模式
    pub mode: config::SyskitsMode,
    /// 多命令模式下的命令目录
    pub commands_dir: Option<PathBuf>,
    /// 是否显示详细输出
    pub verbose: bool,
    /// 是否启用调试输出
    pub debug: bool,
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            syskits_path: PathBuf::from("target/debug/syskits"),
            coreutils_path: None,
            test_cases_dir: PathBuf::from("test_cases"),
            show_progress: true,
            cleanup: true,
            report_format: "text".to_string(),
            report_dir: PathBuf::from("test_reports"),
            default_timeout: 30,
            show_diff: true,
            mode: config::SyskitsMode::Single,
            commands_dir: None,
            verbose: false,
            debug: false,
        }
    }
}

impl TestConfig {
    /// 通过合并命令行参数和配置文件值创建新的 TestConfig
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        cmd_syskits_path: Option<PathBuf>,
        cmd_coreutils_path: Option<PathBuf>,
        cmd_test_cases_dir: Option<PathBuf>,
        cmd_no_progress: bool,
        cmd_no_cleanup: bool,
        cmd_report_format: Option<String>,
        cmd_verbose: bool,
        cmd_debug: bool,
        config_file: Option<&Config>,
    ) -> Self {
        // 从默认值开始
        let mut config = Self::default();

        // 应用配置文件中的值（如果有）
        if let Some(cfg) = config_file {
            if let Some(path) = &cfg.syskits.syskits_path {
                config.syskits_path = path.clone();
            }
            if let Some(path) = &cfg.syskits.coreutils_path {
                config.coreutils_path = Some(path.clone());
            }
            if let Some(path) = &cfg.test.test_cases_dir {
                config.test_cases_dir = path.clone();
            }
            config.show_progress = cfg.test.env.show_progress;
            config.cleanup = cfg.test.env.cleanup;
            config.report_format = cfg.test.env.report_format.clone();
            config.report_dir = cfg.test.env.report_dir.clone();
            config.default_timeout = cfg.test.env.default_timeout;
            config.show_diff = cfg.test.env.show_diff;
            config.mode = cfg.syskits.mode.clone();
            config.commands_dir = cfg.syskits.commands_dir.clone();
            config.verbose = cfg.test.env.verbose;
            config.debug = cfg.test.env.debug;
        }

        // 应用命令行参数（最高优先级）
        if let Some(path) = cmd_syskits_path {
            config.syskits_path = path;
        }
        if let Some(path) = cmd_coreutils_path {
            config.coreutils_path = Some(path);
        }
        if let Some(path) = cmd_test_cases_dir {
            config.test_cases_dir = path;
        }
        if cmd_no_progress {
            config.show_progress = false;
        }
        if cmd_no_cleanup {
            config.cleanup = false;
        }
        if let Some(format) = cmd_report_format {
            config.report_format = format;
        }
        if cmd_verbose {
            config.verbose = true;
        }
        if cmd_debug {
            config.debug = true;
        }

        config
    }
}

/// 兼容性测试框架的错误类型
#[derive(Debug, thiserror::Error)]
pub enum TestError {
    #[error("Failed to execute command: {0}")]
    ExecutionError(String),
    #[error("Failed to read test case: {0}")]
    TestCaseError(String),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    SerializationError(String),
    #[error("System error: {0}")]
    SystemError(#[from] nix::Error),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, TestError>;

pub fn add(left: usize, right: usize) -> usize {
    left + right
}

/// 测试运行器
/// 用于管理和执行兼容性测试
pub struct TestRunner {
    /// 测试配置
    config: TestConfig,
    /// 测试用例管理器
    pub test_manager: TestCaseManager,
}

impl TestRunner {
    /// 创建新的测试运行器
    pub fn new(config: TestConfig) -> Self {
        let test_manager = TestCaseManager::new(&config.test_cases_dir);
        Self {
            config,
            test_manager,
        }
    }

    /// 运行指定命令的测试
    pub fn run_command_tests(&self, command: &str) -> Result<Vec<ComparisonResult>> {
        let executor = CommandExecutor::new(self.config.clone());
        let test_cases = self.test_manager.load_test_cases(command)?;

        let mut results = Vec::new();
        for test_case in test_cases {
            let result = executor.execute_test(&test_case)?;
            results.push(result);
        }

        Ok(results)
    }

    /// 运行所有可用命令的测试
    pub fn run_all_tests(&self) -> Result<HashMap<String, Vec<ComparisonResult>>> {
        let mut all_results = HashMap::new();
        let commands = self.test_manager.get_available_commands()?;

        // 串行执行每个命令的测试
        for command in commands {
            let results = self.run_command_tests(&command)?;
            all_results.insert(command, results);
        }

        Ok(all_results)
    }

    /// 并行运行指定命令的测试
    pub fn run_command_tests_parallel(&self, command: &str) -> Result<Vec<ComparisonResult>> {
        let test_cases = self.test_manager.load_test_cases(command)?;

        // 使用 rayon 并行执行测试用例
        let results: Result<Vec<_>> = test_cases
            .par_iter()
            .map(|test_case| {
                let executor = ParallelTestExecutor::new(self.config.clone());
                executor.execute_test(test_case)
            })
            .collect();

        results
    }

    /// 并行运行所有可用命令的测试
    pub fn run_all_tests_parallel(&self) -> Result<HashMap<String, Vec<ComparisonResult>>> {
        let commands = self.test_manager.get_available_commands()?;

        let results: HashMap<String, Vec<ComparisonResult>> = commands
            .par_iter()
            .map(|cmd| {
                let results = self.run_command_tests_parallel(cmd)?;
                Ok((cmd.clone(), results))
            })
            .collect::<Result<HashMap<_, _>>>()?;

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }

    #[test]
    fn test_config_default() {
        let config = TestConfig::default();
        assert_eq!(config.syskits_path, PathBuf::from("target/debug/syskits"));
        assert_eq!(config.test_cases_dir, PathBuf::from("test_cases"));
        assert!(config.show_progress);
        assert!(config.cleanup);
        assert_eq!(config.report_format, "text");
        assert_eq!(config.report_dir, PathBuf::from("test_reports"));
        assert_eq!(config.default_timeout, 30);
        assert!(config.show_diff);
    }
}