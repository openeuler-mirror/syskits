use crate::error::PluginError;
use crate::proto::{HostFrame, PROTOCOL_VERSION, PluginFrame};
use crate::util::{
    protocol_max_frame_bytes, protocol_timeout_ms, read_frame_line, spawn_timeout_killer,
};
use ctengine::context::DataEngineContext;
use ctpipeline::metadata::CtPipelineMetadata;
use ctpipeline::pipeline_data::{CtListStream, CtPipelineData};
use ctsig::DataCall;
use std::io::{BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct PluginHostRunner {
    pub path: PathBuf,
}

impl PluginHostRunner {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn call(
        &self,
        name: &str,
        call: &DataCall,
        input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, PluginError> {
        let timeout_ms = protocol_timeout_ms();
        let max_frame_bytes = protocol_max_frame_bytes();
        let mut child = Command::new(&self.path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let child_id = child.id();
        let timed_out = Arc::new(AtomicBool::new(false));
        let process_done = Arc::new(AtomicBool::new(false));
        spawn_timeout_killer(
            child_id,
            timeout_ms,
            timed_out.clone(),
            process_done.clone(),
        );

        let mut stdin = child.stdin.take().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());

        // 1. Handshake
        let hello_req = HostFrame::Hello {
            protocol: PROTOCOL_VERSION.to_string(),
        };
        writeln!(stdin, "{}", serde_json::to_string(&hello_req)?)?;

        let line = match read_frame_line(&mut stdout, max_frame_bytes)? {
            Some(line) => line,
            None => {
                if timed_out.load(Ordering::Relaxed) {
                    let _ = child.kill();
                    return Err(PluginError::Protocol(format!(
                        "plugin handshake timeout after {timeout_ms}ms"
                    )));
                }
                let _ = child.kill();
                return Err(PluginError::Protocol(
                    "plugin closed stdout before Hello".into(),
                ));
            }
        };
        let resp: PluginFrame = serde_json::from_str(&line)?;
        match resp {
            PluginFrame::Hello { protocol, .. } => {
                if protocol != PROTOCOL_VERSION {
                    let _ = child.kill();
                    return Err(PluginError::Protocol(format!(
                        "protocol version mismatch: {protocol}"
                    )));
                }
            }
            _ => {
                let _ = child.kill();
                return Err(PluginError::Protocol("expected Hello frame".into()));
            }
        }

        // 2. Send Run
        let call_req = HostFrame::Run {
            name: name.to_string(),
            args: call.clone(),
        };
        writeln!(stdin, "{}", serde_json::to_string(&call_req)?)?;

        // 3. Send Input stream frames
        send_input_frames(&mut stdin, input)?;

        let end_req = HostFrame::End;
        writeln!(stdin, "{}", serde_json::to_string(&end_req)?)?;

        // 4. Read Output stream from plugin
        let mut results = Vec::new();
        loop {
            let Some(line) = read_frame_line(&mut stdout, max_frame_bytes)? else {
                break; // EOF
            };

            let frame: PluginFrame = serde_json::from_str(&line)?;
            match frame {
                PluginFrame::Data { value } => {
                    results.push(value);
                }
                PluginFrame::CallResponse {
                    accepted,
                    message,
                    code,
                } => {
                    if !accepted {
                        let _ = child.kill();
                        process_done.store(true, Ordering::Relaxed);
                        let msg = message.unwrap_or_else(|| "unknown".to_string());
                        return Err(PluginError::PluginFailed(format!(
                            "plugin rejected call (code={code}): {msg}"
                        )));
                    }
                }
                PluginFrame::Drop { .. } => {
                    // ignore
                }
                PluginFrame::Error { message, code: _ } => {
                    let _ = child.kill();
                    process_done.store(true, Ordering::Relaxed);
                    return Err(PluginError::PluginFailed(message));
                }
                PluginFrame::End => {
                    break;
                }
                PluginFrame::Goodbye => break,
                _ => {}
            }
        }

        process_done.store(true, Ordering::Relaxed);
        let status = child.wait()?;
        if !status.success() {
            return Err(PluginError::PluginFailed(format!(
                "plugin process exited with error status: {status}"
            )));
        }
        if timed_out.load(Ordering::Relaxed) {
            let _ = child.kill();
            return Err(PluginError::Protocol(format!(
                "plugin call timeout after {timeout_ms}ms"
            )));
        }

        // If exactly one value, return Value, else ListStream
        if results.len() == 1 {
            Ok(CtPipelineData::Value(
                results.pop().unwrap(),
                ctpipeline::metadata::CtPipelineMetadata::default(),
            ))
        } else {
            // Need to convert Vec<CtValue> to ListStream
            let iter = results.into_iter();
            let stream = CtListStream::new(iter, CtPipelineMetadata::default());
            Ok(CtPipelineData::ListStream(stream))
        }
    }
}

fn send_input_frames(
    stdin: &mut std::process::ChildStdin,
    input: CtPipelineData,
) -> Result<(), PluginError> {
    match input {
        CtPipelineData::Empty => {}
        CtPipelineData::Value(value, _) => {
            let frame = HostFrame::Data { value };
            writeln!(stdin, "{}", serde_json::to_string(&frame)?)?;
        }
        CtPipelineData::ListStream(stream) => {
            for value in stream {
                let frame = HostFrame::Data { value };
                writeln!(stdin, "{}", serde_json::to_string(&frame)?)?;
            }
        }
        CtPipelineData::ByteStream(mut stream) => {
            let mut buf = [0u8; 8192];
            loop {
                let n = stream.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                let frame = HostFrame::Data {
                    value: ctpipeline::CtValue::Binary(buf[..n].to_vec()),
                };
                writeln!(stdin, "{}", serde_json::to_string(&frame)?)?;
            }
        }
    }
    Ok(())
}
