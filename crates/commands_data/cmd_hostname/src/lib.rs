use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{CtPositionalArg, DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdHostname;

#[derive(Debug)]
struct HostnameIntent {
    query_type: ct_hostname::NameType,
    has_explicit_query: bool,
}

struct HostnameCore;

impl HostnameIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("hostname"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple(
                    "hostname: argument must be string",
                ));
            };
            argv.push(OsString::from(arg));
        }

        let matches = ct_hostname::hostname_app("hostname")
            .try_get_matches_from(argv.iter().cloned())
            .map_err(|e| CtDiagnosticError::simple(e.to_string()).with_code(1))?;

        if matches.get_flag(ct_hostname::opt_flags::BOOT)
            || matches
                .get_one::<String>(ct_hostname::opt_flags::FILE)
                .is_some()
            || matches
                .get_many::<String>(ct_hostname::opt_flags::NAME)
                .is_some()
        {
            return Err(CtDiagnosticError::simple(
                "hostname: setting hostname is not supported in data mode",
            )
            .with_code(1));
        }

        let query_type = ct_hostname::resolve_name_type(ct_hostname::NameType::Default, &argv);
        let has_explicit_query = query_type != ct_hostname::NameType::Default;

        Ok(Self {
            query_type,
            has_explicit_query,
        })
    }
}

impl HostnameCore {
    fn run_core(intent: &HostnameIntent) -> Result<(CtValue, String), CtDiagnosticError> {
        let classic_text = ct_hostname::hostname_text_for_name_type("hostname", intent.query_type)
            .map_err(|e| CtDiagnosticError::simple(e.to_string()).with_code(e.code()))?;

        let value = if intent.has_explicit_query {
            value_for_query(intent.query_type, &classic_text)
        } else {
            let short =
                ct_hostname::hostname_text_for_name_type("hostname", ct_hostname::NameType::Short)
                    .map_err(|e| CtDiagnosticError::simple(e.to_string()).with_code(e.code()))?;
            CtValue::Record(vec![
                ("hostname".into(), CtValue::String(classic_text.clone())),
                ("short".into(), CtValue::String(short)),
            ])
        };

        Ok((value, classic_text))
    }
}

fn value_for_query(query_type: ct_hostname::NameType, text: &str) -> CtValue {
    match query_type {
        ct_hostname::NameType::Short => {
            CtValue::Record(vec![("short".into(), CtValue::String(text.to_string()))])
        }
        ct_hostname::NameType::Dns => {
            CtValue::Record(vec![("domain".into(), CtValue::String(text.to_string()))])
        }
        ct_hostname::NameType::Fqdn => {
            CtValue::Record(vec![("fqdn".into(), CtValue::String(text.to_string()))])
        }
        ct_hostname::NameType::Alias => CtValue::Record(vec![(
            "aliases".into(),
            CtValue::List(
                text.split_whitespace()
                    .map(|value| CtValue::String(value.to_string()))
                    .collect(),
            ),
        )]),
        ct_hostname::NameType::Ip => CtValue::Record(vec![(
            "ip_addresses".into(),
            CtValue::List(
                text.split_whitespace()
                    .map(|value| CtValue::String(value.to_string()))
                    .collect(),
            ),
        )]),
        ct_hostname::NameType::Nis | ct_hostname::NameType::NisDef => CtValue::Record(vec![(
            "nis_domain".into(),
            CtValue::String(text.to_string()),
        )]),
        ct_hostname::NameType::AllFqdns => CtValue::Record(vec![(
            "all_fqdns".into(),
            CtValue::List(
                text.split_whitespace()
                    .map(|value| CtValue::String(value.to_string()))
                    .collect(),
            ),
        )]),
        ct_hostname::NameType::AllIps => CtValue::Record(vec![(
            "all_ip_addresses".into(),
            CtValue::List(
                text.split_whitespace()
                    .map(|value| CtValue::String(value.to_string()))
                    .collect(),
            ),
        )]),
        ct_hostname::NameType::Default => {
            CtValue::Record(vec![("hostname".into(), CtValue::String(text.to_string()))])
        }
    }
}

impl DataCommand for CmdHostname {
    fn signature(&self) -> DataSignature {
        DataSignature::new("hostname", "structured hostname information")
            .rest(CtPositionalArg::optional(
                "arg",
                "GNU-compatible hostname query arguments",
                CtType::Any,
            ))
            .input(CtType::Nothing)
            .output(CtType::Record)
            .allow_unknown_args(true)
    }

    fn run(
        &self,
        call: &DataCall,
        _input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let intent = HostnameIntent::from_call(call)?;
        let (value, classic_text) = HostnameCore::run_core(&intent)?;
        Ok(CtPipelineData::Value(
            value,
            CtPipelineMetadata {
                classic_text: Some(classic_text),
                classic_bytes: None,
                classic_append_newline: true,
                stderr_text: None,
                exit_code: 0,
                source: Some("hostname".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{HostnameIntent, value_for_query};
    use ctpipeline::CtValue;
    use ctsig::{BoundArg, DataCall};

    #[test]
    fn from_call_rejects_set_hostname_operands() {
        let call = DataCall {
            positionals: vec![BoundArg::new(CtValue::String("new-host".into()), None)],
            ..DataCall::named("hostname")
        };

        let err = HostnameIntent::from_call(&call).expect_err("set operation should fail");

        assert_eq!(err.code, 1);
    }

    #[test]
    fn value_for_short_query_uses_short_field() {
        assert_eq!(
            value_for_query(ct_hostname::NameType::Short, "node"),
            CtValue::Record(vec![("short".into(), CtValue::String("node".into()))])
        );
    }
}
