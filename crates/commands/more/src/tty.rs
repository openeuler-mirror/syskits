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

//! TTY Layer - Terminal I/O and control sequences
//!
//! This module handles:
//! - Raw mode terminal setup/teardown
//! - Key event reading
//! - Line input with editing (for search/colon commands)
//! - Terminal control sequences (clear, move cursor, bell)

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    terminal,
};
use std::io::{self, Stderr, Write};

use ctcore::ct_error::CTResult;

/// TTY input handler
pub struct TtyInput {
    /// Whether raw mode is currently enabled
    raw_mode_enabled: bool,
}

impl TtyInput {
    pub fn new() -> Self {
        Self {
            raw_mode_enabled: false,
        }
    }

    /// Enable raw mode
    pub fn enable_raw_mode(&mut self) -> CTResult<()> {
        if !self.raw_mode_enabled {
            terminal::enable_raw_mode()?;
            self.raw_mode_enabled = true;
        }
        Ok(())
    }

    /// Disable raw mode
    pub fn disable_raw_mode(&mut self) -> CTResult<()> {
        if self.raw_mode_enabled {
            terminal::disable_raw_mode()?;
            self.raw_mode_enabled = false;
        }
        Ok(())
    }

    /// Read next key event (blocking)
    pub fn read_key(&self) -> CTResult<KeyEvent> {
        loop {
            if let Event::Key(key) = event::read()? {
                // Only process key press events (ignore release)
                if key.kind == KeyEventKind::Press {
                    return Ok(key);
                }
            }
        }
    }

    /// Read a line of input with prompt (for search/colon commands)
    ///
    /// Supports:
    /// - Backspace for editing
    /// - Esc to cancel (returns None)
    /// - Enter to submit
    pub fn read_line_with_prompt(
        &self,
        stderr: &mut Stderr,
        prompt_char: char,
    ) -> CTResult<Option<String>> {
        // Clear line and show prompt
        TtyControl::clear_current_line(stderr)?;
        write!(stderr, "\r{prompt_char}")?;
        stderr.flush()?;

        let mut input = String::new();

        loop {
            match event::read()? {
                Event::Key(KeyEvent {
                    code: KeyCode::Esc,
                    kind: KeyEventKind::Press,
                    ..
                }) => {
                    // Cancel - clear line and return None
                    TtyControl::clear_current_line(stderr)?;
                    stderr.flush()?;
                    return Ok(None);
                }

                Event::Key(KeyEvent {
                    code: KeyCode::Enter,
                    kind: KeyEventKind::Press,
                    ..
                }) => {
                    // Submit
                    break;
                }

                Event::Key(KeyEvent {
                    code: KeyCode::Backspace,
                    kind: KeyEventKind::Press,
                    ..
                }) => {
                    // Delete last character
                    if input.pop().is_some() {
                        // Redraw prompt and input
                        TtyControl::clear_current_line(stderr)?;
                        write!(stderr, "\r{prompt_char}{input}")?;
                        stderr.flush()?;
                    }
                }

                Event::Key(KeyEvent {
                    code: KeyCode::Char(ch),
                    modifiers: KeyModifiers::NONE | KeyModifiers::SHIFT,
                    kind: KeyEventKind::Press,
                    ..
                }) => {
                    // Add character
                    input.push(ch);
                    write!(stderr, "{ch}")?;
                    stderr.flush()?;
                }

                _ => {}
            }
        }

        let result = input.trim_end().to_string();
        Ok(Some(result))
    }
}

impl Default for TtyInput {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TtyInput {
    fn drop(&mut self) {
        let _ = self.disable_raw_mode();
    }
}

/// Terminal control sequences
pub struct TtyControl;

impl TtyControl {
    /// Clear the current line
    pub fn clear_current_line(stderr: &mut Stderr) -> io::Result<()> {
        use crossterm::{execute, terminal::Clear, terminal::ClearType};
        execute!(stderr, Clear(ClearType::CurrentLine))?;
        write!(stderr, "\r")?;
        Ok(())
    }

    /// Clear entire screen and move to home
    pub fn clear_screen(stderr: &mut Stderr) -> io::Result<()> {
        use crossterm::{cursor::MoveTo, execute, terminal::Clear, terminal::ClearType};
        execute!(stderr, Clear(ClearType::All), MoveTo(0, 0))?;
        Ok(())
    }

    /// Ring the terminal bell
    pub fn bell(stderr: &mut Stderr) -> io::Result<()> {
        write!(stderr, "\x07")?;
        stderr.flush()?;
        Ok(())
    }

    /// Move cursor to beginning of line without newline
    pub fn move_to_line_start(stderr: &mut Stderr) -> io::Result<()> {
        write!(stderr, "\r")?;
        Ok(())
    }

    /// Erase from cursor to end of line
    pub fn erase_to_eol(stderr: &mut Stderr) -> io::Result<()> {
        use crossterm::{execute, terminal::Clear, terminal::ClearType};
        execute!(stderr, Clear(ClearType::UntilNewLine))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tty_input_creation() {
        let input = TtyInput::new();
        assert!(!input.raw_mode_enabled);
    }

    #[test]
    fn test_tty_control_sequences() {
        // These are just smoke tests to ensure the functions compile
        // Real testing would require a PTY
        let mut stderr = io::stderr();

        // Should not panic
        let _ = TtyControl::clear_current_line(&mut stderr);
        let _ = TtyControl::move_to_line_start(&mut stderr);
    }
}
