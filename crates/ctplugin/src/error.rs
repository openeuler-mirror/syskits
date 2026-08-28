// error.rs for ctplugin
use thiserror::Error;

#[derive(Error, Debug)]
pub enum PluginError {
    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("plugin failed: {0}")]
    PluginFailed(String),
}
