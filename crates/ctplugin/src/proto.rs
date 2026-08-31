use ctpipeline::CtSpan;
use ctpipeline::value::{CtValue, CtValueError};
use ctsig::{BoundArg, DataCall};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const PROTOCOL_VERSION: &str = "2";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginRecordField {
    pub key: String,
    pub value: PluginValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum PluginValue {
    Nothing,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    List(Vec<PluginValue>),
    Binary(Vec<u8>),
    DateTime(i128),
    Duration(i64),
    Size(u64),
    Record(Vec<PluginRecordField>),
    Error(String),
}

impl From<CtValue> for PluginValue {
    fn from(value: CtValue) -> Self {
        match value {
            CtValue::Nothing => PluginValue::Nothing,
            CtValue::Bool(value) => PluginValue::Bool(value),
            CtValue::Int(value) => PluginValue::Int(value),
            CtValue::Float(value) => PluginValue::Float(value),
            CtValue::String(value) => PluginValue::String(value),
            CtValue::List(values) => {
                PluginValue::List(values.into_iter().map(PluginValue::from).collect())
            }
            CtValue::Binary(value) => PluginValue::Binary(value),
            CtValue::DateTime(value) => PluginValue::DateTime(value),
            CtValue::Duration(value) => PluginValue::Duration(value),
            CtValue::Size(value) => PluginValue::Size(value),
            CtValue::Record(fields) => PluginValue::Record(
                fields
                    .into_iter()
                    .map(|(key, value)| PluginRecordField {
                        key,
                        value: PluginValue::from(value),
                    })
                    .collect(),
            ),
            CtValue::Error(err) => PluginValue::Error(err.to_string()),
        }
    }
}

impl From<PluginValue> for CtValue {
    fn from(value: PluginValue) -> Self {
        match value {
            PluginValue::Nothing => CtValue::Nothing,
            PluginValue::Bool(value) => CtValue::Bool(value),
            PluginValue::Int(value) => CtValue::Int(value),
            PluginValue::Float(value) => CtValue::Float(value),
            PluginValue::String(value) => CtValue::String(value),
            PluginValue::List(values) => {
                CtValue::List(values.into_iter().map(CtValue::from).collect())
            }
            PluginValue::Binary(value) => CtValue::Binary(value),
            PluginValue::DateTime(value) => CtValue::DateTime(value),
            PluginValue::Duration(value) => CtValue::Duration(value),
            PluginValue::Size(value) => CtValue::Size(value),
            PluginValue::Record(fields) => CtValue::Record(
                fields
                    .into_iter()
                    .map(|field| (field.key, CtValue::from(field.value)))
                    .collect(),
            ),
            PluginValue::Error(message) => CtValue::Error(Box::new(CtValueError::custom(message))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginBoundArg {
    pub value: PluginValue,
    pub span: Option<CtSpan>,
}

impl From<BoundArg> for PluginBoundArg {
    fn from(arg: BoundArg) -> Self {
        Self {
            value: PluginValue::from(arg.value),
            span: arg.span,
        }
    }
}

impl From<PluginBoundArg> for BoundArg {
    fn from(arg: PluginBoundArg) -> Self {
        BoundArg::new(CtValue::from(arg.value), arg.span)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginDataCall {
    pub head: Option<CtSpan>,
    pub command_name: String,
    pub positionals: Vec<PluginBoundArg>,
    pub flags: HashMap<String, Option<PluginBoundArg>>,
    pub rest: Vec<PluginBoundArg>,
}

impl From<DataCall> for PluginDataCall {
    fn from(call: DataCall) -> Self {
        Self {
            head: call.head,
            command_name: call.command_name,
            positionals: call
                .positionals
                .into_iter()
                .map(PluginBoundArg::from)
                .collect(),
            flags: call
                .flags
                .into_iter()
                .map(|(name, arg)| (name, arg.map(PluginBoundArg::from)))
                .collect(),
            rest: call.rest.into_iter().map(PluginBoundArg::from).collect(),
        }
    }
}

impl From<PluginDataCall> for DataCall {
    fn from(call: PluginDataCall) -> Self {
        Self {
            head: call.head,
            command_name: call.command_name,
            positionals: call.positionals.into_iter().map(BoundArg::from).collect(),
            flags: call
                .flags
                .into_iter()
                .map(|(name, arg)| (name, arg.map(BoundArg::from)))
                .collect(),
            rest: call.rest.into_iter().map(BoundArg::from).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum HostFrame {
    Hello { protocol: String },
    Signature,
    Run { name: String, args: PluginDataCall },
    // Backward compatible alias for legacy plugins.
    Call { name: String, args: PluginDataCall },
    Data { value: PluginValue },
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
        value: PluginValue,
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
    use ctsig::{BoundArg, DataCall};

    #[test]
    fn test_host_frame_hello() {
        let frame = HostFrame::Hello {
            protocol: PROTOCOL_VERSION.to_string(),
        };
        let js = serde_json::to_string(&frame).unwrap();
        assert_eq!(
            js,
            format!(r#"{{"type":"Hello","protocol":"{PROTOCOL_VERSION}"}}"#)
        );
    }

    #[test]
    fn test_plugin_frame_hello() {
        let frame = PluginFrame::Hello {
            protocol: PROTOCOL_VERSION.to_string(),
            commands: vec!["my-cmd".to_string()],
        };
        let js = serde_json::to_string(&frame).unwrap();
        assert_eq!(
            js,
            format!(r#"{{"type":"Hello","protocol":"{PROTOCOL_VERSION}","commands":["my-cmd"]}}"#)
        );
    }

    #[test]
    fn test_plugin_frame_data_numeric_array_deserializes_as_list() {
        let frame = PluginFrame::Data {
            value: PluginValue::from(CtValue::List(vec![
                CtValue::Int(1),
                CtValue::Int(2),
                CtValue::Int(3),
            ])),
        };
        let js = serde_json::to_string(&frame).unwrap();
        assert!(
            js.contains(r#""kind":"list""#),
            "list value must be tagged: {js}"
        );

        let frame: PluginFrame = serde_json::from_str(&js).unwrap();
        let PluginFrame::Data { value } = frame else {
            panic!("expected Data frame");
        };
        let CtValue::List(items) = CtValue::from(value) else {
            panic!("expected List value");
        };
        assert_eq!(items.len(), 3);
        assert!(matches!(items[0], CtValue::Int(1)));
        assert!(matches!(items[1], CtValue::Int(2)));
        assert!(matches!(items[2], CtValue::Int(3)));
    }

    #[test]
    fn test_plugin_frame_binary_value_uses_explicit_type_tag() {
        let frame = PluginFrame::Data {
            value: PluginValue::from(CtValue::Binary(vec![1, 2, 3])),
        };

        let js = serde_json::to_string(&frame).unwrap();
        assert!(
            js.contains(r#""kind":"binary""#),
            "binary plugin values must be explicitly tagged: {js}"
        );

        let parsed: PluginFrame = serde_json::from_str(&js).unwrap();
        let PluginFrame::Data { value } = parsed else {
            panic!("expected Data frame");
        };
        assert_eq!(CtValue::from(value), CtValue::Binary(vec![1, 2, 3]));
    }

    #[test]
    fn test_host_run_arguments_use_explicit_value_tags() {
        let mut call = DataCall::named("plugin-echo");
        call.positionals
            .push(BoundArg::new(CtValue::Binary(vec![9, 8, 7]), None));
        let frame = HostFrame::Run {
            name: "plugin-echo".to_string(),
            args: PluginDataCall::from(call),
        };

        let js = serde_json::to_string(&frame).unwrap();
        assert!(
            js.contains(r#""kind":"binary""#),
            "DataCall arguments crossing plugin boundary must be explicitly tagged: {js}"
        );

        let parsed: HostFrame = serde_json::from_str(&js).unwrap();
        let HostFrame::Run { args, .. } = parsed else {
            panic!("expected Run frame");
        };
        let roundtrip = DataCall::from(args);
        assert!(matches!(
            roundtrip.positionals.first().map(|arg| &arg.value),
            Some(CtValue::Binary(bytes)) if bytes == &[9, 8, 7]
        ));
    }
}
