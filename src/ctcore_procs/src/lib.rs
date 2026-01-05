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

use std::{fs::File, io::Read, path::PathBuf};

use proc_macro::{Literal, TokenStream, TokenTree};
use quote::quote;

#[proc_macro_attribute]
pub fn main(_ct_args: TokenStream, ct_stream: TokenStream) -> TokenStream {
    // 将输入TokenStream解析为proc_macro2::TokenStream
    let my_stream = proc_macro2::TokenStream::from(ct_stream);

    // 生成新的main函数
    let ct_main = quote!(
        pub fn ctmain(args: impl ctcore::Args) -> i32 {
            #my_stream
            let result = ctmain(args);
            match result {
                Ok(()) => ctcore::ct_error::get_ct_exit_code(),
                Err(err) => {
                    let s_err = format!("{}", err);
                    if !s_err.is_empty() {
                        ctcore::ct_show_error!("{}", s_err);
                    }
                    if err.usage() {
                        eprintln!("Try '{} --help' for more information.", ctcore::ct_execute_phrase());
                    }
                    err.code()
                }
            }
        }
    );

    // 将生成的新main函数转换为TokenStream
    TokenStream::from(ct_main)
}

fn ct_render_markdown(str: &str) -> String {
    str.replace('`', "")
}

/// 从帮助文件中获取“关于”文本。
///
/// 假设“关于”文本位于第一个Markdown代码块与下一个标题（如果有）之间的文本。它可能跨越多行。
#[proc_macro]
pub fn ct_help_about(ct_input: TokenStream) -> TokenStream {
    // 将输入转换为TokenTree向量
    let ct_input_token_tree: Vec<TokenTree> = ct_input.into_iter().collect();

    // 获取文件名参数
    let ct_filename_arg = ct_get_argument(&ct_input_token_tree, 0, "filename");

    // 从帮助文件中解析“关于”文本
    let ct_help_content = ct_read_help(&ct_filename_arg);
    let ct_about_text = cthelp_parser::ct_parse_about(&ct_help_content);

    if ct_about_text.is_empty() {
        panic!("About text not found! Please make sure the markdown text ct_format is correct");
    }
    // 将关于文本转换为TokenStream
    TokenTree::Literal(Literal::string(&ct_about_text)).into()
}

/// 从帮助文件获取用法信息。
///
/// 假定用法信息被Markdown代码围栏包围，可能跨越多行。
/// 每行的第一个单词被视为工具名称，并替换为 "{}"，以便此函数输出能与 ctcore::format_usage 配合使用。
#[proc_macro]
pub fn ct_help_usage(ct_input: TokenStream) -> TokenStream {
    // 将输入转换为TokenTree向量
    let ct_input_token_tree: Vec<TokenTree> = ct_input.into_iter().collect();

    // 获取文件名参数
    let ct_filename_arg = ct_get_argument(&ct_input_token_tree, 0, "filename");

    // 从帮助文件解析用法文本
    let ct_help_content = ct_read_help(&ct_filename_arg);
    let ct_usage_text: String = cthelp_parser::ct_parse_usage(&ct_help_content);
    if ct_usage_text.is_empty() {
        panic!("Usage text is not found! Please make sure the markdown text ct_format is correct");
    }

    // 将关于文本转换为TokenStream
    TokenTree::Literal(Literal::string(&ct_usage_text)).into()
}

/// 从工具文件中读取指定部分作为 str 字面值。
///
/// 文件由第二个参数指定，相对于crate根目录。该文件内容将按原样读取，不进行解析或转义处理。
/// 帮助文件名应与工具名匹配，例如numfmt应有名为numfmt.md的文件。
/// 按照惯例，文件应以工具名命名的顶级章节开始，其他章节必须以两个#字符开始。
/// 章节名称的大小写无关紧要，每个章节的前后空白字符会被移除。
///
/// 示例：
///
/// md
/// # numfmt
/// ## About
/// Convert numbers from/to human-readable strings
///
/// ## Long help
/// This text will be the long help
///
///
/// rust,ignore
/// help_section!("about", "numfmt.md");
///
#[proc_macro]
pub fn ct_help_section(ct_input: TokenStream) -> TokenStream {
    // 将输入转换为TokenTree向量
    let ct_input_content: Vec<TokenTree> = ct_input.into_iter().collect();

    // 获取文件名参数
    let ct_input_section = ct_get_argument(&ct_input_content, 0, "section");
    let ct_input_filename = ct_get_argument(&ct_input_content, 1, "filename");

    // 将关于文本转换为TokenStream
    let ct_help_content = ct_read_help(&ct_input_filename);
    let ct_text_info = match cthelp_parser::ct_parse_section(&ct_input_section, &ct_help_content) {
        Some(text) => text,
        None => panic!(
            "The section '{}' could not be found in the help file. Maybe it is spelled wrong?",
            ct_help_content
        ),
    };

    // 将解析后的文本渲染为Markdown格式
    let ct_rendered_text = ct_render_markdown(&ct_text_info);

    // 将已渲染的Markdown文本转换为TokenStream
    TokenTree::Literal(Literal::string(&ct_rendered_text)).into()
}

/// 读取帮助文件
fn ct_read_help(filename: &str) -> String {
    let mut ct_content = String::new();

    // 获取Cargo清单目录
    let ct_manifest_dir = match std::env::var("CARGO_MANIFEST_DIR") {
        Ok(dir) => dir,
        Err(err) => {
            panic!("Error: Failed to get CARGO_MANIFEST_DIR: {}", err);
        }
    };

    // 构建指向文件的路径
    let mut ct_manifest_path = PathBuf::from(ct_manifest_dir);
    ct_manifest_path.push(filename);

    // 打开清单文件
    let mut file = match File::open(&ct_manifest_path) {
        Ok(f) => f,
        Err(err) => {
            panic!(
                "Error: Failed to open file {}: {}",
                ct_manifest_path.display(),
                err
            );
        }
    };

    // 读取文件内容到字符串中
    if let Err(err) = file.read_to_string(&mut ct_content) {
        panic!(
            "Error: Failed to read file {}: {}",
            ct_manifest_path.display(),
            err
        );
    }

    ct_content
}

/// 从输入的TokenTree向量中获取一个参数。
///
/// 断言该参数为字符串字面量，并返回其字符串值，否则将以错误信息引发panic。
fn ct_get_argument(ct_input: &[TokenTree], index: usize, name: &str) -> String {
    // 乘以二以忽略参数之间的,
    let token_index = index * 2;

    let token = match &ct_input.get(token_index) {
        Some(TokenTree::Literal(lit)) => lit.to_string(),
        Some(_) => panic!("Argument {} should be a string literal.", index),
        None => panic!("Missing argument at index {} for {}", index, name),
    };

    let value = match token.parse::<String>() {
        Ok(c) => c,
        _ => panic!("Invalid literal"),
    };

    // 确保值被双引号包围
    if !value.starts_with('"') || !value.ends_with('"') {
        panic!(
            "Invalid string literal ct_format for argument {} in {}",
            index, name
        );
    }

    // 去掉双引号
    let value = &value[1..value.len() - 1];

    value.to_string()
}
