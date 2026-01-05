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

//! 测试执行器模块
//! 提供并行和串行测试执行功能，支持命令执行和结果比较

use crate::config::SyskitsMode;
use crate::sandbox::IsolatedSandbox;
use crate::test_case::TestCase;
use crate::{CommandResult, ComparisonResult, Result, TestConfig, TestError};

/// 并行测试执行器
pub struct ParallelTestExecutor {
    /// 测试配置
    config: TestConfig,
}

impl ParallelTestExecutor {
    /// 创建新的并行测试执行器
    pub fn new(config: TestConfig) -> Self {
        Self { config }
    }

    /// 执行测试用例
    pub fn execute_test(&self, test_case: &TestCase) -> Result<ComparisonResult> {
        // 使用 CommandExecutor 来执行测试
        let executor = CommandExecutor::new(self.config.clone());
        executor.execute_test(test_case)
    }
}

/// 命令执行器
/// 用于运行和比较命令执行结果
pub struct CommandExecutor {
    /// 测试配置
    config: TestConfig,
}

impl CommandExecutor {
    /// 创建新的命令执行器
    pub fn new(config: TestConfig) -> Self {
        Self { config }
    }

    /// 执行测试用例
    pub fn execute_test(&self, test_case: &TestCase) -> Result<ComparisonResult> {
        // 获取期望结果
        let (expected, expected_verifications) = if self.config.coreutils_path.is_some() {
            // 基准模式：在独立沙箱中执行 coreutils 获取基准结果
            self.get_coreutils_test_result(test_case)?
        } else {
            // 标准模式：使用测试用例中的期望结果
            let expected = test_case.expectation.execution.clone().into();
            let verification_results = test_case
                .expectation
                .verifications
                .iter()
                .map(|v| CommandResult {
                    exit_code: v.expected_exit.unwrap_or(0),
                    stdout: v.expected_stdout.clone().unwrap_or_default(),
                    stderr: v.expected_stderr.clone().unwrap_or_default(),
                })
                .collect();
            (expected, verification_results)
        };

        // 在新的沙箱中执行 syskits 测试
        let (actual, actual_verifications) = self.execute_syskits_test(test_case)?;

        // 比较结果
        compare_results(
            test_case,
            expected,
            expected_verifications,
            actual,
            actual_verifications,
        )
    }

    fn execute_syskits_test(
        &self,
        test_case: &TestCase,
    ) -> Result<(CommandResult, Vec<CommandResult>)> {
        let mut sandbox = IsolatedSandbox::new()?;
        sandbox.setup(test_case)?;

        // 执行设置命令
        for cmd in &test_case.setup_commands {
            sandbox.execute_shell_command(cmd)?;
        }

        // 执行主命令
        let actual = self.execute_syskits(&test_case.command, &test_case.args, &mut sandbox)?;

        // 此时，CMD_EXIT_CODE, CMD_STDOUT, CMD_STDERR 环境变量已经设置

        // 执行验证命令
        let mut actual_verifications = Vec::new();
        for verification in &test_case.expectation.verifications {
            let result = sandbox.execute_shell_command(&verification.command)?;
            actual_verifications.push(result);
        }

        // 执行清理命令
        for cmd in &test_case.cleanup_commands {
            sandbox.execute_shell_command(cmd)?;
        }

        sandbox.cleanup()?;

        Ok((actual, actual_verifications))
    }

    fn get_coreutils_test_result(
        &self,
        test_case: &TestCase,
    ) -> Result<(CommandResult, Vec<CommandResult>)> {
        let mut coreutils_sandbox = IsolatedSandbox::new()?;
        coreutils_sandbox.setup(test_case)?;

        // 执行设置命令
        for cmd in &test_case.setup_commands {
            coreutils_sandbox.execute_shell_command(cmd)?;
        }
        // 执行主命令
        let expected =
            self.execute_coreutils(&test_case.command, &test_case.args, &mut coreutils_sandbox)?;
        // 执行验证命令
        let mut verification_results = Vec::new();
        for verification in &test_case.expectation.verifications {
            let result = coreutils_sandbox.execute_shell_command(&verification.command)?;
            verification_results.push(result);
        }
        // 执行清理命令
        for cmd in &test_case.cleanup_commands {
            coreutils_sandbox.execute_shell_command(cmd)?;
        }

        coreutils_sandbox.cleanup()?;
        Ok((expected, verification_results))
    }

    /// 在沙箱中执行 syskits 命令
    fn execute_syskits(
        &self,
        command: &str,
        args: &[String],
        sandbox: &mut IsolatedSandbox,
    ) -> Result<CommandResult> {
        let (cmd, args) = match self.config.mode {
            SyskitsMode::Single => {
                // 确保 syskits_path 是绝对路径
                let syskits_path = if self.config.syskits_path.is_absolute() {
                    self.config.syskits_path.clone()
                } else {
                    std::env::current_dir()?.join(&self.config.syskits_path)
                };

                if !syskits_path.exists() {
                    return Err(TestError::ExecutionError(format!(
                        "Syskits binary not found at: {}",
                        syskits_path.display()
                    )));
                }

                let syskits_str = syskits_path.to_str().unwrap().to_string();
                let mut modified_args = Vec::with_capacity(args.len() + 1);
                modified_args.push(command.to_string());
                modified_args.extend_from_slice(args);
                (syskits_str, modified_args)
            }
            SyskitsMode::Multiple => (command.to_string(), args.to_vec()),
        };

        sandbox.execute_command(&cmd, &args, true)
    }

    /// 在沙箱中执行 GNU coreutils 命令
    fn execute_coreutils(
        &self,
        command: &str,
        args: &[String],
        sandbox: &mut IsolatedSandbox,
    ) -> Result<CommandResult> {
        // 确保设置正确的 PATH 环境变量
        if let Some(ref coreutils_path) = self.config.coreutils_path {
            // 获取当前的 PATH 环境变量
            let current_path = sandbox.get_env("PATH").unwrap_or_default();
            // 将 coreutils 路径添加到 PATH 的开头
            let new_path = format!("{}:{}", coreutils_path.display(), current_path);
            sandbox.add_env("PATH", &new_path);
        }

        // 沙箱中执行命令
        sandbox.execute_command(command, args, true)
    }
}

fn compare_results(
    test_case: &TestCase,
    expected: CommandResult,
    expected_verifications: Vec<CommandResult>,
    actual: CommandResult,
    actual_verifications: Vec<CommandResult>,
) -> Result<ComparisonResult> {
    let mut differences = Vec::new();
    let mut passed = true;

    // 比较主命令结果
    if !test_case.expectation.ignore_fields.ignore_stdout
        && test_case.expectation.execution.stdout.is_some()
        && expected.stdout != actual.stdout
    {
        differences.push(format!(
            "Main command stdout differs:\nExpected:\n{}\nActual:\n{}",
            expected.stdout, actual.stdout
        ));
        passed = false;
    }

    if !test_case.expectation.ignore_fields.ignore_stderr
        && test_case.expectation.execution.stderr.is_some()
        && expected.stderr != actual.stderr
    {
        differences.push(format!(
            "Main command stderr differs:\nExpected:\n{}\nActual:\n{}",
            expected.stderr, actual.stderr
        ));
        passed = false;
    }

    if !test_case.expectation.ignore_fields.ignore_exit_code
        && test_case.expectation.execution.exit_code.is_some()
        && expected.exit_code != actual.exit_code
    {
        differences.push(format!(
            "Main command exit code differs: expected {}, got {}",
            expected.exit_code, actual.exit_code
        ));
        passed = false;
    }

    // 比较验证命令结果
    if !test_case.expectation.ignore_fields.ignore_verifications {
        for ((expected, actual), verification) in expected_verifications
            .iter()
            .zip(actual_verifications.iter())
            .zip(test_case.expectation.verifications.iter())
        {
            // 只有当expected_exit不为null时才比较exit_code
            if verification.expected_exit.is_some() && expected.exit_code != actual.exit_code {
                differences.push(format!(
                    "Verification '{}' exit code differs: expected {}, got {}",
                    verification.command, expected.exit_code, actual.exit_code
                ));
                passed = false;
            }

            // 检查verification的expected_stdout是否为null
            if verification.expected_stdout.is_some() && expected.stdout != actual.stdout {
                differences.push(format!(
                    "Verification '{}' stdout differs:\nExpected:\n{}\nActual:\n{}",
                    verification.command, expected.stdout, actual.stdout
                ));
                passed = false;
            }

            // 检查verification的expected_stderr是否为null
            if verification.expected_stderr.is_some() && expected.stderr != actual.stderr {
                differences.push(format!(
                    "Verification '{}' stderr differs:\nExpected:\n{}\nActual:\n{}",
                    verification.command, expected.stderr, actual.stderr
                ));
                passed = false;
            }
        }
    }

    Ok(ComparisonResult {
        command: test_case.command.clone(),
        description: test_case.description.clone(),
        args: test_case.args.clone(),
        expected,
        actual,
        passed,
        differences,
    })
}


