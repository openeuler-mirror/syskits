/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 */

//! `cmd_from` - 将输入解析为结构化 `CtPipelineData`。
//!
//! 支持格式：
//! - `json`
//! - `jsonl`/`jsonlines`
//! - `csv`
//! - `yaml`/`yml`
//! - `toml`
//! - `text`

use ctengine::command::DataCommand;
use ctengine::context::DataEngineContext;
use ctengine::error::CtDiagnosticError;
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtFlag, CtPositionalArg, DataCall, DataSignature};
use std::io::Read;

#[derive(Default)]
pub struct CmdFrom;

impl DataCommand for CmdFrom {
    fn signature(&self) -> DataSignature {
        DataSignature::new("from", "parse input into structured CtPipelineData")
            .positional(CtPositionalArg::optional(
                "format",
                "json | jsonl | csv | yaml | toml | text",
                CtType::String,
            ))
            .positional(CtPositionalArg::optional(
                "source",
                "inline source string (optional)",
                CtType::String,
            ))
            .flag(CtFlag::switch(
                "help",
                Some('h'),
                "show help for `from` formats",
            ))
            .flag(CtFlag::switch(
                "objects",
                Some('o'),
                "for json: parse each non-empty line as a separate JSON value",
            ))
            .flag(CtFlag::switch(
                "strict",
                Some('s'),
                "for json: enforce strict JSON (no comments/trailing commas)",
            ))
            .flag(CtFlag::switch(
                "transpose",
                None,
                "for csv: parse field-oriented CSV where first column contains field names",
            ))
            .input(CtType::Any)
            .output(CtType::Any)
    }

    fn run(
        &self,
        call: &DataCall,
        input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        if call.has_flag("help") || call.has_flag("h") {
            return Ok(help_output());
        }

        let format = call
            .opt::<String>(0)
            .map_err(|e| CtDiagnosticError::simple(e.to_string()))?;
        let Some(format) = format else {
            return Ok(help_output());
        };
        let format = normalize_format(&format).ok_or_else(|| {
            CtDiagnosticError::simple(format!(
                "from: unsupported format `{format}`; supported: json, jsonl, csv, ssv, yaml, toml, text"
            ))
        })?;

        let source_arg: Option<String> = call
            .opt::<String>(1)
            .map_err(|e| CtDiagnosticError::simple(e.to_string()))?;
        let json_objects = call.has_flag("objects") || call.has_flag("o");
        let json_strict = call.has_flag("strict") || call.has_flag("s");
        let csv_transpose = call.has_flag("transpose");

        let raw = if let Some(src) = source_arg {
            src
        } else {
            match input {
                CtPipelineData::Empty => {
                    let mut buf = String::new();
                    std::io::stdin().read_to_string(&mut buf).map_err(|e| {
                        CtDiagnosticError::simple(format!("from: stdin read error: {e}"))
                    })?;
                    buf
                }
                other => pipeline_data_to_string(other)?,
            }
        };

        if raw.trim().is_empty() {
            return Err(CtDiagnosticError::simple(
                "from: no input (provide argument or pipe data into this command)",
            ));
        }

        let value = match format {
            "json" => parse_json(&raw, json_strict, json_objects)?,
            "jsonl" => parse_jsonl(&raw)?,
            "csv" if csv_transpose => parse_csv_transposed(&raw)?,
            "csv" => parse_csv(&raw)?,
            "ssv" => parse_ssv(&raw),
            "yaml" => parse_yaml(&raw)?,
            "toml" => parse_toml(&raw)?,
            "text" => parse_text(&raw),
            _ => unreachable!("format normalized"),
        };

        Ok(CtPipelineData::Value(value, CtPipelineMetadata::default()))
    }
}

fn normalize_format(input: &str) -> Option<&'static str> {
    match input.trim().to_ascii_lowercase().as_str() {
        "json" => Some("json"),
        "jsonl" | "jsonlines" => Some("jsonl"),
        "csv" => Some("csv"),
        "ssv" => Some("ssv"),
        "yaml" | "yml" => Some("yaml"),
        "toml" => Some("toml"),
        "text" => Some("text"),
        _ => None,
    }
}

fn help_output() -> CtPipelineData {
    let help = [
        "from: parse input into structured data",
        "",
        "Usage:",
        "  from <format> [source]",
        "",
        "Formats:",
        "  json                parse JSON text",
        "  jsonl | jsonlines   parse line-delimited JSON",
        "  csv                 parse CSV text (header row required)",
        "  ssv                 parse space-separated values (first line = headers)",
        "  yaml | yml          parse YAML text",
        "  toml                parse TOML text",
        "  text                split text into List<String> by lines",
        "",
        "JSON flags:",
        "  --objects, -o       parse each non-empty line as one JSON object/value",
        "  --strict, -s        strict JSON only (default is lenient json5-compatible parse)",
        "",
        "CSV flags:",
        "  --transpose         parse field-oriented CSV (first column = field names)",
        "",
        "Examples:",
        "  from json '{\"a\":1}'",
        "  from json --objects '{\"a\":1}\\n{\"b\":2}'",
        "  from json --strict '{\"a\":1}'",
        "  from jsonl '{\"a\":1}\\n{\"a\":2}'",
        "  from csv 'name,age\\nAlice,30'",
    ]
    .join("\n");
    CtPipelineData::Value(CtValue::String(help), CtPipelineMetadata::default())
}

fn parse_text(s: &str) -> CtValue {
    CtValue::List(
        s.lines()
            .map(|line| CtValue::String(line.to_string()))
            .collect(),
    )
}

fn parse_json(s: &str, strict: bool, objects: bool) -> Result<CtValue, CtDiagnosticError> {
    if objects {
        return parse_json_objects(s, strict);
    }
    parse_one_json_value(s, strict, None).map(json_value_to_ct)
}

fn parse_json_objects(s: &str, strict: bool) -> Result<CtValue, CtDiagnosticError> {
    let mut out = Vec::new();
    for (idx, line) in s.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value = parse_one_json_value(line, strict, Some(idx + 1))?;
        out.push(json_value_to_ct(value));
    }
    Ok(CtValue::List(out))
}

fn parse_one_json_value(
    s: &str,
    strict: bool,
    line_no: Option<usize>,
) -> Result<serde_json::Value, CtDiagnosticError> {
    if strict {
        return serde_json::from_str::<serde_json::Value>(s).map_err(|e| {
            if let Some(n) = line_no {
                CtDiagnosticError::simple(format!("from json: parse error at line {n}: {e}"))
            } else {
                CtDiagnosticError::simple(format!("from json: {e}"))
            }
        });
    }

    json5::from_str::<serde_json::Value>(s).map_err(|e| {
        if let Some(n) = line_no {
            CtDiagnosticError::simple(format!("from json: parse error at line {n}: {e}"))
        } else {
            CtDiagnosticError::simple(format!("from json: {e}"))
        }
    })
}

fn parse_jsonl(s: &str) -> Result<CtValue, CtDiagnosticError> {
    let mut out = Vec::new();
    for (idx, line) in s.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(line).map_err(|e| {
            CtDiagnosticError::simple(format!("from jsonl: parse error at line {}: {e}", idx + 1))
        })?;
        out.push(json_value_to_ct(v));
    }
    Ok(CtValue::List(out))
}

fn parse_csv(s: &str) -> Result<CtValue, CtDiagnosticError> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(s.as_bytes());

    let headers = reader
        .headers()
        .map_err(|e| CtDiagnosticError::simple(format!("from csv: cannot read headers: {e}")))?
        .iter()
        .map(|h| h.to_string())
        .collect::<Vec<_>>();
    validate_csv_field_names(&headers)?;

    let mut rows = Vec::new();
    for result in reader.records() {
        let record = result
            .map_err(|e| CtDiagnosticError::simple(format!("from csv: record error: {e}")))?;
        let mut fields = Vec::new();
        for (idx, value) in record.iter().enumerate() {
            let key = headers
                .get(idx)
                .cloned()
                .unwrap_or_else(|| format!("column{idx}"));
            fields.push((key, infer_scalar(value)));
        }
        rows.push(CtValue::Record(fields));
    }
    Ok(CtValue::List(rows))
}

fn validate_csv_field_names(headers: &[String]) -> Result<(), CtDiagnosticError> {
    let mut fields_seen = std::collections::HashSet::new();
    for (idx, header) in headers.iter().enumerate() {
        let header = header.trim();
        if header.is_empty() {
            return Err(CtDiagnosticError::simple(format!(
                "from csv: header name is empty at column {}",
                idx + 1
            )));
        }
        if !fields_seen.insert(header.to_string()) {
            return Err(CtDiagnosticError::simple(format!(
                "from csv: duplicate header `{header}`"
            )));
        }
    }
    Ok(())
}

fn parse_csv_transposed(s: &str) -> Result<CtValue, CtDiagnosticError> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(s.as_bytes());

    let mut rows: Vec<Vec<String>> = Vec::new();
    for result in reader.records() {
        let record = result
            .map_err(|e| CtDiagnosticError::simple(format!("from csv: record error: {e}")))?;
        if record.iter().all(|value| value.trim().is_empty()) {
            continue;
        }
        rows.push(
            record
                .iter()
                .map(|value| value.trim().to_string())
                .collect(),
        );
    }

    if rows.is_empty() {
        return Ok(CtValue::List(vec![]));
    }

    let max_cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let record_count = max_cols.saturating_sub(1);
    let mut records: Vec<Vec<(String, CtValue)>> = vec![Vec::new(); record_count];
    let mut fields_seen = std::collections::HashSet::new();

    for (row_idx, row) in rows.iter().enumerate() {
        let field_name = row.first().map(String::as_str).unwrap_or("").trim();
        if field_name.is_empty() {
            return Err(CtDiagnosticError::simple(format!(
                "from csv: transpose field name is empty at row {}",
                row_idx + 1
            )));
        }
        if !fields_seen.insert(field_name.to_string()) {
            return Err(CtDiagnosticError::simple(format!(
                "from csv: duplicate transpose field `{field_name}`"
            )));
        }

        for record_idx in 0..record_count {
            let value = row
                .get(record_idx + 1)
                .map(String::as_str)
                .unwrap_or("")
                .trim();
            records[record_idx].push((field_name.to_string(), infer_scalar(value)));
        }
    }

    Ok(CtValue::List(
        records.into_iter().map(CtValue::Record).collect(),
    ))
}

fn parse_yaml(s: &str) -> Result<CtValue, CtDiagnosticError> {
    let v: serde_yaml::Value = serde_yaml::from_str(s)
        .map_err(|e| CtDiagnosticError::simple(format!("from yaml: {e}")))?;
    Ok(yaml_value_to_ct(v))
}

fn parse_toml(s: &str) -> Result<CtValue, CtDiagnosticError> {
    let v: toml::Value =
        toml::from_str(s).map_err(|e| CtDiagnosticError::simple(format!("from toml: {e}")))?;
    Ok(toml_value_to_ct(v))
}

/// 解析空白分隔值（Space-Separated Values）。
/// 第一行作为列名，后续行按连续空白分割为字段。
fn parse_ssv(s: &str) -> CtValue {
    let mut lines = s.lines().filter(|l| !l.trim().is_empty());
    let Some(header_line) = lines.next() else {
        return CtValue::List(vec![]);
    };
    let headers: Vec<&str> = header_line.split_whitespace().collect();
    let mut rows = Vec::new();
    for line in lines {
        let cols: Vec<&str> = line.split_whitespace().collect();
        let mut fields = Vec::new();
        for (idx, header) in headers.iter().enumerate() {
            let val = cols.get(idx).copied().unwrap_or("");
            fields.push((header.to_string(), infer_scalar(val)));
        }
        // 如果行有更多列，用 column{N} 命名
        for (idx, col) in cols.iter().enumerate().skip(headers.len()) {
            fields.push((format!("column{idx}"), infer_scalar(col)));
        }
        rows.push(CtValue::Record(fields));
    }
    CtValue::List(rows)
}

fn infer_scalar(raw: &str) -> CtValue {
    let s = raw.trim();
    if s.is_empty() {
        return CtValue::Nothing;
    }
    if s.eq_ignore_ascii_case("true") {
        return CtValue::Bool(true);
    }
    if s.eq_ignore_ascii_case("false") {
        return CtValue::Bool(false);
    }
    if let Ok(n) = s.parse::<i64>() {
        return CtValue::Int(n);
    }
    if let Ok(n) = s.parse::<f64>() {
        return CtValue::Float(n);
    }
    CtValue::String(raw.to_string())
}

fn json_value_to_ct(v: serde_json::Value) -> CtValue {
    match v {
        serde_json::Value::Null => CtValue::Nothing,
        serde_json::Value::Bool(b) => CtValue::Bool(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                CtValue::Int(i)
            } else {
                CtValue::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_json::Value::String(s) => CtValue::String(s),
        serde_json::Value::Array(arr) => {
            CtValue::List(arr.into_iter().map(json_value_to_ct).collect())
        }
        serde_json::Value::Object(obj) => CtValue::Record(
            obj.into_iter()
                .map(|(k, v)| (k, json_value_to_ct(v)))
                .collect(),
        ),
    }
}

fn yaml_value_to_ct(v: serde_yaml::Value) -> CtValue {
    match v {
        serde_yaml::Value::Null => CtValue::Nothing,
        serde_yaml::Value::Bool(b) => CtValue::Bool(b),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                CtValue::Int(i)
            } else if let Some(u) = n.as_u64() {
                if u > i64::MAX as u64 {
                    CtValue::Float(u as f64)
                } else {
                    CtValue::Int(u as i64)
                }
            } else {
                CtValue::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_yaml::Value::String(s) => CtValue::String(s),
        serde_yaml::Value::Sequence(items) => {
            CtValue::List(items.into_iter().map(yaml_value_to_ct).collect())
        }
        serde_yaml::Value::Mapping(map) => CtValue::Record(
            map.into_iter()
                .map(|(k, v)| (yaml_key_to_string(k), yaml_value_to_ct(v)))
                .collect(),
        ),
        serde_yaml::Value::Tagged(tagged) => yaml_value_to_ct(tagged.value),
    }
}

fn yaml_key_to_string(v: serde_yaml::Value) -> String {
    match v {
        serde_yaml::Value::String(s) => s,
        serde_yaml::Value::Bool(b) => b.to_string(),
        serde_yaml::Value::Number(n) => n.to_string(),
        serde_yaml::Value::Null => "null".to_string(),
        other => format!("{other:?}"),
    }
}

fn toml_value_to_ct(v: toml::Value) -> CtValue {
    match v {
        toml::Value::String(s) => CtValue::String(s),
        toml::Value::Integer(i) => CtValue::Int(i),
        toml::Value::Float(f) => CtValue::Float(f),
        toml::Value::Boolean(b) => CtValue::Bool(b),
        toml::Value::Datetime(dt) => CtValue::String(dt.to_string()),
        toml::Value::Array(items) => {
            CtValue::List(items.into_iter().map(toml_value_to_ct).collect())
        }
        toml::Value::Table(map) => CtValue::Record(
            map.into_iter()
                .map(|(k, v)| (k, toml_value_to_ct(v)))
                .collect(),
        ),
    }
}

fn pipeline_data_to_string(data: CtPipelineData) -> Result<String, CtDiagnosticError> {
    match data {
        CtPipelineData::Empty => Ok(String::new()),
        CtPipelineData::Value(CtValue::String(s), _) => Ok(s),
        CtPipelineData::Value(v, _) => Ok(format!("{v:?}")),
        CtPipelineData::ListStream(stream) => {
            let lines: Vec<String> = stream
                .map(|v| {
                    if let CtValue::String(s) = v {
                        s
                    } else {
                        format!("{v:?}")
                    }
                })
                .collect();
            Ok(lines.join("\n"))
        }
        CtPipelineData::ByteStream(mut bs) => {
            let mut buf = String::new();
            bs.read_to_string(&mut buf)
                .map_err(|e| CtDiagnosticError::simple(format!("from: io error: {e}")))?;
            Ok(buf)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctengine::context::{CommandRegistry, DataEngineContext};

    fn ctx() -> DataEngineContext {
        DataEngineContext::new(CommandRegistry::empty(), None, None)
    }

    fn call_fmt(fmt: &str) -> DataCall {
        let mut c = DataCall::empty();
        c.positionals
            .push(ctsig::BoundArg::new(CtValue::String(fmt.to_string()), None));
        c
    }

    fn call_fmt_src(fmt: &str, src: &str) -> DataCall {
        let mut c = call_fmt(fmt);
        c.positionals
            .push(ctsig::BoundArg::new(CtValue::String(src.to_string()), None));
        c
    }

    fn call_fmt_src_transpose(fmt: &str, src: &str) -> DataCall {
        let mut c = call_fmt_src(fmt, src);
        c.flags.insert("transpose".to_string(), None);
        c
    }

    fn str_input(s: &str) -> CtPipelineData {
        CtPipelineData::Value(
            CtValue::String(s.to_string()),
            CtPipelineMetadata::default(),
        )
    }

    #[test]
    fn test_from_json_object() {
        let r = CmdFrom
            .run(&call_fmt("json"), str_input(r#"{"name":"Alice"}"#), &ctx())
            .unwrap();
        assert!(matches!(r, CtPipelineData::Value(CtValue::Record(_), _)));
    }

    #[test]
    fn test_from_jsonl() {
        let r = CmdFrom
            .run(
                &call_fmt("jsonl"),
                str_input("{\"a\":1}\n{\"a\":2}"),
                &ctx(),
            )
            .unwrap();
        let CtPipelineData::Value(CtValue::List(items), _) = r else {
            panic!("expected list");
        };
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn test_from_csv() {
        let r = CmdFrom
            .run(
                &call_fmt("csv"),
                str_input("name,age,ok\nalice,30,true\nbob,20,false"),
                &ctx(),
            )
            .unwrap();
        let CtPipelineData::Value(CtValue::List(items), _) = r else {
            panic!("expected list");
        };
        assert_eq!(items.len(), 2);
        let CtValue::Record(first) = &items[0] else {
            panic!("record");
        };
        assert!(matches!(
            first.iter().find(|(k, _)| k == "age").map(|(_, v)| v),
            Some(CtValue::Int(30))
        ));
    }

    #[test]
    fn test_from_csv_rejects_empty_header() {
        let err = CmdFrom
            .run(&call_fmt("csv"), str_input("name,\nchr,12"), &ctx())
            .unwrap_err();
        assert!(err.to_string().contains("header name is empty"));
    }

    #[test]
    fn test_from_csv_rejects_duplicate_header() {
        let err = CmdFrom
            .run(&call_fmt("csv"), str_input("name,name\nchr,chr"), &ctx())
            .unwrap_err();
        assert!(err.to_string().contains("duplicate header"));
    }

    #[test]
    fn test_from_csv_transpose() {
        let r = CmdFrom
            .run(
                &call_fmt_src_transpose("csv", "name,a,b\nage,12,11\ngender,male,female"),
                CtPipelineData::Empty,
                &ctx(),
            )
            .unwrap();
        let CtPipelineData::Value(CtValue::List(items), _) = r else {
            panic!("expected list");
        };
        assert_eq!(items.len(), 2);
        let CtValue::Record(first) = &items[0] else {
            panic!("record");
        };
        assert!(matches!(
            first.iter().find(|(k, _)| k == "name").map(|(_, v)| v),
            Some(CtValue::String(s)) if s == "a"
        ));
        assert!(matches!(
            first.iter().find(|(k, _)| k == "age").map(|(_, v)| v),
            Some(CtValue::Int(12))
        ));
        assert!(matches!(
            first.iter().find(|(k, _)| k == "gender").map(|(_, v)| v),
            Some(CtValue::String(s)) if s == "male"
        ));
    }

    #[test]
    fn test_from_csv_transpose_trims_cells() {
        let r = CmdFrom
            .run(
                &call_fmt_src_transpose("csv", "name, a, b\nage, 12, 11"),
                CtPipelineData::Empty,
                &ctx(),
            )
            .unwrap();
        let CtPipelineData::Value(CtValue::List(items), _) = r else {
            panic!("expected list");
        };
        let CtValue::Record(first) = &items[0] else {
            panic!("record");
        };
        assert!(matches!(
            first.iter().find(|(k, _)| k == "name").map(|(_, v)| v),
            Some(CtValue::String(s)) if s == "a"
        ));
    }

    #[test]
    fn test_from_csv_transpose_short_rows_become_nothing() {
        let r = CmdFrom
            .run(
                &call_fmt_src_transpose("csv", "name,a,b\nage,12"),
                CtPipelineData::Empty,
                &ctx(),
            )
            .unwrap();
        let CtPipelineData::Value(CtValue::List(items), _) = r else {
            panic!("expected list");
        };
        let CtValue::Record(second) = &items[1] else {
            panic!("record");
        };
        assert!(matches!(
            second.iter().find(|(k, _)| k == "age").map(|(_, v)| v),
            Some(CtValue::Nothing)
        ));
    }

    #[test]
    fn test_from_csv_transpose_extra_columns_create_records() {
        let r = CmdFrom
            .run(
                &call_fmt_src_transpose("csv", "name,a,b,c\nage,12,11,10"),
                CtPipelineData::Empty,
                &ctx(),
            )
            .unwrap();
        let CtPipelineData::Value(CtValue::List(items), _) = r else {
            panic!("expected list");
        };
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn test_from_csv_transpose_rejects_empty_field_name() {
        let err = CmdFrom
            .run(
                &call_fmt_src_transpose("csv", "name,a,b\n,12,11"),
                CtPipelineData::Empty,
                &ctx(),
            )
            .unwrap_err();
        assert!(err.to_string().contains("field name is empty"));
    }

    #[test]
    fn test_from_csv_transpose_rejects_duplicate_field_name() {
        let err = CmdFrom
            .run(
                &call_fmt_src_transpose("csv", "name,a,b\nname,c,d"),
                CtPipelineData::Empty,
                &ctx(),
            )
            .unwrap_err();
        assert!(err.to_string().contains("duplicate transpose field"));
    }

    #[test]
    fn test_from_yaml() {
        let r = CmdFrom
            .run(
                &call_fmt("yaml"),
                str_input("a: 1\nb:\n  - 2\n  - 3"),
                &ctx(),
            )
            .unwrap();
        assert!(matches!(r, CtPipelineData::Value(CtValue::Record(_), _)));
    }

    #[test]
    fn test_from_toml() {
        let r = CmdFrom
            .run(&call_fmt("toml"), str_input("a = 1\nb = \"x\""), &ctx())
            .unwrap();
        assert!(matches!(r, CtPipelineData::Value(CtValue::Record(_), _)));
    }

    #[test]
    fn test_from_text() {
        let r = CmdFrom
            .run(&call_fmt("text"), str_input("a\nb\nc"), &ctx())
            .unwrap();
        if let CtPipelineData::Value(CtValue::List(items), _) = r {
            assert_eq!(items.len(), 3);
        } else {
            panic!("expected List");
        }
    }

    #[test]
    fn test_from_invalid_format() {
        let e = CmdFrom
            .run(&call_fmt("xml"), CtPipelineData::Empty, &ctx())
            .unwrap_err();
        assert!(e.to_string().contains("unsupported format"));
    }

    #[test]
    fn test_from_json_invalid() {
        let e = CmdFrom
            .run(&call_fmt("json"), str_input("not json"), &ctx())
            .unwrap_err();
        assert!(e.to_string().contains("from json"));
    }

    #[test]
    fn test_from_json_objects_flag() {
        let mut c = DataCall::named("from");
        c.positionals.push(ctsig::BoundArg::new(
            CtValue::String("json".to_string()),
            None,
        ));
        c.positionals.push(ctsig::BoundArg::new(
            CtValue::String("{\"a\":1}\n{\"b\":2}".to_string()),
            None,
        ));
        c.flags.insert("objects".to_string(), None);
        let r = CmdFrom.run(&c, CtPipelineData::Empty, &ctx()).unwrap();
        let CtPipelineData::Value(CtValue::List(items), _) = r else {
            panic!("expected list");
        };
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn test_from_json_non_strict_allows_trailing_comma() {
        let r = CmdFrom
            .run(
                &call_fmt_src("json", "{a:1,}"),
                CtPipelineData::Empty,
                &ctx(),
            )
            .unwrap();
        assert!(matches!(r, CtPipelineData::Value(CtValue::Record(_), _)));
    }

    #[test]
    fn test_from_json_strict_rejects_trailing_comma() {
        let mut c = DataCall::named("from");
        c.positionals.push(ctsig::BoundArg::new(
            CtValue::String("json".to_string()),
            None,
        ));
        c.positionals.push(ctsig::BoundArg::new(
            CtValue::String("{\"a\":1,}".to_string()),
            None,
        ));
        c.flags.insert("strict".to_string(), None);

        let e = CmdFrom.run(&c, CtPipelineData::Empty, &ctx()).unwrap_err();
        assert!(e.to_string().contains("from json"));
    }

    #[test]
    fn test_from_json_inline_source() {
        let r = CmdFrom
            .run(
                &call_fmt_src("json", r#"{"name":"inline"}"#),
                CtPipelineData::Empty,
                &ctx(),
            )
            .unwrap();
        assert!(matches!(r, CtPipelineData::Value(CtValue::Record(_), _)));
    }

    #[test]
    fn test_from_empty_default_fails() {
        assert!(
            CmdFrom
                .run(&call_fmt_src("json", ""), CtPipelineData::Empty, &ctx())
                .is_err()
        );
    }

    #[test]
    fn test_from_without_format_returns_help() {
        let out = CmdFrom
            .run(&DataCall::empty(), CtPipelineData::Empty, &ctx())
            .unwrap();
        let CtPipelineData::Value(CtValue::String(help), _) = out else {
            panic!("expected help text");
        };
        assert!(help.contains("Usage:"));
        assert!(help.contains("from <format> [source]"));
    }

    #[test]
    fn test_from_help_flag_returns_help() {
        let mut c = DataCall::named("from");
        c.flags.insert("help".to_string(), None);
        let out = CmdFrom.run(&c, CtPipelineData::Empty, &ctx()).unwrap();
        let CtPipelineData::Value(CtValue::String(help), _) = out else {
            panic!("expected help text");
        };
        assert!(help.contains("Formats:"));
    }

    #[test]
    fn test_from_ssv_basic() {
        let input = "NAME   PID  CPU\nalice  123  0.5\nbob    456  1.2";
        let r = CmdFrom
            .run(&call_fmt("ssv"), str_input(input), &ctx())
            .unwrap();
        let CtPipelineData::Value(CtValue::List(items), _) = r else {
            panic!("expected list");
        };
        assert_eq!(items.len(), 2);
        let CtValue::Record(first) = &items[0] else {
            panic!("record");
        };
        assert!(matches!(
            first.iter().find(|(k, _)| k == "NAME").map(|(_, v)| v),
            Some(CtValue::String(s)) if s == "alice"
        ));
        assert!(matches!(
            first.iter().find(|(k, _)| k == "PID").map(|(_, v)| v),
            Some(CtValue::Int(123))
        ));
        assert!(matches!(
            first.iter().find(|(k, _)| k == "CPU").map(|(_, v)| v),
            Some(CtValue::Float(f)) if (*f - 0.5).abs() < 1e-9
        ));
    }

    #[test]
    fn test_from_ssv_empty() {
        let r = CmdFrom.run(&call_fmt_src("ssv", "  "), CtPipelineData::Empty, &ctx());
        assert!(r.is_err()); // empty input is an error
    }
}
