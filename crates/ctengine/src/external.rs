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

use std::collections::BTreeMap;
use std::path::PathBuf;

/// 定义如何将数据管线中的输入编码提供给外部命令的 stdin。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExternalStdinMode {
    /// 直接将字节流透传给 stdin
    #[default]
    Raw,
    /// 将管线中的文本逐行写入，并添加后缀 \n
    TextLines,
    /// 将单个大 Value 格式化为 JSON 全集写入 stdin
    Json,
    /// 将管线流中的记录逐条格式化为 JSON 单行写入 stdin
    JsonLines,
}

/// 定义如何解码外部命令 stdout 并吸入数据管线。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExternalStdoutMode {
    /// 输出保持为裸字节流 (ByteStream)
    ///
    /// 注意：Raw 模式下会延迟等待子进程退出，非零退出码会在消费者把
    /// ByteStream 读到 EOF 时才返回错误。
    #[default]
    Raw,
    /// 按行读取并作为字符串列表输出 (ListStream<String>)
    TextLines,
    /// 将全量输出解析为单一 JSON 对象 (CtValue)
    Json,
    /// 将输出逐行解析为 JSON 对象列表 (ListStream<CtValue>)
    JsonLines,
    /// 将全量输出作为 CSV 解析，映射为 Record 列表
    Csv,
    /// 自动嗅探格式（高置信优先 JSON/CSV/SSV，文本回退 TextLines，二进制保持 Raw）
    Auto,
}

/// 定义外部命令 stderr 的处理策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExternalStderrMode {
    /// 继承当前进程的 stderr 输出（默认行为）
    #[default]
    Inherit,
    /// 将 stderr 重定向并入 stdout
    MergeToStdout,
    /// 捕获 stderr 用于失败诊断（设定容量上限如 64KB）
    Capture,
}

/// 定义进程异常退出时的错误抛出策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExternalExitPolicy {
    /// 若退出码非 0 则终止管线报错
    #[default]
    FailOnNonZero,
    /// 允许非 0 退出，不抛出异常，管线继续执行
    AllowNonZero,
}

/// 描述一次外部命令的完整调用参数与行为策略。
#[derive(Debug, Clone)]
pub struct ExternalCallSpec {
    pub cmd: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env_overrides: BTreeMap<String, String>,
    pub stdin_mode: ExternalStdinMode,
    pub stdout_mode: ExternalStdoutMode,
    pub stderr_mode: ExternalStderrMode,
    pub exit_policy: ExternalExitPolicy,
    pub timeout_ms: Option<u64>,
}

impl ExternalCallSpec {
    /// 快速创建一个具有默认行为和策略的调用规格
    pub fn quick(cmd: &str, args: &[String]) -> Self {
        Self {
            cmd: cmd.to_string(),
            args: args.to_vec(),
            cwd: None,
            env_overrides: BTreeMap::new(),
            stdin_mode: ExternalStdinMode::Raw,
            stdout_mode: ExternalStdoutMode::Raw,
            stderr_mode: ExternalStderrMode::Inherit,
            exit_policy: ExternalExitPolicy::FailOnNonZero,
            timeout_ms: None,
        }
    }
}

use ctpipeline::pipeline_data::CtPipelineData;
use std::io::Write;

/// 将管线数据编码后写入给定的输出流（通常是外部命令的 stdin）。
pub struct ExternalInputEncoder;

impl ExternalInputEncoder {
    /// 根据指定的 stdin 模式，将 `input` 编码并写入 `writer`。
    pub fn encode(
        input: CtPipelineData,
        mode: ExternalStdinMode,
        mut writer: impl Write,
    ) -> std::io::Result<()> {
        match mode {
            ExternalStdinMode::Raw => {
                crate::pipeline_stdin::write_pipeline_as_text(input, &mut writer)?;
            }
            ExternalStdinMode::TextLines => match input {
                CtPipelineData::ByteStream(mut stream) => {
                    std::io::copy(&mut stream, &mut writer)?;
                }
                CtPipelineData::Value(ctpipeline::CtValue::List(items), meta) => {
                    crate::pipeline_stdin::write_pipeline_as_text(
                        CtPipelineData::Value(ctpipeline::CtValue::List(items), meta),
                        &mut writer,
                    )?;
                }
                CtPipelineData::Value(val, _) => {
                    writer.write_all(val.to_text().as_bytes())?;
                    writer.write_all(b"\n")?;
                }
                CtPipelineData::ListStream(stream) => {
                    crate::pipeline_stdin::write_pipeline_as_text(
                        CtPipelineData::ListStream(stream),
                        &mut writer,
                    )?;
                }
                CtPipelineData::Empty => {}
            },
            ExternalStdinMode::Json => {
                if let CtPipelineData::Value(val, _) = input.collect_values() {
                    serde_json::to_writer(&mut writer, &val)?;
                }
            }
            ExternalStdinMode::JsonLines => match input {
                CtPipelineData::ListStream(stream) => {
                    for item in stream {
                        serde_json::to_writer(&mut writer, &item)?;
                        writer.write_all(b"\n")?;
                    }
                }
                CtPipelineData::Value(val, _) => {
                    serde_json::to_writer(&mut writer, &val)?;
                    writer.write_all(b"\n")?;
                }
                _ => {}
            },
        }
        writer.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctpipeline::metadata::CtPipelineMetadata;
    use ctpipeline::pipeline_data::{CtByteStream, CtListStream};
    use ctpipeline::value::CtValue;
    use std::io::Cursor;

    #[test]
    fn test_encode_raw() {
        let meta = CtPipelineMetadata::default();
        let val = CtValue::String("hello world".into());
        let data = CtPipelineData::Value(val, meta);

        let mut buf = Vec::new();
        ExternalInputEncoder::encode(data, ExternalStdinMode::Raw, &mut buf).unwrap();
        assert_eq!(buf, b"hello world");
    }

    #[test]
    fn test_encode_text_lines() {
        let meta = CtPipelineMetadata::default();
        let items = vec![CtValue::Int(10), CtValue::String("hi".into())];
        let stream = CtListStream::new(items.into_iter(), meta);
        let data = CtPipelineData::ListStream(stream);

        let mut buf = Vec::new();
        ExternalInputEncoder::encode(data, ExternalStdinMode::TextLines, &mut buf).unwrap();
        assert_eq!(buf, b"10\nhi\n");
    }

    #[test]
    fn test_encode_json() {
        let meta = CtPipelineMetadata::default();
        let items = vec![CtValue::Int(1), CtValue::Bool(true)];
        let stream = CtListStream::new(items.into_iter(), meta);
        let data = CtPipelineData::ListStream(stream); // 将在 encode 中自动 collect_values

        let mut buf = Vec::new();
        ExternalInputEncoder::encode(data, ExternalStdinMode::Json, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert_eq!(s, "[1,true]");
    }

    #[test]
    fn test_encode_json_lines() {
        let meta = CtPipelineMetadata::default();
        let items = vec![CtValue::Int(1), CtValue::Bool(true)];
        let stream = CtListStream::new(items.into_iter(), meta);
        let data = CtPipelineData::ListStream(stream);

        let mut buf = Vec::new();
        ExternalInputEncoder::encode(data, ExternalStdinMode::JsonLines, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert_eq!(s, "1\ntrue\n");
    }

    #[test]
    fn test_encode_raw_bytestream() {
        let meta = CtPipelineMetadata::default();
        let cursor = Cursor::new(b"byte content".to_vec());
        let bs = CtByteStream::new(cursor, meta);
        let data = CtPipelineData::ByteStream(bs);

        let mut buf = Vec::new();
        ExternalInputEncoder::encode(data, ExternalStdinMode::Raw, &mut buf).unwrap();
        assert_eq!(buf, b"byte content");
    }

    #[test]
    fn test_encode_text_lines_bytestream_preserves_bytes() {
        let meta = CtPipelineMetadata::default();
        let cursor = Cursor::new(b"byte content".to_vec());
        let bs = CtByteStream::new(cursor, meta);
        let data = CtPipelineData::ByteStream(bs);

        let mut buf = Vec::new();
        ExternalInputEncoder::encode(data, ExternalStdinMode::TextLines, &mut buf).unwrap();
        assert_eq!(buf, b"byte content");
    }
}

use std::any::Any;
use std::io::{BufRead, BufReader, Read};

const AUTO_DECODE_MAX_BYTES: usize = 1024 * 1024;
/// 用于格式嗅探的初始探测窗口。足够识别 JSON/CSV/SSV 特征，同时避免等待 EOF。
const AUTO_PROBE_BYTES: usize = 8 * 1024;
const MAX_CAPTURED_STDERR_BYTES: usize = 64 * 1024;

type MetadataCustom = std::sync::Arc<std::sync::Mutex<BTreeMap<String, ctpipeline::CtValue>>>;

fn decode_error(message: impl Into<String>, timed_out: bool) -> crate::error::CtDiagnosticError {
    let err = crate::error::CtDiagnosticError::simple(message.into());
    if timed_out {
        err.with_code(crate::execution::exit_code::TIMEOUT)
    } else {
        err
    }
}

fn decode_io_error(
    message: impl Into<String>,
    err: std::io::Error,
) -> crate::error::CtDiagnosticError {
    decode_error(
        format!("{}: {err}", message.into()),
        err.kind() == std::io::ErrorKind::TimedOut,
    )
}

fn decode_json_error(
    message: impl Into<String>,
    err: serde_json::Error,
) -> crate::error::CtDiagnosticError {
    decode_error(
        format!("{}: {err}", message.into()),
        matches!(err.io_error_kind(), Some(std::io::ErrorKind::TimedOut)),
    )
}

fn decode_csv_error(
    message: impl Into<String>,
    err: csv::Error,
) -> crate::error::CtDiagnosticError {
    decode_error(
        format!("{}: {err}", message.into()),
        matches!(
            err.kind(),
            csv::ErrorKind::Io(io_err) if io_err.kind() == std::io::ErrorKind::TimedOut
        ),
    )
}

fn json_to_ctvalue(v: serde_json::Value) -> ctpipeline::value::CtValue {
    match v {
        serde_json::Value::Null => ctpipeline::value::CtValue::Nothing,
        serde_json::Value::Bool(b) => ctpipeline::value::CtValue::Bool(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                ctpipeline::value::CtValue::Int(i)
            } else {
                ctpipeline::value::CtValue::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_json::Value::String(s) => ctpipeline::value::CtValue::String(s),
        serde_json::Value::Array(arr) => {
            ctpipeline::value::CtValue::List(arr.into_iter().map(json_to_ctvalue).collect())
        }
        serde_json::Value::Object(obj) => ctpipeline::value::CtValue::Record(
            obj.into_iter()
                .map(|(k, v)| (k, json_to_ctvalue(v)))
                .collect(),
        ),
    }
}

/// 解码外部命令的输出并将其转换为管线数据流
pub struct ExternalOutputDecoder;

impl ExternalOutputDecoder {
    /// 根据指定的 stdout 模式，将 `reader` (通常是 stdout) 解码为 `CtPipelineData`。
    pub fn decode(
        reader: impl Read + Send + 'static + Any,
        mode: ExternalStdoutMode,
        mut meta: ctpipeline::metadata::CtPipelineMetadata,
    ) -> Result<CtPipelineData, crate::error::CtDiagnosticError> {
        match mode {
            ExternalStdoutMode::Raw => decode_raw_reader(reader, meta),
            ExternalStdoutMode::TextLines => decode_text_lines_from_reader(reader, meta, "text"),
            ExternalStdoutMode::Json => {
                meta.content_type = Some("application/json".to_string());
                let val = serde_json::from_reader::<_, serde_json::Value>(reader)
                    .map_err(|e| decode_json_error("stdout decode(json) failed", e))?;
                Ok(CtPipelineData::Value(json_to_ctvalue(val), meta))
            }
            ExternalStdoutMode::JsonLines => {
                meta.content_type = Some("application/x-ndjson".to_string());
                let buf_reader = BufReader::new(reader);
                let mut values = Vec::new();
                for (idx, line_res) in buf_reader.lines().enumerate() {
                    let line_no = idx + 1;
                    let line = line_res.map_err(|e| {
                        decode_io_error(
                            format!("stdout decode(jsonlines) read failed at line {line_no}"),
                            e,
                        )
                    })?;
                    if line.trim().is_empty() {
                        continue;
                    }
                    let parsed = serde_json::from_str::<serde_json::Value>(&line).map_err(|e| {
                        crate::error::CtDiagnosticError::simple(format!(
                            "stdout decode(jsonlines) parse failed at line {line_no}: {e}"
                        ))
                    })?;
                    values.push(json_to_ctvalue(parsed));
                }
                Ok(CtPipelineData::ListStream(
                    ctpipeline::pipeline_data::CtListStream::new(values.into_iter(), meta),
                ))
            }
            ExternalStdoutMode::Csv => decode_csv(reader, meta),
            ExternalStdoutMode::Auto => decode_auto(reader, meta),
        }
    }
}

fn decode_auto(
    mut reader: impl Read + Send + 'static + Any,
    meta: ctpipeline::metadata::CtPipelineMetadata,
) -> Result<CtPipelineData, crate::error::CtDiagnosticError> {
    // Phase 1: ONE read() call — returns immediately after the first OS-level syscall.
    // A fill-loop would block on the second call for commands such as
    // `sh -c 'echo ready; sleep 60'` that emit a small prefix then hold stdout open.
    let mut probe_buf = vec![0u8; AUTO_PROBE_BYTES];
    let probe_n = match reader.read(&mut probe_buf) {
        Ok(n) => n,
        Err(e) => {
            return Err(decode_io_error(
                "stdout decode(auto) failed to read bytes",
                e,
            ));
        }
    };

    if probe_n == 0 {
        // Immediate EOF — empty output.
        return decode_text_lines_from_str("", meta);
    }

    let probe_bytes = &probe_buf[..probe_n];

    if probe_bytes.contains(&0u8) {
        // Binary: chain probe + remainder and pass as raw byte stream.
        let chained = std::io::Cursor::new(probe_bytes.to_vec()).chain(reader);
        return decode_raw_reader(chained, meta);
    }

    let text_probe = match std::str::from_utf8(probe_bytes) {
        Ok(s) => s,
        Err(_) => {
            let chained = std::io::Cursor::new(probe_bytes.to_vec()).chain(reader);
            return decode_raw_reader(chained, meta);
        }
    };

    if probe_n < AUTO_PROBE_BYTES {
        // The single read returned all currently-available bytes without filling the
        // buffer.  We still chain `reader` so that any subsequent output (e.g. a
        // multi-burst producer) is not silently dropped.
        return decode_auto_partial_probe(probe_bytes, text_probe, reader, meta);
    }

    // Probe buffer is full. If this is a still-running external process, avoid
    // blocking reads here and defer full consumption to downstream stream readers.
    if should_defer_partial_probe_decode(&mut reader)? {
        let complete = std::io::Cursor::new(probe_bytes.to_vec()).chain(reader);
        return decode_raw_reader(complete, meta);
    }

    decode_auto_full_probe(probe_bytes, reader, meta)
}

fn decode_auto_partial_probe<R: Read + Send + 'static + Any>(
    probe_bytes: &[u8],
    text_probe: &str,
    mut remainder: R,
    meta: ctpipeline::metadata::CtPipelineMetadata,
) -> Result<CtPipelineData, crate::error::CtDiagnosticError> {
    if should_defer_partial_probe_decode(&mut remainder)? {
        let complete = std::io::Cursor::new(probe_bytes.to_vec()).chain(remainder);
        return decode_raw_reader(complete, meta);
    }

    if starts_like_json_document(text_probe) {
        let full = read_probe_with_remainder_capped(probe_bytes, &mut remainder)?;
        if full.len() > AUTO_DECODE_MAX_BYTES {
            let complete = std::io::Cursor::new(full).chain(remainder);
            return decode_raw_reader(complete, meta);
        }
        return decode_auto_json_candidate(full, meta);
    }

    if looks_like_csv(text_probe) {
        let full = read_probe_with_remainder_capped(probe_bytes, &mut remainder)?;
        if full.len() > AUTO_DECODE_MAX_BYTES {
            let complete = std::io::Cursor::new(full).chain(remainder);
            return decode_raw_reader(complete, meta);
        }
        let text_full = match std::str::from_utf8(&full) {
            Ok(s) => s,
            Err(_) => return decode_raw_bytes(full, meta),
        };
        return match decode_csv(std::io::Cursor::new(full.clone()), meta.clone()) {
            Ok(data) => Ok(data),
            Err(_) => decode_text_lines_from_str(text_full, meta),
        };
    }

    if looks_like_ssv(text_probe) {
        let full = read_probe_with_remainder_capped(probe_bytes, &mut remainder)?;
        if full.len() > AUTO_DECODE_MAX_BYTES {
            let complete = std::io::Cursor::new(full).chain(remainder);
            return decode_raw_reader(complete, meta);
        }
        let text_full = match std::str::from_utf8(&full) {
            Ok(s) => s,
            Err(_) => return decode_raw_bytes(full, meta),
        };
        return if looks_like_ssv(text_full) {
            decode_ssv_from_str(text_full, meta)
        } else {
            decode_text_lines_from_str(text_full, meta)
        };
    }

    // Plain text: slurp once so we can safely downgrade to raw if later bytes are
    // non-UTF8 (probe-only UTF8 check is insufficient for multi-burst producers).
    let full = read_probe_with_remainder_capped(probe_bytes, &mut remainder)?;
    if full.len() > AUTO_DECODE_MAX_BYTES {
        let complete = std::io::Cursor::new(full).chain(remainder);
        return decode_raw_reader(complete, meta);
    }
    let text_full = match std::str::from_utf8(&full) {
        Ok(s) => s,
        Err(_) => return decode_raw_bytes(full, meta),
    };
    if looks_like_text(text_full) {
        decode_text_lines_from_str(text_full, meta)
    } else {
        decode_raw_bytes(full, meta)
    }
}

fn should_defer_partial_probe_decode(
    reader: &mut (impl Read + Send + 'static + Any),
) -> Result<bool, crate::error::CtDiagnosticError> {
    let any_reader = reader as &mut dyn Any;
    let Some(external) = any_reader.downcast_mut::<ExternalStream>() else {
        return Ok(false);
    };
    let mut child = external.child.lock().map_err(|_| {
        crate::error::CtDiagnosticError::simple(
            "stdout decode(auto) failed to inspect child state: process lock poisoned",
        )
    })?;
    let running = child
        .try_wait()
        .map_err(|e| decode_io_error("stdout decode(auto) failed to inspect child state", e))?
        .is_none();
    Ok(running)
}

fn decode_auto_full_probe<R: Read + Send + 'static>(
    probe_bytes: &[u8],
    mut reader: R,
    meta: ctpipeline::metadata::CtPipelineMetadata,
) -> Result<CtPipelineData, crate::error::CtDiagnosticError> {
    // Probe buffer was completely filled → definitely more data.
    let rest =
        read_capped_auto_bytes(&mut reader, (AUTO_DECODE_MAX_BYTES - probe_bytes.len()) + 1)?;
    if probe_bytes.len() + rest.len() > AUTO_DECODE_MAX_BYTES {
        let chained = std::io::Cursor::new(probe_bytes.to_vec())
            .chain(std::io::Cursor::new(rest))
            .chain(reader);
        return decode_raw_reader(chained, meta);
    }

    let mut full_bytes = probe_bytes.to_vec();
    full_bytes.extend_from_slice(&rest);
    decode_auto_full_bytes(full_bytes, meta)
}

fn decode_auto_full_bytes(
    full_bytes: Vec<u8>,
    meta: ctpipeline::metadata::CtPipelineMetadata,
) -> Result<CtPipelineData, crate::error::CtDiagnosticError> {
    if full_bytes.contains(&0u8) {
        return decode_raw_bytes(full_bytes, meta);
    }
    let text = match std::str::from_utf8(&full_bytes) {
        Ok(s) => s,
        Err(_) => return decode_raw_bytes(full_bytes, meta),
    };

    if starts_like_json_document(text) {
        return decode_auto_json_candidate(full_bytes, meta);
    }
    if looks_like_csv(text) {
        return match decode_csv(std::io::Cursor::new(full_bytes.clone()), meta.clone()) {
            Ok(data) => Ok(data),
            Err(_) => decode_text_lines_from_str(text, meta),
        };
    }
    if looks_like_ssv(text) {
        return decode_ssv_from_str(text, meta);
    }
    if looks_like_text(text) {
        return decode_text_lines_from_str(text, meta);
    }
    decode_raw_bytes(full_bytes, meta)
}

fn decode_auto_json_candidate(
    full_bytes: Vec<u8>,
    mut meta: ctpipeline::metadata::CtPipelineMetadata,
) -> Result<CtPipelineData, crate::error::CtDiagnosticError> {
    if full_bytes.contains(&0u8) {
        return decode_raw_bytes(full_bytes, meta);
    }
    let text = match std::str::from_utf8(&full_bytes) {
        Ok(s) => s,
        Err(_) => return decode_raw_bytes(full_bytes, meta),
    };

    match serde_json::from_slice::<serde_json::Value>(&full_bytes) {
        Ok(val) => {
            meta.content_type = Some("application/json".to_string());
            Ok(CtPipelineData::Value(json_to_ctvalue(val), meta))
        }
        Err(_) => {
            if looks_like_text(text) {
                decode_text_lines_from_str(text, meta)
            } else {
                decode_raw_bytes(full_bytes, meta)
            }
        }
    }
}

fn starts_like_json_document(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with('{') || trimmed.starts_with('[')
}

fn read_probe_with_remainder_capped(
    probe_bytes: &[u8],
    remainder: &mut impl Read,
) -> Result<Vec<u8>, crate::error::CtDiagnosticError> {
    let mut chained = std::io::Cursor::new(probe_bytes.to_vec()).chain(remainder);
    read_capped_auto_bytes(&mut chained, AUTO_DECODE_MAX_BYTES + 1)
}

fn read_capped_auto_bytes(
    reader: &mut impl Read,
    limit: usize,
) -> Result<Vec<u8>, crate::error::CtDiagnosticError> {
    let mut out = Vec::new();
    reader
        .take(limit as u64)
        .read_to_end(&mut out)
        .map_err(|e| decode_io_error("stdout decode(auto) failed to read bytes", e))?;
    Ok(out)
}

fn read_capped_stderr(mut reader: impl Read) -> std::io::Result<Vec<u8>> {
    let mut retained = Vec::with_capacity(MAX_CAPTURED_STDERR_BYTES);
    let mut buf = [0u8; 8192];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let remaining = MAX_CAPTURED_STDERR_BYTES.saturating_sub(retained.len());
        if remaining > 0 {
            retained.extend_from_slice(&buf[..n.min(remaining)]);
        }
    }
    Ok(retained)
}

fn decode_csv(
    reader: impl Read,
    mut meta: ctpipeline::metadata::CtPipelineMetadata,
) -> Result<CtPipelineData, crate::error::CtDiagnosticError> {
    meta.content_type = Some("text/csv".to_string());
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_reader(reader);
    let headers = reader
        .headers()
        .map_err(|e| decode_csv_error("stdout decode(csv) failed to read headers", e))?
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();

    let mut rows = Vec::new();
    for (idx, rec_res) in reader.into_records().enumerate() {
        let rec = rec_res.map_err(|e| {
            decode_csv_error(format!("stdout decode(csv) failed at row {}", idx + 1), e)
        })?;
        let mut pairs = Vec::with_capacity(headers.len());
        for (i, field) in rec.iter().enumerate() {
            let key = headers
                .get(i)
                .cloned()
                .unwrap_or_else(|| format!("col_{i}"));
            pairs.push((key, ctpipeline::value::CtValue::String(field.to_string())));
        }
        rows.push(ctpipeline::value::CtValue::Record(pairs));
    }
    Ok(CtPipelineData::ListStream(
        ctpipeline::pipeline_data::CtListStream::new(rows.into_iter(), meta),
    ))
}

fn decode_raw_bytes(
    bytes: Vec<u8>,
    meta: ctpipeline::metadata::CtPipelineMetadata,
) -> Result<CtPipelineData, crate::error::CtDiagnosticError> {
    decode_raw_reader(std::io::Cursor::new(bytes), meta)
}

fn decode_raw_reader(
    reader: impl Read + Send + 'static,
    mut meta: ctpipeline::metadata::CtPipelineMetadata,
) -> Result<CtPipelineData, crate::error::CtDiagnosticError> {
    meta.content_type = Some("application/octet-stream".to_string());
    Ok(CtPipelineData::ByteStream(
        ctpipeline::pipeline_data::CtByteStream::new(reader, meta),
    ))
}

fn decode_text_lines_from_str(
    text: &str,
    mut meta: ctpipeline::metadata::CtPipelineMetadata,
) -> Result<CtPipelineData, crate::error::CtDiagnosticError> {
    meta.content_type = Some("text/plain".to_string());
    let lines = text
        .lines()
        .map(|line| ctpipeline::value::CtValue::String(line.to_string()))
        .collect::<Vec<_>>();
    Ok(CtPipelineData::ListStream(
        ctpipeline::pipeline_data::CtListStream::new(lines.into_iter(), meta),
    ))
}

/// テキスト行を Reader から読み込み、ListStream に変換する。
/// probe + chained-reader を渡すことで、プロセスが保持するストリームも
/// 終端まで正しく収集される。追加の blocking read は行わない。
fn decode_text_lines_from_reader(
    reader: impl Read + Send + 'static,
    mut meta: ctpipeline::metadata::CtPipelineMetadata,
    mode: &str,
) -> Result<CtPipelineData, crate::error::CtDiagnosticError> {
    meta.content_type = Some("text/plain".to_string());
    let buf_reader = std::io::BufReader::new(reader);
    let mut lines = Vec::new();
    for (idx, line_res) in buf_reader.lines().enumerate() {
        let line = line_res.map_err(|e| {
            decode_io_error(
                format!("stdout decode({mode}) failed at line {}", idx + 1),
                e,
            )
        })?;
        lines.push(ctpipeline::value::CtValue::String(line));
    }
    Ok(CtPipelineData::ListStream(
        ctpipeline::pipeline_data::CtListStream::new(lines.into_iter(), meta),
    ))
}

fn decode_ssv_from_str(
    text: &str,
    mut meta: ctpipeline::metadata::CtPipelineMetadata,
) -> Result<CtPipelineData, crate::error::CtDiagnosticError> {
    meta.content_type = Some("text/x-ssv".to_string());

    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let Some(header_line) = lines.next() else {
        return Ok(CtPipelineData::ListStream(
            ctpipeline::pipeline_data::CtListStream::new(
                std::iter::empty::<ctpipeline::CtValue>(),
                meta,
            ),
        ));
    };
    let headers = header_line
        .split_whitespace()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();

    let mut rows = Vec::new();
    for line in lines {
        let cols = line.split_whitespace().collect::<Vec<_>>();
        let mut fields = Vec::with_capacity(headers.len().max(cols.len()));
        for (idx, header) in headers.iter().enumerate() {
            let val = cols.get(idx).copied().unwrap_or("");
            fields.push((header.clone(), infer_text_scalar(val)));
        }
        for (idx, col) in cols.iter().enumerate().skip(headers.len()) {
            fields.push((format!("column{idx}"), infer_text_scalar(col)));
        }
        rows.push(ctpipeline::CtValue::Record(fields));
    }

    Ok(CtPipelineData::ListStream(
        ctpipeline::pipeline_data::CtListStream::new(rows.into_iter(), meta),
    ))
}

fn infer_text_scalar(raw: &str) -> ctpipeline::CtValue {
    let s = raw.trim();
    if s.is_empty() {
        return ctpipeline::CtValue::Nothing;
    }
    if s.eq_ignore_ascii_case("true") {
        return ctpipeline::CtValue::Bool(true);
    }
    if s.eq_ignore_ascii_case("false") {
        return ctpipeline::CtValue::Bool(false);
    }
    if let Ok(n) = s.parse::<i64>() {
        return ctpipeline::CtValue::Int(n);
    }
    if let Ok(n) = s.parse::<f64>() {
        return ctpipeline::CtValue::Float(n);
    }
    ctpipeline::CtValue::String(raw.to_string())
}

fn looks_like_csv(text: &str) -> bool {
    let mut non_empty = text.lines().filter(|line| !line.trim().is_empty());
    let Some(first) = non_empty.next() else {
        return false;
    };
    // The first line must contain a comma and look like a header row:
    // every comma-separated token must be non-empty and not a bare number.
    if !first.contains(',') {
        return false;
    }
    let header_tokens: Vec<&str> = first.split(',').collect();
    if header_tokens.len() < 2 {
        return false;
    }
    // Reject if any header token is empty or purely numeric – that means the
    // first line is plain data (or a sentence like "hello, world"), not a header.
    if header_tokens
        .iter()
        .any(|t| t.trim().is_empty() || t.trim().parse::<f64>().is_ok())
    {
        return false;
    }
    let expected_cols = header_tokens.len();
    // Need at least 2 data rows that all share the same column count.
    let mut matching = 0usize;
    for line in non_empty {
        if !line.contains(',') {
            continue;
        }
        if line.split(',').count() == expected_cols {
            matching += 1;
        }
    }
    matching >= 2
}

fn looks_like_ssv(text: &str) -> bool {
    let lines = text
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .take(24)
        .collect::<Vec<_>>();
    if lines.len() < 2 {
        return false;
    }

    let headers = lines[0].split_whitespace().collect::<Vec<_>>();
    if headers.len() < 2 || headers.len() > 32 {
        return false;
    }
    if headers
        .iter()
        .any(|h| h.len() > 32 || h.parse::<f64>().is_ok())
    {
        return false;
    }
    if headers.iter().any(|h| {
        !h.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '%' | '/'))
    }) {
        return false;
    }

    let mut sampled = 0usize;
    let mut compatible = 0usize;
    let mut has_numeric_value = false;
    for line in lines.iter().skip(1) {
        let cols = line.split_whitespace().collect::<Vec<_>>();
        if cols.len() < 2 {
            continue;
        }
        sampled += 1;
        if cols.len() + 1 >= headers.len() {
            compatible += 1;
        }
        if cols.iter().any(|c| c.parse::<f64>().is_ok()) {
            has_numeric_value = true;
        }
    }

    // Require at least 3 sampled data rows so that a short two-line message
    // (e.g. "hello world there / line 1 now") is never silently treated as SSV.
    sampled >= 3 && compatible * 100 >= sampled * 70 && has_numeric_value
}

fn looks_like_text(text: &str) -> bool {
    !text
        .chars()
        .any(|ch| ch.is_control() && ch != '\n' && ch != '\r' && ch != '\t')
}

#[cfg(test)]
mod decoder_tests {
    use super::*;
    use ctpipeline::metadata::CtPipelineMetadata;
    use ctpipeline::value::CtValue;
    use std::io::Cursor;

    struct ChunkedReader {
        chunks: Vec<Vec<u8>>,
        chunk_idx: usize,
        offset: usize,
    }

    impl ChunkedReader {
        fn new(chunks: Vec<Vec<u8>>) -> Self {
            Self {
                chunks,
                chunk_idx: 0,
                offset: 0,
            }
        }
    }

    impl Read for ChunkedReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            while self.chunk_idx < self.chunks.len() {
                let chunk = &self.chunks[self.chunk_idx];
                if self.offset >= chunk.len() {
                    self.chunk_idx += 1;
                    self.offset = 0;
                    continue;
                }
                let n = (chunk.len() - self.offset).min(buf.len());
                buf[..n].copy_from_slice(&chunk[self.offset..self.offset + n]);
                self.offset += n;
                return Ok(n);
            }
            Ok(0)
        }
    }

    #[test]
    fn test_decode_raw() {
        let meta = CtPipelineMetadata::default();
        let cursor = Cursor::new(b"hello".to_vec());
        let data = ExternalOutputDecoder::decode(cursor, ExternalStdoutMode::Raw, meta).unwrap();
        if let CtPipelineData::ByteStream(mut b) = data {
            let mut buf = Vec::new();
            b.read_to_end(&mut buf).unwrap();
            assert_eq!(buf, b"hello");
        } else {
            panic!("Expected ByteStream");
        }
    }

    #[test]
    fn test_decode_text_lines() {
        let meta = CtPipelineMetadata::default();
        let cursor = Cursor::new(b"line1\nline2\n".to_vec());
        let data =
            ExternalOutputDecoder::decode(cursor, ExternalStdoutMode::TextLines, meta).unwrap();
        if let CtPipelineData::ListStream(mut stream) = data {
            let v1 = stream.next().unwrap();
            let v2 = stream.next().unwrap();
            assert_eq!(v1.to_text(), "line1");
            assert_eq!(v2.to_text(), "line2");
        } else {
            panic!("Expected ListStream");
        }
    }

    #[test]
    fn test_decode_json() {
        let meta = CtPipelineMetadata::default();
        let cursor = Cursor::new(b"{\"a\": 1}".to_vec());
        let data = ExternalOutputDecoder::decode(cursor, ExternalStdoutMode::Json, meta).unwrap();
        if let CtPipelineData::Value(CtValue::Record(r), _) = data {
            assert_eq!(r[0].0, "a");
            assert_eq!(r[0].1.as_int().unwrap(), 1);
        } else {
            panic!("Expected Value(Record)");
        }
    }

    #[test]
    fn test_decode_json_lines() {
        let meta = CtPipelineMetadata::default();
        let cursor = Cursor::new(b"{\"a\": 1}\n{\"b\": 2}\n".to_vec());
        let data =
            ExternalOutputDecoder::decode(cursor, ExternalStdoutMode::JsonLines, meta).unwrap();
        if let CtPipelineData::ListStream(stream) = data {
            let items: Vec<_> = stream.collect();
            assert_eq!(items.len(), 2);
        } else {
            panic!("Expected ListStream");
        }
    }

    #[test]
    fn test_decode_csv() {
        let meta = CtPipelineMetadata::default();
        let cursor = Cursor::new(b"name,age\nAlice,30\nBob,25\n".to_vec());
        let data = ExternalOutputDecoder::decode(cursor, ExternalStdoutMode::Csv, meta).unwrap();
        if let CtPipelineData::ListStream(stream) = data {
            let items: Vec<_> = stream.collect();
            assert_eq!(items.len(), 2);
            if let CtValue::Record(ref pairs) = items[0] {
                assert_eq!(pairs[0].0, "name");
                assert_eq!(pairs[0].1.to_text(), "Alice");
                assert_eq!(pairs[1].0, "age");
                assert_eq!(pairs[1].1.to_text(), "30");
            } else {
                panic!("Expected Record");
            }
        } else {
            panic!("Expected ListStream");
        }
    }

    #[test]
    fn test_decode_auto_json() {
        let meta = CtPipelineMetadata::default();
        let cursor = Cursor::new(br#"{"name":"demo","id":1}"#.to_vec());
        let data = ExternalOutputDecoder::decode(cursor, ExternalStdoutMode::Auto, meta).unwrap();
        assert!(matches!(data, CtPipelineData::Value(CtValue::Record(_), _)));
    }

    #[test]
    fn test_decode_auto_csv() {
        let meta = CtPipelineMetadata::default();
        let cursor = Cursor::new(b"name,age\nAlice,30\nBob,25\n".to_vec());
        let data = ExternalOutputDecoder::decode(cursor, ExternalStdoutMode::Auto, meta).unwrap();
        if let CtPipelineData::ListStream(stream) = data {
            let items: Vec<_> = stream.collect();
            assert_eq!(items.len(), 2);
        } else {
            panic!("Expected ListStream");
        }
    }

    #[test]
    fn test_decode_auto_ssv() {
        let meta = CtPipelineMetadata::default();
        // Three data rows are required for the SSV heuristic to fire.
        let cursor = Cursor::new(b"PID USER RSS\n1 root 1024\n2 app 2048\n3 svc 512\n".to_vec());
        let data = ExternalOutputDecoder::decode(cursor, ExternalStdoutMode::Auto, meta).unwrap();
        let CtPipelineData::ListStream(stream) = data else {
            panic!("Expected ListStream");
        };
        let rows: Vec<_> = stream.collect();
        assert_eq!(rows.len(), 3);
        let CtValue::Record(fields) = &rows[0] else {
            panic!("Expected first row record");
        };
        assert!(
            fields
                .iter()
                .any(|(k, v)| k == "PID" && matches!(v, CtValue::Int(1)))
        );
        assert!(
            fields
                .iter()
                .any(|(k, v)| k == "USER" && matches!(v, CtValue::String(s) if s == "root"))
        );
        assert!(
            fields
                .iter()
                .any(|(k, v)| k == "RSS" && matches!(v, CtValue::Int(1024)))
        );
    }

    #[test]
    fn test_decode_auto_text_lines() {
        let meta = CtPipelineMetadata::default();
        let cursor = Cursor::new(b"line1\nline2\n".to_vec());
        let data = ExternalOutputDecoder::decode(cursor, ExternalStdoutMode::Auto, meta).unwrap();
        if let CtPipelineData::ListStream(mut stream) = data {
            assert_eq!(stream.next().unwrap().to_text(), "line1");
            assert_eq!(stream.next().unwrap().to_text(), "line2");
        } else {
            panic!("Expected ListStream");
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_decode_auto_external_partial_probe_returns_quickly_as_raw() {
        let args = vec!["-c".to_string(), "printf 'ready\\n'; sleep 2".to_string()];
        let mut spec = ExternalCallSpec::quick("sh", &args);
        spec.stdout_mode = ExternalStdoutMode::Auto;
        let ctx = crate::context::DataEngineContext::empty_for_test();

        let started = std::time::Instant::now();
        let data = ExternalExecutor::run(spec, CtPipelineData::Empty, &ctx)
            .expect("external run should succeed");
        assert!(
            started.elapsed() < std::time::Duration::from_millis(1000),
            "auto decode should not block waiting for EOF on running process"
        );
        assert!(matches!(data, CtPipelineData::ByteStream(_)));
    }

    #[cfg(unix)]
    #[test]
    fn test_decode_auto_external_full_probe_running_returns_quickly_as_raw() {
        // >8KiB in first burst so probe fills, then keep process alive.
        let args = vec![
            "-c".to_string(),
            "yes a | head -c 9000; sleep 2".to_string(),
        ];
        let mut spec = ExternalCallSpec::quick("sh", &args);
        spec.stdout_mode = ExternalStdoutMode::Auto;
        let ctx = crate::context::DataEngineContext::empty_for_test();

        let started = std::time::Instant::now();
        let data = ExternalExecutor::run(spec, CtPipelineData::Empty, &ctx)
            .expect("external run should succeed");
        assert!(
            started.elapsed() < std::time::Duration::from_millis(1000),
            "auto decode should not block on full-probe running process"
        );
        assert!(matches!(data, CtPipelineData::ByteStream(_)));
    }

    #[test]
    fn test_decode_auto_non_tabular_text_not_ssv() {
        let meta = CtPipelineMetadata::default();
        let cursor = Cursor::new(b"INFO service started\nWARN retry later\n".to_vec());
        let data = ExternalOutputDecoder::decode(cursor, ExternalStdoutMode::Auto, meta).unwrap();
        let CtPipelineData::ListStream(mut stream) = data else {
            panic!("Expected ListStream");
        };
        assert!(matches!(stream.next(), Some(CtValue::String(s)) if s == "INFO service started"));
        assert!(matches!(stream.next(), Some(CtValue::String(s)) if s == "WARN retry later"));
    }

    /// Regression: plain comma-separated sentences must NOT be auto-decoded as CSV.
    /// e.g. `python3 -c "print('hello, world');print('bye, now')"`
    #[test]
    fn test_decode_auto_plain_comma_text_not_csv() {
        let meta = CtPipelineMetadata::default();
        let cursor = Cursor::new(b"hello, world\nbye, now\n".to_vec());
        let data = ExternalOutputDecoder::decode(cursor, ExternalStdoutMode::Auto, meta).unwrap();
        let CtPipelineData::ListStream(mut stream) = data else {
            panic!("Expected ListStream of plain text, not structured records");
        };
        assert!(matches!(stream.next(), Some(CtValue::String(s)) if s == "hello, world"));
        assert!(matches!(stream.next(), Some(CtValue::String(s)) if s == "bye, now"));
    }

    /// Regression: two-line whitespace-separated output must NOT be auto-decoded as SSV.
    /// e.g. "hello world there\nline 1 now"
    #[test]
    fn test_decode_auto_two_line_ssv_not_decoded() {
        let meta = CtPipelineMetadata::default();
        let cursor = Cursor::new(b"hello world there\nline 1 now\n".to_vec());
        let data = ExternalOutputDecoder::decode(cursor, ExternalStdoutMode::Auto, meta).unwrap();
        let CtPipelineData::ListStream(mut stream) = data else {
            panic!("Expected ListStream of plain text");
        };
        assert!(matches!(stream.next(), Some(CtValue::String(s)) if s == "hello world there"));
        assert!(matches!(stream.next(), Some(CtValue::String(s)) if s == "line 1 now"));
    }

    /// Regression (P2): when CSV sniffing succeeds but decode_csv fails (e.g. unclosed quote),
    /// auto mode must fall back to plain text lines, NOT a raw ByteStream.
    #[test]
    fn test_decode_auto_malformed_csv_falls_back_to_text_lines() {
        let meta = CtPipelineMetadata::default();
        // Three data rows for looks_like_csv to fire, but second row has an unclosed quote
        // that causes the CSV parser to fail.
        let input = b"name,age,city\n\"Alice,30,\"London\nBob,25,Paris\nCharlie,22,Rome\n";
        let cursor = Cursor::new(input.to_vec());
        let data = ExternalOutputDecoder::decode(cursor, ExternalStdoutMode::Auto, meta).unwrap();
        // Must NOT be a ByteStream — input is valid UTF-8 text.
        assert!(
            matches!(data, CtPipelineData::ListStream(_)),
            "Expected ListStream fallback, got something else"
        );
    }

    #[test]
    fn test_decode_auto_invalid_json_falls_back_to_text_lines() {
        let meta = CtPipelineMetadata::default();
        let cursor = Cursor::new(b"{invalid json}".to_vec());
        let data = ExternalOutputDecoder::decode(cursor, ExternalStdoutMode::Auto, meta).unwrap();
        let CtPipelineData::ListStream(mut stream) = data else {
            panic!("Expected ListStream");
        };
        assert!(matches!(stream.next(), Some(CtValue::String(s)) if s == "{invalid json}"));
    }

    #[test]
    fn test_decode_auto_binary_falls_back_to_raw() {
        let meta = CtPipelineMetadata::default();
        let cursor = Cursor::new(vec![0x00, 0xFF, 0x10, 0x41]);
        let data = ExternalOutputDecoder::decode(cursor, ExternalStdoutMode::Auto, meta).unwrap();
        if let CtPipelineData::ByteStream(mut stream) = data {
            let mut buf = Vec::new();
            stream.read_to_end(&mut buf).unwrap();
            assert_eq!(buf, vec![0x00, 0xFF, 0x10, 0x41]);
        } else {
            panic!("Expected ByteStream");
        }
    }

    #[test]
    fn test_read_capped_stderr_drains_but_retains_only_limit() {
        let input = vec![b'x'; 70 * 1024];
        let retained = read_capped_stderr(std::io::Cursor::new(input)).unwrap();

        assert_eq!(retained.len(), 64 * 1024);
    }

    #[test]
    fn test_decode_auto_large_output_falls_back_to_raw_stream() {
        let meta = CtPipelineMetadata::default();
        let payload = vec![b'a'; AUTO_DECODE_MAX_BYTES + 8];
        let cursor = Cursor::new(payload.clone());
        let data = ExternalOutputDecoder::decode(cursor, ExternalStdoutMode::Auto, meta).unwrap();
        if let CtPipelineData::ByteStream(mut stream) = data {
            let mut buf = Vec::new();
            stream.read_to_end(&mut buf).unwrap();
            assert_eq!(buf, payload);
        } else {
            panic!("Expected ByteStream");
        }
    }

    #[test]
    fn test_decode_auto_json_probe_overflow_preserves_tail_bytes() {
        let meta = CtPipelineMetadata::default();
        let mut payload = Vec::with_capacity(AUTO_DECODE_MAX_BYTES + 64);
        payload.push(b'{');
        payload.extend(vec![b'a'; AUTO_DECODE_MAX_BYTES + 16]);
        payload.push(b'}');
        let split = ChunkedReader::new(vec![payload[..1].to_vec(), payload[1..].to_vec()]);

        let data = ExternalOutputDecoder::decode(split, ExternalStdoutMode::Auto, meta).unwrap();
        if let CtPipelineData::ByteStream(mut stream) = data {
            let mut buf = Vec::new();
            stream.read_to_end(&mut buf).unwrap();
            assert_eq!(buf, payload);
        } else {
            panic!("Expected ByteStream");
        }
    }

    #[test]
    fn test_decode_auto_plain_text_with_late_non_utf8_falls_back_to_raw() {
        let meta = CtPipelineMetadata::default();
        let chunks = vec![
            b"line1\nline2\n".to_vec(),
            vec![0xE2, 0x82], // truncated UTF-8 lead sequence in later burst
            b"\n".to_vec(),
        ];
        let reader = ChunkedReader::new(chunks.clone());
        let expected = chunks.concat();
        let data = ExternalOutputDecoder::decode(reader, ExternalStdoutMode::Auto, meta).unwrap();
        let CtPipelineData::ByteStream(mut stream) = data else {
            panic!("Expected ByteStream fallback for late non-UTF8 payload");
        };
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).unwrap();
        assert_eq!(buf, expected);
    }

    #[test]
    fn test_decode_json_invalid_returns_error() {
        let meta = CtPipelineMetadata::default();
        let cursor = Cursor::new(b"{invalid json}".to_vec());
        let err = ExternalOutputDecoder::decode(cursor, ExternalStdoutMode::Json, meta)
            .expect_err("expected decode error");
        assert!(err.to_string().contains("decode(json)"));
    }

    #[test]
    fn test_decode_csv_invalid_returns_error() {
        let meta = CtPipelineMetadata::default();
        // Unclosed quote causes CSV parser error on first data row.
        let cursor = Cursor::new(b"name,age\n\"Alice,30\n".to_vec());
        let err = ExternalOutputDecoder::decode(cursor, ExternalStdoutMode::Csv, meta)
            .expect_err("expected decode error");
        assert!(err.to_string().contains("decode(csv)"));
    }

    #[test]
    fn test_wrap_external_decode_error_keeps_inner_exit_code() {
        let wrapped = wrap_external_decode_error(
            "demo",
            crate::error::CtDiagnosticError::simple("timed out after in stderr payload"),
        );
        assert_eq!(wrapped.code, crate::execution::exit_code::RUNTIME_ERROR);

        let wrapped_timeout = wrap_external_decode_error(
            "demo",
            crate::error::CtDiagnosticError::simple("io timeout")
                .with_code(crate::execution::exit_code::TIMEOUT),
        );
        assert_eq!(wrapped_timeout.code, crate::execution::exit_code::TIMEOUT);
    }

    #[test]
    fn test_insert_external_metadata_custom_recovers_poisoned_lock() {
        let custom = std::sync::Arc::new(std::sync::Mutex::new(std::collections::BTreeMap::new()));
        let poisoned = custom.clone();
        let _ = std::panic::catch_unwind(move || {
            let _guard = poisoned.lock().unwrap();
            panic!("poison custom metadata");
        });

        insert_external_metadata_custom(
            &custom,
            vec![("external.exit_code", ctpipeline::CtValue::Int(9))],
        );

        let guard = custom
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(
            guard.get("external.exit_code"),
            Some(&ctpipeline::CtValue::Int(9))
        );
    }
}

struct ExternalStream {
    stdout: Option<Box<dyn std::io::Read + Send>>,
    stderr_thread: Option<std::thread::JoinHandle<Vec<u8>>>,
    child: std::sync::Arc<std::sync::Mutex<std::process::Child>>,
    spec: ExternalCallSpec,
    start_time: std::time::Instant,
    timeout_abort: std::sync::Arc<std::sync::atomic::AtomicBool>,
    process_done: std::sync::Arc<std::sync::atomic::AtomicBool>,
    stderr_buf: Option<std::io::Cursor<Vec<u8>>>,
    stdin_handle: Option<std::thread::JoinHandle<()>>,
    meta_custom: Option<MetadataCustom>,
}

fn insert_external_metadata_custom<I, K>(custom: &MetadataCustom, entries: I)
where
    I: IntoIterator<Item = (K, ctpipeline::CtValue)>,
    K: Into<String>,
{
    let mut guard = custom
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for (key, value) in entries {
        guard.insert(key.into(), value);
    }
}

impl Read for ExternalStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if let Some(stdout) = self.stdout.as_mut() {
            let n = stdout.read(buf)?;
            if n > 0 {
                return Ok(n);
            }
            // EOF reached on stdout.
            self.stdout = None;
        }

        if self.stderr_buf.is_none() {
            if let Some(handle) = self.stdin_handle.take() {
                let _ = handle.join();
            }

            let status = loop {
                let maybe_status = {
                    let mut child = self
                        .child
                        .lock()
                        .map_err(|_| std::io::Error::other("external process lock poisoned"))?;
                    child
                        .try_wait()
                        .map_err(|e| std::io::Error::other(e.to_string()))?
                };
                if let Some(status) = maybe_status {
                    break status;
                }

                if self
                    .timeout_abort
                    .load(std::sync::atomic::Ordering::Relaxed)
                    && let Ok(mut child) = self.child.lock()
                {
                    let _ = child.kill();
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            };
            self.process_done
                .store(true, std::sync::atomic::Ordering::Relaxed);

            let duration_ms = self.start_time.elapsed().as_millis() as u64;
            let timed_out = self
                .timeout_abort
                .load(std::sync::atomic::Ordering::Relaxed);

            let stderr_bytes = if let Some(th) = self.stderr_thread.take() {
                th.join().unwrap_or_default()
            } else {
                Vec::new()
            };

            let stderr_summary =
                String::from_utf8_lossy(&stderr_bytes[..stderr_bytes.len().min(64 * 1024)])
                    .into_owned();
            let exit_code = status.code().unwrap_or(-1);

            if let Some(custom) = &self.meta_custom {
                let mut entries = vec![
                    (
                        "external.exit_code",
                        ctpipeline::CtValue::Int(exit_code as i64),
                    ),
                    (
                        "external.duration_ms",
                        ctpipeline::CtValue::Int(duration_ms.min(i64::MAX as u64) as i64),
                    ),
                ];
                if self.spec.stderr_mode == ExternalStderrMode::Capture {
                    entries.push((
                        "external.stderr_summary",
                        ctpipeline::CtValue::String(stderr_summary.clone()),
                    ));
                }
                insert_external_metadata_custom(custom, entries);
            }

            if timed_out {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!(
                        "External command '{}' timed out after {}ms\nStderr:\n{}",
                        self.spec.cmd, duration_ms, stderr_summary
                    ),
                ));
            }

            if self.spec.exit_policy == ExternalExitPolicy::FailOnNonZero && !status.success() {
                return Err(std::io::Error::other(format!(
                    "External command '{}' failed with exit code {} (duration {}ms)\nStderr:\n{}",
                    self.spec.cmd, exit_code, duration_ms, stderr_summary
                )));
            }

            if self.spec.stderr_mode == ExternalStderrMode::MergeToStdout {
                self.stderr_buf = Some(std::io::Cursor::new(stderr_bytes));
            } else {
                self.stderr_buf = Some(std::io::Cursor::new(Vec::new()));
            }
        }

        if let Some(cur) = self.stderr_buf.as_mut() {
            return cur.read(buf);
        }

        Ok(0)
    }
}

impl Drop for ExternalStream {
    fn drop(&mut self) {
        self.process_done
            .store(true, std::sync::atomic::Ordering::Relaxed);
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

pub struct ExternalExecutor;

impl ExternalExecutor {
    /// 执行外部命令，处理 stdin/stdout/stderr 管道，并返回输出管线数据
    pub fn run(
        spec: ExternalCallSpec,
        input: ctpipeline::pipeline_data::CtPipelineData,
        _ctx: &crate::context::DataEngineContext,
    ) -> Result<ctpipeline::pipeline_data::CtPipelineData, crate::error::CtDiagnosticError> {
        let mut cmd = std::process::Command::new(&spec.cmd);
        cmd.args(&spec.args);
        let stdin_input = if matches!(input, CtPipelineData::Empty) {
            None
        } else {
            Some(input)
        };

        if let Some(cwd) = &spec.cwd {
            cmd.current_dir(cwd);
        }
        cmd.envs(&spec.env_overrides);

        if stdin_input.is_some() {
            cmd.stdin(std::process::Stdio::piped());
        } else {
            cmd.stdin(std::process::Stdio::inherit());
        }

        let mut interleaved_read = None;

        match spec.stderr_mode {
            ExternalStderrMode::Inherit => {
                cmd.stdout(std::process::Stdio::piped());
                cmd.stderr(std::process::Stdio::inherit());
            }
            ExternalStderrMode::Capture => {
                cmd.stdout(std::process::Stdio::piped());
                cmd.stderr(std::process::Stdio::piped());
            }
            ExternalStderrMode::MergeToStdout => {
                // Use os_pipe to truly interleave stdout and stderr at the OS pipe level
                let (reader, writer) = os_pipe::pipe().map_err(|e| {
                    crate::error::CtDiagnosticError::simple(format!(
                        "failed to create merged stderr pipe for '{}': {}",
                        spec.cmd, e
                    ))
                })?;
                let writer_clone = writer.try_clone().map_err(|e| {
                    crate::error::CtDiagnosticError::simple(format!(
                        "failed to duplicate merged stderr pipe for '{}': {}",
                        spec.cmd, e
                    ))
                })?;
                cmd.stdout(std::process::Stdio::from(writer_clone));
                cmd.stderr(std::process::Stdio::from(writer));
                interleaved_read = Some(reader);
            }
        }

        let start_time = std::time::Instant::now();
        let mut child = cmd.spawn().map_err(|e| {
            crate::error::CtDiagnosticError::simple(format!(
                "failed to spawn external command '{}': {}",
                spec.cmd, e
            ))
        })?;

        let timeout_abort = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let process_done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

        let stdin_mode = spec.stdin_mode;
        // 使用独立线程写入 stdin，避免 wait_with_output 与 stdin 写入互相阻塞
        let stdin_handle = stdin_input.and_then(|input| {
            child.stdin.take().map(|mut stdin| {
                std::thread::spawn(move || {
                    let _ = ExternalInputEncoder::encode(input, stdin_mode, &mut stdin);
                })
            })
        });

        let stderr_thread = if spec.stderr_mode == ExternalStderrMode::Capture {
            child.stderr.take().map(|mut stderr| {
                std::thread::spawn(move || read_capped_stderr(&mut stderr).unwrap_or_default())
            })
        } else {
            // For MergeToStdout or Inherit, we do not separately capture stderr
            None
        };

        // If MergeToStdout uses os_pipe, use the reader side of the pipe as stdout
        let stdout: Option<Box<dyn std::io::Read + Send>> = if let Some(r) = interleaved_read {
            Some(Box::new(r))
        } else {
            child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>)
        };
        let child = std::sync::Arc::new(std::sync::Mutex::new(child));

        // 超时 watchdog：超时后标记并尝试终止子进程
        if let Some(ms) = spec.timeout_ms {
            let timeout_abort_clone = timeout_abort.clone();
            let process_done_clone = process_done.clone();
            let child_for_timeout = child.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(ms));
                if process_done_clone.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                timeout_abort_clone.store(true, std::sync::atomic::Ordering::Relaxed);
                if let Ok(mut child) = child_for_timeout.lock() {
                    #[cfg(unix)]
                    {
                        let pid = child.id() as i32;
                        unsafe {
                            libc::kill(pid, libc::SIGTERM);
                        }
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    let _ = child.kill();
                }
            });
        }

        let meta = ctpipeline::metadata::CtPipelineMetadata {
            data_source: ctpipeline::metadata::CtDataSource::ExternalCommand {
                command: spec.cmd.clone(),
            },
            ..Default::default()
        };

        // Inject initial eager metadata to fulfill FR-6.5 contract.
        // True exit_code/stderr/duration will only be known after the stream ends,
        // but downstream requires these keys to exist eagerly.
        let init_duration = start_time.elapsed().as_millis() as u64;
        let mut entries = vec![
            (
                "external.command",
                ctpipeline::CtValue::String(spec.cmd.clone()),
            ),
            (
                "external.args",
                ctpipeline::CtValue::List(
                    spec.args
                        .iter()
                        .map(|s| ctpipeline::CtValue::String(s.clone()))
                        .collect(),
                ),
            ),
            (
                "external.cwd",
                ctpipeline::CtValue::String(
                    spec.cwd
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| ".".to_string()),
                ),
            ),
            ("external.exit_code", ctpipeline::CtValue::Int(0)),
            (
                "external.duration_ms",
                ctpipeline::CtValue::Int(init_duration.min(i64::MAX as u64) as i64),
            ),
        ];
        if spec.stderr_mode == ExternalStderrMode::Capture {
            entries.push((
                "external.stderr_summary",
                ctpipeline::CtValue::String(String::new()),
            ));
        }
        insert_external_metadata_custom(&meta.custom, entries);

        let external_stream = ExternalStream {
            stdout,
            stderr_thread,
            child,
            spec: spec.clone(),
            start_time,
            timeout_abort,
            process_done,
            stderr_buf: None,
            stdin_handle,
            meta_custom: Some(meta.custom.clone()),
        };

        ExternalOutputDecoder::decode(external_stream, spec.stdout_mode, meta)
            .map_err(|e| wrap_external_decode_error(&spec.cmd, e))
    }
}

fn wrap_external_decode_error(
    cmd: &str,
    err: crate::error::CtDiagnosticError,
) -> crate::error::CtDiagnosticError {
    crate::error::CtDiagnosticError::simple(format!(
        "External command '{cmd}' decode failed: {err}"
    ))
    .with_code(err.code)
}
