/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 */

//! `ctrepl` — syskits 数据管线交互模式。

extern crate rust_i18n;
rust_i18n::i18n!("locales", fallback = "en-US");

mod config;
mod control;
mod eval;
mod prompt;

pub use eval::run_repl;

#[cfg(test)]
mod tests {
    use crate::config::{parse_history_persistence, texts_for_current_locale};
    use crate::control::{
        ControlAction, REPL_HISTORY_LINES_LIMIT, cd_target_from_input, handle_control_command,
        load_history_lines_from_file, push_history_line,
    };
    use crate::eval::{
        filter_precheck_diags_for_repl, filter_precheck_diags_for_repl_with_known_command,
        has_precheck_error, parse_pipeline_expr, resolve_repl_line_format,
    };
    use crate::prompt::{
        ReplPrompt, build_completion_candidates, format_prompt_path, line_is_incomplete,
    };
    use ctengine::OutputFormat;
    use reedline::Prompt;
    use std::collections::{HashMap, VecDeque};
    use std::path::PathBuf;

    #[test]
    fn test_debug_command_state_transitions() {
        rust_i18n::set_locale("en-US");
        let mut debug = false;
        let texts = texts_for_current_locale();
        let history = VecDeque::new();
        assert_eq!(
            handle_control_command("debug status", &mut debug, &texts, &history),
            ControlAction::Continue
        );
        assert!(!debug);
        assert_eq!(
            handle_control_command("debug on", &mut debug, &texts, &history),
            ControlAction::Continue
        );
        assert!(debug);
        assert_eq!(
            handle_control_command("debug off", &mut debug, &texts, &history),
            ControlAction::Continue
        );
        assert!(!debug);
    }

    #[test]
    fn test_meta_command_branching() {
        rust_i18n::set_locale("en-US");
        let mut debug = false;
        let texts = texts_for_current_locale();
        let empty_history = VecDeque::new();
        let history = VecDeque::from(vec!["a".to_string()]);
        assert_eq!(
            handle_control_command("help", &mut debug, &texts, &empty_history),
            ControlAction::Continue
        );
        assert_eq!(
            handle_control_command("history", &mut debug, &texts, &history),
            ControlAction::Continue
        );
        assert_eq!(
            handle_control_command("trace on", &mut debug, &texts, &empty_history),
            ControlAction::Continue
        );
        assert_eq!(
            handle_control_command("trace", &mut debug, &texts, &empty_history),
            ControlAction::Continue
        );
        assert!(debug);
        assert_eq!(
            handle_control_command(
                "ast from json | to json",
                &mut debug,
                &texts,
                &empty_history
            ),
            ControlAction::Continue
        );
        assert_eq!(
            handle_control_command("ast", &mut debug, &texts, &empty_history),
            ControlAction::Continue
        );
        assert_eq!(
            handle_control_command("clear", &mut debug, &texts, &empty_history),
            ControlAction::Continue
        );
        assert_eq!(
            handle_control_command("quit", &mut debug, &texts, &empty_history),
            ControlAction::Exit
        );
    }

    #[test]
    fn test_parse_error_loop_continuation_helper() {
        let parsed = parse_pipeline_expr("| bad");
        assert!(parsed.is_err());
        let valid = parse_pipeline_expr("from json | select name | to json");
        assert!(valid.is_ok());
    }

    #[test]
    fn test_repl_line_format_prefix_supports_public_formats() {
        let cases = [
            ("format=auto ls", OutputFormat::Auto, "ls"),
            ("format=text ls", OutputFormat::Text, "ls"),
            ("format=table ls", OutputFormat::Table, "ls"),
            ("format=json ls", OutputFormat::Json, "ls"),
            ("format=classic ls", OutputFormat::Classic, "ls"),
        ];

        for (line, expected_format, expected_expr) in cases {
            let resolved = resolve_repl_line_format(line).expect("line format");
            assert_eq!(resolved.profile.format, expected_format);
            assert_eq!(resolved.expr_src, expected_expr);
            assert!(resolved.profile.stdout_is_tty);
            assert!(resolved.profile.use_pager);
        }
    }

    #[test]
    fn test_repl_line_format_prefix_supports_cli_flag_forms() {
        let resolved = resolve_repl_line_format("--format json ls").expect("long flag");
        assert_eq!(resolved.profile.format, OutputFormat::Json);
        assert_eq!(resolved.expr_src, "ls");

        let resolved = resolve_repl_line_format("--format=classic ls").expect("equals flag");
        assert_eq!(resolved.profile.format, OutputFormat::Classic);
        assert_eq!(resolved.expr_src, "ls");
    }

    #[test]
    fn test_repl_line_format_prefix_last_override_wins() {
        let resolved =
            resolve_repl_line_format("format=json --format text format=classic ls").expect("line");
        assert_eq!(resolved.profile.format, OutputFormat::Classic);
        assert_eq!(resolved.expr_src, "ls");
    }

    #[test]
    fn test_repl_line_format_rejects_unsupported_raw() {
        let err = resolve_repl_line_format("format=raw ls").expect_err("raw unsupported");
        assert_eq!(err, "unsupported format `raw`");
    }

    #[test]
    fn test_repl_line_format_without_prefix_uses_repl_default() {
        let resolved = resolve_repl_line_format("from json | to json").expect("default");
        assert_eq!(resolved.profile.format, OutputFormat::Auto);
        assert_eq!(resolved.expr_src, "from json | to json");
    }

    #[test]
    fn test_texts_for_locale_zh_and_en() {
        rust_i18n::set_locale("en-US");
        let en = texts_for_current_locale();
        rust_i18n::set_locale("zh-CN");
        let zh = texts_for_current_locale();
        assert!(en.banner.contains("structured-data"));
        assert!(zh.banner.contains("结构化数据"));
    }

    #[test]
    fn test_build_completion_candidates_contains_meta_builtin_and_flags() {
        let mut signatures = HashMap::new();
        let sig =
            ctsig::DataSignature::new("from", "convert from").flag(ctsig::CtFlag::with_value(
                "format",
                Some('f'),
                "input format",
                ctpipeline::CtType::String,
            ));
        signatures.insert("from".to_string(), sig);

        let words: Vec<String> = build_completion_candidates(&signatures)
            .into_iter()
            .map(|c| c.value)
            .collect();
        assert!(words.binary_search(&"from".to_string()).is_ok());
        assert!(words.binary_search(&"--format".to_string()).is_ok());
        assert!(words.binary_search(&"-f".to_string()).is_ok());
        assert!(words.binary_search(&"help".to_string()).is_ok());
        assert!(words.binary_search(&"exit".to_string()).is_ok());
        assert!(words.binary_search(&"quit".to_string()).is_ok());
        assert!(words.binary_search(&"history".to_string()).is_ok());
        assert!(words.binary_search(&"trace".to_string()).is_ok());
        assert!(words.binary_search(&"ast".to_string()).is_ok());
        assert!(words.binary_search(&"cd".to_string()).is_ok());
        assert!(words.binary_search(&"pwd".to_string()).is_ok());
        assert!(words.binary_search(&"format=auto".to_string()).is_ok());
        assert!(words.binary_search(&"format=text".to_string()).is_ok());
        assert!(words.binary_search(&"format=table".to_string()).is_ok());
        assert!(words.binary_search(&"format=json".to_string()).is_ok());
        assert!(words.binary_search(&"format=classic".to_string()).is_ok());
        assert!(words.binary_search(&":history".to_string()).is_err());
        assert!(words.binary_search(&":trace".to_string()).is_err());
        assert!(words.binary_search(&":ast".to_string()).is_err());
    }

    #[test]
    fn test_has_precheck_error_gates_execution() {
        let warning = ctdsl::PrecheckDiagnostic {
            level: ctdsl::PrecheckLevel::Warning,
            message: "warn".to_string(),
            stage_index: 0,
            span: None,
        };
        let error = ctdsl::PrecheckDiagnostic {
            level: ctdsl::PrecheckLevel::Error,
            message: "err".to_string(),
            stage_index: 1,
            span: None,
        };
        assert!(!has_precheck_error(&[warning.clone()]));
        assert!(has_precheck_error(&[warning, error]));
    }

    #[test]
    fn test_parse_history_persistence() {
        assert!(parse_history_persistence(None));
        assert!(parse_history_persistence(Some("on")));
        assert!(!parse_history_persistence(Some("off")));
        assert!(!parse_history_persistence(Some("FALSE")));
    }

    #[test]
    fn test_line_is_incomplete() {
        assert!(line_is_incomplete("from json |"));
        assert!(line_is_incomplete("format=table from json |"));
        assert!(line_is_incomplete("run-external echo \"abc"));
        assert!(line_is_incomplete("from json '{\"a\":1"));
        assert!(!line_is_incomplete("from json '{\"a\":1}'"));
        assert!(!line_is_incomplete("format=json from json '{\"a\":1}'"));
        assert!(!line_is_incomplete("from json | select name"));
    }

    #[test]
    fn test_cd_target_from_input() {
        assert_eq!(cd_target_from_input("cd"), Some(None));
        assert_eq!(cd_target_from_input("cd crates"), Some(Some("crates")));
        assert_eq!(cd_target_from_input("pwd"), None);
    }

    #[test]
    fn test_push_history_line_keeps_fixed_capacity() {
        let mut history = VecDeque::new();
        for idx in 0..(REPL_HISTORY_LINES_LIMIT + 3) {
            push_history_line(&mut history, &format!("cmd-{idx}"));
        }
        assert_eq!(history.len(), REPL_HISTORY_LINES_LIMIT);
        assert_eq!(history.front().map(String::as_str), Some("cmd-3"));
        assert_eq!(history.back().map(String::as_str), Some("cmd-502"));
    }

    #[test]
    fn test_load_history_lines_from_file_missing_is_empty() {
        let dir = unique_test_dir("missing-history");
        std::fs::create_dir_all(&dir).expect("create tempdir");
        let history = load_history_lines_from_file(&dir.join("missing-history"))
            .expect("missing history is ok");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(history.is_empty());
    }

    #[test]
    fn test_load_history_lines_from_file_keeps_tail_capacity() {
        let dir = unique_test_dir("tail-history");
        std::fs::create_dir_all(&dir).expect("create tempdir");
        let path = dir.join("history");
        let content = (0..(REPL_HISTORY_LINES_LIMIT + 3))
            .map(|idx| format!("cmd-{idx}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, content).expect("write history");

        let history = load_history_lines_from_file(&path).expect("load history");
        assert_eq!(history.len(), REPL_HISTORY_LINES_LIMIT);
        assert_eq!(history.front().map(String::as_str), Some("cmd-3"));
        assert_eq!(history.back().map(String::as_str), Some("cmd-502"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn unique_test_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ctrepl-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time ok")
                .as_nanos()
        ))
    }

    #[test]
    fn test_prompt_left_contains_cwd_name() {
        let p = ReplPrompt {
            prompt_left: "syskits".to_string(),
            path_depth: 3,
        };
        let left = p.render_prompt_left().to_string();
        assert!(left.starts_with("syskits("));
        assert!(left.ends_with(')'));
    }

    #[test]
    fn test_format_prompt_path_tail_depth() {
        let p = PathBuf::from("/a/b/c/d/e");
        assert_eq!(format_prompt_path(&p, 3), ".../c/d/e");
        assert_eq!(format_prompt_path(&p, 1), ".../e");
    }

    #[test]
    fn test_format_prompt_path_full_when_depth_zero() {
        let p = PathBuf::from("/a/b/c");
        assert_eq!(format_prompt_path(&p, 0), "/a/b/c");
    }

    #[test]
    fn test_format_prompt_path_absolute_shallow_path_has_no_double_slash() {
        let p = PathBuf::from("/tmp");
        assert_eq!(format_prompt_path(&p, 3), "/tmp");
    }

    #[test]
    fn test_cursor_position_timeout_is_recoverable() {
        assert!(super::eval::is_recoverable_cursor_position_error(
            "The cursor position could not be read within a normal duration"
        ));
        assert!(!super::eval::is_recoverable_cursor_position_error(
            "some other readline error"
        ));
    }

    #[test]
    fn test_filter_precheck_diags_keeps_non_unknown() {
        let expr = ctdsl::parse("from json | select name").expect("parse");
        let diags = vec![ctdsl::PrecheckDiagnostic {
            level: ctdsl::PrecheckLevel::Error,
            message: "precheck: stage `select` expects input Record, got String".to_string(),
            stage_index: 1,
            span: None,
        }];
        let mut sigs = HashMap::new();
        sigs.insert(
            "from".to_string(),
            ctsig::DataSignature::new("from", "desc"),
        );
        let out = filter_precheck_diags_for_repl(&expr, diags, &sigs, None);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn test_filter_precheck_diags_suppresses_unknown_for_path_fallback_command() {
        let expr = ctdsl::parse("java --version").expect("parse");
        let diags = vec![ctdsl::PrecheckDiagnostic {
            level: ctdsl::PrecheckLevel::Warning,
            message: "precheck: unknown command `java`; skip type-chain check".to_string(),
            stage_index: 0,
            span: None,
        }];
        let sigs = HashMap::new();
        let out = filter_precheck_diags_for_repl(&expr, diags, &sigs, None);
        assert!(out.is_empty());
    }

    #[test]
    fn test_filter_precheck_diags_suppresses_unknown_for_legacy_command() {
        let expr = ctdsl::parse("chroot --help").expect("parse");
        let diags = vec![ctdsl::PrecheckDiagnostic {
            level: ctdsl::PrecheckLevel::Warning,
            message: "precheck: unknown command `chroot`; skip type-chain check".to_string(),
            stage_index: 0,
            span: None,
        }];
        let sigs = HashMap::new();
        let out = filter_precheck_diags_for_repl_with_known_command(&expr, diags, &sigs, |name| {
            name == "chroot"
        });
        assert!(out.is_empty());
    }

    #[test]
    fn test_filter_precheck_diags_suppresses_forced_external_command() {
        let expr = ctdsl::parse("~chroot --help").expect("parse");
        let diags = vec![ctdsl::PrecheckDiagnostic {
            level: ctdsl::PrecheckLevel::Warning,
            message: "precheck: external command `chroot`; skip type-chain check".to_string(),
            stage_index: 0,
            span: None,
        }];
        let sigs = HashMap::new();
        let out = filter_precheck_diags_for_repl(&expr, diags, &sigs, None);
        assert!(out.is_empty());
    }
}
