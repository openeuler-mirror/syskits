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

//! Pager Layer - State machine and core paging logic
//!
//! This module handles:
//! - File content state (lines, current position)
//! - Window management (rows, columns)
//! - Search state (last pattern, direction, context)
//! - Action execution (NextPage, Search, etc.)
//! - EOF and file transition behavior

use regex::Regex;
use std::io::{self, Stderr, Stdout, Write};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::command::MoreAction;
use crate::render::{PromptRenderer, format_back_message, format_skip_message};
use crate::tty::TtyControl;

/// Pager options (from command-line flags)
#[derive(Clone, Debug, Default)]
pub struct PagerOptions {
    /// -d: Display help instead of bell
    pub silent: bool,

    /// -e: Exit at EOF
    pub exit_on_eof: bool,

    /// -f: Count logical lines (don't fold)
    pub logical_lines: bool,

    /// -l: Don't pause after form feed
    pub no_pause: bool,

    /// -s: Squeeze blank lines
    pub squeeze: bool,

    /// -c: Clear screen before displaying
    pub clean_print: bool,

    /// -p: Clear screen and display (print-over)
    pub print_over: bool,

    /// -u: Suppress underlining (no-op in modern terminals)
    pub plain: bool,

    /// -n <num>: Lines per screen
    pub lines_per_screen: Option<u16>,

    /// +<num>: Start from line
    pub from_line: usize,

    /// +/<pattern>: Start from pattern
    pub start_pattern: Option<String>,
}

/// Pager state machine
pub struct Pager {
    /// File content (broken into display lines)
    lines: Vec<DisplayLine>,

    /// Original content length in bytes, used for GNU-compatible percent prompts.
    content_bytes: usize,

    /// Current line (top of screen)
    current_line: usize,

    /// Terminal dimensions
    #[allow(dead_code)]
    rows: u16,
    #[allow(dead_code)]
    columns: u16,

    /// Content rows (rows - 1 for prompt)
    content_rows: u16,

    /// Scroll length for d/u (default: half page)
    scroll_len: usize,

    /// Options
    options: PagerOptions,

    /// Current file name
    current_file: Option<String>,

    /// Next file name
    next_file: Option<String>,

    /// Last search pattern
    last_search: Option<String>,

    /// Last search direction
    last_search_forward: bool,

    /// Last search context (for ' command)
    last_search_context: Option<usize>,

    /// Prompt renderer
    renderer: PromptRenderer,

    /// Number of blank lines squeezed in last draw
    lines_squeezed: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DisplayLine {
    text: String,
    byte_end: usize,
}

impl Pager {
    pub fn new(
        content: &str,
        rows: u16,
        columns: u16,
        options: PagerOptions,
        current_file: Option<String>,
        next_file: Option<String>,
    ) -> Self {
        let content_rows = rows.saturating_sub(1);
        let scroll_len = (content_rows as usize / 2).max(1);

        // Break content into display lines
        let lines = if options.logical_lines {
            break_logical_lines(content)
        } else {
            break_into_display_lines(content, columns as usize)
        };

        let mut pager = Self {
            lines,
            content_bytes: content.len(),
            current_line: options.from_line,
            rows,
            columns,
            content_rows,
            scroll_len,
            options,
            current_file,
            next_file,
            last_search: None,
            last_search_forward: true,
            last_search_context: None,
            renderer: PromptRenderer::new(),
            lines_squeezed: 0,
        };

        // Handle start pattern
        if let Some(ref pattern) = pager.options.start_pattern {
            if let Ok(re) = Regex::new(pattern) {
                if let Some(line_num) = search_forward(&pager.lines, &re, 0) {
                    pager.current_line = line_num;
                    pager.last_search = Some(pattern.clone());
                }
            }
        }

        pager
    }

    /// Execute an action and return the result
    pub fn execute_action(
        &mut self,
        action: &MoreAction,
        stdout: &mut Stdout,
        stderr: &mut Stderr,
        count: usize,
    ) -> io::Result<PagerResult> {
        match action {
            MoreAction::Continue => Ok(PagerResult::Continue),

            MoreAction::Quit => Ok(PagerResult::Quit),

            MoreAction::NextFile(n) => {
                let skip = if count > 0 { count } else { *n };
                Ok(PagerResult::NextFile(skip))
            }

            MoreAction::PrevFile(n) => {
                let skip = if count > 0 { count } else { *n };
                Ok(PagerResult::PrevFile(skip))
            }

            MoreAction::NextPage(n) => {
                let pages = if count > 0 {
                    count
                } else if *n > 0 {
                    *n
                } else {
                    1
                };
                self.advance_pages(pages);
                self.draw_screen(stdout, stderr)?;
                self.check_eof_action(stderr)
            }

            MoreAction::NextLine(n) => {
                let lines = if count > 0 {
                    count
                } else if *n > 0 {
                    *n
                } else {
                    1
                };
                self.advance_lines(lines);
                self.draw_screen(stdout, stderr)?;
                self.check_eof_action(stderr)
            }

            MoreAction::PrevPage(n) => {
                let pages = if count > 0 {
                    count
                } else if *n > 0 {
                    *n
                } else {
                    1
                };
                self.rewind_pages(pages);
                if !self.options.no_pause {
                    self.renderer
                        .draw_status_message(stderr, &format_back_message(pages))?;
                }
                self.draw_screen(stdout, stderr)?;
                Ok(PagerResult::Continue)
            }

            MoreAction::PrevLine(n) => {
                let lines = if count > 0 {
                    count
                } else if *n > 0 {
                    *n
                } else {
                    1
                };
                self.rewind_lines(lines);
                self.draw_screen(stdout, stderr)?;
                Ok(PagerResult::Continue)
            }

            MoreAction::HalfPageDown(n) => {
                let amount = if count > 0 {
                    count
                } else if *n > 0 {
                    *n
                } else {
                    self.scroll_len
                };
                self.advance_lines(amount);
                self.draw_screen(stdout, stderr)?;
                self.check_eof_action(stderr)
            }

            MoreAction::HalfPageUp(n) => {
                let amount = if count > 0 {
                    count
                } else if *n > 0 {
                    *n
                } else {
                    self.scroll_len
                };
                self.rewind_lines(amount);
                self.draw_screen(stdout, stderr)?;
                Ok(PagerResult::Continue)
            }

            MoreAction::SkipForwardLine(n) => {
                let lines = if count > 0 {
                    count
                } else if *n > 0 {
                    *n
                } else {
                    1
                };
                if !self.options.no_pause {
                    self.renderer
                        .draw_status_message(stderr, &format_skip_message(lines, "line"))?;
                }
                self.advance_lines(lines);
                self.draw_screen(stdout, stderr)?;
                self.check_eof_action(stderr)
            }

            MoreAction::SkipForwardScreen(n) => {
                let screens = if count > 0 {
                    count
                } else if *n > 0 {
                    *n
                } else {
                    1
                };
                let lines = screens * self.content_rows as usize;
                if !self.options.no_pause {
                    self.renderer
                        .draw_status_message(stderr, &format_skip_message(lines, "line"))?;
                }
                self.advance_lines(lines);
                self.draw_screen(stdout, stderr)?;
                self.check_eof_action(stderr)
            }

            MoreAction::GoTop => {
                self.current_line = 0;
                self.draw_screen(stdout, stderr)?;
                Ok(PagerResult::Continue)
            }

            MoreAction::GoBottom => {
                self.current_line = self.lines.len().saturating_sub(self.content_rows as usize);
                self.draw_screen(stdout, stderr)?;
                Ok(PagerResult::Continue)
            }

            MoreAction::Search { pattern, forward } => {
                self.execute_search(pattern, *forward, count, stderr)?;
                self.draw_screen(stdout, stderr)?;
                Ok(PagerResult::Continue)
            }

            MoreAction::RepeatSearch { forward } => {
                if let Some(ref pattern) = self.last_search.clone() {
                    let direction = if *forward {
                        self.last_search_forward
                    } else {
                        !self.last_search_forward
                    };
                    self.execute_search(pattern, direction, count, stderr)?;
                    self.draw_screen(stdout, stderr)?;
                } else {
                    self.renderer
                        .draw_status_message(stderr, "No previous regular expression")?;
                }
                Ok(PagerResult::Continue)
            }

            MoreAction::PreviousSearchMatch => {
                if let Some(context) = self.last_search_context {
                    self.renderer
                        .draw_status_message(stderr, "\n***Back***\n")?;
                    self.current_line = context;
                    self.draw_screen(stdout, stderr)?;
                } else {
                    self.renderer
                        .draw_error_prompt(stderr, self.options.silent)?;
                }
                Ok(PagerResult::Continue)
            }

            MoreAction::ShowLineInfo => {
                let line_num = self.current_line + 1;
                self.renderer
                    .draw_status_message(stderr, &format!("{line_num}"))?;
                Ok(PagerResult::Continue)
            }

            MoreAction::ShowFileInfo => {
                let line_num = self.current_line + 1;
                let msg = if let Some(ref filename) = self.current_file {
                    format!("\"{filename}\" line {line_num}")
                } else {
                    format!("[Not a file] line {line_num}")
                };
                self.renderer.draw_status_message(stderr, &msg)?;
                Ok(PagerResult::Continue)
            }

            MoreAction::ShowHelp => {
                self.show_help(stdout, stderr)?;
                self.draw_screen(stdout, stderr)?;
                Ok(PagerResult::Continue)
            }

            MoreAction::ClearScreen => {
                TtyControl::clear_screen(stderr)?;
                self.draw_screen(stdout, stderr)?;
                Ok(PagerResult::Continue)
            }

            MoreAction::SetLinesPerScreen(_n) => {
                if count > 0 {
                    self.content_rows = count as u16;
                    self.scroll_len = (count / 2).max(1);
                }
                self.advance_lines(self.content_rows as usize);
                self.draw_screen(stdout, stderr)?;
                self.check_eof_action(stderr)
            }

            MoreAction::SetScrollLen(_n) => {
                let amount = if count > 0 { count } else { self.scroll_len };
                self.scroll_len = amount;
                self.advance_lines(amount);
                self.draw_screen(stdout, stderr)?;
                self.check_eof_action(stderr)
            }

            MoreAction::RunShell(cmd) => Ok(PagerResult::RunShell(cmd.clone())),

            MoreAction::RunEditor => Ok(PagerResult::RunEditor),

            MoreAction::RepeatLast => {
                // Handled by caller
                Ok(PagerResult::Continue)
            }
        }
    }

    /// Draw the current screen and prompt without changing pager state.
    pub(crate) fn draw_current_screen(
        &mut self,
        stdout: &mut Stdout,
        stderr: &mut Stderr,
    ) -> io::Result<()> {
        self.draw_screen(stdout, stderr)
    }

    /// Draw the current screen
    fn draw_screen(&mut self, stdout: &mut Stdout, stderr: &mut Stderr) -> io::Result<()> {
        // Clear prompt
        self.renderer.clear_prompt(stderr)?;

        // Draw content lines
        self.draw_content_lines(stdout)?;

        // Draw prompt
        self.draw_prompt(stderr)?;

        stdout.flush()?;
        stderr.flush()?;
        Ok(())
    }

    /// Draw content lines
    fn draw_content_lines(&mut self, stdout: &mut Stdout) -> io::Result<()> {
        self.lines_squeezed = 0;
        let mut lines_drawn = 0;
        let mut prev_blank = false;
        let mut line_idx = self.current_line;

        while lines_drawn < self.content_rows as usize && line_idx < self.lines.len() {
            let line = &self.lines[line_idx];
            let is_blank = line.text.is_empty();

            if self.options.squeeze && is_blank && prev_blank {
                // Skip consecutive blank lines
                self.lines_squeezed += 1;
                line_idx += 1;
                continue;
            }

            write!(stdout, "{}\r\n", line.text)?;
            lines_drawn += 1;
            line_idx += 1;
            prev_blank = is_blank;
        }

        Ok(())
    }

    /// Draw the prompt line
    fn draw_prompt(&mut self, stderr: &mut Stderr) -> io::Result<()> {
        let percent = self.percent_displayed();

        let at_eof = self.is_at_eof();
        let next_file = self.next_file.as_deref();

        self.renderer
            .draw_prompt(stderr, percent, next_file, self.options.silent, at_eof)?;
        Ok(())
    }

    /// Check if at EOF and handle accordingly
    fn check_eof_action(&mut self, stderr: &mut Stderr) -> io::Result<PagerResult> {
        if self.is_at_eof() {
            if self.next_file.is_some() {
                return Ok(PagerResult::NextFile(1));
            }
            self.renderer.clear_prompt(stderr)?;
            return Ok(PagerResult::Quit);
        }
        Ok(PagerResult::Continue)
    }

    /// Check if at end of file
    fn is_at_eof(&self) -> bool {
        self.current_line + self.content_rows as usize >= self.lines.len()
    }

    fn percent_displayed(&self) -> u16 {
        if self.content_bytes == 0 {
            return 100;
        }

        let last_visible = (self.current_line + self.content_rows as usize)
            .min(self.lines.len())
            .saturating_sub(1);
        let byte_end = self
            .lines
            .get(last_visible)
            .map(|line| line.byte_end)
            .unwrap_or(0);

        ((byte_end as f64 / self.content_bytes as f64) * 100.0).round() as u16
    }

    /// Advance by N pages
    fn advance_pages(&mut self, pages: usize) {
        let amount = pages * self.content_rows as usize;
        self.advance_lines(amount);
    }

    /// Advance by N lines
    fn advance_lines(&mut self, lines: usize) {
        let max_line = self.lines.len().saturating_sub(self.content_rows as usize);
        self.current_line = (self.current_line + lines).min(max_line);
    }

    /// Rewind by N pages
    fn rewind_pages(&mut self, pages: usize) {
        let amount = pages * self.content_rows as usize;
        self.rewind_lines(amount);
    }

    /// Rewind by N lines
    fn rewind_lines(&mut self, lines: usize) {
        self.current_line = self.current_line.saturating_sub(lines);
    }

    /// Execute a search
    fn execute_search(
        &mut self,
        pattern: &str,
        forward: bool,
        count: usize,
        stderr: &mut Stderr,
    ) -> io::Result<()> {
        if pattern.is_empty() {
            self.renderer
                .draw_status_message(stderr, "Pattern not found")?;
            return Ok(());
        }

        let re = match Regex::new(pattern) {
            Ok(re) => re,
            Err(err) => {
                self.renderer
                    .draw_status_message(stderr, &err.to_string())?;
                return Ok(());
            }
        };

        // Save context
        self.last_search_context = Some(self.current_line);
        self.last_search = Some(pattern.to_string());
        self.last_search_forward = forward;

        // Search
        let start = if forward {
            self.current_line + 1
        } else {
            self.current_line.saturating_sub(1)
        };

        let occurrences = count.max(1);
        let found = if forward {
            search_forward_n(&self.lines, &re, start, occurrences)
        } else {
            search_backward_n(&self.lines, &re, start, occurrences)
        };

        match found {
            Some(line_num) => {
                let distance = line_num.abs_diff(self.current_line);
                self.current_line = line_num;
                if distance > 2 {
                    self.renderer
                        .draw_status_message(stderr, "\n...skipping\n")?;
                }
            }
            None => {
                self.renderer
                    .draw_status_message(stderr, "Pattern not found")?;
            }
        }

        Ok(())
    }

    /// Show help screen
    fn show_help(&mut self, stdout: &mut Stdout, stderr: &mut Stderr) -> io::Result<()> {
        TtyControl::clear_screen(stderr)?;

        let editor = std::env::var("VISUAL")
            .or_else(|_| std::env::var("EDITOR"))
            .unwrap_or_else(|_| "vi".to_string());

        writeln!(
            stdout,
            "Most commands optionally preceded by integer argument k.  Defaults in brackets."
        )?;
        writeln!(stdout, "Star (*) indicates argument becomes new default.")?;
        writeln!(stdout, "{}", "-".repeat(79))?;
        writeln!(
            stdout,
            "<space>                 Display next k lines of text [current screen size]"
        )?;
        writeln!(
            stdout,
            "z                       Display next k lines of text [current screen size]*"
        )?;
        writeln!(
            stdout,
            "<return>                Display next k lines of text [1]*"
        )?;
        writeln!(
            stdout,
            "d or ctrl-D             Scroll k lines [current scroll size, initially 11]*"
        )?;
        writeln!(stdout, "q or Q or <interrupt>   Exit from more")?;
        writeln!(
            stdout,
            "s                       Skip forward k lines of text [1]"
        )?;
        writeln!(
            stdout,
            "f                       Skip forward k screenfuls of text [1]"
        )?;
        writeln!(
            stdout,
            "b or ctrl-B             Skip backwards k screenfuls of text [1]"
        )?;
        writeln!(
            stdout,
            "'                       Go to place where previous search started"
        )?;
        writeln!(
            stdout,
            "=                       Display current line number"
        )?;
        writeln!(
            stdout,
            "/<regular expression>   Search for kth occurrence of regular expression [1]"
        )?;
        writeln!(
            stdout,
            "n                       Search for kth occurrence of last r.e [1]"
        )?;
        writeln!(
            stdout,
            "!<cmd> or :!<cmd>       Execute <cmd> in a subshell"
        )?;
        writeln!(
            stdout,
            "v                       Start up '{editor}' at current line"
        )?;
        writeln!(stdout, "ctrl-L                  Redraw screen")?;
        writeln!(stdout, ":n                      Go to kth next file [1]")?;
        writeln!(
            stdout,
            ":p                      Go to kth previous file [1]"
        )?;
        writeln!(
            stdout,
            ":f                      Display current file name and line number"
        )?;
        writeln!(stdout, ".                       Repeat previous command")?;
        writeln!(stdout, "{}", "-".repeat(79))?;

        stdout.flush()?;
        Ok(())
    }

    /// Handle wrong key press
    pub fn handle_wrong_key(&mut self, stderr: &mut Stderr) -> io::Result<()> {
        self.renderer.draw_error_prompt(stderr, self.options.silent)
    }

    /// Get current file name
    pub fn current_file(&self) -> Option<&str> {
        self.current_file.as_deref()
    }

    /// Get current line number
    pub fn current_line(&self) -> usize {
        self.current_line
    }

    #[cfg(test)]
    pub(crate) fn prompt_len(&self) -> usize {
        self.renderer.prompt_len()
    }
}

/// Result of pager action
#[derive(Debug, PartialEq, Eq)]
pub enum PagerResult {
    Continue,
    Quit,
    NextFile(usize),
    PrevFile(usize),
    RunShell(String),
    RunEditor,
}

fn break_logical_lines(content: &str) -> Vec<DisplayLine> {
    let mut result = Vec::new();
    let mut line_start = 0;

    for line_with_terminator in content.split_inclusive('\n') {
        let line = line_with_terminator
            .strip_suffix('\n')
            .unwrap_or(line_with_terminator);
        let line_end = line_start + line_with_terminator.len();
        result.push(DisplayLine {
            text: line.to_string(),
            byte_end: line_end,
        });
        line_start = line_end;
    }

    result
}

/// Break content into display lines based on terminal width
fn break_into_display_lines(content: &str, columns: usize) -> Vec<DisplayLine> {
    let mut result = Vec::new();
    let mut line_start = 0;

    for line_with_terminator in content.split_inclusive('\n') {
        let line = line_with_terminator
            .strip_suffix('\n')
            .unwrap_or(line_with_terminator);
        let line_end = line_start + line_with_terminator.len();
        let width = UnicodeWidthStr::width(line);
        if width <= columns {
            result.push(DisplayLine {
                text: line.to_string(),
                byte_end: line_end,
            });
        } else {
            result.extend(break_long_line(line, columns, line_start, line_end));
        }
        line_start = line_end;
    }

    result
}

/// Break a long line into multiple display lines.
fn break_long_line(
    line: &str,
    columns: usize,
    line_start: usize,
    line_end: usize,
) -> Vec<DisplayLine> {
    let mut result = Vec::new();
    let mut current_line = String::new();
    let mut current_width = 0;
    let mut current_byte_end = line_start;

    for (offset, grapheme) in UnicodeSegmentation::grapheme_indices(line, true) {
        let grapheme_width = UnicodeWidthStr::width(grapheme);

        if current_width + grapheme_width > columns && !current_line.is_empty() {
            result.push(DisplayLine {
                text: current_line,
                byte_end: current_byte_end,
            });
            current_line = String::new();
            current_width = 0;
        }

        current_line.push_str(grapheme);
        current_width += grapheme_width;
        current_byte_end = line_start + offset + grapheme.len();
    }

    if !current_line.is_empty() {
        result.push(DisplayLine {
            text: current_line,
            byte_end: line_end,
        });
    }

    result
}

/// Search forward for pattern
fn search_forward(lines: &[DisplayLine], pattern: &Regex, start: usize) -> Option<usize> {
    for (idx, line) in lines.iter().enumerate().skip(start) {
        if pattern.is_match(&line.text) {
            return Some(idx);
        }
    }
    None
}

/// Search forward for Nth occurrence
fn search_forward_n(
    lines: &[DisplayLine],
    pattern: &Regex,
    start: usize,
    n: usize,
) -> Option<usize> {
    let mut count = 0;
    for (idx, line) in lines.iter().enumerate().skip(start) {
        if pattern.is_match(&line.text) {
            count += 1;
            if count == n {
                return Some(idx);
            }
        }
    }
    None
}

/// Search backward for Nth occurrence
fn search_backward_n(
    lines: &[DisplayLine],
    pattern: &Regex,
    start: usize,
    n: usize,
) -> Option<usize> {
    let mut count = 0;
    let end = start.min(lines.len().saturating_sub(1));
    for (idx, line) in lines.iter().take(end + 1).enumerate().rev() {
        if pattern.is_match(&line.text) {
            count += 1;
            if count == n {
                return Some(idx);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_break_long_line() {
        let line = "a".repeat(100);
        let broken = break_long_line(&line, 80, 0, line.len() + 1);
        assert_eq!(broken.len(), 2);
        assert_eq!(broken[0].text.len(), 80);
        assert_eq!(broken[0].byte_end, 80);
        assert_eq!(broken[1].text.len(), 20);
        assert_eq!(broken[1].byte_end, 101);
    }

    #[test]
    fn test_search_forward() {
        let lines = vec![
            DisplayLine {
                text: "line1".into(),
                byte_end: 6,
            },
            DisplayLine {
                text: "line2".into(),
                byte_end: 12,
            },
            DisplayLine {
                text: "foo".into(),
                byte_end: 16,
            },
            DisplayLine {
                text: "line4".into(),
                byte_end: 22,
            },
        ];
        let re = Regex::new("foo").unwrap();
        assert_eq!(search_forward(&lines, &re, 0), Some(2));
        assert_eq!(search_forward(&lines, &re, 3), None);
    }

    #[test]
    fn percent_displayed_uses_source_byte_position() {
        let content = concat!(
            "adsad
", "asdsd
", "s
", "ss
", "s
", "s
", "sss
", "s
", "s
", "s
", "s
", "asdss
", "s
", "s
",
            "ss
", "s
", "s
", "s
", "ss
", "s
", "s
", "s
", "s
", "s
", "s
", "s
", "s
", "s
", "s
", "sss
", "
",
            "ss
", "s
", "s
", "s
", "s
", "s
", "s
", "ss
", "s
", "
"
        );
        let pager = Pager::new(content, 38, 80, PagerOptions::default(), None, None);

        assert_eq!(content.len(), 101);
        assert_eq!(pager.percent_displayed(), 92);
    }

    #[test]
    fn test_pager_options_default() {
        let opts = PagerOptions::default();
        assert!(!opts.silent);
        assert!(!opts.exit_on_eof);
        assert_eq!(opts.from_line, 0);
    }

    #[test]
    fn test_draw_current_screen_renders_prompt() {
        let mut pager = Pager::new(
            "line1\nline2\nline3",
            3,
            80,
            PagerOptions::default(),
            None,
            None,
        );
        let mut stdout = std::io::stdout();
        let mut stderr = std::io::stderr();

        pager.draw_current_screen(&mut stdout, &mut stderr).unwrap();

        assert!(pager.prompt_len() > 0);
    }
}
