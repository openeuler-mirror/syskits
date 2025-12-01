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

//## rust proc-macro background info
//* ref: <https://dev.to/naufraghi/procedural-macro-in-rust-101-k3f> @@ <http://archive.is/Vbr5e>
//* ref: [path construction from LitStr](https://oschwald.github.io/maxminddb-rust/syn/struct.LitStr.html) @@ <http://archive.is/8YDua>

#[proc_macro_attribute]
pub fn main(_args: TokenStream, stream: TokenStream) -> TokenStream {
    // Parse input TokenStream into proc_macro2::TokenStream
    let stream = proc_macro2::TokenStream::from(stream);

    // Generate the new main function
    let new_main = quote!(
        pub fn ctmain(args: impl ctcore::Args) -> i32 {
            #stream
            let result = ctmain(args);
            match result {
                Ok(()) => ctcore::ct_error::get_exit_code(),
                Err(err) => {
                    let s_err = format!("{}", err);
                    if !s_err.is_empty() {
                        ctcore::show_error!("{}", s_err);
                    }
                    if err.usage() {
                        eprintln!("Try '{} --help' for more information.", ctcore::execution_phrase());
                    }
                    err.code()
                }
            }
        }
    );

    // Convert the generated new main function into TokenStream
    TokenStream::from(new_main)
}

// FIXME: This is currently a stub. We could do much more here and could
// even pull in a full markdown parser to get better results.
/// Render markdown into a format that's easier to read in the terminal.
///
/// For now, all this function does is remove backticks.
/// Some ideas for future improvement:
/// - Render headings as bold
/// - Convert triple backticks to indented
/// - Printing tables in a nice format
fn render_markdown(s: &str) -> String {
    s.replace('`', "")
}

/// Get the about text from the help file.
///
/// The about text is assumed to be the text between the first markdown
/// code block and the next header, if any. It may span multiple lines.
#[proc_macro]
pub fn help_about(input: TokenStream) -> TokenStream {
    // Convert input into a vector of TokenTree
    let input_token_tree: Vec<TokenTree> = input.into_iter().collect();

    // Get filename argument
    let filename_arg = get_argument(&input_token_tree, 0, "filename");

    // Parse about text from the help file
    let help_content = read_help(&filename_arg);
    let about_text = cthelp_parser::parse_about(&help_content);

    if about_text.is_empty() {
        panic!("About text not found! Please make sure the markdown text format is correct");
    }
    // Convert the about text into TokenStream
    TokenTree::Literal(Literal::string(&about_text)).into()
}

/// Get the usage from the help file.
///
/// The usage is assumed to be surrounded by markdown code fences. It may span
/// multiple lines. The first word of each line is assumed to be the name of
/// the util and is replaced by "{}" so that the output of this function can be
/// used with `ctcore::format_usage`.
#[proc_macro]
pub fn help_usage(input: TokenStream) -> TokenStream {
    // Convert input into a vector of TokenTree
    let input_token_tree: Vec<TokenTree> = input.into_iter().collect();

    // Get filename argument
    let filename_arg = get_argument(&input_token_tree, 0, "filename");

    // Parse usage text from the help file
    let help_content = read_help(&filename_arg);
    let usage_text: String = cthelp_parser::parse_usage(&help_content);
    if usage_text.is_empty() {
        panic!("Usage text is not found! Please make sure the markdown text format is correct");
    }

    // Convert the usage text into TokenStream
    TokenTree::Literal(Literal::string(&usage_text)).into()
}

/// Reads a section from a file of the util as a `str` literal.
///
/// It reads from the file specified as the second argument, relative to the
/// crate root. The contents of this file are read verbatim, without parsing or
/// escaping. The name of the help file should match the name of the util.
/// I.e. numfmt should have a file called `numfmt.md`. By convention, the file
/// should start with a top-level section with the name of the util. The other
/// sections must start with 2 `#` characters. Capitalization of the sections
/// does not matter. Leading and trailing whitespace of each section will be
/// removed.
///
/// Example:
/// ```md
/// # numfmt
/// ## About
/// Convert numbers from/to human-readable strings
///
/// ## Long help
/// This text will be the long help
/// ```
///
/// ```rust,ignore
/// help_section!("about", "numfmt.md");
/// ```
#[proc_macro]
pub fn help_section(input: TokenStream) -> TokenStream {
    // Convert input into a vector of TokenTree
    let input_content: Vec<TokenTree> = input.into_iter().collect();

    // Get section and filename arguments
    let input_section = get_argument(&input_content, 0, "section");
    let input_filename = get_argument(&input_content, 1, "filename");

    // Parse section from the help file
    let help_content = read_help(&input_filename);
    let text_info = match cthelp_parser::parse_section(&input_section, &help_content) {
        Some(text) => text,
        None => panic!(
            "The section '{}' could not be found in the help file. Maybe it is spelled wrong?",
            help_content
        ),
    };

    // Render the parsed text as markdown
    let rendered_text = render_markdown(&text_info);

    // Convert the rendered markdown text into TokenStream
    TokenTree::Literal(Literal::string(&rendered_text)).into()
}

/// Get an argument from the input vector of `TokenTree`.
///
/// Asserts that the argument is a string literal and returns the string value,
/// otherwise it panics with an error.
fn get_argument(input: &[TokenTree], index: usize, name: &str) -> String {
    // Multiply by two to ignore the `','` in between the arguments
    let token_index = index * 2;

    let token = match &input.get(token_index) {
        Some(TokenTree::Literal(lit)) => lit.to_string(),
        Some(_) => panic!("Argument {} should be a string literal.", index),
        None => panic!("Missing argument at index {} for {}", index, name),
    };

    let value = match token.parse::<String>() {
        Ok(c) => c,
        _ => panic!("Invalid literal"),
    };

    // Ensure that the value is enclosed in double quotes
    if !value.starts_with('"') || !value.ends_with('"') {
        panic!(
            "Invalid string literal format for argument {} in {}",
            index, name
        );
    }

    // Strip the double quotes
    let value = &value[1..value.len() - 1];

    value.to_string()
}

/// Read the help file
fn read_help(filename: &str) -> String {
    let mut content = String::new();

    // Get the cargo manifest directory
    let manifest_dir = match std::env::var("CARGO_MANIFEST_DIR") {
        Ok(dir) => dir,
        Err(err) => {
            panic!("Error: Failed to get CARGO_MANIFEST_DIR: {}", err);
        }
    };

    // Build the path to the file
    let mut manifest_path = PathBuf::from(manifest_dir);
    manifest_path.push(filename);

    // Open the file
    let mut file = match File::open(&manifest_path) {
        Ok(f) => f,
        Err(err) => {
            panic!(
                "Error: Failed to open file {}: {}",
                manifest_path.display(),
                err
            );
        }
    };

    // Read the content of the file into the string
    if let Err(err) = file.read_to_string(&mut content) {
        panic!(
            "Error: Failed to read file {}: {}",
            manifest_path.display(),
            err
        );
    }

    content
}
