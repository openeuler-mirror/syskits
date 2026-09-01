/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 */

use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtValue};
use std::ffi::OsString;
use std::io::{self, Write};

pub fn write_pipeline_as_structured_text(
    input: CtPipelineData,
    mut writer: impl Write,
) -> io::Result<()> {
    match input {
        CtPipelineData::Empty => Ok(()),
        CtPipelineData::ByteStream(mut stream) => {
            std::io::copy(&mut stream, &mut writer)?;
            writer.flush()
        }
        CtPipelineData::Value(value, _) => {
            write_value_as_text(value, &mut writer)?;
            writer.flush()
        }
        CtPipelineData::ListStream(stream) => {
            write_values_as_lines(stream, &mut writer)?;
            writer.flush()
        }
    }
}

pub fn write_pipeline_as_classic_stdin(
    input: CtPipelineData,
    mut writer: impl Write,
) -> io::Result<()> {
    match input {
        CtPipelineData::Empty => Ok(()),
        CtPipelineData::ByteStream(mut stream) => {
            std::io::copy(&mut stream, &mut writer)?;
            writer.flush()
        }
        CtPipelineData::Value(value, metadata) => {
            write_value_with_metadata_as_text(value, metadata, &mut writer)?;
            writer.flush()
        }
        CtPipelineData::ListStream(stream) => {
            write_values_as_lines(stream, &mut writer)?;
            writer.flush()
        }
    }
}

pub fn run_with_optional_pipeline_stdin<T, E, F, M>(
    label: &str,
    input: CtPipelineData,
    use_stdin: bool,
    run: F,
    map_io_err: M,
) -> Result<T, E>
where
    F: FnOnce() -> Result<T, E>,
    M: Copy + Fn(io::Error) -> E,
{
    let _ = label;
    if !use_stdin || matches!(input, CtPipelineData::Empty) {
        return run();
    }

    let mut stdin_bytes = Vec::new();
    write_pipeline_as_classic_stdin(input, &mut stdin_bytes).map_err(map_io_err)?;
    ctcore::ct_io::with_injected_stdin(stdin_bytes, run)
}

pub fn run_with_optional_pipeline_stdin_io<T, F>(
    label: &str,
    input: CtPipelineData,
    use_stdin: bool,
    run: F,
) -> io::Result<T>
where
    F: FnOnce() -> io::Result<T>,
{
    run_with_optional_pipeline_stdin(label, input, use_stdin, run, |e| e)
}

pub fn argv_uses_stdin(argv: &[OsString], value_flags: &[&str]) -> bool {
    let mut saw_file = false;
    let mut end_options = false;
    let mut index = 1;

    while index < argv.len() {
        let arg = argv[index].to_string_lossy();
        if arg == "-" {
            return true;
        }
        if end_options {
            saw_file = true;
            break;
        }
        if arg == "--" {
            end_options = true;
            index += 1;
            continue;
        }
        if arg.starts_with("--") {
            let flag = arg.split('=').next().unwrap_or_default();
            if value_flags.contains(&flag) {
                if arg.ends_with("=-") {
                    return true;
                }
                if !arg.contains('=') {
                    if argv
                        .get(index + 1)
                        .is_some_and(|value| value.to_string_lossy() == "-")
                    {
                        return true;
                    }
                    index += 2;
                } else {
                    index += 1;
                }
            } else {
                index += 1;
            }
            continue;
        }
        if arg.starts_with('-') && arg.len() > 1 {
            if let Some(consumes_next) = short_option_value_consumption(&arg, value_flags) {
                if argv
                    .get(index + 1)
                    .is_some_and(|value| value.to_string_lossy() == "-")
                {
                    return true;
                }
                index += if consumes_next { 2 } else { 1 };
            } else {
                index += 1;
            }
            continue;
        }
        saw_file = true;
        break;
    }

    !saw_file
}

pub fn argv_has_stdin_operand(argv: &[OsString], value_flags: &[&str]) -> bool {
    let mut end_options = false;
    let mut index = 1;

    while index < argv.len() {
        let arg = argv[index].to_string_lossy();
        if arg == "-" {
            return true;
        }
        if end_options {
            index += 1;
            continue;
        }
        if arg == "--" {
            end_options = true;
            index += 1;
            continue;
        }
        if arg.starts_with("--") {
            let flag = arg.split('=').next().unwrap_or_default();
            if value_flags.contains(&flag) && !arg.contains('=') {
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        index += 1;
    }

    false
}

fn short_option_value_consumption(arg: &str, value_flags: &[&str]) -> Option<bool> {
    if !arg.starts_with('-') || arg.starts_with("--") || arg == "-" {
        return None;
    }

    let mut chars = arg[1..].char_indices().peekable();
    while let Some((offset, ch)) = chars.next() {
        let flag = format!("-{ch}");
        if value_flags.contains(&flag.as_str()) {
            let has_inline_value = chars.peek().is_some();
            let is_exact_flag = offset == 0 && !has_inline_value;
            return Some(is_exact_flag || !has_inline_value);
        }
    }
    None
}

fn write_values_as_lines(
    values: impl IntoIterator<Item = CtValue>,
    writer: &mut impl Write,
) -> io::Result<()> {
    for value in values {
        write_line_value(value, writer)?;
        writer.write_all(b"\n")?;
    }
    Ok(())
}

fn write_value_as_text(value: CtValue, writer: &mut impl Write) -> io::Result<()> {
    match value {
        CtValue::List(items) => write_values_as_lines(items, writer),
        other => write_scalar_or_structured_value(&other, writer),
    }
}

fn write_value_with_metadata_as_text(
    value: CtValue,
    metadata: CtPipelineMetadata,
    writer: &mut impl Write,
) -> io::Result<()> {
    if let Some(bytes) = metadata.classic_bytes {
        return writer.write_all(&bytes);
    }
    if let Some(text) = metadata.classic_text {
        writer.write_all(text.as_bytes())?;
        if metadata.classic_append_newline && !text.ends_with('\n') {
            writer.write_all(b"\n")?;
        }
        return Ok(());
    }
    write_value_as_text(value, writer)
}

fn write_line_value(value: CtValue, writer: &mut impl Write) -> io::Result<()> {
    match value {
        CtValue::List(_) | CtValue::Record(_) => write_json_value(&value, writer),
        other => write_scalar_or_structured_value(&other, writer),
    }
}

fn write_scalar_or_structured_value(value: &CtValue, writer: &mut impl Write) -> io::Result<()> {
    match value {
        CtValue::Nothing => Ok(()),
        CtValue::Bool(_)
        | CtValue::Int(_)
        | CtValue::Float(_)
        | CtValue::String(_)
        | CtValue::DateTime(_)
        | CtValue::Duration(_)
        | CtValue::Size(_) => writer.write_all(value.to_text().as_bytes()),
        CtValue::Binary(bytes) => writer.write_all(bytes),
        CtValue::Record(_) | CtValue::List(_) => write_json_value(value, writer),
        CtValue::Error(e) => writer.write_all(format!("<error: {e}>").as_bytes()),
    }
}

fn write_json_value(value: &CtValue, writer: &mut impl Write) -> io::Result<()> {
    serde_json::to_writer(writer, &ct_value_to_json(value)).map_err(io::Error::other)
}

fn ct_value_to_json(value: &CtValue) -> serde_json::Value {
    match value {
        CtValue::Nothing => serde_json::Value::Null,
        CtValue::Bool(value) => serde_json::Value::Bool(*value),
        CtValue::Int(value) => serde_json::Value::Number((*value).into()),
        CtValue::Float(value) => serde_json::Number::from_f64(*value)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        CtValue::String(value) => serde_json::Value::String(value.clone()),
        CtValue::Binary(_) => serde_json::Value::String("<binary>".to_string()),
        CtValue::DateTime(_) | CtValue::Duration(_) | CtValue::Size(_) => {
            serde_json::Value::String(value.to_text())
        }
        CtValue::Record(fields) => serde_json::Value::Object(
            fields
                .iter()
                .map(|(key, value)| (key.clone(), ct_value_to_json(value)))
                .collect(),
        ),
        CtValue::List(items) => {
            serde_json::Value::Array(items.iter().map(ct_value_to_json).collect())
        }
        CtValue::Error(err) => serde_json::Value::String(format!("<error: {err}>")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctpipeline::{CtListStream, CtPipelineMetadata};

    #[test]
    fn write_pipeline_as_structured_text_flattens_list_values_to_lines() {
        let input = CtPipelineData::Value(
            CtValue::List(vec![
                CtValue::String("hello".into()),
                CtValue::String("world".into()),
            ]),
            CtPipelineMetadata::default(),
        );

        let mut buf = Vec::new();
        write_pipeline_as_structured_text(input, &mut buf).expect("should serialize");
        assert_eq!(buf, b"hello\nworld\n");
    }

    #[test]
    fn write_pipeline_as_structured_text_serializes_records_as_json() {
        let input = CtPipelineData::Value(
            CtValue::Record(vec![("name".into(), CtValue::String("alice".into()))]),
            CtPipelineMetadata::default(),
        );

        let mut buf = Vec::new();
        write_pipeline_as_structured_text(input, &mut buf).expect("should serialize");
        assert_eq!(String::from_utf8(buf).unwrap(), "{\"name\":\"alice\"}");
    }

    #[test]
    fn write_pipeline_as_structured_text_ignores_classic_text() {
        let input = CtPipelineData::Value(
            CtValue::Record(vec![("text".into(), CtValue::String("hello".into()))]),
            CtPipelineMetadata {
                classic_text: Some("hello".into()),
                classic_append_newline: false,
                ..Default::default()
            },
        );

        let mut buf = Vec::new();
        write_pipeline_as_structured_text(input, &mut buf).expect("should serialize");
        assert_eq!(String::from_utf8(buf).unwrap(), "{\"text\":\"hello\"}");
    }

    #[test]
    fn write_pipeline_as_classic_stdin_prefers_classic_text_for_value_stdin() {
        let input = CtPipelineData::Value(
            CtValue::Record(vec![("text".into(), CtValue::String("hello".into()))]),
            CtPipelineMetadata {
                classic_text: Some("hello".into()),
                classic_append_newline: false,
                ..Default::default()
            },
        );

        let mut buf = Vec::new();
        write_pipeline_as_classic_stdin(input, &mut buf).expect("should serialize");
        assert_eq!(buf, b"hello");
    }

    #[test]
    fn write_pipeline_as_classic_stdin_honors_classic_append_newline() {
        let input = CtPipelineData::Value(
            CtValue::String("hello".into()),
            CtPipelineMetadata {
                classic_text: Some("hello".into()),
                classic_append_newline: true,
                ..Default::default()
            },
        );

        let mut buf = Vec::new();
        write_pipeline_as_classic_stdin(input, &mut buf).expect("should serialize");
        assert_eq!(buf, b"hello\n");
    }

    #[test]
    fn write_pipeline_as_classic_stdin_prefers_classic_bytes_for_value_stdin() {
        let input = CtPipelineData::Value(
            CtValue::String("ignored".into()),
            CtPipelineMetadata {
                classic_text: Some("ignored".into()),
                classic_bytes: Some(vec![0, 159, 146, 150]),
                ..Default::default()
            },
        );

        let mut buf = Vec::new();
        write_pipeline_as_classic_stdin(input, &mut buf).expect("should serialize");
        assert_eq!(buf, vec![0, 159, 146, 150]);
    }

    #[test]
    fn write_pipeline_as_structured_text_handles_list_stream() {
        let input = CtPipelineData::ListStream(CtListStream::new(
            vec![CtValue::Int(1), CtValue::Int(2)].into_iter(),
            CtPipelineMetadata::default(),
        ));

        let mut buf = Vec::new();
        write_pipeline_as_structured_text(input, &mut buf).expect("should serialize");
        assert_eq!(buf, b"1\n2\n");
    }

    #[test]
    fn argv_uses_stdin_defaults_to_stdin_when_no_file_operand() {
        let argv = vec!["fold".into(), "-w".into(), "3".into()];

        assert!(argv_uses_stdin(&argv, &["-w", "--width"]));
    }

    #[test]
    fn argv_uses_stdin_detects_explicit_dash_operand() {
        let argv = vec!["cat".into(), "-n".into(), "-".into()];

        assert!(argv_uses_stdin(&argv, &[]));
    }

    #[test]
    fn argv_uses_stdin_skips_value_flags_before_file_operand() {
        let argv = vec![
            "cksum".into(),
            "--algorithm".into(),
            "crc".into(),
            "file.txt".into(),
        ];

        assert!(!argv_uses_stdin(&argv, &["--algorithm"]));
    }

    #[test]
    fn argv_uses_stdin_understands_clustered_short_value_flags() {
        let argv = vec!["fold".into(), "-aw".into(), "3".into()];

        assert!(argv_uses_stdin(&argv, &["-w", "--width"]));
    }

    #[test]
    fn argv_has_stdin_operand_ignores_value_flag_dash() {
        let argv = vec![
            "comm".into(),
            "--output-delimiter".into(),
            "-".into(),
            "left.txt".into(),
            "right.txt".into(),
        ];

        assert!(!argv_has_stdin_operand(&argv, &["--output-delimiter"]));
    }

    #[test]
    fn argv_has_stdin_operand_detects_file_dash() {
        let argv = vec!["comm".into(), "-1".into(), "-".into(), "right.txt".into()];

        assert!(argv_has_stdin_operand(&argv, &["--output-delimiter"]));
    }
}
