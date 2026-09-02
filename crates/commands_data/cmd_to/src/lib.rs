/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 */

//! `cmd_to` - 将结构化数据序列化为文本格式。
//!
//! 支持格式：
//! - `json`
//! - `jsonl`/`jsonlines`
//! - `csv`
//! - `ssv`
//! - `yaml`/`yml`
//! - `toml`
//! - `text`

use ctengine::command::DataCommand;
use ctengine::context::DataEngineContext;
use ctengine::error::CtDiagnosticError;
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtFlag, CtPositionalArg, DataCall, DataSignature};

#[derive(Default)]
pub struct CmdTo;

impl DataCommand for CmdTo {
    fn signature(&self) -> DataSignature {
        DataSignature::new("to", "serialize CtPipelineData to text formats")
            .positional(CtPositionalArg::optional(
                "format",
                "json | jsonl | csv | ssv | yaml | toml | text",
                CtType::String,
            ))
            .flag(CtFlag::switch(
                "help",
                Some('h'),
                "show help for `to` formats",
            ))
            .flag(CtFlag::switch(
                "version",
                None,
                "show syskits data to version",
            ))
            .flag(CtFlag::switch(
                "transpose",
                None,
                "for csv/ssv: serialize records as field-oriented data",
            ))
            .input(CtType::Any)
            .output(CtType::String)
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
        if call.has_flag("version") {
            return Ok(meta_text_output(
                "to",
                format!("syskits data to {}", env!("CARGO_PKG_VERSION")),
            ));
        }

        let format = call
            .opt::<String>(0)
            .map_err(|e| CtDiagnosticError::simple(format!("to: {e}")))?;
        let Some(format) = format else {
            return Ok(help_output());
        };
        let format = normalize_format(&format).ok_or_else(|| {
            CtDiagnosticError::simple(format!(
                "to: unsupported format `{format}`; supported: json, jsonl, csv, ssv, yaml, toml, text"
            ))
        })?;

        let transpose = call.has_flag("transpose");

        let out = match format {
            "json" => {
                let value = consume_to_value(input)?;
                serde_json::to_string_pretty(&ct_to_json(&value))
                    .map_err(|e| CtDiagnosticError::simple(format!("to json: {e}")))?
            }
            "jsonl" => to_jsonl(input)?,
            "csv" if transpose => to_csv_transposed(input)?,
            "csv" => to_csv(input)?,
            "ssv" if transpose => to_ssv_transposed(input)?,
            "ssv" => to_ssv(input)?,
            "yaml" => {
                let value = consume_to_value(input)?;
                if is_empty_record_value(&value) {
                    return Ok(CtPipelineData::Value(
                        CtValue::String(String::new()),
                        CtPipelineMetadata::default(),
                    ));
                }
                serde_yaml::to_string(&ct_to_json(&value))
                    .map_err(|e| CtDiagnosticError::simple(format!("to yaml: {e}")))?
            }
            "toml" => {
                let value = consume_to_value(input)?;
                if is_empty_record_or_list_value(&value) {
                    return Ok(CtPipelineData::Value(
                        CtValue::String(String::new()),
                        CtPipelineMetadata::default(),
                    ));
                }
                let tv = ct_to_toml(&value);
                tv.to_string()
            }
            "text" => {
                let lines = consume_to_lines(input)?;
                lines.join("\n")
            }
            _ => unreachable!("format normalized"),
        };

        Ok(CtPipelineData::Value(
            CtValue::String(out),
            CtPipelineMetadata::default(),
        ))
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
        "syskits data to",
        "",
        "This is the syskits structured data pipeline to command.",
        "It serializes structured data into text formats.",
        "",
        "Usage:",
        "  to <format>",
        "",
        "Formats:",
        "  json                serialize to pretty JSON text",
        "  jsonl | jsonlines   serialize list items as JSON lines",
        "  csv                 serialize Record/List<Record> to CSV",
        "  ssv                 serialize Record/List<Record> to space-separated values",
        "  yaml | yml          serialize to YAML text",
        "  toml                serialize to TOML text",
        "  text                flatten values to plain text lines",
        "",
        "CSV/SSV flags:",
        "  --transpose         serialize records as field-oriented data",
        "",
        "Examples:",
        "  [1,2,3] | to json",
        "  [{a:1},{a:2}] | to jsonl",
        "  [{name:'alice',age:30}] | to csv",
        "  [{name:'alice',age:30}] | to ssv",
    ]
    .join("\n");
    meta_text_output("to", help)
}

fn meta_text_output(source: &str, text: String) -> CtPipelineData {
    CtPipelineData::Value(
        CtValue::String(text.clone()),
        CtPipelineMetadata {
            classic_text: Some(text),
            classic_bytes: None,
            classic_append_newline: false,
            exit_code: 0,
            source: Some(source.into()),
            ..Default::default()
        },
    )
}

fn to_jsonl(data: CtPipelineData) -> Result<String, CtDiagnosticError> {
    match data {
        CtPipelineData::Empty => Ok(String::new()),
        CtPipelineData::Value(CtValue::List(items), _) => {
            let mut lines = Vec::with_capacity(items.len());
            for item in items {
                let line = serde_json::to_string(&ct_to_json(&item))
                    .map_err(|e| CtDiagnosticError::simple(format!("to jsonl: {e}")))?;
                lines.push(line);
            }
            Ok(lines.join("\n"))
        }
        CtPipelineData::ListStream(stream) => {
            let mut lines = Vec::new();
            for item in stream {
                let line = serde_json::to_string(&ct_to_json(&item))
                    .map_err(|e| CtDiagnosticError::simple(format!("to jsonl: {e}")))?;
                lines.push(line);
            }
            Ok(lines.join("\n"))
        }
        other => {
            let v = consume_to_value(other)?;
            let line = serde_json::to_string(&ct_to_json(&v))
                .map_err(|e| CtDiagnosticError::simple(format!("to jsonl: {e}")))?;
            Ok(line)
        }
    }
}

fn to_csv(data: CtPipelineData) -> Result<String, CtDiagnosticError> {
    let rows = normalize_rows_for_csv(data)?;
    validate_csv_record_field_names(&rows)?;
    let columns = collect_csv_columns(&rows);
    if columns.is_empty() {
        return Ok(String::new());
    }

    let mut wtr = csv::WriterBuilder::new()
        .has_headers(true)
        .from_writer(Vec::<u8>::new());

    wtr.write_record(columns.iter())
        .map_err(|e| CtDiagnosticError::simple(format!("to csv: header write failed: {e}")))?;

    for row in rows {
        let record = columns
            .iter()
            .map(|col| {
                row.iter()
                    .find(|(k, _)| k == col)
                    .map(|(_, v)| ct_to_text_cell(v))
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();
        wtr.write_record(record)
            .map_err(|e| CtDiagnosticError::simple(format!("to csv: row write failed: {e}")))?;
    }

    let bytes = wtr
        .into_inner()
        .map_err(|e| CtDiagnosticError::simple(format!("to csv: flush failed: {e}")))?;
    String::from_utf8(bytes)
        .map_err(|e| CtDiagnosticError::simple(format!("to csv: utf8 error: {e}")))
}

fn to_csv_transposed(data: CtPipelineData) -> Result<String, CtDiagnosticError> {
    let rows = normalize_rows_for_csv(data)?;
    validate_csv_record_field_names(&rows)?;
    let columns = collect_csv_columns(&rows);
    if columns.is_empty() {
        return Ok(String::new());
    }

    let mut wtr = csv::WriterBuilder::new()
        .has_headers(false)
        .from_writer(Vec::<u8>::new());

    for column in columns {
        let mut record = Vec::with_capacity(rows.len() + 1);
        record.push(column.clone());
        for row in &rows {
            let value = row
                .iter()
                .find(|(key, _)| key == &column)
                .map(|(_, v)| ct_to_text_cell(v))
                .unwrap_or_default();
            record.push(value);
        }

        wtr.write_record(record)
            .map_err(|e| CtDiagnosticError::simple(format!("to csv: row write failed: {e}")))?;
    }

    let bytes = wtr
        .into_inner()
        .map_err(|e| CtDiagnosticError::simple(format!("to csv: flush failed: {e}")))?;
    String::from_utf8(bytes)
        .map_err(|e| CtDiagnosticError::simple(format!("to csv: utf8 error: {e}")))
}

fn to_ssv(data: CtPipelineData) -> Result<String, CtDiagnosticError> {
    let rows = normalize_rows_for_csv(data)?;
    validate_csv_record_field_names(&rows)?;
    let columns = collect_csv_columns(&rows);
    if columns.is_empty() {
        return Ok(String::new());
    }

    let mut lines = Vec::with_capacity(rows.len() + 1);
    lines.push(columns.join(" "));
    for row in rows {
        let record = columns
            .iter()
            .map(|col| {
                row.iter()
                    .find(|(key, _)| key == col)
                    .map(|(_, v)| ct_to_ssv_cell(v))
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();
        lines.push(record.join(" "));
    }

    Ok(lines.join("\n"))
}

fn to_ssv_transposed(data: CtPipelineData) -> Result<String, CtDiagnosticError> {
    let rows = normalize_rows_for_csv(data)?;
    validate_csv_record_field_names(&rows)?;
    let columns = collect_csv_columns(&rows);
    if columns.is_empty() {
        return Ok(String::new());
    }

    let mut lines = Vec::with_capacity(columns.len());
    for column in columns {
        let mut record = Vec::with_capacity(rows.len() + 1);
        record.push(column.clone());
        for row in &rows {
            let value = row
                .iter()
                .find(|(key, _)| key == &column)
                .map(|(_, v)| ct_to_ssv_cell(v))
                .unwrap_or_default();
            record.push(value);
        }
        lines.push(record.join(" "));
    }

    Ok(lines.join("\n"))
}

fn validate_csv_record_field_names(
    rows: &[Vec<(String, CtValue)>],
) -> Result<(), CtDiagnosticError> {
    for (row_idx, row) in rows.iter().enumerate() {
        let mut fields_seen = std::collections::HashSet::new();
        for (field, _) in row {
            let field = field.trim();
            if field.is_empty() {
                return Err(CtDiagnosticError::simple(format!(
                    "to csv: field name is empty at input row {}",
                    row_idx + 1
                )));
            }
            if !fields_seen.insert(field.to_string()) {
                return Err(CtDiagnosticError::simple(format!(
                    "to csv: duplicate field `{field}`"
                )));
            }
        }
    }
    Ok(())
}

fn ct_to_ssv_cell(v: &CtValue) -> String {
    ct_to_text_cell(v)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("_")
}

fn normalize_rows_for_csv(
    data: CtPipelineData,
) -> Result<Vec<Vec<(String, CtValue)>>, CtDiagnosticError> {
    match data {
        CtPipelineData::Empty => Ok(Vec::new()),
        CtPipelineData::Value(CtValue::Record(fields), _) => Ok(vec![fields]),
        CtPipelineData::Value(CtValue::List(items), _) => {
            let mut out = Vec::new();
            for item in items {
                let CtValue::Record(fields) = item else {
                    return Err(CtDiagnosticError::simple(
                        "to csv: expected Record or List<Record> input",
                    ));
                };
                out.push(fields);
            }
            Ok(out)
        }
        CtPipelineData::ListStream(stream) => {
            let mut out = Vec::new();
            for item in stream {
                let CtValue::Record(fields) = item else {
                    return Err(CtDiagnosticError::simple(
                        "to csv: expected Record or List<Record> input",
                    ));
                };
                out.push(fields);
            }
            Ok(out)
        }
        CtPipelineData::ByteStream(_) => Err(CtDiagnosticError::simple(
            "to csv: cannot serialize ByteStream to csv",
        )),
        CtPipelineData::Value(_, _) => Err(CtDiagnosticError::simple(
            "to csv: expected Record or List<Record> input",
        )),
    }
}

fn collect_csv_columns(rows: &[Vec<(String, CtValue)>]) -> Vec<String> {
    let mut cols = Vec::new();
    for row in rows {
        for (key, _) in row {
            if !cols.iter().any(|c| c == key) {
                cols.push(key.clone());
            }
        }
    }
    cols
}

fn ct_to_text_cell(v: &CtValue) -> String {
    match v {
        CtValue::Nothing => String::new(),
        CtValue::Bool(b) => b.to_string(),
        CtValue::Int(n) => n.to_string(),
        CtValue::Float(n) => n.to_string(),
        CtValue::String(s) => s.clone(),
        CtValue::Binary(_) => "<binary>".to_string(),
        CtValue::DateTime(_) | CtValue::Duration(_) | CtValue::Size(_) => v.to_text(),
        CtValue::Record(_) | CtValue::List(_) => {
            serde_json::to_string(&ct_to_json(v)).unwrap_or_else(|_| format!("{v:?}"))
        }
        CtValue::Error(e) => format!("<error: {e}>"),
    }
}

fn ct_to_json(v: &CtValue) -> serde_json::Value {
    match v {
        CtValue::Nothing => serde_json::Value::Null,
        CtValue::Bool(b) => serde_json::Value::Bool(*b),
        CtValue::Int(n) => serde_json::Value::Number((*n).into()),
        CtValue::Float(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        CtValue::String(s) => serde_json::Value::String(s.clone()),
        CtValue::Binary(_) => serde_json::Value::String("<binary>".into()),
        CtValue::DateTime(_) | CtValue::Duration(_) | CtValue::Size(_) => {
            serde_json::Value::String(v.to_text())
        }
        CtValue::Record(fields) => serde_json::Value::Object(
            fields
                .iter()
                .map(|(k, v)| (k.clone(), ct_to_json(v)))
                .collect(),
        ),
        CtValue::List(items) => serde_json::Value::Array(items.iter().map(ct_to_json).collect()),
        CtValue::Error(e) => serde_json::Value::String(format!("<error: {e}>")),
    }
}

fn ct_to_toml(v: &CtValue) -> toml::Value {
    match v {
        CtValue::Nothing => toml::Value::String(String::new()),
        CtValue::Bool(b) => toml::Value::Boolean(*b),
        CtValue::Int(n) => toml::Value::Integer(*n),
        CtValue::Float(f) => toml::Value::Float(*f),
        CtValue::String(s) => toml::Value::String(s.clone()),
        CtValue::Binary(_) => toml::Value::String("<binary>".to_string()),
        CtValue::DateTime(_) | CtValue::Duration(_) | CtValue::Size(_) => {
            toml::Value::String(v.to_text())
        }
        CtValue::Record(fields) => toml::Value::Table(
            fields
                .iter()
                .map(|(k, v)| (k.clone(), ct_to_toml(v)))
                .collect(),
        ),
        CtValue::List(items) => toml::Value::Array(items.iter().map(ct_to_toml).collect()),
        CtValue::Error(e) => toml::Value::String(format!("<error: {e}>")),
    }
}

fn consume_to_value(data: CtPipelineData) -> Result<CtValue, CtDiagnosticError> {
    match data {
        CtPipelineData::Empty => Ok(CtValue::Nothing),
        CtPipelineData::Value(v, _) => Ok(v),
        CtPipelineData::ListStream(s) => Ok(CtValue::List(s.collect())),
        CtPipelineData::ByteStream(_) => Err(CtDiagnosticError::simple(
            "to: cannot serialize ByteStream to structured formats",
        )),
    }
}

fn is_empty_record_value(value: &CtValue) -> bool {
    matches!(value, CtValue::Record(fields) if fields.is_empty())
}

fn is_empty_record_or_list_value(value: &CtValue) -> bool {
    matches!(
        value,
        CtValue::Record(fields) if fields.is_empty()
    ) || matches!(value, CtValue::List(items) if items.is_empty())
}

fn consume_to_lines(data: CtPipelineData) -> Result<Vec<String>, CtDiagnosticError> {
    match data {
        CtPipelineData::Empty => Ok(vec![]),
        CtPipelineData::Value(CtValue::List(items), _) => Ok(items
            .into_iter()
            .map(|v| {
                if let CtValue::String(s) = v {
                    s
                } else {
                    format!("{v:?}")
                }
            })
            .collect()),
        CtPipelineData::Value(CtValue::String(s), _) => {
            Ok(s.lines().map(|l| l.to_string()).collect())
        }
        CtPipelineData::Value(CtValue::Record(fields), _) if fields.is_empty() => Ok(vec![]),
        CtPipelineData::Value(v, _) => Ok(vec![format!("{v:?}")]),
        CtPipelineData::ListStream(s) => Ok(s
            .map(|v| {
                if let CtValue::String(s) = v {
                    s
                } else {
                    format!("{v:?}")
                }
            })
            .collect()),
        CtPipelineData::ByteStream(_) => Err(CtDiagnosticError::simple(
            "to: cannot serialize ByteStream to text",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctengine::context::{CommandRegistry, DataEngineContext};
    use ctpipeline::CtListStream;

    fn ctx() -> DataEngineContext {
        DataEngineContext::new(CommandRegistry::empty(), None, None)
    }

    fn fmt_call(fmt: &str) -> DataCall {
        let mut c = DataCall::empty();
        c.positionals
            .push(ctsig::BoundArg::new(CtValue::String(fmt.to_string()), None));
        c
    }

    fn fmt_call_transpose(fmt: &str) -> DataCall {
        let mut c = fmt_call(fmt);
        c.flags.insert("transpose".to_string(), None);
        c
    }

    fn empty_record_input() -> CtPipelineData {
        CtPipelineData::Value(CtValue::Record(vec![]), CtPipelineMetadata::default())
    }

    #[test]
    fn test_to_json_record() {
        let input = CtPipelineData::Value(
            CtValue::Record(vec![("name".into(), CtValue::String("Alice".into()))]),
            CtPipelineMetadata::default(),
        );
        let r = CmdTo.run(&fmt_call("json"), input, &ctx()).unwrap();
        if let CtPipelineData::Value(CtValue::String(json), _) = r {
            assert!(json.contains("\"Alice\""));
        } else {
            panic!("expected json string");
        }
    }

    #[test]
    fn test_to_jsonl_list() {
        let input = CtPipelineData::Value(
            CtValue::List(vec![
                CtValue::Record(vec![("a".into(), CtValue::Int(1))]),
                CtValue::Record(vec![("a".into(), CtValue::Int(2))]),
            ]),
            CtPipelineMetadata::default(),
        );
        let r = CmdTo.run(&fmt_call("jsonl"), input, &ctx()).unwrap();
        let CtPipelineData::Value(CtValue::String(s), _) = r else {
            panic!("string");
        };
        assert!(s.contains("{\"a\":1}"));
        assert!(s.contains("{\"a\":2}"));
    }

    #[test]
    fn test_to_jsonl_list_stream() {
        let stream = CtListStream::new(
            vec![
                CtValue::Record(vec![("a".into(), CtValue::Int(1))]),
                CtValue::Record(vec![("a".into(), CtValue::Int(2))]),
            ]
            .into_iter(),
            CtPipelineMetadata::default(),
        );
        let input = CtPipelineData::ListStream(stream);
        let r = CmdTo.run(&fmt_call("jsonl"), input, &ctx()).unwrap();
        let CtPipelineData::Value(CtValue::String(s), _) = r else {
            panic!("string");
        };
        assert_eq!(s, "{\"a\":1}\n{\"a\":2}");
    }

    #[test]
    fn test_to_csv_list_records() {
        let input = CtPipelineData::Value(
            CtValue::List(vec![
                CtValue::Record(vec![
                    ("name".into(), CtValue::String("alice".into())),
                    ("age".into(), CtValue::Int(30)),
                ]),
                CtValue::Record(vec![
                    ("name".into(), CtValue::String("bob".into())),
                    ("age".into(), CtValue::Int(20)),
                ]),
            ]),
            CtPipelineMetadata::default(),
        );
        let r = CmdTo.run(&fmt_call("csv"), input, &ctx()).unwrap();
        let CtPipelineData::Value(CtValue::String(csv), _) = r else {
            panic!("string");
        };
        assert!(csv.contains("name,age"));
        assert!(csv.contains("alice,30"));
    }

    #[test]
    fn test_to_csv_empty_record_outputs_empty_text() {
        let r = CmdTo
            .run(&fmt_call("csv"), empty_record_input(), &ctx())
            .unwrap();
        let CtPipelineData::Value(CtValue::String(csv), _) = r else {
            panic!("string");
        };
        assert_eq!(csv, "");
    }

    #[test]
    fn test_to_csv_rejects_empty_field_name() {
        let input = CtPipelineData::Value(
            CtValue::Record(vec![("".into(), CtValue::String("a".into()))]),
            CtPipelineMetadata::default(),
        );
        let err = CmdTo.run(&fmt_call("csv"), input, &ctx()).unwrap_err();
        assert!(err.to_string().contains("field name is empty"));
    }

    #[test]
    fn test_to_csv_rejects_duplicate_field_name() {
        let input = CtPipelineData::Value(
            CtValue::Record(vec![
                ("name".into(), CtValue::String("a".into())),
                ("name".into(), CtValue::String("b".into())),
            ]),
            CtPipelineMetadata::default(),
        );
        let err = CmdTo.run(&fmt_call("csv"), input, &ctx()).unwrap_err();
        assert!(err.to_string().contains("duplicate field"));
    }

    #[test]
    fn test_to_csv_transpose_list_records() {
        let input = CtPipelineData::Value(
            CtValue::List(vec![
                CtValue::Record(vec![
                    ("name".into(), CtValue::String("a".into())),
                    ("age".into(), CtValue::Int(12)),
                    ("gender".into(), CtValue::String("male".into())),
                ]),
                CtValue::Record(vec![
                    ("name".into(), CtValue::String("b".into())),
                    ("age".into(), CtValue::Int(11)),
                    ("gender".into(), CtValue::String("female".into())),
                ]),
            ]),
            CtPipelineMetadata::default(),
        );
        let r = CmdTo
            .run(&fmt_call_transpose("csv"), input, &ctx())
            .unwrap();
        let CtPipelineData::Value(CtValue::String(csv), _) = r else {
            panic!("string");
        };
        assert_eq!(csv, "name,a,b\nage,12,11\ngender,male,female\n");
    }

    #[test]
    fn test_to_csv_transpose_missing_values_are_empty() {
        let input = CtPipelineData::Value(
            CtValue::List(vec![
                CtValue::Record(vec![("name".into(), CtValue::String("a".into()))]),
                CtValue::Record(vec![
                    ("name".into(), CtValue::String("b".into())),
                    ("age".into(), CtValue::Int(11)),
                ]),
            ]),
            CtPipelineMetadata::default(),
        );
        let r = CmdTo
            .run(&fmt_call_transpose("csv"), input, &ctx())
            .unwrap();
        let CtPipelineData::Value(CtValue::String(csv), _) = r else {
            panic!("string");
        };
        assert_eq!(csv, "name,a,b\nage,,11\n");
    }

    #[test]
    fn test_to_csv_transpose_empty_record_outputs_empty_text() {
        let r = CmdTo
            .run(&fmt_call_transpose("csv"), empty_record_input(), &ctx())
            .unwrap();
        let CtPipelineData::Value(CtValue::String(csv), _) = r else {
            panic!("string");
        };
        assert_eq!(csv, "");
    }

    #[test]
    fn test_to_csv_transpose_rejects_empty_field_name() {
        let input = CtPipelineData::Value(
            CtValue::Record(vec![("".into(), CtValue::String("a".into()))]),
            CtPipelineMetadata::default(),
        );
        let err = CmdTo
            .run(&fmt_call_transpose("csv"), input, &ctx())
            .unwrap_err();
        assert!(err.to_string().contains("field name is empty"));
    }

    #[test]
    fn test_to_csv_transpose_rejects_duplicate_field_name() {
        let input = CtPipelineData::Value(
            CtValue::Record(vec![
                ("name".into(), CtValue::String("a".into())),
                ("name".into(), CtValue::String("b".into())),
            ]),
            CtPipelineMetadata::default(),
        );
        let err = CmdTo
            .run(&fmt_call_transpose("csv"), input, &ctx())
            .unwrap_err();
        assert!(err.to_string().contains("duplicate field"));
    }

    #[test]
    fn test_to_ssv_list_records() {
        let input = CtPipelineData::Value(
            CtValue::List(vec![
                CtValue::Record(vec![
                    ("name".into(), CtValue::String("alice".into())),
                    ("age".into(), CtValue::Int(30)),
                ]),
                CtValue::Record(vec![
                    ("name".into(), CtValue::String("bob".into())),
                    ("age".into(), CtValue::Int(20)),
                ]),
            ]),
            CtPipelineMetadata::default(),
        );
        let r = CmdTo.run(&fmt_call("ssv"), input, &ctx()).unwrap();
        let CtPipelineData::Value(CtValue::String(ssv), _) = r else {
            panic!("string");
        };
        assert_eq!(ssv, "name age\nalice 30\nbob 20");
    }

    #[test]
    fn test_to_ssv_list_stream() {
        let stream = CtListStream::new(
            vec![
                CtValue::Record(vec![("name".into(), CtValue::String("alice".into()))]),
                CtValue::Record(vec![
                    ("name".into(), CtValue::String("bob".into())),
                    ("age".into(), CtValue::Int(20)),
                ]),
            ]
            .into_iter(),
            CtPipelineMetadata::default(),
        );
        let input = CtPipelineData::ListStream(stream);
        let r = CmdTo.run(&fmt_call("ssv"), input, &ctx()).unwrap();
        let CtPipelineData::Value(CtValue::String(ssv), _) = r else {
            panic!("string");
        };
        assert_eq!(ssv, "name age\nalice \nbob 20");
    }

    #[test]
    fn test_to_ssv_sanitizes_whitespace_cells() {
        let input = CtPipelineData::Value(
            CtValue::Record(vec![("name".into(), CtValue::String("Alice Smith".into()))]),
            CtPipelineMetadata::default(),
        );
        let r = CmdTo.run(&fmt_call("ssv"), input, &ctx()).unwrap();
        let CtPipelineData::Value(CtValue::String(ssv), _) = r else {
            panic!("string");
        };
        assert_eq!(ssv, "name\nAlice_Smith");
    }

    #[test]
    fn test_to_ssv_transpose_list_records() {
        let input = CtPipelineData::Value(
            CtValue::List(vec![
                CtValue::Record(vec![
                    ("name".into(), CtValue::String("a".into())),
                    ("age".into(), CtValue::Int(12)),
                    ("gender".into(), CtValue::String("male".into())),
                ]),
                CtValue::Record(vec![
                    ("name".into(), CtValue::String("b".into())),
                    ("age".into(), CtValue::Int(11)),
                    ("gender".into(), CtValue::String("female".into())),
                ]),
            ]),
            CtPipelineMetadata::default(),
        );
        let r = CmdTo
            .run(&fmt_call_transpose("ssv"), input, &ctx())
            .unwrap();
        let CtPipelineData::Value(CtValue::String(ssv), _) = r else {
            panic!("string");
        };
        assert_eq!(ssv, "name a b\nage 12 11\ngender male female");
    }

    #[test]
    fn test_to_ssv_transpose_list_stream() {
        let stream = CtListStream::new(
            vec![
                CtValue::Record(vec![("name".into(), CtValue::String("alice".into()))]),
                CtValue::Record(vec![
                    ("name".into(), CtValue::String("bob".into())),
                    ("age".into(), CtValue::Int(20)),
                ]),
            ]
            .into_iter(),
            CtPipelineMetadata::default(),
        );
        let input = CtPipelineData::ListStream(stream);
        let r = CmdTo
            .run(&fmt_call_transpose("ssv"), input, &ctx())
            .unwrap();
        let CtPipelineData::Value(CtValue::String(ssv), _) = r else {
            panic!("string");
        };
        assert_eq!(ssv, "name alice bob\nage  20");
    }

    #[test]
    fn test_to_ssv_transpose_sanitizes_whitespace_cells() {
        let input = CtPipelineData::Value(
            CtValue::Record(vec![("name".into(), CtValue::String("Alice Smith".into()))]),
            CtPipelineMetadata::default(),
        );
        let r = CmdTo
            .run(&fmt_call_transpose("ssv"), input, &ctx())
            .unwrap();
        let CtPipelineData::Value(CtValue::String(ssv), _) = r else {
            panic!("string");
        };
        assert_eq!(ssv, "name Alice_Smith");
    }

    #[test]
    fn test_to_ssv_rejects_empty_field_name() {
        let input = CtPipelineData::Value(
            CtValue::Record(vec![("".into(), CtValue::String("a".into()))]),
            CtPipelineMetadata::default(),
        );
        let err = CmdTo.run(&fmt_call("ssv"), input, &ctx()).unwrap_err();
        assert!(err.to_string().contains("field name is empty"));
    }

    #[test]
    fn test_to_ssv_rejects_duplicate_field_name() {
        let input = CtPipelineData::Value(
            CtValue::Record(vec![
                ("name".into(), CtValue::String("a".into())),
                ("name".into(), CtValue::String("b".into())),
            ]),
            CtPipelineMetadata::default(),
        );
        let err = CmdTo.run(&fmt_call("ssv"), input, &ctx()).unwrap_err();
        assert!(err.to_string().contains("duplicate field"));
    }

    #[test]
    fn test_to_yaml_record() {
        let input = CtPipelineData::Value(
            CtValue::Record(vec![("a".into(), CtValue::Int(1))]),
            CtPipelineMetadata::default(),
        );
        let r = CmdTo.run(&fmt_call("yaml"), input, &ctx()).unwrap();
        let CtPipelineData::Value(CtValue::String(y), _) = r else {
            panic!("string");
        };
        assert!(y.contains("a: 1"));
    }

    #[test]
    fn test_to_yaml_empty_record_outputs_empty_text() {
        let r = CmdTo
            .run(&fmt_call("yaml"), empty_record_input(), &ctx())
            .unwrap();
        let CtPipelineData::Value(CtValue::String(y), _) = r else {
            panic!("string");
        };
        assert_eq!(y, "");
    }

    #[test]
    fn test_to_toml_record() {
        let input = CtPipelineData::Value(
            CtValue::Record(vec![("a".into(), CtValue::Int(1))]),
            CtPipelineMetadata::default(),
        );
        let r = CmdTo.run(&fmt_call("toml"), input, &ctx()).unwrap();
        let CtPipelineData::Value(CtValue::String(t), _) = r else {
            panic!("string");
        };
        assert!(t.contains("a = 1"));
    }

    #[test]
    fn test_to_toml_empty_record_outputs_empty_text() {
        let r = CmdTo
            .run(&fmt_call("toml"), empty_record_input(), &ctx())
            .unwrap();
        let CtPipelineData::Value(CtValue::String(t), _) = r else {
            panic!("string");
        };
        assert_eq!(t, "");
    }

    #[test]
    fn test_to_toml_empty_list_outputs_empty_text() {
        let input = CtPipelineData::Value(CtValue::List(vec![]), CtPipelineMetadata::default());
        let r = CmdTo.run(&fmt_call("toml"), input, &ctx()).unwrap();
        let CtPipelineData::Value(CtValue::String(t), _) = r else {
            panic!("string");
        };
        assert_eq!(t, "");
    }

    #[test]
    fn test_to_text_list() {
        let input = CtPipelineData::Value(
            CtValue::List(vec![
                CtValue::String("a".into()),
                CtValue::String("b".into()),
            ]),
            CtPipelineMetadata::default(),
        );
        let r = CmdTo.run(&fmt_call("text"), input, &ctx()).unwrap();
        if let CtPipelineData::Value(CtValue::String(s), _) = r {
            assert!(s.contains("a"));
            assert!(s.contains("b"));
        } else {
            panic!();
        }
    }

    #[test]
    fn test_to_text_empty_record_outputs_empty_text() {
        let r = CmdTo
            .run(&fmt_call("text"), empty_record_input(), &ctx())
            .unwrap();
        let CtPipelineData::Value(CtValue::String(s), _) = r else {
            panic!("string");
        };
        assert_eq!(s, "");
    }

    #[test]
    fn test_to_invalid_format() {
        let e = CmdTo
            .run(&fmt_call("xml"), CtPipelineData::Empty, &ctx())
            .unwrap_err();
        assert!(e.to_string().contains("unsupported format"));
    }

    #[test]
    fn test_to_without_format_returns_help() {
        let out = CmdTo
            .run(&DataCall::empty(), CtPipelineData::Empty, &ctx())
            .unwrap();
        let CtPipelineData::Value(CtValue::String(help), _) = out else {
            panic!("expected help text");
        };
        assert!(help.contains("Usage:"));
        assert!(help.contains("to <format>"));
    }

    #[test]
    fn test_to_help_flag_returns_help() {
        let mut c = DataCall::named("to");
        c.flags.insert("help".to_string(), None);
        let out = CmdTo.run(&c, CtPipelineData::Empty, &ctx()).unwrap();
        let CtPipelineData::Value(CtValue::String(help), _) = out else {
            panic!("expected help text");
        };
        assert!(help.contains("Formats:"));
    }
}
