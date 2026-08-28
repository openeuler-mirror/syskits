use ctengine::{CtDiagnosticError, DataCommand, DataEngineContext};
use ctpipeline::{CtPipelineData, CtPipelineMetadata, CtType, CtValue};
use ctsig::{DataCall, DataSignature};
use std::ffi::OsString;

#[derive(Default)]
pub struct CmdUname;

struct UnameIntent {
    classic_flags: ct_uname::UnameFlags,
    native_flags: ct_uname::UnameFlags,
}

struct UnameCore;

impl UnameCore {
    fn run_core(intent: &UnameIntent) -> Result<(CtValue, String), CtDiagnosticError> {
        let native_output = ct_uname::UNameOutput::new(&intent.native_flags)
            .map_err(|e| CtDiagnosticError::simple(e.to_string()))?;
        let classic_output = ct_uname::UNameOutput::new(&intent.classic_flags)
            .map_err(|e| CtDiagnosticError::simple(e.to_string()))?;

        Ok((
            uname_output_to_value(&native_output),
            classic_text_from_output(&classic_output),
        ))
    }
}

impl UnameIntent {
    fn from_call(call: &DataCall) -> Result<Self, CtDiagnosticError> {
        let mut argv = Vec::with_capacity(call.positionals.len() + 1);
        argv.push(OsString::from("uname"));

        for arg in &call.positionals {
            let CtValue::String(arg) = &arg.value else {
                return Err(CtDiagnosticError::simple("uname: argument must be string"));
            };
            argv.push(OsString::from(arg));
        }

        let matches = ct_uname::ct_app()
            .try_get_matches_from(argv)
            .map_err(|e| CtDiagnosticError::simple(e.to_string()).with_code(1))?;
        let classic_flags = flags_from_matches(&matches);
        let native_flags = if has_explicit_flags(&classic_flags) {
            copy_flags(&classic_flags)
        } else {
            rich_default_flags()
        };

        Ok(Self {
            classic_flags,
            native_flags,
        })
    }
}

fn rich_default_flags() -> ct_uname::UnameFlags {
    ct_uname::UnameFlags {
        is_all: true,
        is_kernel_name: false,
        is_node_name: false,
        is_kernel_version: false,
        is_kernel_release: false,
        is_machine: false,
        is_processor: false,
        is_hardware_platform: false,
        is_os: false,
    }
}

fn copy_flags(flags: &ct_uname::UnameFlags) -> ct_uname::UnameFlags {
    ct_uname::UnameFlags {
        is_all: flags.is_all,
        is_kernel_name: flags.is_kernel_name,
        is_node_name: flags.is_node_name,
        is_kernel_version: flags.is_kernel_version,
        is_kernel_release: flags.is_kernel_release,
        is_machine: flags.is_machine,
        is_processor: flags.is_processor,
        is_hardware_platform: flags.is_hardware_platform,
        is_os: flags.is_os,
    }
}

fn has_explicit_flags(flags: &ct_uname::UnameFlags) -> bool {
    flags.is_all
        || flags.is_kernel_name
        || flags.is_node_name
        || flags.is_kernel_version
        || flags.is_kernel_release
        || flags.is_machine
        || flags.is_processor
        || flags.is_hardware_platform
        || flags.is_os
}

fn flags_from_matches(matches: &clap::ArgMatches) -> ct_uname::UnameFlags {
    ct_uname::UnameFlags {
        is_all: matches.get_flag(ct_uname::uname_flags::UNAME_ALL),
        is_kernel_name: matches.get_flag(ct_uname::uname_flags::UNAME_KERNEL_NAME),
        is_node_name: matches.get_flag(ct_uname::uname_flags::UNAME_NODE_NAME),
        is_kernel_version: matches.get_flag(ct_uname::uname_flags::UNAME_KERNEL_VERSION),
        is_kernel_release: matches.get_flag(ct_uname::uname_flags::UNAME_KERNEL_RELEASE),
        is_machine: matches.get_flag(ct_uname::uname_flags::UNAME_MACHINE),
        is_processor: matches.get_flag(ct_uname::uname_flags::UNAME_PROCESSOR),
        is_hardware_platform: matches.get_flag(ct_uname::uname_flags::UNAME_HARDWARE_PLATFORM),
        is_os: matches.get_flag(ct_uname::uname_flags::UNAME_OS),
    }
}

fn uname_output_to_value(output: &ct_uname::UNameOutput) -> CtValue {
    let mut fields = Vec::new();

    if let Some(value) = &output.kernel_name {
        fields.push(("kernel_name".into(), CtValue::String(value.clone())));
    }
    if let Some(value) = &output.node_name {
        fields.push(("node_name".into(), CtValue::String(value.clone())));
    }
    if let Some(value) = &output.kernel_release {
        fields.push(("kernel_release".into(), CtValue::String(value.clone())));
    }
    if let Some(value) = &output.kernel_version {
        fields.push(("kernel_version".into(), CtValue::String(value.clone())));
    }
    if let Some(value) = &output.machine {
        fields.push(("machine".into(), CtValue::String(value.clone())));
    }
    if let Some(value) = &output.processor {
        fields.push(("processor".into(), CtValue::String(value.clone())));
    }
    if let Some(value) = &output.hardware_platform {
        fields.push(("hardware_platform".into(), CtValue::String(value.clone())));
    }
    if let Some(value) = &output.os {
        fields.push(("os".into(), CtValue::String(value.clone())));
    }

    CtValue::Record(fields)
}

fn classic_text_from_output(output: &ct_uname::UNameOutput) -> String {
    let mut parts = Vec::new();

    if let Some(value) = &output.kernel_name {
        parts.push(value.clone());
    }
    if let Some(value) = &output.node_name {
        parts.push(value.clone());
    }
    if let Some(value) = &output.kernel_release {
        parts.push(value.clone());
    }
    if let Some(value) = &output.kernel_version {
        parts.push(value.clone());
    }
    if let Some(value) = &output.machine {
        parts.push(value.clone());
    }
    if let Some(value) = &output.processor {
        parts.push(value.clone());
    }
    if let Some(value) = &output.hardware_platform {
        parts.push(value.clone());
    }
    if let Some(value) = &output.os {
        parts.push(value.clone());
    }

    parts.join(" ")
}

impl DataCommand for CmdUname {
    fn signature(&self) -> DataSignature {
        DataSignature::new("uname", "structured system information")
            .input(CtType::Nothing)
            .output(CtType::Record)
    }

    fn run(
        &self,
        call: &DataCall,
        _input: CtPipelineData,
        _ctx: &DataEngineContext,
    ) -> Result<CtPipelineData, CtDiagnosticError> {
        let intent = UnameIntent::from_call(call)?;
        let (value, classic_text) = UnameCore::run_core(&intent)?;
        Ok(CtPipelineData::Value(
            value,
            CtPipelineMetadata {
                classic_text: Some(classic_text),
                classic_bytes: None,
                classic_append_newline: true,
                stderr_text: None,
                exit_code: 0,
                source: Some("uname".into()),
                ..Default::default()
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{UnameIntent, classic_text_from_output, rich_default_flags, uname_output_to_value};
    use ctpipeline::CtValue;
    use ctsig::DataCall;

    #[test]
    fn rich_default_flags_enable_all_output_fields() {
        let flags = rich_default_flags();

        assert!(flags.is_all);
        assert!(!flags.is_kernel_name);
    }

    #[test]
    fn from_call_uses_rich_default_without_explicit_flags() {
        let call = DataCall::named("uname");

        let intent = UnameIntent::from_call(&call).expect("intent");

        assert!(intent.native_flags.is_all);
        assert!(!intent.classic_flags.is_all);
    }

    #[test]
    fn uname_output_to_value_uses_stable_field_names() {
        let value = uname_output_to_value(&ct_uname::UNameOutput {
            kernel_name: Some("Linux".into()),
            node_name: None,
            kernel_release: None,
            kernel_version: None,
            machine: None,
            os: Some("GNU/Linux".into()),
            processor: None,
            hardware_platform: None,
        });

        assert_eq!(
            value,
            CtValue::Record(vec![
                ("kernel_name".into(), CtValue::String("Linux".into())),
                ("os".into(), CtValue::String("GNU/Linux".into())),
            ])
        );
    }

    #[test]
    fn classic_text_from_output_trims_trailing_space() {
        let text = classic_text_from_output(&ct_uname::UNameOutput {
            kernel_name: Some("Linux".into()),
            node_name: None,
            kernel_release: None,
            kernel_version: None,
            machine: None,
            os: None,
            processor: None,
            hardware_platform: None,
        });

        assert_eq!(text, "Linux");
    }
}
