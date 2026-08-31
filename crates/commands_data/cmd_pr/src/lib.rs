use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtFlag, CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdPr;

struct PrIntent {
    argv: Vec<OsString>,
    uses_stdin: bool,
}

struct PrCore;

impl PrIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + call.flags.len() + 1);
        argv.push(OsString::from("pr"));

        push_opt(&mut argv, call, "pages", "pages", "--pages");
        push_opt(&mut argv, call, "header", "h", "--header");
        push_switch(&mut argv, call, "double-space", "d", "--double-space");
        push_opt(&mut argv, call, "number-lines", "n", "--number-lines");
        push_opt(
            &mut argv,
            call,
            "first-line-number",
            "N",
            "--first-line-number",
        );
        push_switch(&mut argv, call, "omit-header", "t", "--omit-header");
        push_opt(&mut argv, call, "length", "l", "--length");
        push_switch(
            &mut argv,
            call,
            "no-file-warnings",
            "r",
            "--no-file-warnings",
        );
        push_switch(&mut argv, call, "form-feed", "F", "--form-feed");
        push_opt(&mut argv, call, "width", "w", "--width");
        push_opt(&mut argv, call, "page-width", "W", "--page-width");
        push_switch(&mut argv, call, "across", "a", "--across");
        push_opt(&mut argv, call, "column", "column", "--column");
        push_opt(&mut argv, call, "separator", "s", "--separator");
        push_opt(&mut argv, call, "sep-string", "S", "--sep-string");
        push_switch(&mut argv, call, "merge", "m", "--merge");
        push_opt(&mut argv, call, "indent", "o", "--indent");
        push_switch(&mut argv, call, "join-lines", "J", "--join-lines");
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

impl PrCore {
    fn run_core(
        intent: &PrIntent,
        input: CtPipelineData,
    ) -> Result<(CtValue, String, String, i32), CtDiagnosticError> {
        let semantic = ctengine::run_with_optional_pipeline_stdin(
            "pr",
            input,
            intent.uses_stdin,
            || {
                ct_pr::pr_native_semantic(intent.argv.iter().cloned())
                    .map_err(|err| CtDiagnosticError::simple(err.to_string()).with_code(err.code()))
            },
            |err| CtDiagnosticError::simple(format!("pr: {err}")),
        )?;
        Ok((
            semantic_to_value(&semantic),
            semantic.classic_text,
            semantic.stderr_text,
            semantic.exit_code,
        ))
    }
}

fn optional_string(value: &Option<String>) -> CtValue {
    match value {
        Some(value) => CtValue::String(value.clone()),
        None => CtValue::Nothing,
    }
}

fn optional_usize(value: Option<usize>) -> CtValue {
    match value {
        Some(value) => CtValue::Int(i64::try_from(value).expect("value fits")),
        None => CtValue::Nothing,
    }
}

fn row_to_value(row: &ct_pr::PrSemanticRow) -> CtValue {
    CtValue::Record(vec![
        (
            "page".into(),
            CtValue::Int(i64::try_from(row.page).expect("page fits")),
        ),
        ("kind".into(), CtValue::String(row.kind.clone())),
        ("section".into(), CtValue::String(row.section.clone())),
        ("file".into(), optional_string(&row.file)),
        (
            "file_id".into(),
            CtValue::Int(i64::try_from(row.file_id).expect("file id fits")),
        ),
        ("line_index".into(), optional_usize(row.line_index)),
        (
            "group_key".into(),
            CtValue::Int(i64::try_from(row.group_key).expect("group key fits")),
        ),
        ("text".into(), CtValue::String(row.text.clone())),
    ])
}

fn semantic_to_value(semantic: &ct_pr::PrSemantic) -> CtValue {
    CtValue::List(semantic.rows.iter().map(row_to_value).collect())
}

fn value_to_arg(value: &CtValue) -> String {
    match value {
        CtValue::String(s) => s.clone(),
        other => other.to_text(),
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
    "pages",
    "header",
    "h",
    "double-space",
    "d",
    "number-lines",
    "n",
    "first-line-number",
    "N",
    "omit-header",
    "t",
    "length",
    "l",
    "no-file-warnings",
    "r",
    "form-feed",
    "F",
    "width",
    "w",
    "page-width",
    "W",
    "across",
    "a",
    "column",
    "separator",
    "s",
    "sep-string",
    "S",
    "merge",
    "m",
    "indent",
    "o",
    "join-lines",
    "J",
];

impl DataCommand for CmdPr {
    fn signature(&self) -> DataSignature {
        DataSignature::new("pr", "structured pr visible rows")
            .flag(CtFlag::with_value(
                "pages",
                None,
                "first:last page range",
                CtType::String,
            ))
            .flag(CtFlag::with_value(
                "header",
                Some('h'),
                "header string",
                CtType::String,
            ))
            .flag(CtFlag::switch(
                "double-space",
                Some('d'),
                "double spacing output",
            ))
            .flag(CtFlag::with_value(
                "number-lines",
                Some('n'),
                "line numbering style",
                CtType::String,
            ))
            .flag(CtFlag::with_value(
                "first-line-number",
                Some('N'),
                "first line number",
                CtType::Int,
            ))
            .flag(CtFlag::switch(
                "omit-header",
                Some('t'),
                "omit header and trailer",
            ))
            .flag(CtFlag::with_value(
                "length",
                Some('l'),
                "page length",
                CtType::Int,
            ))
            .flag(CtFlag::switch(
                "no-file-warnings",
                Some('r'),
                "suppress file warnings",
            ))
            .flag(CtFlag::switch(
                "form-feed",
                Some('F'),
                "use form feed page separator",
            ))
            .flag(CtFlag::with_value(
                "width",
                Some('w'),
                "column width",
                CtType::Int,
            ))
            .flag(CtFlag::with_value(
                "page-width",
                Some('W'),
                "page width",
                CtType::Int,
            ))
            .flag(CtFlag::switch("across", Some('a'), "across mode"))
            .flag(CtFlag::with_value(
                "column",
                None,
                "columns to print",
                CtType::Int,
            ))
            .flag(CtFlag::with_value(
                "separator",
                Some('s'),
                "column separator character",
                CtType::String,
            ))
            .flag(CtFlag::with_value(
                "sep-string",
                Some('S'),
                "column separator string",
                CtType::String,
            ))
            .flag(CtFlag::switch("merge", Some('m'), "merge files"))
            .flag(CtFlag::with_value(
                "indent",
                Some('o'),
                "output indent",
                CtType::Int,
            ))
            .flag(CtFlag::switch("join-lines", Some('J'), "join full lines"))
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
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let intent = PrIntent::from_call(call)?;
        let (value, classic_text, stderr_text, exit_code) = PrCore::run_core(&intent, input)?;
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
                source: Some("pr".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{PrIntent, semantic_to_value};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};
    use std::ffi::OsString;

    #[test]
    fn from_call_builds_argv_from_flags_and_positionals() {
        let mut call = DataCall::named("pr");
        call.flags.insert("t".into(), None);
        call.positionals
            .push(BoundArg::new(CtValue::String("sample.txt".into()), None));

        let intent = PrIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![
                OsString::from("pr"),
                OsString::from("--omit-header"),
                OsString::from("sample.txt"),
            ]
        );
    }

    #[test]
    fn from_call_preserves_unknown_flags_for_gnu_errors() {
        let mut call = DataCall::named("pr");
        call.flags.insert("definitely-invalid".into(), None);

        let intent = PrIntent::from_call(&call).expect("intent");

        assert_eq!(
            intent.argv,
            vec![OsString::from("pr"), OsString::from("--definitely-invalid")]
        );
    }

    #[test]
    fn semantic_to_value_renders_rows() {
        let value = semantic_to_value(&ct_pr::PrSemantic {
            rows: vec![ct_pr::PrSemanticRow {
                page: 1,
                kind: "body".into(),
                section: "content".into(),
                file: Some("sample.txt".into()),
                file_id: 0,
                line_index: Some(1),
                group_key: 0,
                text: "alpha".into(),
            }],
            classic_text: "alpha\n".into(),
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
        assert!(matches!(field(fields, "page"), CtValue::Int(1)));
        assert!(matches!(field(fields, "kind"), CtValue::String(s) if s == "body"));
        assert!(matches!(field(fields, "section"), CtValue::String(s) if s == "content"));
        assert!(matches!(field(fields, "file"), CtValue::String(s) if s == "sample.txt"));
        assert!(matches!(field(fields, "file_id"), CtValue::Int(0)));
        assert!(matches!(field(fields, "line_index"), CtValue::Int(1)));
        assert!(matches!(field(fields, "group_key"), CtValue::Int(0)));
        assert!(matches!(field(fields, "text"), CtValue::String(s) if s == "alpha"));
    }

    fn field<'a>(fields: &'a [(String, CtValue)], name: &str) -> &'a CtValue {
        fields
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
            .unwrap_or_else(|| panic!("missing field {name}"))
    }
}
