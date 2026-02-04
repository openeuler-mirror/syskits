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

//! Render Layer - Terminal rendering and prompt formatting
//!
//! This module handles:
//! - Prompt line formatting (--More--, percentages, file info)
//! - Status messages
//! - Error prompts (with bell or help text)
//! - Reverse video attributes

use crossterm::style::Attribute;
use std::io::{self, Stderr, Write};

use crate::tty::TtyControl;

/// Prompt renderer
pub struct PromptRenderer {
    /// Length of current prompt (for erasing)
    current_prompt_len: usize,
}

impl PromptRenderer {
    pub fn new() -> Self {
        Self {
            current_prompt_len: 0,
        }
    }

    /// Draw the main prompt
    ///
    /// Format depends on state:
    /// - `--More--(Next file: filename)` if next_file is Some
    /// - `--More--(END)` if at EOF (percent == 100)
    /// - `--More--(XX%)` otherwise
    ///
    /// If silent mode, appends: `[Press space to continue, 'q' to quit.]`
    pub fn draw_prompt(
        &mut self,
        stderr: &mut Stderr,
        percent: u16,
        next_file: Option<&str>,
        silent: bool,
        at_eof: bool,
    ) -> io::Result<()> {
        // Clear previous prompt
        self.clear_prompt(stderr)?;

        let mut prompt = String::from("--More--");

        if let Some(filename) = next_file {
            prompt.push_str(&format!("(Next file: {filename})"));
        } else if at_eof || percent >= 100 {
            prompt = "(END)".to_string();
        } else if percent > 0 {
            prompt.push_str(&format!("({percent}%)"));
        }

        if silent {
            prompt.push_str("[Press space to continue, 'q' to quit.]");
        }

        self.write_prompt_with_reverse(stderr, &prompt)?;
        Ok(())
    }

    /// Draw error prompt for wrong key
    ///
    /// In silent mode (-d): shows help text
    /// Otherwise: rings bell
    pub fn draw_error_prompt(&mut self, stderr: &mut Stderr, silent: bool) -> io::Result<()> {
        if silent {
            self.clear_prompt(stderr)?;
            let msg = "[Press 'h' for instructions.]";
            self.write_prompt_with_reverse(stderr, msg)?;
        } else {
            TtyControl::bell(stderr)?;
        }
        Ok(())
    }

    /// Draw a status message (without reverse video)
    pub fn draw_status_message(&mut self, stderr: &mut Stderr, message: &str) -> io::Result<()> {
        self.clear_prompt(stderr)?;
        write!(stderr, "\r{message}")?;
        self.current_prompt_len = message.chars().count();
        stderr.flush()?;
        Ok(())
    }

    /// Draw a message with reverse video
    pub fn draw_message_with_reverse(
        &mut self,
        stderr: &mut Stderr,
        message: &str,
    ) -> io::Result<()> {
        self.clear_prompt(stderr)?;
        self.write_prompt_with_reverse(stderr, message)?;
        Ok(())
    }

    /// Clear the current prompt line
    pub fn clear_prompt(&mut self, stderr: &mut Stderr) -> io::Result<()> {
        if self.current_prompt_len > 0 {
            TtyControl::clear_current_line(stderr)?;
            self.current_prompt_len = 0;
        }
        Ok(())
    }

    /// Write prompt with reverse video attribute
    fn write_prompt_with_reverse(&mut self, stderr: &mut Stderr, prompt: &str) -> io::Result<()> {
        write!(
            stderr,
            "\r{}{}{}",
            Attribute::Reverse,
            prompt,
            Attribute::Reset
        )?;
        self.current_prompt_len = prompt.chars().count();
        stderr.flush()?;
        Ok(())
    }

    /// Get current prompt length
    pub fn prompt_len(&self) -> usize {
        self.current_prompt_len
    }
}

impl Default for PromptRenderer {
    fn default() -> Self {
        Self::new()
    }
}

/// Format a "skip" message
pub fn format_skip_message(count: usize, unit: &str) -> String {
    let label = if count == 1 {
        unit.to_string()
    } else {
        format!("{unit}s")
    };
    format!("...skipping {count} {label}")
}

/// Format a "back" message
pub fn format_back_message(pages: usize) -> String {
    let label = if pages == 1 { "page" } else { "pages" };
    format!("...back {pages} {label}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_skip_message() {
        assert_eq!(format_skip_message(1, "line"), "...skipping 1 line");
        assert_eq!(format_skip_message(5, "line"), "...skipping 5 lines");
    }

    #[test]
    fn test_format_back_message() {
        assert_eq!(format_back_message(1), "...back 1 page");
        assert_eq!(format_back_message(3), "...back 3 pages");
    }

    #[test]
    fn test_prompt_renderer_creation() {
        let renderer = PromptRenderer::new();
        assert_eq!(renderer.prompt_len(), 0);
    }
}
