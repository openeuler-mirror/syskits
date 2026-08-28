use crate::config::ReplTexts;
use crate::eval::parse_pipeline_expr;
use crossterm::{cursor::MoveTo, execute, terminal::Clear, terminal::ClearType};
use std::collections::VecDeque;
use std::env;
use std::io;
use std::path::PathBuf;

pub(crate) const REPL_HISTORY_LINES_LIMIT: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ControlAction {
    Continue,
    Exit,
    NotControl,
}

pub(crate) fn cd_target_from_input(input: &str) -> Option<Option<&str>> {
    if input == "cd" {
        return Some(None);
    }
    input.strip_prefix("cd ").map(|rest| Some(rest.trim()))
}

fn resolve_cd_target(raw: Option<&str>) -> Result<PathBuf, String> {
    let target = raw.unwrap_or("~");
    if target.is_empty() {
        return env::current_dir().map_err(|e| e.to_string());
    }
    if target == "~" {
        return env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| "HOME is not set".to_string());
    }
    if let Some(suffix) = target.strip_prefix("~/") {
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| "HOME is not set".to_string())?;
        return Ok(home.join(suffix));
    }
    Ok(PathBuf::from(target))
}

fn change_repl_directory(raw: Option<&str>) -> Result<(), String> {
    let path = resolve_cd_target(raw)?;
    env::set_current_dir(&path).map_err(|e| format!("cd: {}: {e}", path.display()))
}

pub(crate) fn handle_control_command(
    input: &str,
    debug_enabled: &mut bool,
    texts: &ReplTexts,
    history: &VecDeque<String>,
) -> ControlAction {
    if let Some(target) = cd_target_from_input(input) {
        if let Err(e) = change_repl_directory(target) {
            eprintln!("{e}");
        }
        return ControlAction::Continue;
    }

    match input {
        "exit" | "quit" => ControlAction::Exit,
        "help" => {
            println!("{}", texts.help_intro);
            println!("{}", texts.help_example);
            println!("{}", texts.help_builtins);
            ControlAction::Continue
        }
        "debug on" | "trace on" => {
            *debug_enabled = true;
            println!("{}", texts.debug_enabled);
            ControlAction::Continue
        }
        "debug off" | "trace off" => {
            *debug_enabled = false;
            println!("{}", texts.debug_disabled);
            ControlAction::Continue
        }
        "debug status" | "trace status" => {
            println!(
                "{}",
                if *debug_enabled {
                    texts.debug_status_enabled.as_str()
                } else {
                    texts.debug_status_disabled.as_str()
                }
            );
            ControlAction::Continue
        }
        "clear" => {
            let _ = execute!(io::stdout(), Clear(ClearType::All), MoveTo(0, 0));
            ControlAction::Continue
        }
        "pwd" => {
            match env::current_dir() {
                Ok(path) => println!("{}", path.display()),
                Err(e) => eprintln!("pwd: {e}"),
            }
            ControlAction::Continue
        }
        "history" => {
            if history.is_empty() {
                println!("{}", texts.history_empty);
            } else {
                for (idx, h) in history.iter().enumerate() {
                    println!("{}: {}", idx + 1, h);
                }
            }
            ControlAction::Continue
        }
        "trace" => {
            eprintln!("{}", texts.trace_usage);
            ControlAction::Continue
        }
        _ if input.starts_with("trace ") => {
            eprintln!("{}", texts.trace_usage);
            ControlAction::Continue
        }
        "ast" => {
            eprintln!("{}", texts.ast_usage);
            ControlAction::Continue
        }
        _ if input.starts_with("ast ") => {
            let expr_src = input.strip_prefix("ast ").unwrap_or_default();
            match parse_pipeline_expr(expr_src.trim()) {
                Ok(expr) => println!("{expr:#?}"),
                Err(e) => eprintln!("parse error: {e}"),
            }
            ControlAction::Continue
        }
        _ if input.starts_with(':') => {
            eprintln!("{}", texts.meta_unknown);
            ControlAction::Continue
        }
        _ => ControlAction::NotControl,
    }
}

pub(crate) fn push_history_line(history_lines: &mut VecDeque<String>, line: &str) {
    if history_lines.len() == REPL_HISTORY_LINES_LIMIT {
        let _ = history_lines.pop_front();
    }
    history_lines.push_back(line.to_string());
}
