/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2.
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2.
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

//! More command main entry point
//!
//! This module integrates all layers:
//! - TTY layer for input/output
//! - Command layer for parsing
//! - Pager layer for state management
//! - Render layer for display

use std::{
    ffi::OsString,
    fs::File,
    io::{self, BufReader, IsTerminal, Read, Write, stderr, stdin, stdout},
    panic::set_hook,
    path::Path,
};

use clap::{Arg, ArgAction, ArgMatches, Command, crate_version, value_parser};
use crossterm::{event::KeyCode, terminal};
use sys_locale::get_locale;

use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTError, CTResult, CTsageError, CtSimpleError, ExitCode};

use crate::command::{CommandParser, MoreAction};
use crate::pager::{Pager, PagerOptions, PagerResult};
use crate::tty::TtyInput;

use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");

pub mod more_options {
    pub const MORE_SILENT: &str = "silent";
    pub const MORE_LOGICAL: &str = "logical";
    pub const MORE_NO_PAUSE: &str = "no-pause";
    pub const MORE_EXIT_ON_EOF: &str = "exit-on-eof";
    pub const MORE_PRINT_OVER: &str = "print-over";
    pub const MORE_CLEAN_PRINT: &str = "clean-print";
    pub const MORE_SQUEEZE: &str = "squeeze";
    pub const MORE_PLAIN: &str = "plain";
    pub const MORE_LINES: &str = "lines";
    pub const MORE_NUMBER: &str = "number";
    pub const MORE_PATTERN: &str = "pattern";
    pub const MORE_FROM_LINE: &str = "from-line";
    pub const MORE_FILES: &str = "files";
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoreSemanticRow {
    pub kind: String,
    pub file: Option<String>,
    pub line_index: Option<usize>,
    pub text: String,
    pub source: String,
    pub terminated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoreSemantic {
    pub rows: Vec<MoreSemanticRow>,
    pub classic_text: String,
    pub stderr_text: String,
    pub exit_code: i32,
}

/// Main entry point for more command
pub fn more_main(args: impl Iterator<Item = OsString>) -> CTResult<()> {
    init_more_locale();

    // Setup panic handler
    setup_panic_handler();

    // Parse arguments
    let normalized_args = normalize_more_args(args);
    let matches = parse_arguments(normalized_args.into_iter())?;
    let options = build_pager_options(&matches);

    // Get file list
    let files: Vec<String> = matches
        .get_many::<String>(more_options::MORE_FILES)
        .map(|values| values.cloned().collect())
        .unwrap_or_default();

    let stdin_is_tty = stdin().is_terminal();
    let stdout_is_tty = stdout().is_terminal();

    // Check for invalid usage
    if stdin_is_tty && files.is_empty() {
        return Err(CTsageError::new(1, "bad usage"));
    }

    // Non-TTY mode: direct output
    if !stdout_is_tty {
        let semantic = more_non_interactive_semantic(&files, stdin_is_tty)?;
        write_more_semantic_output(&semantic)?;
        return finalize_more_semantic_exit(&semantic);
    }

    // TTY mode: interactive paging
    if !files.is_empty() {
        interactive_mode_files(files, options)
    } else {
        interactive_mode_stdin(options)
    }
}

pub fn more_native_semantic(args: impl Iterator<Item = OsString>) -> CTResult<MoreSemantic> {
    init_more_locale();

    let normalized_args = normalize_more_args(args);
    let matches = match parse_arguments_for_semantic(normalized_args) {
        Ok(matches) => matches,
        Err(semantic) => return Ok(semantic),
    };

    let files: Vec<String> = matches
        .get_many::<String>(more_options::MORE_FILES)
        .map(|values| values.cloned().collect())
        .unwrap_or_default();

    let stdin_is_tty = stdin().is_terminal();
    if stdin_is_tty && files.is_empty() {
        return Ok(more_semantic_error(
            CTsageError::new(1, "bad usage").as_ref(),
        ));
    }

    match more_non_interactive_semantic(&files, stdin_is_tty) {
        Ok(semantic) => Ok(semantic),
        Err(err) => Ok(more_semantic_error(err.as_ref())),
    }
}

fn parse_arguments_for_semantic(args: Vec<OsString>) -> Result<ArgMatches, MoreSemantic> {
    match ct_app().try_get_matches_from(args) {
        Ok(matches) => Ok(matches),
        Err(err) => {
            let rendered = err.to_string();
            if err.use_stderr() {
                Err(MoreSemantic {
                    rows: Vec::new(),
                    classic_text: String::new(),
                    stderr_text: rendered,
                    exit_code: 1,
                })
            } else {
                Err(MoreSemantic {
                    rows: Vec::new(),
                    classic_text: rendered,
                    stderr_text: String::new(),
                    exit_code: 0,
                })
            }
        }
    }
}

fn init_more_locale() {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
}

fn more_semantic_error(err: &dyn CTError) -> MoreSemantic {
    let mut stderr_text = format!("more: {err}\n");
    if err.usage() {
        stderr_text.push_str("Try 'more --help' for more information.\n");
    }

    MoreSemantic {
        rows: Vec::new(),
        classic_text: String::new(),
        stderr_text,
        exit_code: err.code(),
    }
}

fn finalize_more_semantic_exit(semantic: &MoreSemantic) -> CTResult<()> {
    if semantic.exit_code == 0 {
        Ok(())
    } else {
        Err(ExitCode::new(semantic.exit_code))
    }
}

fn write_more_semantic_output(semantic: &MoreSemantic) -> CTResult<()> {
    let mut out = stdout();
    let mut err = stderr();

    if !semantic.classic_text.is_empty() {
        out.write_all(semantic.classic_text.as_bytes())?;
    }
    if !semantic.stderr_text.is_empty() {
        err.write_all(semantic.stderr_text.as_bytes())?;
    }

    out.flush()?;
    err.flush()?;
    Ok(())
}

fn more_non_interactive_semantic(files: &[String], stdin_is_tty: bool) -> CTResult<MoreSemantic> {
    let mut semantic = MoreSemantic {
        rows: Vec::new(),
        classic_text: String::new(),
        stderr_text: String::new(),
        exit_code: 0,
    };

    if !stdin_is_tty {
        let mut stdin_buf = Vec::new();
        ctcore::ct_io::stdin_reader_box().read_to_end(&mut stdin_buf)?;
        if !stdin_buf.is_empty() {
            semantic
                .classic_text
                .push_str(&String::from_utf8_lossy(&stdin_buf));
            push_text_rows(&mut semantic.rows, "stdin_line", None, "stdout", &stdin_buf);
        }
    }

    for file in files {
        collect_file_noninteractive_semantic(file, &mut semantic)?;
    }

    Ok(semantic)
}

fn push_row(
    rows: &mut Vec<MoreSemanticRow>,
    kind: &str,
    file: Option<&str>,
    line_index: Option<usize>,
    text: String,
    source: &str,
    terminated: bool,
) {
    rows.push(MoreSemanticRow {
        kind: kind.into(),
        file: file.map(str::to_string),
        line_index,
        text,
        source: source.into(),
        terminated,
    });
}

fn push_text_rows(
    rows: &mut Vec<MoreSemanticRow>,
    kind: &str,
    file: Option<&str>,
    source: &str,
    bytes: &[u8],
) {
    let mut start = 0;
    let mut line_index = 1;

    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            push_row(
                rows,
                kind,
                file,
                Some(line_index),
                String::from_utf8_lossy(&bytes[start..index]).into_owned(),
                source,
                true,
            );
            start = index + 1;
            line_index += 1;
        }
    }

    if start < bytes.len() {
        push_row(
            rows,
            kind,
            file,
            Some(line_index),
            String::from_utf8_lossy(&bytes[start..]).into_owned(),
            source,
            false,
        );
    }
}

fn push_literal_rows(
    rows: &mut Vec<MoreSemanticRow>,
    kind: &str,
    file: Option<&str>,
    source: &str,
    text: &str,
) {
    let mut start = 0;
    let bytes = text.as_bytes();

    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            push_row(
                rows,
                kind,
                file,
                None,
                String::from_utf8_lossy(&bytes[start..index]).into_owned(),
                source,
                true,
            );
            start = index + 1;
        }
    }

    if start < bytes.len() {
        push_row(
            rows,
            kind,
            file,
            None,
            String::from_utf8_lossy(&bytes[start..]).into_owned(),
            source,
            false,
        );
    }
}

fn push_file_header_rows(rows: &mut Vec<MoreSemanticRow>, file: &str) {
    push_row(
        rows,
        "file_header_border",
        Some(file),
        None,
        "::::::::::::::".into(),
        "stdout",
        true,
    );
    push_row(
        rows,
        "file_header_name",
        Some(file),
        None,
        file.into(),
        "stdout",
        true,
    );
    push_row(
        rows,
        "file_header_border",
        Some(file),
        None,
        "::::::::::::::".into(),
        "stdout",
        true,
    );
}

/// Collect a single file in non-interactive mode.
fn collect_file_noninteractive_semantic(file: &str, semantic: &mut MoreSemantic) -> CTResult<()> {
    let path = Path::new(file);

    if path.is_dir() {
        let notice = t!("more.is_directory", file = file);
        semantic.classic_text.push_str(&notice);
        push_literal_rows(
            &mut semantic.rows,
            "directory_notice",
            Some(file),
            "stdout",
            &notice,
        );
        return Ok(());
    }

    let opened_file = match File::open(path) {
        Ok(f) => f,
        Err(err) => {
            let message = format!("more: cannot open {}: {}\n", file, os_error_message(&err));
            semantic.stderr_text.push_str(&message);
            push_literal_rows(
                &mut semantic.rows,
                "open_error",
                Some(file),
                "stderr",
                &message,
            );
            return Ok(());
        }
    };

    semantic
        .classic_text
        .push_str(&format!("::::::::::::::\n{file}\n::::::::::::::\n"));
    push_file_header_rows(&mut semantic.rows, file);

    let mut reader = BufReader::new(opened_file);
    let mut file_buf = Vec::new();
    reader.read_to_end(&mut file_buf)?;
    semantic
        .classic_text
        .push_str(&String::from_utf8_lossy(&file_buf));
    push_text_rows(
        &mut semantic.rows,
        "content_line",
        Some(file),
        "stdout",
        &file_buf,
    );
    Ok(())
}

/// Interactive mode with files
fn interactive_mode_files(files: Vec<String>, options: PagerOptions) -> CTResult<()> {
    let (cols, rows) = terminal::size()?;
    let mut tty_input = TtyInput::new();
    let mut command_parser = CommandParser::new();

    let mut file_index = 0;

    while file_index < files.len() {
        let file = &files[file_index];
        let next_file = files.get(file_index + 1).map(|s| s.as_str());

        // Read file
        let content = match read_file_content(file) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{e}");
                file_index += 1;
                continue;
            }
        };

        // Create pager
        let mut pager = Pager::new(
            &content,
            rows,
            cols,
            options.clone(),
            Some(file.clone()),
            next_file.map(|s| s.to_string()),
        );

        if files.len() == 1
            && maybe_print_single_screen_content(&content, &mut pager, &options, rows)?
        {
            return Ok(());
        }

        tty_input.enable_raw_mode()?;

        // Paging loop
        let result = paging_loop(&mut pager, &mut tty_input, &mut command_parser);
        if result.is_ok() {
            tty_input.disable_raw_mode()?;
        }

        match result? {
            LoopResult::Quit => break,
            LoopResult::NextFile(skip) => {
                file_index += skip;
            }
            LoopResult::PrevFile(skip) => {
                file_index = file_index.saturating_sub(skip);
            }
            LoopResult::Continue => {
                file_index += 1;
            }
        }
    }

    tty_input.disable_raw_mode()?;
    Ok(())
}

/// Interactive mode with stdin
fn interactive_mode_stdin(options: PagerOptions) -> CTResult<()> {
    let mut content = String::new();
    stdin().read_to_string(&mut content)?;

    if content.is_empty() {
        return Ok(());
    }

    let (cols, rows) = terminal::size()?;

    // Create pager first to handle start_pattern
    let mut pager = Pager::new(&content, rows, cols, options.clone(), None, None);

    if maybe_print_single_screen_content(&content, &mut pager, &options, rows)? {
        return Ok(());
    }

    let mut tty_input = TtyInput::new();
    tty_input.enable_raw_mode()?;
    let mut command_parser = CommandParser::new();

    let result = paging_loop(&mut pager, &mut tty_input, &mut command_parser);
    if result.is_ok() {
        tty_input.disable_raw_mode()?;
    }

    match result? {
        LoopResult::Quit
        | LoopResult::Continue
        | LoopResult::NextFile(_)
        | LoopResult::PrevFile(_) => Ok(()),
    }
}

fn maybe_print_single_screen_content(
    content: &str,
    pager: &mut Pager,
    options: &PagerOptions,
    rows: u16,
) -> CTResult<bool> {
    let content_rows = rows.saturating_sub(1);
    if content.lines().count() > content_rows as usize {
        return Ok(false);
    }

    let mut stdout = stdout();

    // If we started from a pattern match, show the skipping message
    if options.start_pattern.is_some() && pager.current_line() > 0 {
        writeln!(stdout, "\n{}", t!("more.skipping"))?;
    }

    // Display content from current line with squeeze logic
    let mut prev_blank = false;
    for line in content.lines().skip(pager.current_line()) {
        let is_blank = line.is_empty();

        if options.squeeze && is_blank && prev_blank {
            // Skip consecutive blank lines
            continue;
        }

        writeln!(stdout, "{line}")?;
        prev_blank = is_blank;
    }

    Ok(true)
}

/// Main paging loop
fn paging_loop(
    pager: &mut Pager,
    tty_input: &mut TtyInput,
    command_parser: &mut CommandParser,
) -> CTResult<LoopResult> {
    let mut stdout = stdout();
    let mut stderr = stderr();

    // Initial draw
    pager.draw_current_screen(&mut stdout, &mut stderr)?;

    loop {
        let key = tty_input.read_key()?;

        // Handle special input modes
        match key.code {
            KeyCode::Char('/') => {
                // Search mode
                if let Some(pattern) = tty_input.read_line_with_prompt(&mut stderr, '/')? {
                    let action = MoreAction::Search {
                        pattern,
                        forward: true,
                    };
                    let count = command_parser.take_number().unwrap_or(0);
                    match pager.execute_action(&action, &mut stdout, &mut stderr, count)? {
                        PagerResult::Quit => return Ok(LoopResult::Quit),
                        PagerResult::NextFile(n) => return Ok(LoopResult::NextFile(n)),
                        PagerResult::PrevFile(n) => return Ok(LoopResult::PrevFile(n)),
                        _ => {}
                    }
                }
                continue;
            }

            KeyCode::Char(':') => {
                // Colon command mode
                if let Some(cmd) = tty_input.read_line_with_prompt(&mut stderr, ':')? {
                    if let Some(action) = command_parser.parse_colon_command(&cmd) {
                        let count = command_parser.take_number().unwrap_or(0);
                        match pager.execute_action(&action, &mut stdout, &mut stderr, count)? {
                            PagerResult::Quit => return Ok(LoopResult::Quit),
                            PagerResult::NextFile(n) => return Ok(LoopResult::NextFile(n)),
                            PagerResult::PrevFile(n) => return Ok(LoopResult::PrevFile(n)),
                            PagerResult::RunShell(cmd) => {
                                run_shell_command(tty_input, &cmd)?;
                                pager.execute_action(
                                    &MoreAction::ClearScreen,
                                    &mut stdout,
                                    &mut stderr,
                                    0,
                                )?;
                            }
                            PagerResult::RunEditor => {
                                run_editor(tty_input, pager)?;
                                pager.execute_action(
                                    &MoreAction::ClearScreen,
                                    &mut stdout,
                                    &mut stderr,
                                    0,
                                )?;
                            }
                            _ => {}
                        }
                    } else {
                        pager.handle_wrong_key(&mut stderr)?;
                    }
                }
                continue;
            }

            KeyCode::Char('!') => {
                // Shell command mode
                if let Some(cmd) = tty_input.read_line_with_prompt(&mut stderr, '!')? {
                    if !cmd.is_empty() {
                        run_shell_command(tty_input, &cmd)?;
                        pager.execute_action(
                            &MoreAction::ClearScreen,
                            &mut stdout,
                            &mut stderr,
                            0,
                        )?;
                    }
                }
                continue;
            }

            _ => {}
        }

        // Parse regular key
        if let Some(action) = command_parser.parse_key(key) {
            // Handle repeat last command
            let action = if matches!(action, MoreAction::RepeatLast) {
                if let Some(last) = command_parser.last_command() {
                    last.clone()
                } else {
                    pager.handle_wrong_key(&mut stderr)?;
                    continue;
                }
            } else {
                action
            };

            let count = command_parser.peek_number().unwrap_or(0);

            match pager.execute_action(&action, &mut stdout, &mut stderr, count)? {
                PagerResult::Quit => return Ok(LoopResult::Quit),
                PagerResult::NextFile(n) => return Ok(LoopResult::NextFile(n)),
                PagerResult::PrevFile(n) => return Ok(LoopResult::PrevFile(n)),
                PagerResult::RunShell(cmd) => {
                    run_shell_command(tty_input, &cmd)?;
                    pager.execute_action(&MoreAction::ClearScreen, &mut stdout, &mut stderr, 0)?;
                }
                PagerResult::RunEditor => {
                    run_editor(tty_input, pager)?;
                    pager.execute_action(&MoreAction::ClearScreen, &mut stdout, &mut stderr, 0)?;
                }
                _ => {}
            }
        } else {
            // Wrong key
            pager.handle_wrong_key(&mut stderr)?;
        }
    }
}

/// Result of paging loop
#[allow(dead_code)]
enum LoopResult {
    Continue,
    Quit,
    NextFile(usize),
    PrevFile(usize),
}

/// Run a shell command
fn run_shell_command(tty_input: &mut TtyInput, cmd: &str) -> CTResult<()> {
    tty_input.disable_raw_mode()?;

    let _ = std::process::Command::new("sh").arg("-c").arg(cmd).status();

    tty_input.enable_raw_mode()?;
    Ok(())
}

/// Run editor at current line
fn run_editor(tty_input: &mut TtyInput, pager: &Pager) -> CTResult<()> {
    tty_input.disable_raw_mode()?;

    if let Some(file) = pager.current_file() {
        let editor = std::env::var("VISUAL")
            .or_else(|_| std::env::var("EDITOR"))
            .unwrap_or_else(|_| "vi".to_string());

        let line = pager.current_line() + 1;

        let mut command = std::process::Command::new(&editor);
        if editor.ends_with("vi") || editor.ends_with("ex") {
            command.arg("-c").arg(line.to_string()).arg(file);
        } else {
            command.arg(format!("+{line}")).arg(file);
        }

        let _ = command.status();
    }

    tty_input.enable_raw_mode()?;
    Ok(())
}

/// Read file content
fn read_file_content(file: &str) -> CTResult<String> {
    let path = Path::new(file);

    if path.is_dir() {
        return Err(CTsageError::new(
            0,
            format!("{} is a directory.", path.quote()),
        ));
    }

    if !path.exists() {
        return Err(CtSimpleError::new(
            0,
            format!("cannot open {}: No such file or directory", path.quote()),
        ));
    }

    let mut file = File::open(path).map_err(|why| {
        CtSimpleError::new(
            0,
            format!("cannot open {}: {}", path.quote(), os_error_message(&why)),
        )
    })?;

    let mut content = String::new();
    file.read_to_string(&mut content)?;
    Ok(content)
}

/// Build pager options from command-line matches
fn build_pager_options(matches: &ArgMatches) -> PagerOptions {
    let lines_per_screen = match (
        matches.get_one::<u16>(more_options::MORE_LINES).copied(),
        matches.get_one::<u16>(more_options::MORE_NUMBER).copied(),
    ) {
        (Some(n), _) if n > 0 => Some(n + 1), // +1 for prompt line
        (None, Some(n)) if n > 0 => Some(n + 1),
        _ => None,
    };

    let from_line = matches
        .get_one::<usize>(more_options::MORE_FROM_LINE)
        .copied()
        .unwrap_or(0)
        .saturating_sub(1);

    let start_pattern = matches
        .get_one::<String>(more_options::MORE_PATTERN)
        .map(|s| s.to_owned());

    PagerOptions {
        silent: matches.get_flag(more_options::MORE_SILENT),
        exit_on_eof: matches.get_flag(more_options::MORE_EXIT_ON_EOF),
        logical_lines: matches.get_flag(more_options::MORE_LOGICAL),
        no_pause: matches.get_flag(more_options::MORE_NO_PAUSE),
        squeeze: matches.get_flag(more_options::MORE_SQUEEZE),
        clean_print: matches.get_flag(more_options::MORE_CLEAN_PRINT),
        print_over: matches.get_flag(more_options::MORE_PRINT_OVER),
        plain: matches.get_flag(more_options::MORE_PLAIN),
        lines_per_screen,
        from_line,
        start_pattern,
    }
}

/// Setup panic handler
fn setup_panic_handler() {
    set_hook(Box::new(|panic_info| {
        let _ = terminal::disable_raw_mode();
        print!("\r");
        println!("{panic_info}");
    }));
}

/// Parse command-line arguments
fn parse_arguments(args: impl Iterator<Item = OsString>) -> CTResult<ArgMatches> {
    ct_app().try_get_matches_from(args).map_err(Into::into)
}

/// Normalize more-specific argument syntax
fn normalize_more_args(args: impl Iterator<Item = OsString>) -> Vec<OsString> {
    let mut normalized = Vec::new();
    let mut end_of_options = false;

    for (index, arg) in args.enumerate() {
        if index == 0 {
            normalized.push(arg);
            continue;
        }

        if end_of_options {
            normalized.push(arg);
            continue;
        }

        let arg_lossy = arg.to_string_lossy();
        if arg_lossy == "--" {
            end_of_options = true;
            normalized.push(arg);
            continue;
        }

        // -<number> -> --lines <number>
        if let Some(rest) = arg_lossy.strip_prefix('-') {
            if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
                normalized.push(OsString::from(format!("--{}", more_options::MORE_LINES)));
                normalized.push(OsString::from(rest.to_string()));
                continue;
            }
        }

        // +/<pattern> -> --pattern <pattern>
        if let Some(rest) = arg_lossy.strip_prefix("+/") {
            normalized.push(OsString::from(format!("--{}", more_options::MORE_PATTERN)));
            normalized.push(OsString::from(rest.to_string()));
            continue;
        }

        // +<number> -> --from-line <number>
        if let Some(rest) = arg_lossy.strip_prefix('+') {
            if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
                normalized.push(OsString::from(format!(
                    "--{}",
                    more_options::MORE_FROM_LINE
                )));
                normalized.push(OsString::from(rest.to_string()));
                continue;
            }
        }

        normalized.push(arg);
    }

    normalized
}

/// Build clap command
pub fn ct_app() -> Command {
    Command::new(ctcore::ct_util_name())
        .about(t!("more.about"))
        .override_usage(t!("more.usage"))
        .version(crate_version!())
        .infer_long_args(true)
        .args([
            Arg::new(more_options::MORE_PRINT_OVER)
                .short('c')
                .long(more_options::MORE_PRINT_OVER)
                .help(t!("more.clap.more_print_over"))
                .action(ArgAction::SetTrue),
            Arg::new(more_options::MORE_SILENT)
                .short('d')
                .long(more_options::MORE_SILENT)
                .help(t!("more.clap.more_silent"))
                .action(ArgAction::SetTrue),
            Arg::new(more_options::MORE_LOGICAL)
                .short('f')
                .long(more_options::MORE_LOGICAL)
                .help(t!("more.clap.more_logical"))
                .action(ArgAction::SetTrue),
            Arg::new(more_options::MORE_NO_PAUSE)
                .short('l')
                .long(more_options::MORE_NO_PAUSE)
                .help(t!("more.clap.more_no_pause"))
                .action(ArgAction::SetTrue),
            Arg::new(more_options::MORE_CLEAN_PRINT)
                .short('p')
                .long(more_options::MORE_CLEAN_PRINT)
                .help(t!("more.clap.more_clean_print"))
                .action(ArgAction::SetTrue),
            Arg::new(more_options::MORE_EXIT_ON_EOF)
                .short('e')
                .long(more_options::MORE_EXIT_ON_EOF)
                .help(t!("more.clap.more_exit_on_eof"))
                .action(ArgAction::SetTrue),
            Arg::new(more_options::MORE_SQUEEZE)
                .short('s')
                .long(more_options::MORE_SQUEEZE)
                .help(t!("more.clap.more_squeeze"))
                .action(ArgAction::SetTrue),
            Arg::new(more_options::MORE_PLAIN)
                .short('u')
                .long(more_options::MORE_PLAIN)
                .action(ArgAction::SetTrue)
                .hide(true),
            Arg::new(more_options::MORE_PATTERN)
                .short('P')
                .long(more_options::MORE_PATTERN)
                .allow_hyphen_values(true)
                .value_name("pattern")
                .help(t!("more.clap.more_pattern")),
            Arg::new(more_options::MORE_FROM_LINE)
                .short('F')
                .long(more_options::MORE_FROM_LINE)
                .value_name("number")
                .value_parser(value_parser!(usize))
                .help("Display file beginning from line number"),
            Arg::new(more_options::MORE_LINES)
                .short('n')
                .long(more_options::MORE_LINES)
                .value_name("number")
                .value_parser(value_parser!(u16).range(0..))
                .help("The number of lines per screen full"),
            Arg::new(more_options::MORE_NUMBER)
                .long(more_options::MORE_NUMBER)
                .value_parser(value_parser!(u16).range(0..))
                .help("Same as --lines"),
            Arg::new(more_options::MORE_FILES)
                .action(ArgAction::Append)
                .help(t!("more.clap.more_files"))
                .value_hint(clap::ValueHint::FilePath),
        ])
}

/// Convert OS error to message
fn os_error_message(err: &io::Error) -> String {
    use io::ErrorKind;
    match err.kind() {
        ErrorKind::NotFound => "No such file or directory".to_string(),
        ErrorKind::PermissionDenied => "Permission denied".to_string(),
        ErrorKind::AlreadyExists => "File exists".to_string(),
        ErrorKind::IsADirectory => "Is a directory".to_string(),
        ErrorKind::NotADirectory => "Not a directory".to_string(),
        ErrorKind::InvalidInput => "Invalid argument".to_string(),
        _ => err.to_string(),
    }
}

/// Tool trait implementation
#[derive(Default)]
pub struct More;

impl Tool for More {
    fn name(&self) -> &'static str {
        "more"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        more_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_normalize_args() {
        let args = vec![
            OsString::from("more"),
            OsString::from("-10"),
            OsString::from("+5"),
            OsString::from("+/pattern"),
        ];

        let normalized = normalize_more_args(args.into_iter());

        assert_eq!(normalized[0], OsString::from("more"));
        assert_eq!(normalized[1], OsString::from("--lines"));
        assert_eq!(normalized[2], OsString::from("10"));
        assert_eq!(normalized[3], OsString::from("--from-line"));
        assert_eq!(normalized[4], OsString::from("5"));
        assert_eq!(normalized[5], OsString::from("--pattern"));
        assert_eq!(normalized[6], OsString::from("pattern"));
    }

    #[test]
    fn test_normalize_args_respects_double_dash() {
        let args = vec![
            OsString::from("more"),
            OsString::from("--"),
            OsString::from("-10"),
            OsString::from("+7"),
            OsString::from("+/pattern"),
        ];

        let normalized = normalize_more_args(args.into_iter());

        assert_eq!(normalized[0], OsString::from("more"));
        assert_eq!(normalized[1], OsString::from("--"));
        assert_eq!(normalized[2], OsString::from("-10"));
        assert_eq!(normalized[3], OsString::from("+7"));
        assert_eq!(normalized[4], OsString::from("+/pattern"));
    }

    #[test]
    fn test_tool_implementation() {
        let tool = More;
        assert_eq!(tool.name(), "more");

        let cmd = tool.command();
        assert!(cmd.get_name().contains("more"));
    }

    #[test]
    fn more_native_semantic_collects_non_interactive_rows_and_metadata() {
        let temp_dir = TempDir::new().expect("tempdir");
        let path = temp_dir.path().join("sample.txt");
        fs::write(&path, "alpha\nbeta\n").expect("write more sample");
        let path = path.display().to_string();

        let semantic =
            more_native_semantic(vec![OsString::from("more"), OsString::from(&path)].into_iter())
                .expect("more semantic");

        assert_eq!(semantic.exit_code, 0);
        assert_eq!(
            semantic.classic_text,
            format!("::::::::::::::\n{path}\n::::::::::::::\nalpha\nbeta\n")
        );
        assert!(semantic.stderr_text.is_empty());
        assert!(
            semantic.rows.iter().any(|row| {
                row.kind == "file_header_name"
                    && row.file.as_deref() == Some(path.as_str())
                    && row.text == path
            }),
            "rows: {:?}",
            semantic.rows
        );
        assert!(
            semantic.rows.iter().any(|row| {
                row.kind == "content_line"
                    && row.file.as_deref() == Some(path.as_str())
                    && row.line_index == Some(1)
                    && row.text == "alpha"
                    && row.source == "stdout"
                    && row.terminated
            }),
            "rows: {:?}",
            semantic.rows
        );
    }

    #[test]
    fn more_native_semantic_preserves_missing_file_error() {
        let temp_dir = TempDir::new().expect("tempdir");
        let missing = temp_dir.path().join("missing.txt");
        let missing = missing.display().to_string();

        let semantic = more_native_semantic(
            vec![OsString::from("more"), OsString::from(&missing)].into_iter(),
        )
        .expect("more semantic missing file");

        assert_eq!(semantic.exit_code, 0);
        assert!(semantic.classic_text.is_empty());
        assert_eq!(
            semantic.stderr_text,
            format!("more: cannot open {missing}: No such file or directory\n")
        );
        let expected_text = format!("more: cannot open {missing}: No such file or directory");
        assert_eq!(
            semantic.rows,
            vec![MoreSemanticRow {
                kind: "open_error".into(),
                file: Some(missing),
                line_index: None,
                text: expected_text,
                source: "stderr".into(),
                terminated: true,
            }]
        );
    }

    #[test]
    fn more_native_semantic_preserves_clap_parse_error_text() {
        let semantic = more_native_semantic(
            vec![OsString::from("more"), OsString::from("--badflag")].into_iter(),
        )
        .expect("more semantic parse error");

        assert_eq!(semantic.exit_code, 1);
        assert!(semantic.classic_text.is_empty(), "{semantic:?}");
        assert!(
            semantic
                .stderr_text
                .contains("unexpected argument '--badflag' found"),
            "{semantic:?}"
        );
        assert!(!semantic.stderr_text.ends_with("more: \n"), "{semantic:?}");
    }
}
