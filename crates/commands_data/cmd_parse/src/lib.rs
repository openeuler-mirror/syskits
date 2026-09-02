/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 */

//! `cmd_parse` — 将文本按模板或正则解析为结构化记录。
//!
//! - 模板模式（默认）：`parse "{name} {version}"`
//! - 正则模式（自动或 `--regex` 强制）：支持命名分组 `(?P<name>...)`

use ctengine::command::DataCommand;
use ctengine::context::DataEngineContext;
use ctengine::error::CtDiagnosticError;
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtFlag, CtPositionalArg, DataCall, DataSignature};
use regex::Regex;
use std::io::Read;

#[derive(Default)]
pub struct CmdParse;

const PARSE_HELP: &str = r#"syskits data parse

This is the syskits structured data pipeline parse command.
It parses text input into structured records with a template or regex pattern.

Usage:
  parse <pattern>
  parse --regex <pattern>
  parse --help
  parse --version

Examples:
  from text 'alice 30' | parse '{name} {age}'
  from text 'pid=123' | parse --regex 'pid=(?P<pid>[0-9]+)'
"#;

impl DataCommand for CmdParse {
    fn signature(&self) -> DataSignature {
        DataSignature::new("parse", "parse text into structured records")
            .positional(CtPositionalArg::required(
                "pattern",
                "template like '{name} {version}' or regex with captures",
                CtType::String,
            ))
            .flag(CtFlag::switch(
                "regex",
                Some('r'),
                "force regex mode for the given pattern",
            ))
            .flag(CtFlag::switch(
                "help",
                Some('h'),
                "show help for syskits data parse",
            ))
            .flag(CtFlag::switch(
                "version",
                None,
                "show syskits data parse version",
            ))
            .input(CtType::Any)
            .output(CtType::List)
    }

    fn run(
        &self,
        call: &DataCall,
        input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        if call.has_flag("help") || call.has_flag("h") {
            return Ok(meta_text_output(PARSE_HELP.to_string()));
        }
        if call.has_flag("version") {
            return Ok(meta_text_output(format!(
                "syskits data parse {}",
                env!("CARGO_PKG_VERSION")
            )));
        }

        let pattern = call
            .req::<String>(0)
            .map_err(|e| CtDiagnosticError::simple(format!("parse: {e}")))?;
        let force_regex = call.has_flag("regex") || call.has_flag("r");

        let regex = if force_regex {
            compile_regex(&pattern)?
        } else if is_template_pattern(&pattern) {
            compile_template(&pattern)?
        } else {
            compile_regex(&pattern)?
        };

        let (lines, meta) = collect_lines(input)?;
        let mut out = Vec::new();
        for line in lines {
            if let Some(record) = capture_to_record(&regex, &line) {
                out.push(record);
            }
        }

        Ok(CtPipelineData::Value(CtValue::List(out), meta))
    }
}

fn meta_text_output(text: String) -> CtPipelineData {
    CtPipelineData::Value(
        CtValue::String(text.clone()),
        CtPipelineMetadata {
            classic_text: Some(text),
            classic_bytes: None,
            classic_append_newline: false,
            exit_code: 0,
            source: Some("parse".into()),
            ..Default::default()
        },
    )
}

fn compile_regex(pattern: &str) -> Result<Regex, CtDiagnosticError> {
    Regex::new(pattern)
        .map_err(|e| CtDiagnosticError::simple(format!("parse: invalid regex pattern: {e}")))
}

fn is_template_pattern(pattern: &str) -> bool {
    let bytes = pattern.as_bytes();
    let mut search_from = 0usize;
    while let Some(open_rel) = pattern[search_from..].find('{') {
        let open = search_from + open_rel;
        if is_escaped_open_brace(bytes, open) || is_regex_unicode_property_open(bytes, open) {
            search_from = open + 1;
            continue;
        }
        let Some(close_rel) = pattern[open + 1..].find('}') else {
            break;
        };
        let close = open + 1 + close_rel;
        let inner = pattern[open + 1..close].trim();
        if inner
            .chars()
            .next()
            .is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
        {
            return true;
        }
        search_from = close + 1;
    }
    false
}

fn is_escaped_open_brace(pattern_bytes: &[u8], open_idx: usize) -> bool {
    if open_idx == 0 {
        return false;
    }
    let mut slash_count = 0usize;
    let mut idx = open_idx;
    while idx > 0 && pattern_bytes[idx - 1] == b'\\' {
        slash_count += 1;
        idx -= 1;
    }
    slash_count % 2 == 1
}

fn is_regex_unicode_property_open(pattern_bytes: &[u8], open_idx: usize) -> bool {
    open_idx >= 2
        && pattern_bytes[open_idx - 2] == b'\\'
        && matches!(pattern_bytes[open_idx - 1], b'p' | b'P')
}

fn compile_template(template: &str) -> Result<Regex, CtDiagnosticError> {
    let mut regex = String::from("^");
    let mut last = 0usize;
    let mut names = std::collections::BTreeSet::new();
    let mut chars = template.char_indices().peekable();

    while let Some((idx, ch)) = chars.next() {
        if ch == '{' {
            if last < idx {
                regex.push_str(&regex::escape(&template[last..idx]));
            }

            let name_start = idx + 1;
            let mut end = None;
            for (j, c) in chars.by_ref() {
                if c == '}' {
                    end = Some(j);
                    break;
                }
            }

            let end_idx = end.ok_or_else(|| {
                CtDiagnosticError::simple("parse: template has unmatched '{'".to_string())
            })?;
            let name = template[name_start..end_idx].trim();
            if !is_valid_capture_name(name) {
                return Err(CtDiagnosticError::simple(format!(
                    "parse: invalid template field name `{name}`"
                )));
            }
            if !names.insert(name.to_string()) {
                return Err(CtDiagnosticError::simple(format!(
                    "parse: duplicated template field name `{name}`"
                )));
            }

            regex.push_str("(?P<");
            regex.push_str(name);
            regex.push_str(">.+?)");
            last = end_idx + 1;
            continue;
        }

        if ch == '}' {
            return Err(CtDiagnosticError::simple(
                "parse: template has unmatched '}'".to_string(),
            ));
        }
    }

    if last < template.len() {
        regex.push_str(&regex::escape(&template[last..]));
    }
    regex.push('$');

    compile_regex(&regex)
}

fn is_valid_capture_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn collect_lines(
    input: CtPipelineData,
) -> Result<(Vec<String>, CtPipelineMetadata), CtDiagnosticError> {
    match input {
        CtPipelineData::Empty => Ok((Vec::new(), CtPipelineMetadata::default())),
        CtPipelineData::Value(v, meta) => Ok((value_to_lines(v), meta)),
        CtPipelineData::ListStream(stream) => {
            let meta = stream.metadata.clone();
            let lines = stream.map(|v| v.to_text()).collect::<Vec<_>>();
            Ok((lines, meta))
        }
        CtPipelineData::ByteStream(mut stream) => {
            let meta = stream.metadata.clone();
            let mut buf = Vec::new();
            stream.read_to_end(&mut buf).map_err(|e| {
                CtDiagnosticError::simple(format!("parse: failed to read byte stream: {e}"))
            })?;
            let text = String::from_utf8(buf).map_err(|_| {
                CtDiagnosticError::simple("parse: input byte stream is not valid UTF-8")
            })?;
            Ok((
                text.lines().map(|s| s.to_string()).collect::<Vec<_>>(),
                meta,
            ))
        }
    }
}

fn value_to_lines(v: CtValue) -> Vec<String> {
    match v {
        CtValue::String(s) => s.lines().map(|line| line.to_string()).collect::<Vec<_>>(),
        CtValue::List(items) => items.into_iter().map(|item| item.to_text()).collect(),
        other => vec![other.to_text()],
    }
}

fn capture_to_record(regex: &Regex, line: &str) -> Option<CtValue> {
    let captures = regex.captures(line)?;
    let mut fields = Vec::new();

    let named: Vec<&str> = regex.capture_names().flatten().collect();
    if !named.is_empty() {
        for name in named {
            let value = captures
                .name(name)
                .map(|m| CtValue::String(m.as_str().to_string()))
                .unwrap_or(CtValue::Nothing);
            fields.push((name.to_string(), value));
        }
        return Some(CtValue::Record(fields));
    }

    for idx in 1..captures.len() {
        let value = captures
            .get(idx)
            .map(|m| CtValue::String(m.as_str().to_string()))
            .unwrap_or(CtValue::Nothing);
        fields.push((format!("capture{idx}"), value));
    }

    if fields.is_empty() {
        let text = captures
            .get(0)
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();
        fields.push(("match".to_string(), CtValue::String(text)));
    }

    Some(CtValue::Record(fields))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctengine::context::{CommandRegistry, DataEngineContext};
    use ctpipeline::CtListStream;
    use ctsig::BoundArg;
    use std::io::Cursor;

    fn ctx() -> DataEngineContext {
        DataEngineContext::new(CommandRegistry::empty(), None, None)
    }

    fn call(pattern: &str, regex_mode: bool) -> DataCall {
        let mut c = DataCall::named("parse");
        c.positionals
            .push(BoundArg::new(CtValue::String(pattern.to_string()), None));
        if regex_mode {
            c.flags.insert("regex".to_string(), None);
        }
        c
    }

    fn extract_field(record: &CtValue, name: &str) -> String {
        let CtValue::Record(fields) = record else {
            panic!("expected record");
        };
        let (_, value) = fields
            .iter()
            .find(|(k, _)| k == name)
            .expect("field exists");
        let CtValue::String(s) = value else {
            panic!("expected string field");
        };
        s.clone()
    }

    #[test]
    fn parse_template_records() {
        let input = CtPipelineData::Value(
            CtValue::String("nu 0.100\nbash 5.2".to_string()),
            CtPipelineMetadata::default(),
        );
        let out = CmdParse
            .run(&call("{shell} {version}", false), input, &ctx())
            .unwrap();
        let CtPipelineData::Value(CtValue::List(rows), _) = out else {
            panic!("expected list");
        };
        assert_eq!(rows.len(), 2);
        assert_eq!(extract_field(&rows[0], "shell"), "nu");
        assert_eq!(extract_field(&rows[0], "version"), "0.100");
    }

    #[test]
    fn parse_regex_named_captures() {
        let stream = CtListStream::new(
            vec![
                CtValue::String("pod-1".to_string()),
                CtValue::String("bad".to_string()),
                CtValue::String("api-42".to_string()),
            ]
            .into_iter(),
            CtPipelineMetadata::default(),
        );
        let input = CtPipelineData::ListStream(stream);
        let out = CmdParse
            .run(
                &call(r"^(?P<name>[a-z]+)-(?P<id>\d+)$", false),
                input,
                &ctx(),
            )
            .unwrap();
        let CtPipelineData::Value(CtValue::List(rows), _) = out else {
            panic!("expected list");
        };
        assert_eq!(rows.len(), 2);
        assert_eq!(extract_field(&rows[1], "name"), "api");
        assert_eq!(extract_field(&rows[1], "id"), "42");
    }

    #[test]
    fn parse_regex_unnamed_captures() {
        let input = CtPipelineData::Value(
            CtValue::String("svc-7".to_string()),
            CtPipelineMetadata::default(),
        );
        let out = CmdParse
            .run(&call(r"^([a-z]+)-(\d+)$", true), input, &ctx())
            .unwrap();
        let CtPipelineData::Value(CtValue::List(rows), _) = out else {
            panic!("expected list");
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(extract_field(&rows[0], "capture1"), "svc");
        assert_eq!(extract_field(&rows[0], "capture2"), "7");
    }

    #[test]
    fn parse_from_byte_stream() {
        let bytes = Cursor::new(b"nginx 1.27\nredis 7.2\n".to_vec());
        let input = CtPipelineData::ByteStream(ctpipeline::CtByteStream::new(
            bytes,
            CtPipelineMetadata::default(),
        ));
        let out = CmdParse
            .run(&call("{name} {version}", false), input, &ctx())
            .unwrap();
        let CtPipelineData::Value(CtValue::List(rows), _) = out else {
            panic!("expected list");
        };
        assert_eq!(rows.len(), 2);
        assert_eq!(extract_field(&rows[0], "name"), "nginx");
    }

    #[test]
    fn parse_invalid_template_fails() {
        let input = CtPipelineData::Value(
            CtValue::String("demo".to_string()),
            CtPipelineMetadata::default(),
        );
        let err = CmdParse
            .run(&call("{bad-name}", false), input, &ctx())
            .expect_err("expected template parse error");
        assert!(err.to_string().contains("invalid template field name"));
    }

    #[test]
    fn parse_invalid_regex_fails() {
        let input = CtPipelineData::Value(
            CtValue::String("demo".to_string()),
            CtPipelineMetadata::default(),
        );
        let err = CmdParse
            .run(&call("([", true), input, &ctx())
            .expect_err("expected regex parse error");
        assert!(err.to_string().contains("invalid regex pattern"));
    }

    #[test]
    fn parse_auto_mode_keeps_regex_quantifiers() {
        let input = CtPipelineData::Value(
            CtValue::String("abc\na\n".to_string()),
            CtPipelineMetadata::default(),
        );
        let out = CmdParse
            .run(&call(r"^\w{3}$", false), input, &ctx())
            .expect("regex quantifier should work in auto mode");
        let CtPipelineData::Value(CtValue::List(rows), _) = out else {
            panic!("expected list");
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(extract_field(&rows[0], "match"), "abc");
    }

    #[test]
    fn parse_auto_mode_keeps_unicode_property_regex() {
        let input = CtPipelineData::Value(
            CtValue::String("abc\n123\n".to_string()),
            CtPipelineMetadata::default(),
        );
        let out = CmdParse
            .run(&call(r"^\p{L}+$", false), input, &ctx())
            .expect("unicode property regex should work in auto mode");
        let CtPipelineData::Value(CtValue::List(rows), _) = out else {
            panic!("expected list");
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(extract_field(&rows[0], "match"), "abc");
    }

    #[test]
    fn parse_auto_mode_keeps_escaped_brace_regex() {
        let input = CtPipelineData::Value(
            CtValue::String("{foo}\nbar\n".to_string()),
            CtPipelineMetadata::default(),
        );
        let out = CmdParse
            .run(&call(r"^\{foo\}$", false), input, &ctx())
            .expect("escaped-brace regex should stay in regex mode");
        let CtPipelineData::Value(CtValue::List(rows), _) = out else {
            panic!("expected list");
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(extract_field(&rows[0], "match"), "{foo}");
    }
}
