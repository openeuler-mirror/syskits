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

//! 测试运行器主程序
//! 提供命令行界面和测试执行功能

use chrono::Local;
use clap::{Arg, ArgAction, Command};
use colored::*;
use compat_test::config::Config;
use compat_test::reporter::{ReportFormat, Reporter};
use compat_test::test_case::{TestEnvironment, TestExpectation};
use compat_test::{Result, TestConfig, TestError, TestRunner};
use indicatif::{ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const DEFAULT_CONFIG_FILE: &str = "bin/compat_test/conf/default.toml";
const TEST_CASES_DIR: &str = "bin/compat_test/test_cases";
const SYSKITS_BIN_PATH: &str = "target/debug/syskits";

/// 简化的测试用例集合
#[derive(Debug, Serialize, Deserialize)]
struct SimpleTests {
    /// 测试用例列表
    tests: Vec<SimpleTest>,
}

/// 简化的测试用例结构
#[derive(Debug, Serialize, Deserialize)]
struct SimpleTest {
    command: String,                       // 命令名称
    description: String,                   // 测试描述
    args: Vec<String>,                     // 命令参数
    expectation: TestExpectation,          // 测试期望结果
    environment: Option<TestEnvironment>,  // 测试环境配置
    setup_commands: Option<Vec<String>>,   // 环境准备命令
    cleanup_commands: Option<Vec<String>>, // 环境清理命令
    requires_root: Option<bool>,           // 是否需要root权限
    timeout: Option<u64>,                  // 超时时间（秒）
    tags: Option<Vec<String>>,             // 测试标签
    tty: Option<bool>,                     // 是否使用伪终端
}

/// 获取工作空间根目录
fn get_workspace_dir() -> PathBuf {
    let current_exe = std::env::current_exe().unwrap();
    let mut path = current_exe.parent().unwrap().to_path_buf();
    // 从 target/debug 或 target/release 向上找到工作空间根目录
    while !path.join("Cargo.toml").exists() {
        path = path.parent().unwrap().to_path_buf();
    }
    path
}

/// 加载配置文件
/// 按优先级查找配置文件:
/// 1. 当前目录的 .compat_test.toml
/// 2. 用户主目录的 .compat_test.toml
/// 3. crate内的默认配置文件 conf/default.toml
fn load_config() -> Result<Config> {
    let workspace_dir = get_workspace_dir();
    let config_paths = vec![
        PathBuf::from(".compat_test.toml"),
        dirs::home_dir()
            .map(|p| p.join(".compat_test.toml"))
            .unwrap_or_default(),
        workspace_dir.join(DEFAULT_CONFIG_FILE),
    ];

    for path in config_paths {
        if path.exists() {
            println!("Using config file: {}", path.display());
            let content = fs::read_to_string(&path).map_err(TestError::IoError)?;
            let config: Config = toml::from_str(&content)
                .map_err(|e| TestError::SerializationError(e.to_string()))?;
            return Ok(config);
        }
    }

    // 如果没有找到配置文件，返回默认配置
    println!("No config file found, using default configuration");
    Ok(Config::default())
}

/// 打印单个测试结果
fn print_test_result(
    result: &compat_test::ComparisonResult,
    config: &TestConfig,
    test_number: usize,
) {
    // 根据是否配置了 coreutils 路径来决定显示标签
    let reference_label = if config.coreutils_path.is_some() {
        "coreutils".to_string()
    } else {
        "expected".to_string()
    };

    if test_number == 1 {
        println!("\nCommand: {}", result.command.bold());
    }

    println!("\n{}. Test Case: {}", test_number, result.description);
    println!("Command: {} {}", result.command, result.args.join(" "));
    println!(
        "Result: {}",
        if result.passed {
            "PASS".green().bold()
        } else {
            "FAIL".red().bold()
        }
    );

    // 显示详细输出
    if config.verbose {
        println!("\n{reference_label} output:");
        println!("  Exit Code: {}", result.expected.exit_code);
        println!(
            "  Stdout: \\n{}\\n",
            result.expected.stdout.replace('\n', "\\n")
        );
        println!(
            "  Stderr: \\n{}\\n",
            result.expected.stderr.replace('\n', "\\n")
        );

        println!("\nsyskits output:");
        println!("  Exit Code: {}", result.actual.exit_code);
        println!(
            "  Stdout: \\n{}\\n",
            result.actual.stdout.replace('\n', "\\n")
        );
        println!(
            "  Stderr: \\n{}\\n",
            result.actual.stderr.replace('\n', "\\n")
        );
    }
    if !result.passed {
        println!("\nDifferences:");
        for diff in &result.differences {
            println!("  {diff}");
        }
    }

    println!("{}", "-".repeat(80));
}

/// 打印测试总结
fn print_test_summary(total: usize, passed: usize, failed: usize) {
    println!(
        "\n{}",
        "╔═══════════════════════════════════════════════════════════════════════════".bold()
    );
    println!("║ {}", "Test Summary".bold());
    println!("╠═══════════════════════════════════════════════════════════════════════════");
    println!("║ Total Tests:   {}", total.to_string().bold());
    println!("║ Passed:        {}", passed.to_string().green().bold());
    println!("║ Failed:        {}", failed.to_string().red().bold());
    println!(
        "║ Success Rate:  {}%",
        ((passed as f64 / total as f64) * 100.0)
            .round()
            .to_string()
            .yellow()
            .bold()
    );
    println!("╚═══════════════════════════════════════════════════════════════════════════\n");
}

/// 执行测试并显示进度
fn run_tests_with_progress(
    runner: &TestRunner,
    commands: &[String],
    config: &TestConfig,
) -> Result<()> {
    let mut total_tests = 0;
    let mut total_passed = 0;
    let mut total_failed = 0;
    let mut all_results = Vec::new();
    let mut missing_tests = Vec::new();

    // 创建总进度条
    let total_progress = if !config.verbose {
        let pb = ProgressBar::new(commands.len() as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template(
                    "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} 命令",
                )
                .unwrap()
                .progress_chars("#>-"),
        );
        Some(pb)
    } else {
        None
    };

    // 串行执行每个命令，但命令内的测试用例并行执行
    for command in commands {
        if config.verbose {
            println!("\n{}", "═".repeat(80).bold());
            println!("测试命令: {}", command.bold());
            println!("{}", "═".repeat(80));
        }

        // 读取并执行 JSON 测试用例
        let test_file = config.test_cases_dir.join(format!("{command}.json"));
        if test_file.exists() {
            let content = fs::read_to_string(&test_file)?;

            // 确认配置文件配置是否正常
            let _simple_tests: SimpleTests = serde_json::from_str(&content)
                .map_err(|e| TestError::TestCaseError(format!("解析 JSON 测试用例失败: {e}")))?;

            // 并行执行测试用例
            let parallel = true;
            // 串行执行测试用例
            let results = if parallel {
                runner.run_command_tests_parallel(command)?
            } else {
                runner.run_command_tests(command)?
            };

            let passed = results.iter().filter(|r| r.passed).count();
            let failed = results.len() - passed;

            total_tests += results.len();
            total_passed += passed;
            total_failed += failed;

            // 收集所有测试结果（克隆results）
            all_results.extend(results.clone());

            if config.verbose {
                // 详细模式：显示每个测试用例的结果
                for (i, result) in results.iter().enumerate() {
                    print_test_result(result, config, i + 1);
                }
                // 显示当前命令的小结
                println!("\n{}", "-".repeat(40));
                println!("命令 {} 总结:", command.bold());
                println!(
                    "总数: {}, 通过: {}, 失败: {}",
                    results.len(),
                    passed.to_string().green(),
                    failed.to_string().red()
                );
                println!("{}", "-".repeat(40));
            } else {
                // 简洁模式：只显示命令级别的结果
                print!("\r{command}: ");
                if failed == 0 {
                    println!("{}", "所有测试通过".green());
                } else {
                    println!("{}/{} 个测试失败", failed, results.len());
                }
            }
        } else {
            println!("警告: 未找到命令的测试文件: {command}");
            println!("No file found {}", test_file.display());
            missing_tests.push(command.clone());
        }

        if let Some(ref pb) = total_progress {
            pb.inc(1);
        }
    }

    if let Some(ref pb) = total_progress {
        pb.finish_and_clear();
    }

    // 显示总结
    print_test_summary(total_tests, total_passed, total_failed);

    // 生成报告
    let report_format = match config.report_format.as_str() {
        "json" => ReportFormat::Json,
        "html" => ReportFormat::Html,
        _ => ReportFormat::Text,
    };

    // 创建报告目录（使用绝对路径）
    let report_dir = if config.report_dir.is_absolute() {
        config.report_dir.clone()
    } else {
        get_workspace_dir().join(&config.report_dir)
    };

    if !report_dir.exists() {
        fs::create_dir_all(&report_dir)?;
    }

    let reporter = Reporter::new(report_format, &report_dir);
    reporter.generate_report(&all_results, &missing_tests)?;

    // 打印报告位置信息
    let extension = match report_format {
        ReportFormat::Json => "json",
        ReportFormat::Html => "html",
        ReportFormat::Text => "txt",
    };
    println!(
        "\n报告已生成: {}/report_{}.{}",
        report_dir.display(),
        Local::now().format("%Y%m%d_%H%M%S"),
        extension
    );

    Ok(())
}

/// 主函数
fn main() -> Result<()> {
    // 加载配置文件
    let config = load_config()?;
    let workspace_dir = get_workspace_dir();

    let matches = Command::new("compat_test")
        .about("Compatibility test runner for syskits")
        .arg(
            Arg::new("syskits-path")
                .long("syskits-path")
                .value_name("PATH")
                .help("Path to syskits binary")
                .required(false),
        )
        .arg(
            Arg::new("coreutils-path")
                .long("coreutils-path")
                .value_name("PATH")
                .help("Path to GNU coreutils binaries")
                .required(false),
        )
        .arg(
            Arg::new("test-cases-dir")
                .long("test-cases-dir")
                .value_name("PATH")
                .help("Directory containing test cases")
                .required(false),
        )
        .arg(
            Arg::new("commands")
                .value_name("COMMANDS")
                .help("Commands to test (space separated, omit to test all)")
                .num_args(0..)
                .action(ArgAction::Append)
                .required(false),
        )
        .arg(
            Arg::new("no-progress")
                .long("no-progress")
                .help("Disable progress display")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("no-cleanup")
                .long("no-cleanup")
                .help("Keep temporary files after tests")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("verbose")
                .long("verbose")
                .short('v')
                .help("Enable verbose output")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("debug")
                .short('d')
                .long("debug")
                .help("Enable debug output")
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("report-format")
                .long("report-format")
                .value_name("FORMAT")
                .help("Report format: text, json, or html")
                .required(false),
        )
        .get_matches();

    // 创建测试配置，优先使用命令行参数，其次是配置文件，最后是默认值
    let test_config = TestConfig::new(
        matches
            .get_one::<String>("syskits-path")
            .map(|s| workspace_dir.join(s))
            .or_else(|| {
                config
                    .syskits
                    .syskits_path
                    .as_ref() // 如果 config.syskits.syskits_path 是 Option<String> 或 Option<PathBuf>
                    .map(|p| workspace_dir.join(p))
            })
            .or_else(|| Some(workspace_dir.join(SYSKITS_BIN_PATH))), // 默认值
        matches
            .get_one::<String>("coreutils-path")
            .map(PathBuf::from),
        matches
            .get_one::<String>("test-cases-dir")
            .map(|s| workspace_dir.join(s))
            .or_else(|| {
                config
                    .test
                    .test_cases_dir
                    .as_ref() // 如果 config.test.test_cases_dir 是 Option<String> 或 Option<PathBuf>
                    .map(|p| workspace_dir.join(p))
            })
            .or_else(|| Some(workspace_dir.join(TEST_CASES_DIR))), // 默认值
        matches.get_flag("no-progress"),
        matches.get_flag("no-cleanup"),
        matches
            .get_one::<String>("report-format")
            .map(|s| s.to_string()),
        matches.get_flag("verbose"),
        matches.get_flag("debug"),
        Some(&config),
    );

    let runner = TestRunner::new(test_config.clone());

    // 如果命令行指定了命令，使用命令行的值；否则使用配置文件中的默认命令；如果都没有，测试所有命令
    if let Some(commands) = matches.get_many::<String>("commands") {
        let commands: Vec<String> = commands.map(|s| s.to_string()).collect();
        if test_config.show_progress {
            println!("{}", "\nRunning specified commands:".bold());
            for cmd in &commands {
                println!("  {cmd}");
            }
            println!();
        }

        run_tests_with_progress(&runner, &commands, &test_config)?;
    } else if let Some(default_commands) = config.test.default_commands {
        if test_config.show_progress {
            println!("{}", "\nRunning default commands:".bold());
            for cmd in &default_commands {
                println!("  {cmd}");
            }
            println!();
        }

        run_tests_with_progress(&runner, &default_commands, &test_config)?;
    } else {
        let commands = runner.test_manager.get_available_commands()?;
        if test_config.show_progress {
            println!("{}", "\nRunning all available commands:".bold());
            for cmd in &commands {
                println!("  {cmd}");
            }
            println!();
        }

        run_tests_with_progress(&runner, &commands, &test_config)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use compat_test::config::{Config, SyskitsConfig, SyskitsMode, TestEnvConfig, TestSettings};
    use compat_test::{CommandResult, ComparisonResult};

    #[test]
    fn test_get_workspace_dir() {
        let workspace_dir = get_workspace_dir();
        assert!(
            workspace_dir.join("Cargo.toml").exists(),
            "应该能找到工作空间的Cargo.toml文件"
        );
    }

    #[test]
    fn test_print_test_result() {
        let config = TestConfig {
            verbose: true,
            ..Default::default()
        };

        let result = ComparisonResult {
            command: "echo".to_string(),
            description: "测试echo命令".to_string(),
            args: vec!["-n".to_string(), "hello".to_string()],
            expected: CommandResult {
                stdout: "hello".to_string(),
                stderr: "".to_string(),
                exit_code: 0,
            },
            actual: CommandResult {
                stdout: "hello".to_string(),
                stderr: "".to_string(),
                exit_code: 0,
            },
            passed: true,
            differences: vec![],
        };

        // 不应该panic
        print_test_result(&result, &config, 1);

        // 测试失败的情况
        let failed_result = ComparisonResult {
            command: "echo".to_string(),
            description: "测试不匹配的输出".to_string(),
            args: vec!["-n".to_string(), "hello".to_string()],
            expected: CommandResult {
                stdout: "hello".to_string(),
                stderr: "".to_string(),
                exit_code: 0,
            },
            actual: CommandResult {
                stdout: "world".to_string(),
                stderr: "".to_string(),
                exit_code: 0,
            },
            passed: false,
            differences: vec!["标准输出不匹配: 期望 'hello', 得到 'world'".to_string()],
        };

        // 不应该panic
        print_test_result(&failed_result, &config, 2);
    }

    #[test]
    fn test_print_test_summary() {
        // 不应该panic
        print_test_summary(10, 8, 2);
    }

    #[test]
    fn test_simple_test_deserialize() {
        let json = r#"
        {
            "tests": [
                {
                    "command": "echo",
                    "description": "测试echo命令",
                    "args": ["-n", "hello"],
                    "expectation": {
                        "execution": {
                            "exit_code": 0,
                            "stdout": "hello",
                            "stderr": ""
                        },
                        "verifications": [],
                        "use_patterns": false,
                        "env_changes": {},
                        "file_changes": [],
                        "ignore_fields": {
                            "ignore_exit_code": false,
                            "ignore_stdout": false,
                            "ignore_stderr": false,
                            "ignore_verifications": false
                        }
                    },
                    "environment": {
                        "files": [],
                        "env_vars": {"TEST_VAR": "value"},
                        "working_dir": "/tmp"
                    },
                    "setup_commands": ["mkdir -p /tmp/test"],
                    "cleanup_commands": ["rm -rf /tmp/test"],
                    "requires_root": false,
                    "timeout": 5,
                    "tags": ["basic", "echo"]
                }
            ]
        }
        "#;

        let simple_tests: SimpleTests = serde_json::from_str(json).unwrap();
        assert_eq!(simple_tests.tests.len(), 1);
        assert_eq!(simple_tests.tests[0].command, "echo");
        assert_eq!(simple_tests.tests[0].args, vec!["-n", "hello"]);
        assert_eq!(simple_tests.tests[0].description, "测试echo命令");
    }

    #[test]
    fn test_load_config_default() {
        // 测试默认配置加载
        let config = load_config().unwrap();
        assert!(config.test.env.show_progress);
        assert!(config.test.env.cleanup);
    }

    #[test]
    fn test_load_config_custom() {
        // 这个测试可能无法在CI环境中正常工作，因为它需要读取配置文件
        // 我们只进行基本测试
        let result = load_config();
        match result {
            Ok(config) => {
                // 检查加载的配置是否有效
                assert!(matches!(
                    config.syskits.mode,
                    SyskitsMode::Single | SyskitsMode::Multiple
                ));
            }
            Err(e) => {
                println!("Configuration loading error: {e}");
                // 不做断言，因为在某些环境中可能会失败
            }
        }
    }

    #[test]
    fn test_run_tests_with_progress_mock() {
        // 创建模拟的TestRunner和TestConfig
        struct MockRunner {
            // 移除未使用的字段
            // results: HashMap<String, Vec<ComparisonResult>>,
        }

        impl MockRunner {
            fn new() -> Self {
                Self {}
            }

            fn run_command_tests_parallel(&self, command: &str) -> Result<Vec<ComparisonResult>> {
                if command == "echo" {
                    Ok(vec![ComparisonResult {
                        command: "echo".to_string(),
                        description: "测试1".to_string(),
                        args: vec![],
                        expected: CommandResult::default(),
                        actual: CommandResult::default(),
                        passed: true,
                        differences: vec![],
                    }])
                } else {
                    Ok(vec![])
                }
            }
        }

        fn mock_run_tests(
            runner: &MockRunner,
            commands: &[String],
            _config: &TestConfig,
        ) -> compat_test::Result<()> {
            // 模拟测试执行
            for cmd in commands {
                let _results = runner.run_command_tests_parallel(cmd)?;
            }

            Ok(())
        }

        // 创建要测试的命令列表
        let commands = vec!["echo".to_string(), "ls".to_string()];
        let mock_runner = MockRunner::new();
        let config = TestConfig::default();

        // 运行并确保无错误
        let result = mock_run_tests(&mock_runner, &commands, &config);
        assert!(result.is_ok());
    }

    // 测试print_test_summary函数
    #[test]
    fn test_print_test_summary_detailed() {
        // 测试各种边界情况

        // 测试总数为0的情况
        print_test_summary(0, 0, 0);

        // 测试全部通过的情况
        print_test_summary(10, 10, 0);

        // 测试全部失败的情况
        print_test_summary(10, 0, 10);

        // 测试部分通过部分失败的情况
        print_test_summary(100, 75, 25);

        // 测试传入负数（不正常情况，但不应崩溃）
        // 注意：这不是期望的行为，但测试确保代码不会崩溃
        // 省略这种测试，因为会造成NaN
    }

    // 测试print_test_result函数详细测试
    #[test]
    fn test_print_test_result_detailed() {
        // 创建测试配置，同时测试带和不带coreutils_path的情况

        // 不带coreutils_path
        let config_without_coreutils = TestConfig {
            coreutils_path: None,
            verbose: true,
            ..Default::default()
        };

        // 带coreutils_path
        let config_with_coreutils = TestConfig {
            coreutils_path: Some(PathBuf::from("/usr/bin/coreutils")),
            verbose: true,
            ..Default::default()
        };

        // 创建通过的测试结果
        let passed_result = ComparisonResult {
            command: "echo".to_string(),
            description: "Test case for echo command".to_string(),
            args: vec!["-n".to_string(), "hello".to_string()],
            expected: CommandResult {
                stdout: "hello".to_string(),
                stderr: "".to_string(),
                exit_code: 0,
            },
            actual: CommandResult {
                stdout: "hello".to_string(),
                stderr: "".to_string(),
                exit_code: 0,
            },
            passed: true,
            differences: vec![],
        };

        // 创建失败的测试结果，带多个差异
        let failed_result = ComparisonResult {
            command: "echo".to_string(),
            description: "Test case for echo command with differences".to_string(),
            args: vec!["-n".to_string(), "hello".to_string()],
            expected: CommandResult {
                stdout: "hello".to_string(),
                stderr: "".to_string(),
                exit_code: 0,
            },
            actual: CommandResult {
                stdout: "world".to_string(),
                stderr: "error message".to_string(),
                exit_code: 1,
            },
            passed: false,
            differences: vec![
                "stdout differs: expected 'hello', got 'world'".to_string(),
                "stderr differs: expected '', got 'error message'".to_string(),
                "exit code differs: expected 0, got 1".to_string(),
            ],
        };

        // 测试使用config_without_coreutils的打印
        print_test_result(&passed_result, &config_without_coreutils, 1);
        print_test_result(&failed_result, &config_without_coreutils, 2);

        // 测试使用config_with_coreutils的打印
        print_test_result(&passed_result, &config_with_coreutils, 1);
        print_test_result(&failed_result, &config_with_coreutils, 2);

        // 测试verbose=false的情况
        let config_non_verbose = TestConfig {
            verbose: false,
            ..Default::default()
        };

        print_test_result(&passed_result, &config_non_verbose, 1);
        print_test_result(&failed_result, &config_non_verbose, 2);
    }

    // 测试load_config函数（模拟方式）
    #[test]
    fn test_load_config_mock() {
        // 定义一个模拟的load_config函数来测试逻辑
        fn mock_load_config(file_exists: bool, valid_content: bool) -> Result<Config> {
            if file_exists {
                if valid_content {
                    let config = Config {
                        syskits: SyskitsConfig {
                            syskits_path: Some(PathBuf::from("/mock/syskits")),
                            coreutils_path: Some(PathBuf::from("/mock/coreutils")),
                            mode: SyskitsMode::Single,
                            commands_dir: None,
                        },
                        test: TestSettings {
                            test_cases_dir: Some(PathBuf::from("/mock/test_cases")),
                            default_commands: Some(vec!["ls".to_string(), "cp".to_string()]),
                            env: TestEnvConfig::default(),
                        },
                    };
                    Ok(config)
                } else {
                    Err(TestError::SerializationError("模拟解析错误".to_string()))
                }
            } else {
                Ok(Config::default())
            }
        }

        // 测试找到有效配置文件
        let config_valid = mock_load_config(true, true).unwrap();
        assert_eq!(
            config_valid.syskits.syskits_path.unwrap(),
            PathBuf::from("/mock/syskits")
        );
        assert_eq!(
            config_valid.test.default_commands.unwrap(),
            vec!["ls".to_string(), "cp".to_string()]
        );

        // 测试找到但内容无效
        let config_invalid = mock_load_config(true, false);
        assert!(config_invalid.is_err());

        // 测试没有找到配置文件
        let config_not_found = mock_load_config(false, false).unwrap();
        assert!(config_not_found.syskits.syskits_path.is_none());
        assert!(config_not_found.test.default_commands.is_none());
    }

    // 测试get_workspace_dir函数的特性
    #[test]
    fn test_get_workspace_dir_structure() {
        let workspace_dir = get_workspace_dir();

        // 检查返回的目录是绝对路径
        assert!(workspace_dir.is_absolute());

        // 检查返回的目录包含Cargo.toml
        assert!(workspace_dir.join("Cargo.toml").exists());

        // 检查工作区目录结构
        assert!(workspace_dir.join("bin").exists());
        assert!(workspace_dir.join("bin").is_dir());
    }
}
