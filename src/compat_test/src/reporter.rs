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

//! 测试报告生成模块
//! 提供测试结果的格式化输出，支持文本、JSON和HTML格式

use crate::{ComparisonResult, Result, TestError};
use chrono::Local;
use std::fs::File;
use std::io::Write;
use std::path::Path;

/// 报告输出格式
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ReportFormat {
    /// 文本格式
    Text,
    /// JSON格式
    Json,
    /// HTML格式
    Html,
}

/// 测试报告生成器
pub struct Reporter {
    /// 报告格式
    format: ReportFormat,
    /// 输出目录
    output_dir: std::path::PathBuf,
}

impl Reporter {
    /// 创建新的报告生成器
    pub fn new<P: AsRef<Path>>(format: ReportFormat, output_dir: P) -> Self {
        Self {
            format,
            output_dir: output_dir.as_ref().to_path_buf(),
        }
    }

    /// 为一组测试结果生成报告
    pub fn generate_report(
        &self,
        results: &[ComparisonResult],
        missing_tests: &[String],
    ) -> Result<()> {
        match self.format {
            ReportFormat::Text => self.generate_text_report(results, missing_tests),
            ReportFormat::Json => self.generate_json_report(results, missing_tests),
            ReportFormat::Html => self.generate_html_report(results, missing_tests),
        }
    }

    /// 生成文本格式报告
    fn generate_text_report(
        &self,
        results: &[ComparisonResult],
        missing_tests: &[String],
    ) -> Result<()> {
        let timestamp = Local::now().format("%Y%m%d_%H%M%S");
        let path = self.output_dir.join(format!("report_{}.txt", timestamp));
        let mut file = File::create(path)?;

        // 1. 总体报告头部
        writeln!(file, "═══════════════════════════════════════════════════")?;
        writeln!(file, "              兼容性测试报告")?;
        writeln!(file, "═══════════════════════════════════════════════════")?;
        writeln!(
            file,
            "生成时间: {}",
            Local::now().format("%Y-%m-%d %H:%M:%S")
        )?;
        writeln!(file)?;

        // 2. 总体测试结果统计
        writeln!(file, "【测试总结】")?;
        writeln!(file, "├─ 测试总数: {}", results.len())?;
        writeln!(
            file,
            "├─ 通过数量: {}",
            results.iter().filter(|r| r.passed).count()
        )?;
        writeln!(
            file,
            "├─ 失败数量: {}",
            results.iter().filter(|r| !r.passed).count()
        )?;
        writeln!(file, "└─ 未找到测试文件的命令: {}", missing_tests.len())?;
        writeln!(file)?;

        // 2.1 列出未找到测试文件的命令
        if !missing_tests.is_empty() {
            writeln!(file, "【未找到测试文件的命令】")?;
            for cmd in missing_tests.iter().take(missing_tests.len() - 1) {
                writeln!(file, "├─ {}", cmd)?;
            }
            if let Some(last) = missing_tests.last() {
                writeln!(file, "└─ {}", last)?;
            }
            writeln!(file)?;
        }

        // 3. 按命令分组结果
        let mut command_groups: std::collections::HashMap<String, Vec<&ComparisonResult>> =
            std::collections::HashMap::new();

        for result in results {
            command_groups
                .entry(result.command.clone())
                .or_default()
                .push(result);
        }

        // 4. 输出每个命令的测试结果
        for (command, command_results) in command_groups {
            writeln!(file, "───────────────────────────────────────────────────")?;
            writeln!(file, "【命令: {}】", command)?;
            let passed = command_results.iter().filter(|r| r.passed).count();
            let total = command_results.len();
            writeln!(file, "├─ 测试用例数: {}", total)?;
            writeln!(file, "├─ 通过数量: {}", passed)?;
            writeln!(
                file,
                "└─ 成功率: {:.1}%",
                (passed as f64 / total as f64) * 100.0
            )?;
            writeln!(file)?;

            // 5. 输出该命令的具体测试用例
            for (index, result) in command_results.iter().enumerate() {
                writeln!(file, "    测试用例 #{}", index + 1)?;
                writeln!(file, "    ├─ 描述: {}", result.description)?;
                writeln!(file, "    ├─ 参数: {}", result.args.join(" "))?;
                writeln!(
                    file,
                    "    ├─ 状态: {}",
                    if result.passed {
                        "✓ 通过"
                    } else {
                        "✗ 失败"
                    }
                )?;

                if !result.passed {
                    writeln!(file, "    └─ 差异详情:")?;
                    for diff in &result.differences {
                        writeln!(file, "        {}", diff)?;
                    }
                } else {
                    writeln!(file, "    └─ 测试通过，无差异")?;
                }
                writeln!(file)?;
            }
            writeln!(file, "───────────────────────────────────────────────────")?;
        }

        // 6. 报告尾部
        writeln!(file, "═══════════════════════════════════════════════════")?;
        writeln!(file, "                报告结束")?;
        writeln!(file, "═══════════════════════════════════════════════════")?;

        Ok(())
    }

    /// 生成JSON格式报告
    fn generate_json_report(
        &self,
        results: &[ComparisonResult],
        missing_tests: &[String],
    ) -> Result<()> {
        let timestamp = Local::now().format("%Y%m%d_%H%M%S");
        let path = self.output_dir.join(format!("report_{}.json", timestamp));

        println!("正在生成 JSON 报告: {}", path.display());

        // 构建分层的报告结构
        let total_tests = results.len();
        let total_passed = results.iter().filter(|r| r.passed).count();
        let total_failed = results.iter().filter(|r| !r.passed).count();

        // 按命令分组结果
        let mut command_groups: std::collections::HashMap<String, Vec<&ComparisonResult>> =
            std::collections::HashMap::new();

        for result in results {
            command_groups
                .entry(result.command.clone())
                .or_default()
                .push(result);
        }

        // 构建命令级别的结果
        let command_results: Vec<_> = command_groups
            .iter()
            .map(|(command, results)| {
                let passed = results.iter().filter(|r| r.passed).count();
                let total = results.len();
                serde_json::json!({
                    "command": command,
                    "summary": {
                        "total_cases": total,
                        "passed_cases": passed,
                        "failed_cases": total - passed,
                        "success_rate": format!("{:.1}%", (passed as f64 / total as f64) * 100.0)
                    },
                    "test_cases": results
                })
            })
            .collect();

        // 构建完整的报告结构
        let report = serde_json::json!({
            "report_info": {
                "title": "兼容性测试报告",
                "generated_at": Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            },
            "summary": {
                "total_tests": total_tests,
                "total_passed": total_passed,
                "total_failed": total_failed,
                "overall_success_rate": format!("{:.1}%", (total_passed as f64 / total_tests as f64) * 100.0),
                "missing_tests_count": missing_tests.len()
            },
            "missing_tests": missing_tests,
            "commands": command_results
        });

        let file = File::create(&path).map_err(|e| {
            TestError::ExecutionError(format!("无法创建报告文件 {}: {}", path.display(), e))
        })?;

        serde_json::to_writer_pretty(file, &report)
            .map_err(|e| TestError::ExecutionError(format!("写入 JSON 报告失败: {}", e)))?;

        println!("报告已成功生成");
        Ok(())
    }

    /// 生成HTML格式报告
    fn generate_html_report(
        &self,
        results: &[ComparisonResult],
        missing_tests: &[String],
    ) -> Result<()> {
        let timestamp = Local::now().format("%Y%m%d_%H%M%S");
        let path = self.output_dir.join(format!("report_{}.html", timestamp));
        let mut file = File::create(path)?;

        // 按命令分组结果
        let mut command_groups: std::collections::HashMap<String, Vec<&ComparisonResult>> =
            std::collections::HashMap::new();

        for result in results {
            command_groups
                .entry(result.command.clone())
                .or_default()
                .push(result);
        }

        // 写入HTML头部
        write!(
            file,
            r#"<!DOCTYPE html>
<html>
<head>
    <title>兼容性测试报告</title>
    <style>
        body {{ font-family: Arial, sans-serif; margin: 20px; }}
        .summary {{ background-color: #f8f9fa; padding: 20px; margin-bottom: 20px; border-radius: 5px; }}
        .command-group {{ margin: 20px 0; border: 1px solid #ddd; border-radius: 5px; overflow: hidden; }}
        .command-header {{ background-color: #f1f1f1; padding: 10px; border-bottom: 1px solid #ddd; }}
        .command-summary {{ padding: 10px; background-color: #f8f9fa; }}
        .test-case {{ margin: 10px; padding: 10px; border: 1px solid #eee; }}
        .passed {{ background-color: #dff0d8; }}
        .failed {{ background-color: #f2dede; }}
        .missing {{ background-color: #fcf8e3; padding: 10px; margin: 10px 0; border-radius: 5px; }}
        .diff {{ white-space: pre-wrap; font-family: monospace; margin-left: 20px; }}
        .command {{ font-family: monospace; background-color: #f5f5f5; padding: 5px; margin: 5px 0; }}
    </style>
</head>
<body>
    <h1>兼容性测试报告</h1>
    
    <div class="summary">
        <h2>测试总结</h2>
        <p>生成时间: {}</p>
        <p>测试总数: {}</p>
        <p>通过数量: {}</p>
        <p>失败数量: {}</p>
        <p>总体成功率: {:.1}%</p>
        <p>未找到测试文件的命令数: {}</p>
    </div>
"#,
            Local::now().format("%Y-%m-%d %H:%M:%S"),
            results.len(),
            results.iter().filter(|r| r.passed).count(),
            results.iter().filter(|r| !r.passed).count(),
            (results.iter().filter(|r| r.passed).count() as f64 / results.len() as f64) * 100.0,
            missing_tests.len()
        )?;

        // 添加未找到测试文件的命令列表
        if !missing_tests.is_empty() {
            writeln!(
                file,
                r#"        <div class="missing">
            <h3>未找到测试文件的命令</h3>
            <ul>"#
            )?;
            for cmd in missing_tests {
                writeln!(file, "                <li>{}</li>", cmd)?;
            }
            writeln!(file, "            </ul>\n        </div>")?;
        }
        writeln!(file, "    </div>")?;

        // 写入每个命令的结果
        for (command, command_results) in command_groups {
            let passed = command_results.iter().filter(|r| r.passed).count();
            let total = command_results.len();

            write!(
                file,
                r#"    <div class="command-group">
        <div class="command-header">
            <h2>命令: {}</h2>
        </div>
        <div class="command-summary">
            <p>测试用例数: {}</p>
            <p>通过数量: {}</p>
            <p>失败数量: {}</p>
            <p>成功率: {:.1}%</p>
        </div>
"#,
                command,
                total,
                passed,
                total - passed,
                (passed as f64 / total as f64) * 100.0
            )?;

            // 写入该命令的测试用例
            for (index, result) in command_results.iter().enumerate() {
                write!(
                    file,
                    r#"        <div class="test-case {}">
            <h3>测试用例 #{}</h3>
            <p>描述: {}</p>
            <div class="command">参数: {}</div>
            <p>状态: {}</p>
"#,
                    if result.passed { "passed" } else { "failed" },
                    index + 1,
                    result.description,
                    result.args.join(" "),
                    if result.passed {
                        "✓ 通过"
                    } else {
                        "✗ 失败"
                    }
                )?;

                if !result.passed {
                    writeln!(file, "            <div class=\"diff\">")?;
                    for diff in &result.differences {
                        writeln!(file, "                {}", html_escape::encode_text(diff))?;
                    }
                    writeln!(file, "            </div>")?;
                }

                writeln!(file, "        </div>")?;
            }

            writeln!(file, "    </div>")?;
        }

        // 写入HTML尾部
        writeln!(file, "</body>\n</html>")?;

        Ok(())
    }
}
