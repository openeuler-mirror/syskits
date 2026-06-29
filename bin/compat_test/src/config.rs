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
#[derive(Debug, Deserialize)]
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
    /// 是否启用调试输出
    #[serde(default = "default_debug")]
    pub debug: bool,
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

impl Default for TestEnvConfig {
    fn default() -> Self {
        Self {
            show_diff: default_show_diff(),
            default_timeout: default_timeout(),
            cleanup: default_cleanup(),
            show_progress: default_show_progress(),
            verbose: default_verbose(),
            debug: default_debug(),
            report_format: default_report_format(),
            report_dir: default_report_dir(),
            detail: DetailConfig::default(),
        }
    }
}

/// 详细配置选项
#[derive(Debug, Deserialize)]
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

impl Default for DetailConfig {
    fn default() -> Self {
        Self {
            show_command: default_true(),
            show_description: default_true(),
            show_env_vars: default_true(),
            show_resource_limits: default_true(),
            show_file_changes: default_true(),
            show_full_output: default_true(),
            show_tags: default_true(),
        }
    }
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

/// 默认值：启用调试输出
fn default_debug() -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_syskits_mode_default() {
        let default_mode = SyskitsMode::default();
        assert!(matches!(default_mode, SyskitsMode::Single));
    }

    #[test]
    fn test_config_default() {
        let config = Config::default();

        // 验证 SyskitsConfig 默认值
        assert!(config.syskits.syskits_path.is_none());
        assert!(config.syskits.coreutils_path.is_none());
        assert!(matches!(config.syskits.mode, SyskitsMode::Single));
        assert!(config.syskits.commands_dir.is_none());

        // 验证 TestSettings 默认值
        assert!(config.test.test_cases_dir.is_none());
        assert!(config.test.default_commands.is_none());

        // 验证 TestEnvConfig 默认值通过默认函数设置
        assert_eq!(config.test.env.show_diff, default_show_diff());
        assert_eq!(config.test.env.default_timeout, default_timeout());
        assert_eq!(config.test.env.cleanup, default_cleanup());
        assert_eq!(config.test.env.show_progress, default_show_progress());
        assert_eq!(config.test.env.verbose, default_verbose());
        assert_eq!(config.test.env.debug, default_debug());
        assert_eq!(config.test.env.report_format, default_report_format());
        assert_eq!(config.test.env.report_dir, default_report_dir());

        // 验证 DetailConfig 默认值
        assert_eq!(config.test.env.detail.show_command, default_true());
        assert_eq!(config.test.env.detail.show_description, default_true());
        assert_eq!(config.test.env.detail.show_env_vars, default_true());
        assert_eq!(config.test.env.detail.show_resource_limits, default_true());
        assert_eq!(config.test.env.detail.show_file_changes, default_true());
        assert_eq!(config.test.env.detail.show_full_output, default_true());
        assert_eq!(config.test.env.detail.show_tags, default_true());
    }

    #[test]
    fn test_syskits_config_default() {
        let config = SyskitsConfig::default();

        assert!(config.syskits_path.is_none());
        assert!(config.coreutils_path.is_none());
        assert!(matches!(config.mode, SyskitsMode::Single));
        assert!(config.commands_dir.is_none());
    }

    #[test]
    fn test_test_settings_default() {
        let settings = TestSettings::default();

        assert!(settings.test_cases_dir.is_none());
        assert!(settings.default_commands.is_none());

        // 环境配置应采用默认值
        assert_eq!(settings.env.show_diff, default_show_diff());
        assert_eq!(settings.env.default_timeout, default_timeout());
        assert_eq!(settings.env.cleanup, default_cleanup());
        assert_eq!(settings.env.show_progress, default_show_progress());
        assert_eq!(settings.env.verbose, default_verbose());
        assert_eq!(settings.env.debug, default_debug());
        assert_eq!(settings.env.report_format, default_report_format());
        assert_eq!(settings.env.report_dir, default_report_dir());
    }

    #[test]
    fn test_test_env_config_default() {
        let env_config = TestEnvConfig::default();

        assert_eq!(env_config.show_diff, default_show_diff());
        assert_eq!(env_config.default_timeout, default_timeout());
        assert_eq!(env_config.cleanup, default_cleanup());
        assert_eq!(env_config.show_progress, default_show_progress());
        assert_eq!(env_config.verbose, default_verbose());
        assert_eq!(env_config.debug, default_debug());
        assert_eq!(env_config.report_format, default_report_format());
        assert_eq!(env_config.report_dir, default_report_dir());

        // 详细配置应全部为 true
        assert_eq!(env_config.detail.show_command, default_true());
        assert_eq!(env_config.detail.show_description, default_true());
        assert_eq!(env_config.detail.show_env_vars, default_true());
        assert_eq!(env_config.detail.show_resource_limits, default_true());
        assert_eq!(env_config.detail.show_file_changes, default_true());
        assert_eq!(env_config.detail.show_full_output, default_true());
        assert_eq!(env_config.detail.show_tags, default_true());
    }

    #[test]
    fn test_detail_config_default() {
        let detail_config = DetailConfig::default();

        assert_eq!(detail_config.show_command, default_true());
        assert_eq!(detail_config.show_description, default_true());
        assert_eq!(detail_config.show_env_vars, default_true());
        assert_eq!(detail_config.show_resource_limits, default_true());
        assert_eq!(detail_config.show_file_changes, default_true());
        assert_eq!(detail_config.show_full_output, default_true());
        assert_eq!(detail_config.show_tags, default_true());
    }

    #[test]
    fn test_default_functions() {
        assert!(default_show_diff());
        assert_eq!(default_timeout(), 30);
        assert!(default_cleanup());
        assert!(default_show_progress());
        assert!(!default_verbose());
        assert!(!default_debug());
        assert_eq!(default_report_format(), "text");
        assert_eq!(default_report_dir(), Path::new("test_reports"));
        assert!(default_true());
    }

    #[test]
    fn test_config_deserialization() {
        // 使用JSON格式而不是YAML
        let json = r#"
        {
            "syskits": {
                "syskits_path": "/usr/bin/syskits",
                "coreutils_path": "/usr/bin/coreutils",
                "mode": "multiple",
                "commands_dir": "/usr/lib/syskits/commands"
            },
            "test": {
                "test_cases_dir": "/path/to/test_cases",
                "default_commands": ["ls", "cp", "rm"],
                "env": {
                    "show_diff": false,
                    "default_timeout": 60,
                    "cleanup": false,
                    "show_progress": false,
                    "verbose": true,
                    "debug": true,
                    "report_format": "json",
                    "report_dir": "custom_reports",
                    "detail": {
                        "show_command": false,
                        "show_description": false,
                        "show_env_vars": false,
                        "show_resource_limits": false,
                        "show_file_changes": false,
                        "show_full_output": false,
                        "show_tags": false
                    }
                }
            }
        }
        "#;

        let config: Config = serde_json::from_str(json).unwrap();

        // 验证 SyskitsConfig
        assert_eq!(
            config.syskits.syskits_path.unwrap(),
            Path::new("/usr/bin/syskits")
        );
        assert_eq!(
            config.syskits.coreutils_path.unwrap(),
            Path::new("/usr/bin/coreutils")
        );
        assert!(matches!(config.syskits.mode, SyskitsMode::Multiple));
        assert_eq!(
            config.syskits.commands_dir.unwrap(),
            Path::new("/usr/lib/syskits/commands")
        );

        // 验证 TestSettings
        assert_eq!(
            config.test.test_cases_dir.unwrap(),
            Path::new("/path/to/test_cases")
        );
        assert_eq!(config.test.default_commands.as_ref().unwrap().len(), 3);
        assert_eq!(config.test.default_commands.as_ref().unwrap()[0], "ls");
        assert_eq!(config.test.default_commands.as_ref().unwrap()[1], "cp");
        assert_eq!(config.test.default_commands.as_ref().unwrap()[2], "rm");

        // 验证 TestEnvConfig
        assert!(!config.test.env.show_diff);
        assert_eq!(config.test.env.default_timeout, 60);
        assert!(!config.test.env.cleanup);
        assert!(!config.test.env.show_progress);
        assert!(config.test.env.verbose);
        assert!(config.test.env.debug);
        assert_eq!(config.test.env.report_format, "json");
        assert_eq!(config.test.env.report_dir, Path::new("custom_reports"));

        // 验证 DetailConfig
        assert!(!config.test.env.detail.show_command);
        assert!(!config.test.env.detail.show_description);
        assert!(!config.test.env.detail.show_env_vars);
        assert!(!config.test.env.detail.show_resource_limits);
        assert!(!config.test.env.detail.show_file_changes);
        assert!(!config.test.env.detail.show_full_output);
        assert!(!config.test.env.detail.show_tags);
    }
}
