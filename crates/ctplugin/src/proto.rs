use ctpipeline::value::CtValue;
use ctsig::DataCall;
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: &str = "1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum HostFrame {
    Hello { protocol: String },
    Signature,
    Run { name: String, args: DataCall },
    // Backward compatible alias for legacy plugins.
    Call { name: String, args: DataCall },
    Data { value: CtValue },
    End,
    Goodbye,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PluginFrame {
    Hello {
        protocol: String,
        commands: Vec<String>,
    },
    Signature {
        commands: Vec<String>,
    },
    CallResponse {
        accepted: bool,
        message: Option<String>,
        code: i32,
    },
    Data {
        value: CtValue,
    },
    Drop {
        reason: String,
    },
    End,
    Ack,
    Goodbye,
    Error {
        message: String,
        code: i32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctpipeline::value::CtValue;

    #[test]
    fn test_host_frame_hello() {
        let frame = HostFrame::Hello {
            protocol: "1".to_string(),
        };
        let js = serde_json::to_string(&frame).unwrap();
        assert_eq!(js, r#"{"type":"Hello","protocol":"1"}"#);
    }

    #[test]
    fn test_plugin_frame_hello() {
        let frame = PluginFrame::Hello {
            protocol: "1".to_string(),
            commands: vec!["my-cmd".to_string()],
        };
        let js = serde_json::to_string(&frame).unwrap();
        assert_eq!(
            js,
            r#"{"type":"Hello","protocol":"1","commands":["my-cmd"]}"#
        );
    }

    #[test]
    fn test_plugin_frame_data_numeric_array_deserializes_as_list() {
        let frame: PluginFrame =
            serde_json::from_str(r#"{"type":"Data","value":[1,2,3]}"#).unwrap();
        let PluginFrame::Data { value } = frame else {
            panic!("expected Data frame");
        };
        let CtValue::List(items) = value else {
            panic!("expected List value");
        };
        assert_eq!(items.len(), 3);
        assert!(matches!(items[0], CtValue::Int(1)));
        assert!(matches!(items[1], CtValue::Int(2)));
        assert!(matches!(items[2], CtValue::Int(3)));
    }
}
