/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved
 *   syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
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
//! - 自动扫描 `src/ct/` 目录下所有实现了 `Tool` trait 的模块
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
use syn::{DeriveInput, parse_macro_input};

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
/// 这个宏会自动扫描项目的 `src/ct/` 目录，查找所有实现了 `Tool` trait 的模块，
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
    // 解析输入的TokenStream
    let input = parse_macro_input!(input as DeriveInput);

    // 获取结构体名称，用于生成辅助代码
    let _struct_name = input.ident;

    // 获取当前工作目录
    let current_dir = std::env::current_dir().expect("Failed to get current directory");
    eprintln!("Current directory: {:?}", current_dir);

    // 扫描项目目录，收集工具
    let tools = collect_tools(&current_dir);

    // 生成工具注册代码
    generate_tools_code(&tools)
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
/// 该函数会扫描 `src/ct/` 目录下的所有子目录，查找实现了 `Tool` trait 的 Rust 文件，
/// 并将这些工具的信息收集到 `ToolInfo` 结构体数组中。
///
/// 参数:
/// - `root_dir`: 项目根目录路径
///
/// 返回:
/// - `Vec<ToolInfo>`: 收集到的工具信息列表
fn collect_tools(root_dir: &std::path::Path) -> Vec<ToolInfo> {
    let mut tools = Vec::new();

    // 构建 src/ct/ 目录路径
    let ct_dir = root_dir.join("src").join("ct");

    // 检查目录是否存在
    if !ct_dir.exists() || !ct_dir.is_dir() {
        // 如果目录不存在，返回空列表
        // eprintln!("Directory not found: {:?}", ct_dir);
        return tools;
    }

    // 使用glob模式匹配所有可能的工具目录
    let pattern = ct_dir.join("*").to_string_lossy().to_string();
    // eprintln!("Scanning for tools with pattern: {}", pattern);

    // 遍历匹配到的目录，使用flatten简化处理
    if let Ok(entries) = glob(&pattern) {
        for path in entries.flatten() {
            if path.is_dir() {
                // 获取目录名称，作为模块名
                if let Some(module_name) = path.file_name().and_then(|n| n.to_str()) {
                    // 确认是否有与模块名同名的 .rs 文件
                    let module_file = path.join("src").join(format!("{}.rs", module_name));
                    if module_file.exists() {
                        let mut file_tools = scan_tool_implementations(&module_file, module_name);
                        tools.append(&mut file_tools);
                    }
                }
            }
        }
    }

    // 按命令名排序，保持结果稳定
    tools.sort_by(|a, b| a.cmd_name.cmp(&b.cmd_name));

    //eprintln!("Found {} tools: {:?}", tools.len(), tools);
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
fn generate_tools_code(tools: &[ToolInfo]) -> TokenStream {
    if tools.is_empty() {
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
    eprintln!("tools cnt: {}", tools.len());
    for tool in tools {
        eprintln!("tool: {:?}", tool);
        let type_name = syn::Ident::new(&tool.type_name, proc_macro2::Span::call_site());

        // 使用 quote::format_ident! 来处理可能需要 r# 前缀的标识符
        let module_name = if tool.module_name == "true" || tool.module_name == "false" {
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
            tool.module_name // 使用原始模块名，不使用raw_module_name
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
