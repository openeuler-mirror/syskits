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
use std::char::from_digit;
use std::ffi::OsStr;
use std::fmt;

#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStrExt;

#[cfg(all(target_os = "linux", target_env = "gnu"))]
unsafe extern "C" {
    fn __ctype_get_mb_cur_max() -> usize;
}

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn mbrtowc(
        wide: *mut crate::libc::wchar_t,
        bytes: *const crate::libc::c_char,
        length: usize,
        state: *mut crate::libc::mbstate_t,
    ) -> usize;
    fn iswprint(wide: crate::libc::c_uint) -> crate::libc::c_int;
}

// 这些是在shell（如bash）中有特殊含义的字符。
// 第一个常量包含仅在名称开始处出现时才有特殊含义的字符
const CT_SPECIAL_SHELL_CHARS_START: &[char] = &['~', '#'];
const CT_SPECIAL_SHELL_CHARS: &str = "`$&*()|[{};\\'\"<>=^?! ";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CtQuotingStyle {
    Shell {
        escape: bool,
        always_quote: bool,
        show_control: bool,
    },
    C {
        quotes: CtQuotes,
    },
    Literal {
        show_control: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CtQuotes {
    None,
    Single,
    Double,
    // TODO: Locale
}

// 此实现深受Rust标准库中std::char::EscapeDefault实现的启发。
// 需要这个自定义实现是因为Rust不识别字符\a、\b、\e、\f和\v。
struct CtEscapedChar {
    state: CtEscapeState,
}

enum CtEscapeState {
    Done,
    Char(char),
    Backslash(char),
    ForceQuote(char),
    Octal(CtEscapeOctal),
}

struct CtEscapeOctal {
    c: char,
    state: CtEscapeOctalState,
    idx: usize,
}

enum CtEscapeOctalState {
    Done,
    Backslash,
    Value,
}

impl Iterator for CtEscapeOctal {
    type Item = char;

    fn next(&mut self) -> Option<char> {
        match self.state {
            CtEscapeOctalState::Done => None,
            CtEscapeOctalState::Backslash => {
                self.state = CtEscapeOctalState::Value;
                Some('\\')
            }
            CtEscapeOctalState::Value => {
                let octal_digit = ((self.c as u32) >> (self.idx * 3)) & 0o7;
                if self.idx == 0 {
                    self.state = CtEscapeOctalState::Done;
                } else {
                    self.idx -= 1;
                }
                Some(from_digit(octal_digit, 8).unwrap())
            }
        }
    }
}

impl CtEscapeOctal {
    fn from(c: char) -> Self {
        Self {
            c,
            idx: 2,
            state: CtEscapeOctalState::Backslash,
        }
    }
}

impl CtEscapedChar {
    fn new_literal(c: char) -> Self {
        Self {
            state: CtEscapeState::Char(c),
        }
    }

    fn new_c(c: char, quotes: CtQuotes) -> Self {
        use CtEscapeState::*;
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
                CtQuotes::Single => Backslash('\''),
                _ => Char('\''),
            },
            '"' => match quotes {
                CtQuotes::Double => Backslash('"'),
                _ => Char('"'),
            },
            ' ' => match quotes {
                CtQuotes::None => Backslash(' '),
                _ => Char(' '),
            },
            _ if c.is_ascii_control() => Octal(CtEscapeOctal::from(c)),
            _ => Char(c),
        };
        Self { state: init_state }
    }

    fn new_shell(c: char, escape: bool, quotes: CtQuotes) -> Self {
        use CtEscapeState::*;
        let init_state = match c {
            _ if !escape && c.is_control() => Char(c),
            '\x07' => Backslash('a'),
            '\x08' => Backslash('b'),
            '\t' => Backslash('t'),
            '\n' => Backslash('n'),
            '\x0B' => Backslash('v'),
            '\x0C' => Backslash('f'),
            '\r' => Backslash('r'),
            '\x00'..='\x1F' | '\x7F' => Octal(CtEscapeOctal::from(c)),
            '\'' => match quotes {
                CtQuotes::Single => Backslash('\''),
                _ => Char('\''),
            },
            _ if CT_SPECIAL_SHELL_CHARS.contains(c) => ForceQuote(c),
            _ => Char(c),
        };
        Self { state: init_state }
    }

    fn hide_control(self) -> Self {
        match self.state {
            CtEscapeState::Char(c) if c.is_control() => Self {
                state: CtEscapeState::Char('?'),
            },
            _ => self,
        }
    }
}

impl Iterator for CtEscapedChar {
    type Item = char;

    fn next(&mut self) -> Option<char> {
        match self.state {
            CtEscapeState::Backslash(c) => {
                self.state = CtEscapeState::Char(c);
                Some('\\')
            }
            CtEscapeState::Char(c) | CtEscapeState::ForceQuote(c) => {
                self.state = CtEscapeState::Done;
                Some(c)
            }
            CtEscapeState::Done => None,
            CtEscapeState::Octal(ref mut iter) => iter.next(),
        }
    }
}

fn shell_without_escape(name: &str, quotes: CtQuotes, show_control_chars: bool) -> (String, bool) {
    let mut must_quote = false;
    let mut escaped_str = String::with_capacity(name.len());

    for c in name.chars() {
        let escaped = {
            let ec = CtEscapedChar::new_shell(c, false, quotes);
            if show_control_chars {
                ec
            } else {
                ec.hide_control()
            }
        };

        match escaped.state {
            CtEscapeState::Backslash('\'') => escaped_str.push_str("'\\''"),
            CtEscapeState::ForceQuote(x) => {
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

    must_quote = must_quote || name.starts_with(CT_SPECIAL_SHELL_CHARS_START);
    (escaped_str, must_quote)
}

fn shell_with_escape_pass(
    name: &str,
    quotes: CtQuotes,
    mut in_dollar: bool,
) -> (String, bool, bool) {
    // We need to keep track of whether we are in a dollar expression
    // because e.g. \b\n is escaped as $'\b\n' and not like $'b'$'n'
    let mut must_quote = false;
    let mut escaped_str = String::with_capacity(name.len());

    for c in name.chars() {
        if c.is_control() && !c.is_ascii() {
            if !in_dollar {
                escaped_str.push_str("'$'");
                in_dollar = true;
            }
            must_quote = true;

            let mut encoded = [0; 4];
            for byte in c.encode_utf8(&mut encoded).as_bytes() {
                escaped_str.push('\\');
                escaped_str.push(char::from(b'0' + (byte >> 6)));
                escaped_str.push(char::from(b'0' + ((byte >> 3) & 0o7)));
                escaped_str.push(char::from(b'0' + (byte & 0o7)));
            }
            continue;
        }

        let escaped = CtEscapedChar::new_shell(c, true, quotes);
        match escaped.state {
            CtEscapeState::Char(x) => {
                if in_dollar {
                    escaped_str.push_str("''");
                    in_dollar = false;
                }
                escaped_str.push(x);
            }
            CtEscapeState::ForceQuote(x) => {
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
            CtEscapeState::Backslash('\'') => {
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
    (escaped_str, must_quote, in_dollar)
}

fn shell_with_escape(name: &str, quotes: CtQuotes) -> (String, bool) {
    let (escaped_str, must_quote, in_dollar) = shell_with_escape_pass(name, quotes, false);
    // GNU's apostrophe-minimization rescan retains an open trailing $'...' state.
    let (escaped_str, mut must_quote) = if in_dollar && name.contains('\'') {
        let (escaped_str, must_quote, _) = shell_with_escape_pass(name, quotes, true);
        (escaped_str, must_quote)
    } else {
        (escaped_str, must_quote)
    };

    must_quote = must_quote || name.starts_with(CT_SPECIAL_SHELL_CHARS_START);
    (escaped_str, must_quote)
}

pub(crate) fn uses_unibyte_locale() -> bool {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        unsafe { __ctype_get_mb_cur_max() == 1 }
    }

    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    false
}

#[cfg(test)]
pub(crate) fn escape_unibyte_c_bytes(name: &[u8], quotes: CtQuotes) -> String {
    let mut escaped = String::with_capacity(name.len());
    for byte in name {
        if byte.is_ascii() {
            escaped.extend(CtEscapedChar::new_c(char::from(*byte), quotes));
        } else {
            escaped.extend(CtEscapeOctal::from(char::from(*byte)));
        }
    }

    match quotes {
        CtQuotes::Single => format!("'{escaped}'"),
        CtQuotes::Double => format!("\"{escaped}\""),
        CtQuotes::None => escaped,
    }
}

#[cfg(test)]
pub(crate) fn escape_unibyte_shell_bytes(name: &[u8]) -> String {
    if name.is_ascii() {
        let ascii = unsafe { std::str::from_utf8_unchecked(name) };
        if shell_escape_can_use_c_quotes(ascii) {
            return format!("\"{ascii}\"");
        }
    }

    let (escaped_str, must_quote, in_dollar) = escape_unibyte_shell_bytes_pass(name, false);
    let (escaped_str, mut must_quote) = if in_dollar && name.contains(&b'\'') {
        let (escaped_str, must_quote, _) = escape_unibyte_shell_bytes_pass(name, true);
        (escaped_str, must_quote)
    } else {
        (escaped_str, must_quote)
    };

    must_quote = must_quote || matches!(name.first(), Some(b'~' | b'#'));
    if must_quote {
        format!("'{escaped_str}'")
    } else {
        escaped_str
    }
}

#[cfg(test)]
fn escape_unibyte_shell_bytes_pass(name: &[u8], mut in_dollar: bool) -> (String, bool, bool) {
    let mut must_quote = false;
    let mut escaped_str = String::with_capacity(name.len());

    for byte in name {
        if !byte.is_ascii() {
            if !in_dollar {
                escaped_str.push_str("'$'");
                in_dollar = true;
            }
            must_quote = true;
            escaped_str.push('\\');
            escaped_str.push(char::from(b'0' + (byte >> 6)));
            escaped_str.push(char::from(b'0' + ((byte >> 3) & 0o7)));
            escaped_str.push(char::from(b'0' + (byte & 0o7)));
            continue;
        }

        let escaped = CtEscapedChar::new_shell(char::from(*byte), true, CtQuotes::Single);
        match escaped.state {
            CtEscapeState::Char(c) => {
                if in_dollar {
                    escaped_str.push_str("''");
                    in_dollar = false;
                }
                escaped_str.push(c);
            }
            CtEscapeState::ForceQuote(c) => {
                if in_dollar {
                    escaped_str.push_str("''");
                    in_dollar = false;
                }
                must_quote = true;
                escaped_str.push(c);
            }
            CtEscapeState::Backslash('\'') => {
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
                for c in escaped {
                    escaped_str.push(c);
                }
            }
        }
    }
    (escaped_str, must_quote, in_dollar)
}

#[derive(Clone, Copy)]
enum ShellByteSegment {
    Ascii {
        byte: u8,
        index: usize,
    },
    Locale {
        start: usize,
        end: usize,
        printable: bool,
    },
}

/// Shell-quote bytes using the caller's locale sequence classifier.
pub fn escape_shell_bytes_with_classifier<F>(name: &[u8], mut classify: F) -> Vec<u8>
where
    F: FnMut(&[u8]) -> (usize, bool),
{
    if name.is_empty() {
        return b"''".to_vec();
    }

    let mut segments = Vec::with_capacity(name.len());
    let mut index = 0;
    while index < name.len() {
        let byte = name[index];
        if byte.is_ascii() {
            segments.push(ShellByteSegment::Ascii { byte, index });
            index += 1;
        } else {
            let (length, printable) = classify(&name[index..]);
            let length = length.clamp(1, name.len() - index);
            segments.push(ShellByteSegment::Locale {
                start: index,
                end: index + length,
                printable,
            });
            index += length;
        }
    }

    let mut encountered_apostrophe = false;
    let mut all_c_and_shell_quote_compatible = true;
    for segment in &segments {
        match *segment {
            ShellByteSegment::Ascii { byte, index } => {
                let character = char::from(byte);
                encountered_apostrophe |= character == '\'';
                all_c_and_shell_quote_compatible &= c_and_shell_quote_compatible(character, index);
            }
            ShellByteSegment::Locale { printable, .. } => {
                all_c_and_shell_quote_compatible &= printable;
            }
        }
    }

    if encountered_apostrophe && all_c_and_shell_quote_compatible {
        let mut escaped = Vec::with_capacity(name.len() + 2);
        escaped.push(b'"');
        escaped.extend_from_slice(name);
        escaped.push(b'"');
        return escaped;
    }

    let (escaped, must_quote, in_dollar) = escape_shell_byte_segments_pass(name, &segments, false);
    let (escaped, mut must_quote) = if encountered_apostrophe && in_dollar {
        let (escaped, must_quote, _) = escape_shell_byte_segments_pass(name, &segments, true);
        (escaped, must_quote)
    } else {
        (escaped, must_quote)
    };

    must_quote |= matches!(name.first(), Some(b'~' | b'#'));
    if must_quote {
        let mut quoted = Vec::with_capacity(escaped.len() + 2);
        quoted.push(b'\'');
        quoted.extend_from_slice(&escaped);
        quoted.push(b'\'');
        quoted
    } else {
        escaped
    }
}

/// Quote a diagnostic operand with GNU coreutils shell-quoting semantics.
///
/// This matches GNU's `quoteaf` behavior on Linux: printable locale sequences
/// remain literal, while control and invalid bytes are split into `$'...'`
/// segments with GNU escape spellings. `always_quote` requests the outer
/// single quotes used by `quoteaf` when the operand otherwise needs no escape.
#[cfg(target_os = "linux")]
pub fn gnu_quote_shell(name: &OsStr, always_quote: bool) -> String {
    let bytes = name.as_bytes();
    let mut quoted = escape_shell_bytes_with_classifier(bytes, |remaining| unsafe {
        let mut state: crate::libc::mbstate_t = std::mem::zeroed();
        let mut wide = 0 as crate::libc::wchar_t;
        let length = mbrtowc(
            &mut wide,
            remaining.as_ptr().cast(),
            remaining.len(),
            &mut state,
        );
        if length == usize::MAX {
            return (1, false);
        }
        if length == usize::MAX - 1 {
            return (remaining.len(), false);
        }

        let length = if length == 0 { 1 } else { length };
        let is_utf8 = std::str::from_utf8(&remaining[..length]).is_ok();
        (
            length,
            is_utf8 && iswprint(wide as crate::libc::c_uint) != 0,
        )
    });

    if always_quote && quoted.as_slice() == bytes {
        quoted.insert(0, b'\'');
        quoted.push(b'\'');
    }

    String::from_utf8(quoted).expect("GNU shell-escaped file names are valid UTF-8")
}

#[cfg(not(target_os = "linux"))]
pub fn gnu_quote_shell(name: &OsStr, always_quote: bool) -> String {
    let name = name.to_string_lossy();
    if always_quote {
        format!("'{}'", name.replace('\\', "\\\\").replace('\'', "\\'"))
    } else {
        name.into_owned()
    }
}

fn escape_shell_byte_segments_pass(
    name: &[u8],
    segments: &[ShellByteSegment],
    mut in_dollar: bool,
) -> (Vec<u8>, bool, bool) {
    let mut must_quote = false;
    let mut escaped = Vec::with_capacity(name.len());

    for segment in segments {
        match *segment {
            ShellByteSegment::Ascii { byte, .. } => {
                let escaped_character =
                    CtEscapedChar::new_shell(char::from(byte), true, CtQuotes::Single);
                match escaped_character.state {
                    CtEscapeState::Char(character) => {
                        if in_dollar {
                            escaped.extend_from_slice(b"''");
                            in_dollar = false;
                        }
                        escaped.push(character as u8);
                    }
                    CtEscapeState::ForceQuote(character) => {
                        if in_dollar {
                            escaped.extend_from_slice(b"''");
                            in_dollar = false;
                        }
                        must_quote = true;
                        escaped.push(character as u8);
                    }
                    CtEscapeState::Backslash('\'') => {
                        must_quote = true;
                        in_dollar = false;
                        escaped.extend_from_slice(b"'\\''");
                    }
                    _ => {
                        if !in_dollar {
                            escaped.extend_from_slice(b"'$'");
                            in_dollar = true;
                        }
                        must_quote = true;
                        escaped.extend(escaped_character.map(|character| character as u8));
                    }
                }
            }
            ShellByteSegment::Locale {
                start,
                end,
                printable,
            } => {
                let bytes = &name[start..end];
                if printable {
                    if in_dollar {
                        escaped.extend_from_slice(b"''");
                        in_dollar = false;
                    }
                    must_quote |= bytes[1..]
                        .iter()
                        .any(|byte| matches!(byte, b'[' | b'\\' | b'^' | b'`' | b'|'));
                    escaped.extend_from_slice(bytes);
                } else {
                    if !in_dollar {
                        escaped.extend_from_slice(b"'$'");
                        in_dollar = true;
                    }
                    must_quote = true;
                    for byte in bytes {
                        escaped.push(b'\\');
                        escaped.push(b'0' + (byte >> 6));
                        escaped.push(b'0' + ((byte >> 3) & 0o7));
                        escaped.push(b'0' + (byte & 0o7));
                    }
                }
            }
        }
    }

    (escaped, must_quote, in_dollar)
}

pub fn escape_name(name: &OsStr, style: &CtQuotingStyle) -> String {
    match style {
        CtQuotingStyle::Literal { show_control } => {
            if *show_control {
                name.to_string_lossy().into_owned()
            } else {
                name.to_string_lossy()
                    .chars()
                    .flat_map(|c| CtEscapedChar::new_literal(c).hide_control())
                    .collect()
            }
        }
        CtQuotingStyle::C { quotes } => {
            let escaped_str: String = name
                .to_string_lossy()
                .chars()
                .flat_map(|c| CtEscapedChar::new_c(c, *quotes))
                .collect();

            match quotes {
                CtQuotes::Single => format!("'{escaped_str}'"),
                CtQuotes::Double => format!("\"{escaped_str}\""),
                CtQuotes::None => escaped_str,
            }
        }
        CtQuotingStyle::Shell {
            escape,
            always_quote,
            show_control,
        } => {
            let name = name.to_string_lossy();
            if *escape && shell_escape_can_use_c_quotes(&name) {
                return format!("\"{name}\"");
            }
            let (quotes, must_quote) = if name.contains(&['"', '`', '$', '\\'][..]) {
                (CtQuotes::Single, true)
            } else if name.contains('\'') && !*escape {
                (CtQuotes::Double, true)
            } else if *always_quote {
                (CtQuotes::Single, true)
            } else {
                (CtQuotes::Single, false)
            };

            let (escaped_str, contains_quote_chars) = if *escape {
                shell_with_escape(&name, quotes)
            } else {
                shell_without_escape(&name, quotes, *show_control)
            };

            match (must_quote | contains_quote_chars, quotes) {
                (true, CtQuotes::Single) => format!("'{escaped_str}'"),
                (true, CtQuotes::Double) => format!("\"{escaped_str}\""),
                _ => escaped_str,
            }
        }
    }
}

fn shell_escape_can_use_c_quotes(name: &str) -> bool {
    let mut encountered_apostrophe = false;
    for (index, character) in name.chars().enumerate() {
        if character == '\'' {
            encountered_apostrophe = true;
        }
        if !c_and_shell_quote_compatible(character, index) {
            return false;
        }
    }
    encountered_apostrophe
}

fn c_and_shell_quote_compatible(character: char, index: usize) -> bool {
    if !character.is_ascii() {
        return !character.is_control();
    }
    if character.is_ascii_control() {
        return false;
    }
    if matches!(character, '#' | '~') {
        return index == 0;
    }

    !matches!(
        character,
        '?' | '\\'
            | '{'
            | '}'
            | '!'
            | '"'
            | '$'
            | '&'
            | '('
            | ')'
            | '*'
            | ';'
            | '<'
            | '='
            | '>'
            | '['
            | '^'
            | '`'
            | '|'
    )
}

impl fmt::Display for CtQuotingStyle {
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

impl fmt::Display for CtQuotes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::None => f.write_str("None"),
            Self::Single => f.write_str("Single"),
            Self::Double => f.write_str("Double"),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::ct_quoting_style::{
        CtQuotes, CtQuotingStyle, escape_name, escape_shell_bytes_with_classifier,
        escape_unibyte_c_bytes, escape_unibyte_shell_bytes, gnu_quote_shell,
    };
    use std::ffi::OsStr;

    //拼写检查器忽略（tests/words）one'two one'two

    fn get_style(s: &str) -> CtQuotingStyle {
        match s {
            "literal" => CtQuotingStyle::Literal {
                show_control: false,
            },
            "literal-show" => CtQuotingStyle::Literal { show_control: true },
            "escape" => CtQuotingStyle::C {
                quotes: CtQuotes::None,
            },
            "c" => CtQuotingStyle::C {
                quotes: CtQuotes::Double,
            },
            "shell" => CtQuotingStyle::Shell {
                escape: false,
                always_quote: false,
                show_control: false,
            },
            "shell-show" => CtQuotingStyle::Shell {
                escape: false,
                always_quote: false,
                show_control: true,
            },
            "shell-always" => CtQuotingStyle::Shell {
                escape: false,
                always_quote: true,
                show_control: false,
            },
            "shell-always-show" => CtQuotingStyle::Shell {
                escape: false,
                always_quote: true,
                show_control: true,
            },
            "shell-escape" => CtQuotingStyle::Shell {
                escape: true,
                always_quote: false,
                show_control: false,
            },
            "shell-escape-always" => CtQuotingStyle::Shell {
                escape: true,
                always_quote: true,
                show_control: false,
            },
            _ => panic!("Invalid name!"),
        }
    }

    fn check_names(name: &str, map: &[(&str, &str)]) {
        assert_eq!(
            map.iter()
                .map(|(_, style)| escape_name(name.as_ref(), &get_style(style)))
                .collect::<Vec<String>>(),
            map.iter()
                .map(|(correct, _)| correct.to_string())
                .collect::<Vec<String>>()
        );
    }

    #[test]
    fn test_simple_names() {
        check_names(
            "one_two",
            &[
                ("one_two", "literal"),
                ("one_two", "literal-show"),
                ("one_two", "escape"),
                ("\"one_two\"", "c"),
                ("one_two", "shell"),
                ("one_two", "shell-show"),
                ("\'one_two\'", "shell-always"),
                ("\'one_two\'", "shell-always-show"),
                ("one_two", "shell-escape"),
                ("\'one_two\'", "shell-escape-always"),
            ],
        );
    }

    #[test]
    fn test_spaces() {
        check_names(
            "one two",
            &[
                ("one two", "literal"),
                ("one two", "literal-show"),
                ("one\\ two", "escape"),
                ("\"one two\"", "c"),
                ("\'one two\'", "shell"),
                ("\'one two\'", "shell-show"),
                ("\'one two\'", "shell-always"),
                ("\'one two\'", "shell-always-show"),
                ("\'one two\'", "shell-escape"),
                ("\'one two\'", "shell-escape-always"),
            ],
        );

        check_names(
            " one",
            &[
                (" one", "literal"),
                (" one", "literal-show"),
                ("\\ one", "escape"),
                ("\" one\"", "c"),
                ("' one'", "shell"),
                ("' one'", "shell-show"),
                ("' one'", "shell-always"),
                ("' one'", "shell-always-show"),
                ("' one'", "shell-escape"),
                ("' one'", "shell-escape-always"),
            ],
        );
    }

    #[test]
    fn test_quotes() {
        // One double quote
        check_names(
            "one\"two",
            &[
                ("one\"two", "literal"),
                ("one\"two", "literal-show"),
                ("one\"two", "escape"),
                ("\"one\\\"two\"", "c"),
                ("'one\"two'", "shell"),
                ("'one\"two'", "shell-show"),
                ("'one\"two'", "shell-always"),
                ("'one\"two'", "shell-always-show"),
                ("'one\"two'", "shell-escape"),
                ("'one\"two'", "shell-escape-always"),
            ],
        );

        // One single quote
        check_names(
            "one\'two",
            &[
                ("one'two", "literal"),
                ("one'two", "literal-show"),
                ("one'two", "escape"),
                ("\"one'two\"", "c"),
                ("\"one'two\"", "shell"),
                ("\"one'two\"", "shell-show"),
                ("\"one'two\"", "shell-always"),
                ("\"one'two\"", "shell-always-show"),
                ("\"one'two\"", "shell-escape"),
                ("\"one'two\"", "shell-escape-always"),
            ],
        );

        // One single quote and one double quote
        check_names(
            "one'two\"three",
            &[
                ("one'two\"three", "literal"),
                ("one'two\"three", "literal-show"),
                ("one'two\"three", "escape"),
                ("\"one'two\\\"three\"", "c"),
                ("'one'\\''two\"three'", "shell"),
                ("'one'\\''two\"three'", "shell-show"),
                ("'one'\\''two\"three'", "shell-always"),
                ("'one'\\''two\"three'", "shell-always-show"),
                ("'one'\\''two\"three'", "shell-escape"),
                ("'one'\\''two\"three'", "shell-escape-always"),
            ],
        );

        // Consecutive quotes
        check_names(
            "one''two\"\"three",
            &[
                ("one''two\"\"three", "literal"),
                ("one''two\"\"three", "literal-show"),
                ("one''two\"\"three", "escape"),
                ("\"one''two\\\"\\\"three\"", "c"),
                ("'one'\\'''\\''two\"\"three'", "shell"),
                ("'one'\\'''\\''two\"\"three'", "shell-show"),
                ("'one'\\'''\\''two\"\"three'", "shell-always"),
                ("'one'\\'''\\''two\"\"three'", "shell-always-show"),
                ("'one'\\'''\\''two\"\"three'", "shell-escape"),
                ("'one'\\'''\\''two\"\"three'", "shell-escape-always"),
            ],
        );
    }

    #[test]
    fn test_control_chars() {
        // A simple newline
        check_names(
            "one\ntwo",
            &[
                ("one?two", "literal"),
                ("one\ntwo", "literal-show"),
                ("one\\ntwo", "escape"),
                ("\"one\\ntwo\"", "c"),
                ("one?two", "shell"),
                ("one\ntwo", "shell-show"),
                ("'one?two'", "shell-always"),
                ("'one\ntwo'", "shell-always-show"),
                ("'one'$'\\n''two'", "shell-escape"),
                ("'one'$'\\n''two'", "shell-escape-always"),
            ],
        );

        // A control character followed by a special shell character
        check_names(
            "one\n&two",
            &[
                ("one?&two", "literal"),
                ("one\n&two", "literal-show"),
                ("one\\n&two", "escape"),
                ("\"one\\n&two\"", "c"),
                ("'one?&two'", "shell"),
                ("'one\n&two'", "shell-show"),
                ("'one?&two'", "shell-always"),
                ("'one\n&two'", "shell-always-show"),
                ("'one'$'\\n''&two'", "shell-escape"),
                ("'one'$'\\n''&two'", "shell-escape-always"),
            ],
        );

        // The first 16 control characters. NUL is also included, even though it is of
        // no importance for file names.
        check_names(
            "\x00\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0A\x0B\x0C\x0D\x0E\x0F",
            &[
                ("????????????????", "literal"),
                (
                    "\x00\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0A\x0B\x0C\x0D\x0E\x0F",
                    "literal-show",
                ),
                (
                    "\\000\\001\\002\\003\\004\\005\\006\\a\\b\\t\\n\\v\\f\\r\\016\\017",
                    "escape",
                ),
                (
                    "\"\\000\\001\\002\\003\\004\\005\\006\\a\\b\\t\\n\\v\\f\\r\\016\\017\"",
                    "c",
                ),
                ("????????????????", "shell"),
                (
                    "\x00\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0A\x0B\x0C\x0D\x0E\x0F",
                    "shell-show",
                ),
                ("'????????????????'", "shell-always"),
                (
                    "'\x00\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0A\x0B\x0C\x0D\x0E\x0F'",
                    "shell-always-show",
                ),
                (
                    "''$'\\000\\001\\002\\003\\004\\005\\006\\a\\b\\t\\n\\v\\f\\r\\016\\017'",
                    "shell-escape",
                ),
                (
                    "''$'\\000\\001\\002\\003\\004\\005\\006\\a\\b\\t\\n\\v\\f\\r\\016\\017'",
                    "shell-escape-always",
                ),
            ],
        );

        // The last 16 control characters.
        check_names(
            "\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19\x1A\x1B\x1C\x1D\x1E\x1F",
            &[
                ("????????????????", "literal"),
                (
                    "\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19\x1A\x1B\x1C\x1D\x1E\x1F",
                    "literal-show",
                ),
                (
                    "\\020\\021\\022\\023\\024\\025\\026\\027\\030\\031\\032\\033\\034\\035\\036\\037",
                    "escape",
                ),
                (
                    "\"\\020\\021\\022\\023\\024\\025\\026\\027\\030\\031\\032\\033\\034\\035\\036\\037\"",
                    "c",
                ),
                ("????????????????", "shell"),
                (
                    "\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19\x1A\x1B\x1C\x1D\x1E\x1F",
                    "shell-show",
                ),
                ("'????????????????'", "shell-always"),
                (
                    "'\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19\x1A\x1B\x1C\x1D\x1E\x1F'",
                    "shell-always-show",
                ),
                (
                    "''$'\\020\\021\\022\\023\\024\\025\\026\\027\\030\\031\\032\\033\\034\\035\\036\\037'",
                    "shell-escape",
                ),
                (
                    "''$'\\020\\021\\022\\023\\024\\025\\026\\027\\030\\031\\032\\033\\034\\035\\036\\037'",
                    "shell-escape-always",
                ),
            ],
        );

        // DEL
        check_names(
            "\x7F",
            &[
                ("?", "literal"),
                ("\x7F", "literal-show"),
                ("\\177", "escape"),
                ("\"\\177\"", "c"),
                ("?", "shell"),
                ("\x7F", "shell-show"),
                ("'?'", "shell-always"),
                ("'\x7F'", "shell-always-show"),
                ("''$'\\177'", "shell-escape"),
                ("''$'\\177'", "shell-escape-always"),
            ],
        );
    }

    #[test]
    fn test_question_mark() {
        // A question mark must force quotes in shell and shell-always, unless
        // it is in place of a control character (that case is already covered
        // in other tests)
        check_names(
            "one?two",
            &[
                ("one?two", "literal"),
                ("one?two", "literal-show"),
                ("one?two", "escape"),
                ("\"one?two\"", "c"),
                ("'one?two'", "shell"),
                ("'one?two'", "shell-show"),
                ("'one?two'", "shell-always"),
                ("'one?two'", "shell-always-show"),
                ("'one?two'", "shell-escape"),
                ("'one?two'", "shell-escape-always"),
            ],
        );
    }

    #[test]
    fn test_backslash() {
        // Escaped in C-style, but not in Shell-style escaping
        check_names(
            "one\\two",
            &[
                ("one\\two", "literal"),
                ("one\\two", "literal-show"),
                ("one\\\\two", "escape"),
                ("\"one\\\\two\"", "c"),
                ("'one\\two'", "shell"),
                ("\'one\\two\'", "shell-always"),
                ("'one\\two'", "shell-escape"),
                ("'one\\two'", "shell-escape-always"),
            ],
        );
    }

    #[test]
    fn test_tilde_and_hash() {
        check_names("~", &[("'~'", "shell"), ("'~'", "shell-escape")]);
        check_names(
            "~name",
            &[("'~name'", "shell"), ("'~name'", "shell-escape")],
        );
        check_names(
            "some~name",
            &[("some~name", "shell"), ("some~name", "shell-escape")],
        );
        check_names("name~", &[("name~", "shell"), ("name~", "shell-escape")]);

        check_names("#", &[("'#'", "shell"), ("'#'", "shell-escape")]);
        check_names(
            "#name",
            &[("'#name'", "shell"), ("'#name'", "shell-escape")],
        );
        check_names(
            "some#name",
            &[("some#name", "shell"), ("some#name", "shell-escape")],
        );
        check_names("name#", &[("name#", "shell"), ("name#", "shell-escape")]);
    }

    #[test]
    fn test_special_chars_in_double_quotes() {
        check_names(
            "can'$t",
            &[
                ("'can'\\''$t'", "shell"),
                ("'can'\\''$t'", "shell-always"),
                ("'can'\\''$t'", "shell-escape"),
                ("'can'\\''$t'", "shell-escape-always"),
            ],
        );

        check_names(
            "can'`t",
            &[
                ("'can'\\''`t'", "shell"),
                ("'can'\\''`t'", "shell-always"),
                ("'can'\\''`t'", "shell-escape"),
                ("'can'\\''`t'", "shell-escape-always"),
            ],
        );

        check_names(
            "can'\\t",
            &[
                ("'can'\\''\\t'", "shell"),
                ("'can'\\''\\t'", "shell-always"),
                ("'can'\\''\\t'", "shell-escape"),
                ("'can'\\''\\t'", "shell-escape-always"),
            ],
        );
    }

    #[test]
    fn test_quoting_style_display() {
        let style = CtQuotingStyle::Shell {
            escape: true,
            always_quote: false,
            show_control: false,
        };
        assert_eq!(format!("{style}"), "shell-escape");

        let style = CtQuotingStyle::Shell {
            escape: false,
            always_quote: true,
            show_control: false,
        };
        assert_eq!(format!("{style}"), "shell-always-quote");

        let style = CtQuotingStyle::Shell {
            escape: false,
            always_quote: false,
            show_control: true,
        };
        assert_eq!(format!("{style}"), "shell-show-control");

        let style = CtQuotingStyle::C {
            quotes: CtQuotes::Double,
        };
        assert_eq!(format!("{style}"), "C");

        let style = CtQuotingStyle::Literal {
            show_control: false,
        };
        assert_eq!(format!("{style}"), "literal");
    }

    #[test]
    fn test_quotes_display() {
        let none = CtQuotes::None;
        let single = CtQuotes::Single;
        let double = CtQuotes::Double;
        assert_eq!(format!("{none}"), "None");
        assert_eq!(format!("{single}"), "Single");
        assert_eq!(format!("{double}"), "Double");
    }

    #[test]
    fn test_escape_name_shell_escape_true() {
        let test_cases = vec![
            (
                OsStr::new("hello world"),
                CtQuotingStyle::Shell {
                    escape: true,
                    always_quote: false,
                    show_control: false,
                },
                "'hello world'",
            ),
            (
                OsStr::new("~#$&*()|[]{};\\'\"<>?! "),
                CtQuotingStyle::Shell {
                    escape: true,
                    always_quote: false,
                    show_control: false,
                },
                "'~#$&*()|[]{};\\'\\\''\"<>?! '",
            ),
            (
                OsStr::new("\x07\x08\t\n\r"),
                CtQuotingStyle::Shell {
                    escape: true,
                    always_quote: false,
                    show_control: true,
                },
                "''$'\\a\\b\\t\\n\\r'",
            ),
            (
                OsStr::new("\\'"),
                CtQuotingStyle::Shell {
                    escape: true,
                    always_quote: false,
                    show_control: false,
                },
                "'\\'\\'''",
            ),
        ];

        for (input, style, expected_output) in test_cases {
            // println!("[{}]",escape_name(input, &style));
            // println!("[{}]",expected_output);
            // println!("---------------------------");
            assert_eq!(escape_name(input, &style), expected_output);
        }
    }

    #[test]
    fn unibyte_shell_quoting_escapes_each_non_ascii_byte() {
        assert_eq!(escape_unibyte_shell_bytes(b"\xc3\xa9"), "''$'\\303\\251'");
        assert_eq!(
            escape_unibyte_shell_bytes(b"a\xc3\xa9b"),
            "'a'$'\\303\\251''b'"
        );
    }

    #[test]
    fn shell_byte_quoting_uses_locale_character_boundaries_and_printability() {
        // Invalid UTF-8 byte ff -> ASCII shell text ''$'\377'.
        assert_eq!(
            escape_shell_bytes_with_classifier(&[0xff], |_| (1, false)),
            b"''$'\\377'"
        );

        // Printable GBK bytes c2 81 -> the same two raw bytes.
        assert_eq!(
            escape_shell_bytes_with_classifier(&[0xc2, 0x81], |_| (2, true)),
            vec![0xc2, 0x81]
        );

        // Printable GBK e2 80 followed by incomplete 8b -> 'e2 80'$'\213'.
        let mixed = escape_shell_bytes_with_classifier(&[0xe2, 0x80, 0x8b], |bytes| {
            if bytes.starts_with(&[0xe2, 0x80]) {
                (2, true)
            } else {
                (1, false)
            }
        });
        assert_eq!(
            mixed,
            vec![
                b'\'', 0xe2, 0x80, b'\'', b'$', b'\'', b'\\', b'2', b'1', b'3', b'\''
            ]
        );

        // Printable ISO-8859-1 byte ff -> the same raw byte.
        assert_eq!(
            escape_shell_bytes_with_classifier(&[0xff], |_| (1, true)),
            vec![0xff]
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn gnu_shell_quoting_splits_control_bytes_from_plain_text() {
        assert_eq!(
            gnu_quote_shell(OsStr::new("line\nbreak"), true),
            "'line'$'\\n''break'"
        );
    }

    #[test]
    fn shell_escape_quoting_minimizes_apostrophe_quotes() {
        let style = CtQuotingStyle::Shell {
            escape: true,
            always_quote: false,
            show_control: true,
        };

        assert_eq!(escape_unibyte_shell_bytes(b"a'b"), "\"a'b\"");
        assert_eq!(escape_name(OsStr::new("a'b"), &style), "\"a'b\"");
        assert_eq!(escape_unibyte_shell_bytes(b"#'"), "\"#'\"");
        assert_eq!(escape_name(OsStr::new("#'"), &style), "\"#'\"");
        assert_eq!(escape_unibyte_shell_bytes(b"a'b;"), "'a'\\''b;'");
        assert_eq!(escape_name(OsStr::new("a'b;"), &style), "'a'\\''b;'");
        assert_eq!(escape_unibyte_shell_bytes(b"a'b#"), "'a'\\''b#'");
        assert_eq!(escape_name(OsStr::new("a'b#"), &style), "'a'\\''b#'");
    }

    #[test]
    fn shell_escape_quoting_leaves_right_bracket_unquoted() {
        let style = CtQuotingStyle::Shell {
            escape: true,
            always_quote: false,
            show_control: true,
        };

        assert_eq!(escape_unibyte_shell_bytes(b"]"), "]");
        assert_eq!(escape_name(OsStr::new("a]"), &style), "a]");
        assert_eq!(escape_unibyte_shell_bytes(b"["), "'['");
        assert_eq!(escape_name(OsStr::new("a["), &style), "'a['");
    }

    #[test]
    fn shell_escape_quoting_quotes_equal_and_caret() {
        let style = CtQuotingStyle::Shell {
            escape: true,
            always_quote: false,
            show_control: true,
        };

        assert_eq!(escape_name(OsStr::new("="), &style), "'='");
        assert_eq!(escape_name(OsStr::new("a=b"), &style), "'a=b'");
        assert_eq!(escape_name(OsStr::new("^"), &style), "'^'");
        assert_eq!(escape_name(OsStr::new("a^b"), &style), "'a^b'");
    }

    #[test]
    fn shell_escape_quoting_preserves_apostrophe_scan_before_trailing_escape() {
        let style = CtQuotingStyle::Shell {
            escape: true,
            always_quote: false,
            show_control: true,
        };

        let expected = "'''a'\\''b'$'\\t'";
        assert_eq!(escape_unibyte_shell_bytes(b"a'b\t"), expected);
        assert_eq!(escape_name(OsStr::new("a'b\t"), &style), expected);

        let control_before_apostrophe = "''$'\\t'\\''ab'";
        assert_eq!(
            escape_unibyte_shell_bytes(b"\t'ab"),
            control_before_apostrophe
        );
        assert_eq!(
            escape_name(OsStr::new("\t'ab"), &style),
            control_before_apostrophe
        );

        let leading_apostrophe = "''\\'''$'\\t'";
        assert_eq!(escape_unibyte_shell_bytes(b"'\t"), leading_apostrophe);
        assert_eq!(escape_name(OsStr::new("'\t"), &style), leading_apostrophe);

        let leading_control = "'\\t''a'\\''b'$'\\t'";
        assert_eq!(escape_unibyte_shell_bytes(b"\ta'b\t"), leading_control);
        assert_eq!(escape_name(OsStr::new("\ta'b\t"), &style), leading_control);
    }

    #[test]
    fn unibyte_c_quoting_escapes_each_non_ascii_byte() {
        assert_eq!(
            escape_unibyte_c_bytes(b"\xc2\xa01.5", CtQuotes::Single),
            "'\\302\\2401.5'"
        );
    }

    #[test]
    fn test_escape_name_shell_escape_false() {
        let test_cases = vec![
            (
                OsStr::new("hello world"),
                CtQuotingStyle::Shell {
                    escape: false,
                    always_quote: false,
                    show_control: false,
                },
                "'hello world'",
            ),
            (
                OsStr::new("\x07"),
                CtQuotingStyle::Shell {
                    escape: false,
                    always_quote: false,
                    show_control: false,
                },
                "?",
            ),
            (
                OsStr::new("hello world"),
                CtQuotingStyle::Shell {
                    escape: false,
                    always_quote: false,
                    show_control: false,
                },
                "'hello world'",
            ),
            (
                OsStr::new("`hello world`"),
                CtQuotingStyle::Shell {
                    escape: false,
                    always_quote: false,
                    show_control: false,
                },
                "\'`hello world`\'",
            ),
        ];

        for (input, style, expected_output) in test_cases {
            // println!("[{}]",escape_name(input, &style));
            // println!("{}",expected_output);
            assert_eq!(escape_name(input, &style), expected_output);
        }
    }

    #[test]
    fn test_escape_name_c_style() {
        let test_cases = vec![
            (
                OsStr::new("hello world"),
                CtQuotingStyle::C {
                    quotes: CtQuotes::Double,
                },
                "\"hello world\"",
            ),
            (
                OsStr::new("'hello world'"),
                CtQuotingStyle::C {
                    quotes: CtQuotes::Single,
                },
                "'\\'hello world\\''",
            ),
            (
                OsStr::new("`hello world`"),
                CtQuotingStyle::C {
                    quotes: CtQuotes::Double,
                },
                "\"`hello world`\"",
            ),
            (
                OsStr::new("hello\\world"),
                CtQuotingStyle::C {
                    quotes: CtQuotes::None,
                },
                "hello\\\\world",
            ),
        ];

        for (input, style, expected_output) in test_cases {
            assert_eq!(escape_name(input, &style), expected_output);
        }
    }

    #[test]
    fn test_escape_name_literal() {
        let test_cases = vec![
            (
                OsStr::new("hello world"),
                CtQuotingStyle::Literal { show_control: true },
                "hello world",
            ),
            (
                OsStr::new("\x07"),
                CtQuotingStyle::Literal { show_control: true },
                "\u{07}",
            ),
            (
                OsStr::new("\x07"),
                CtQuotingStyle::Literal {
                    show_control: false,
                },
                "?",
            ),
        ];

        for (input, style, expected_output) in test_cases {
            assert_eq!(escape_name(input, &style), expected_output);
        }
    }
}
