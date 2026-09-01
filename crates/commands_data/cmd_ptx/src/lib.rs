/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 */

use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext, OutputFormat};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtFlag, CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;
use std::io::Read;

#[derive(Default)]
pub struct CmdPtx;

struct PtxIntent {
    argv: Vec<OsString>,
    uses_stdin: bool,
}

struct PtxCore;

impl PtxIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + call.flags.len() + 1);
        argv.push(OsString::from("ptx"));

        push_switch(&mut argv, call, "auto-reference", "A", "-A");
        push_switch(&mut argv, call, "traditional", "G", "-G");
        push_opt(&mut argv, call, "flag-truncation", "F", "-F");
        push_opt(&mut argv, call, "macro-name", "M", "-M");
        push_switch(&mut argv, call, "format-roff", "O", "-O");
        push_switch(&mut argv, call, "right-side-refs", "R", "-R");
        push_opt(&mut argv, call, "sentence-regexp", "S", "-S");
        push_switch(&mut argv, call, "format-tex", "T", "-T");
        push_opt(&mut argv, call, "word-regexp", "W", "-W");
        push_opt(&mut argv, call, "break-file", "b", "-b");
        push_switch(&mut argv, call, "ignore-case", "f", "-f");
        push_opt(&mut argv, call, "gap-size", "g", "-g");
        push_opt(&mut argv, call, "ignore-file", "i", "-i");
        push_opt(&mut argv, call, "only-file", "o", "-o");
        push_switch(&mut argv, call, "references", "r", "-r");
        push_opt(&mut argv, call, "width", "w", "-w");
        push_unknown_flags(&mut argv, call);

        let mut uses_stdin = call.positionals.is_empty();
        for arg in &call.positionals {
            let arg = value_to_arg(&arg.value);
            if arg == "-" {
                uses_stdin = true;
            }
            argv.push(OsString::from(arg));
        }

        Ok(Self { argv, uses_stdin })
    }
}

impl PtxCore {
    fn run_core(
        intent: &PtxIntent,
        input: CtPipelineData,
        output_format: OutputFormat,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let semantic = if output_format == OutputFormat::Classic {
            let stdin_bytes = if intent.uses_stdin {
                Some(pipeline_or_process_stdin_to_bytes(input)?)
            } else {
                None
            };
            ct_ptx::ptx_native_semantic_with_stdin(intent.argv.iter().cloned(), stdin_bytes)
                .map_err(|err| CtDiagnosticError::simple(err.to_string()).with_code(err.code()))?
        } else {
            ctengine::run_with_optional_pipeline_stdin(
                "ptx",
                input,
                intent.uses_stdin,
                || {
                    ct_ptx::ptx_native_semantic_rows_only(intent.argv.iter().cloned()).map_err(
                        |err| CtDiagnosticError::simple(err.to_string()).with_code(err.code()),
                    )
                },
                |err| CtDiagnosticError::simple(format!("ptx: {err}")),
            )?
        };

        Ok((
            semantic_to_value(&semantic),
            semantic.classic_text,
            semantic.stderr_text,
            semantic.exit_code,
        ))
    }
}

fn pipeline_or_process_stdin_to_bytes(input: CtPipelineData) -> Result<Vec<u8>, CtDiagnosticError> {
    if let CtPipelineData::Empty = input {
        let mut bytes = Vec::new();
        std::io::stdin()
            .read_to_end(&mut bytes)
            .map_err(|err| CtDiagnosticError::simple(format!("ptx: stdin read error: {err}")))?;
        return Ok(bytes);
    }

    let mut bytes = Vec::new();
    ctengine::write_pipeline_as_classic_stdin(input, &mut bytes)
        .map_err(|err| CtDiagnosticError::simple(format!("ptx: {err}")))?;
    Ok(bytes)
}

fn row_to_value(row: &ct_ptx::PtxSemanticRow) -> CtValue {
    CtValue::Record(vec![
        (
            "row_index".into(),
            CtValue::Int(i64::try_from(row.row_index).expect("row index fits")),
        ),
        ("keyword".into(), CtValue::String(row.keyword.clone())),
        ("before".into(), CtValue::String(row.before.clone())),
        ("after".into(), CtValue::String(row.after.clone())),
        ("head".into(), CtValue::String(row.head.clone())),
        ("tail".into(), CtValue::String(row.tail.clone())),
        ("reference".into(), CtValue::String(row.reference.clone())),
        ("file".into(), CtValue::String(row.file.clone())),
        (
            "line_index".into(),
            CtValue::Int(i64::try_from(row.line_index).expect("line index fits")),
        ),
        (
            "global_line_index".into(),
            CtValue::Int(i64::try_from(row.global_line_index).expect("global line index fits")),
        ),
        (
            "rendered_text".into(),
            CtValue::String(row.rendered_text.clone()),
        ),
        ("format".into(), CtValue::String(row.format.clone())),
    ])
}

fn semantic_to_value(semantic: &ct_ptx::PtxSemantic) -> CtValue {
    CtValue::List(semantic.rows.iter().map(row_to_value).collect())
}

fn value_to_arg(value: &CtValue) -> String {
    match value {
        CtValue::String(s) => s.clone(),
        other => other.to_text(),
    }
}

fn get_flag_value(call: &DataCall, long: &str, short: &str) -> Option<String> {
    [long, short]
        .iter()
        .find_map(|key| call.flags.get(*key))
        .and_then(|arg| arg.as_ref())
        .map(|arg| value_to_arg(&arg.value))
}

fn push_opt(argv: &mut Vec<OsString>, call: &DataCall, long: &str, short: &str, opt: &str) {
    if let Some(value) = get_flag_value(call, long, short) {
        argv.push(OsString::from(opt));
        argv.push(OsString::from(value));
    }
}

fn push_switch(argv: &mut Vec<OsString>, call: &DataCall, long: &str, short: &str, opt: &str) {
    if call.has_flag(long) || call.has_flag(short) {
        argv.push(OsString::from(opt));
    }
}

fn push_unknown_flags(argv: &mut Vec<OsString>, call: &DataCall) {
    for (name, value) in &call.flags {
        if KNOWN_FLAGS.contains(&name.as_str()) {
            continue;
        }
        let prefix = if name.chars().count() == 1 { "-" } else { "--" };
        argv.push(OsString::from(format!("{prefix}{name}")));
        if let Some(value) = value {
            argv.push(OsString::from(value_to_arg(&value.value)));
        }
    }
}

const KNOWN_FLAGS: &[&str] = &[
    "auto-reference",
    "A",
    "traditional",
    "G",
    "flag-truncation",
    "F",
    "macro-name",
    "M",
    "format-roff",
    "O",
    "right-side-refs",
    "R",
    "sentence-regexp",
    "S",
    "format-tex",
    "T",
    "word-regexp",
    "W",
    "break-file",
    "b",
    "ignore-case",
    "f",
    "gap-size",
    "g",
    "ignore-file",
    "i",
    "only-file",
    "o",
    "references",
    "r",
    "width",
    "w",
];

impl DataCommand for CmdPtx {
    fn signature(&self) -> DataSignature {
        DataSignature::new("ptx", "structured ptx permuted-index rows")
            .flag(CtFlag::switch(
                "auto-reference",
                Some('A'),
                "output automatically generated references",
            ))
            .flag(CtFlag::switch(
                "traditional",
                Some('G'),
                "behave like System V ptx",
            ))
            .flag(CtFlag::with_value(
                "flag-truncation",
                Some('F'),
                "truncation marker",
                CtType::String,
            ))
            .flag(CtFlag::with_value(
                "macro-name",
                Some('M'),
                "macro name",
                CtType::String,
            ))
            .flag(CtFlag::switch(
                "format-roff",
                Some('O'),
                "generate roff directives",
            ))
            .flag(CtFlag::switch(
                "right-side-refs",
                Some('R'),
                "put references on right side",
            ))
            .flag(CtFlag::with_value(
                "sentence-regexp",
                Some('S'),
                "sentence regex",
                CtType::String,
            ))
            .flag(CtFlag::switch(
                "format-tex",
                Some('T'),
                "generate TeX directives",
            ))
            .flag(CtFlag::with_value(
                "word-regexp",
                Some('W'),
                "word regex",
                CtType::String,
            ))
            .flag(CtFlag::with_value(
                "break-file",
                Some('b'),
                "break file",
                CtType::String,
            ))
            .flag(CtFlag::switch(
                "ignore-case",
                Some('f'),
                "ignore case for sorting",
            ))
            .flag(CtFlag::with_value(
                "gap-size",
                Some('g'),
                "gap size",
                CtType::Int,
            ))
            .flag(CtFlag::with_value(
                "ignore-file",
                Some('i'),
                "ignore words file",
                CtType::String,
            ))
            .flag(CtFlag::with_value(
                "only-file",
                Some('o'),
                "only words file",
                CtType::String,
            ))
            .flag(CtFlag::switch(
                "references",
                Some('r'),
                "first field is reference",
            ))
            .flag(CtFlag::with_value(
                "width",
                Some('w'),
                "output width",
                CtType::Int,
            ))
            .rest(CtPositionalArg::optional(
                "files",
                "input files",
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
        ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let intent = PtxIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) =
            PtxCore::run_core(&intent, input, ctx.output_format)?;
        Ok(CtPipelineData::Value(
            value,
            CtPipelineMetadata {
                classic_text: Some(classic_text),
                classic_bytes: None,
                classic_append_newline: false,
                stderr_text: if stderr_text.is_empty() {
                    None
                } else {
                    Some(stderr_text)
                },
                exit_code,
                source: Some("ptx".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{PtxIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_flags_and_positionals() {
        let mut call = DataCall::named("ptx");
        call.flags.insert(
            "w".into(),
            Some(BoundArg::new(CtValue::String("30".into()), None)),
        );
        call.positionals
            .push(BoundArg::new(CtValue::String("sample.txt".into()), None));

        let intent = PtxIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("ptx"),
                OsString::from("-w"),
                OsString::from("30"),
                OsString::from("sample.txt"),
            ]
        );
    }

    #[test]
    fn from_call_preserves_unknown_flags_for_gnu_errors() {
        let mut call = DataCall::named("ptx");
        call.flags.insert("definitely-invalid".into(), None);

        let intent = PtxIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("ptx"),
                OsString::from("--definitely-invalid")
            ]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_ptx::PtxSemantic {
            rows: vec![ct_ptx::PtxSemanticRow {
                row_index: 1,
                keyword: "beta".into(),
                before: "alpha".into(),
                after: "gamma".into(),
                head: String::new(),
                tail: String::new(),
                reference: "sample.txt:1".into(),
                file: "sample.txt".into(),
                line_index: 1,
                global_line_index: 1,
                rendered_text: " alpha   beta gamma".into(),
                format: "dumb".into(),
            }],
            classic_text: " alpha   beta gamma\n".into(),
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
        assert!(matches!(field(fields, "row_index"), CtValue::Int(1)));
        assert!(matches!(field(fields, "keyword"), CtValue::String(s) if s == "beta"));
        assert!(matches!(field(fields, "before"), CtValue::String(s) if s == "alpha"));
        assert!(matches!(field(fields, "after"), CtValue::String(s) if s == "gamma"));
        assert!(matches!(field(fields, "reference"), CtValue::String(s) if s == "sample.txt:1"));
        assert!(matches!(field(fields, "file"), CtValue::String(s) if s == "sample.txt"));
        assert!(matches!(field(fields, "line_index"), CtValue::Int(1)));
        assert!(matches!(
            field(fields, "global_line_index"),
            CtValue::Int(1)
        ));
        assert!(
            matches!(field(fields, "rendered_text"), CtValue::String(s) if s == " alpha   beta gamma")
        );
        assert!(matches!(field(fields, "format"), CtValue::String(s) if s == "dumb"));
    }

    fn field<'a>(fields: &'a [(String, CtValue)], name: &str) -> &'a CtValue {
        fields
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
            .unwrap_or_else(|| panic!("missing field {name}"))
    }
}
