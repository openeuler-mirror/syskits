use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("bin/syskits is two levels below the repo root")
        .to_path_buf()
}

fn syskits_manifest() -> String {
    fs::read_to_string(repo_root().join("bin/syskits/Cargo.toml"))
        .expect("read bin/syskits/Cargo.toml")
}

fn uname_manifest() -> String {
    fs::read_to_string(repo_root().join("crates/commands/uname/Cargo.toml"))
        .expect("read crates/commands/uname/Cargo.toml")
}

fn feature_entries(manifest: &str, feature: &str) -> Vec<String> {
    let needle = format!("{feature} = [");
    let mut in_feature = false;
    let mut entries = Vec::new();

    for line in manifest.lines() {
        let trimmed = line.trim();
        if !in_feature {
            if let Some(rest) = trimmed.strip_prefix(&needle) {
                entries.extend(quoted_values(rest));
                if rest.contains(']') {
                    break;
                }
                in_feature = true;
            }
            continue;
        }

        entries.extend(quoted_values(trimmed));
        if trimmed.starts_with(']') {
            break;
        }
    }

    entries
}

fn quoted_values(text: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut rest = text;

    while let Some((_, after_open)) = rest.split_once('"') {
        let Some((value, after_close)) = after_open.split_once('"') else {
            break;
        };
        values.push(value.to_string());
        rest = after_close;
    }

    values
}

#[test]
fn feature_manifest_removes_typed_cmds_axis() {
    let manifest = syskits_manifest();
    let uname = uname_manifest();

    assert!(!manifest.contains("feat_typed_cmds"));
    assert!(!uname.contains("feat_typed_cmds"));
}

#[test]
fn unix_feature_includes_shell_data_workflow_and_plugin_stack() {
    let manifest = syskits_manifest();
    let unix = feature_entries(&manifest, "unix");

    assert!(unix.contains(&"feat_os_unix".to_string()));
    assert!(unix.contains(&"feat_shell_interactive".to_string()));
    assert!(unix.contains(&"feat_data_workflow".to_string()));
    assert!(unix.contains(&"feat_data_plugin".to_string()));
}

#[test]
fn shell_interactive_publishes_data_pipeline_and_shell_init() {
    let manifest = syskits_manifest();
    let shell_interactive = feature_entries(&manifest, "feat_shell_interactive");
    let shell_init = feature_entries(&manifest, "feat_shell_init");
    let common_core = feature_entries(&manifest, "feat_common_core");

    assert!(shell_interactive.contains(&"feat_data_pipeline".to_string()));
    assert!(shell_interactive.contains(&"feat_shell_init".to_string()));
    assert_eq!(shell_init, vec!["shell_init"]);
    assert!(!common_core.contains(&"shell_init".to_string()));
}

#[test]
fn uname_data_adapter_stays_in_data_pipeline() {
    let manifest = syskits_manifest();
    let data_pipeline = feature_entries(&manifest, "feat_data_pipeline");

    assert!(data_pipeline.contains(&"cmd_uname".to_string()));
}

#[test]
fn data_runtime_dependencies_are_optional_and_pipeline_gated() {
    let manifest = syskits_manifest();
    let data_pipeline = feature_entries(&manifest, "feat_data_pipeline");

    for dependency in ["ctengine", "ctpipeline", "ctsig", "ctrepl"] {
        assert!(
            manifest.contains(&format!("{dependency} = {{ optional = true")),
            "{dependency} must be optional in bin/syskits/Cargo.toml"
        );
        assert!(
            data_pipeline.contains(&dependency.to_string()),
            "{dependency} must be enabled by feat_data_pipeline"
        );
    }
}
