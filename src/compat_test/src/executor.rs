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
        if self.config.debug {
            eprintln!("DEBUG: 开始执行测试用例: {}", test_case.description);
            eprintln!("DEBUG: 命令: {} {:?}", test_case.command, test_case.args);
        }

        // 获取期望结果
        let (expected, expected_verifications) = if self.config.coreutils_path.is_some() {
            // 基准模式：在独立沙箱中执行 coreutils 获取基准结果
            if self.config.debug {
                eprintln!("DEBUG: 使用基准模式（coreutils）");
            }
            self.get_coreutils_test_result(test_case)?
        } else {
            // 标准模式：使用测试用例中的期望结果
            if self.config.debug {
                eprintln!("DEBUG: 使用标准模式（预期结果）");
            }
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
        if self.config.debug {
            eprintln!("DEBUG: 开始执行syskits测试");
        }
        let (actual, actual_verifications) = self.execute_syskits_test(test_case)?;

        // 比较结果
        if self.config.debug {
            eprintln!("DEBUG: 开始比较测试结果");
        }
        let result = compare_results(
            test_case,
            expected,
            expected_verifications,
            actual,
            actual_verifications,
        )?;

        if self.config.debug {
            eprintln!(
                "DEBUG: 测试结果: {}",
                if result.passed { "通过" } else { "失败" }
            );
            if !result.differences.is_empty() {
                eprintln!("DEBUG: 差异:");
                for diff in &result.differences {
                    eprintln!("DEBUG:   {}", diff);
                }
            }
        }

        Ok(result)
    }

    fn execute_syskits_test(
        &self,
        test_case: &TestCase,
    ) -> Result<(CommandResult, Vec<CommandResult>)> {
        let mut sandbox = IsolatedSandbox::new(self.config.debug)?;
        sandbox.setup(test_case)?;

        // 执行设置命令
        if self.config.debug {
            eprintln!("DEBUG: 执行设置命令");
        }
        for cmd in &test_case.setup_commands {
            sandbox.execute_shell_command(cmd)?;
        }

        // 执行主命令
        if self.config.debug {
            eprintln!("DEBUG: 执行主命令");
        }

        let actual = self.execute_syskits(
            &test_case.tstdin,
            &test_case.command,
            &test_case.args,
            &mut sandbox,
        )?;
        // 执行验证命令
        if self.config.debug {
            eprintln!("DEBUG: 执行验证命令");
        }
        let mut actual_verifications = Vec::new();
        for verification in &test_case.expectation.verifications {
            if self.config.debug {
                eprintln!("DEBUG: 执行验证命令: {}", verification.command);
            }
            let result = sandbox.execute_shell_command(&verification.command)?;
            actual_verifications.push(result);
        }

        // 执行清理命令
        if self.config.debug {
            eprintln!("DEBUG: 执行清理命令");
        }
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
        let mut coreutils_sandbox = IsolatedSandbox::new(self.config.debug)?;
        coreutils_sandbox.setup(test_case)?;

        // 执行设置命令
        if self.config.debug {
            eprintln!("DEBUG: 执行coreutils设置命令");
        }
        for cmd in &test_case.setup_commands {
            coreutils_sandbox.execute_shell_command(cmd)?;
        }

        // 执行主命令
        if self.config.debug {
            eprintln!("DEBUG: 执行coreutils主命令");
        }
        let expected = self.execute_coreutils(
            &test_case.tstdin,
            &test_case.command,
            &test_case.args,
            &mut coreutils_sandbox,
        )?;

        // 执行验证命令
        if self.config.debug {
            eprintln!("DEBUG: 执行coreutils验证命令");
        }
        let mut verification_results = Vec::new();
        for verification in &test_case.expectation.verifications {
            if self.config.debug {
                eprintln!("DEBUG: 执行coreutils验证命令: {}", verification.command);
            }
            let result = coreutils_sandbox.execute_shell_command(&verification.command)?;
            verification_results.push(result);
        }

        // 执行清理命令
        if self.config.debug {
            eprintln!("DEBUG: 执行coreutils清理命令");
        }
        for cmd in &test_case.cleanup_commands {
            coreutils_sandbox.execute_shell_command(cmd)?;
        }

        coreutils_sandbox.cleanup()?;
        Ok((expected, verification_results))
    }

    /// 在沙箱中执行 syskits 命令
    fn execute_syskits(
        &self,
        tstdin: &str,
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

        sandbox.execute_command(&cmd, &args, Some(tstdin), true)
    }

    /// 在沙箱中执行 GNU coreutils 命令
    fn execute_coreutils(
        &self,
        tstdin: &str,
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

        sandbox.execute_command(command, args, Some(tstdin), true)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_case::{
        CommandExecution, FunctionalVerification, IgnoreFields, TestCase, TestExpectation,
    };

    // 创建一个简单的测试用例
    fn create_test_case(
        stdout: Option<String>,
        stderr: Option<String>,
        exit_code: Option<i32>,
        ignore_stdout: bool,
        ignore_stderr: bool,
        ignore_exit_code: bool,
    ) -> TestCase {
        TestCase {
            tstdin: "".to_string(),
            command: "test_command".to_string(),
            description: "Test case description".to_string(),
            args: vec!["arg1".to_string(), "arg2".to_string()],
            expectation: TestExpectation {
                execution: CommandExecution {
                    exit_code,
                    stdout,
                    stderr,
                },
                verifications: Vec::new(),
                use_patterns: false,
                env_changes: std::collections::HashMap::new(),
                file_changes: Vec::new(),
                ignore_fields: IgnoreFields {
                    ignore_exit_code,
                    ignore_stdout,
                    ignore_stderr,
                    ignore_verifications: false,
                },
            },
            setup_commands: Vec::new(),
            cleanup_commands: Vec::new(),
            requires_root: false,
            timeout: 30,
            tags: Vec::new(),
            environment: Default::default(),
        }
    }

    // 创建命令结果
    fn create_command_result(stdout: &str, stderr: &str, exit_code: i32) -> CommandResult {
        CommandResult {
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            exit_code,
        }
    }

    #[test]
    fn test_compare_results_all_match() {
        // 创建测试用例，所有字段都匹配
        let test_case = create_test_case(
            Some("output".to_string()),
            Some("error".to_string()),
            Some(0),
            false,
            false,
            false,
        );

        let expected = create_command_result("output", "error", 0);
        let actual = create_command_result("output", "error", 0);

        let result = compare_results(&test_case, expected, Vec::new(), actual, Vec::new()).unwrap();

        assert!(result.passed);
        assert!(result.differences.is_empty());
    }

    #[test]
    fn test_compare_results_stdout_differs() {
        // 创建测试用例，stdout不匹配
        let test_case = create_test_case(
            Some("expected_output".to_string()),
            Some("error".to_string()),
            Some(0),
            false,
            false,
            false,
        );

        let expected = create_command_result("expected_output", "error", 0);
        let actual = create_command_result("actual_output", "error", 0);

        let result = compare_results(&test_case, expected, Vec::new(), actual, Vec::new()).unwrap();

        assert!(!result.passed);
        assert_eq!(result.differences.len(), 1);
        assert!(result.differences[0].contains("stdout differs"));
    }

    #[test]
    fn test_compare_results_stderr_differs() {
        // 创建测试用例，stderr不匹配
        let test_case = create_test_case(
            Some("output".to_string()),
            Some("expected_error".to_string()),
            Some(0),
            false,
            false,
            false,
        );

        let expected = create_command_result("output", "expected_error", 0);
        let actual = create_command_result("output", "actual_error", 0);

        let result = compare_results(&test_case, expected, Vec::new(), actual, Vec::new()).unwrap();

        assert!(!result.passed);
        assert_eq!(result.differences.len(), 1);
        assert!(result.differences[0].contains("stderr differs"));
    }

    #[test]
    fn test_compare_results_exit_code_differs() {
        // 创建测试用例，exit_code不匹配
        let test_case = create_test_case(
            Some("output".to_string()),
            Some("error".to_string()),
            Some(0),
            false,
            false,
            false,
        );

        let expected = create_command_result("output", "error", 0);
        let actual = create_command_result("output", "error", 1);

        let result = compare_results(&test_case, expected, Vec::new(), actual, Vec::new()).unwrap();

        assert!(!result.passed);
        assert_eq!(result.differences.len(), 1);
        assert!(result.differences[0].contains("exit code differs"));
    }

    #[test]
    fn test_compare_results_ignore_stdout() {
        // 创建测试用例，忽略stdout比较
        let test_case = create_test_case(
            Some("expected_output".to_string()),
            Some("error".to_string()),
            Some(0),
            true, // ignore_stdout
            false,
            false,
        );

        let expected = create_command_result("expected_output", "error", 0);
        let actual = create_command_result("actual_output", "error", 0);

        let result = compare_results(&test_case, expected, Vec::new(), actual, Vec::new()).unwrap();

        assert!(result.passed);
        assert!(result.differences.is_empty());
    }

    #[test]
    fn test_compare_results_ignore_stderr() {
        // 创建测试用例，忽略stderr比较
        let test_case = create_test_case(
            Some("output".to_string()),
            Some("expected_error".to_string()),
            Some(0),
            false,
            true, // ignore_stderr
            false,
        );

        let expected = create_command_result("output", "expected_error", 0);
        let actual = create_command_result("output", "actual_error", 0);

        let result = compare_results(&test_case, expected, Vec::new(), actual, Vec::new()).unwrap();

        assert!(result.passed);
        assert!(result.differences.is_empty());
    }

    #[test]
    fn test_compare_results_ignore_exit_code() {
        // 创建测试用例，忽略exit_code比较
        let test_case = create_test_case(
            Some("output".to_string()),
            Some("error".to_string()),
            Some(0),
            false,
            false,
            true, // ignore_exit_code
        );

        let expected = create_command_result("output", "error", 0);
        let actual = create_command_result("output", "error", 1);

        let result = compare_results(&test_case, expected, Vec::new(), actual, Vec::new()).unwrap();

        assert!(result.passed);
        assert!(result.differences.is_empty());
    }

    #[test]
    fn test_compare_results_null_stdout() {
        // 创建测试用例，stdout为null
        let test_case = create_test_case(
            None, // stdout is null
            Some("error".to_string()),
            Some(0),
            false,
            false,
            false,
        );

        let expected = create_command_result("expected_output", "error", 0);
        let actual = create_command_result("actual_output", "error", 0);

        let result = compare_results(&test_case, expected, Vec::new(), actual, Vec::new()).unwrap();

        assert!(result.passed);
        assert!(result.differences.is_empty());
    }

    #[test]
    fn test_compare_results_null_stderr() {
        // 创建测试用例，stderr为null
        let test_case = create_test_case(
            Some("output".to_string()),
            None, // stderr is null
            Some(0),
            false,
            false,
            false,
        );

        let expected = create_command_result("output", "expected_error", 0);
        let actual = create_command_result("output", "actual_error", 0);

        let result = compare_results(&test_case, expected, Vec::new(), actual, Vec::new()).unwrap();

        assert!(result.passed);
        assert!(result.differences.is_empty());
    }

    #[test]
    fn test_compare_results_null_exit_code() {
        // 创建测试用例，exit_code为null
        let test_case = create_test_case(
            Some("output".to_string()),
            Some("error".to_string()),
            None, // exit_code is null
            false,
            false,
            false,
        );

        let expected = create_command_result("output", "error", 0);
        let actual = create_command_result("output", "error", 1);

        let result = compare_results(&test_case, expected, Vec::new(), actual, Vec::new()).unwrap();

        assert!(result.passed);
        assert!(result.differences.is_empty());
    }

    #[test]
    fn test_compare_results_all_null() {
        // 创建测试用例，所有字段都为null
        let test_case = create_test_case(
            None, // stdout is null
            None, // stderr is null
            None, // exit_code is null
            false, false, false,
        );

        let expected = create_command_result("expected_output", "expected_error", 0);
        let actual = create_command_result("actual_output", "actual_error", 1);

        let result = compare_results(&test_case, expected, Vec::new(), actual, Vec::new()).unwrap();

        assert!(result.passed);
        assert!(result.differences.is_empty());
    }

    #[test]
    fn test_compare_results_with_verifications() {
        // 创建带验证命令的测试用例
        let mut test_case = create_test_case(
            Some("output".to_string()),
            Some("error".to_string()),
            Some(0),
            false,
            false,
            false,
        );

        // 添加验证命令
        test_case.expectation.verifications = vec![FunctionalVerification {
            command: "test_verification".to_string(),
            expected_exit: Some(0),
            expected_stdout: Some("verification_output".to_string()),
            expected_stderr: Some("verification_error".to_string()),
        }];

        let expected = create_command_result("output", "error", 0);
        let actual = create_command_result("output", "error", 0);

        let expected_verification =
            create_command_result("verification_output", "verification_error", 0);
        let actual_verification =
            create_command_result("verification_output", "verification_error", 0);

        let result = compare_results(
            &test_case,
            expected,
            vec![expected_verification],
            actual,
            vec![actual_verification],
        )
        .unwrap();

        assert!(result.passed);
        assert!(result.differences.is_empty());
    }

    #[test]
    fn test_compare_results_verification_differs() {
        // 创建带验证命令的测试用例，验证结果不匹配
        let mut test_case = create_test_case(
            Some("output".to_string()),
            Some("error".to_string()),
            Some(0),
            false,
            false,
            false,
        );

        // 添加验证命令
        test_case.expectation.verifications = vec![FunctionalVerification {
            command: "test_verification".to_string(),
            expected_exit: Some(0),
            expected_stdout: Some("expected_verification_output".to_string()),
            expected_stderr: Some("verification_error".to_string()),
        }];

        let expected = create_command_result("output", "error", 0);
        let actual = create_command_result("output", "error", 0);

        let expected_verification =
            create_command_result("expected_verification_output", "verification_error", 0);
        let actual_verification =
            create_command_result("actual_verification_output", "verification_error", 0);

        let result = compare_results(
            &test_case,
            expected,
            vec![expected_verification],
            actual,
            vec![actual_verification],
        )
        .unwrap();

        assert!(!result.passed);
        assert_eq!(result.differences.len(), 1);
        assert!(result.differences[0].contains("Verification"));
    }

    #[test]
    fn test_compare_results_verification_null_stdout() {
        // 创建带验证命令的测试用例，验证命令的stdout为null
        let mut test_case = create_test_case(
            Some("output".to_string()),
            Some("error".to_string()),
            Some(0),
            false,
            false,
            false,
        );

        // 添加验证命令，stdout为null
        test_case.expectation.verifications = vec![FunctionalVerification {
            command: "test_verification".to_string(),
            expected_exit: Some(0),
            expected_stdout: None, // stdout is null
            expected_stderr: Some("verification_error".to_string()),
        }];

        let expected = create_command_result("output", "error", 0);
        let actual = create_command_result("output", "error", 0);

        let expected_verification =
            create_command_result("expected_verification_output", "verification_error", 0);
        let actual_verification =
            create_command_result("actual_verification_output", "verification_error", 0);

        let result = compare_results(
            &test_case,
            expected,
            vec![expected_verification],
            actual,
            vec![actual_verification],
        )
        .unwrap();

        // 即使stdout不同，但因为expected_stdout为null，所以测试应该通过
        assert!(result.passed);
        assert!(result.differences.is_empty());
    }

    #[test]
    fn test_compare_results_verification_null_stderr() {
        // 创建带验证命令的测试用例，验证命令的stderr为null
        let mut test_case = create_test_case(
            Some("output".to_string()),
            Some("error".to_string()),
            Some(0),
            false,
            false,
            false,
        );

        // 添加验证命令，stderr为null
        test_case.expectation.verifications = vec![FunctionalVerification {
            command: "test_verification".to_string(),
            expected_exit: Some(0),
            expected_stdout: Some("verification_output".to_string()),
            expected_stderr: None, // stderr is null
        }];

        let expected = create_command_result("output", "error", 0);
        let actual = create_command_result("output", "error", 0);

        let expected_verification =
            create_command_result("verification_output", "expected_verification_error", 0);
        let actual_verification =
            create_command_result("verification_output", "actual_verification_error", 0);

        let result = compare_results(
            &test_case,
            expected,
            vec![expected_verification],
            actual,
            vec![actual_verification],
        )
        .unwrap();

        // 即使stderr不同，但因为expected_stderr为null，所以测试应该通过
        assert!(result.passed);
        assert!(result.differences.is_empty());
    }

    #[test]
    fn test_compare_results_verification_all_null() {
        // 创建带验证命令的测试用例，验证命令的stdout和stderr都为null
        let mut test_case = create_test_case(
            Some("output".to_string()),
            Some("error".to_string()),
            Some(0),
            false,
            false,
            false,
        );

        // 添加验证命令，stdout和stderr都为null
        test_case.expectation.verifications = vec![FunctionalVerification {
            command: "test_verification".to_string(),
            expected_exit: Some(0),
            expected_stdout: None, // stdout is null
            expected_stderr: None, // stderr is null
        }];

        let expected = create_command_result("output", "error", 0);
        let actual = create_command_result("output", "error", 0);

        let expected_verification = create_command_result(
            "expected_verification_output",
            "expected_verification_error",
            0,
        );
        let actual_verification =
            create_command_result("actual_verification_output", "actual_verification_error", 0);

        let result = compare_results(
            &test_case,
            expected,
            vec![expected_verification],
            actual,
            vec![actual_verification],
        )
        .unwrap();

        // 即使stdout和stderr都不同，但因为expected_stdout和expected_stderr都为null，所以测试应该通过
        assert!(result.passed);
        assert!(result.differences.is_empty());
    }

    #[test]
    fn test_compare_results_mixed_null_and_non_null() {
        // 创建测试用例，混合null和非null字段
        let test_case = create_test_case(
            Some("output".to_string()), // stdout不为null
            None,                       // stderr为null
            Some(0),                    // exit_code不为null
            false,
            false,
            false,
        );

        let expected = create_command_result("output", "expected_error", 0);
        let actual = create_command_result("output", "actual_error", 0);

        let result = compare_results(&test_case, expected, Vec::new(), actual, Vec::new()).unwrap();

        // 只比较非null字段，所以测试应该通过
        assert!(result.passed);
        assert!(result.differences.is_empty());
    }

    #[test]
    fn test_compare_results_verification_null_exit_code() {
        // 创建带验证命令的测试用例，验证命令的expected_exit为null
        let mut test_case = create_test_case(
            Some("output".to_string()),
            Some("error".to_string()),
            Some(0),
            false,
            false,
            false,
        );

        // 添加验证命令，expected_exit为null
        test_case.expectation.verifications = vec![FunctionalVerification {
            command: "test_verification".to_string(),
            expected_exit: None, // exit_code is null
            expected_stdout: Some("verification_output".to_string()),
            expected_stderr: Some("verification_error".to_string()),
        }];

        let expected = create_command_result("output", "error", 0);
        let actual = create_command_result("output", "error", 0);

        let expected_verification =
            create_command_result("verification_output", "verification_error", 0);
        let actual_verification =
            create_command_result("verification_output", "verification_error", 1); // 不同的exit_code

        let result = compare_results(
            &test_case,
            expected,
            vec![expected_verification],
            actual,
            vec![actual_verification],
        )
        .unwrap();

        // 即使exit_code不同，但因为expected_exit为null，所以测试应该通过
        assert!(result.passed);
        assert!(result.differences.is_empty());
    }
}
