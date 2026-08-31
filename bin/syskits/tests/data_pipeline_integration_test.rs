#![cfg(feature = "feat_data_pipeline")]

use std::ffi::OsString;

use ctengine::{CommandRegistry, DataCommand, DataCommandFactory};

fn from_factory() -> Box<dyn DataCommand> {
    Box::new(cmd_from::CmdFrom)
}

fn get_factory() -> Box<dyn DataCommand> {
    Box::new(cmd_get::CmdGet)
}

fn select_factory() -> Box<dyn DataCommand> {
    Box::new(cmd_select::CmdSelect)
}

fn to_factory() -> Box<dyn DataCommand> {
    Box::new(cmd_to::CmdTo)
}

fn make_registry() -> CommandRegistry {
    let factories: Vec<(&'static str, DataCommandFactory)> = vec![
        ("from", from_factory as DataCommandFactory),
        ("get", get_factory as DataCommandFactory),
        ("select", select_factory as DataCommandFactory),
        ("to", to_factory as DataCommandFactory),
    ];
    CommandRegistry::from_factories(&factories)
}

#[test]
fn test_run_data_entry_from_to_json_inline_source() {
    let args = vec![OsString::from(
        r#"from json "{\"name\":\"CTyunOS\"}" | to json"#,
    )];
    let code = ctengine::run_data_entry_with_registry(&args, make_registry());
    assert_eq!(code, 0);
}

#[test]
fn test_run_data_entry_from_select_to_json_inline_source() {
    let args = vec![OsString::from(
        r#"from json "{\"name\":\"CTyunOS\",\"id\":1}" | select name | to json"#,
    )];
    let code = ctengine::run_data_entry_with_registry(&args, make_registry());
    assert_eq!(code, 0);
}

#[test]
fn test_run_data_entry_usage_error_exit_code() {
    let args = vec![OsString::from("| bad")];
    let code = ctengine::run_data_entry_with_registry(&args, make_registry());
    assert_eq!(code, ctengine::exit_code::USAGE_ERROR);
}

#[test]
fn test_run_data_entry_runtime_error_exit_code() {
    let args = vec![OsString::from("__syskits_no_such_cmd_xyz__")];
    let code = ctengine::run_data_entry_with_registry(&args, make_registry());
    assert_eq!(code, ctengine::exit_code::RUNTIME_ERROR);
}
