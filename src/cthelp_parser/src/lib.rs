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

//! A collection of functions to parse the markdown code of help files.
//! 用于解析帮助文件标记代码的函数集
//! The structure of the markdown code is assumed to be:
//! 假定 Markdown 代码的结构为:
//!
//! # 模块名称 name
//!
//! ```text
//! usage info
//! ```
//!
//! About 内容
//!
//! ## 段落 1
//!
//! Some 上下文
//!
//! ## 段落 2
//!
//! Some 上下文

const MARKDOWN_CODE_FLAGS: &str = "```";

/// 跳过markdown 代码块，在下一个标题（如果有）如果下下标题之间的文本解析为一个 about 字符串。
pub fn parse_about(content: &str) -> String {
    let mut lines = content.lines();

    // Skip lines until the first markdown code fence
    for line in lines.by_ref() {
        if line.starts_with(MARKDOWN_CODE_FLAGS) {
            break;
        }
    }

    // Skip one more line after the markdown code fence ,跳过空行
    let _ = lines.next();

    // Skip lines until the second markdown code fence
    for line in lines.by_ref() {
        if line.starts_with(MARKDOWN_CODE_FLAGS) {
            break;
        }
    }

    // Skip one more line after the second markdown code fence
    let _ = lines.next();

    // Take lines until a line starts with '#'
    let mut about_content = String::new();
    for line in lines {
        if line.starts_with('#') {
            break;
        }
        about_content.push_str(line);
        about_content.push('\n');
    }

    about_content.trim().to_string()
}

/// Parses the first markdown code block into a usage string
///
/// The code fences are removed and the name of the util is replaced
/// with `{}` so that it can be replaced with the appropriate name
/// at runtime.
pub fn parse_usage(content: &str) -> String {
    let mut result = String::new();
    let mut inside_code_block = false;

    for line in content.lines() {
        if !inside_code_block {
            if line.starts_with(MARKDOWN_CODE_FLAGS) {
                inside_code_block = true;
            }
        } else {
            if line.starts_with(MARKDOWN_CODE_FLAGS) {
                inside_code_block = false;
                continue;
            }

            if let Some((_util, args)) = line.split_once(' ') {
                result.push_str(&format!("{{}} {}\n", args));
            } else {
                result.push_str("{}\n");
            }
        }
    }

    result.trim().to_string()
}

/// Get a single section from content
///
/// The section must be a second level section (i.e. start with `##`).
pub fn parse_section(section: &str, content: &str) -> Option<String> {
    let section = section.to_lowercase();

    // Check if the section exists
    let mut section_exists = false;
    for line in content.lines() {
        if let Some(header) = line.strip_prefix("##") {
            if header.trim().to_lowercase() == section {
                section_exists = true;
                break;
            }
        }
    }
    if !section_exists {
        return None;
    }

    let section_header_prefix = "## ";

    let mut section_content = String::new();
    let mut found_section = false;
    for line in content.lines() {
        if found_section {
            if line.starts_with(section_header_prefix) {
                break;
            }
            section_content.push_str(line);
            section_content.push('\n');
        } else if let Some(header) = line.strip_prefix("##") {
            if header.trim().to_lowercase() == section {
                found_section = true;
            }
        }
    }

    Some(section_content.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_section() {
        let input = r#"# arch
## some arch section
This is some arch section

## ANOTHER SECTION
            This is the other arch section
with multiple lines test
"#;
        let expected_section1 = "This is some arch section";
        let expected_section2 = "This is some arch section";
        let expected_section3 = "This is the other arch section\nwith multiple lines test";

        assert_eq!(
            parse_section("some arch section", input).unwrap(),
            expected_section1
        );
        assert_eq!(
            parse_section("SOME ARCH SECTION", input).unwrap(),
            expected_section2
        );
        assert_eq!(
            parse_section("another section", input).unwrap(),
            expected_section3
        );
    }

    #[test]
    fn test_parse_section_with_sub_headers() {
        let input = r#"# ls
## after section
This is some section

### level 3 header

Additional text under the section.

#### level 4 header

Yet another paragraph
"#;
        let expeted_section = r#"This is some section

### level 3 header

Additional text under the section.

#### level 4 header

Yet another paragraph"#;

        assert_eq!(
            parse_section("after section", input).unwrap(),
            expeted_section
        );
    }

    #[test]
    fn test_parse_non_existing_section() {
        let input = r#"# ls
## some section
This is some section

## ANOTHER SECTION
            This is the other section
with multiple lines
"#;

        assert!(parse_section("non-existing section", input).is_none());
    }

    #[test]
    fn test_parse_usage() {
        let input = r#"# ls
```
ls -l
```
## some section
This is some section

## ANOTHER SECTION
            This is the other section
with multiple lines
"#;

        assert_eq!(parse_usage(input), "{} -l");
    }

    #[test]
    fn test_parse_multi_line_usage() {
        let input = r#"# ls
```
ls -a
ls -b
ls -c
```
## some section
This is some section
"#;

        assert_eq!(parse_usage(input), "{} -a\n{} -b\n{} -c");
    }

    #[test]
    fn test_parse_about() {
        let input = r#"
# ll

```
ll -h
```

This is the about section for ll

## some section

This is some section for ll
"#;

        assert_eq!(parse_about(input), "This is the about section for ll");
    }

    #[test]
    fn test_parse_multi_line_about() {
        let input = r#"# ll
```
ll -h
```

about ctyunos22.09.1

about ctyunos22.09.2

## some section
This is some section
"#;

        assert_eq!(
            parse_about(input),
            "about ctyunos22.09.1\n\nabout ctyunos22.09.2"
        );
    }
}
