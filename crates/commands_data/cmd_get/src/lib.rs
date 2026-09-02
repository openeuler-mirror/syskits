/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 */

//! `cmd_get` — 从 Record 中提取字段值。

use ctengine::command::DataCommand;
use ctengine::context::DataEngineContext;
use ctengine::error::CtDiagnosticError;
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtFlag, CtPositionalArg, DataCall, DataSignature};

#[derive(Default)]
pub struct CmdGet;

const GET_HELP: &str = r#"syskits data get

This is the syskits structured data pipeline get command.
It extracts a field value from a structured Record input.

Usage:
  get <field>
  get --help
  get --version

Examples:
  whoami | get username
  from json '{"name":"alice","age":30}' | get name
"#;

impl DataCommand for CmdGet {
    fn signature(&self) -> DataSignature {
        DataSignature::new("get", "get a field value from a Record")
            .positional(CtPositionalArg::required(
                "field",
                "field name to extract",
                CtType::String,
            ))
            .flag(CtFlag::switch(
                "help",
                Some('h'),
                "show help for syskits data get",
            ))
            .flag(CtFlag::switch(
                "version",
                None,
                "show syskits data get version",
            ))
            .input(CtType::Record)
            .output(CtType::Any)
    }

    fn run(
        &self,
        call: &DataCall,
        input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        if call.has_flag("help") || call.has_flag("h") {
            return Ok(meta_text_output(GET_HELP.to_string()));
        }
        if call.has_flag("version") {
            return Ok(meta_text_output(format!(
                "syskits data get {}",
                env!("CARGO_PKG_VERSION")
            )));
        }

        let field: String = call
            .req::<String>(0)
            .map_err(|e| CtDiagnosticError::simple(format!("get: {e}")))?;
        let meta = CtPipelineMetadata::default();

        match input {
            CtPipelineData::Value(CtValue::Record(fields), _) => {
                let value = fields
                    .into_iter()
                    .find(|(k, _)| k == &field)
                    .map(|(_, v)| v)
                    .ok_or_else(|| {
                        CtDiagnosticError::simple(format!("get: field `{field}` not found"))
                    })?;
                Ok(CtPipelineData::Value(value, meta))
            }
            CtPipelineData::Value(other, _) => Err(CtDiagnosticError::simple(format!(
                "get: expected Record, got {}",
                type_name_of(&other)
            ))),
            CtPipelineData::Empty => Err(CtDiagnosticError::simple("get: empty input")),
            _ => Err(CtDiagnosticError::simple(
                "get: expected a single Record value",
            )),
        }
    }
}

fn meta_text_output(text: String) -> CtPipelineData {
    CtPipelineData::Value(
        CtValue::String(text.clone()),
        CtPipelineMetadata {
            classic_text: Some(text),
            classic_bytes: None,
            classic_append_newline: false,
            exit_code: 0,
            source: Some("get".into()),
            ..Default::default()
        },
    )
}

fn type_name_of(v: &CtValue) -> &'static str {
    match v {
        CtValue::Nothing => "Nothing",
        CtValue::Bool(_) => "Bool",
        CtValue::Int(_) => "Int",
        CtValue::Float(_) => "Float",
        CtValue::String(_) => "String",
        CtValue::Binary(_) => "Binary",
        CtValue::DateTime(_) => "DateTime",
        CtValue::Duration(_) => "Duration",
        CtValue::Size(_) => "Size",
        CtValue::Record(_) => "Record",
        CtValue::List(_) => "List",
        CtValue::Error(_) => "Error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctengine::context::{CommandRegistry, DataEngineContext};

    fn ctx() -> DataEngineContext {
        DataEngineContext::new(CommandRegistry::empty(), None, None)
    }

    fn field_call(f: &str) -> DataCall {
        let mut c = DataCall::empty();
        c.positionals
            .push(ctsig::BoundArg::new(CtValue::String(f.to_string()), None));
        c
    }
    fn flag_call(name: &str) -> DataCall {
        let mut c = DataCall::named("get");
        c.flags.insert(name.to_string(), None);
        c
    }
    fn rec(fields: Vec<(&str, CtValue)>) -> CtPipelineData {
        CtPipelineData::Value(
            CtValue::Record(
                fields
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v))
                    .collect(),
            ),
            CtPipelineMetadata::default(),
        )
    }

    #[test]
    fn test_get_string_field() {
        let r = CmdGet
            .run(
                &field_call("name"),
                rec(vec![("name", CtValue::String("Alice".into()))]),
                &ctx(),
            )
            .unwrap();
        assert!(matches!(r, CtPipelineData::Value(CtValue::String(s), _) if s == "Alice"));
    }
    #[test]
    fn test_get_int_field() {
        let r = CmdGet
            .run(
                &field_call("age"),
                rec(vec![("age", CtValue::Int(30))]),
                &ctx(),
            )
            .unwrap();
        assert!(matches!(r, CtPipelineData::Value(CtValue::Int(30), _)));
    }
    #[test]
    fn test_get_missing_field() {
        let e = CmdGet
            .run(&field_call("x"), rec(vec![("a", CtValue::Int(1))]), &ctx())
            .unwrap_err();
        assert!(e.to_string().contains("not found"));
    }
    #[test]
    fn test_get_non_record() {
        let input = CtPipelineData::Value(CtValue::Int(1), CtPipelineMetadata::default());
        let e = CmdGet.run(&field_call("f"), input, &ctx()).unwrap_err();
        assert!(e.to_string().contains("expected Record"));
    }
    #[test]
    fn test_get_empty() {
        let e = CmdGet
            .run(&field_call("f"), CtPipelineData::Empty, &ctx())
            .unwrap_err();
        assert!(e.to_string().contains("empty input"));
    }

    #[test]
    fn test_get_help_output() {
        let out = CmdGet
            .run(&flag_call("help"), CtPipelineData::Empty, &ctx())
            .expect("help should not require input");
        let CtPipelineData::Value(CtValue::String(text), meta) = out else {
            panic!("expected help text");
        };
        assert!(text.contains("syskits structured data pipeline get command"));
        assert_eq!(meta.exit_code, 0);
    }

    #[test]
    fn test_get_version_output() {
        let out = CmdGet
            .run(&flag_call("version"), CtPipelineData::Empty, &ctx())
            .expect("version should not require input");
        let CtPipelineData::Value(CtValue::String(text), meta) = out else {
            panic!("expected version text");
        };
        assert!(text.starts_with("syskits data get "));
        assert_eq!(meta.exit_code, 0);
    }
}
