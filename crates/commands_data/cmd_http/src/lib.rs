/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 */

//! `cmd_http` — 结构化 HTTP 请求命令（MVP: GET）。

use ctengine::command::DataCommand;
use ctengine::context::DataEngineContext;
use ctengine::error::CtDiagnosticError;
use ctengine::execution::{CommandCore, CommandRunner};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};

#[derive(Default)]
pub struct CmdHttp;

struct HttpCore;

impl DataCommand for CmdHttp {
    fn signature(&self) -> DataSignature {
        DataSignature::new(
            "http",
            "structured http request (blocking mvp, supports get)",
        )
        .positional(CtPositionalArg::required(
            "method_or_url",
            "HTTP method (get) or URL (default method: get)",
            CtType::String,
        ))
        .positional(CtPositionalArg::optional(
            "url",
            "request url (required when method is provided)",
            CtType::String,
        ))
        .input(CtType::Nothing)
        .output(CtType::Any)
    }

    fn run(
        &self,
        call: &DataCall,
        input: CtPipelineData,
        ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        CommandRunner::run(&HttpCore, call, input, ctx)
    }
}

impl CommandCore for HttpCore {
    fn run_core(
        &self,
        call: &DataCall,
        _input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let first: String = call
            .req(0)
            .map_err(|e| CtDiagnosticError::simple(format!("http: {e}")))?;
        let second: Option<String> = call
            .opt(1)
            .map_err(|e| CtDiagnosticError::simple(format!("http: {e}")))?;
        http_core_pipeline(first, second)
    }
}

/// 统一核心输出：供 DataCommand 与 Legacy Tool 共同复用。
pub fn http_core_pipeline(
    first: String,
    second: Option<String>,
) -> Result<CtPipelineData, CtDiagnosticError> {
    let first_lower = first.to_ascii_lowercase();
    let (method, url) = match second {
        Some(url) => (first_lower, url),
        None => {
            if is_http_method_token(&first_lower) {
                return Err(CtDiagnosticError::simple(format!(
                    "http: missing url for method `{first_lower}`"
                )));
            }
            ("get".to_string(), first)
        }
    };

    if method != "get" {
        return Err(CtDiagnosticError::simple(format!(
            "http: unsupported method `{method}`, only `get` is supported"
        )));
    }

    let (value, content_type, status) = http_get(&url)?;
    let mut meta = CtPipelineMetadata::builtin("http");
    if !content_type.is_empty() {
        meta.content_type = Some(content_type.clone());
    }
    if let Ok(mut custom) = meta.custom.lock() {
        custom.insert("status".to_string(), CtValue::Int(status as i64));
        custom.insert("url".to_string(), CtValue::String(url));
    }
    Ok(CtPipelineData::Value(value, meta))
}

fn http_get(url: &str) -> Result<(CtValue, String, u16), CtDiagnosticError> {
    let response = ureq::get(url).call().map_err(|err| match err {
        ureq::Error::Status(code, resp) => {
            let detail = resp
                .into_string()
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "empty response body".to_string());
            CtDiagnosticError::simple(format!(
                "http: GET {url} failed with status {code}: {detail}"
            ))
        }
        other => CtDiagnosticError::simple(format!("http: GET {url} failed: {other}")),
    })?;

    let status = response.status();
    let content_type = response
        .header("content-type")
        .unwrap_or_default()
        .to_string();
    let body = response.into_string().map_err(|e| {
        CtDiagnosticError::simple(format!("http: failed to read response body: {e}"))
    })?;

    Ok((decode_body(&content_type, &body)?, content_type, status))
}

fn decode_body(content_type: &str, body: &str) -> Result<CtValue, CtDiagnosticError> {
    let trimmed = body.trim();
    let declared_json = is_json_content(content_type);
    if declared_json {
        if trimmed.is_empty() {
            return Ok(CtValue::Nothing);
        }
        let value: serde_json::Value = serde_json::from_str(trimmed).map_err(|e| {
            CtDiagnosticError::simple(format!("http: failed to parse JSON response: {e}"))
        })?;
        return Ok(json_to_ct(value));
    }
    if looks_like_json(trimmed)
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed)
    {
        return Ok(json_to_ct(value));
    }
    Ok(CtValue::String(body.to_string()))
}

fn is_json_content(content_type: &str) -> bool {
    content_type.to_ascii_lowercase().contains("json")
}

fn is_http_method_token(raw: &str) -> bool {
    matches!(
        raw,
        "get" | "post" | "put" | "patch" | "delete" | "head" | "options"
    )
}

fn looks_like_json(trimmed: &str) -> bool {
    trimmed.starts_with('{') || trimmed.starts_with('[')
}

fn json_to_ct(v: serde_json::Value) -> CtValue {
    match v {
        serde_json::Value::Null => CtValue::Nothing,
        serde_json::Value::Bool(b) => CtValue::Bool(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                CtValue::Int(i)
            } else if let Some(f) = n.as_f64() {
                CtValue::Float(f)
            } else {
                CtValue::String(n.to_string())
            }
        }
        serde_json::Value::String(s) => CtValue::String(s),
        serde_json::Value::Array(arr) => CtValue::List(arr.into_iter().map(json_to_ct).collect()),
        serde_json::Value::Object(map) => CtValue::Record(
            map.into_iter()
                .map(|(k, v)| (k, json_to_ct(v)))
                .collect::<Vec<_>>(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctengine::context::{CommandRegistry, DataEngineContext};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn ctx() -> DataEngineContext {
        DataEngineContext::new(CommandRegistry::empty(), None, None)
    }

    fn call_http(args: &[&str]) -> DataCall {
        let mut c = DataCall::named("http");
        for arg in args {
            c.positionals.push(ctsig::BoundArg::new(
                CtValue::String((*arg).to_string()),
                None,
            ));
        }
        c
    }

    fn spawn_server(status: u16, content_type: &str, body: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind server");
        let addr = listener.local_addr().expect("server addr");
        let status_line = match status {
            200 => "HTTP/1.1 200 OK",
            404 => "HTTP/1.1 404 Not Found",
            _ => "HTTP/1.1 500 Internal Server Error",
        }
        .to_string();
        let content_type = content_type.to_string();
        let body = body.to_string();

        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut req = [0_u8; 1024];
            let _ = stream.read(&mut req);
            let resp = format!(
                "{status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(resp.as_bytes()).expect("write response");
            stream.flush().expect("flush response");
        });

        format!("http://{addr}/")
    }

    #[test]
    fn http_get_json_returns_structured_values() {
        let url = spawn_server(
            200,
            "application/json",
            r#"[{"login":"alice","contributions":7}]"#,
        );
        let out = CmdHttp
            .run(&call_http(&["get", &url]), CtPipelineData::Empty, &ctx())
            .expect("http get should succeed");

        let CtPipelineData::Value(CtValue::List(rows), meta) = out else {
            panic!("expected list output");
        };
        assert_eq!(meta.content_type.as_deref(), Some("application/json"));
        assert_eq!(rows.len(), 1);
        let CtValue::Record(fields) = &rows[0] else {
            panic!("expected record row");
        };
        assert!(matches!(
            fields.iter().find(|(k, _)| k == "login").map(|(_, v)| v),
            Some(CtValue::String(v)) if v == "alice"
        ));
    }

    #[test]
    fn http_url_shorthand_defaults_to_get() {
        let url = spawn_server(200, "text/plain", "ok");
        let out = CmdHttp
            .run(&call_http(&[&url]), CtPipelineData::Empty, &ctx())
            .expect("http shorthand should succeed");
        let CtPipelineData::Value(CtValue::String(body), _) = out else {
            panic!("expected string output");
        };
        assert_eq!(body, "ok");
    }

    #[test]
    fn http_rejects_unsupported_method() {
        let err = CmdHttp
            .run(
                &call_http(&["post", "https://example.com"]),
                CtPipelineData::Empty,
                &ctx(),
            )
            .expect_err("expected unsupported method");
        assert!(err.to_string().contains("unsupported method"));
    }

    #[test]
    fn http_method_without_url_fails_fast() {
        let err = CmdHttp
            .run(&call_http(&["get"]), CtPipelineData::Empty, &ctx())
            .expect_err("expected missing url error");
        assert!(err.to_string().contains("missing url"));
    }

    #[test]
    fn decode_body_json_like_text_falls_back_to_string() {
        let out = decode_body("text/plain", "{not-json}").expect("should fallback to text");
        assert!(matches!(out, CtValue::String(v) if v == "{not-json}"));
    }
}
