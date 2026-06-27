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

//! # Tool Derive 过程宏
//!
//! 这个过程宏用于自动扫描和注册项目中的工具模块，简化系统工具的集成过程。
//!
//! ## 功能
//!
//! - 自动扫描 `crates/commands/` 目录下所有实现了 `Tool` trait 的模块
//! - 生成工具注册代码，包括导入语句、静态命令列表和工具获取函数
//! - 为每个工具添加对应的 feature 条件编译控制
//!
//! ## 生成的代码
//!
//! 这个宏会生成以下内容（以下是示例，不会直接编译）：
//!
//! ```text
//! // 导入工具模块（每个都有feature控制）
//! #[cfg(feature = "arch")]
//! use arch::Arch;
//! #[cfg(feature = "cat")]
//! use cat::Cat;
//! // ... 其他工具模块的导入
//!
//! // 所有命令列表
//! static ALL_COMMANDS: &[&str] = &[
//!     #[cfg(feature = "arch")]
//!     "arch",
//!     #[cfg(feature = "cat")]
//!     "cat",
//!     // ... 其他命令名称
//! ];
//!
//! // 通过命令名称获取工具实例
//! fn get_tool(command: &str) -> Option<Box<dyn Tool>> {
//!     match command {
//!         #[cfg(feature = "arch")]
//!         "arch" => Some(Box::new(Arch::default())),
//!         #[cfg(feature = "cat")]
//!         "cat" => Some(Box::new(Cat::default())),
//!         // ... 其他工具的匹配分支
//!         _ => None,
//!     }
//! }
//! ```
//!
//! ## 使用方法
//!
//! 在实际项目中，使用方式如下：
//!
//! ```no_run
//! // 这只是一个演示，不会实际编译运行
//! extern crate tool_derive;
//!
//! // 假设存在以下trait定义
//! trait Tool {
//!     fn name(&self) -> &'static str;
//!     fn execute(&self) -> Result<(), Box<dyn std::error::Error>>;
//! }
//!
//! #[derive(tool_derive::Tools)]
//! struct ToolsRegistry;
//!
//! // 宏展开后，可以使用生成的函数和变量
//! fn main() {
//!     // 假设宏生成了ALL_COMMANDS和get_tool
//!     println!("Available tools: {:?}", ALL_COMMANDS);
//!     
//!     if let Some(tool) = get_tool("example") {
//!         let _ = tool.execute();
//!     }
//! }
//! ```

extern crate proc_macro;
use glob::glob;
use proc_macro::TokenStream;
use quote::quote;
use std::fs;
use std::io::Read;
use std::io::Write;
use syn::{DeriveInput, parse_macro_input};

/// 辅助宏，移到模块顶层，避免重复定义
macro_rules! current_log_or_eprintln {
    ($log_file_opt:expr, $log_path_for_print:expr, $($arg:tt)*) => {
        if let Some(file) = $log_file_opt { // 移除 ref mut
            if writeln!(file, $($arg)*).is_err() {
                eprintln!($($arg)*);
                eprintln!("(Above message also failed to write to tool_derive log file at {:?})", $log_path_for_print);
            }
        } else {
            eprintln!($($arg)*);
            eprintln!("(Log file {:?} could not be opened for tool_derive)", $log_path_for_print);
        }
    }
}

/// 工具信息结构体，用于存储从项目中扫描到的工具信息
#[derive(Debug)]
struct ToolInfo {
    /// 模块名称，通常是目录名称，例如 "arch"
    module_name: String,
    /// 类型名称，例如 "Arch"
    type_name: String,
    /// 命令名称，从name方法中提取，例如 "arch"
    cmd_name: String,
}

/// 派生宏定义 - 为结构体实现工具注册逻辑
///
/// 使用方式: `#[derive(Tools)]`
///
/// 这个宏会自动扫描项目的 `crates/commands/` 目录，查找所有实现了 `Tool` trait 的模块，
/// 然后生成必要的代码来注册这些工具，使它们可以被主程序使用。
///
/// 生成的代码包括：
/// 1. 导入各个工具模块（带有对应的feature条件编译）
/// 2. `ALL_COMMANDS` 静态数组，包含所有工具命令名
/// 3. `get_tool` 函数，用于根据命令名获取工具实例
///
/// 每个工具都有对应的feature控制，只有启用了对应feature的工具才会被包含在编译中。
#[proc_macro_derive(Tools)]
pub fn derive_tools(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let _struct_name = input.ident;

    let out_dir_path = match std::env::var("OUT_DIR") {
        Ok(path) => std::path::PathBuf::from(path),
        Err(_) => {
            let manifest_dir =
                std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
            let target_dir = std::path::Path::new(&manifest_dir).join("../../target");
            if !target_dir.exists() {
                let _ = fs::create_dir_all(&target_dir); // 忽略创建错误，后续文件打开会失败
            }
            target_dir
        }
    };
    let log_file_path = out_dir_path.join("tool_derive_debug.log");

    let mut log_file_option = fs::File::create(&log_file_path).ok();

    let current_dir = std::env::current_dir().expect("Failed to get current directory");
    current_log_or_eprintln!(
        &mut log_file_option,
        &log_file_path,
        "Current directory for tool_derive macro execution: {:?}",
        current_dir
    );
    current_log_or_eprintln!(
        &mut log_file_option,
        &log_file_path,
        "OUT_DIR is: {:?}",
        out_dir_path
    );
    current_log_or_eprintln!(
        &mut log_file_option,
        &log_file_path,
        "Log file target is: {:?}",
        log_file_path
    );

    let tools = collect_tools(&current_dir, &mut log_file_option, &log_file_path);
    generate_tools_code(&tools, &mut log_file_option, &log_file_path)
}

/// 扫描文件中的所有 Tool trait 实现并提取信息
///
/// 该函数查找文件中所有 `impl Tool for XXX` 实现，并为每个实现提取命令名称
/// 从对应的 `fn name(&self) -> &'static str { "command_name" }` 方法中。
///
/// 参数:
/// - `file_path`: 要扫描的文件路径
/// - `module_name`: 模块名称（目录名）
///
/// 返回:
/// - `Vec<ToolInfo>`: 提取到的工具信息列表
fn scan_tool_implementations(file_path: &std::path::Path, module_name: &str) -> Vec<ToolInfo> {
    let mut tools = Vec::new();

    // 读取文件内容
    let mut file_content = String::new();
    if let Ok(mut file) = fs::File::open(file_path) {
        if file.read_to_string(&mut file_content).is_err() {
            return tools;
        }
    } else {
        return tools;
    }

    // 使用简单的文本搜索查找 impl Tool for 语句
    let lines: Vec<&str> = file_content.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        // 支持更多可能的 impl Tool 格式
        if trimmed.contains("impl Tool for ")
            || trimmed.contains("impl ") && trimmed.contains(" Tool ") && trimmed.contains(" for ")
        {
            // 提取类型名
            if let Some(type_name) = extract_type_name(trimmed) {
                // 在后续行中查找 name 方法实现
                if let Some(cmd_name) = find_name_method(&lines, i) {
                    tools.push(ToolInfo {
                        module_name: module_name.to_string(),
                        type_name: type_name.to_string(),
                        cmd_name,
                    });
                }
            }
        }
    }

    tools
}

/// 从 impl Tool for XXX 语句中提取类型名
fn extract_type_name(line: &str) -> Option<&str> {
    // 尝试更宽松的匹配模式
    let trimmed = line.trim();

    // 尝试直接的匹配模式
    if let Some(pos) = trimmed.find("impl Tool for ") {
        let after_for = &trimmed[pos + "impl Tool for ".len()..];
        // 提取到类型名（处理可能的泛型、where 子句等）
        let end_pos = after_for
            .find('{')
            .unwrap_or_else(|| after_for.find("where").unwrap_or(after_for.len()));

        let type_name = after_for[..end_pos].trim();
        return Some(type_name);
    }

    None
}

/// 在给定的行后查找 name 方法实现并提取命令名
fn find_name_method(lines: &[&str], start_line: usize) -> Option<String> {
    let mut brace_count = 0;
    let mut in_name_method = false;
    let mut in_impl_block = false;

    // 从 impl 语句后开始搜索
    for (i, line) in lines.iter().enumerate().skip(start_line) {
        let trimmed = line.trim();

        // 检查是否进入了 impl 块
        if i == start_line || !in_impl_block {
            if trimmed.contains("{") {
                in_impl_block = true;
            }
            if !in_impl_block {
                continue;
            }
        }

        // 计算花括号数量以确定何时退出当前实现块
        brace_count += trimmed.chars().filter(|&c| c == '{').count();
        brace_count -= trimmed.chars().filter(|&c| c == '}').count();

        // 检查是否为 name 方法
        if !in_name_method
            && (trimmed.contains("fn name(&self)")
                || trimmed.contains("fn name( &self )")
                || trimmed.contains("fn name(&self )")
                || trimmed.contains("fn name( &self)"))
        {
            in_name_method = true;
            //eprintln!("Found name method at line {}", i);
        }

        // 如果在 name 方法中，提取返回的字符串字面量
        if in_name_method {
            if let Some(start) = trimmed.find('"') {
                if let Some(end) = trimmed[start + 1..].find('"') {
                    let cmd_name = trimmed[start + 1..start + 1 + end].to_string();
                    // eprintln!("Extracted command name: {}", cmd_name);
                    return Some(cmd_name);
                }
            }

            // 如果找到了 name 方法但没有直接返回字符串，则退出
            if brace_count == 0 || (trimmed.contains("fn ") && !trimmed.contains("fn name")) {
                // eprintln!("Exiting name method search without finding command name");
                break;
            }
        }

        // 如果退出了 impl 块，则退出循环
        if in_impl_block && brace_count == 0 {
            // eprintln!("Exiting impl block search");
            break;
        }
    }

    // eprintln!("Failed to find command name in name method");
    None
}

/// 扫描项目目录，收集所有工具模块
///
/// 该函数会扫描 `crates/commands/` 目录下的所有子目录，查找实现了 `Tool` trait 的 Rust 文件，
/// 并将这些工具的信息收集到 `ToolInfo` 结构体数组中。
///
/// 参数:
/// - `root_dir`: 项目根目录路径
///
/// 返回:
/// - `Vec<ToolInfo>`: 收集到的工具信息列表
fn collect_tools(
    root_dir: &std::path::Path,
    log_file_option: &mut Option<fs::File>,
    log_file_path_for_print: &std::path::Path,
) -> Vec<ToolInfo> {
    let mut tools = Vec::new();
    let commands_dir = root_dir.join("crates/commands");

    current_log_or_eprintln!(
        log_file_option,
        log_file_path_for_print,
        "Attempting to scan commands directory: {:?}",
        commands_dir
    );

    if !commands_dir.exists() || !commands_dir.is_dir() {
        current_log_or_eprintln!(
            log_file_option,
            log_file_path_for_print,
            "Directory not found or not a directory: {:?}",
            commands_dir
        );
        return tools;
    }

    let pattern = commands_dir.join("*").to_string_lossy().to_string();
    current_log_or_eprintln!(
        log_file_option,
        log_file_path_for_print,
        "Scanning for tools with pattern: {}",
        pattern
    );

    if let Ok(entries) = glob(&pattern) {
        for path_result in entries {
            match path_result {
                Ok(path) => {
                    if path.is_dir() {
                        if let Some(module_name) = path.file_name().and_then(|n| n.to_str()) {
                            let lib_rs_path = path.join("src").join("lib.rs");
                            let module_name_rs_path =
                                path.join("src").join(format!("{}.rs", module_name));
                            let mut found_tools_in_module = false;

                            if lib_rs_path.exists() {
                                current_log_or_eprintln!(
                                    log_file_option,
                                    log_file_path_for_print,
                                    "Scanning {:?} for module: {}",
                                    lib_rs_path,
                                    module_name
                                );
                                let mut file_tools =
                                    scan_tool_implementations(&lib_rs_path, module_name);
                                if !file_tools.is_empty() {
                                    tools.append(&mut file_tools);
                                    found_tools_in_module = true;
                                }
                            }
                            if !found_tools_in_module && module_name_rs_path.exists() {
                                current_log_or_eprintln!(
                                    log_file_option,
                                    log_file_path_for_print,
                                    "Scanning {:?} for module: {}",
                                    module_name_rs_path,
                                    module_name
                                );
                                let mut file_tools =
                                    scan_tool_implementations(&module_name_rs_path, module_name);

                                tools.append(&mut file_tools);
                            }
                        }
                    }
                }
                Err(e) => {
                    current_log_or_eprintln!(
                        log_file_option,
                        log_file_path_for_print,
                        "Error processing glob entry: {:?}",
                        e
                    );
                }
            }
        }
    }

    tools.sort_by(|a, b| a.cmd_name.cmp(&b.cmd_name));
    current_log_or_eprintln!(
        log_file_option,
        log_file_path_for_print,
        "Found {} tools by tool_derive: {:?}",
        tools.len(),
        tools
    );
    tools
}

/// 生成工具注册代码
///
/// 该函数根据收集到的工具信息生成必要的代码，包括导入语句、静态命令列表和获取工具实例的函数。
/// 每个工具都会带有对应的feature条件编译标记，确保只有启用了对应feature的工具才会被包含在编译中。
///
/// 生成的代码示例:
/// ```text
/// // 导入工具模块
/// #[cfg(feature = "arch")]
/// use arch::Arch;
/// // ... 其他导入
///
/// // 所有命令列表
/// static ALL_COMMANDS: &[&str] = &[
///     #[cfg(feature = "arch")]
///     "arch",
///     // ... 其他命令
/// ];
///
/// // 获取工具实例的函数
/// fn get_tool(command: &str) -> Option<Box<dyn Tool>> {
///     match command {
///         #[cfg(feature = "arch")]
///         "arch" => Some(Box::new(Arch::new())),
///         // ... 其他匹配分支
///         _ => None,
///     }
/// }
/// ```
///
/// 参数:
/// - `tools`: 工具信息列表
///
/// 返回:
/// - `TokenStream`: 生成的代码作为令牌流返回
fn generate_tools_code(
    tools: &[ToolInfo],
    log_file_option: &mut Option<fs::File>,
    log_file_path_for_print: &std::path::Path,
) -> TokenStream {
    if tools.is_empty() {
        current_log_or_eprintln!(
            log_file_option,
            log_file_path_for_print,
            "No tools found, generating empty stubs."
        );
        return quote! {
            static ALL_COMMANDS: &[&str] = &[];

            fn get_tool(_command: &str) -> Option<Box<dyn Tool>> {
                None
            }
        }
        .into();
    }

    let mut match_arms_get_tool = Vec::new();
    let mut all_commands = Vec::new();
    let mut use_statements = Vec::new();

    current_log_or_eprintln!(
        log_file_option,
        log_file_path_for_print,
        "Generating code for {} tools.",
        tools.len()
    );
    for tool in tools {
        current_log_or_eprintln!(
            log_file_option,
            log_file_path_for_print,
            "Processing tool: {:?}",
            tool
        );
        let type_name = syn::Ident::new(&tool.type_name, proc_macro2::Span::call_site());
        
        // 对于"test"模块，使用"command_test"作为特性名
        // 这必须与bin/syskits/Cargo.toml中的依赖设置保持一致
        // Cargo.toml中已将test模块映射为command_test特性，以避免与Rust内置的test特性冲突
        let feature_name = if tool.module_name == "test" {
            "command_test".to_string()
        } else {
            tool.module_name.clone()
        };
        
        // 对于test模块，在导入路径中也使用command_test，而不是test
        let module_name = if tool.module_name == "test" {
            quote::format_ident!(
                "command_test",
                span = proc_macro2::Span::call_site()
            )
        } else if tool.module_name == "true" || tool.module_name == "false" {
            quote::format_ident!(
                "r#{}",
                tool.module_name,
                span = proc_macro2::Span::call_site()
            )
        } else {
            quote::format_ident!(
                "{}",
                tool.module_name,
                span = proc_macro2::Span::call_site()
            )
        };

        let cmd_name_literal = proc_macro2::Literal::string(&tool.cmd_name);

        let cfg_attr = syn::parse_str::<proc_macro2::TokenStream>(&format!(
            "#[cfg(feature = \"{}\")]",
            feature_name // 使用根据模块类型确定的特性名
        ))
        .unwrap();

        use_statements.push(quote! {
            #cfg_attr
            use #module_name::#type_name;
        });

        all_commands.push(quote! {
            #cfg_attr
            #cmd_name_literal
        });

        match_arms_get_tool.push(quote! {
            #cfg_attr
            #cmd_name_literal => Some(Box::new(#type_name::default()))
        });
    }

    let expanded = quote! {
        #(#use_statements)*

        static ALL_COMMANDS: &[&str] = &[
            #(#all_commands),*
        ];

        fn get_tool(command: &str) -> Option<Box<dyn Tool>> {
            match command {
                #(#match_arms_get_tool),*,
                _ => None,
            }
        }
    };

    expanded.into()
}
