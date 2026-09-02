use crate::config::{ReplConfig, ReplTexts};
use crate::eval::resolve_repl_line_format;
use ctsig::DataSignature;
use nu_ansi_term::{Color, Style};
use reedline::{
    ColumnarMenu, Completer, DefaultHinter, Emacs, FileBackedHistory, Highlighter, KeyCode,
    KeyModifiers, MenuBuilder, Prompt, PromptEditMode, PromptHistorySearch, Reedline,
    ReedlineEvent, ReedlineMenu, Span, StyledText, Suggestion, ValidationResult, Validator,
    default_emacs_keybindings,
};
use std::borrow::Cow;
use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub(crate) struct ReplPrompt {
    pub(crate) prompt_left: String,
    pub(crate) path_depth: usize,
}

impl Prompt for ReplPrompt {
    fn render_prompt_left(&self) -> Cow<str> {
        let cwd = env::current_dir()
            .ok()
            .map(|p| format_prompt_path(&p, self.path_depth))
            .unwrap_or_else(|| ".".to_string());
        Cow::Owned(format!("{}({cwd})", self.prompt_left))
    }

    fn render_prompt_right(&self) -> Cow<str> {
        Cow::Borrowed("")
    }

    fn render_prompt_indicator(&self, _prompt_mode: PromptEditMode) -> Cow<str> {
        Cow::Borrowed("〉 ")
    }

    fn render_prompt_multiline_indicator(&self) -> Cow<str> {
        Cow::Borrowed("… ")
    }

    fn render_prompt_history_search_indicator(
        &self,
        _history_search: PromptHistorySearch,
    ) -> Cow<str> {
        Cow::Borrowed("(search) ")
    }
}

#[derive(Debug, Clone)]
struct ReplHighlighter {
    signatures: Arc<HashMap<String, DataSignature>>,
}

impl Highlighter for ReplHighlighter {
    fn highlight(&self, line: &str, _cursor: usize) -> StyledText {
        let mut out = StyledText::new();
        let base = Style::new();
        out.push((base, line.to_string()));

        let resolved = match resolve_repl_line_format(line) {
            Ok(resolved) => resolved,
            Err(_) => return out,
        };
        let trimmed = resolved.expr_src.trim();
        if trimmed.is_empty() {
            return out;
        }
        let expr_offset = trimmed.as_ptr() as usize - line.as_ptr() as usize;

        match ctdsl::parse(trimmed) {
            Ok(expr) => {
                let diags = ctdsl::precheck_expr(&expr, &self.signatures);
                if let Some(err) = diags
                    .iter()
                    .find(|d| matches!(d.level, ctdsl::PrecheckLevel::Error))
                {
                    if let Some(span) = &err.span {
                        out.style_range(
                            (expr_offset + span.start).min(line.len()),
                            (expr_offset + span.end).min(line.len()),
                            Style::new().fg(Color::Red),
                        );
                    } else {
                        out.style_range(0, line.len(), Style::new().fg(Color::Red));
                    }
                } else if diags
                    .iter()
                    .any(|d| matches!(d.level, ctdsl::PrecheckLevel::Warning))
                {
                    out.style_range(0, line.len(), Style::new().fg(Color::Yellow));
                }
                out
            }
            Err(ctdsl::ParseError::LexError { span, .. })
            | Err(ctdsl::ParseError::SyntaxError { span, .. }) => {
                out.style_range(
                    (expr_offset + span.start).min(line.len()),
                    (expr_offset + span.end).min(line.len()),
                    Style::new().fg(Color::Red),
                );
                out
            }
            Err(ctdsl::ParseError::UnexpectedEof) => {
                out.style_range(expr_offset, line.len(), Style::new().fg(Color::Yellow));
                out
            }
        }
    }
}

#[derive(Debug, Default)]
struct ReplValidator;

impl Validator for ReplValidator {
    fn validate(&self, line: &str) -> ValidationResult {
        if line_is_incomplete(line) {
            ValidationResult::Incomplete
        } else {
            ValidationResult::Complete
        }
    }
}

#[derive(Debug, Clone)]
struct ReplCompleter {
    commands: Vec<CompletionCandidate>,
    flags: Vec<CompletionCandidate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompletionKind {
    Command,
    Flag,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompletionCandidate {
    pub(crate) value: String,
    pub(crate) description: Option<String>,
    pub(crate) kind: CompletionKind,
}

impl ReplCompleter {
    fn new(items: Vec<CompletionCandidate>) -> Self {
        let mut commands = Vec::new();
        let mut flags = Vec::new();
        for item in items {
            match item.kind {
                CompletionKind::Command => commands.push(item),
                CompletionKind::Flag => flags.push(item),
            }
        }

        commands.sort_by(|a, b| a.value.cmp(&b.value));
        flags.sort_by(|a, b| a.value.cmp(&b.value));

        Self { commands, flags }
    }

    fn complete_inner(&self, line: &str, pos: usize) -> Vec<Suggestion> {
        let before_cursor = if line.len() > pos { &line[..pos] } else { line };
        let token_start = completion_token_start(before_cursor);
        let prefix = &before_cursor[token_start..];

        let candidates = if prefix.starts_with('-') {
            &self.flags
        } else if is_command_position(before_cursor, token_start) {
            &self.commands
        } else {
            return filesystem_suggestions(prefix, token_start, pos);
        };
        let line_prefix_position = before_cursor[..token_start].trim().is_empty();

        candidates
            .iter()
            .filter(|item| {
                if item.value.starts_with("format=") && !line_prefix_position {
                    return false;
                }
                prefix.is_empty() || item.value.starts_with(prefix)
            })
            .map(|item| Suggestion {
                value: item.value.clone(),
                description: item.description.clone(),
                style: None,
                extra: None,
                span: Span::new(token_start, pos),
                append_whitespace: true,
            })
            .collect()
    }
}

fn filesystem_suggestions(prefix: &str, token_start: usize, pos: usize) -> Vec<Suggestion> {
    let (dir_part, name_prefix) = prefix
        .rsplit_once('/')
        .map(|(dir, name)| (Some(dir), name))
        .unwrap_or((None, prefix));

    let base_dir = resolve_base_dir(dir_part);
    let Ok(entries) = std::fs::read_dir(base_dir) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with(name_prefix) {
            continue;
        }

        let mut value = match dir_part {
            Some(dir) if !dir.is_empty() => format!("{dir}/{name}"),
            Some(_) => format!("/{name}"),
            None => name,
        };
        let append_whitespace = if file_type.is_dir() {
            value.push('/');
            false
        } else {
            true
        };
        out.push(Suggestion {
            value,
            description: None,
            style: None,
            extra: None,
            span: Span::new(token_start, pos),
            append_whitespace,
        });
    }
    out.sort_by(|a, b| a.value.cmp(&b.value));
    out
}

fn resolve_base_dir(dir_part: Option<&str>) -> PathBuf {
    match dir_part {
        Some("") => PathBuf::from("/"),
        Some(dir) => {
            let path = PathBuf::from(dir);
            if path.is_absolute() {
                path
            } else {
                env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(path)
            }
        }
        None => env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    }
}

impl Completer for ReplCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> Vec<Suggestion> {
        self.complete_inner(line, pos)
    }
}

fn completion_token_start(input: &str) -> usize {
    let mut start = 0;
    for (idx, ch) in input.char_indices() {
        if ch.is_whitespace() || ch == '|' {
            start = idx + ch.len_utf8();
        }
    }
    start
}

fn is_command_position(before_cursor: &str, token_start: usize) -> bool {
    if token_start == 0 {
        return true;
    }
    before_cursor[..token_start]
        .chars()
        .rev()
        .find(|ch| !ch.is_whitespace())
        .is_none_or(|ch| ch == '|')
}

pub(crate) fn format_prompt_path(path: &Path, depth: usize) -> String {
    if depth == 0 {
        return path.to_string_lossy().into_owned();
    }
    let parts: Vec<String> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    if parts.is_empty() {
        return ".".to_string();
    }
    if parts.len() <= depth {
        return path.to_string_lossy().into_owned();
    }
    let tail = parts[parts.len() - depth..].join("/");
    format!(".../{tail}")
}

pub(crate) fn line_is_incomplete(line: &str) -> bool {
    let Ok(resolved) = resolve_repl_line_format(line) else {
        return false;
    };
    let trimmed = resolved.expr_src.trim_end();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.ends_with('|') {
        return true;
    }

    let mut double_quote_open = false;
    let mut single_quote_open = false;
    let mut escaped = false;
    for ch in line.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' && !single_quote_open {
            double_quote_open = !double_quote_open;
        } else if ch == '\'' && !double_quote_open {
            single_quote_open = !single_quote_open;
        }
    }
    if double_quote_open || single_quote_open {
        return true;
    }

    matches!(ctdsl::parse(trimmed), Err(ctdsl::ParseError::UnexpectedEof))
}

pub(crate) fn build_reedline_editor(
    completion_candidates: Vec<CompletionCandidate>,
    texts: &ReplTexts,
    signatures: Arc<HashMap<String, DataSignature>>,
    config: &ReplConfig,
) -> Reedline {
    let mut editor = Reedline::create();

    if config.persist_history
        && let Some(path) = &config.history_file
    {
        if let Some(parent) = path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            eprintln!("{}: {}", texts.history_io_warning, e);
        }
        if let Ok(history) = FileBackedHistory::with_file(500, path.clone()) {
            editor = editor.with_history(Box::new(history));
        } else {
            eprintln!("{}", texts.history_io_warning);
        }
    }

    let completer = Box::new(ReplCompleter::new(completion_candidates));
    let completion_menu = Box::new(ColumnarMenu::default().with_name("completion_menu"));
    let mut keybindings = default_emacs_keybindings();
    keybindings.add_binding(
        KeyModifiers::NONE,
        KeyCode::Tab,
        ReedlineEvent::UntilFound(vec![
            ReedlineEvent::HistoryHintComplete,
            ReedlineEvent::Menu("completion_menu".to_string()),
            ReedlineEvent::MenuNext,
        ]),
    );
    let edit_mode = Box::new(Emacs::new(keybindings));
    editor
        .with_hinter(Box::new(DefaultHinter::default()))
        .with_validator(Box::new(ReplValidator))
        .with_highlighter(Box::new(ReplHighlighter { signatures }))
        .with_menu(ReedlineMenu::EngineCompleter(completion_menu))
        .with_edit_mode(edit_mode)
        .with_completer(completer)
}

fn command_candidate(value: &str, description: &str) -> CompletionCandidate {
    CompletionCandidate {
        value: value.to_string(),
        description: Some(description.to_string()),
        kind: CompletionKind::Command,
    }
}

pub(crate) fn build_completion_candidates(
    signatures: &HashMap<String, DataSignature>,
) -> Vec<CompletionCandidate> {
    let mut out: Vec<CompletionCandidate> = Vec::new();

    let mut names: Vec<String> = signatures.keys().cloned().collect();
    names.sort();
    for name in names {
        if let Some(sig) = signatures.get(&name) {
            out.push(command_candidate(&name, sig.desc));

            for flag in &sig.flags {
                out.push(CompletionCandidate {
                    value: format!("--{}", flag.long),
                    description: Some(flag.desc.to_string()),
                    kind: CompletionKind::Flag,
                });
                if let Some(short) = flag.short {
                    out.push(CompletionCandidate {
                        value: format!("-{short}"),
                        description: Some(flag.desc.to_string()),
                        kind: CompletionKind::Flag,
                    });
                }
            }
        }
    }

    out.extend([
        command_candidate(
            "format=auto",
            "render this expression with automatic output",
        ),
        command_candidate("format=text", "render this expression as text"),
        command_candidate("format=table", "render this expression as a table"),
        command_candidate("format=json", "render this expression as JSON"),
        command_candidate("format=classic", "render this expression as classic stdout"),
        command_candidate("help", "show REPL help"),
        command_candidate("exit", "exit REPL"),
        command_candidate("quit", "exit REPL"),
        command_candidate("debug", "debug tracing: on/off/status"),
        command_candidate("trace", "trace output: on/off/status"),
        command_candidate("history", "show REPL history"),
        command_candidate("ast", "print AST for expression"),
        command_candidate("cd", "change current directory"),
        command_candidate("clear", "clear terminal screen"),
    ]);

    out.sort_by(|a, b| a.value.cmp(&b.value));
    out.dedup_by(|a, b| a.value == b.value);
    out
}

#[cfg(test)]
mod tests {
    use super::{
        CompletionCandidate, CompletionKind, ReplCompleter, build_completion_candidates,
        completion_token_start, is_command_position,
    };
    use ctsig::DataSignature;
    use reedline::Completer;
    use std::collections::HashMap;

    fn completer() -> ReplCompleter {
        ReplCompleter::new(vec![
            CompletionCandidate {
                value: "from".to_string(),
                description: Some("from desc".to_string()),
                kind: CompletionKind::Command,
            },
            CompletionCandidate {
                value: "select".to_string(),
                description: Some("select desc".to_string()),
                kind: CompletionKind::Command,
            },
            CompletionCandidate {
                value: "help".to_string(),
                description: Some("help desc".to_string()),
                kind: CompletionKind::Command,
            },
            CompletionCandidate {
                value: "format=table".to_string(),
                description: Some("format table desc".to_string()),
                kind: CompletionKind::Command,
            },
            CompletionCandidate {
                value: "--format".to_string(),
                description: Some("format desc".to_string()),
                kind: CompletionKind::Flag,
            },
            CompletionCandidate {
                value: "-f".to_string(),
                description: Some("format desc".to_string()),
                kind: CompletionKind::Flag,
            },
        ])
    }

    #[test]
    fn test_completion_token_start_tracks_last_separator() {
        assert_eq!(completion_token_start("from json | sel"), 12);
        assert_eq!(completion_token_start("from "), 5);
        assert_eq!(completion_token_start(""), 0);
    }

    #[test]
    fn test_is_command_position_at_line_start_or_after_pipe() {
        assert!(is_command_position("", 0));
        assert!(is_command_position("from json | ", 12));
        assert!(!is_command_position("from json ", 10));
    }

    #[test]
    fn test_completer_empty_line_suggests_commands() {
        let mut c = completer();
        let got = c.complete("", 0);
        assert!(got.iter().any(|s| s.value == "from"));
        assert!(got.iter().any(|s| s.value == "select"));
        assert!(got.iter().any(|s| s.value == "help"));
        assert!(got.iter().any(|s| s.value == "format=table"));
        assert!(!got.iter().any(|s| s.value == "--format"));
    }

    #[test]
    fn test_completer_after_pipe_suggests_commands() {
        let mut c = completer();
        let line = "from json | ";
        let got = c.complete(line, line.len());
        assert!(got.iter().any(|s| s.value == "from"));
        assert!(got.iter().any(|s| s.value == "select"));
        assert!(!got.iter().any(|s| s.value == "format=table"));
    }

    #[test]
    fn test_completer_argument_position_without_dash_suggests_files() {
        let mut c = completer();
        let old = std::env::current_dir().expect("current dir");
        let dir = std::env::temp_dir().join(format!(
            "ctrepl_complete_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time ok")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("demo.txt"), "x").expect("write");
        std::env::set_current_dir(&dir).expect("chdir temp");

        let line = "from ";
        let got = c.complete(line, line.len());

        std::env::set_current_dir(old).expect("restore cwd");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(got.iter().any(|s| s.value == "demo.txt"));
    }

    #[test]
    fn test_completer_dash_prefix_suggests_flags() {
        let mut c = completer();
        let line = "from --f";
        let got = c.complete(line, line.len());
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].value, "--format");
    }

    #[test]
    fn test_completer_preserves_description_for_menu_rendering() {
        let mut c = completer();
        let got = c.complete("", 0);
        let from = got
            .iter()
            .find(|s| s.value == "from")
            .expect("from suggestion exists");
        assert_eq!(from.description.as_deref(), Some("from desc"));
    }

    #[test]
    fn test_build_completion_candidates_includes_builtin_without_colon() {
        let mut signatures = HashMap::new();
        signatures.insert(
            "from".to_string(),
            DataSignature::new("from", "convert input"),
        );
        let values: Vec<String> = build_completion_candidates(&signatures)
            .into_iter()
            .map(|c| c.value)
            .collect();
        assert!(values.contains(&"history".to_string()));
        assert!(values.contains(&"trace".to_string()));
        assert!(values.contains(&"ast".to_string()));
        assert!(!values.contains(&":history".to_string()));
        assert!(!values.contains(&":trace".to_string()));
        assert!(!values.contains(&":ast".to_string()));
    }
}
