/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved
 *   syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */
use std::char::from_digit;
use std::ffi::OsStr;
use std::fmt;

// These are characters with special meaning in the shell (e.g. bash).
// The first const contains characters that only have a special meaning when they appear at the beginning of a name.
const SPECIAL_SHELL_CHARS_START: &[char] = &['~', '#'];
const SPECIAL_SHELL_CHARS: &str = "`$&*()|[]{};\\'\"<>?! ";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuotingStyle {
    Shell {
        escape: bool,
        always_quote: bool,
        show_control: bool,
    },
    C {
        quotes: Quotes,
    },
    Literal {
        show_control: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Quotes {
    None,
    Single,
    Double,
    // TODO: Locale
}

// This implementation is heavily inspired by the std::char::EscapeDefault implementation
// in the Rust standard library. This custom implementation is needed because the
// characters \a, \b, \e, \f & \v are not recognized by Rust.
struct EscapedChar {
    state: EscapeState,
}

enum EscapeState {
    Done,
    Char(char),
    Backslash(char),
    ForceQuote(char),
    Octal(EscapeOctal),
}

struct EscapeOctal {
    c: char,
    state: EscapeOctalState,
    idx: usize,
}

enum EscapeOctalState {
    Done,
    Backslash,
    Value,
}

impl Iterator for EscapeOctal {
    type Item = char;

    fn next(&mut self) -> Option<char> {
        match self.state {
            EscapeOctalState::Done => None,
            EscapeOctalState::Backslash => {
                self.state = EscapeOctalState::Value;
                Some('\\')
            }
            EscapeOctalState::Value => {
                let octal_digit = ((self.c as u32) >> (self.idx * 3)) & 0o7;
                if self.idx == 0 {
                    self.state = EscapeOctalState::Done;
                } else {
                    self.idx -= 1;
                }
                Some(from_digit(octal_digit, 8).unwrap())
            }
        }
    }
}

impl EscapeOctal {
    fn from(c: char) -> Self {
        Self {
            c,
            idx: 2,
            state: EscapeOctalState::Backslash,
        }
    }
}

impl EscapedChar {
    fn new_literal(c: char) -> Self {
        Self {
            state: EscapeState::Char(c),
        }
    }

    fn new_c(c: char, quotes: Quotes) -> Self {
        use EscapeState::*;
        let init_state = match c {
            '\x07' => Backslash('a'),
            '\x08' => Backslash('b'),
            '\t' => Backslash('t'),
            '\n' => Backslash('n'),
            '\x0B' => Backslash('v'),
            '\x0C' => Backslash('f'),
            '\r' => Backslash('r'),
            '\\' => Backslash('\\'),
            '\'' => match quotes {
                Quotes::Single => Backslash('\''),
                _ => Char('\''),
            },
            '"' => match quotes {
                Quotes::Double => Backslash('"'),
                _ => Char('"'),
            },
            ' ' => match quotes {
                Quotes::None => Backslash(' '),
                _ => Char(' '),
            },
            _ if c.is_ascii_control() => Octal(EscapeOctal::from(c)),
            _ => Char(c),
        };
        Self { state: init_state }
    }

    fn new_shell(c: char, escape: bool, quotes: Quotes) -> Self {
        use EscapeState::*;
        let init_state = match c {
            _ if !escape && c.is_control() => Char(c),
            '\x07' => Backslash('a'),
            '\x08' => Backslash('b'),
            '\t' => Backslash('t'),
            '\n' => Backslash('n'),
            '\x0B' => Backslash('v'),
            '\x0C' => Backslash('f'),
            '\r' => Backslash('r'),
            '\x00'..='\x1F' | '\x7F' => Octal(EscapeOctal::from(c)),
            '\'' => match quotes {
                Quotes::Single => Backslash('\''),
                _ => Char('\''),
            },
            _ if SPECIAL_SHELL_CHARS.contains(c) => ForceQuote(c),
            _ => Char(c),
        };
        Self { state: init_state }
    }

    fn hide_control(self) -> Self {
        match self.state {
            EscapeState::Char(c) if c.is_control() => Self {
                state: EscapeState::Char('?'),
            },
            _ => self,
        }
    }
}

impl Iterator for EscapedChar {
    type Item = char;

    fn next(&mut self) -> Option<char> {
        match self.state {
            EscapeState::Backslash(c) => {
                self.state = EscapeState::Char(c);
                Some('\\')
            }
            EscapeState::Char(c) | EscapeState::ForceQuote(c) => {
                self.state = EscapeState::Done;
                Some(c)
            }
            EscapeState::Done => None,
            EscapeState::Octal(ref mut iter) => iter.next(),
        }
    }
}

fn shell_without_escape(name: &str, quotes: Quotes, show_control_chars: bool) -> (String, bool) {
    let mut must_quote = false;
    let mut escaped_str = String::with_capacity(name.len());

    for c in name.chars() {
        let escaped = {
            let ec = EscapedChar::new_shell(c, false, quotes);
            if show_control_chars {
                ec
            } else {
                ec.hide_control()
            }
        };

        match escaped.state {
            EscapeState::Backslash('\'') => escaped_str.push_str("'\\''"),
            EscapeState::ForceQuote(x) => {
                must_quote = true;
                escaped_str.push(x);
            }
            _ => {
                for char in escaped {
                    escaped_str.push(char);
                }
            }
        }
    }

    must_quote = must_quote || name.starts_with(SPECIAL_SHELL_CHARS_START);
    (escaped_str, must_quote)
}

fn shell_with_escape(name: &str, quotes: Quotes) -> (String, bool) {
    // We need to keep track of whether we are in a dollar expression
    // because e.g. \b\n is escaped as $'\b\n' and not like $'b'$'n'
    let mut in_dollar = false;
    let mut must_quote = false;
    let mut escaped_str = String::with_capacity(name.len());

    for c in name.chars() {
        let escaped = EscapedChar::new_shell(c, true, quotes);
        match escaped.state {
            EscapeState::Char(x) => {
                if in_dollar {
                    escaped_str.push_str("''");
                    in_dollar = false;
                }
                escaped_str.push(x);
            }
            EscapeState::ForceQuote(x) => {
                if in_dollar {
                    escaped_str.push_str("''");
                    in_dollar = false;
                }
                must_quote = true;
                escaped_str.push(x);
            }
            // Single quotes are not put in dollar expressions, but are escaped
            // if the string also contains double quotes. In that case, they must
            // be handled separately.
            EscapeState::Backslash('\'') => {
                must_quote = true;
                in_dollar = false;
                escaped_str.push_str("'\\''");
            }
            _ => {
                if !in_dollar {
                    escaped_str.push_str("'$'");
                    in_dollar = true;
                }
                must_quote = true;
                for char in escaped {
                    escaped_str.push(char);
                }
            }
        }
    }
    must_quote = must_quote || name.starts_with(SPECIAL_SHELL_CHARS_START);
    (escaped_str, must_quote)
}

pub fn escape_name(name: &OsStr, style: &QuotingStyle) -> String {
    match style {
        QuotingStyle::Literal { show_control } => {
            if *show_control {
                name.to_string_lossy().into_owned()
            } else {
                name.to_string_lossy()
                    .chars()
                    .flat_map(|c| EscapedChar::new_literal(c).hide_control())
                    .collect()
            }
        }
        QuotingStyle::C { quotes } => {
            let escaped_str: String = name
                .to_string_lossy()
                .chars()
                .flat_map(|c| EscapedChar::new_c(c, *quotes))
                .collect();

            match quotes {
                Quotes::Single => format!("'{escaped_str}'"),
                Quotes::Double => format!("\"{escaped_str}\""),
                Quotes::None => escaped_str,
            }
        }
        QuotingStyle::Shell {
            escape,
            always_quote,
            show_control,
        } => {
            let name = name.to_string_lossy();
            let (quotes, must_quote) = if name.contains(&['"', '`', '$', '\\'][..]) {
                (Quotes::Single, true)
            } else if name.contains('\'') {
                (Quotes::Double, true)
            } else if *always_quote {
                (Quotes::Single, true)
            } else {
                (Quotes::Single, false)
            };

            let (escaped_str, contains_quote_chars) = if *escape {
                shell_with_escape(&name, quotes)
            } else {
                shell_without_escape(&name, quotes, *show_control)
            };

            match (must_quote | contains_quote_chars, quotes) {
                (true, Quotes::Single) => format!("'{escaped_str}'"),
                (true, Quotes::Double) => format!("\"{escaped_str}\""),
                _ => escaped_str,
            }
        }
    }
}

impl fmt::Display for QuotingStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Shell {
                escape,
                always_quote,
                show_control,
            } => {
                let mut style = "shell".to_string();
                if escape {
                    style.push_str("-escape");
                }
                if always_quote {
                    style.push_str("-always-quote");
                }
                if show_control {
                    style.push_str("-show-control");
                }
                f.write_str(&style)
            }
            Self::C { .. } => f.write_str("C"),
            Self::Literal { .. } => f.write_str("literal"),
        }
    }
}

impl fmt::Display for Quotes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::None => f.write_str("None"),
            Self::Single => f.write_str("Single"),
            Self::Double => f.write_str("Double"),
        }
    }
}

