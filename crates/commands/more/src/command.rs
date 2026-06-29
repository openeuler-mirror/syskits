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

//! Command Layer - Parse input into semantic actions
//!
//! This module handles:
//! - Mapping KeyEvent to MoreAction
//! - Managing numeric prefixes
//! - Parsing colon commands (:n, :p, :f, :q)
//! - Parsing search patterns (/pattern)
//! - Command repetition (.)

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Semantic actions for the pager
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MoreAction {
    /// Continue paging (no-op)
    Continue,

    /// Quit the pager
    Quit,

    /// Move to next file (with optional skip count)
    NextFile(usize),

    /// Move to previous file (with optional skip count)
    PrevFile(usize),

    /// Advance by N lines (default: screen size)
    NextPage(usize),

    /// Advance by N lines (default: 1)
    NextLine(usize),

    /// Go back by N pages
    PrevPage(usize),

    /// Go back by N lines
    PrevLine(usize),

    /// Half page down (Ctrl-D)
    HalfPageDown(usize),

    /// Half page up (Ctrl-U)
    HalfPageUp(usize),

    /// Skip forward N lines (with message)
    SkipForwardLine(usize),

    /// Skip forward N screens (with message)
    SkipForwardScreen(usize),

    /// Go to top of file
    GoTop,

    /// Go to bottom of file
    GoBottom,

    /// Search for pattern (forward/backward)
    Search { pattern: String, forward: bool },

    /// Repeat last search (with optional direction override)
    RepeatSearch { forward: bool },

    /// Go to previous search context
    PreviousSearchMatch,

    /// Show current line number
    ShowLineInfo,

    /// Show file name and line number
    ShowFileInfo,

    /// Show help screen
    ShowHelp,

    /// Clear screen and redraw
    ClearScreen,

    /// Set lines per screen
    SetLinesPerScreen(usize),

    /// Set scroll length (for d/u)
    SetScrollLen(usize),

    /// Run shell command
    RunShell(String),

    /// Run editor at current line
    RunEditor,

    /// Repeat last command
    RepeatLast,
}

/// Command parser state
pub struct CommandParser {
    /// Pending numeric prefix
    pending_number: Option<usize>,

    /// Last executed command (for . repetition)
    last_command: Option<MoreAction>,
}

impl CommandParser {
    pub fn new() -> Self {
        Self {
            pending_number: None,
            last_command: None,
        }
    }

    /// Add a digit to the pending number
    pub fn add_digit(&mut self, digit: u32) {
        let current = self.pending_number.unwrap_or(0);
        self.pending_number = Some(current.saturating_mul(10).saturating_add(digit as usize));
    }

    /// Get and clear the pending number
    pub fn take_number(&mut self) -> Option<usize> {
        self.pending_number.take()
    }

    /// Get the pending number without clearing
    pub fn peek_number(&self) -> Option<usize> {
        self.pending_number
    }

    /// Clear the pending number
    pub fn clear_number(&mut self) {
        self.pending_number = None;
    }

    /// Parse a key event into an action
    pub fn parse_key(&mut self, key: KeyEvent) -> Option<MoreAction> {
        // Handle digit input for numeric prefix
        if let KeyEvent {
            code: KeyCode::Char(ch),
            modifiers: KeyModifiers::NONE,
            ..
        } = key
        {
            if let Some(digit) = ch.to_digit(10) {
                self.add_digit(digit);
                return None; // Continue accumulating
            }
        }

        let count = self.take_number().unwrap_or(0);

        let action = match key {
            // Quit
            KeyEvent {
                code: KeyCode::Char('q') | KeyCode::Char('Q'),
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('c'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => Some(MoreAction::Quit),

            // Navigation - forward
            KeyEvent {
                code: KeyCode::Char(' ') | KeyCode::Char('f'),
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('f'),
                modifiers: KeyModifiers::CONTROL,
                ..
            }
            | KeyEvent {
                code: KeyCode::Down | KeyCode::PageDown,
                modifiers: KeyModifiers::NONE,
                ..
            } => Some(MoreAction::NextPage(count)),

            KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('j'),
                modifiers: KeyModifiers::NONE,
                ..
            } => Some(MoreAction::NextLine(count)),

            // Navigation - backward
            KeyEvent {
                code: KeyCode::Char('b'),
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('b'),
                modifiers: KeyModifiers::CONTROL,
                ..
            }
            | KeyEvent {
                code: KeyCode::Up | KeyCode::PageUp,
                modifiers: KeyModifiers::NONE,
                ..
            } => Some(MoreAction::PrevPage(count)),

            KeyEvent {
                code: KeyCode::Char('k'),
                modifiers: KeyModifiers::NONE,
                ..
            } => Some(MoreAction::PrevLine(count)),

            // Half page
            KeyEvent {
                code: KeyCode::Char('d'),
                modifiers: KeyModifiers::NONE | KeyModifiers::CONTROL,
                ..
            } => Some(MoreAction::HalfPageDown(count)),

            KeyEvent {
                code: KeyCode::Char('u'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => Some(MoreAction::HalfPageUp(count)),

            // Skip
            KeyEvent {
                code: KeyCode::Char('s'),
                modifiers: KeyModifiers::NONE,
                ..
            } => Some(MoreAction::SkipForwardLine(count)),

            // Go to top/bottom
            KeyEvent {
                code: KeyCode::Char('g'),
                modifiers: KeyModifiers::NONE,
                ..
            } => Some(MoreAction::GoTop),

            KeyEvent {
                code: KeyCode::Char('G'),
                modifiers: KeyModifiers::NONE,
                ..
            } => Some(MoreAction::GoBottom),

            // Search
            KeyEvent {
                code: KeyCode::Char('n'),
                modifiers: KeyModifiers::NONE,
                ..
            } => Some(MoreAction::RepeatSearch { forward: true }),

            KeyEvent {
                code: KeyCode::Char('N'),
                modifiers: KeyModifiers::NONE,
                ..
            } => Some(MoreAction::RepeatSearch { forward: false }),

            KeyEvent {
                code: KeyCode::Char('\''),
                modifiers: KeyModifiers::NONE,
                ..
            } => Some(MoreAction::PreviousSearchMatch),

            // Info
            KeyEvent {
                code: KeyCode::Char('='),
                modifiers: KeyModifiers::NONE,
                ..
            } => Some(MoreAction::ShowLineInfo),

            // Help
            KeyEvent {
                code: KeyCode::Char('h') | KeyCode::Char('?'),
                modifiers: KeyModifiers::NONE,
                ..
            } => Some(MoreAction::ShowHelp),

            // Clear screen
            KeyEvent {
                code: KeyCode::Char('l'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => Some(MoreAction::ClearScreen),

            // Set lines per screen
            KeyEvent {
                code: KeyCode::Char('z'),
                modifiers: KeyModifiers::NONE,
                ..
            } => Some(MoreAction::SetLinesPerScreen(count)),

            // Repeat last command
            KeyEvent {
                code: KeyCode::Char('.'),
                modifiers: KeyModifiers::NONE,
                ..
            } => Some(MoreAction::RepeatLast),

            // Editor
            KeyEvent {
                code: KeyCode::Char('v'),
                modifiers: KeyModifiers::NONE,
                ..
            } => Some(MoreAction::RunEditor),

            _ => None,
        };

        // Save command for repetition (except Continue and RepeatLast)
        if let Some(ref act) = action {
            if !matches!(act, MoreAction::Continue | MoreAction::RepeatLast) {
                self.last_command = Some(act.clone());
            }
        }

        action
    }

    /// Get the last command for repetition
    pub fn last_command(&self) -> Option<&MoreAction> {
        self.last_command.as_ref()
    }

    /// Parse a colon command string
    pub fn parse_colon_command(&self, cmd: &str) -> Option<MoreAction> {
        let trimmed = cmd.trim();

        // Remove leading colon if present
        let trimmed = trimmed.strip_prefix(':').unwrap_or(trimmed);

        // Handle shell command
        if let Some(shell_cmd) = trimmed.strip_prefix('!') {
            let shell_cmd = shell_cmd.trim();
            if shell_cmd.is_empty() {
                return None;
            }
            return Some(MoreAction::RunShell(shell_cmd.to_string()));
        }

        // Handle other colon commands
        match trimmed.chars().next()? {
            'n' => Some(MoreAction::NextFile(1)),
            'p' => Some(MoreAction::PrevFile(1)),
            'f' => Some(MoreAction::ShowFileInfo),
            'q' | 'Q' => Some(MoreAction::Quit),
            _ => None,
        }
    }
}

impl Default for CommandParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_numeric_prefix() {
        let mut parser = CommandParser::new();

        // Add digits
        parser.add_digit(1);
        parser.add_digit(2);
        parser.add_digit(3);

        assert_eq!(parser.take_number(), Some(123));
        assert_eq!(parser.take_number(), None);
    }

    #[test]
    fn test_parse_space_key() {
        let mut parser = CommandParser::new();
        let key = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE);

        let action = parser.parse_key(key);
        assert_eq!(action, Some(MoreAction::NextPage(0)));
    }

    #[test]
    fn test_parse_with_prefix() {
        let mut parser = CommandParser::new();

        // Simulate "5<space>"
        parser.add_digit(5);
        let key = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE);
        let action = parser.parse_key(key);

        assert_eq!(action, Some(MoreAction::NextPage(5)));
    }

    #[test]
    fn test_parse_colon_commands() {
        let parser = CommandParser::new();

        assert_eq!(
            parser.parse_colon_command(":n"),
            Some(MoreAction::NextFile(1))
        );
        assert_eq!(
            parser.parse_colon_command(":p"),
            Some(MoreAction::PrevFile(1))
        );
        assert_eq!(
            parser.parse_colon_command(":f"),
            Some(MoreAction::ShowFileInfo)
        );
        assert_eq!(parser.parse_colon_command(":q"), Some(MoreAction::Quit));

        // Shell command
        assert_eq!(
            parser.parse_colon_command(":!ls"),
            Some(MoreAction::RunShell("ls".to_string()))
        );
    }

    #[test]
    fn test_last_command_repetition() {
        let mut parser = CommandParser::new();

        let key = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE);
        parser.parse_key(key);

        assert_eq!(parser.last_command(), Some(&MoreAction::NextPage(0)));
    }
}
