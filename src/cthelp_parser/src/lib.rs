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

const CT_MARKDOWN_CODE_FLAGS: &str = "```";

/// 解析首个Markdown代码块以生成使用字符串
///
/// 移除代码围栏，并将实用程序名称替换为`{}`，以便在运行时用适当名称替换。
pub fn ct_parse_usage(ct_content: &str) -> String {
    let mut ct_result = String::new();
    let mut ct_inside_code_block = false;

    for ct_line in ct_content.lines() {
        if !ct_inside_code_block {
            if ct_line.starts_with(CT_MARKDOWN_CODE_FLAGS) {
                ct_inside_code_block = true;
            }
        } else {
            if ct_line.starts_with(CT_MARKDOWN_CODE_FLAGS) {
                ct_inside_code_block = false;
                continue;
            }

            if let Some((_util, args)) = ct_line.split_once(' ') {
                ct_result.push_str(&format!("{{}} {}\n", args));
            } else {
                ct_result.push_str("{}\n");
            }
        }
    }

    ct_result.trim().to_string()
}

/// 跳过markdown 代码块，在下一个标题（如果有）如果下下标题之间的文本解析为一个 about 字符串。
pub fn ct_parse_about(ct_content: &str) -> String {
    let mut ct_lines = ct_content.lines();

    // 跳过行直到遇到第一个 Markdown
    for ct_line in ct_lines.by_ref() {
        if ct_line.starts_with(CT_MARKDOWN_CODE_FLAGS) {
            break;
        }
    }

    // 跳过空行
    let _ = ct_lines.next();

    // 跳过行直到遇到第二个 Markdown
    for ct_line in ct_lines.by_ref() {
        if ct_line.starts_with(CT_MARKDOWN_CODE_FLAGS) {
            break;
        }
    }

    // 在第二个 Markdown 后跳过一行
    let _ = ct_lines.next();

    // Take lines until a line starts with '#'
    let mut ct_about_content = String::new();
    for ct_line in ct_lines {
        if ct_line.starts_with('#') {
            break;
        }
        ct_about_content.push_str(ct_line);
        ct_about_content.push('\n');
    }

    ct_about_content.trim().to_string()
}

/// 从内容中获取单个章节
///
/// 章节必须为二级章节（即以 `##` 开头）。
pub fn ct_parse_section(ct_section: &str, ct_content: &str) -> Option<String> {
    let ct_sect = ct_section.to_lowercase();

    // Check if the section exists
    let mut ct_section_exists = false;
    for ct_line in ct_content.lines() {
        if let Some(ct_header) = ct_line.strip_prefix("##") {
            if ct_header.trim().to_lowercase() == ct_sect {
                ct_section_exists = true;
                break;
            }
        }
    }
    if !ct_section_exists {
        return None;
    }

    let ct_section_header_prefix = "## ";

    let mut ct_section_content = String::new();
    let mut ct_found_section = false;
    for ct_line in ct_content.lines() {
        if ct_found_section {
            if ct_line.starts_with(ct_section_header_prefix) {
                break;
            }
            ct_section_content.push_str(ct_line);
            ct_section_content.push('\n');
        } else if let Some(ct_header) = ct_line.strip_prefix("##") {
            if ct_header.trim().to_lowercase() == ct_sect {
                ct_found_section = true;
            }
        }
    }

    Some(ct_section_content.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ct_parse_section() {
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
            ct_parse_section("some arch section", input).unwrap(),
            expected_section1
        );
        assert_eq!(
            ct_parse_section("SOME ARCH SECTION", input).unwrap(),
            expected_section2
        );
        assert_eq!(
            ct_parse_section("another section", input).unwrap(),
            expected_section3
        );
    }

    #[test]
    fn test_ct_parse_section_with_sub_headers() {
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
            ct_parse_section("after section", input).unwrap(),
            expeted_section
        );
    }

    #[test]
    fn test_ct_parse_non_existing_section() {
        let input = r#"# ls
## some section
This is some section

## ANOTHER SECTION
            This is the other section
with multiple lines
"#;

        assert!(ct_parse_section("non-existing section", input).is_none());
    }

    #[test]
    fn test_ct_parse_usage() {
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

        assert_eq!(ct_parse_usage(input), "{} -l");
    }

    #[test]
    fn test_ct_parse_multi_line_usage() {
        let input = r#"# ls
```
ls -a
ls -b
ls -c
```
## some section
This is some section
"#;

        assert_eq!(ct_parse_usage(input), "{} -a\n{} -b\n{} -c");
    }

    #[test]
    fn test_ct_parse_about() {
        let input = r#"
# ll

```
ll -h
```

This is the about section for ll

## some section

This is some section for ll
"#;

        assert_eq!(ct_parse_about(input), "This is the about section for ll");
    }

    #[test]
    fn test_ct_parse_multi_line_about() {
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
            ct_parse_about(input),
            "about ctyunos22.09.1\n\nabout ctyunos22.09.2"
        );
    }
}
