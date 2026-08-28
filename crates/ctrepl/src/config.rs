use rust_i18n::t;
use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReplTexts {
    pub(crate) banner: String,
    pub(crate) prompt_left: String,
    pub(crate) prompt_right: String,
    pub(crate) exit_hint: String,
    pub(crate) help_intro: String,
    pub(crate) help_example: String,
    pub(crate) help_builtins: String,
    pub(crate) debug_enabled: String,
    pub(crate) debug_disabled: String,
    pub(crate) debug_status_enabled: String,
    pub(crate) debug_status_disabled: String,
    pub(crate) meta_unknown: String,
    pub(crate) history_empty: String,
    pub(crate) history_io_warning: String,
    pub(crate) precheck_error_prefix: String,
    pub(crate) precheck_warning_prefix: String,
    pub(crate) trace_usage: String,
    pub(crate) ast_usage: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReplConfig {
    pub(crate) persist_history: bool,
    pub(crate) history_file: Option<PathBuf>,
}

pub(crate) fn texts_for_current_locale() -> ReplTexts {
    ReplTexts {
        banner: t!("repl.banner").to_string(),
        prompt_left: t!("repl.prompt_left").to_string(),
        prompt_right: t!("repl.prompt_right").to_string(),
        exit_hint: t!("repl.exit_hint").to_string(),
        help_intro: t!("repl.help_intro").to_string(),
        help_example: t!("repl.help_example").to_string(),
        help_builtins: t!("repl.help_builtins").to_string(),
        debug_enabled: t!("repl.debug_enabled").to_string(),
        debug_disabled: t!("repl.debug_disabled").to_string(),
        debug_status_enabled: t!("repl.debug_status_enabled").to_string(),
        debug_status_disabled: t!("repl.debug_status_disabled").to_string(),
        meta_unknown: t!("repl.meta_unknown").to_string(),
        history_empty: t!("repl.history_empty").to_string(),
        history_io_warning: t!("repl.history_io_warning").to_string(),
        precheck_error_prefix: t!("repl.precheck_error_prefix").to_string(),
        precheck_warning_prefix: t!("repl.precheck_warning_prefix").to_string(),
        trace_usage: t!("repl.trace_usage").to_string(),
        ast_usage: t!("repl.ast_usage").to_string(),
    }
}

fn default_history_file_path() -> Option<PathBuf> {
    let home = env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join(".config")
            .join("syskits")
            .join("repl")
            .join("history"),
    )
}

pub(crate) fn parse_history_persistence(raw: Option<&str>) -> bool {
    let Some(v) = raw else {
        return true;
    };
    !matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "0" | "off" | "false" | "no" | "disable" | "disabled" | "memory"
    )
}

pub(crate) fn repl_config_from_env() -> ReplConfig {
    let persist_history =
        parse_history_persistence(std::env::var("SYSKITS_REPL_HISTORY").ok().as_deref());

    let history_file = if persist_history {
        std::env::var("SYSKITS_REPL_HISTORY_FILE")
            .ok()
            .and_then(|v| {
                let t = v.trim();
                if t.is_empty() {
                    None
                } else {
                    Some(PathBuf::from(t))
                }
            })
            .or_else(default_history_file_path)
    } else {
        None
    };

    ReplConfig {
        persist_history,
        history_file,
    }
}

pub(crate) fn prompt_path_depth_from_env() -> usize {
    env::var("SYSKITS_REPL_PROMPT_PATH_DEPTH")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(3)
}
