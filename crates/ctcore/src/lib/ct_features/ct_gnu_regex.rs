/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 * syskits is licensed under Mulan PSL v2.
 */

//! Safe ownership wrapper for the GNU regex API exported by glibc.

use std::error::Error;
use std::ffi::{CStr, c_char, c_void};
use std::fmt::{Display, Formatter};
use std::sync::Mutex;

/// GNU Emacs regular-expression syntax, used by utilities such as `tac` and `ptx`.
pub const GNU_REGEX_SYNTAX_EMACS: libc::c_ulong = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GnuRegexMatch {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GnuRegexError {
    Compile(Vec<u8>),
    Search,
    RecordTooLarge,
}

impl GnuRegexError {
    pub fn compile_message(&self) -> Option<&[u8]> {
        match self {
            Self::Compile(message) => Some(message),
            _ => None,
        }
    }
}

impl Display for GnuRegexError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Compile(message) => formatter.write_str(&String::from_utf8_lossy(message)),
            Self::Search => formatter.write_str("error in regular expression search"),
            Self::RecordTooLarge => formatter.write_str("record too large"),
        }
    }
}

impl Error for GnuRegexError {}

#[derive(Debug, Clone, Copy)]
pub struct GnuRegexCompileOptions<'a> {
    syntax: libc::c_ulong,
    translate: Option<&'a [u8; 256]>,
    fastmap: bool,
}

impl GnuRegexCompileOptions<'_> {
    pub const fn emacs() -> Self {
        Self {
            syntax: GNU_REGEX_SYNTAX_EMACS,
            translate: None,
            fastmap: true,
        }
    }

    pub const fn translate<'a>(self, table: &'a [u8; 256]) -> GnuRegexCompileOptions<'a> {
        GnuRegexCompileOptions {
            syntax: self.syntax,
            translate: Some(table),
            fastmap: self.fastmap,
        }
    }

    pub const fn fastmap(mut self, enabled: bool) -> Self {
        self.fastmap = enabled;
        self
    }
}

#[repr(C)]
struct GnuRegexPattern {
    buffer: *mut c_void,
    allocated: usize,
    used: usize,
    syntax: libc::c_ulong,
    fastmap: *mut c_char,
    translate: *mut c_char,
    re_nsub: usize,
    bitfield: u8,
}

const _: () = {
    assert!(std::mem::size_of::<GnuRegexPattern>() == std::mem::size_of::<libc::regex_t>());
    assert!(std::mem::align_of::<GnuRegexPattern>() == std::mem::align_of::<libc::regex_t>());
};

#[repr(C)]
struct GnuRegexRegisters {
    num_regs: libc::c_uint,
    start: *mut libc::regoff_t,
    end: *mut libc::regoff_t,
}

impl Default for GnuRegexRegisters {
    fn default() -> Self {
        Self {
            num_regs: 0,
            start: std::ptr::null_mut(),
            end: std::ptr::null_mut(),
        }
    }
}

impl Drop for GnuRegexRegisters {
    fn drop(&mut self) {
        unsafe {
            libc::free(self.start.cast());
            libc::free(self.end.cast());
        }
    }
}

unsafe extern "C" {
    fn re_compile_pattern(
        pattern: *const c_char,
        length: usize,
        buffer: *mut GnuRegexPattern,
    ) -> *const c_char;
    fn re_compile_fastmap(buffer: *mut GnuRegexPattern) -> libc::c_int;
    fn re_search(
        buffer: *mut GnuRegexPattern,
        string: *const c_char,
        length: libc::regoff_t,
        start: libc::regoff_t,
        range: libc::regoff_t,
        registers: *mut GnuRegexRegisters,
    ) -> libc::regoff_t;
    fn re_match(
        buffer: *mut GnuRegexPattern,
        string: *const c_char,
        length: libc::regoff_t,
        start: libc::regoff_t,
        registers: *mut GnuRegexRegisters,
    ) -> libc::regoff_t;
    fn re_set_syntax(syntax: libc::c_ulong) -> libc::c_ulong;
}

static GNU_REGEX_SYNTAX_LOCK: Mutex<()> = Mutex::new(());

pub struct GnuRegex {
    compiled: GnuRegexPattern,
    _translate: Option<Box<[u8; 256]>>,
    _fastmap: Option<Box<[c_char; 256]>>,
}

impl GnuRegex {
    pub fn compile(
        pattern: &[u8],
        options: GnuRegexCompileOptions<'_>,
    ) -> Result<Self, GnuRegexError> {
        let mut translate = options.translate.map(|table| Box::new(*table));
        let mut fastmap = options.fastmap.then(|| Box::new([0; 256]));
        let mut compiled = GnuRegexPattern {
            buffer: std::ptr::null_mut(),
            allocated: 0,
            used: 0,
            syntax: 0,
            fastmap: fastmap
                .as_mut()
                .map_or(std::ptr::null_mut(), |map| map.as_mut_ptr()),
            translate: translate
                .as_mut()
                .map_or(std::ptr::null_mut(), |table| table.as_mut_ptr().cast()),
            re_nsub: 0,
            bitfield: 0,
        };

        let syntax_guard = GNU_REGEX_SYNTAX_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous_syntax = unsafe { re_set_syntax(options.syntax) };
        let error =
            unsafe { re_compile_pattern(pattern.as_ptr().cast(), pattern.len(), &mut compiled) };
        unsafe {
            re_set_syntax(previous_syntax);
        }
        drop(syntax_guard);

        if !error.is_null() {
            let message = unsafe { CStr::from_ptr(error) }.to_bytes().to_vec();
            compiled.translate = std::ptr::null_mut();
            compiled.fastmap = std::ptr::null_mut();
            unsafe {
                libc::regfree((&raw mut compiled).cast());
            }
            return Err(GnuRegexError::Compile(message));
        }
        if options.fastmap && unsafe { re_compile_fastmap(&mut compiled) } == -2 {
            compiled.translate = std::ptr::null_mut();
            compiled.fastmap = std::ptr::null_mut();
            unsafe {
                libc::regfree((&raw mut compiled).cast());
            }
            return Err(GnuRegexError::Search);
        }

        Ok(Self {
            compiled,
            _translate: translate,
            _fastmap: fastmap,
        })
    }

    pub fn search(
        &mut self,
        data: &[u8],
        start: usize,
        range: isize,
    ) -> Result<Option<GnuRegexMatch>, GnuRegexError> {
        let length =
            libc::regoff_t::try_from(data.len()).map_err(|_| GnuRegexError::RecordTooLarge)?;
        let start = libc::regoff_t::try_from(start).map_err(|_| GnuRegexError::RecordTooLarge)?;
        let range = libc::regoff_t::try_from(range).map_err(|_| GnuRegexError::RecordTooLarge)?;
        let mut registers = GnuRegexRegisters::default();
        let found = unsafe {
            re_search(
                &mut self.compiled,
                data.as_ptr().cast(),
                length,
                start,
                range,
                &mut registers,
            )
        };
        match found {
            -1 => Ok(None),
            -2 => Err(GnuRegexError::Search),
            _ if found >= 0 => match_from_registers(&registers).map(Some),
            _ => Err(GnuRegexError::Search),
        }
    }

    pub fn search_backward(&mut self, data: &[u8]) -> Result<Option<GnuRegexMatch>, GnuRegexError> {
        if data.is_empty() {
            return Ok(None);
        }
        let range = 1isize
            .checked_sub(isize::try_from(data.len()).map_err(|_| GnuRegexError::RecordTooLarge)?)
            .ok_or(GnuRegexError::RecordTooLarge)?;
        self.search(data, data.len() - 1, range)
    }

    pub fn match_at(
        &mut self,
        data: &[u8],
        start: usize,
    ) -> Result<Option<GnuRegexMatch>, GnuRegexError> {
        let length =
            libc::regoff_t::try_from(data.len()).map_err(|_| GnuRegexError::RecordTooLarge)?;
        let start_offset =
            libc::regoff_t::try_from(start).map_err(|_| GnuRegexError::RecordTooLarge)?;
        let matched = unsafe {
            re_match(
                &mut self.compiled,
                data.as_ptr().cast(),
                length,
                start_offset,
                std::ptr::null_mut(),
            )
        };
        match matched {
            -1 => Ok(None),
            -2 => Err(GnuRegexError::Search),
            length if length >= 0 => Ok(Some(GnuRegexMatch {
                start,
                end: start + length as usize,
            })),
            _ => Err(GnuRegexError::Search),
        }
    }
}

fn match_from_registers(registers: &GnuRegexRegisters) -> Result<GnuRegexMatch, GnuRegexError> {
    if registers.start.is_null() || registers.end.is_null() || registers.num_regs == 0 {
        return Err(GnuRegexError::Search);
    }
    let match_start = unsafe { *registers.start };
    let match_end = unsafe { *registers.end };
    if match_start < 0 || match_end < match_start {
        return Err(GnuRegexError::Search);
    }
    Ok(GnuRegexMatch {
        start: match_start as usize,
        end: match_end as usize,
    })
}

impl Drop for GnuRegex {
    fn drop(&mut self) {
        self.compiled.translate = std::ptr::null_mut();
        self.compiled.fastmap = std::ptr::null_mut();
        unsafe {
            libc::regfree((&raw mut self.compiled).cast());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emacs_syntax_supports_backreferences_and_backward_search() {
        let mut regex = GnuRegex::compile(b"\\(..\\)\\1", GnuRegexCompileOptions::emacs()).unwrap();

        let found = regex.search_backward(b"ababXcdcd").unwrap().unwrap();

        assert_eq!(found, GnuRegexMatch { start: 5, end: 9 });
        let found = regex.search_backward(b"ababX").unwrap().unwrap();
        assert_eq!(found, GnuRegexMatch { start: 0, end: 4 });
    }

    #[test]
    fn match_at_uses_gnu_match_semantics() {
        let mut regex = GnuRegex::compile(b"a+", GnuRegexCompileOptions::emacs()).unwrap();

        assert_eq!(
            regex.match_at(b"xaa", 1).unwrap(),
            Some(GnuRegexMatch { start: 1, end: 3 })
        );
        assert_eq!(regex.match_at(b"xaa", 0).unwrap(), None);
    }

    #[test]
    fn translation_table_is_owned_by_the_compiled_pattern() {
        let mut upper = Box::new([0u8; 256]);
        for (value, translated) in upper.iter_mut().enumerate() {
            *translated = (value as u8).to_ascii_uppercase();
        }
        let mut regex =
            GnuRegex::compile(b"ABC", GnuRegexCompileOptions::emacs().translate(&upper)).unwrap();
        drop(upper);

        assert!(regex.match_at(b"abc", 0).unwrap().is_some());
    }

    #[test]
    fn compile_error_preserves_glibc_diagnostic_bytes() {
        let error = GnuRegex::compile(b"[", GnuRegexCompileOptions::emacs())
            .err()
            .unwrap();

        assert!(
            error
                .compile_message()
                .is_some_and(|message| !message.is_empty())
        );
    }
}
