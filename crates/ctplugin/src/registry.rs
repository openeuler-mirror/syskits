use crate::error::PluginError;
use crate::util::{
    protocol_max_frame_bytes, protocol_timeout_ms, read_frame_line, spawn_timeout_killer,
};
use ctengine::command::DataCommand;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone)]
pub struct PluginDescriptor {
    pub path: PathBuf,
    pub protocol: String,
    pub commands: Vec<String>,
}

#[derive(Debug)]
pub struct PluginRegistry {
    pub plugins: Vec<PluginDescriptor>,
}

impl ctengine::context::PluginProvider for PluginRegistry {
    fn get_command(&self, name: &str) -> Option<Box<dyn DataCommand>> {
        self.get_command(name)
    }
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    /// Scan directory for plugins
    pub fn discover() -> Self {
        let mut registry = Self::new();
        if let Ok(paths) = std::env::var("SYSKITS_PLUGIN_PATH") {
            for dir in std::env::split_paths(&paths) {
                // Read dir
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if is_executable_plugin_candidate(&path) {
                            match Self::handshake(&path) {
                                Ok(desc) => {
                                    registry.plugins.push(desc);
                                }
                                Err(e) => {
                                    eprintln!(
                                        "syskits plugin warning: failed to load {}: {}",
                                        path.display(),
                                        e
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        registry
    }

    fn handshake(path: &PathBuf) -> Result<PluginDescriptor, PluginError> {
        use crate::proto::{HostFrame, PROTOCOL_VERSION, PluginFrame};
        use std::io::{BufReader, Write};
        use std::process::{Command, Stdio};
        let timeout_ms = protocol_timeout_ms();
        let max_frame_bytes = protocol_max_frame_bytes();

        let mut child = Command::new(path)
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

        // Send Hello
        let req = HostFrame::Hello {
            protocol: PROTOCOL_VERSION.to_string(),
        };
        writeln!(stdin, "{}", serde_json::to_string(&req)?)?;

        // Read Hello
        let line = match read_frame_line(&mut stdout, max_frame_bytes)? {
            Some(line) => line,
            None => {
                let _ = child.kill();
                process_done.store(true, Ordering::Relaxed);
                if timed_out.load(Ordering::Relaxed) {
                    return Err(PluginError::Protocol(format!(
                        "plugin handshake timeout after {timeout_ms}ms"
                    )));
                }
                return Err(PluginError::Protocol(
                    "plugin closed stdout before Hello".into(),
                ));
            }
        };
        let resp: PluginFrame = serde_json::from_str(&line).map_err(|e| {
            let _ = child.kill();
            process_done.store(true, Ordering::Relaxed);
            PluginError::Serialization(e)
        })?;

        let (protocol, commands_from_hello) = match resp {
            PluginFrame::Hello { protocol, commands } => {
                if protocol != PROTOCOL_VERSION {
                    let _ = child.kill();
                    process_done.store(true, Ordering::Relaxed);
                    return Err(PluginError::Protocol(format!(
                        "protocol mismatch: expected {PROTOCOL_VERSION}, got {protocol}"
                    )));
                }
                (
                    protocol,
                    if commands.is_empty() {
                        None
                    } else {
                        Some(commands)
                    },
                )
            }
            _ => {
                let _ = child.kill();
                process_done.store(true, Ordering::Relaxed);
                return Err(PluginError::Protocol(format!(
                    "expected Hello frame, got {resp:?}"
                )));
            }
        };

        let commands = if let Some(commands) = commands_from_hello {
            commands
        } else {
            let req = HostFrame::Signature;
            writeln!(stdin, "{}", serde_json::to_string(&req)?)?;
            let line = match read_frame_line(&mut stdout, max_frame_bytes)? {
                Some(line) => line,
                None => {
                    let _ = child.kill();
                    process_done.store(true, Ordering::Relaxed);
                    return Err(PluginError::Protocol(
                        "plugin closed stdout before Signature".into(),
                    ));
                }
            };
            let sig: PluginFrame = serde_json::from_str(&line).map_err(|e| {
                let _ = child.kill();
                process_done.store(true, Ordering::Relaxed);
                PluginError::Serialization(e)
            })?;
            match sig {
                PluginFrame::Signature { commands } => commands,
                PluginFrame::Hello { commands, .. } if !commands.is_empty() => commands,
                other => {
                    let _ = child.kill();
                    process_done.store(true, Ordering::Relaxed);
                    return Err(PluginError::Protocol(format!(
                        "expected Signature frame, got {other:?}"
                    )));
                }
            }
        };

        let desc = PluginDescriptor {
            path: path.clone(),
            protocol,
            commands,
        };

        process_done.store(true, Ordering::Relaxed);
        let _ = child.kill();
        let _ = child.wait();

        Ok(desc)
    }

    pub fn get_command(&self, name: &str) -> Option<Box<dyn DataCommand>> {
        for p in &self.plugins {
            if p.commands.contains(&name.to_string()) {
                let runner = crate::runner::PluginHostRunner::new(p.path.clone());
                return Some(Box::new(PluginCommand {
                    runner,
                    name: name.to_string(),
                }));
            }
        }
        None
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn is_executable_plugin_candidate(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|metadata| (metadata.permissions().mode() & 0o111) != 0)
            .unwrap_or(false)
    }

    #[cfg(not(unix))]
    {
        true
    }
}

pub struct PluginCommand {
    runner: crate::runner::PluginHostRunner,
    name: String,
}

static PLUGIN_CMD_NAME_CACHE: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();

fn intern_plugin_command_name(name: &str) -> &'static str {
    let cache = PLUGIN_CMD_NAME_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = cache.lock().expect("plugin command name cache poisoned");
    if let Some(existing) = guard.get(name) {
        return existing;
    }
    // DataSignature requires &'static str; intern each unique plugin command name once.
    let leaked = Box::leak(name.to_string().into_boxed_str());
    guard.insert(name.to_string(), leaked);
    leaked
}

impl DataCommand for PluginCommand {
    fn signature(&self) -> ctsig::DataSignature {
        let name_static = intern_plugin_command_name(&self.name);
        ctsig::DataSignature::new(name_static, "Plugin command")
    }

    fn run(
        &self,
        call: &ctsig::DataCall,
        input: ctpipeline::pipeline_data::CtPipelineData,
        ctx: &ctengine::context::DataEngineContext,
    ) -> Result<ctpipeline::pipeline_data::CtPipelineData, ctengine::error::CtDiagnosticError> {
        self.runner
            .call(&self.name, call, input, ctx)
            .map_err(|e| ctengine::error::CtDiagnosticError::simple(e.to_string()))
    }
}
