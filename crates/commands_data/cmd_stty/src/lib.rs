/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 */

use ctengine::command::DataCommand;
use ctengine::context::DataEngineContext;
use ctengine::error::CtDiagnosticError;
use ctengine::execution::{CommandCore, CommandRunner};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtFlag, CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdStty;

struct SttyCore;

impl DataCommand for CmdStty {
    fn signature(&self) -> DataSignature {
        DataSignature::new("stty", "structured terminal settings output")
            .flag(CtFlag::switch("all", Some('a'), "print all settings"))
            .flag(CtFlag::switch(
                "save",
                Some('g'),
                "print save-able settings",
            ))
            .flag(CtFlag::with_value(
                "file",
                Some('F'),
                "open and use specified device",
                CtType::String,
            ))
            .rest(CtPositionalArg::optional(
                "settings",
                "terminal settings",
                CtType::Any,
            ))
            .input(CtType::Nothing)
            .output(CtType::List)
    }

    fn run(
        &self,
        call: &DataCall,
        input: CtPipelineData,
        ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        CommandRunner::run(&SttyCore, call, input, ctx)
    }
}

impl CommandCore for SttyCore {
    fn run_core(
        &self,
        call: &DataCall,
        _input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let argv = build_stty_argv(call)?;
        let mut out = Vec::new();
        ct_stty::stty_main_with_writer(argv.clone().into_iter(), &mut out)
            .map_err(|e| CtDiagnosticError::simple(format!("stty: {e}")))?;
        let rows = parse_stty_output_rows(&argv, out);
        Ok(CtPipelineData::Value(
            CtValue::List(rows),
            CtPipelineMetadata::default(),
        ))
    }
}

fn parse_stty_output_rows(argv: &[OsString], out: Vec<u8>) -> Vec<CtValue> {
    let text = String::from_utf8_lossy(&out).trim().to_string();
    if text.is_empty() {
        return Vec::new();
    }

    if argv.iter().any(|v| v == "-g") {
        return vec![CtValue::Record(vec![
            ("setting".to_string(), CtValue::String("save".to_string())),
            ("kind".to_string(), CtValue::String("mode".to_string())),
            ("value".to_string(), CtValue::String(text)),
        ])];
    }

    let mut rows = Vec::new();
    for segment in text.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        if let Some((k, v)) = parse_key_value_segment(segment) {
            rows.push(CtValue::Record(vec![
                ("setting".to_string(), CtValue::String(k)),
                ("kind".to_string(), CtValue::String("value".to_string())),
                ("value".to_string(), CtValue::String(v)),
            ]));
            continue;
        }

        for token in segment.split_whitespace() {
            if token.is_empty() {
                continue;
            }
            let (enabled, name) = if let Some(rest) = token.strip_prefix('-') {
                (false, rest.to_string())
            } else {
                (true, token.to_string())
            };
            rows.push(CtValue::Record(vec![
                ("setting".to_string(), CtValue::String(name)),
                ("kind".to_string(), CtValue::String("flag".to_string())),
                ("enabled".to_string(), CtValue::Bool(enabled)),
            ]));
        }
    }

    rows
}

fn parse_key_value_segment(segment: &str) -> Option<(String, String)> {
    if let Some((k, v)) = segment.split_once('=') {
        return Some((k.trim().to_string(), v.trim().to_string()));
    }

    if let Some(speed) = segment
        .strip_prefix("speed ")
        .and_then(|s| s.strip_suffix(" baud"))
    {
        return Some(("speed".to_string(), speed.trim().to_string()));
    }

    if let Some((k, v)) = segment.split_once(' ') {
        if matches!(k, "rows" | "columns" | "line" | "min" | "time" | "iutf8") {
            return Some((k.to_string(), v.trim().to_string()));
        }
    }

    None
}

fn build_stty_argv(call: &DataCall) -> Result<Vec<OsString>, CtDiagnosticError> {
    let mut argv = vec![OsString::from("stty")];

    push_switch(&mut argv, call, "all", "a", "-a");
    push_switch(&mut argv, call, "save", "g", "-g");
    push_opt(&mut argv, call, "file", "F", "-F");

    let settings = call
        .rest::<CtValue>(0)
        .map_err(|e| CtDiagnosticError::simple(format!("stty: {e}")))?;
    for setting in settings {
        argv.push(OsString::from(match setting {
            CtValue::String(s) => s,
            other => other.to_text(),
        }));
    }

    Ok(argv)
}

fn push_switch(argv: &mut Vec<OsString>, call: &DataCall, long: &str, short: &str, cli: &str) {
    if call.has_flag(long) || call.has_flag(short) {
        argv.push(OsString::from(cli));
    }
}

fn push_opt(argv: &mut Vec<OsString>, call: &DataCall, long: &str, short: &str, cli: &str) {
    let value = [long, short]
        .iter()
        .find_map(|k| call.flags.get(*k))
        .and_then(|arg| arg.as_ref())
        .map(|arg| match &arg.value {
            CtValue::String(s) => s.clone(),
            other => other.to_text(),
        });

    if let Some(v) = value {
        argv.push(OsString::from(cli));
        argv.push(OsString::from(v));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctengine::context::{CommandRegistry, DataEngineContext};
    use ctsig::BoundArg;
    use std::io::IsTerminal;

    fn ctx() -> DataEngineContext {
        DataEngineContext::new(CommandRegistry::empty(), None, None)
    }

    #[test]
    fn stty_build_argv_maps_switches_file_and_settings() {
        let mut call = DataCall::named("stty");
        call.flags.insert("a".to_string(), None);
        call.flags.insert("save".to_string(), None);
        call.flags.insert(
            "file".to_string(),
            Some(BoundArg::new(
                CtValue::String("/dev/ttyS0".to_string()),
                None,
            )),
        );
        call.positionals
            .push(BoundArg::new(CtValue::String("rows".to_string()), None));
        call.positionals.push(BoundArg::new(CtValue::Int(40), None));

        let argv = build_stty_argv(&call).expect("argv should build");
        let args = argv
            .iter()
            .map(|s| s.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert_eq!(
            args,
            vec![
                "stty".to_string(),
                "-a".to_string(),
                "-g".to_string(),
                "-F".to_string(),
                "/dev/ttyS0".to_string(),
                "rows".to_string(),
                "40".to_string(),
            ]
        );
    }

    #[test]
    fn stty_build_argv_prefers_long_file_flag_over_short_alias() {
        let mut call = DataCall::named("stty");
        call.flags.insert(
            "file".to_string(),
            Some(BoundArg::new(
                CtValue::String("/dev/ttyLONG".to_string()),
                None,
            )),
        );
        call.flags.insert(
            "F".to_string(),
            Some(BoundArg::new(
                CtValue::String("/dev/ttySHORT".to_string()),
                None,
            )),
        );

        let argv = build_stty_argv(&call).expect("argv should build");
        let args = argv
            .iter()
            .map(|s| s.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert!(args.windows(2).any(|w| w == ["-F", "/dev/ttyLONG"]));
        assert!(!args.iter().any(|a| a == "/dev/ttySHORT"));
    }

    #[test]
    fn stty_parse_rows_for_all_mode() {
        let rows = parse_stty_output_rows(
            &[OsString::from("stty"), OsString::from("-a")],
            b"speed 38400 baud; rows 20; columns 80; isig -echo".to_vec(),
        );
        assert!(rows.iter().any(|row| {
            matches!(
                row,
                CtValue::Record(fields)
                    if fields.iter().any(|(k, v)| k == "setting" && matches!(v, CtValue::String(s) if s == "speed"))
            )
        }));
        assert!(rows.iter().any(|row| {
            matches!(
                row,
                CtValue::Record(fields)
                    if fields.iter().any(|(k, v)| k == "setting" && matches!(v, CtValue::String(s) if s == "echo"))
            )
        }));
    }

    #[test]
    fn stty_output_is_structured_list() {
        if !std::io::stdout().is_terminal() {
            return;
        }

        let mut call = DataCall::named("stty");
        call.flags.insert("save".to_string(), None);

        let out = CmdStty
            .run(&call, CtPipelineData::Empty, &ctx())
            .expect("stty should work");

        let CtPipelineData::Value(CtValue::List(items), _) = out else {
            panic!("expected list output");
        };
        assert!(!items.is_empty());
    }
}
