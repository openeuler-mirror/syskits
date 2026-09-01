use ctcore::ct_error::{CTError, CTResult};
use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;
use std::io::{self, Write};

#[derive(Default)]
pub struct CmdDu;

struct DuIntent {
    argv: Vec<OsString>,
}

struct DuCore;

impl DuIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("du"));

        for arg in &call.positionals {
            argv.push(OsString::from(value_to_arg(&arg.value)));
        }

        Ok(Self { argv })
    }
}

fn value_to_arg(value: &CtValue) -> String {
    match value {
        CtValue::String(s) => s.clone(),
        other => other.to_text(),
    }
}

impl DuCore {
    fn run_core(
        intent: &DuIntent,
        input: CtPipelineData,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let result = run_du_with_optional_files0_stdin(intent, input, argv_uses_files0_from_stdin)?;
        Ok(match result {
            Ok(semantic) => (
                semantic_to_value(&semantic),
                semantic.classic_text,
                semantic.stderr_text,
                semantic.exit_code,
            ),
            Err(err) => (
                CtValue::List(Vec::new()),
                String::new(),
                render_error_text(err.as_ref()),
                err.code(),
            ),
        })
    }
}

fn run_du_with_optional_files0_stdin(
    intent: &DuIntent,
    input: CtPipelineData,
    use_stdin: impl Fn(&[OsString]) -> bool,
) -> Result<CTResult<ct_du::DuSemantic>, CtDiagnosticError> {
    if !use_stdin(&intent.argv) || matches!(input, CtPipelineData::Empty) {
        return Ok(ct_du::du_native_semantic(intent.argv.iter().cloned()));
    }

    let mut stdin_bytes = Vec::new();
    write_files0_pipeline_input(input, &mut stdin_bytes)
        .map_err(|err| CtDiagnosticError::simple(format!("du: {err}")))?;
    Ok(ctcore::ct_io::with_injected_stdin(stdin_bytes, || {
        ct_du::du_native_semantic(intent.argv.iter().cloned())
    }))
}

fn render_error_text(err: &dyn CTError) -> String {
    let mut stderr = format!("du: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'du --help' for more information.\n");
    }
    stderr
}

fn argv_uses_files0_from_stdin(argv: &[OsString]) -> bool {
    let mut index = 1;
    while index < argv.len() {
        let arg = argv[index].to_string_lossy();
        if arg == "--files0-from=-" {
            return true;
        }
        if arg == "--files0-from" {
            return argv
                .get(index + 1)
                .is_some_and(|value| value.to_string_lossy() == "-");
        }
        index += 1;
    }
    false
}

fn write_files0_pipeline_input(input: CtPipelineData, mut writer: impl Write) -> io::Result<()> {
    match input {
        CtPipelineData::Empty => {}
        CtPipelineData::ByteStream(mut stream) => {
            std::io::copy(&mut stream, &mut writer)?;
        }
        CtPipelineData::Value(CtValue::List(items), _) => {
            for item in items {
                write_files0_value(item, &mut writer)?;
            }
        }
        CtPipelineData::Value(value, _) => write_files0_value(value, &mut writer)?,
        CtPipelineData::ListStream(stream) => {
            for value in stream {
                write_files0_value(value, &mut writer)?;
            }
        }
    }
    writer.flush()
}

fn write_files0_value(value: CtValue, writer: &mut impl Write) -> io::Result<()> {
    let text = value.to_text();
    writer.write_all(text.as_bytes())?;
    if !text.as_bytes().ends_with(&[0]) {
        writer.write_all(&[0])?;
    }
    Ok(())
}

fn row_to_value(row: &ct_du::DuRow) -> CtValue {
    let mut fields = vec![
        ("kind".into(), CtValue::String(row.kind.clone())),
        (
            "depth".into(),
            CtValue::Int(i64::try_from(row.depth).expect("depth fits in i64")),
        ),
        ("is_dir".into(), CtValue::Bool(row.is_dir)),
        (
            "display_size".into(),
            CtValue::String(row.display_size.clone()),
        ),
        (
            "measured_size".into(),
            CtValue::Int(i64::try_from(row.measured_size).expect("measured size fits in i64")),
        ),
        (
            "apparent_size_bytes".into(),
            CtValue::Int(
                i64::try_from(row.apparent_size_bytes).expect("apparent size fits in i64"),
            ),
        ),
        (
            "allocated_bytes".into(),
            CtValue::Int(i64::try_from(row.allocated_bytes).expect("allocated bytes fit in i64")),
        ),
        (
            "inodes".into(),
            CtValue::Int(i64::try_from(row.inodes).expect("inodes fit in i64")),
        ),
        (
            "time_seconds".into(),
            match row.time_seconds {
                Some(value) => CtValue::Int(i64::try_from(value).expect("time fits in i64")),
                None => CtValue::Nothing,
            },
        ),
        (
            "time_display".into(),
            match &row.time_display {
                Some(value) => CtValue::String(value.clone()),
                None => CtValue::Nothing,
            },
        ),
    ];

    fields.push((
        "path".into(),
        match &row.path {
            Some(value) => CtValue::String(value.clone()),
            None => CtValue::Nothing,
        },
    ));
    fields.push((
        "label".into(),
        match &row.label {
            Some(value) => CtValue::String(value.clone()),
            None => CtValue::Nothing,
        },
    ));

    CtValue::Record(fields)
}

fn semantic_to_value(semantic: &ct_du::DuSemantic) -> CtValue {
    CtValue::List(semantic.rows.iter().map(row_to_value).collect())
}

fn display_columns() -> CtValue {
    CtValue::List(
        ["display_size", "path", "kind", "depth"]
            .into_iter()
            .map(|column| CtValue::String(column.into()))
            .collect(),
    )
}

impl DataCommand for CmdDu {
    fn signature(&self) -> DataSignature {
        DataSignature::new("du", "structured disk usage output")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible du arguments",
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
        let intent = DuIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = DuCore::run_core(&intent, input)?;
        let metadata = CtPipelineMetadata {
            classic_text: Some(classic_text),
            classic_bytes: None,
            classic_append_newline: false,
            stderr_text: Some(stderr_text),
            exit_code,
            source: Some("du".into()),
            ..Default::default()
        };
        if let Ok(mut custom) = metadata.custom.lock() {
            custom.insert("display.columns".into(), display_columns());
        }
        Ok(CtPipelineData::Value(value, metadata))
    }
}

#[cfg(test)]
mod tests {
    use super::{DuIntent, display_columns, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_positionals() {
        let call = DataCall {
            positionals: vec![
                BoundArg::new(CtValue::String("-a".into()), None),
                BoundArg::new(CtValue::String("/tmp".into()), None),
            ],
            ..DataCall::named("du")
        };

        let intent = DuIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("du"),
                OsString::from("-a"),
                OsString::from("/tmp")
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_du::DuSemantic {
            rows: vec![ct_du::DuRow {
                kind: "entry".into(),
                path: Some("/tmp/a".into()),
                label: None,
                depth: 1,
                is_dir: true,
                display_size: "4".into(),
                measured_size: 4,
                apparent_size_bytes: 0,
                allocated_bytes: 4096,
                inodes: 1,
                time_seconds: None,
                time_display: None,
            }],
            classic_text: "4\t/tmp/a\n".into(),
            stderr_text: String::new(),
            exit_code: 0,
        });

        assert_eq!(
            value,
            CtValue::List(vec![CtValue::Record(vec![
                ("kind".into(), CtValue::String("entry".into())),
                ("depth".into(), CtValue::Int(1)),
                ("is_dir".into(), CtValue::Bool(true)),
                ("display_size".into(), CtValue::String("4".into())),
                ("measured_size".into(), CtValue::Int(4)),
                ("apparent_size_bytes".into(), CtValue::Int(0)),
                ("allocated_bytes".into(), CtValue::Int(4096)),
                ("inodes".into(), CtValue::Int(1)),
                ("time_seconds".into(), CtValue::Nothing),
                ("time_display".into(), CtValue::Nothing),
                ("path".into(), CtValue::String("/tmp/a".into())),
                ("label".into(), CtValue::Nothing),
            ])])
        );
    }

    #[test]
    fn display_columns_focus_on_disk_usage_result() {
        assert_eq!(
            display_columns(),
            CtValue::List(vec![
                CtValue::String("display_size".into()),
                CtValue::String("path".into()),
                CtValue::String("kind".into()),
                CtValue::String("depth".into()),
            ])
        );
    }
}
