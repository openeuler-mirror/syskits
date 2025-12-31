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
        workspace_dir.join("src/compat_test/conf/default.toml"),
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
        println!("\n{} output:", reference_label);
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
            println!("  {}", diff);
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
        let test_file = config.test_cases_dir.join(format!("{}.json", command));
        if test_file.exists() {
            let content = fs::read_to_string(&test_file)?;

            // 确认配置文件配置是否正常
            let _simple_tests: SimpleTests = serde_json::from_str(&content)
                .map_err(|e| TestError::TestCaseError(format!("解析 JSON 测试用例失败: {}", e)))?;

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
                print!("\r{}: ", command);
                if failed == 0 {
                    println!("{}", "所有测试通过".green());
                } else {
                    println!("{}/{} 个测试失败", failed, results.len());
                }
            }
        } else {
            println!("警告: 未找到命令的测试文件: {}", command);
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
            .or_else(|| Some(workspace_dir.join("target/debug/syskits"))),
        matches
            .get_one::<String>("coreutils-path")
            .map(PathBuf::from),
        matches
            .get_one::<String>("test-cases-dir")
            .map(|s| workspace_dir.join(s))
            .or_else(|| Some(workspace_dir.join("src/compat_test/test_cases"))),
        matches.get_flag("no-progress"),
        matches.get_flag("no-cleanup"),
        matches
            .get_one::<String>("report-format")
            .map(|s| s.to_string()),
        matches.get_flag("verbose"),
        Some(&config),
    );

    let runner = TestRunner::new(test_config.clone());

    // 如果命令行指定了命令，使用命令行的值；否则使用配置文件中的默认命令；如果都没有，测试所有命令
    if let Some(commands) = matches.get_many::<String>("commands") {
        let commands: Vec<String> = commands.map(|s| s.to_string()).collect();
        if test_config.show_progress {
            println!("{}", "\nRunning specified commands:".bold());
            for cmd in &commands {
                println!("  {}", cmd);
            }
            println!();
        }

        run_tests_with_progress(&runner, &commands, &test_config)?;
    } else if let Some(default_commands) = config.test.default_commands {
        if test_config.show_progress {
            println!("{}", "\nRunning default commands:".bold());
            for cmd in &default_commands {
                println!("  {}", cmd);
            }
            println!();
        }

        run_tests_with_progress(&runner, &default_commands, &test_config)?;
    } else {
        let commands = runner.test_manager.get_available_commands()?;
        if test_config.show_progress {
            println!("{}", "\nRunning all available commands:".bold());
            for cmd in &commands {
                println!("  {}", cmd);
            }
            println!();
        }

        run_tests_with_progress(&runner, &commands, &test_config)?;
    }

    Ok(())
}
