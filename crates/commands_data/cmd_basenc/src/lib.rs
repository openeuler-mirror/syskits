use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtFlag, CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdBasenc;

struct BasencIntent {
    argv: Vec<OsString>,
}

struct BasencCore;

impl BasencIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + call.flags.len() + 1);
        argv.push(OsString::from("basenc"));

        push_switch(&mut argv, call, "decode", "d", "--decode");
        push_switch(&mut argv, call, "ignore-garbage", "i", "--ignore-garbage");
        push_opt(&mut argv, call, "wrap", "w", "--wrap");

        push_long_switch(&mut argv, call, "base64");
        push_long_switch(&mut argv, call, "base64url");
        push_long_switch(&mut argv, call, "base32");
        push_long_switch(&mut argv, call, "base32hex");
        push_long_switch(&mut argv, call, "base16");
        push_long_switch(&mut argv, call, "base2lsbf");
        push_long_switch(&mut argv, call, "base2msbf");
        push_long_switch(&mut argv, call, "z85");

        for arg in &call.positionals {
            argv.push(OsString::from(value_to_arg(&arg.value)));
        }

        Ok(Self { argv })
    }
}

impl BasencCore {
    fn run_core(
        intent: &BasencIntent,
        input: CtPipelineData,
    ) -> Result<(CtValue, String, Vec<u8>, String, i32), CtDiagnosticError> {
        let semantic = ctengine::run_with_optional_pipeline_stdin(
            "basenc",
            input,
            argv_uses_stdin(&intent.argv, &["-w", "--wrap"]),
            || {
                ct_basenc::basenc_native_semantic(intent.argv.iter().cloned())
                    .map_err(|err| CtDiagnosticError::simple(err.to_string()).with_code(err.code()))
            },
            |err| CtDiagnosticError::simple(format!("basenc: {err}")),
        )?;
        Ok((
            semantic_to_value(&semantic),
            semantic.classic_text,
            semantic.classic_bytes,
            semantic.stderr_text,
            semantic.exit_code,
        ))
    }
}

fn argv_uses_stdin(argv: &[OsString], value_flags: &[&str]) -> bool {
    ctengine::argv_uses_stdin(argv, value_flags)
}

fn optional_string(value: &Option<String>) -> CtValue {
    match value {
        Some(value) => CtValue::String(value.clone()),
        None => CtValue::Nothing,
    }
}

fn semantic_to_value(semantic: &ct_basenc::BasencSemantic) -> CtValue {
    CtValue::List(semantic.rows.iter().map(row_to_value).collect())
}

fn row_to_value(row: &ct_basenc::BasencSemanticRow) -> CtValue {
    CtValue::Record(vec![
        ("encoding".into(), CtValue::String(row.encoding.clone())),
        ("type".into(), CtValue::String(row.kind.clone())),
        ("mode".into(), CtValue::String(row.mode.clone())),
        (
            "wrap".into(),
            CtValue::Int(i64::try_from(row.wrap).expect("wrap fits")),
        ),
        ("ignore_garbage".into(), CtValue::Bool(row.ignore_garbage)),
        ("input".into(), CtValue::String(row.input.clone())),
        ("file".into(), optional_string(&row.file)),
        (
            "line".into(),
            CtValue::Int(i64::try_from(row.line).expect("line fits")),
        ),
        (
            "output_text".into(),
            CtValue::String(row.output_text.clone()),
        ),
        (
            "byte_len".into(),
            CtValue::Int(i64::try_from(row.byte_len).expect("byte len fits")),
        ),
    ])
}

fn value_to_arg(value: &CtValue) -> String {
    match value {
        CtValue::String(s) => s.clone(),
        other => other.to_text(),
    }
}

fn push_long_switch(argv: &mut Vec<OsString>, call: &DataCall, long: &str) {
    if call.has_flag(long) {
        argv.push(OsString::from(format!("--{long}")));
    }
}

fn push_switch(argv: &mut Vec<OsString>, call: &DataCall, long: &str, short: &str, cli_long: &str) {
    if call.has_flag(long) || call.has_flag(short) {
        argv.push(OsString::from(cli_long));
    }
}

fn get_flag_value(call: &DataCall, long: &str, short: &str) -> Option<String> {
    [long, short]
        .iter()
        .find_map(|key| call.flags.get(*key))
        .and_then(|arg| arg.as_ref())
        .map(|arg| value_to_arg(&arg.value))
}

fn push_opt(argv: &mut Vec<OsString>, call: &DataCall, long: &str, short: &str, cli_long: &str) {
    if let Some(value) = get_flag_value(call, long, short) {
        argv.push(OsString::from(cli_long));
        argv.push(OsString::from(value));
    }
}

impl DataCommand for CmdBasenc {
    fn signature(&self) -> DataSignature {
        DataSignature::new("basenc", "structured basenc rows")
            .flag(CtFlag::switch("decode", Some('d'), "decode data"))
            .flag(CtFlag::switch(
                "ignore-garbage",
                Some('i'),
                "when decoding, ignore non-alphabetic chars",
            ))
            .flag(CtFlag::with_value(
                "wrap",
                Some('w'),
                "wrap encoded lines after COLS",
                CtType::String,
            ))
            .flag(CtFlag::switch("base64", None, "same as base64 program"))
            .flag(CtFlag::switch(
                "base64url",
                None,
                "file- and url-safe base64",
            ))
            .flag(CtFlag::switch("base32", None, "same as base32 program"))
            .flag(CtFlag::switch(
                "base32hex",
                None,
                "extended hex alphabet base32",
            ))
            .flag(CtFlag::switch("base16", None, "hex encoding"))
            .flag(CtFlag::switch(
                "base2lsbf",
                None,
                "bit string with least significant bit first",
            ))
            .flag(CtFlag::switch(
                "base2msbf",
                None,
                "bit string with most significant bit first",
            ))
            .flag(CtFlag::switch("z85", None, "ascii85-like encoding for ZMQ"))
            .rest(CtPositionalArg::optional(
                "file",
                "input file, default stdin",
                CtType::Any,
            ))
            .input(CtType::Any)
            .output(CtType::List)
            .allow_unknown_args(true)
    }

    fn run(
        &self,
        call: &DataCall,
        input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let intent = BasencIntent::from_call(call)?;
        let (value, classic_text, classic_bytes, stderr_text, exit_code) =
            BasencCore::run_core(&intent, input)?;
        Ok(CtPipelineData::Value(
            value,
            CtPipelineMetadata {
                classic_text: Some(classic_text),
                classic_bytes: Some(classic_bytes),
                classic_append_newline: false,
                stderr_text: if stderr_text.is_empty() {
                    None
                } else {
                    Some(stderr_text)
                },
                exit_code,
                source: Some("basenc".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{BasencIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_flags_and_positionals() {
        let mut call = DataCall::named("basenc");
        call.flags.insert("decode".into(), None);
        call.flags.insert("base64".into(), None);
        call.flags.insert(
            "wrap".into(),
            Some(BoundArg::new(CtValue::String("12".into()), None)),
        );
        call.positionals
            .push(BoundArg::new(CtValue::String("sample.txt".into()), None));

        let intent = BasencIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("basenc"),
                OsString::from("--decode"),
                OsString::from("--wrap"),
                OsString::from("12"),
                OsString::from("--base64"),
                OsString::from("sample.txt"),
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_basenc::BasencSemantic {
            rows: vec![ct_basenc::BasencSemanticRow {
                encoding: "base64".into(),
                kind: "text".into(),
                mode: "encode".into(),
                wrap: 76,
                ignore_garbage: false,
                input: "file".into(),
                file: Some("sample.txt".into()),
                line: 1,
                output_text: "YWJj".into(),
                byte_len: 4,
            }],
            classic_text: "YWJj\n".into(),
            classic_bytes: b"YWJj\n".to_vec(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        let CtValue::List(items) = value else {
            panic!("expected list value");
        };
        assert_eq!(items.len(), 1);
        let CtValue::Record(fields) = &items[0] else {
            panic!("expected record row");
        };
        assert!(matches!(field(fields, "encoding"), CtValue::String(s) if s == "base64"));
        assert!(matches!(field(fields, "type"), CtValue::String(s) if s == "text"));
        assert!(matches!(field(fields, "mode"), CtValue::String(s) if s == "encode"));
        assert!(matches!(field(fields, "wrap"), CtValue::Int(76)));
        assert!(matches!(
            field(fields, "ignore_garbage"),
            CtValue::Bool(false)
        ));
        assert!(matches!(field(fields, "input"), CtValue::String(s) if s == "file"));
        assert!(matches!(field(fields, "file"), CtValue::String(s) if s == "sample.txt"));
        assert!(matches!(field(fields, "line"), CtValue::Int(1)));
        assert!(matches!(field(fields, "output_text"), CtValue::String(s) if s == "YWJj"));
        assert!(matches!(field(fields, "byte_len"), CtValue::Int(4)));
    }

    fn field<'a>(fields: &'a [(String, CtValue)], name: &str) -> &'a CtValue {
        fields
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
            .unwrap_or_else(|| panic!("missing field {name}"))
    }
}
