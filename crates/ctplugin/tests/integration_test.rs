use ctengine::context::DataEngineContext;
use ctpipeline::pipeline_data::CtPipelineData;
use ctpipeline::value::CtValue;
use ctplugin::registry::PluginRegistry;
use ctsig::{BoundArg, DataCall};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(unix)]
fn write_test_plugin(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join("plugin_echo_test");
    let script = r#"#!/bin/sh
IFS= read -r _hello || exit 0
printf '%s\n' '{"type":"Hello","protocol":"2","commands":["plugin-echo"]}'
while IFS= read -r line; do
  case "$line" in
    *'"type":"Signature"'*) printf '%s\n' '{"type":"Signature","commands":["plugin-echo"]}' ;;
    *'"type":"Run"'*|*'"type":"Call"'*) printf '%s\n' '{"type":"CallResponse","accepted":true,"message":null,"code":0}'; break ;;
    *'"type":"Goodbye"'*) exit 0 ;;
  esac
done
while IFS= read -r line; do
  case "$line" in
    *'"type":"End"'*) break ;;
  esac
done
printf '%s\n' '{"type":"Data","value":{"kind":"string","value":"integration_test from plugin!"}}'
printf '%s\n' '{"type":"End"}'
printf '%s\n' '{"type":"Goodbye"}'
"#;
    fs::write(&path, script).expect("write test plugin");
    let mut perms = fs::metadata(&path).expect("plugin metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).expect("set executable bit");
    path
}

#[cfg(unix)]
#[test]
fn test_plugin_echo_integration() {
    let d = env::temp_dir().join(format!("syskits-ctplugin-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("create plugin test dir");
    let _plugin = write_test_plugin(&d);

    unsafe {
        env::set_var("SYSKITS_PLUGIN_PATH", d.to_string_lossy().as_ref());
    }

    let registry = PluginRegistry::discover();
    let cmd = registry.get_command("plugin-echo");

    assert!(
        cmd.is_some(),
        "plugin-echo should be discovered in {:?}",
        d.display()
    );

    let cmd = cmd.unwrap();

    let ctx = DataEngineContext::empty_for_test();
    let positionals = vec![BoundArg::new(
        CtValue::String("integration_test".into()),
        None,
    )];

    let mut call = DataCall::named("plugin-echo");
    call.positionals = positionals;

    let result = cmd
        .run(&call, CtPipelineData::Empty, &ctx)
        .expect("Command execution failed");

    match result {
        CtPipelineData::Value(val, _) => {
            if let CtValue::String(s) = &val {
                assert_eq!(s, "integration_test from plugin!");
            } else {
                panic!("Expected String value from plugin-echo");
            }
        }
        _ => panic!("Expected Value from plugin runner"),
    }

    let _ = fs::remove_dir_all(&d);
}
