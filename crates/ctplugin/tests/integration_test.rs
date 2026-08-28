use ctengine::context::DataEngineContext;
use ctpipeline::pipeline_data::CtPipelineData;
use ctpipeline::value::CtValue;
use ctplugin::registry::PluginRegistry;
use ctsig::{BoundArg, DataCall};
use std::env;
use std::path::PathBuf;

#[test]
fn test_plugin_echo_integration() {
    // 1. Build the path to the examples directory
    let mut d = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    d.pop(); // move up from "ctplugin"
    d.pop(); // move up from "crates" to workspace root
    d.push("target");
    // Depending on profile, it might be in debug or release.
    // We assume debug for cargo test.
    d.push("debug");
    d.push("examples");

    // 2. Set SYSKITS_PLUGIN_PATH
    unsafe {
        env::set_var("SYSKITS_PLUGIN_PATH", d.to_string_lossy().as_ref());
    }

    // 3. Discover plugins
    let registry = PluginRegistry::discover();
    let cmd = registry.get_command("plugin-echo");

    assert!(
        cmd.is_some(),
        "plugin-echo should be discovered in {:?}",
        d.display()
    );

    let cmd = cmd.unwrap();

    // 4. Setup mock call
    let ctx = DataEngineContext::empty_for_test();
    let mut positionals = Vec::new();
    positionals.push(BoundArg::new(
        CtValue::String("integration_test".into()),
        None,
    ));

    let mut call = DataCall::named("plugin-echo");
    call.positionals = positionals;

    // 5. Run command
    let result = cmd
        .run(&call, CtPipelineData::Empty, &ctx)
        .expect("Command execution failed");

    // 6. Verify result
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
}
