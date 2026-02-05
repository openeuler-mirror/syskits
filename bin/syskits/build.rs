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

use glob::glob;
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

// 统一的顶层辅助宏 - 调整为 build.rs 的上下文
// 在 build.rs 中，通常使用 println!("cargo:warning=...") 进行调试输出
// 或者直接写入 $OUT_DIR 下的日志文件
macro_rules! build_script_log {
    ($log_file_opt_param:expr, $log_path_for_print_param:expr, $($arg:tt)*) => {
        let msg = format!($($arg)*);
        if let Some(file) = $log_file_opt_param.as_mut() {
            if writeln!(file, "{}", msg).is_err() {
                println!("cargo:warning={}", msg);
                println!("cargo:warning=(Above message also failed to write to build script log file at {:?})", $log_path_for_print_param);
            }
        } else {
            println!("cargo:warning={}", msg);
            println!("cargo:warning=(Log file {:?} could not be opened for build script)", $log_path_for_print_param);
        }
    }
}

/// 工具信息结构体
#[derive(Debug)]
struct ToolInfo {
    module_name: String,
    type_name: String,
    cmd_name: String,
}

fn main() {
    let out_dir_str = env::var("OUT_DIR").expect("OUT_DIR not set");
    let out_dir_path = PathBuf::from(&out_dir_str);
    let generated_file_path = out_dir_path.join("generated_tools.rs");
    let log_file_path = out_dir_path.join("syskits_build_debug.log");

    let mut log_file_option = fs::File::create(&log_file_path).ok();

    let manifest_dir = env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .expect("CARGO_MANIFEST_DIR not set");

    let project_root = manifest_dir
        .parent() // bin/syskits -> bin
        .and_then(Path::parent) // bin -> <project_root>
        .expect("Failed to determine project root from CARGO_MANIFEST_DIR");

    build_script_log!(
        &mut log_file_option,
        &log_file_path,
        "Project root directory: {:?}",
        project_root
    );
    build_script_log!(
        &mut log_file_option,
        &log_file_path,
        "OUT_DIR is: {:?}",
        out_dir_path
    );
    build_script_log!(
        &mut log_file_option,
        &log_file_path,
        "Generated code target is: {:?}",
        generated_file_path
    );
    build_script_log!(
        &mut log_file_option,
        &log_file_path,
        "Log file target is: {:?}",
        log_file_path
    );

    let tools = collect_tools(project_root, &mut log_file_option, &log_file_path);
    let generated_code = generate_tools_code(&tools, &mut log_file_option, &log_file_path);

    fs::write(&generated_file_path, generated_code).expect("Failed to write generated tools code");

    build_script_log!(
        &mut log_file_option,
        &log_file_path,
        "Successfully wrote generated_tools.rs"
    );

    // 告诉 Cargo 何时重新运行此构建脚本
    println!("cargo:rerun-if-changed=build.rs");
    // 监视整个 commands 目录的变化
    let commands_path_str = project_root
        .join("crates")
        .join("commands")
        .to_string_lossy()
        .into_owned();
    println!("cargo:rerun-if-changed={commands_path_str}");

    // 还需要为每个可能的特性变化添加 rerun-if-env-changed
    // 这部分可以动态生成，或者如果特性列表相对固定，可以硬编码
    // 为简化，暂时只 rerun if any CARGO_FEATURE_* changes.
    // 更精细的做法是只针对在 commands 中找到的模块名对应的特性。
    // TODO: 优化为只监控实际存在的特性。
    // 遍历 tools 收集到的 module_name 来生成更精确的 rerun-if-env-changed
    for tool in &tools {
        println!(
            "cargo:rerun-if-env-changed=CARGO_FEATURE_{}",
            tool.module_name.to_uppercase().replace('-', "_")
        );
    }
    // Fallback for any other features that might affect compilation, though less direct.
    // Consider if this generic one is truly needed if the above loop is comprehensive.
    // println!("cargo:rerun-if-env-changed=CARGO_FEATURE_*");
}

fn scan_tool_implementations(file_path: &Path, module_name: &str) -> Vec<ToolInfo> {
    let mut tools = Vec::new();
    let mut file_content = String::new();
    if let Ok(mut file) = fs::File::open(file_path) {
        if file.read_to_string(&mut file_content).is_err() {
            return tools;
        }
    } else {
        return tools;
    }

    let lines: Vec<&str> = file_content.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.contains("impl Tool for ")
            || trimmed.contains("impl ") && trimmed.contains(" Tool ") && trimmed.contains(" for ")
        {
            if let Some(type_name) = extract_type_name(trimmed) {
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

fn extract_type_name(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if let Some(pos) = trimmed.find("impl Tool for ") {
        let after_for = &trimmed[pos + "impl Tool for ".len()..];
        let end_pos = after_for
            .find('{')
            .unwrap_or_else(|| after_for.find("where").unwrap_or(after_for.len()));
        let type_name = after_for[..end_pos].trim();
        return Some(type_name.to_string());
    }
    // Add more robust parsing if needed, e.g. using syn
    None
}

fn find_name_method(lines: &[&str], start_line: usize) -> Option<String> {
    let mut brace_count = 0;
    let mut in_name_method = false;
    let mut in_impl_block = false;

    for (i, line) in lines.iter().enumerate().skip(start_line) {
        let trimmed = line.trim();
        if i == start_line || !in_impl_block {
            if trimmed.contains("{") {
                in_impl_block = true;
            }
            if !in_impl_block {
                continue;
            }
        }
        brace_count += trimmed.chars().filter(|&c| c == '{').count();
        brace_count -= trimmed.chars().filter(|&c| c == '}').count();
        if !in_name_method
            && (trimmed.contains("fn name(&self)")
                || trimmed.contains("fn name( &self )")
                || trimmed.contains("fn name(&self )")
                || trimmed.contains("fn name( &self)"))
        {
            in_name_method = true;
        }
        if in_name_method {
            if let Some(start) = trimmed.find('"') {
                if let Some(end) = trimmed[start + 1..].find('"') {
                    return Some(trimmed[start + 1..start + 1 + end].to_string());
                }
            }
            if brace_count == 0 || (trimmed.contains("fn ") && !trimmed.contains("fn name")) {
                break;
            }
        }
        if in_impl_block && brace_count == 0 {
            break;
        }
    }
    None
}

fn collect_tools(
    project_root: &Path,
    log_file_option: &mut Option<fs::File>,
    log_file_path_for_print: &Path,
) -> Vec<ToolInfo> {
    let mut tools = Vec::new();
    let commands_dir = project_root.join("crates").join("commands");

    build_script_log!(
        log_file_option,
        log_file_path_for_print,
        "Attempting to scan commands directory: {:?}",
        commands_dir
    );

    if !commands_dir.exists() || !commands_dir.is_dir() {
        build_script_log!(
            log_file_option,
            log_file_path_for_print,
            "Directory not found or not a directory: {:?}",
            commands_dir
        );
        return tools;
    }

    let pattern = commands_dir.join("*").to_string_lossy().to_string();
    build_script_log!(
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
                                path.join("src").join(format!("{module_name}.rs"));
                            let mut found_tools_in_module = false;

                            if lib_rs_path.exists() {
                                build_script_log!(
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
                                build_script_log!(
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
                    build_script_log!(
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
    build_script_log!(
        log_file_option,
        log_file_path_for_print,
        "Found {} tools: {:?}",
        tools.len(),
        tools.iter().map(|t| &t.cmd_name).collect::<Vec<_>>()
    );
    tools
}

fn generate_tools_code(
    tools: &[ToolInfo],
    log_file_option: &mut Option<fs::File>,
    log_file_path_for_print: &Path,
) -> String {
    if tools.is_empty() {
        build_script_log!(
            log_file_option,
            log_file_path_for_print,
            "No tools found, generating empty stubs."
        );
        return "
            static ALL_COMMANDS: &[&str] = &[];

            #[allow(dead_code)]
            fn get_tool(_command: &str) -> Option<Box<dyn Tool>> {
                None
            }
        "
        .to_string();
    }

    let mut use_statements = String::new();
    let mut all_commands_entries = String::new();
    let mut match_arms = String::new();

    build_script_log!(
        log_file_option,
        log_file_path_for_print,
        "Generating code for {} potential tools. Checking active features...",
        tools.len()
    );

    for tool in tools {
        let feature_name_for_env = tool.module_name.to_uppercase().replace('-', "_");
        let env_var_name = format!("CARGO_FEATURE_{feature_name_for_env}");
        let is_feature_active = env::var(&env_var_name).is_ok();

        build_script_log!(
            log_file_option,
            log_file_path_for_print,
            "  - Tool: module='{}', type='{}', cmd='{}', Associated Feature: '{}', Active in this build: {}",
            tool.module_name,
            tool.type_name,
            tool.cmd_name,
            tool.module_name, // Feature name matches module name by convention
            if is_feature_active { "YES" } else { "NO" }
        );
        // 模块名可能需要处理关键字，例如 "true"
        // 在生成的代码中，我们假设依赖的 crate 名与模块名一致
        let type_name = &tool.type_name; // 例如 Arch
        let cmd_name = &tool.cmd_name; // 例如 "arch"

        // 创建一个在 use 语句中使用的、格式正确的模块名
        // 1. 将 '-' 替换为 '_'
        // 2. 如果是关键字 ("true", "false"), 添加 "r#" 前缀
        let mut final_crate_name_for_use_stmt = tool.module_name.replace('-', "_");
        if final_crate_name_for_use_stmt == "true" || final_crate_name_for_use_stmt == "false" {
            final_crate_name_for_use_stmt = format!("r#{final_crate_name_for_use_stmt}");
        }

        // 对于 test 模块的特殊处理：
        // 1. 使用 command_test 作为特性名 (与Cargo.toml中的依赖名一致)
        // 2. 导入语句中保持使用 "command_test"，而不是"test" (对应Cargo.toml中的依赖名)
        let (feature_name, module_name_for_import) = if tool.module_name == "test" {
            ("command_test", "command_test")
        } else {
            (
                tool.module_name.as_str(),
                final_crate_name_for_use_stmt.as_str(),
            )
        };

        // 生成导入语句
        use_statements.push_str(&format!(
            "#[cfg(feature = \"{feature_name}\")]\nuse {module_name_for_import}::{type_name};\n"
        ));

        all_commands_entries.push_str(&format!(
            "    #[cfg(feature = \"{feature_name}\")]\n    \"{cmd_name}\",\n"
        ));

        match_arms.push_str(&format!(
            "        #[cfg(feature = \"{feature_name}\")]\n        \"{cmd_name}\" => Some(Box::new({type_name})),\n"
        ));
    }

    format!(
        "// Generated by build.rs\n\
        {use_statements}\n\
        static ALL_COMMANDS: &[&str] = &[\n\
        {all_commands_entries}\
        ];\n\n\
        #[allow(dead_code)]\n\
        fn get_tool(command: &str) -> Option<Box<dyn Tool>> {{\n\
        match command {{\n\
        {match_arms}\
                _ => None,\n\
            }}\n\
        }}\n"
    )
}
