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

//! 配置管理模块
//! 提供测试框架的配置管理功能，包括配置文件解析和默认配置设置

use serde::Deserialize;
use std::path::PathBuf;

/// 配置结构体
#[derive(Debug, Default, Deserialize)]
pub struct Config {
    /// syskits 相关配置
    #[serde(default)]
    pub syskits: SyskitsConfig,
    /// 测试相关配置
    #[serde(default)]
    pub test: TestSettings,
}

/// syskits 执行模式
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum SyskitsMode {
    /// 单一二进制文件模式，所有命令都是子命令 (syskits cmd [args])
    Single,
    /// 多命令模式，每个命令都是独立的二进制文件
    Multiple,
}

impl Default for SyskitsMode {
    fn default() -> Self {
        Self::Single
    }
}

/// syskits 配置
#[derive(Debug, Deserialize, Default)]
pub struct SyskitsConfig {
    /// syskits 可执行文件路径
    pub syskits_path: Option<PathBuf>,
    /// GNU coreutils 可执行文件路径
    pub coreutils_path: Option<PathBuf>,
    /// 执行模式
    #[serde(default)]
    pub mode: SyskitsMode,
    /// 多命令模式下的命令目录
    pub commands_dir: Option<PathBuf>,
}

/// 测试配置
#[derive(Debug, Deserialize, Default)]
pub struct TestSettings {
    /// 测试用例目录
    pub test_cases_dir: Option<PathBuf>,
    /// 默认测试命令列表
    pub default_commands: Option<Vec<String>>,
    /// 测试环境配置
    #[serde(default)]
    pub env: TestEnvConfig,
}

/// 测试环境配置
#[derive(Debug, Deserialize, Default)]
pub struct TestEnvConfig {
    /// 是否显示差异
    #[serde(default = "default_show_diff")]
    pub show_diff: bool,
    /// 默认超时时间（秒）
    #[serde(default = "default_timeout")]
    pub default_timeout: u64,
    /// 是否清理临时文件
    #[serde(default = "default_cleanup")]
    pub cleanup: bool,
    /// 是否显示进度
    #[serde(default = "default_show_progress")]
    pub show_progress: bool,
    /// 是否显示详细信息
    #[serde(default = "default_verbose")]
    pub verbose: bool,
    /// 报告格式
    #[serde(default = "default_report_format")]
    pub report_format: String,
    /// 报告输出目录
    #[serde(default = "default_report_dir")]
    pub report_dir: PathBuf,
    /// 详细配置
    #[serde(default)]
    pub detail: DetailConfig,
}

/// 详细配置选项
#[derive(Debug, Deserialize, Default)]
pub struct DetailConfig {
    /// 是否显示命令
    #[serde(default = "default_true")]
    pub show_command: bool,
    /// 是否显示描述
    #[serde(default = "default_true")]
    pub show_description: bool,
    /// 是否显示环境变量
    #[serde(default = "default_true")]
    pub show_env_vars: bool,
    /// 是否显示资源限制
    #[serde(default = "default_true")]
    pub show_resource_limits: bool,
    /// 是否显示文件变化
    #[serde(default = "default_true")]
    pub show_file_changes: bool,
    /// 是否显示完整输出
    #[serde(default = "default_true")]
    pub show_full_output: bool,
    /// 是否显示标签
    #[serde(default = "default_true")]
    pub show_tags: bool,
}

/// 默认值：显示差异
fn default_show_diff() -> bool {
    true
}

/// 默认值：超时时间
fn default_timeout() -> u64 {
    30
}

/// 默认值：清理临时文件
fn default_cleanup() -> bool {
    true
}

/// 默认值：显示进度
fn default_show_progress() -> bool {
    true
}

/// 默认值：不显示详细信息
fn default_verbose() -> bool {
    false
}

/// 默认值：文本格式报告
fn default_report_format() -> String {
    "text".to_string()
}

/// 默认值：报告目录
fn default_report_dir() -> PathBuf {
    PathBuf::from("test_reports")
}

/// 默认值：true
fn default_true() -> bool {
    true
}
