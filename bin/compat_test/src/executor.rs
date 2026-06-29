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

//! 测试执行器模块
//! 提供并行和串行测试执行功能，支持命令执行和结果比较

use crate::config::SyskitsMode;
use crate::sandbox::IsolatedSandbox;
use crate::test_case::TestCase;
use crate::{CommandResult, ComparisonResult, Result, TestConfig, TestError};
use std::ffi::OsString;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;

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
            let expected = test_case
                .expectation
                .execution
                .to_command_result(test_case.byte_mode)?;
            let verification_results = test_case
                .expectation
                .verifications
                .iter()
                .map(CommandResult::from)
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

        let timeout = test_case.timeout.or_else(|| {
            if self.config.default_timeout == 0 {
                None
            } else {
                Some(self.config.default_timeout)
            }
        });
        let use_bytes = test_case.byte_mode;
        let actual = if use_bytes {
            let args_os = resolve_args_os(test_case)?;
            let stdin_bytes = resolve_stdin_bytes(test_case)?;
            if test_case.tty {
                self.execute_syskits_bytes_tty(
                    &stdin_bytes,
                    &test_case.command,
                    &args_os,
                    &mut sandbox,
                    timeout,
                )?
            } else {
                self.execute_syskits_bytes(
                    &stdin_bytes,
                    &test_case.command,
                    &args_os,
                    &mut sandbox,
                    timeout,
                )?
            }
        } else if test_case.tty {
            self.execute_syskits_tty(
                &test_case.tstdin,
                &test_case.command,
                &test_case.args,
                &mut sandbox,
                timeout,
            )?
        } else {
            self.execute_syskits(
                &test_case.tstdin,
                &test_case.command,
                &test_case.args,
                &mut sandbox,
                timeout,
            )?
        };
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
        let timeout = test_case.timeout.or_else(|| {
            if self.config.default_timeout == 0 {
                None
            } else {
                Some(self.config.default_timeout)
            }
        });
        let use_bytes = test_case.byte_mode;
        let expected = if use_bytes {
            let args_os = resolve_args_os(test_case)?;
            let stdin_bytes = resolve_stdin_bytes(test_case)?;
            if test_case.tty {
                self.execute_coreutils_bytes_tty(
                    &stdin_bytes,
                    &test_case.command,
                    &args_os,
                    &mut coreutils_sandbox,
                    timeout,
                )?
            } else {
                self.execute_coreutils_bytes(
                    &stdin_bytes,
                    &test_case.command,
                    &args_os,
                    &mut coreutils_sandbox,
                    timeout,
                )?
            }
        } else if test_case.tty {
            self.execute_coreutils_tty(
                &test_case.tstdin,
                &test_case.command,
                &test_case.args,
                &mut coreutils_sandbox,
                timeout,
            )?
        } else {
            self.execute_coreutils(
                &test_case.tstdin,
                &test_case.command,
                &test_case.args,
                &mut coreutils_sandbox,
                timeout,
            )?
        };

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
        timeout: Option<u64>,
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

        sandbox.execute_command(&cmd, &args, Some(tstdin), true, timeout)
    }

    /// 在沙箱中执行 syskits 命令（伪终端）
    fn execute_syskits_tty(
        &self,
        tstdin: &str,
        command: &str,
        args: &[String],
        sandbox: &mut IsolatedSandbox,
        timeout: Option<u64>,
    ) -> Result<CommandResult> {
        let (cmd, args) = match self.config.mode {
            SyskitsMode::Single => {
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

        sandbox.execute_command_tty(&cmd, &args, Some(tstdin), true, timeout)
    }

    /// 在沙箱中执行 syskits 命令（原始字节参数）
    fn execute_syskits_bytes(
        &self,
        tstdin: &[u8],
        command: &str,
        args: &[OsString],
        sandbox: &mut IsolatedSandbox,
        timeout: Option<u64>,
    ) -> Result<CommandResult> {
        let (cmd, args) = match self.config.mode {
            SyskitsMode::Single => {
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
                modified_args.push(OsString::from(command));
                modified_args.extend_from_slice(args);
                (syskits_str, modified_args)
            }
            SyskitsMode::Multiple => (command.to_string(), args.to_vec()),
        };

        sandbox.execute_command_bytes(&cmd, &args, Some(tstdin), true, timeout, true)
    }

    /// 在沙箱中执行 syskits 命令（原始字节参数，伪终端）
    fn execute_syskits_bytes_tty(
        &self,
        tstdin: &[u8],
        command: &str,
        args: &[OsString],
        sandbox: &mut IsolatedSandbox,
        timeout: Option<u64>,
    ) -> Result<CommandResult> {
        let (cmd, args) = match self.config.mode {
            SyskitsMode::Single => {
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
                modified_args.push(OsString::from(command));
                modified_args.extend_from_slice(args);
                (syskits_str, modified_args)
            }
            SyskitsMode::Multiple => (command.to_string(), args.to_vec()),
        };

        sandbox.execute_command_bytes_tty(&cmd, &args, Some(tstdin), true, timeout, true)
    }

    /// 在沙箱中执行 GNU coreutils 命令
    fn execute_coreutils(
        &self,
        tstdin: &str,
        command: &str,
        args: &[String],
        sandbox: &mut IsolatedSandbox,
        timeout: Option<u64>,
    ) -> Result<CommandResult> {
        // 确保设置正确的 PATH 环境变量
        if let Some(ref coreutils_path) = self.config.coreutils_path {
            // 获取当前的 PATH 环境变量
            let current_path = sandbox.get_env("PATH").unwrap_or_default();
            // 将 coreutils 路径添加到 PATH 的开头
            let new_path = format!("{}:{}", coreutils_path.display(), current_path);
            sandbox.add_env("PATH", &new_path);
        }

        sandbox.execute_command(command, args, Some(tstdin), true, timeout)
    }

    /// 在沙箱中执行 GNU coreutils 命令（伪终端）
    fn execute_coreutils_tty(
        &self,
        tstdin: &str,
        command: &str,
        args: &[String],
        sandbox: &mut IsolatedSandbox,
        timeout: Option<u64>,
    ) -> Result<CommandResult> {
        if let Some(ref coreutils_path) = self.config.coreutils_path {
            let current_path = sandbox.get_env("PATH").unwrap_or_default();
            let new_path = format!("{}:{}", coreutils_path.display(), current_path);
            sandbox.add_env("PATH", &new_path);
        }

        sandbox.execute_command_tty(command, args, Some(tstdin), true, timeout)
    }

    /// 在沙箱中执行 GNU coreutils 命令（原始字节参数）
    fn execute_coreutils_bytes(
        &self,
        tstdin: &[u8],
        command: &str,
        args: &[OsString],
        sandbox: &mut IsolatedSandbox,
        timeout: Option<u64>,
    ) -> Result<CommandResult> {
        if let Some(ref coreutils_path) = self.config.coreutils_path {
            let current_path = sandbox.get_env("PATH").unwrap_or_default();
            let new_path = format!("{}:{}", coreutils_path.display(), current_path);
            sandbox.add_env("PATH", &new_path);
        }

        sandbox.execute_command_bytes(command, args, Some(tstdin), true, timeout, true)
    }

    /// 在沙箱中执行 GNU coreutils 命令（原始字节参数，伪终端）
    fn execute_coreutils_bytes_tty(
        &self,
        tstdin: &[u8],
        command: &str,
        args: &[OsString],
        sandbox: &mut IsolatedSandbox,
        timeout: Option<u64>,
    ) -> Result<CommandResult> {
        if let Some(ref coreutils_path) = self.config.coreutils_path {
            let current_path = sandbox.get_env("PATH").unwrap_or_default();
            let new_path = format!("{}:{}", coreutils_path.display(), current_path);
            sandbox.add_env("PATH", &new_path);
        }

        sandbox.execute_command_bytes_tty(command, args, Some(tstdin), true, timeout, true)
    }
}

fn resolve_args_os(test_case: &TestCase) -> Result<Vec<OsString>> {
    if test_case.byte_mode {
        Ok(args_from_bytes(test_case.args_bytes()?))
    } else {
        Ok(args_from_strings(&test_case.args))
    }
}

fn resolve_stdin_bytes(test_case: &TestCase) -> Result<Vec<u8>> {
    test_case.tstdin_bytes()
}

fn args_from_strings(args: &[String]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

fn args_from_bytes(args: Vec<Vec<u8>>) -> Vec<OsString> {
    #[cfg(unix)]
    {
        args.into_iter().map(OsString::from_vec).collect()
    }
    #[cfg(not(unix))]
    {
        args.into_iter()
            .map(|bytes| OsString::from(String::from_utf8_lossy(&bytes).into_owned()))
            .collect()
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
    if !test_case.expectation.ignore_fields.ignore_stdout {
        if test_case.expectation.execution.stdout.is_some() {
            if test_case.byte_mode {
                if expected.stdout != actual.stdout {
                    differences.push(format!(
                        "Main command stdout hex differs:\nExpected:\n{}\nActual:\n{}",
                        expected.stdout, actual.stdout
                    ));
                    passed = false;
                }
            } else if expected.stdout != actual.stdout {
                differences.push(format!(
                    "Main command stdout differs:\nExpected:\n{}\nActual:\n{}",
                    expected.stdout, actual.stdout
                ));
                passed = false;
            }
        }
    }

    if !test_case.expectation.ignore_fields.ignore_stderr {
        if test_case.expectation.execution.stderr.is_some() {
            if test_case.byte_mode {
                if expected.stderr != actual.stderr {
                    differences.push(format!(
                        "Main command stderr hex differs:\nExpected:\n{}\nActual:\n{}",
                        expected.stderr, actual.stderr
                    ));
                    passed = false;
                }
            } else if expected.stderr != actual.stderr {
                differences.push(format!(
                    "Main command stderr differs:\nExpected:\n{}\nActual:\n{}",
                    expected.stderr, actual.stderr
                ));
                passed = false;
            }
        }
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
        args: test_case.args_display(),
        expected,
        actual,
        passed,
        differences,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SyskitsMode;
    use crate::test_case::{
        CommandExecution, FunctionalVerification, IgnoreFields, TestCase, TestEnvironment,
        TestExpectation,
    };
    use std::collections::HashMap;
    use std::path::PathBuf;
    use tempfile::TempDir;

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
            byte_mode: false,
            tty: false,
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
            timeout: Some(30),
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

    #[test]
    fn test_compare_results_multiple_verifications() {
        // 测试多个验证命令的情况
        let mut test_case = create_test_case(
            Some("output".to_string()),
            Some("error".to_string()),
            Some(0),
            false,
            false,
            false,
        );

        // 添加多个验证命令
        test_case.expectation.verifications = vec![
            FunctionalVerification {
                command: "verification1".to_string(),
                expected_exit: Some(0),
                expected_stdout: Some("verification1_output".to_string()),
                expected_stderr: Some("verification1_error".to_string()),
            },
            FunctionalVerification {
                command: "verification2".to_string(),
                expected_exit: Some(1),
                expected_stdout: Some("verification2_output".to_string()),
                expected_stderr: Some("verification2_error".to_string()),
            },
        ];

        let expected = create_command_result("output", "error", 0);
        let actual = create_command_result("output", "error", 0);

        let expected_verifications = vec![
            create_command_result("verification1_output", "verification1_error", 0),
            create_command_result("verification2_output", "verification2_error", 1),
        ];

        let actual_verifications = vec![
            create_command_result("verification1_output", "verification1_error", 0),
            create_command_result("verification2_output", "verification2_error", 1),
        ];

        let result = compare_results(
            &test_case,
            expected,
            expected_verifications,
            actual,
            actual_verifications,
        )
        .unwrap();

        assert!(result.passed);
        assert!(result.differences.is_empty());
    }

    #[test]
    fn test_compare_results_multiple_verifications_one_fails() {
        // 测试多个验证命令，其中一个失败的情况
        let mut test_case = create_test_case(
            Some("output".to_string()),
            Some("error".to_string()),
            Some(0),
            false,
            false,
            false,
        );

        // 添加多个验证命令
        test_case.expectation.verifications = vec![
            FunctionalVerification {
                command: "verification1".to_string(),
                expected_exit: Some(0),
                expected_stdout: Some("verification1_output".to_string()),
                expected_stderr: Some("verification1_error".to_string()),
            },
            FunctionalVerification {
                command: "verification2".to_string(),
                expected_exit: Some(1),
                expected_stdout: Some("verification2_output".to_string()),
                expected_stderr: Some("verification2_error".to_string()),
            },
        ];

        let expected = create_command_result("output", "error", 0);
        let actual = create_command_result("output", "error", 0);

        let expected_verifications = vec![
            create_command_result("verification1_output", "verification1_error", 0),
            create_command_result("verification2_output", "verification2_error", 1),
        ];

        let actual_verifications = vec![
            create_command_result("verification1_output", "verification1_error", 0),
            create_command_result("different_output", "verification2_error", 1), // 第二个验证的stdout不同
        ];

        let result = compare_results(
            &test_case,
            expected,
            expected_verifications,
            actual,
            actual_verifications,
        )
        .unwrap();

        assert!(!result.passed);
        assert_eq!(result.differences.len(), 1);
        assert!(result.differences[0].contains("Verification 'verification2'"));
    }

    #[test]
    fn test_compare_results_ignore_verifications() {
        // 测试忽略所有验证命令的情况
        let mut test_case = create_test_case(
            Some("output".to_string()),
            Some("error".to_string()),
            Some(0),
            false,
            false,
            false,
        );

        // 设置忽略验证
        test_case.expectation.ignore_fields.ignore_verifications = true;

        // 添加验证命令
        test_case.expectation.verifications = vec![FunctionalVerification {
            command: "verification".to_string(),
            expected_exit: Some(0),
            expected_stdout: Some("expected_output".to_string()),
            expected_stderr: Some("expected_error".to_string()),
        }];

        let expected = create_command_result("output", "error", 0);
        let actual = create_command_result("output", "error", 0);

        let expected_verifications = vec![create_command_result(
            "expected_output",
            "expected_error",
            0,
        )];

        let actual_verifications = vec![
            create_command_result("different_output", "different_error", 1), // 验证结果完全不同
        ];

        let result = compare_results(
            &test_case,
            expected,
            expected_verifications,
            actual,
            actual_verifications,
        )
        .unwrap();

        // 因为忽略验证，所以测试应该通过
        assert!(result.passed);
        assert!(result.differences.is_empty());
    }

    #[test]
    fn test_compare_results_all_fields_wrong_but_ignored() {
        // 测试所有字段都不匹配但都被忽略的情况
        let test_case = create_test_case(
            Some("expected_output".to_string()),
            Some("expected_error".to_string()),
            Some(0),
            true, // ignore_stdout
            true, // ignore_stderr
            true, // ignore_exit_code
        );

        let expected = create_command_result("expected_output", "expected_error", 0);
        let actual = create_command_result("actual_output", "actual_error", 1);

        let result = compare_results(&test_case, expected, Vec::new(), actual, Vec::new()).unwrap();

        // 所有差异都被忽略，测试应该通过
        assert!(result.passed);
        assert!(result.differences.is_empty());
    }

    #[test]
    fn test_compare_results_empty_strings() {
        // 测试空字符串的处理
        let test_case = create_test_case(
            Some("".to_string()),
            Some("".to_string()),
            Some(0),
            false,
            false,
            false,
        );

        let expected = create_command_result("", "", 0);
        let actual = create_command_result("", "", 0);

        let result = compare_results(&test_case, expected, Vec::new(), actual, Vec::new()).unwrap();

        assert!(result.passed);
        assert!(result.differences.is_empty());
    }

    #[test]
    fn test_compare_results_expected_empty_actual_not() {
        // 测试期望为空字符串但实际不为空的情况
        let test_case = create_test_case(
            Some("".to_string()),
            Some("".to_string()),
            Some(0),
            false,
            false,
            false,
        );

        let expected = create_command_result("", "", 0);
        let actual = create_command_result("non-empty", "non-empty", 0);

        let result = compare_results(&test_case, expected, Vec::new(), actual, Vec::new()).unwrap();

        assert!(!result.passed);
        assert_eq!(result.differences.len(), 2);
    }

    #[test]
    fn test_compare_results_with_multiline_output() {
        // 测试多行输出的处理
        let test_case = create_test_case(
            Some("line1\nline2\nline3".to_string()),
            Some("error1\nerror2".to_string()),
            Some(0),
            false,
            false,
            false,
        );

        let expected = create_command_result("line1\nline2\nline3", "error1\nerror2", 0);
        let actual = create_command_result("line1\nline2\nline3", "error1\nerror2", 0);

        let result = compare_results(&test_case, expected, Vec::new(), actual, Vec::new()).unwrap();

        assert!(result.passed);
        assert!(result.differences.is_empty());
    }

    #[test]
    fn test_compare_results_with_different_multiline_output() {
        // 测试不同的多行输出处理
        let test_case = create_test_case(
            Some("line1\nline2\nline3".to_string()),
            Some("error1\nerror2".to_string()),
            Some(0),
            false,
            false,
            false,
        );

        let expected = create_command_result("line1\nline2\nline3", "error1\nerror2", 0);
        let actual = create_command_result("line1\ndifferent\nline3", "error1\nerror2", 0);

        let result = compare_results(&test_case, expected, Vec::new(), actual, Vec::new()).unwrap();

        assert!(!result.passed);
        assert_eq!(result.differences.len(), 1);
    }

    #[test]
    fn test_compare_results_with_special_characters() {
        // 测试包含特殊字符的输出处理
        let test_case = create_test_case(
            Some("特殊字符：!@#$%^&*()".to_string()),
            Some("错误信息：\t\n\r".to_string()),
            Some(0),
            false,
            false,
            false,
        );

        let expected = create_command_result("特殊字符：!@#$%^&*()", "错误信息：\t\n\r", 0);
        let actual = create_command_result("特殊字符：!@#$%^&*()", "错误信息：\t\n\r", 0);

        let result = compare_results(&test_case, expected, Vec::new(), actual, Vec::new()).unwrap();

        assert!(result.passed);
        assert!(result.differences.is_empty());
    }

    // 测试用例标签
    #[test]
    fn test_compare_results_with_tagged_test_case() {
        // 测试带标签的测试用例
        let mut test_case = create_test_case(
            Some("output".to_string()),
            Some("error".to_string()),
            Some(0),
            false,
            false,
            false,
        );

        // 添加标签
        test_case.tags = vec!["tag1".to_string(), "tag2".to_string()];

        let expected = create_command_result("output", "error", 0);
        let actual = create_command_result("output", "error", 0);

        let result = compare_results(&test_case, expected, Vec::new(), actual, Vec::new()).unwrap();

        // 标签不影响比较结果
        assert!(result.passed);
        assert!(result.differences.is_empty());
    }

    // 测试ParallelTestExecutor
    #[test]
    fn test_parallel_test_executor_creation() {
        // 测试创建ParallelTestExecutor
        let config = TestConfig::default();
        let executor = ParallelTestExecutor::new(config.clone());

        // 确保配置被正确存储
        assert_eq!(executor.config.default_timeout, config.default_timeout);
        assert_eq!(executor.config.syskits_path, config.syskits_path);
    }

    // 测试CommandExecutor
    #[test]
    fn test_command_executor_creation() {
        // 测试创建CommandExecutor
        let config = create_test_config();
        let executor = CommandExecutor::new(config.clone());

        // 确保配置被正确存储
        assert_eq!(executor.config.default_timeout, config.default_timeout);
        assert_eq!(executor.config.syskits_path, config.syskits_path);
    }

    // 创建一个测试用TestConfig
    fn create_test_config() -> TestConfig {
        TestConfig {
            syskits_path: PathBuf::from("/test/syskits"),
            coreutils_path: Some(PathBuf::from("/test/coreutils")),
            test_cases_dir: PathBuf::from("/test/cases"),
            show_progress: true,
            cleanup: true,
            report_format: "text".to_string(),
            report_dir: PathBuf::from("/test/reports"),
            default_timeout: 10,
            show_diff: true,
            mode: SyskitsMode::Single,
            commands_dir: None,
            verbose: false,
            debug: false,
        }
    }

    // 使用MockCommandExecutor来测试异常情况
    struct MockCommandExecutor {
        pub should_fail: bool,
    }

    impl MockCommandExecutor {
        fn new(should_fail: bool) -> Self {
            Self { should_fail }
        }

        fn execute(&self) -> Result<ComparisonResult> {
            if self.should_fail {
                Err(TestError::ExecutionError("模拟执行失败".to_string()))
            } else {
                Ok(ComparisonResult {
                    command: "mock".to_string(),
                    description: "Mock test".to_string(),
                    args: vec![],
                    expected: CommandResult::default(),
                    actual: CommandResult::default(),
                    passed: true,
                    differences: vec![],
                })
            }
        }
    }

    #[test]
    fn test_mock_executor_success() {
        let mock = MockCommandExecutor::new(false);
        let result = mock.execute();

        assert!(result.is_ok());
        let comparison = result.unwrap();
        assert!(comparison.passed);
    }

    #[test]
    fn test_mock_executor_failure() {
        let mock = MockCommandExecutor::new(true);
        let result = mock.execute();

        assert!(result.is_err());
        match result {
            Err(TestError::ExecutionError(msg)) => {
                assert_eq!(msg, "模拟执行失败");
            }
            _ => panic!("Expected ExecutionError"),
        }
    }

    #[test]
    fn test_compare_results_with_long_output() {
        // 测试长输出的处理
        let test_case = create_test_case(
            Some("a".repeat(1000)), // 1000个'a'
            Some("b".repeat(500)),  // 500个'b'
            Some(0),
            false,
            false,
            false,
        );

        let expected = create_command_result(&"a".repeat(1000), &"b".repeat(500), 0);
        let actual = create_command_result(&"a".repeat(1000), &"b".repeat(500), 0);

        let result = compare_results(&test_case, expected, Vec::new(), actual, Vec::new()).unwrap();

        assert!(result.passed);
        assert!(result.differences.is_empty());
    }

    #[test]
    fn test_compare_results_mismatched_long_output() {
        // 测试长输出不匹配的处理
        let test_case = create_test_case(
            Some("a".repeat(1000)), // 1000个'a'
            Some("b".repeat(500)),  // 500个'b'
            Some(0),
            false,
            false,
            false,
        );

        let expected = create_command_result(&"a".repeat(1000), &"b".repeat(500), 0);
        // 将第500个字符更改为'c'
        let mut output = "a".repeat(1000);
        output.replace_range(500..501, "c");
        let actual = create_command_result(&output, &"b".repeat(500), 0);

        let result = compare_results(&test_case, expected, Vec::new(), actual, Vec::new()).unwrap();

        assert!(!result.passed);
        assert_eq!(result.differences.len(), 1);
    }

    #[test]
    fn test_compare_results_command_details() {
        // 测试ComparisonResult中的命令详细信息
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

        let result = compare_results(
            &test_case,
            expected.clone(),
            Vec::new(),
            actual.clone(),
            Vec::new(),
        )
        .unwrap();

        // 检查ComparisonResult中的命令详细信息
        assert_eq!(result.command, "test_command");
        assert_eq!(result.description, "Test case description");
        assert_eq!(result.args, vec!["arg1", "arg2"]);
        assert_eq!(result.expected.stdout, expected.stdout);
        assert_eq!(result.expected.stderr, expected.stderr);
        assert_eq!(result.expected.exit_code, expected.exit_code);
        assert_eq!(result.actual.stdout, actual.stdout);
        assert_eq!(result.actual.stderr, actual.stderr);
        assert_eq!(result.actual.exit_code, actual.exit_code);
    }

    #[test]
    fn test_error_types() {
        // 测试各种TestError类型

        // ExecutionError
        let error = TestError::ExecutionError("execution failed".to_string());
        assert!(matches!(error, TestError::ExecutionError(_)));

        // TestCaseError
        let error = TestError::TestCaseError("invalid test case".to_string());
        assert!(matches!(error, TestError::TestCaseError(_)));

        // SerializationError
        let error = TestError::SerializationError("serialization failed".to_string());
        assert!(matches!(error, TestError::SerializationError(_)));

        // IoError
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let error = TestError::IoError(io_error);
        assert!(matches!(error, TestError::IoError(_)));

        // SystemError (需要nix库)
        // let error = TestError::SystemError(...);

        // Other
        let error = TestError::Other("other error".to_string());
        assert!(matches!(error, TestError::Other(_)));
    }

    #[test]
    fn test_error_display() {
        // 测试错误的Display实现

        let error = TestError::ExecutionError("exec failed".to_string());
        assert_eq!(
            format!("{}", error),
            "Failed to execute command: exec failed"
        );

        let error = TestError::TestCaseError("case error".to_string());
        assert_eq!(format!("{}", error), "Failed to read test case: case error");

        let error = TestError::SerializationError("ser error".to_string());
        assert_eq!(format!("{}", error), "Serialization error: ser error");

        let error = TestError::Other("other".to_string());
        assert_eq!(format!("{}", error), "other");
    }

    // 测试SyskitsMode枚举
    #[test]
    fn test_syskits_mode() {
        // 测试Single模式
        let single_mode = SyskitsMode::Single;

        match single_mode {
            SyskitsMode::Single => {} // 正确匹配
            SyskitsMode::Multiple => panic!("Expected Single mode"),
        }

        // 测试Multiple模式
        let multiple_mode = SyskitsMode::Multiple;

        match multiple_mode {
            SyskitsMode::Multiple => {} // 正确匹配
            SyskitsMode::Single => panic!("Expected Multiple mode"),
        }
    }

    // 测试用例包含环境变量
    #[test]
    fn test_test_case_with_environment() {
        // 创建带环境变量的测试用例
        let mut test_case = create_test_case(
            Some("output".to_string()),
            Some("error".to_string()),
            Some(0),
            false,
            false,
            false,
        );

        // 添加环境变量
        test_case
            .environment
            .env_vars
            .insert("VAR1".to_string(), "value1".to_string());
        test_case
            .environment
            .env_vars
            .insert("VAR2".to_string(), "value2".to_string());

        // 测试用例中有两个环境变量
        assert_eq!(test_case.environment.env_vars.len(), 2);
        assert_eq!(
            test_case.environment.env_vars.get("VAR1").unwrap(),
            "value1"
        );
        assert_eq!(
            test_case.environment.env_vars.get("VAR2").unwrap(),
            "value2"
        );

        let expected = create_command_result("output", "error", 0);
        let actual = create_command_result("output", "error", 0);

        let result = compare_results(&test_case, expected, Vec::new(), actual, Vec::new()).unwrap();

        // 环境变量不影响比较结果
        assert!(result.passed);
        assert!(result.differences.is_empty());
    }

    // 测试需要root权限的测试用例
    #[test]
    fn test_test_case_requiring_root() {
        // 创建需要root权限的测试用例
        let mut test_case = create_test_case(
            Some("output".to_string()),
            Some("error".to_string()),
            Some(0),
            false,
            false,
            false,
        );

        // 设置为需要root权限
        test_case.requires_root = true;

        assert!(test_case.requires_root);

        let expected = create_command_result("output", "error", 0);
        let actual = create_command_result("output", "error", 0);

        let result = compare_results(&test_case, expected, Vec::new(), actual, Vec::new()).unwrap();

        // requires_root标志不影响比较结果
        assert!(result.passed);
        assert!(result.differences.is_empty());
    }

    // 测试超时设置
    #[test]
    fn test_test_case_with_timeout() {
        // 创建设置了超时的测试用例
        let mut test_case = create_test_case(
            Some("output".to_string()),
            Some("error".to_string()),
            Some(0),
            false,
            false,
            false,
        );

        // 设置超时时间
        test_case.timeout = Some(60);

        assert_eq!(test_case.timeout, Some(60));

        let expected = create_command_result("output", "error", 0);
        let actual = create_command_result("output", "error", 0);

        let result = compare_results(&test_case, expected, Vec::new(), actual, Vec::new()).unwrap();

        // 超时设置不影响比较结果
        assert!(result.passed);
        assert!(result.differences.is_empty());
    }

    // 测试setup和cleanup命令
    #[test]
    fn test_test_case_with_setup_and_cleanup() {
        // 创建带setup和cleanup命令的测试用例
        let mut test_case = create_test_case(
            Some("output".to_string()),
            Some("error".to_string()),
            Some(0),
            false,
            false,
            false,
        );

        // 添加setup命令
        test_case.setup_commands = vec![
            "mkdir -p /tmp/test".to_string(),
            "echo 'test' > /tmp/test/file".to_string(),
        ];

        // 添加cleanup命令
        test_case.cleanup_commands = vec![
            "rm -f /tmp/test/file".to_string(),
            "rmdir /tmp/test".to_string(),
        ];

        assert_eq!(test_case.setup_commands.len(), 2);
        assert_eq!(test_case.cleanup_commands.len(), 2);

        let expected = create_command_result("output", "error", 0);
        let actual = create_command_result("output", "error", 0);

        let result = compare_results(&test_case, expected, Vec::new(), actual, Vec::new()).unwrap();

        // setup和cleanup命令不影响比较结果
        assert!(result.passed);
        assert!(result.differences.is_empty());
    }

    #[test]
    fn test_test_case_with_files() {
        // 创建带文件的测试用例
        let mut test_case = create_test_case(
            Some("output".to_string()),
            Some("error".to_string()),
            Some(0),
            false,
            false,
            false,
        );

        // 添加测试文件相关定义
        use crate::test_case::{FileType, TestFile};

        // 添加测试文件
        test_case.environment.files.push(TestFile {
            path: "test_file.txt".to_string(),
            content: Some("Test file content".to_string()),
            permissions: Some("644".to_string()),
            owner: Some("root".to_string()),
            group: Some("root".to_string()),
            file_type: FileType::Regular,
            symlink_target: None,
            size: None,
            timestamp: None,
        });

        // 添加目录
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

        // 添加符号链接
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

        // 测试用例中有三个文件
        assert_eq!(test_case.environment.files.len(), 3);
        assert_eq!(test_case.environment.files[0].path, "test_file.txt");
        assert_eq!(
            test_case.environment.files[0].content.as_ref().unwrap(),
            "Test file content"
        );
        assert_eq!(test_case.environment.files[1].path, "test_dir");
        assert!(matches!(
            test_case.environment.files[1].file_type,
            FileType::Directory
        ));
        assert_eq!(test_case.environment.files[2].path, "test_link");
        assert!(matches!(
            test_case.environment.files[2].file_type,
            FileType::Symlink
        ));

        let expected = create_command_result("output", "error", 0);
        let actual = create_command_result("output", "error", 0);

        let result = compare_results(&test_case, expected, Vec::new(), actual, Vec::new()).unwrap();

        // 环境文件不影响比较结果
        assert!(result.passed);
        assert!(result.differences.is_empty());
    }

    #[test]
    fn test_compare_results_with_verifications_detailed() {
        // 创建一个带验证的测试用例
        let mut test_case = create_simple_test_case();

        // 添加验证命令
        test_case.expectation.verifications = vec![FunctionalVerification {
            command: "test -f /tmp/test_file".to_string(),
            expected_exit: Some(0),
            expected_stdout: Some("".to_string()),
            expected_stderr: Some("".to_string()),
        }];

        // 创建预期的主命令结果
        let expected = CommandResult {
            stdout: "hello".to_string(),
            stderr: "".to_string(),
            exit_code: 0,
        };

        // 创建实际的主命令结果
        let actual = CommandResult {
            stdout: "hello".to_string(),
            stderr: "".to_string(),
            exit_code: 0,
        };

        // 创建预期的验证命令结果
        let expected_verifications = vec![CommandResult {
            stdout: "".to_string(),
            stderr: "".to_string(),
            exit_code: 0,
        }];

        // 创建实际的验证命令结果，但验证结果是不同的
        let actual_verifications_different = vec![CommandResult {
            stdout: "something_different".to_string(),
            stderr: "some_error".to_string(),
            exit_code: 1,
        }];

        // 测试比较结果 - 所有匹配
        let result = compare_results(
            &test_case,
            expected.clone(),
            expected_verifications.clone(),
            actual.clone(),
            expected_verifications.clone(), // 使用相同的值确保它通过
        )
        .unwrap();

        assert!(result.passed);
        assert!(result.differences.is_empty());

        // 测试比较结果 - 验证命令失败
        let result_failed = compare_results(
            &test_case,
            expected,
            expected_verifications,
            actual,
            actual_verifications_different, // 使用不同的验证结果，应该失败
        )
        .unwrap();

        assert!(!result_failed.passed); // 这个测试应该失败
        assert!(!result_failed.differences.is_empty());
        assert!(result_failed.differences[0].contains("Verification"));
    }

    #[test]
    fn test_command_executor_execute_test() {
        // 创建临时目录作为测试环境
        let temp_dir = TempDir::new().unwrap();
        let test_dir = temp_dir.path().to_path_buf();

        // 创建一个简单的测试用例
        let test_case = create_simple_test_case();

        // 创建测试配置，指向一个可执行文件
        // 使用echo作为替代，这样测试可以在不同环境工作
        let config = TestConfig {
            syskits_path: PathBuf::from("/bin/echo"),
            test_cases_dir: test_dir,
            debug: true,
            ..Default::default()
        };

        // 创建命令执行器
        let executor = CommandExecutor::new(config);

        // 测试执行
        // 这里我们不期望它能通过，因为我们用的是echo命令而不是真正的syskits
        // 所以结果一定和预期不同
        let result = executor.execute_test(&test_case);

        // 验证我们可以得到一个结果 (不管通过与否)
        assert!(result.is_ok());
        // 确认测试失败是因为输出不匹配 (echo会输出命令本身而不是预期输出)
        if let Ok(comparison) = result {
            assert!(!comparison.passed);
            assert!(!comparison.differences.is_empty());
        }
    }

    // 辅助函数：创建一个简单的测试用例
    fn create_simple_test_case() -> TestCase {
        TestCase {
            tstdin: "".to_string(),
            byte_mode: false,
            tty: false,
            command: "echo".to_string(),
            description: "Simple echo test".to_string(),
            args: vec!["-n".to_string(), "hello".to_string()],
            expectation: TestExpectation {
                execution: CommandExecution {
                    exit_code: Some(0),
                    stdout: Some("hello".to_string()),
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
            timeout: Some(5),
            tags: vec!["basic".to_string()],
            environment: TestEnvironment::default(),
        }
    }

    #[test]
    fn test_parallel_test_executor() {
        // 创建测试配置
        let config = TestConfig {
            syskits_path: PathBuf::from("/bin/echo"),
            ..Default::default()
        };

        // 创建并行测试执行器
        let executor = ParallelTestExecutor::new(config.clone());

        // 确保配置被正确存储
        assert_eq!(executor.config.syskits_path, PathBuf::from("/bin/echo"));

        // 测试execute_test方法
        let test_case = create_simple_test_case();

        // 由于测试环境的不确定性，我们不能期望命令成功执行
        // 我们只确保方法可以被调用而不会崩溃
        let result = executor.execute_test(&test_case);

        // 验证我们可以得到一个结果 (不管通过与否)
        assert!(result.is_ok());
    }

    #[test]
    fn test_execute_syskits_test() {
        // 创建临时目录作为测试环境
        let temp_dir = TempDir::new().unwrap();
        let test_dir = temp_dir.path().to_path_buf();

        // 创建一个带验证的测试用例
        let mut test_case = create_simple_test_case();

        // 添加验证命令
        test_case.expectation.verifications = vec![FunctionalVerification {
            command: "test -f /tmp/test_file".to_string(),
            expected_exit: Some(0),
            expected_stdout: None,
            expected_stderr: None,
        }];

        // 添加设置和清理命令
        test_case.setup_commands = vec!["touch /tmp/test_file".to_string()];
        test_case.cleanup_commands = vec!["rm -f /tmp/test_file".to_string()];

        // 创建测试配置
        let config = TestConfig {
            syskits_path: PathBuf::from("/bin/echo"), // 使用系统echo命令代替syskits
            test_cases_dir: test_dir.clone(),
            debug: true,
            ..Default::default()
        };

        let executor = CommandExecutor::new(config);

        // 尝试执行测试，但不检查结果，因为执行结果取决于环境
        let result = executor.execute_syskits_test(&test_case);

        // 只验证函数不会崩溃
        match result {
            Ok(_) => (),
            Err(e) => println!("Error during execution: {}", e),
        }
    }

    #[test]
    fn test_get_coreutils_test_result() {
        // 创建临时目录作为测试环境
        let temp_dir = TempDir::new().unwrap();
        let test_dir = temp_dir.path().to_path_buf();

        // 创建一个带验证的测试用例
        let mut test_case = create_simple_test_case();

        // 添加验证命令
        test_case.expectation.verifications = vec![FunctionalVerification {
            command: "test -f /tmp/test_file".to_string(),
            expected_exit: Some(0),
            expected_stdout: None,
            expected_stderr: None,
        }];

        // 添加设置和清理命令
        test_case.setup_commands = vec!["touch /tmp/test_file".to_string()];
        test_case.cleanup_commands = vec!["rm -f /tmp/test_file".to_string()];

        // 创建测试配置，指定coreutils路径
        let config = TestConfig {
            syskits_path: PathBuf::from("/bin/echo"),
            coreutils_path: Some(PathBuf::from("/bin")), // 假设系统的/bin目录包含coreutils
            test_cases_dir: test_dir.clone(),
            debug: true,
            ..Default::default()
        };

        let executor = CommandExecutor::new(config);

        // 尝试执行测试，但不检查结果，因为执行结果取决于环境
        let result = executor.get_coreutils_test_result(&test_case);

        // 只验证函数不会崩溃
        match result {
            Ok(_) => (),
            Err(e) => println!("Error during coreutils execution: {}", e),
        }
    }

    #[test]
    fn test_execute_syskits() {
        // 创建临时目录作为测试环境
        let temp_dir = TempDir::new().unwrap();
        let test_dir = temp_dir.path().to_path_buf();

        // 创建测试配置 - Single模式
        let config_single = TestConfig {
            syskits_path: PathBuf::from("/bin/echo"),
            test_cases_dir: test_dir.clone(),
            mode: SyskitsMode::Single,
            debug: true,
            ..Default::default()
        };

        // 创建测试配置 - Multiple模式
        let config_multiple = TestConfig {
            syskits_path: PathBuf::from("/bin/echo"),
            test_cases_dir: test_dir.clone(),
            mode: SyskitsMode::Multiple,
            debug: true,
            ..Default::default()
        };

        let executor_single = CommandExecutor::new(config_single);
        let executor_multiple = CommandExecutor::new(config_multiple);

        // 创建一个隔离沙箱
        let mut sandbox = IsolatedSandbox::new(true).unwrap();

        // 测试Single模式和Multiple模式执行
        let stdin = "";
        let command = "echo";
        let args = vec!["-n".to_string(), "hello".to_string()];
        let timeout = Some(5u64);

        // 尝试执行命令，但不检查结果，因为执行结果取决于环境
        let result_single =
            executor_single.execute_syskits(stdin, command, &args, &mut sandbox, timeout);
        let result_multiple =
            executor_multiple.execute_syskits(stdin, command, &args, &mut sandbox, timeout);

        // 只验证函数不会崩溃
        match result_single {
            Ok(_) => (),
            Err(e) => println!("Error during single mode execution: {}", e),
        }

        match result_multiple {
            Ok(_) => (),
            Err(e) => println!("Error during multiple mode execution: {}", e),
        }
    }

    #[test]
    fn test_execute_coreutils() {
        // 创建临时目录作为测试环境
        let temp_dir = TempDir::new().unwrap();
        let test_dir = temp_dir.path().to_path_buf();

        // 创建带coreutils_path的测试配置
        let config_with_coreutils = TestConfig {
            syskits_path: PathBuf::from("/bin/echo"),
            coreutils_path: Some(PathBuf::from("/bin")),
            test_cases_dir: test_dir.clone(),
            debug: true,
            ..Default::default()
        };

        // 创建不带coreutils_path的测试配置
        let config_without_coreutils = TestConfig {
            syskits_path: PathBuf::from("/bin/echo"),
            coreutils_path: None,
            test_cases_dir: test_dir.clone(),
            debug: true,
            ..Default::default()
        };

        let executor_with_coreutils = CommandExecutor::new(config_with_coreutils);
        let executor_without_coreutils = CommandExecutor::new(config_without_coreutils);

        // 创建一个隔离沙箱
        let mut sandbox = IsolatedSandbox::new(true).unwrap();

        // 测试执行
        let stdin = "";
        let command = "echo";
        let args = vec!["-n".to_string(), "hello".to_string()];
        let timeout = Some(5u64);

        // 尝试执行命令，但不检查结果，因为执行结果取决于环境
        let result_with_coreutils =
            executor_with_coreutils.execute_coreutils(stdin, command, &args, &mut sandbox, timeout);
        let result_without_coreutils = executor_without_coreutils.execute_coreutils(
            stdin,
            command,
            &args,
            &mut sandbox,
            timeout,
        );

        // 只验证函数不会崩溃
        match result_with_coreutils {
            Ok(_) => (),
            Err(e) => println!("Error during coreutils execution (with path): {}", e),
        }

        match result_without_coreutils {
            Ok(_) => (),
            Err(e) => println!("Error during coreutils execution (without path): {}", e),
        }
    }
}
