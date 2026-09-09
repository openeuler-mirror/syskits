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

// spell-checker:ignore (ToDOs) corasick memchr Roff trunc oset iset CHARCLASS

//! PTX (Permuted Index) 实现
//!
//! 该模块实现了类似于 GNU PTX 的排列索引功能。它可以从输入文件中提取关键词，
//! 并生成一个排序的索引，每个关键词都显示在其上下文中。
//!
//! 主要功能:
//! - 从文件或标准输入读取文本
//! - 提取和过滤关键词
//! - 生成格式化的输出(支持 Roff 和 TeX 格式)
//! - 提供引用和上下文显示

extern crate rust_i18n;
use clap::{
    Arg, ArgAction, Command, builder::OsStringValueParser, crate_version, error::ErrorKind,
};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::Tool;
use ctcore::ct_error::{CTError, CTResult, CTsageError, CtSimpleError, FromIo, strip_errno};
use ctcore::ct_shortcut_value_parser::CtShortcutValueParser;
use onig::{EncodedBytes, Regex as OnigRegex, RegexOptions, Region, SearchOptions, Syntax};
use std::borrow::Cow;
use std::collections::{BTreeSet, HashSet};
use std::ffi::{CStr, CString, OsStr, OsString};
use std::fmt::Write as FmtWrite;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write, stdout};
use std::os::unix::ffi::OsStrExt;
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::{Arc, OnceLock};
use sys_locale::get_locale;

const REGEX_CHARCLASS: &str = "^-]\\";
const GNU_DEFAULT_CONTEXT_PATTERN: &[u8] = b"[.?!][]\"')}]*\\($\\|\t\\|  \\)[ \t\n]*";
const GNU_DEFAULT_CONTEXT_REGEX: &str = r#"(?m)[.?!][\]\"')}]*($|\t|  )[ \t\n]*"#;
const NEVER_MATCH_REGEX: &str = r"[^\s\S]";

type GlibcReCompilePattern = unsafe extern "C" fn(
    *const ctcore::libc::c_char,
    usize,
    *mut GlibcRegexPattern,
) -> *const ctcore::libc::c_char;
type GlibcReMatch = unsafe extern "C" fn(
    *mut GlibcRegexPattern,
    *const ctcore::libc::c_char,
    ctcore::libc::regoff_t,
    ctcore::libc::regoff_t,
    *mut ctcore::libc::c_void,
) -> ctcore::libc::regoff_t;

#[repr(C)]
struct GlibcRegexPattern {
    buffer: *mut ctcore::libc::c_void,
    allocated: usize,
    used: usize,
    syntax: ctcore::libc::c_ulong,
    fastmap: *mut ctcore::libc::c_char,
    translate: *mut ctcore::libc::c_char,
    re_nsub: usize,
    bitfield: u8,
}

const _: () = {
    assert!(
        std::mem::size_of::<GlibcRegexPattern>() == std::mem::size_of::<ctcore::libc::regex_t>()
    );
    assert!(
        std::mem::align_of::<GlibcRegexPattern>() == std::mem::align_of::<ctcore::libc::regex_t>()
    );
};

struct GlibcRegexApi {
    compile_pattern: GlibcReCompilePattern,
    is_match: GlibcReMatch,
}

fn glibc_regex_api() -> Option<&'static GlibcRegexApi> {
    static API: OnceLock<Option<GlibcRegexApi>> = OnceLock::new();

    API.get_or_init(|| unsafe {
        // Oniguruma exports ABI-incompatible GNU regex compatibility symbols.
        // Resolve the glibc implementation from its own handle explicitly.
        let handle = ctcore::libc::dlopen(
            c"libc.so.6".as_ptr(),
            ctcore::libc::RTLD_LAZY | ctcore::libc::RTLD_LOCAL,
        );
        if handle.is_null() {
            return None;
        }
        let compile_pattern = ctcore::libc::dlsym(handle, c"re_compile_pattern".as_ptr());
        let is_match = ctcore::libc::dlsym(handle, c"re_match".as_ptr());
        if compile_pattern.is_null() || is_match.is_null() {
            ctcore::libc::dlclose(handle);
            return None;
        }
        Some(GlibcRegexApi {
            compile_pattern: std::mem::transmute::<
                *mut ctcore::libc::c_void,
                GlibcReCompilePattern,
            >(compile_pattern),
            is_match: std::mem::transmute::<*mut ctcore::libc::c_void, GlibcReMatch>(is_match),
        })
    })
    .as_ref()
}

unsafe extern "C" {
    fn fnmatch(
        pattern: *const ctcore::libc::c_char,
        string: *const ctcore::libc::c_char,
        flags: ctcore::libc::c_int,
    ) -> ctcore::libc::c_int;
    fn mbrtowc(
        wide: *mut ctcore::libc::wchar_t,
        bytes: *const ctcore::libc::c_char,
        length: usize,
        state: *mut ctcore::libc::mbstate_t,
    ) -> usize;
}

fn ptx_is_space_byte(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

fn ptx_is_space_char(ch: char) -> bool {
    u8::try_from(ch).is_ok_and(ptx_is_space_byte)
}

fn ptx_write_error_context() -> String {
    "write error".to_string()
}

fn ptx_locale_name() -> OsString {
    ptx_locale_category_name("LC_CTYPE")
}

fn ptx_locale_category_name(category: &str) -> OsString {
    ["LC_ALL", category, "LANG"]
        .into_iter()
        .find_map(|name| std::env::var_os(name).filter(|value| !value.is_empty()))
        .unwrap_or_else(|| OsString::from("C"))
}

fn ptx_is_single_byte_locale() -> bool {
    let codeset = ptx_locale_codeset();
    matches!(codeset.as_str(), "ANSI_X3.4-1968" | "ASCII")
        || codeset.contains("ISO-8859")
        || codeset.contains("ISO8859")
}

fn ptx_locale_codeset() -> String {
    let fallback = ptx_locale_name()
        .to_string_lossy()
        .trim()
        .to_ascii_uppercase();
    let Ok(locale_name) = CString::new(ptx_locale_name().as_encoded_bytes()) else {
        return fallback;
    };
    unsafe {
        let locale = ctcore::libc::newlocale(
            ctcore::libc::LC_CTYPE_MASK,
            locale_name.as_ptr(),
            std::ptr::null_mut(),
        );
        if locale.is_null() {
            // GNU ptx leaves the process in its startup C locale when
            // setlocale(LC_ALL, "") rejects the environment locale.
            return "ASCII".to_string();
        }
        let codeset = ctcore::libc::nl_langinfo_l(ctcore::libc::CODESET, locale);
        let value = if codeset.is_null() {
            fallback
        } else {
            CStr::from_ptr(codeset)
                .to_string_lossy()
                .to_ascii_uppercase()
        };
        ctcore::libc::freelocale(locale);
        value
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum LocaleRegexEncoding {
    #[default]
    Ascii,
    SingleByte,
    EucCn,
    EucTw,
    Big5,
    Big5Hkscs,
    Gb18030,
}

#[derive(Debug)]
struct LocaleMultibyteValidator {
    locale: usize,
    allow_high_single_byte: bool,
}

impl LocaleMultibyteValidator {
    fn from_environment() -> Option<Self> {
        let codeset = ptx_locale_codeset();
        let locale_name = CString::new(ptx_locale_name().as_encoded_bytes()).ok()?;
        let locale = unsafe {
            ctcore::libc::newlocale(
                ctcore::libc::LC_CTYPE_MASK,
                locale_name.as_ptr(),
                std::ptr::null_mut(),
            )
        };
        if locale.is_null() {
            None
        } else {
            Some(Self {
                locale: locale as usize,
                allow_high_single_byte: codeset.contains("BIG5"),
            })
        }
    }

    fn decode_character(&self, bytes: &[u8]) -> Option<(usize, char)> {
        let &first = bytes.first()?;
        if first.is_ascii() {
            return Some((1, char::from(first)));
        }
        unsafe {
            let locale = self.locale as ctcore::libc::locale_t;
            let previous = ctcore::libc::uselocale(locale);
            let mut state: ctcore::libc::mbstate_t = std::mem::zeroed();
            let mut wide: ctcore::libc::wchar_t = 0;
            let length = mbrtowc(&mut wide, bytes.as_ptr().cast(), bytes.len(), &mut state);
            ctcore::libc::uselocale(previous);
            if length == usize::MAX
                || length == usize::MAX - 1
                || length == 0
                || (length == 1 && !self.allow_high_single_byte)
            {
                None
            } else {
                char::from_u32(wide).map(|character| (length, character))
            }
        }
    }

    fn valid_character_len(&self, bytes: &[u8]) -> Option<usize> {
        self.decode_character(bytes).map(|(length, _)| length)
    }

    fn is_word_character(&self, codepoint: u32) -> bool {
        codepoint == u32::from(b'_')
            || unsafe {
                iswalnum_l(
                    codepoint as ctcore::libc::c_uint,
                    self.locale as ctcore::libc::locale_t,
                ) != 0
            }
    }

    fn is_space_character(&self, codepoint: u32) -> bool {
        unsafe {
            iswspace_l(
                codepoint as ctcore::libc::c_uint,
                self.locale as ctcore::libc::locale_t,
            ) != 0
        }
    }
}

impl Drop for LocaleMultibyteValidator {
    fn drop(&mut self) {
        unsafe { ctcore::libc::freelocale(self.locale as ctcore::libc::locale_t) };
    }
}

#[derive(Debug)]
struct LocaleCollation {
    locale: usize,
}

impl LocaleCollation {
    fn c_locale() -> Option<ctcore::libc::locale_t> {
        let locale = unsafe {
            ctcore::libc::newlocale(
                ctcore::libc::LC_ALL_MASK,
                c"C".as_ptr(),
                std::ptr::null_mut(),
            )
        };
        (!locale.is_null()).then_some(locale)
    }

    fn from_environment() -> Option<Self> {
        let mut locale = Self::c_locale()?;
        for (category, mask) in [
            ("LC_CTYPE", ctcore::libc::LC_CTYPE_MASK),
            ("LC_COLLATE", ctcore::libc::LC_COLLATE_MASK),
        ] {
            let name = CString::new(ptx_locale_category_name(category).as_encoded_bytes()).ok()?;
            let combined = unsafe { ctcore::libc::newlocale(mask, name.as_ptr(), locale) };
            if combined.is_null() {
                unsafe { ctcore::libc::freelocale(locale) };
                return Self::c_locale().map(|locale| Self {
                    locale: locale as usize,
                });
            }
            locale = combined;
        }
        Some(Self {
            locale: locale as usize,
        })
    }

    fn range_members(&self, start: u8, end: u8) -> Option<[bool; 256]> {
        let pattern = [b'[', start, b'-', end, b']', 0];
        let mut members = [false; 256];
        let previous = unsafe { ctcore::libc::uselocale(self.locale as ctcore::libc::locale_t) };
        for value in 1u16..=255 {
            let candidate = [value as u8, 0];
            members[usize::from(value)] =
                unsafe { fnmatch(pattern.as_ptr().cast(), candidate.as_ptr().cast(), 0) == 0 };
        }
        unsafe { ctcore::libc::uselocale(previous) };
        members.iter().any(|&member| member).then_some(members)
    }

    fn matching_candidates(
        &self,
        pattern: &[u8],
        candidates: &[Vec<u8>],
        fold_upper: Option<&[u8; 256]>,
    ) -> Option<Vec<Vec<u8>>> {
        let regex_api = glibc_regex_api()?;
        let previous = unsafe { ctcore::libc::uselocale(self.locale as ctcore::libc::locale_t) };
        let mut regex: GlibcRegexPattern = unsafe { std::mem::zeroed() };
        regex.translate = fold_upper.map_or(std::ptr::null_mut(), |upper| {
            upper.as_ptr().cast_mut().cast()
        });
        let compile_error = unsafe {
            (regex_api.compile_pattern)(pattern.as_ptr().cast(), pattern.len(), &mut regex)
        };
        if !compile_error.is_null() {
            unsafe { ctcore::libc::uselocale(previous) };
            return None;
        }
        let mut matches = Vec::new();
        for candidate in candidates
            .iter()
            .filter(|candidate| !candidate.contains(&0))
        {
            let Ok(length) = ctcore::libc::regoff_t::try_from(candidate.len()) else {
                continue;
            };
            if unsafe {
                (regex_api.is_match)(
                    &mut regex,
                    candidate.as_ptr().cast(),
                    length,
                    0,
                    std::ptr::null_mut(),
                ) >= 0
            } {
                let mut matching = candidate.clone();
                if let Some(upper) = fold_upper {
                    for byte in &mut matching {
                        *byte = upper[usize::from(*byte)];
                    }
                }
                matches.push(matching);
            }
        }
        matches.sort_unstable();
        matches.dedup();
        regex.translate = std::ptr::null_mut();
        unsafe { ctcore::libc::regfree((&raw mut regex).cast()) };
        unsafe { ctcore::libc::uselocale(previous) };
        Some(matches)
    }

    fn compile_error(&self, pattern: &[u8], fold_upper: Option<&[u8; 256]>) -> Option<Vec<u8>> {
        let regex_api = glibc_regex_api()?;
        let previous = unsafe { ctcore::libc::uselocale(self.locale as ctcore::libc::locale_t) };
        let mut regex: GlibcRegexPattern = unsafe { std::mem::zeroed() };
        regex.translate = fold_upper.map_or(std::ptr::null_mut(), |upper| {
            upper.as_ptr().cast_mut().cast()
        });
        let compile_error = unsafe {
            (regex_api.compile_pattern)(pattern.as_ptr().cast(), pattern.len(), &mut regex)
        };
        let message = (!compile_error.is_null())
            .then(|| unsafe { CStr::from_ptr(compile_error) }.to_bytes().to_vec());
        regex.translate = std::ptr::null_mut();
        unsafe { ctcore::libc::regfree((&raw mut regex).cast()) };
        unsafe { ctcore::libc::uselocale(previous) };
        message
    }
}

impl Drop for LocaleCollation {
    fn drop(&mut self) {
        unsafe { ctcore::libc::freelocale(self.locale as ctcore::libc::locale_t) };
    }
}

impl LocaleRegexEncoding {
    fn from_environment() -> Self {
        let codeset = ptx_locale_codeset();
        if codeset.contains("GB18030") || codeset.contains("GBK") {
            Self::Gb18030
        } else if codeset.contains("GB2312") {
            Self::EucCn
        } else if codeset.contains("EUC-TW") || codeset.contains("EUCTW") {
            Self::EucTw
        } else if codeset.contains("BIG5-HKSCS") || codeset.contains("BIG5HKSCS") {
            Self::Big5Hkscs
        } else if codeset.contains("BIG5") {
            Self::Big5
        } else if codeset.contains("ISO-8859") || codeset.contains("ISO8859") {
            Self::SingleByte
        } else {
            Self::Ascii
        }
    }

    fn encoded(self, bytes: &[u8]) -> EncodedBytes<'_> {
        let Some(encoding) = self.onig_encoding() else {
            return EncodedBytes::ascii(bytes);
        };
        EncodedBytes::from_parts(bytes, encoding)
    }

    fn onig_encoding(self) -> Option<onig_sys::OnigEncoding> {
        let encoding = match self {
            Self::Ascii => return None,
            Self::SingleByte => std::ptr::addr_of_mut!(onig_sys::OnigEncodingISO_8859_1),
            Self::EucCn => std::ptr::addr_of_mut!(onig_sys::OnigEncodingEUC_CN),
            Self::EucTw => std::ptr::addr_of_mut!(onig_sys::OnigEncodingEUC_TW),
            Self::Big5 => std::ptr::addr_of_mut!(onig_sys::OnigEncodingBIG5),
            // HKSCS retains Big5's two-byte structure but extends its lead-byte range.
            // GB18030 accepts that superset; LocaleMultibyteValidator still enforces
            // the actual glibc Big5-HKSCS character repertoire before a match is used.
            Self::Big5Hkscs => std::ptr::addr_of_mut!(onig_sys::OnigEncodingGB18030),
            Self::Gb18030 => std::ptr::addr_of_mut!(onig_sys::OnigEncodingGB18030),
        };
        Some(encoding)
    }

    fn is_non_utf8_multibyte(self) -> bool {
        !matches!(self, Self::Ascii | Self::SingleByte)
    }

    fn valid_character_len(self, bytes: &[u8]) -> Option<usize> {
        let &first = bytes.first()?;
        if first.is_ascii() || matches!(self, Self::Ascii | Self::SingleByte) {
            return Some(1);
        }
        let encoding = self.onig_encoding()?;
        let mut probe = [0u8; 4];
        let available = bytes.len().min(probe.len());
        probe[..available].copy_from_slice(&bytes[..available]);
        unsafe {
            let encoding = &*encoding;
            let encoded_length = encoding
                .mbc_enc_len
                .expect("Oniguruma encoding must provide character lengths");
            let is_valid = encoding
                .is_valid_mbc_string
                .expect("Oniguruma encoding must validate byte strings");
            let length = usize::try_from(encoded_length(probe.as_ptr())).ok()?;
            if length <= 1 || length > bytes.len() || length > probe.len() {
                return None;
            }
            (is_valid(probe.as_ptr(), probe.as_ptr().add(length)) != 0).then_some(length)
        }
    }

    fn mark_valid_multibyte_sequences(self, bytes: &[u8], printable: &mut [bool]) {
        let Some(encoding) = self.onig_encoding() else {
            return;
        };
        let mut padded = bytes.to_vec();
        padded.extend_from_slice(&[0; 4]);
        let mut index = 0usize;
        unsafe {
            let encoding = &*encoding;
            let encoded_length = encoding
                .mbc_enc_len
                .expect("Oniguruma encoding must provide character lengths");
            let is_valid = encoding
                .is_valid_mbc_string
                .expect("Oniguruma encoding must validate byte strings");
            while index < bytes.len() {
                if bytes[index].is_ascii() {
                    index += 1;
                    continue;
                }
                let length = encoded_length(padded.as_ptr().add(index));
                let Ok(length) = usize::try_from(length) else {
                    index += 1;
                    continue;
                };
                if length <= 1 || index + length > bytes.len() {
                    index += 1;
                    continue;
                }
                let start = padded.as_ptr().add(index);
                if is_valid(start, start.add(length)) != 0 {
                    printable[index..index + length].fill(true);
                    index += length;
                } else {
                    index += 1;
                }
            }
        }
    }
}

#[derive(Debug)]
struct LocaleByteCtype {
    alpha: [bool; 256],
    print: [bool; 256],
    space: [bool; 256],
    upper: [u8; 256],
}

impl LocaleByteCtype {
    fn from_environment() -> Self {
        let mut table = Self {
            alpha: std::array::from_fn(|index| (index as u8).is_ascii_alphabetic()),
            print: std::array::from_fn(|index| (index as u8).is_ascii_graphic() || index == 32),
            space: std::array::from_fn(|index| ptx_is_space_byte(index as u8)),
            upper: std::array::from_fn(|index| (index as u8).to_ascii_uppercase()),
        };
        let Ok(locale_name) = CString::new(ptx_locale_name().as_encoded_bytes()) else {
            return table;
        };

        unsafe {
            let locale = ctcore::libc::newlocale(
                ctcore::libc::LC_CTYPE_MASK,
                locale_name.as_ptr(),
                std::ptr::null_mut(),
            );
            if locale.is_null() {
                return table;
            }
            for byte in 0u16..=255 {
                table.alpha[usize::from(byte)] = isalpha_l(i32::from(byte), locale) != 0;
                table.print[usize::from(byte)] = isprint_l(i32::from(byte), locale) != 0;
                table.space[usize::from(byte)] = isspace_l(i32::from(byte), locale) != 0;
                table.upper[usize::from(byte)] = toupper_l(i32::from(byte), locale) as u8;
            }
            ctcore::libc::freelocale(locale);
        }
        table
    }

    fn is_alpha(&self, byte: u8) -> bool {
        self.alpha[usize::from(byte)]
    }

    fn is_print(&self, byte: u8) -> bool {
        self.print[usize::from(byte)]
    }

    fn is_space(&self, byte: u8) -> bool {
        self.space[usize::from(byte)]
    }

    fn uppercase(&self, bytes: &mut [u8]) {
        for byte in bytes {
            *byte = self.upper[usize::from(*byte)];
        }
    }
}

unsafe extern "C" {
    fn isalpha_l(
        character: ctcore::libc::c_int,
        locale: ctcore::libc::locale_t,
    ) -> ctcore::libc::c_int;
    fn isprint_l(
        character: ctcore::libc::c_int,
        locale: ctcore::libc::locale_t,
    ) -> ctcore::libc::c_int;
    fn isspace_l(
        character: ctcore::libc::c_int,
        locale: ctcore::libc::locale_t,
    ) -> ctcore::libc::c_int;
    fn toupper_l(
        character: ctcore::libc::c_int,
        locale: ctcore::libc::locale_t,
    ) -> ctcore::libc::c_int;
    fn iswalnum_l(
        character: ctcore::libc::c_uint,
        locale: ctcore::libc::locale_t,
    ) -> ctcore::libc::c_int;
    fn iswspace_l(
        character: ctcore::libc::c_uint,
        locale: ctcore::libc::locale_t,
    ) -> ctcore::libc::c_int;
}

fn ptx_push_unicode_range(class: &mut String, start: u32, end: u32) {
    write!(class, r"\x{{{start:X}}}").expect("writing to a String cannot fail");
    if end != start {
        write!(class, r"-\x{{{end:X}}}").expect("writing to a String cannot fail");
    }
}

fn ptx_class_pair(positive: Vec<u8>) -> (Vec<u8>, Vec<u8>) {
    let mut negative = Vec::with_capacity(positive.len() + 1);
    negative.extend_from_slice(b"[^");
    negative.extend_from_slice(&positive[1..]);
    (positive, negative)
}

fn ptx_single_byte_classes(mut is_member: impl FnMut(u8) -> bool) -> (Vec<u8>, Vec<u8>) {
    let mut class = vec![b'['];
    let mut range_start = None;
    for value in 0u16..=256 {
        let member = value < 256 && is_member(value as u8);
        match (range_start, member) {
            (None, true) => range_start = Some(value),
            (Some(start), false) => {
                class.push(start as u8);
                if value - 1 != start {
                    class.push(b'-');
                    class.push((value - 1) as u8);
                }
                range_start = None;
            }
            _ => {}
        }
    }
    class.push(b']');
    ptx_class_pair(class)
}

fn ptx_unicode_classes(mut is_member: impl FnMut(u32) -> bool) -> (Vec<u8>, Vec<u8>) {
    let mut class = String::from("[");
    let mut range_start = None;
    for codepoint in 0..=0x11_0000 {
        let member =
            char::from_u32(codepoint).is_some() && codepoint <= 0x10_ffff && is_member(codepoint);
        match (range_start, member) {
            (None, true) => range_start = Some(codepoint),
            (Some(start), false) => {
                ptx_push_unicode_range(&mut class, start, codepoint - 1);
                range_start = None;
            }
            _ => {}
        }
    }
    class.push(']');
    ptx_class_pair(class.into_bytes())
}

fn ptx_locale_word_classes(
    single_byte_locale: bool,
    encoding: LocaleRegexEncoding,
    byte_ctype: &LocaleByteCtype,
    locale_validator: Option<&LocaleMultibyteValidator>,
) -> (Vec<u8>, Vec<u8>) {
    if single_byte_locale {
        return ptx_single_byte_classes(|byte| {
            byte_ctype.is_alpha(byte) || byte.is_ascii_digit() || byte == b'_'
        });
    }
    if encoding == LocaleRegexEncoding::Ascii || encoding.is_non_utf8_multibyte() {
        let Some(validator) = locale_validator else {
            return (b"[[:alnum:]_]".to_vec(), b"[^[:alnum:]_]".to_vec());
        };
        return ptx_unicode_classes(|codepoint| validator.is_word_character(codepoint));
    }
    (b"[[:alnum:]_]".to_vec(), b"[^[:alnum:]_]".to_vec())
}

fn ptx_locale_space_classes(
    single_byte_locale: bool,
    encoding: LocaleRegexEncoding,
    byte_ctype: &LocaleByteCtype,
    locale_validator: Option<&LocaleMultibyteValidator>,
) -> (Vec<u8>, Vec<u8>) {
    if single_byte_locale {
        return ptx_single_byte_classes(|byte| byte_ctype.is_space(byte));
    }
    if encoding == LocaleRegexEncoding::Ascii || encoding.is_non_utf8_multibyte() {
        let Some(validator) = locale_validator else {
            return (b"[[:space:]]".to_vec(), b"[^[:space:]]".to_vec());
        };
        return ptx_unicode_classes(|codepoint| validator.is_space_character(codepoint));
    }
    (b"[[:space:]]".to_vec(), b"[^[:space:]]".to_vec())
}

#[derive(Debug)]
struct Regex {
    search: OnigRegex,
    longest: OnigRegex,
}

impl Regex {
    #[cfg(test)]
    fn new(pattern: &str) -> Result<Self, onig::Error> {
        compile_regex(pattern, false)
    }

    fn find(&self, text: &str) -> Option<(usize, usize)> {
        self.find_at(text, 0)
    }

    fn find_at(&self, text: &str, from: usize) -> Option<(usize, usize)> {
        let mut region = Region::new();
        let start = self.search.search_with_options(
            text,
            from,
            text.len(),
            SearchOptions::SEARCH_OPTION_NONE,
            Some(&mut region),
        )?;
        let mut longest_region = Region::new();
        self.longest.search_with_options(
            text,
            start,
            text.len(),
            SearchOptions::SEARCH_OPTION_NONE,
            Some(&mut longest_region),
        )?;
        longest_region.pos(0)
    }

    fn find_iter<'r, 't>(&'r self, text: &'t str) -> RegexFindIter<'r, 't> {
        RegexFindIter {
            regex: self,
            text,
            next_start: 0,
            previous_end: None,
        }
    }
}

struct RegexFindIter<'r, 't> {
    regex: &'r Regex,
    text: &'t str,
    next_start: usize,
    previous_end: Option<usize>,
}

impl Iterator for RegexFindIter<'_, '_> {
    type Item = (usize, usize);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.next_start > self.text.len() {
                return None;
            }
            let base = self.next_start;
            let (start, end) = self.regex.find(&self.text[base..])?;
            let (start, end) = (base + start, base + end);
            if start == end && self.previous_end == Some(end) {
                let next = self.text[end..]
                    .chars()
                    .next()
                    .map_or(self.text.len() + 1, |ch| end + ch.len_utf8());
                self.next_start = next;
                continue;
            }
            self.previous_end = Some(end);
            self.next_start = end;
            return Some((start, end));
        }
    }
}

#[derive(Debug)]
struct ByteRegex {
    search: OnigRegex,
    longest: OnigRegex,
    fold_upper: Option<[u8; 256]>,
    encoding: LocaleRegexEncoding,
    locale_validator: Option<Arc<LocaleMultibyteValidator>>,
    transcode_locale: bool,
    single_byte_locale: bool,
}

struct LocaleUtf8Text {
    text: String,
    boundaries: Vec<(usize, usize)>,
    invalid_raw_offsets: Vec<usize>,
}

impl LocaleUtf8Text {
    fn from_bytes(bytes: &[u8], validator: &LocaleMultibyteValidator) -> Self {
        let mut text = String::with_capacity(bytes.len());
        let mut boundaries = Vec::with_capacity(bytes.len() + 1);
        let mut invalid_raw_offsets = Vec::new();
        let mut raw_offset = 0usize;
        boundaries.push((0, 0));
        while raw_offset < bytes.len() {
            let (length, character) = validator
                .decode_character(&bytes[raw_offset..])
                .unwrap_or_else(|| {
                    invalid_raw_offsets.push(raw_offset);
                    (
                        1,
                        char::from_u32(0xe000 + u32::from(bytes[raw_offset])).unwrap(),
                    )
                });
            text.push(character);
            raw_offset += length;
            boundaries.push((raw_offset, text.len()));
        }
        Self {
            text,
            boundaries,
            invalid_raw_offsets,
        }
    }

    fn utf8_offset_at_or_after(&self, raw_offset: usize) -> Option<usize> {
        let index = self
            .boundaries
            .partition_point(|&(raw, _)| raw < raw_offset);
        self.boundaries.get(index).map(|&(_, utf8)| utf8)
    }

    fn raw_offset(&self, utf8_offset: usize) -> Option<usize> {
        self.boundaries
            .binary_search_by_key(&utf8_offset, |&(_, utf8)| utf8)
            .ok()
            .map(|index| self.boundaries[index].0)
    }

    fn first_invalid_at_or_after(&self, raw_offset: usize) -> Option<usize> {
        let index = self
            .invalid_raw_offsets
            .partition_point(|&invalid| invalid < raw_offset);
        self.invalid_raw_offsets.get(index).copied()
    }

    fn contains_invalid(&self, start: usize, end: usize) -> bool {
        self.first_invalid_at_or_after(start)
            .is_some_and(|invalid| invalid < end)
    }
}

impl ByteRegex {
    fn valid_character_len(&self, bytes: &[u8]) -> Option<usize> {
        if self.single_byte_locale || self.encoding == LocaleRegexEncoding::SingleByte {
            return self.encoding.valid_character_len(bytes);
        }
        match &self.locale_validator {
            Some(validator) => validator.valid_character_len(bytes),
            None => self.encoding.valid_character_len(bytes),
        }
    }

    fn is_valid_match_range(&self, bytes: &[u8], start: usize, end: usize) -> bool {
        if self.single_byte_locale || self.encoding == LocaleRegexEncoding::SingleByte {
            return true;
        }
        let mut cursor = 0usize;
        let mut start_is_boundary = start == 0;
        let mut end_is_boundary = end == 0;
        let mut contains_invalid_byte = false;
        while cursor < bytes.len() {
            if cursor == start {
                start_is_boundary = true;
            }
            if cursor == end {
                end_is_boundary = true;
            }
            match self.valid_character_len(&bytes[cursor..]) {
                Some(length) => cursor += length,
                None => {
                    if (start..end).contains(&cursor) {
                        contains_invalid_byte = true;
                    }
                    cursor += 1;
                }
            }
        }
        if cursor == start {
            start_is_boundary = true;
        }
        if cursor == end {
            end_is_boundary = true;
        }
        start_is_boundary && end_is_boundary && !contains_invalid_byte
    }

    fn first_invalid_byte_at_or_after(&self, bytes: &[u8], from: usize) -> Option<usize> {
        if self.single_byte_locale || self.encoding == LocaleRegexEncoding::SingleByte {
            return None;
        }
        let mut cursor = 0usize;
        while cursor < bytes.len() {
            match self.valid_character_len(&bytes[cursor..]) {
                Some(length) => cursor += length,
                None if cursor >= from => return Some(cursor),
                None => cursor += 1,
            }
        }
        None
    }

    fn folded_bytes<'a>(&self, bytes: &'a [u8]) -> Cow<'a, [u8]> {
        let Some(upper) = &self.fold_upper else {
            return Cow::Borrowed(bytes);
        };
        let mut folded = bytes.to_vec();
        for byte in &mut folded {
            if !byte.is_ascii() {
                *byte = upper[usize::from(*byte)];
            }
        }
        Cow::Owned(folded)
    }

    fn find(&self, bytes: &[u8]) -> Option<(usize, usize)> {
        self.find_at(bytes, 0)
    }

    fn find_at(&self, bytes: &[u8], from: usize) -> Option<(usize, usize)> {
        let folded = self.folded_bytes(bytes);
        let bytes = folded.as_ref();
        if self.transcode_locale {
            return self.find_at_transcoded(bytes, from);
        }
        let mut search_from = from;
        while search_from <= bytes.len() {
            let mut region = Region::new();
            let start = self.search.search_with_encoding(
                self.encoding.encoded(bytes),
                search_from,
                bytes.len(),
                SearchOptions::SEARCH_OPTION_NONE,
                Some(&mut region),
            );
            let Some(start) = start else {
                let invalid = self.first_invalid_byte_at_or_after(bytes, search_from)?;
                search_from = invalid + 1;
                continue;
            };
            if let Some(invalid) = self.first_invalid_byte_at_or_after(bytes, search_from)
                && invalid < start
            {
                search_from = invalid + 1;
                continue;
            }
            let mut longest_region = Region::new();
            self.longest.search_with_encoding(
                self.encoding.encoded(bytes),
                start,
                bytes.len(),
                SearchOptions::SEARCH_OPTION_NONE,
                Some(&mut longest_region),
            )?;
            let (_, end) = longest_region.pos(0)?;
            if self.is_valid_match_range(bytes, start, end) {
                return Some((start, end));
            }
            search_from = start + 1;
        }
        None
    }

    fn find_at_transcoded(&self, bytes: &[u8], from: usize) -> Option<(usize, usize)> {
        let validator = self.locale_validator.as_ref()?;
        let transcoded = LocaleUtf8Text::from_bytes(bytes, validator);
        self.find_at_transcoded_text(bytes, 0, from, &transcoded)
    }

    fn find_at_transcoded_text(
        &self,
        bytes: &[u8],
        raw_base: usize,
        from: usize,
        transcoded: &LocaleUtf8Text,
    ) -> Option<(usize, usize)> {
        let utf8_base = transcoded.utf8_offset_at_or_after(raw_base)?;
        let text = &transcoded.text[utf8_base..];
        let mut search_from = from;
        while search_from <= bytes.len() {
            let utf8_from = transcoded
                .utf8_offset_at_or_after(search_from)?
                .saturating_sub(utf8_base);
            let mut region = Region::new();
            let utf8_start = self.search.search_with_options(
                text,
                utf8_from,
                text.len(),
                SearchOptions::SEARCH_OPTION_NONE,
                Some(&mut region),
            )?;
            let utf8_start = utf8_base + utf8_start;
            let start = transcoded.raw_offset(utf8_start)?;
            if let Some(invalid) = transcoded.first_invalid_at_or_after(search_from)
                && invalid < start
            {
                search_from = invalid + 1;
                continue;
            }
            let mut longest_region = Region::new();
            let relative_start = utf8_start - utf8_base;
            self.longest.search_with_options(
                text,
                relative_start,
                text.len(),
                SearchOptions::SEARCH_OPTION_NONE,
                Some(&mut longest_region),
            )?;
            let (_, relative_end) = longest_region.pos(0)?;
            let end = transcoded.raw_offset(utf8_base + relative_end)?;
            if !transcoded.contains_invalid(start, end) {
                return Some((start, end));
            }
            search_from = start + 1;
        }
        None
    }

    fn find_iter<'r, 't>(&'r self, bytes: &'t [u8]) -> ByteRegexFindIter<'r, 't> {
        let transcoded = self
            .transcode_locale
            .then(|| {
                self.locale_validator
                    .as_ref()
                    .map(|validator| LocaleUtf8Text::from_bytes(bytes, validator))
            })
            .flatten();
        ByteRegexFindIter {
            regex: self,
            bytes,
            next_start: 0,
            previous_end: None,
            transcoded,
        }
    }
}

struct ByteRegexFindIter<'r, 't> {
    regex: &'r ByteRegex,
    bytes: &'t [u8],
    next_start: usize,
    previous_end: Option<usize>,
    transcoded: Option<LocaleUtf8Text>,
}

impl Iterator for ByteRegexFindIter<'_, '_> {
    type Item = (usize, usize);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.next_start > self.bytes.len() {
                return None;
            }
            let base = self.next_start;
            let (start, end) = if let Some(transcoded) = &self.transcoded {
                self.regex
                    .find_at_transcoded_text(self.bytes, base, base, transcoded)?
            } else {
                let (start, end) = self.regex.find(&self.bytes[base..])?;
                (base + start, base + end)
            };
            if start == end && self.previous_end == Some(end) {
                self.next_start = end + 1;
                continue;
            }
            self.previous_end = Some(end);
            self.next_start = end;
            return Some((start, end));
        }
    }
}

#[derive(Debug)]
enum OutFormat {
    Dumb,
    Roff,
    Tex,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtxSemanticRow {
    pub row_index: usize,
    pub keyword: String,
    pub before: String,
    pub after: String,
    pub head: String,
    pub tail: String,
    pub reference: String,
    pub file: String,
    pub line_index: usize,
    pub global_line_index: usize,
    pub rendered_text: String,
    pub format: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtxSemantic {
    pub rows: Vec<PtxSemanticRow>,
    pub classic_text: String,
    pub stderr_text: String,
    pub exit_code: i32,
}

struct DirectPtxInvocation {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exit_code: i32,
}

#[derive(Debug)]
struct PtxConfig {
    /// 是否启用 GNU 扩展
    is_gnu_ext: bool,
    /// 是否忽略大小写
    is_ignore_case: bool,
    /// 是否自动生成引用
    is_auto_ref: bool,
    /// 是否使用输入引用
    is_input_ref: bool,
    /// 引用是否在右侧显示
    is_right_ref: bool,
    /// 输出格式(Roff 或 TeX)
    format: OutFormat,
    /// 行宽度
    line_width: usize,
    /// 间隔大小
    gap_size: usize,
    /// 截断标记字符串
    trunc_str: String,
    /// 截断标记的原始字节，Linux命令输出及字段宽度按GNU字节语义处理。
    trunc_bytes: Vec<u8>,
    /// 宏名称
    macro_name: String,
    /// Linux输出使用的宏名称原始字节。
    macro_bytes: Vec<u8>,
    /// 上下文正则表达式
    context_regex: String,
    /// 用户指定的上下文正则原始字节，用于GNU兼容诊断。
    context_pattern_bytes: Option<Vec<u8>>,
    /// 非UTF-8上下文正则的原始字节模式及编译结果。
    context_byte_pattern: Option<Vec<u8>>,
    context_byte_regex: Option<ByteRegex>,
    /// break-file定义的分词边界，输出布局阶段必须复用同一规则。
    word_break_bytes: Option<HashSet<u8>>,
    /// 用户指定的word regexp，字段规划阶段按GNU re_match语义复用。
    word_regex: Option<Regex>,
    /// 非UTF-8单词正则按原始字节执行。
    word_byte_regex: Option<ByteRegex>,
    /// 保证内部文本偏移与原始字节一一对应。
    force_byte_mode: bool,
    /// C/POSIX locale中的GNU正则按单字节执行。
    single_byte_locale: bool,
    /// GNU ptx按当前LC_CTYPE对每个原始字节执行isalpha和toupper。
    byte_ctype: LocaleByteCtype,
    /// 非UTF-8多字节locale使用的Oniguruma编码。
    locale_regex_encoding: LocaleRegexEncoding,
    /// Linux locale转换器用于验证Oniguruma候选的实际字符边界。
    locale_validator: Option<Arc<LocaleMultibyteValidator>>,
    /// glibc locale排序规则，用于展开单字节正则范围。
    locale_collation: Option<Arc<LocaleCollation>>,
    /// GNU libc当前locale下的正则word和non-word字符类。
    regex_word_class: Vec<u8>,
    regex_non_word_class: Vec<u8>,
    regex_space_class: Vec<u8>,
    regex_non_space_class: Vec<u8>,
}

impl Default for PtxConfig {
    fn default() -> Self {
        Self {
            format: OutFormat::Dumb,
            is_gnu_ext: true,
            is_auto_ref: false,
            is_input_ref: false,
            is_right_ref: false,
            is_ignore_case: false,
            macro_name: "xx".to_owned(),
            macro_bytes: b"xx".to_vec(),
            trunc_str: "/".to_owned(),
            trunc_bytes: b"/".to_vec(),
            context_regex: GNU_DEFAULT_CONTEXT_REGEX.to_owned(),
            context_pattern_bytes: None,
            context_byte_pattern: None,
            context_byte_regex: None,
            word_break_bytes: None,
            word_regex: None,
            word_byte_regex: None,
            force_byte_mode: false,
            single_byte_locale: false,
            byte_ctype: LocaleByteCtype::from_environment(),
            locale_regex_encoding: LocaleRegexEncoding::default(),
            locale_validator: None,
            locale_collation: None,
            regex_word_class: b"[[:alnum:]_]".to_vec(),
            regex_non_word_class: b"[^[:alnum:]_]".to_vec(),
            regex_space_class: b"[[:space:]]".to_vec(),
            regex_non_space_class: b"[^[:space:]]".to_vec(),
            line_width: 72,
            gap_size: 3,
        }
    }
}

fn read_word_filter_file(matches: &clap::ArgMatches, option: &str) -> CTResult<HashSet<Vec<u8>>> {
    let filename = matches
        .get_one::<OsString>(option)
        .expect("parsing options failed!");
    let mut file =
        File::open(filename).map_err(|error| ptx_file_io_error(filename.as_os_str(), error))?;
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)
        .map_err(|error| ptx_file_io_error(filename.as_os_str(), error))?;
    let mut words: HashSet<Vec<u8>> = HashSet::new();
    for line in contents.split(|&byte| byte == b'\n') {
        if !line.is_empty() {
            words.insert(line.to_vec());
        }
    }
    Ok(words)
}

/// reads contents of file as unique set of characters to be used with the break-file option
fn read_char_filter_file(matches: &clap::ArgMatches, option: &str) -> CTResult<HashSet<u8>> {
    let filename = matches
        .get_one::<OsString>(option)
        .expect("parsing options failed!");
    let mut reader =
        File::open(filename).map_err(|error| ptx_file_io_error(filename.as_os_str(), error))?;
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|error| ptx_file_io_error(filename.as_os_str(), error))?;
    Ok(bytes.into_iter().collect())
}

fn gnu_emacs_regex_to_rust(pattern: &str, config: &PtxConfig) -> String {
    gnu_emacs_regex_to_rust_with_candidates(pattern, config, None)
}

fn gnu_emacs_regex_to_rust_with_candidates(
    pattern: &str,
    config: &PtxConfig,
    locale_candidates: Option<&[Vec<u8>]>,
) -> String {
    let (word_class, non_word_class) = if std::str::from_utf8(&config.regex_word_class).is_ok() {
        (
            config.regex_word_class.as_slice(),
            config.regex_non_word_class.as_slice(),
        )
    } else {
        (b"[[:alnum:]_]".as_slice(), b"[^[:alnum:]_]".as_slice())
    };
    let (space_class, non_space_class) = if std::str::from_utf8(&config.regex_space_class).is_ok() {
        (
            config.regex_space_class.as_slice(),
            config.regex_non_space_class.as_slice(),
        )
    } else {
        (b"[[:space:]]".as_slice(), b"[^[:space:]]".as_slice())
    };
    let context = RegexTranslationContext {
        word_class,
        non_word_class,
        space_class,
        non_space_class,
        locale_collation: config.locale_collation.as_deref(),
        expand_locale_ranges: false,
        defer_locale_ranges: locale_candidates.is_none()
            && !config.single_byte_locale
            && config.locale_regex_encoding == LocaleRegexEncoding::Ascii,
        locale_candidates,
        range_fold_upper: config.is_ignore_case.then_some(&config.byte_ctype.upper),
    };
    let translated =
        gnu_emacs_regex_to_onig_bytes_with_classes_and_collation(pattern.as_bytes(), &context);
    String::from_utf8(translated).expect("UTF-8 pattern translation must remain UTF-8")
}

fn gnu_emacs_regex_to_onig_bytes(pattern: &[u8], config: &PtxConfig) -> Vec<u8> {
    gnu_emacs_regex_to_onig_bytes_with_candidates(pattern, config, None)
}

fn gnu_emacs_regex_to_onig_bytes_with_candidates(
    pattern: &[u8],
    config: &PtxConfig,
    locale_candidates: Option<&[Vec<u8>]>,
) -> Vec<u8> {
    let context = RegexTranslationContext {
        word_class: &config.regex_word_class,
        non_word_class: &config.regex_non_word_class,
        space_class: &config.regex_space_class,
        non_space_class: &config.regex_non_space_class,
        locale_collation: config.locale_collation.as_deref(),
        expand_locale_ranges: config.single_byte_locale,
        defer_locale_ranges: locale_candidates.is_none()
            && !config.single_byte_locale
            && config.locale_regex_encoding == LocaleRegexEncoding::Ascii,
        locale_candidates,
        range_fold_upper: config.is_ignore_case.then_some(&config.byte_ctype.upper),
    };
    gnu_emacs_regex_to_onig_bytes_with_classes_and_collation(pattern, &context)
}

fn ptx_bracket_symbol_end(pattern: &[u8], start: usize) -> Option<(u8, usize)> {
    if pattern.get(start) != Some(&b'[') {
        return None;
    }
    let &delimiter @ (b'.' | b'=') = pattern.get(start + 1)? else {
        return None;
    };
    pattern[start + 2..]
        .windows(2)
        .position(|closing| closing == [delimiter, b']'])
        .map(|relative| (delimiter, start + relative + 4))
}

fn ptx_character_class_end(pattern: &[u8], start: usize) -> Option<usize> {
    let mut index = start + 1;
    if pattern.get(index) == Some(&b'^') {
        index += 1;
    }
    if pattern.get(index) == Some(&b']') {
        index += 1;
    }
    while index < pattern.len() {
        if let Some((_, end)) = ptx_bracket_symbol_end(pattern, index) {
            index = end;
            continue;
        }
        if pattern[index] == b']' {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn ptx_character_class_has_range(class: &[u8]) -> bool {
    if class.len() < 4 {
        return false;
    }
    let content = &class[1..class.len() - 1];
    let mut index = usize::from(content.first() == Some(&b'^'));
    if content.get(index) == Some(&b']') {
        index += 1;
    }
    while index < content.len() {
        let symbol = ptx_bracket_symbol_end(content, index);
        let start_end = symbol.map_or(index + 1, |(_, end)| end);
        let valid_start = symbol.is_some_and(|(delimiter, _)| delimiter == b'.')
            || !matches!(content[index], b'[' | b']' | b'\\' | b'-' | b'^');
        let end_start = start_end + 1;
        let valid_end = ptx_bracket_symbol_end(content, end_start)
            .is_some_and(|(delimiter, _)| delimiter == b'.')
            || content
                .get(end_start)
                .is_some_and(|end| !matches!(end, b'[' | b']' | b'\\' | b'^'));
        if valid_start && content.get(start_end) == Some(&b'-') && valid_end {
            return true;
        }
        index = start_end;
    }
    false
}

fn ptx_character_class_has_bracket_symbol(class: &[u8]) -> bool {
    if class.len() < 4 {
        return false;
    }
    let content = &class[1..class.len() - 1];
    let mut index = usize::from(content.first() == Some(&b'^'));
    if content.get(index) == Some(&b']') {
        index += 1;
    }
    while index < content.len() {
        if ptx_bracket_symbol_end(content, index).is_some() {
            return true;
        }
        index += 1;
    }
    false
}

fn ptx_character_class_needs_locale_matching(class: &[u8]) -> bool {
    ptx_character_class_has_range(class) || ptx_character_class_has_bracket_symbol(class)
}

fn ptx_pattern_needs_locale_class_matching(pattern: &[u8]) -> bool {
    let mut index = 0usize;
    let mut escaped = false;
    while index < pattern.len() {
        let byte = pattern[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if byte == b'\\' {
            escaped = true;
            index += 1;
            continue;
        }
        if byte == b'['
            && let Some(end) = ptx_character_class_end(pattern, index)
        {
            if ptx_character_class_needs_locale_matching(&pattern[index..=end]) {
                return true;
            }
            index = end + 1;
            continue;
        }
        index += 1;
    }
    false
}

fn ptx_character_class_has_equivalence_range_endpoint(class: &[u8]) -> bool {
    if class.len() < 4 {
        return false;
    }
    let content = &class[1..class.len() - 1];
    let mut index = usize::from(content.first() == Some(&b'^'));
    if content.get(index) == Some(&b']') {
        index += 1;
    }
    while index < content.len() {
        let start_symbol = ptx_bracket_symbol_end(content, index);
        let start_end = start_symbol.map_or(index + 1, |(_, end)| end);
        let end_start = start_end + 1;
        if content.get(start_end) == Some(&b'-')
            && end_start < content.len()
            && (start_symbol.is_some_and(|(delimiter, _)| delimiter == b'=')
                || ptx_bracket_symbol_end(content, end_start)
                    .is_some_and(|(delimiter, _)| delimiter == b'='))
        {
            return true;
        }
        index = start_end;
    }
    false
}

fn ptx_pattern_has_equivalence_range_endpoint(pattern: &[u8]) -> bool {
    let mut index = 0usize;
    let mut escaped = false;
    while index < pattern.len() {
        let byte = pattern[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if byte == b'\\' {
            escaped = true;
            index += 1;
            continue;
        }
        if byte == b'['
            && let Some(end) = ptx_character_class_end(pattern, index)
        {
            if ptx_character_class_has_equivalence_range_endpoint(&pattern[index..=end]) {
                return true;
            }
            index = end + 1;
            continue;
        }
        index += 1;
    }
    false
}

fn ptx_push_onig_candidate_alternation(output: &mut Vec<u8>, candidates: &[Vec<u8>]) {
    if candidates.is_empty() {
        output.extend_from_slice(b"(?:(?!)\\x00)");
        return;
    }
    output.extend_from_slice(b"(?:");
    for (index, candidate) in candidates.iter().enumerate() {
        if index > 0 {
            output.push(b'|');
        }
        if candidate.len() == 1 {
            write!(output, r"\x{:02X}", candidate[0]).expect("writing to a Vec cannot fail");
        } else {
            output.extend_from_slice(candidate);
        }
    }
    output.push(b')');
}

struct RegexTranslationContext<'a> {
    word_class: &'a [u8],
    non_word_class: &'a [u8],
    space_class: &'a [u8],
    non_space_class: &'a [u8],
    locale_collation: Option<&'a LocaleCollation>,
    expand_locale_ranges: bool,
    defer_locale_ranges: bool,
    locale_candidates: Option<&'a [Vec<u8>]>,
    range_fold_upper: Option<&'a [u8; 256]>,
}

fn gnu_emacs_regex_to_onig_bytes_with_classes_and_collation(
    pattern: &[u8],
    context: &RegexTranslationContext<'_>,
) -> Vec<u8> {
    let mut translated = Vec::with_capacity(pattern.len());
    let mut escaped = false;
    let mut in_bracket = false;
    let mut skip_before = 0usize;
    let mut bracket_output_start = 0usize;
    let mut bracket_at_start = false;
    let mut bracket_negated = false;
    let mut bracket_has_member = false;
    let mut bracket_removed_empty_range = false;

    for (index, &byte) in pattern.iter().enumerate() {
        if index < skip_before {
            continue;
        }
        if !in_bracket
            && byte == b'['
            && let Some(end) = ptx_character_class_end(pattern, index)
            && ptx_character_class_needs_locale_matching(&pattern[index..=end])
            && let Some(locale_collation) = context.locale_collation
        {
            let class = &pattern[index..=end];
            if let Some(candidates) = context.locale_candidates {
                if let Some(matching) = locale_collation.matching_candidates(
                    class,
                    candidates,
                    context.range_fold_upper,
                ) {
                    ptx_push_onig_candidate_alternation(&mut translated, &matching);
                    skip_before = end + 1;
                    continue;
                }
            }
            if context.defer_locale_ranges {
                translated.extend_from_slice(b"(?:(?!)\\x00)");
                skip_before = end + 1;
                continue;
            }
        }
        if in_bracket && !escaped && byte == b'\\' {
            translated.extend_from_slice(b"\\\\");
            bracket_at_start = false;
            bracket_has_member = true;
            continue;
        }
        if escaped {
            if !in_bracket && matches!(byte, b'(' | b')' | b'|') {
                translated.pop();
                translated.push(byte);
            } else if !in_bracket && matches!(byte, b'<' | b'>' | b'`' | b'\'') {
                translated.pop();
                match byte {
                    b'<' => {
                        translated.extend_from_slice(b"(?<!");
                        translated.extend_from_slice(context.word_class);
                        translated.extend_from_slice(b")(?=");
                        translated.extend_from_slice(context.word_class);
                        translated.push(b')');
                    }
                    b'>' => {
                        translated.extend_from_slice(b"(?<=");
                        translated.extend_from_slice(context.word_class);
                        translated.extend_from_slice(b")(?!");
                        translated.extend_from_slice(context.word_class);
                        translated.push(b')');
                    }
                    b'`' => translated.extend_from_slice(b"\\A"),
                    b'\'' => translated.extend_from_slice(b"\\z"),
                    _ => unreachable!(),
                }
            } else if !in_bracket && matches!(byte, b'w' | b'W') {
                translated.pop();
                translated.extend_from_slice(if byte == b'w' {
                    context.word_class
                } else {
                    context.non_word_class
                });
            } else if !in_bracket && byte == b'B' {
                translated.pop();
                translated.extend_from_slice(b"(?:(?<=");
                translated.extend_from_slice(context.word_class);
                translated.extend_from_slice(b")(?=");
                translated.extend_from_slice(context.word_class);
                translated.extend_from_slice(b")|(?<!");
                translated.extend_from_slice(context.word_class);
                translated.extend_from_slice(b")(?!");
                translated.extend_from_slice(context.word_class);
                translated.extend_from_slice(b"))");
            } else if !in_bracket && matches!(byte, b's' | b'S') {
                translated.pop();
                translated.extend_from_slice(if byte == b's' {
                    context.space_class
                } else {
                    context.non_space_class
                });
            } else if !in_bracket && byte.is_ascii_alphabetic() {
                // GNU's Emacs syntax treats other alphabetic escapes literally.
                translated.pop();
                translated.push(byte);
            } else {
                translated.push(byte);
            }
            escaped = false;
            continue;
        }

        if in_bracket && bracket_at_start && byte == b'^' {
            translated.push(byte);
            bracket_at_start = false;
            bracket_negated = true;
            continue;
        }

        if in_bracket
            && !matches!(byte, b'[' | b']' | b'\\' | b'-' | b'^')
            && pattern.get(index + 1) == Some(&b'-')
            && pattern
                .get(index + 2)
                .is_some_and(|end| !matches!(end, b'[' | b']' | b'\\' | b'^'))
            && let Some(locale_collation) = context.locale_collation
        {
            if let Some(members) = locale_collation.range_members(byte, pattern[index + 2]) {
                if context.expand_locale_ranges {
                    for (value, member) in members.into_iter().enumerate() {
                        if member {
                            write!(translated, r"\x{value:02X}")
                                .expect("writing to a Vec cannot fail");
                        }
                    }
                } else {
                    translated.extend_from_slice(&pattern[index..index + 3]);
                }
                bracket_has_member = true;
            } else {
                bracket_removed_empty_range = true;
            }
            bracket_at_start = false;
            skip_before = index + 3;
            continue;
        }

        // Emacs syntax treats a nested '[' literally; Oniguruma otherwise
        // recognizes constructs such as [:alpha:] anywhere in the class.
        if in_bracket && byte == b'[' && pattern.get(index + 1) == Some(&b':') {
            translated.extend_from_slice(b"\\[");
            bracket_at_start = false;
            bracket_has_member = true;
        } else {
            if !in_bracket && matches!(byte, b'(' | b')' | b'|' | b'{' | b'}') {
                translated.push(b'\\');
            }
            translated.push(byte);
            if in_bracket && byte != b']' {
                bracket_at_start = false;
                bracket_has_member = true;
            }
        }

        if byte == b'\\' {
            escaped = true;
        } else if byte == b'[' && !in_bracket {
            in_bracket = true;
            bracket_output_start = translated.len() - 1;
            bracket_at_start = true;
            bracket_negated = false;
            bracket_has_member = false;
            bracket_removed_empty_range = false;
        } else if byte == b']' && in_bracket {
            in_bracket = false;
            if bracket_removed_empty_range && !bracket_has_member {
                translated.truncate(bracket_output_start);
                translated.extend_from_slice(if bracket_negated { b"(?m:.)" } else { b"(?!)" });
            }
        }
    }

    translated
}

fn ptx_has_chained_character_class_range(pattern: &[u8]) -> bool {
    let mut index = 0usize;
    let mut in_bracket = false;
    let mut escaped = false;
    let mut range_just_ended = false;
    while index < pattern.len() {
        let byte = pattern[index];
        if escaped {
            escaped = false;
            range_just_ended = false;
            index += 1;
            continue;
        }
        if byte == b'\\' {
            escaped = true;
            range_just_ended = false;
            index += 1;
            continue;
        }
        if !in_bracket {
            if byte == b'[' {
                in_bracket = true;
            }
            index += 1;
            continue;
        }
        if byte == b']' {
            in_bracket = false;
            range_just_ended = false;
            index += 1;
            continue;
        }
        if range_just_ended && byte == b'-' && pattern.get(index + 1) != Some(&b']') {
            return true;
        }
        if !matches!(byte, b'[' | b']' | b'-' | b'^')
            && pattern.get(index + 1) == Some(&b'-')
            && pattern
                .get(index + 2)
                .is_some_and(|end| !matches!(end, b'[' | b']' | b'\\' | b'^'))
        {
            range_just_ended = true;
            index += 3;
            continue;
        }
        range_just_ended = false;
        index += 1;
    }
    false
}

#[derive(Debug)]
struct WordFilter {
    /// 是否只包含指定的单词
    is_only_specified: bool,
    /// 是否忽略指定的单词
    is_ignore_specified: bool,
    /// 要包含的单词集合
    only_set: HashSet<Vec<u8>>,
    /// 要忽略的单词集合
    ignore_set: HashSet<Vec<u8>>,
    /// 用于匹配单词的正则表达式
    word_regex: String,
    /// 非UTF-8自定义单词正则的原始字节模式。
    word_byte_pattern: Option<Vec<u8>>,
    /// 用户指定并完成GNU反转义的原始单词正则。
    word_pattern_bytes: Option<Vec<u8>>,
    /// break-file中的边界字符。
    break_set: Option<HashSet<u8>>,
    /// 是否使用用户指定的word regexp。
    uses_custom_regex: bool,
}

impl WordFilter {
    #[allow(clippy::cognitive_complexity)]
    fn new(matches: &clap::ArgMatches, config: &PtxConfig) -> CTResult<Self> {
        // Ignore empty string regex from cmd-line-args
        let arg_reg_bytes: Option<Vec<u8>> = if matches.contains_id(ptx_options::PTX_WORD_REGEXP) {
            match matches.get_one::<OsString>(ptx_options::PTX_WORD_REGEXP) {
                Some(v) => {
                    let value = ptx_unescape_bytes(v.as_os_str().as_bytes());
                    if value.is_empty() { None } else { Some(value) }
                }
                None => None,
            }
        } else {
            None
        };
        let loaded_break_set: Option<HashSet<u8>> =
            if matches.contains_id(ptx_options::PTX_BREAK_FILE) {
                let bytes = read_char_filter_file(matches, ptx_options::PTX_BREAK_FILE)?;
                let mut hs: HashSet<u8> = if config.is_gnu_ext {
                    HashSet::new() // really only chars found in file
                } else {
                    // GNU off means at least these are considered
                    [b' ', b'\t', b'\n'].iter().cloned().collect()
                };
                hs.extend(bytes);
                Some(hs)
            } else {
                None
            };
        let break_set = arg_reg_bytes
            .is_none()
            .then_some(loaded_break_set)
            .flatten();
        let (i, iset): (bool, HashSet<Vec<u8>>) =
            if matches.contains_id(ptx_options::PTX_IGNORE_FILE) {
                let mut words = read_word_filter_file(matches, ptx_options::PTX_IGNORE_FILE)?;
                if config.is_ignore_case {
                    words = words
                        .into_iter()
                        .map(|mut word| {
                            config.byte_ctype.uppercase(&mut word);
                            word
                        })
                        .collect();
                }
                (!words.is_empty(), words)
            } else {
                (false, HashSet::new())
            };
        let (o, oset): (bool, HashSet<Vec<u8>>) = if matches.contains_id(ptx_options::PTX_ONLY_FILE)
        {
            let mut words = read_word_filter_file(matches, ptx_options::PTX_ONLY_FILE)?;
            if config.is_ignore_case {
                words = words
                    .into_iter()
                    .map(|mut word| {
                        config.byte_ctype.uppercase(&mut word);
                        word
                    })
                    .collect();
            }
            (!words.is_empty(), words)
        } else {
            (false, HashSet::new())
        };
        let uses_custom_regex = arg_reg_bytes.is_some();
        let word_byte_pattern = arg_reg_bytes
            .as_ref()
            .filter(|pattern| {
                config.is_ignore_case
                    || config.single_byte_locale
                    || config.locale_regex_encoding.is_non_utf8_multibyte()
                    || std::str::from_utf8(pattern).is_err()
            })
            .map(|pattern| gnu_emacs_regex_to_onig_bytes(pattern, config));
        let arg_reg = arg_reg_bytes.as_ref().map(|bytes| {
            let byte_mode = std::str::from_utf8(bytes).is_err();
            ptx_internal_text(bytes, byte_mode)
        });
        let reg = match arg_reg {
            Some(arg_reg) => gnu_emacs_regex_to_rust(&arg_reg, config),
            None => {
                if let Some(break_set) = &break_set {
                    format!(
                        "[^{}]+",
                        break_set
                            .iter()
                            .copied()
                            .map(char::from)
                            .map(|c| if REGEX_CHARCLASS.contains(c) {
                                format!("\\{c}")
                            } else {
                                c.to_string()
                            })
                            .collect::<String>()
                    )
                } else if config.is_gnu_ext {
                    "[A-Za-z]+".to_owned()
                } else {
                    "[^ \t\n]+".to_owned()
                }
            }
        };
        Ok(Self {
            is_only_specified: o,
            is_ignore_specified: i,
            only_set: oset,
            ignore_set: iset,
            word_regex: reg,
            word_byte_pattern,
            word_pattern_bytes: arg_reg_bytes,
            break_set,
            uses_custom_regex,
        })
    }
}

impl Default for WordFilter {
    fn default() -> Self {
        Self {
            is_only_specified: false,
            is_ignore_specified: false,
            only_set: HashSet::new(),
            ignore_set: HashSet::new(),
            word_regex: "[A-Za-z]+".to_string(),
            word_byte_pattern: None,
            word_pattern_bytes: None,
            break_set: None,
            uses_custom_regex: false,
        }
    }
}

/// 单词引用
///
/// 记录单词在文本中的位置和上下文信息
#[derive(Debug, PartialOrd, PartialEq, Eq, Ord, Default)]
struct WordRef {
    /// GNU ptx按原始无符号字节排序，不能使用非UTF-8内部映射代替。
    raw_word: Vec<u8>,
    /// 单词本身
    word: String,
    /// 在所有文件中的行号
    global_line_nr: usize,
    /// 在当前文件中的行号
    local_line_nr: usize,
    /// 单词在行中的起始位置
    position: usize,
    /// 单词在行中的结束位置
    position_end: usize,
    /// 单词在完整文件文本中的起始位置
    global_position: usize,
    /// 单词在完整文件文本中的结束位置
    global_position_end: usize,
    /// 当前上下文在完整文件文本中的起始位置
    context_start: usize,
    /// 当前上下文在完整文件文本中的结束位置
    context_end: usize,
    /// 单词在完整文件字符数组中的起始位置
    global_char_position: usize,
    /// 单词在完整文件字符数组中的结束位置
    global_char_position_end: usize,
    /// 当前上下文在完整文件字符数组中的起始位置
    context_char_start: usize,
    /// 当前上下文在完整文件字符数组中的结束位置
    context_char_end: usize,
    file_index: usize,
    /// GNU输入引用扫描器为该关键字保存的原始引用字节。
    input_reference: Vec<u8>,
    /// 自动引用使用的GNU逻辑行号；与-r组合时可能不同于物理行号。
    reference_line_nr: usize,
}

fn parse_positive_base0(value: &str, description: &str) -> CTResult<usize> {
    let invalid = || ptx_invalid_numeric_arg_error(value.as_bytes(), description);
    let value_without_leading_space =
        value.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let unsigned = value_without_leading_space
        .strip_prefix('+')
        .unwrap_or(value_without_leading_space);
    if unsigned.is_empty() || unsigned.starts_with('-') {
        return Err(invalid());
    }

    let (digits, radix) = if let Some(digits) = unsigned
        .strip_prefix("0x")
        .or_else(|| unsigned.strip_prefix("0X"))
    {
        (digits, 16)
    } else if unsigned.len() > 1 && unsigned.starts_with('0') {
        (&unsigned[1..], 8)
    } else {
        (unsigned, 10)
    };
    let parsed = u128::from_str_radix(digits, radix).map_err(|_| invalid())?;
    if parsed == 0 || parsed > isize::MAX as u128 {
        return Err(invalid());
    }
    Ok(parsed as usize)
}

fn ptx_unescape_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != b'\\' {
            output.push(bytes[index]);
            index += 1;
            continue;
        }

        index += 1;
        let Some(&escaped) = bytes.get(index) else {
            break;
        };
        match escaped {
            b'x' => {
                index += 1;
                let start = index;
                let mut number = 0u32;
                while index < bytes.len() && index - start < 3 {
                    let Some(digit) = char::from(bytes[index]).to_digit(16) else {
                        break;
                    };
                    number = number * 16 + digit;
                    index += 1;
                }
                if index == start {
                    output.extend_from_slice(b"\\x");
                } else {
                    output.push(number as u8);
                }
            }
            b'0' => {
                index += 1;
                let start = index;
                let mut number = 0u32;
                while index < bytes.len() && index - start < 3 {
                    let Some(digit) = char::from(bytes[index]).to_digit(8) else {
                        break;
                    };
                    number = number * 8 + digit;
                    index += 1;
                }
                output.push(number as u8);
            }
            b'a' => {
                output.push(b'\x07');
                index += 1;
            }
            b'b' => {
                output.push(b'\x08');
                index += 1;
            }
            b'c' => break,
            b'f' => {
                output.push(b'\x0c');
                index += 1;
            }
            b'n' => {
                output.push(b'\n');
                index += 1;
            }
            b'r' => {
                output.push(b'\r');
                index += 1;
            }
            b't' => {
                output.push(b'\t');
                index += 1;
            }
            b'v' => {
                output.push(b'\x0b');
                index += 1;
            }
            _ => {
                output.push(b'\\');
                output.push(escaped);
                index += 1;
            }
        }
    }
    if let Some(nul) = output.iter().position(|&byte| byte == 0) {
        output.truncate(nul);
    }
    output
}

fn get_config(matches: &clap::ArgMatches) -> CTResult<PtxConfig> {
    let mut config = PtxConfig {
        is_ignore_case: matches.get_flag(ptx_options::PTX_IGNORE_CASE),
        single_byte_locale: ptx_is_single_byte_locale(),
        locale_regex_encoding: LocaleRegexEncoding::from_environment(),
        locale_validator: LocaleMultibyteValidator::from_environment().map(Arc::new),
        locale_collation: LocaleCollation::from_environment().map(Arc::new),
        ..Default::default()
    };
    (config.regex_word_class, config.regex_non_word_class) = ptx_locale_word_classes(
        config.single_byte_locale,
        config.locale_regex_encoding,
        &config.byte_ctype,
        config.locale_validator.as_deref(),
    );
    (config.regex_space_class, config.regex_non_space_class) = ptx_locale_space_classes(
        config.single_byte_locale,
        config.locale_regex_encoding,
        &config.byte_ctype,
        config.locale_validator.as_deref(),
    );
    let err_msg = "parsing options failed";
    if matches.get_flag(ptx_options::PTX_TRADITIONAL) {
        config.is_gnu_ext = false;
        config.format = OutFormat::Roff;
        "\n".clone_into(&mut config.context_regex);
    }
    if let Some(reg) = matches.get_one::<OsString>(ptx_options::PTX_SENTENCE_REGEXP) {
        let bytes = ptx_unescape_bytes(reg.as_os_str().as_bytes());
        config.context_pattern_bytes = Some(bytes.clone());
        let byte_mode = std::str::from_utf8(&bytes).is_err();
        if config.is_ignore_case
            || byte_mode
            || config.single_byte_locale
            || config.locale_regex_encoding.is_non_utf8_multibyte()
        {
            config.context_byte_pattern = Some(gnu_emacs_regex_to_onig_bytes(&bytes, &config));
            config.force_byte_mode = true;
        }
        let internal = ptx_internal_text(&bytes, byte_mode);
        config.context_regex = if internal.is_empty() {
            NEVER_MATCH_REGEX.to_string()
        } else {
            gnu_emacs_regex_to_rust(&internal, &config)
        };
        // Note: Zero-length regex check is deferred to actual usage time
        // to match GNU ptx behavior (only errors when processing non-empty content)
    }
    config.is_auto_ref = matches.get_flag(ptx_options::PTX_AUTO_REFERENCE);
    config.is_input_ref = matches.get_flag(ptx_options::PTX_REFERENCES);
    if config.is_input_ref && !matches.contains_id(ptx_options::PTX_SENTENCE_REGEXP) {
        config.context_regex = "\n".to_string();
    }
    if config.is_gnu_ext
        && !config.is_input_ref
        && !matches.contains_id(ptx_options::PTX_SENTENCE_REGEXP)
    {
        config.context_byte_regex = Some(
            compile_byte_regex(
                GNU_DEFAULT_CONTEXT_REGEX.as_bytes(),
                config.is_ignore_case,
                &config.byte_ctype,
                config.locale_regex_encoding,
                config.locale_validator.clone(),
                config.single_byte_locale,
            )
            .expect("GNU default context regexp must compile"),
        );
    }
    config.is_right_ref = matches.get_flag(ptx_options::PTX_RIGHT_SIDE_REFS);
    if matches.contains_id(ptx_options::PTX_MACRO_NAME) {
        let value = matches
            .get_one::<OsString>(ptx_options::PTX_MACRO_NAME)
            .expect(err_msg);
        config.macro_bytes = value.as_os_str().as_bytes().to_vec();
        let byte_mode = std::str::from_utf8(&config.macro_bytes).is_err();
        config.macro_name = ptx_internal_text(&config.macro_bytes, byte_mode);
    }
    if matches.contains_id(ptx_options::PTX_FLAG_TRUNCATION) {
        let value = matches
            .get_one::<OsString>(ptx_options::PTX_FLAG_TRUNCATION)
            .expect(err_msg);
        config.trunc_bytes = ptx_unescape_bytes(value.as_os_str().as_bytes());
        let byte_mode = std::str::from_utf8(&config.trunc_bytes).is_err();
        config.trunc_str = ptx_internal_text(&config.trunc_bytes, byte_mode);
    }
    if matches.contains_id(ptx_options::PTX_WIDTH) {
        let value = matches
            .get_one::<String>(ptx_options::PTX_WIDTH)
            .expect(err_msg);
        config.line_width = parse_positive_base0(value, "line width")?;
    }
    if matches.contains_id(ptx_options::PTX_GAP_SIZE) {
        let value = matches
            .get_one::<String>(ptx_options::PTX_GAP_SIZE)
            .expect(err_msg);
        config.gap_size = parse_positive_base0(value, "gap width")?;
    }
    let format_option = matches
        .get_one::<String>(ptx_options::PTX_FORMAT)
        .and_then(|format| {
            matches.index_of(ptx_options::PTX_FORMAT).map(|index| {
                let format = if format == "roff" {
                    OutFormat::Roff
                } else {
                    OutFormat::Tex
                };
                (index, format)
            })
        });
    let roff_option = matches
        .get_flag(ptx_options::PTX_FORMAT_ROFF)
        .then(|| matches.index_of(ptx_options::PTX_FORMAT_ROFF))
        .flatten()
        .map(|index| (index, OutFormat::Roff));
    let tex_option = matches
        .get_flag(ptx_options::PTX_FORMAT_TEX)
        .then(|| matches.index_of(ptx_options::PTX_FORMAT_TEX))
        .flatten()
        .map(|index| (index, OutFormat::Tex));
    if let Some((_, format)) = [format_option, roff_option, tex_option]
        .into_iter()
        .flatten()
        .max_by_key(|(index, _)| *index)
    {
        config.format = format;
    }
    Ok(config)
}

fn compile_regex_case_lossy(pattern: &str, ignore_case: bool) -> Regex {
    let build = |pattern: &str| compile_regex(pattern, ignore_case);
    if let Ok(re) = build(pattern) {
        return re;
    }

    if pattern.ends_with('\\') {
        let mut fixed = pattern.to_owned();
        fixed.push('\\');
        if let Ok(re) = build(&fixed) {
            return re;
        }
    }

    build(r"$^").expect("fallback regex must be valid")
}

fn compile_user_regex(pattern: &str, ignore_case: bool) -> CTResult<Regex> {
    compile_regex(pattern, ignore_case).map_err(|_| ptx_invalid_regex_error(pattern.as_bytes()))
}

fn compile_regex(pattern: &str, ignore_case: bool) -> Result<Regex, onig::Error> {
    let mut options = RegexOptions::REGEX_OPTION_NONE;
    if ignore_case {
        options |= RegexOptions::REGEX_OPTION_IGNORECASE;
    }
    let search = OnigRegex::with_options(pattern, options, Syntax::default())?;
    let longest_pattern = format!(r"\G(?:{pattern})");
    let longest = OnigRegex::with_options(
        &longest_pattern,
        options | RegexOptions::REGEX_OPTION_FIND_LONGEST,
        Syntax::default(),
    )?;
    Ok(Regex { search, longest })
}

fn compile_user_byte_regex(
    pattern: &[u8],
    ignore_case: bool,
    byte_ctype: &LocaleByteCtype,
    encoding: LocaleRegexEncoding,
    locale_validator: Option<Arc<LocaleMultibyteValidator>>,
    single_byte_locale: bool,
) -> CTResult<ByteRegex> {
    compile_byte_regex(
        pattern,
        ignore_case,
        byte_ctype,
        encoding,
        locale_validator,
        single_byte_locale,
    )
    .map_err(|_| ptx_invalid_regex_error(pattern))
}

fn compile_byte_regex(
    pattern: &[u8],
    ignore_case: bool,
    byte_ctype: &LocaleByteCtype,
    encoding: LocaleRegexEncoding,
    locale_validator: Option<Arc<LocaleMultibyteValidator>>,
    single_byte_locale: bool,
) -> Result<ByteRegex, onig::Error> {
    let transcode_locale = encoding.is_non_utf8_multibyte() && locale_validator.is_some();
    let mut options = RegexOptions::REGEX_OPTION_NONE;
    if ignore_case {
        options |= RegexOptions::REGEX_OPTION_IGNORECASE;
        if transcode_locale
            || matches!(
                encoding,
                LocaleRegexEncoding::Ascii | LocaleRegexEncoding::SingleByte
            )
        {
            // onig exposes this option in onig_sys but omits it from RegexOptions.
            options |= unsafe {
                RegexOptions::from_bits_unchecked(onig_sys::ONIG_OPTION_IGNORECASE_IS_ASCII)
            };
        }
    }
    let mut folded_pattern = pattern.to_vec();
    if ignore_case
        && matches!(
            encoding,
            LocaleRegexEncoding::Ascii | LocaleRegexEncoding::SingleByte
        )
    {
        for byte in &mut folded_pattern {
            if !byte.is_ascii() {
                *byte = byte_ctype.upper[usize::from(*byte)];
            }
        }
    }
    let (search, longest) = if transcode_locale {
        let validator = locale_validator
            .as_ref()
            .expect("locale transcoding requires a locale validator");
        let pattern = LocaleUtf8Text::from_bytes(&folded_pattern, validator).text;
        let search = OnigRegex::with_options(&pattern, options, Syntax::default())?;
        let longest_pattern = format!(r"\G(?:{pattern})");
        let longest = OnigRegex::with_options(
            &longest_pattern,
            options | RegexOptions::REGEX_OPTION_FIND_LONGEST,
            Syntax::default(),
        )?;
        (search, longest)
    } else {
        let search = OnigRegex::with_options_and_encoding(
            encoding.encoded(&folded_pattern),
            options,
            Syntax::default(),
        )?;
        let mut longest_pattern = b"\\G(?:".to_vec();
        longest_pattern.extend_from_slice(&folded_pattern);
        longest_pattern.push(b')');
        let longest = OnigRegex::with_options_and_encoding(
            encoding.encoded(&longest_pattern),
            options | RegexOptions::REGEX_OPTION_FIND_LONGEST,
            Syntax::default(),
        )?;
        (search, longest)
    };
    Ok(ByteRegex {
        search,
        longest,
        fold_upper: (ignore_case
            && matches!(
                encoding,
                LocaleRegexEncoding::Ascii | LocaleRegexEncoding::SingleByte
            ))
        .then_some(byte_ctype.upper),
        encoding,
        locale_validator,
        transcode_locale,
        single_byte_locale,
    })
}

/// 文件内容
///
/// 存储文件的行内容和字符级表示
#[derive(Debug)]
struct FileContent {
    /// 文件名 (从 Map 键移入内部)
    filename: String,
    /// Linux文件名原始字节。
    raw_filename: Vec<u8>,
    /// 文件完整文本，物理行之间保留 '\n'，用于 GNU 默认跨行上下文处理。
    text: String,
    /// 命令输出使用的原始字节。
    raw_text: Vec<u8>,
    /// 多字节locale下正则不能匹配这些非法UTF-8字节。
    invalid_utf8_bytes: Vec<bool>,
    chars_text: Vec<char>,
    byte_to_char: Vec<usize>,
    /// 每个物理行在完整文本中的起始字节偏移。
    line_starts: Vec<usize>,
    /// 文件的所有行
    lines: Vec<String>,
    /// 不包含换行符的原始行字节。
    raw_lines: Vec<Vec<u8>>,
    /// 每行的字符数组表示，用于快速索引
    chars_lines: Vec<Vec<char>>,
    /// 在所有文件中的行偏移量
    offset: usize,
}

type FileMap = Vec<FileContent>;

fn ptx_collect_locale_regex_candidates(file_map: &FileMap, config: &PtxConfig) -> Vec<Vec<u8>> {
    let mut candidates = BTreeSet::new();
    for file in file_map {
        ptx_add_locale_regex_candidates(&file.raw_text, config, &mut candidates);
    }
    candidates.into_iter().collect()
}

fn ptx_add_locale_regex_candidates(
    bytes: &[u8],
    config: &PtxConfig,
    candidates: &mut BTreeSet<Vec<u8>>,
) {
    if config.single_byte_locale {
        candidates.extend(
            bytes
                .iter()
                .copied()
                .filter(|&byte| byte != 0)
                .map(|byte| vec![byte]),
        );
        return;
    }
    let Some(validator) = config.locale_validator.as_deref() else {
        return;
    };
    let mut index = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == 0 {
            index += 1;
            continue;
        }
        let length = if byte.is_ascii() {
            Some(1)
        } else {
            validator.valid_character_len(&bytes[index..])
        };
        if let Some(length) = length {
            candidates.insert(bytes[index..index + length].to_vec());
            index += length;
        } else {
            index += 1;
        }
    }
}

fn ptx_locale_context_matches_at_boundary(
    config: &PtxConfig,
    content: &FileContent,
) -> CTResult<bool> {
    if config.locale_regex_encoding.is_non_utf8_multibyte() {
        return Ok(false);
    }
    let Some(pattern) = config.context_pattern_bytes.as_deref() else {
        return Ok(false);
    };
    if !ptx_pattern_needs_locale_class_matching(pattern) {
        return Ok(false);
    }
    let mut candidates = BTreeSet::new();
    ptx_add_locale_regex_candidates(&content.raw_text, config, &mut candidates);
    let candidates: Vec<Vec<u8>> = candidates.into_iter().collect();
    if candidates.is_empty() {
        return Ok(false);
    }
    let byte_mode = std::str::from_utf8(pattern).is_err();
    if config.is_ignore_case
        || byte_mode
        || content.invalid_utf8_bytes.iter().any(|&invalid| invalid)
    {
        let translated =
            gnu_emacs_regex_to_onig_bytes_with_candidates(pattern, config, Some(&candidates));
        let regex = compile_user_byte_regex(
            &translated,
            config.is_ignore_case,
            &config.byte_ctype,
            config.locale_regex_encoding,
            config.locale_validator.clone(),
            config.single_byte_locale,
        )?;
        Ok(context_regexp_matches_at_boundary_bytes(
            &regex,
            &content.raw_text,
        ))
    } else {
        let pattern = std::str::from_utf8(pattern).expect("validated UTF-8 context regexp");
        let translated =
            gnu_emacs_regex_to_rust_with_candidates(pattern, config, Some(&candidates));
        let regex = compile_user_regex(&translated, config.is_ignore_case)?;
        Ok(context_regexp_matches_at_boundary(&regex, &content.text))
    }
}

fn ptx_recompile_locale_range_regexps(
    config: &mut PtxConfig,
    word_filter: &mut WordFilter,
    file_map: &FileMap,
) -> CTResult<()> {
    if config.locale_regex_encoding.is_non_utf8_multibyte() {
        return Ok(());
    }
    let candidates = ptx_collect_locale_regex_candidates(file_map, config);
    if candidates.is_empty() {
        return Ok(());
    }
    let has_invalid_input = file_map
        .iter()
        .any(|file| file.invalid_utf8_bytes.iter().any(|&invalid| invalid));

    if let Some(pattern) = config.context_pattern_bytes.clone()
        && !pattern.is_empty()
        && ptx_pattern_needs_locale_class_matching(&pattern)
    {
        let byte_mode = std::str::from_utf8(&pattern).is_err();
        if config.is_ignore_case || byte_mode || has_invalid_input {
            let translated =
                gnu_emacs_regex_to_onig_bytes_with_candidates(&pattern, config, Some(&candidates));
            config.context_byte_regex = Some(compile_user_byte_regex(
                &translated,
                config.is_ignore_case,
                &config.byte_ctype,
                config.locale_regex_encoding,
                config.locale_validator.clone(),
                config.single_byte_locale,
            )?);
            config.context_byte_pattern = Some(translated);
            config.force_byte_mode = true;
        } else {
            let pattern = std::str::from_utf8(&pattern).expect("validated UTF-8 context regexp");
            let translated =
                gnu_emacs_regex_to_rust_with_candidates(pattern, config, Some(&candidates));
            config.context_regex = translated.clone();
            config.context_byte_pattern = None;
            config.context_byte_regex = None;
            compile_user_regex(&translated, config.is_ignore_case)?;
        }
    }

    if let Some(pattern) = word_filter.word_pattern_bytes.as_deref()
        && ptx_pattern_needs_locale_class_matching(pattern)
    {
        let byte_mode = std::str::from_utf8(pattern).is_err();
        if config.is_ignore_case || byte_mode || has_invalid_input {
            let translated =
                gnu_emacs_regex_to_onig_bytes_with_candidates(pattern, config, Some(&candidates));
            config.word_byte_regex = Some(compile_user_byte_regex(
                &translated,
                config.is_ignore_case,
                &config.byte_ctype,
                config.locale_regex_encoding,
                config.locale_validator.clone(),
                config.single_byte_locale,
            )?);
            config.force_byte_mode = true;
        } else {
            let pattern = std::str::from_utf8(pattern).expect("validated UTF-8 word regexp");
            let translated =
                gnu_emacs_regex_to_rust_with_candidates(pattern, config, Some(&candidates));
            word_filter.word_regex.clone_from(&translated);
            config.word_regex = Some(compile_user_regex(&translated, config.is_ignore_case)?);
            config.word_byte_regex = None;
        }
    }
    Ok(())
}

fn build_byte_to_char_map(text: &str) -> Vec<usize> {
    let mut map = vec![0; text.len() + 1];
    let mut char_index = 0usize;
    for (byte_index, ch) in text.char_indices() {
        for slot in &mut map[byte_index..byte_index + ch.len_utf8()] {
            *slot = char_index;
        }
        char_index += 1;
        map[byte_index + ch.len_utf8()] = char_index;
    }
    map[text.len()] = char_index;
    map
}

fn line_index_for_offset(line_starts: &[usize], offset: usize) -> usize {
    match line_starts.binary_search(&offset) {
        Ok(index) => index,
        Err(0) => 0,
        Err(index) => index - 1,
    }
}

fn ptx_internal_text(bytes: &[u8], byte_mode: bool) -> String {
    if byte_mode {
        let mut text = String::with_capacity(bytes.len());
        let mut offset = 0usize;
        while offset < bytes.len() {
            match std::str::from_utf8(&bytes[offset..]) {
                Ok(valid) => {
                    text.push_str(valid);
                    break;
                }
                Err(error) => {
                    let valid_end = offset + error.valid_up_to();
                    text.push_str(
                        std::str::from_utf8(&bytes[offset..valid_end])
                            .expect("validated UTF-8 prefix"),
                    );
                    let invalid_len = error.error_len().unwrap_or_else(|| bytes.len() - valid_end);
                    text.extend(std::iter::repeat_n('\x01', invalid_len));
                    offset = valid_end + invalid_len;
                }
            }
        }
        text
    } else {
        String::from_utf8(bytes.to_vec()).expect("validated UTF-8")
    }
}

fn ptx_internal_byte_text(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&byte| {
            if byte.is_ascii() {
                char::from(byte)
            } else {
                '\x01'
            }
        })
        .collect()
}

fn ptx_invalid_utf8_mask(bytes: &[u8]) -> Vec<bool> {
    let mut invalid = vec![false; bytes.len()];
    let mut offset = 0usize;
    while offset < bytes.len() {
        let Err(error) = std::str::from_utf8(&bytes[offset..]) else {
            break;
        };
        let invalid_start = offset + error.valid_up_to();
        let invalid_len = error
            .error_len()
            .unwrap_or_else(|| bytes.len() - invalid_start);
        invalid[invalid_start..invalid_start + invalid_len].fill(true);
        offset = invalid_start + invalid_len;
    }
    invalid
}

fn ptx_regex_find_at_valid_utf8(
    regex: &Regex,
    text: &str,
    invalid_bytes: &[bool],
    from: usize,
) -> Option<(usize, usize)> {
    debug_assert_eq!(text.len(), invalid_bytes.len());
    let mut cursor = from;
    while cursor < text.len() {
        while cursor < text.len() && invalid_bytes[cursor] {
            cursor += 1;
        }
        let segment_start = cursor;
        while cursor < text.len() && !invalid_bytes[cursor] {
            cursor += 1;
        }
        if segment_start < cursor
            && let Some((start, end)) = regex.find(&text[segment_start..cursor])
        {
            return Some((segment_start + start, segment_start + end));
        }
    }
    None
}

fn ptx_regex_find_iter_valid_utf8(
    regex: &Regex,
    text: &str,
    invalid_bytes: &[bool],
) -> Vec<(usize, usize)> {
    debug_assert_eq!(text.len(), invalid_bytes.len());
    let mut matches = Vec::new();
    let mut cursor = 0usize;
    while cursor < text.len() {
        while cursor < text.len() && invalid_bytes[cursor] {
            cursor += 1;
        }
        let segment_start = cursor;
        while cursor < text.len() && !invalid_bytes[cursor] {
            cursor += 1;
        }
        if segment_start < cursor {
            matches.extend(
                regex
                    .find_iter(&text[segment_start..cursor])
                    .map(|(start, end)| (segment_start + start, segment_start + end)),
            );
        }
    }
    matches
}

fn next_context_end_valid_utf8(
    context_reg: &Regex,
    text: &str,
    invalid_bytes: &[bool],
    start: usize,
) -> usize {
    match ptx_regex_find_at_valid_utf8(context_reg, text, invalid_bytes, start) {
        Some((_, end)) if end > start => end,
        _ => text.len(),
    }
}

fn context_regexp_matches_at_boundary(context_reg: &Regex, text: &str) -> bool {
    let mut context_start = 0usize;
    while context_start < text.len() {
        let Some((start, end)) = context_reg.find_at(text, context_start) else {
            break;
        };
        if start == context_start {
            return true;
        }
        context_start = end;
    }
    false
}

fn context_regexp_matches_at_boundary_valid_utf8(
    context_reg: &Regex,
    text: &str,
    invalid_bytes: &[bool],
) -> bool {
    let mut context_start = 0usize;
    while context_start < text.len() {
        let Some((start, end)) =
            ptx_regex_find_at_valid_utf8(context_reg, text, invalid_bytes, context_start)
        else {
            break;
        };
        if start == context_start {
            return true;
        }
        context_start = end;
    }
    false
}

fn next_context_end_bytes(context_reg: &ByteRegex, bytes: &[u8], start: usize) -> usize {
    match context_reg.find_at(bytes, start) {
        Some((_, end)) if end > start => end,
        _ => bytes.len(),
    }
}

fn context_regexp_matches_at_boundary_bytes(context_reg: &ByteRegex, bytes: &[u8]) -> bool {
    let mut context_start = 0usize;
    while context_start < bytes.len() {
        let Some((start, end)) = context_reg.find_at(bytes, context_start) else {
            break;
        };
        if start == context_start {
            return true;
        }
        context_start = end;
    }
    false
}

fn trim_context_end(text: &str, start: usize, end: usize) -> usize {
    let mut trimmed = end;
    while trimmed > start {
        let prefix = &text[start..trimmed];
        match prefix.chars().next_back() {
            Some(ch) if ptx_is_space_char(ch) => trimmed -= ch.len_utf8(),
            _ => break,
        }
    }
    trimmed
}

fn trim_context_end_bytes(bytes: &[u8], start: usize, end: usize) -> usize {
    let mut trimmed = end;
    while trimmed > start && ptx_is_space_byte(bytes[trimmed - 1]) {
        trimmed -= 1;
    }
    trimmed
}

fn ptx_input_reference_span(line: &str) -> Option<(usize, usize)> {
    let first = line.chars().next()?;
    if ptx_is_space_char(first) {
        return None;
    }

    let end = line
        .char_indices()
        .find_map(|(idx, ch)| ptx_is_space_char(ch).then_some(idx))
        .unwrap_or(line.len());

    Some((0, end))
}

fn ptx_input_reference_content_start(line: &str) -> usize {
    let content_base = ptx_input_reference_span(line).map_or(0, |(_, end)| end);
    let mut start = content_base;
    for (idx, ch) in line[content_base..].char_indices() {
        if !ptx_is_space_char(ch) {
            start = content_base + idx;
            break;
        }
        start = content_base + idx + ch.len_utf8();
    }

    start
}

/// 从输入文件读取内容并构建文件映射
///
/// # 参数
/// * `input_files` - 输入文件路径列表
/// * `config` - PTX 配置，控制是否启用 GNU 扩展
///
/// # 返回值
/// 返回一个 HashMap，键为文件名，值为文件内容和偏移量
fn ptx_read_input(input_files: &[OsString], config: &PtxConfig) -> CTResult<FileMap> {
    // 初始化文件数组
    let mut file_map: FileMap = Vec::new();
    let mut files = Vec::new();

    if input_files.is_empty() {
        files.push(None);
    } else if config.is_gnu_ext {
        files.extend(input_files.iter().map(Some));
    } else {
        files.push(Some(&input_files[0]));
    }

    let context_reg = compile_regex_case_lossy(&config.context_regex, config.is_ignore_case);
    let mut offset: usize = 0;
    for filename in files {
        let filename_bytes = filename.map_or(b"-".as_slice(), |name| name.as_os_str().as_bytes());
        let using_stdin = filename_bytes.is_empty() || filename_bytes == b"-";
        let display_name = filename
            .filter(|name| !name.is_empty())
            .cloned()
            .unwrap_or_else(|| OsString::from("-"));
        let mut reader: BufReader<Box<dyn Read>> = BufReader::new(if using_stdin {
            ctcore::ct_io::stdin_reader_box()
        } else {
            Box::new(
                File::open(filename.expect("non-stdin filename"))
                    .map_err(|error| ptx_file_io_error(display_name.as_os_str(), error))?,
            )
        });
        let mut input_bytes = Vec::new();
        reader
            .read_to_end(&mut input_bytes)
            .map_err(|error| ptx_file_io_error(display_name.as_os_str(), error))?;
        let mut raw_lines: Vec<Vec<u8>> = if input_bytes.is_empty() {
            Vec::new()
        } else {
            input_bytes
                .split(|&byte| byte == b'\n')
                .map(<[u8]>::to_vec)
                .collect()
        };
        if input_bytes.ends_with(b"\n") {
            raw_lines.pop();
        }
        let mut raw_text = Vec::new();
        for (index, line) in raw_lines.iter().enumerate() {
            if index > 0 {
                raw_text.push(b'\n');
            }
            raw_text.extend_from_slice(line);
        }
        if input_bytes.ends_with(b"\n") {
            raw_text.push(b'\n');
        }
        let byte_mode = config.force_byte_mode || std::str::from_utf8(&raw_text).is_err();
        let invalid_utf8_bytes = ptx_invalid_utf8_mask(&raw_text);
        let lines: Vec<String> = raw_lines
            .iter()
            .map(|line| {
                if config.force_byte_mode {
                    ptx_internal_byte_text(line)
                } else {
                    ptx_internal_text(line, byte_mode)
                }
            })
            .collect();
        let mut text = String::new();
        let mut line_starts = Vec::with_capacity(lines.len());
        for (index, line) in lines.iter().enumerate() {
            if index > 0 {
                text.push('\n');
            }
            line_starts.push(text.len());
            text.push_str(line);
        }
        if input_bytes.ends_with(b"\n") {
            text.push('\n');
        }
        let chars_text: Vec<char> = text.chars().collect();
        let byte_to_char = build_byte_to_char_map(&text);
        let chars_lines: Vec<Vec<char>> = lines.iter().map(|x| x.chars().collect()).collect();

        let size = lines.len();
        let content = FileContent {
            filename: if using_stdin {
                String::new()
            } else {
                filename
                    .expect("non-stdin filename")
                    .to_string_lossy()
                    .into_owned()
            },
            raw_filename: if using_stdin {
                Vec::new()
            } else {
                filename_bytes.to_vec()
            },
            text,
            raw_text,
            invalid_utf8_bytes,
            chars_text,
            byte_to_char,
            line_starts,
            lines,
            raw_lines,
            chars_lines,
            offset,
        };
        let has_boundary_match = config.context_byte_regex.as_ref().map_or_else(
            || {
                context_regexp_matches_at_boundary_valid_utf8(
                    &context_reg,
                    &content.text,
                    &content.invalid_utf8_bytes,
                )
            },
            |regex| context_regexp_matches_at_boundary_bytes(regex, &content.raw_text),
        ) || ptx_locale_context_matches_at_boundary(config, &content)?;
        if has_boundary_match {
            let default_pattern = if config.is_gnu_ext && !config.is_input_ref {
                GNU_DEFAULT_CONTEXT_PATTERN
            } else {
                config.context_regex.as_bytes()
            };
            let pattern = config
                .context_pattern_bytes
                .as_deref()
                .unwrap_or(default_pattern);
            return Err(ptx_zero_length_regex_error(
                pattern,
                &config.byte_ctype,
                config.single_byte_locale,
            ));
        }
        file_map.push(content);
        offset += size;
    }
    Ok(file_map)
}

/// 从文件内容中提取单词并创建单词引用集合
///
/// # 参数
/// * `config` - PTX 配置，控制大小写敏感性和引用处理
/// * `filter` - 单词过滤器，定义单词匹配和过滤规则
/// * `file_map` - 文件内容映射
///
/// # 返回值
/// 返回一个有序集合，包含所有匹配的单词引用
fn ptx_create_word_set(
    config: &PtxConfig,
    filter: &WordFilter,
    file_map: &FileMap,
) -> BTreeSet<WordRef> {
    let reg = compile_regex_case_lossy(&filter.word_regex, config.is_ignore_case);
    let ref_reg = compile_regex_case_lossy(&config.context_regex, config.is_ignore_case);
    let mut word_set: BTreeSet<WordRef> = BTreeSet::new();

    for (file_idx, content) in file_map.iter().enumerate() {
        let mut reference_line_start = 0usize;
        let mut reference_scan = 0usize;
        let mut reference_length = 0usize;
        let mut reference_line_nr = 0usize;
        if config.is_input_ref {
            reference_scan =
                ptx_skip_non_white_bytes(&content.raw_text, reference_scan, content.raw_text.len());
            reference_length = reference_scan - reference_line_start;
            reference_scan =
                ptx_skip_white_bytes(&content.raw_text, reference_scan, content.raw_text.len());
        }

        let mut context_start = 0usize;
        while context_start < content.raw_text.len() {
            let (context_end_raw, context_end) = if let Some(byte_regex) =
                &config.context_byte_regex
            {
                let raw_end = next_context_end_bytes(byte_regex, &content.raw_text, context_start);
                let end = trim_context_end_bytes(&content.raw_text, context_start, raw_end);
                (raw_end, end)
            } else {
                let raw_end = next_context_end_valid_utf8(
                    &ref_reg,
                    &content.text,
                    &content.invalid_utf8_bytes,
                    context_start,
                );
                let end = trim_context_end(&content.text, context_start, raw_end);
                (raw_end, end)
            };
            let context_text = &content.text[context_start..context_end];
            let mut output_context_start = context_start;

            let matches: Vec<(usize, usize)> = if let Some(byte_regex) = &config.word_byte_regex {
                byte_regex
                    .find_iter(&content.raw_text[context_start..context_end])
                    .collect()
            } else if let Some(break_set) = &filter.break_set {
                let mut ranges = Vec::new();
                let mut cursor = context_start;
                while cursor < context_end {
                    while cursor < context_end && break_set.contains(&content.raw_text[cursor]) {
                        cursor += 1;
                    }
                    let start = cursor;
                    while cursor < context_end && !break_set.contains(&content.raw_text[cursor]) {
                        cursor += 1;
                    }
                    if start < cursor {
                        ranges.push((start - context_start, cursor - context_start));
                    }
                }
                ranges
            } else if !filter.uses_custom_regex {
                let mut ranges = Vec::new();
                let mut cursor = context_start;
                let is_word_byte = |byte| {
                    if config.is_gnu_ext {
                        config.byte_ctype.is_alpha(byte)
                    } else {
                        !matches!(byte, b' ' | b'\t' | b'\n')
                    }
                };
                while cursor < context_end {
                    while cursor < context_end && !is_word_byte(content.raw_text[cursor]) {
                        cursor += 1;
                    }
                    let start = cursor;
                    while cursor < context_end && is_word_byte(content.raw_text[cursor]) {
                        cursor += 1;
                    }
                    if start < cursor {
                        ranges.push((start - context_start, cursor - context_start));
                    }
                }
                ranges
            } else {
                ptx_regex_find_iter_valid_utf8(
                    &reg,
                    context_text,
                    &content.invalid_utf8_bytes[context_start..context_end],
                )
            };

            for (start, end) in matches {
                if start == end {
                    continue;
                }
                let (global_beg, global_end) = (context_start + start, context_start + end);
                let local_line_nr = line_index_for_offset(&content.line_starts, global_beg);
                let line_start = content.line_starts[local_line_nr];
                if config.is_input_ref {
                    while reference_scan < global_beg {
                        if content.raw_text[reference_scan] == b'\n' {
                            reference_line_nr += 1;
                            reference_scan += 1;
                            reference_line_start = reference_scan;
                            reference_scan = ptx_skip_non_white_bytes(
                                &content.raw_text,
                                reference_scan,
                                content.raw_text.len(),
                            );
                            reference_length = reference_scan - reference_line_start;
                        } else {
                            reference_scan += 1;
                        }
                    }
                    if reference_scan > global_beg {
                        continue;
                    }
                }

                let mut word = content.text[global_beg..global_end].to_owned();
                let mut raw_word = content.raw_text[global_beg..global_end].to_vec();
                let filter_word = if config.is_ignore_case {
                    word.to_lowercase()
                } else {
                    word.clone()
                };
                if config.is_ignore_case {
                    config.byte_ctype.uppercase(&mut raw_word);
                }
                if filter.is_only_specified && !filter.only_set.contains(&raw_word) {
                    continue;
                }
                if filter.is_ignore_specified && filter.ignore_set.contains(&raw_word) {
                    continue;
                }
                if config.is_ignore_case {
                    word = filter_word;
                }

                if config.is_input_ref && reference_line_start == output_context_start {
                    output_context_start = ptx_skip_non_white_bytes(
                        &content.raw_text,
                        output_context_start,
                        context_end,
                    );
                    output_context_start =
                        ptx_skip_white_bytes(&content.raw_text, output_context_start, context_end);
                }
                let global_char_position = content.byte_to_char[global_beg];
                let global_char_position_end = content.byte_to_char[global_end];
                let context_char_start = content.byte_to_char[output_context_start];
                let context_char_end = content.byte_to_char[context_end];
                word_set.insert(WordRef {
                    raw_word,
                    word,
                    file_index: file_idx,
                    global_line_nr: content.offset + local_line_nr,
                    local_line_nr,
                    position: global_beg - line_start,
                    position_end: global_end - line_start,
                    global_position: global_beg,
                    global_position_end: global_end,
                    context_start: output_context_start,
                    context_end,
                    global_char_position,
                    global_char_position_end,
                    context_char_start,
                    context_char_end,
                    input_reference: if config.is_input_ref {
                        content.raw_text
                            [reference_line_start..reference_line_start + reference_length]
                            .to_vec()
                    } else {
                        Vec::new()
                    },
                    reference_line_nr: if config.is_input_ref {
                        reference_line_nr
                    } else {
                        local_line_nr
                    },
                });
            }

            if context_end_raw <= context_start {
                break;
            }
            context_start = context_end_raw;
        }
    }
    word_set
}

/// 获取单词的引用字符串
///
/// # 参数
/// * `config` - PTX 配置，控制引用生成方式
/// * `word_ref` - 单词引用信息
/// * `line` - 包含单词的行文本
/// * `context_reg` - 上下文正则表达式
///
/// # 返回值
/// 返回生成的引用字符串
fn ptx_get_reference(
    config: &PtxConfig,
    word_ref: &WordRef,
    file_name: &str,
    _line: &str,
    _context_reg: &Regex,
) -> String {
    if config.is_auto_ref {
        format!("{}:{}", file_name, word_ref.reference_line_nr + 1)
    } else if config.is_input_ref {
        let byte_mode = std::str::from_utf8(&word_ref.input_reference).is_err();
        ptx_internal_text(&word_ref.input_reference, byte_mode)
    } else {
        String::new()
    }
}

fn ptx_get_reference_bytes(
    config: &PtxConfig,
    word_ref: &WordRef,
    content: &FileContent,
) -> Vec<u8> {
    if config.is_auto_ref {
        let mut reference = content.raw_filename.clone();
        reference.push(b':');
        reference.extend_from_slice((word_ref.reference_line_nr + 1).to_string().as_bytes());
        reference
    } else if config.is_input_ref {
        word_ref.input_reference.clone()
    } else {
        Vec::new()
    }
}

fn run_ptx_direct_process(
    argv: &[OsString],
    stdin_bytes: Option<&[u8]>,
) -> CTResult<DirectPtxInvocation> {
    let current_exe = std::env::current_exe()?;
    let mut command = ProcessCommand::new(current_exe);
    command
        .args(argv)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if stdin_bytes.is_some() {
        command.stdin(Stdio::piped());
    } else {
        command.stdin(Stdio::inherit());
    }

    let mut child = command.spawn()?;
    let mut stdin_write_error = None;
    if let Some(bytes) = stdin_bytes
        && let Some(mut stdin) = child.stdin.take()
        && let Err(err) = stdin.write_all(bytes)
        && err.kind() != std::io::ErrorKind::BrokenPipe
    {
        stdin_write_error = Some(err);
    }

    let output = child.wait_with_output()?;
    if let Some(err) = stdin_write_error {
        return Err(err.into());
    }

    Ok(DirectPtxInvocation {
        stdout: output.stdout,
        stderr: output.stderr,
        exit_code: output.status.code().unwrap_or(1),
    })
}

fn ptx_format_name(format: &OutFormat) -> &'static str {
    match format {
        OutFormat::Dumb => "dumb",
        OutFormat::Roff => "roff",
        OutFormat::Tex => "tex",
    }
}

#[derive(Debug, Clone, Default)]
struct PtxOutputFields {
    tail: String,
    before: String,
    keyafter: String,
    head: String,
    keyword_len: usize,
    before_padding_adjustment: usize,
    tail_truncation: bool,
    before_truncation: bool,
    keyafter_truncation: bool,
    head_truncation: bool,
}

fn ptx_chars_to_string(chars: &[char], start: usize, end: usize) -> String {
    if start >= end || start >= chars.len() {
        return String::new();
    }
    chars[start..end.min(chars.len())].iter().collect()
}

fn ptx_skip_white(chars: &[char], mut cursor: usize, limit: usize) -> usize {
    while cursor < limit && ptx_is_space_char(chars[cursor]) {
        cursor += 1;
    }
    cursor
}

fn ptx_skip_white_backwards(chars: &[char], mut cursor: usize, start: usize) -> usize {
    while cursor > start && ptx_is_space_char(chars[cursor - 1]) {
        cursor -= 1;
    }
    cursor
}

fn ptx_is_default_word_char(config: &PtxConfig, c: char) -> bool {
    if let Some(break_bytes) = &config.word_break_bytes {
        return u8::try_from(c).is_ok_and(|byte| !break_bytes.contains(&byte));
    }
    if config.is_gnu_ext {
        u8::try_from(c).is_ok_and(|byte| config.byte_ctype.is_alpha(byte))
    } else {
        !matches!(c, ' ' | '\t' | '\n')
    }
}

fn ptx_skip_something(chars: &[char], cursor: usize, limit: usize, config: &PtxConfig) -> usize {
    if cursor >= limit {
        return cursor;
    }
    if let Some(regex) = &config.word_regex {
        let segment: String = chars[cursor..limit].iter().collect();
        return regex
            .find(&segment)
            .filter(|&(start, end)| start == 0 && end > 0)
            .map_or(cursor + 1, |(_, end)| {
                cursor + segment[..end].chars().count()
            });
    }

    let mut next = cursor;
    if ptx_is_default_word_char(config, chars[next]) {
        while next < limit && ptx_is_default_word_char(config, chars[next]) {
            next += 1;
        }
    } else {
        next += 1;
    }
    next
}

fn ptx_field_dimensions_for_trunc_len(
    config: &PtxConfig,
    line_width: usize,
    trunc_len: usize,
) -> (usize, usize, usize, usize) {
    let half_line_width = line_width / 2;

    let mut before_max_width = half_line_width.saturating_sub(config.gap_size);
    let mut keyafter_max_width = half_line_width;
    if trunc_len > 0 {
        if config.is_gnu_ext {
            before_max_width = before_max_width.saturating_sub(2 * trunc_len);
            keyafter_max_width = keyafter_max_width.saturating_sub(2 * trunc_len);
        } else {
            keyafter_max_width = keyafter_max_width.saturating_sub(2 * trunc_len + 1);
        }
    }

    (
        half_line_width,
        before_max_width,
        keyafter_max_width,
        trunc_len,
    )
}

fn ptx_field_dimensions(config: &PtxConfig, line_width: usize) -> (usize, usize, usize, usize) {
    ptx_field_dimensions_for_trunc_len(config, line_width, config.trunc_str.chars().count())
}

fn ptx_field_dimensions_bytes(
    config: &PtxConfig,
    line_width: usize,
) -> (usize, usize, usize, usize) {
    ptx_field_dimensions_for_trunc_len(config, line_width, config.trunc_bytes.len())
}

fn ptx_content_maximum_word_length(content: &FileContent, config: &PtxConfig) -> usize {
    ptx_maximum_word_length_in_chars(&content.chars_text, config)
}

fn ptx_content_maximum_word_length_bytes(content: &FileContent, config: &PtxConfig) -> usize {
    ptx_maximum_word_length_in_bytes(&content.raw_text, config)
}

fn ptx_global_maximum_word_length(file_map: &FileMap, config: &PtxConfig) -> usize {
    file_map
        .iter()
        .map(|content| ptx_content_maximum_word_length(content, config))
        .max()
        .unwrap_or(0)
}

fn ptx_global_maximum_word_length_bytes(file_map: &FileMap, config: &PtxConfig) -> usize {
    file_map
        .iter()
        .map(|content| ptx_content_maximum_word_length_bytes(content, config))
        .max()
        .unwrap_or(0)
}

fn ptx_maximum_word_length_in_chars(chars: &[char], config: &PtxConfig) -> usize {
    if let Some(regex) = &config.word_regex {
        let text: String = chars.iter().collect();
        return regex
            .find_iter(&text)
            .map(|(start, end)| text[start..end].chars().count())
            .max()
            .unwrap_or(0);
    }
    let mut max_len = 0usize;
    let mut cursor = 0usize;
    while cursor < chars.len() {
        if ptx_is_default_word_char(config, chars[cursor]) {
            let start = cursor;
            while cursor < chars.len() && ptx_is_default_word_char(config, chars[cursor]) {
                cursor += 1;
            }
            max_len = max_len.max(cursor - start);
        } else {
            cursor += 1;
        }
    }
    max_len
}

fn ptx_maximum_word_length_in_bytes(bytes: &[u8], config: &PtxConfig) -> usize {
    if let Some(regex) = &config.word_byte_regex {
        return regex
            .find_iter(bytes)
            .map(|(start, end)| end - start)
            .max()
            .unwrap_or(0);
    }
    if let Some(regex) = &config.word_regex {
        let mut max_len = 0usize;
        let mut offset = 0usize;
        while offset < bytes.len() {
            match std::str::from_utf8(&bytes[offset..]) {
                Ok(text) => {
                    max_len = max_len.max(
                        regex
                            .find_iter(text)
                            .map(|(start, end)| end - start)
                            .max()
                            .unwrap_or(0),
                    );
                    break;
                }
                Err(error) => {
                    let valid_len = error.valid_up_to();
                    if valid_len > 0 {
                        let text = std::str::from_utf8(&bytes[offset..offset + valid_len])
                            .expect("validated UTF-8 prefix");
                        max_len = max_len.max(
                            regex
                                .find_iter(text)
                                .map(|(start, end)| end - start)
                                .max()
                                .unwrap_or(0),
                        );
                    }
                    let invalid_len = error
                        .error_len()
                        .unwrap_or_else(|| bytes.len() - offset - valid_len);
                    offset += valid_len + invalid_len;
                }
            }
        }
        return max_len;
    }
    let mut max_len = 0usize;
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        if ptx_is_default_word_byte(config, bytes[cursor]) {
            let start = cursor;
            while cursor < bytes.len() && ptx_is_default_word_byte(config, bytes[cursor]) {
                cursor += 1;
            }
            max_len = max_len.max(cursor - start);
        } else {
            cursor += 1;
        }
    }
    max_len
}

fn ptx_define_output_fields_for_width(
    all_before: &[char],
    keyword: &str,
    all_after: &[char],
    config: &PtxConfig,
    line_width: usize,
    maximum_word_length: usize,
) -> PtxOutputFields {
    let (half_line_width, before_max_width, keyafter_max_width, _) =
        ptx_field_dimensions(config, line_width);
    let truncation_enabled = !config.trunc_str.is_empty();

    let mut chars =
        Vec::with_capacity(all_before.len() + keyword.chars().count() + all_after.len());
    chars.extend_from_slice(all_before);
    let keyword_chars: Vec<char> = keyword.chars().collect();
    chars.extend(keyword_chars.iter());
    chars.extend_from_slice(all_after);

    let context_start = 0usize;
    let context_end = chars.len();
    let key_start = all_before.len();
    let key_end = key_start + keyword_chars.len();
    let left_context_start = context_start;
    let right_context_end = context_end;
    let left_field_start = if key_start.saturating_sub(left_context_start)
        > half_line_width.saturating_add(maximum_word_length)
    {
        let jump = half_line_width.saturating_add(maximum_word_length);
        let mut start = key_start.saturating_sub(jump);
        start = ptx_skip_something(&chars, start, key_start, config);
        start
    } else {
        left_context_start
    };

    let keyafter_start = key_start;
    let mut keyafter_end = key_end;
    let mut cursor = keyafter_end;
    let keyafter_limit = keyafter_start.saturating_add(keyafter_max_width);
    while cursor < right_context_end && cursor <= keyafter_limit {
        keyafter_end = cursor;
        cursor = ptx_skip_something(&chars, cursor, right_context_end, config);
    }
    if cursor <= keyafter_limit {
        keyafter_end = cursor;
    }
    let mut keyafter_truncation = truncation_enabled && keyafter_end < right_context_end;
    keyafter_end = ptx_skip_white_backwards(&chars, keyafter_end, keyafter_start);

    let mut before_start = left_field_start;
    let mut before_end = keyafter_start;
    before_end = ptx_skip_white_backwards(&chars, before_end, before_start);
    while before_start.saturating_add(before_max_width) < before_end {
        let next = ptx_skip_something(&chars, before_start, before_end, config);
        if next <= before_start {
            break;
        }
        before_start = next;
    }

    let mut before_truncation = if truncation_enabled {
        ptx_skip_white_backwards(&chars, before_start, context_start) > left_context_start
    } else {
        false
    };
    before_start = ptx_skip_white(&chars, before_start, context_end);

    let before_len = before_end.saturating_sub(before_start);
    let before_padding_adjustment = before_start.saturating_sub(before_end);
    let tail_max_width = before_max_width
        .saturating_add(before_padding_adjustment)
        .saturating_sub(before_len)
        .saturating_sub(config.gap_size);
    let (tail_start, tail_end, tail_truncation) = if tail_max_width > 0 {
        let tail_start = ptx_skip_white(&chars, keyafter_end, context_end);
        let mut tail_end = tail_start;
        let mut cursor = tail_end;
        let tail_limit = tail_start.saturating_add(tail_max_width);
        while cursor < right_context_end && cursor < tail_limit {
            tail_end = cursor;
            cursor = ptx_skip_something(&chars, cursor, right_context_end, config);
        }
        if cursor < tail_limit {
            tail_end = cursor;
        }

        let mut tail_truncation = false;
        if tail_end > tail_start {
            keyafter_truncation = false;
            tail_truncation = truncation_enabled && tail_end < right_context_end;
        }
        tail_end = ptx_skip_white_backwards(&chars, tail_end, tail_start);
        (tail_start, tail_end, tail_truncation)
    } else {
        (0, 0, false)
    };

    let keyafter_len = keyafter_end.saturating_sub(keyafter_start);
    let head_max_width = keyafter_max_width
        .saturating_sub(keyafter_len)
        .saturating_sub(config.gap_size);
    let (head_start, head_end, head_truncation) = if head_max_width > 0 {
        let head_end = ptx_skip_white_backwards(&chars, before_start, context_start);
        let mut head_start = left_field_start;
        while head_start.saturating_add(head_max_width) < head_end {
            let next = ptx_skip_something(&chars, head_start, head_end, config);
            if next <= head_start {
                break;
            }
            head_start = next;
        }

        let mut head_truncation = false;
        if head_end > head_start {
            before_truncation = false;
            head_truncation = truncation_enabled && head_start > left_context_start;
        }
        head_start = ptx_skip_white(&chars, head_start, head_end);
        (head_start, head_end, head_truncation)
    } else {
        (0, 0, false)
    };

    PtxOutputFields {
        tail: ptx_chars_to_string(&chars, tail_start, tail_end),
        before: ptx_chars_to_string(&chars, before_start, before_end),
        keyafter: ptx_chars_to_string(&chars, keyafter_start, keyafter_end),
        head: ptx_chars_to_string(&chars, head_start, head_end),
        keyword_len: keyword_chars.len(),
        before_padding_adjustment,
        tail_truncation,
        before_truncation,
        keyafter_truncation,
        head_truncation,
    }
}

#[derive(Debug, Clone, Default)]
struct PtxOutputFieldsBytes {
    tail: Vec<u8>,
    before: Vec<u8>,
    keyafter: Vec<u8>,
    head: Vec<u8>,
    before_padding_adjustment: usize,
    tail_truncation: bool,
    before_truncation: bool,
    keyafter_truncation: bool,
    head_truncation: bool,
}

fn ptx_is_default_word_byte(config: &PtxConfig, byte: u8) -> bool {
    if let Some(break_bytes) = &config.word_break_bytes {
        return !break_bytes.contains(&byte);
    }
    if config.is_gnu_ext {
        config.byte_ctype.is_alpha(byte)
    } else {
        !matches!(byte, b' ' | b'\t' | b'\n')
    }
}

fn ptx_skip_white_bytes(bytes: &[u8], mut cursor: usize, limit: usize) -> usize {
    while cursor < limit && ptx_is_space_byte(bytes[cursor]) {
        cursor += 1;
    }
    cursor
}

fn ptx_skip_non_white_bytes(bytes: &[u8], mut cursor: usize, limit: usize) -> usize {
    while cursor < limit && !ptx_is_space_byte(bytes[cursor]) {
        cursor += 1;
    }
    cursor
}

fn ptx_skip_white_backwards_bytes(bytes: &[u8], mut cursor: usize, start: usize) -> usize {
    while cursor > start && ptx_is_space_byte(bytes[cursor - 1]) {
        cursor -= 1;
    }
    cursor
}

fn ptx_skip_something_bytes(
    bytes: &[u8],
    cursor: usize,
    limit: usize,
    config: &PtxConfig,
) -> usize {
    if cursor >= limit {
        return cursor;
    }
    if let Some(regex) = &config.word_byte_regex {
        if regex.valid_character_len(&bytes[cursor..limit]).is_none() {
            return cursor + 1;
        }
        return regex
            .find_at(&bytes[cursor..limit], 0)
            .filter(|&(start, end)| start == 0 && end > 0)
            .map_or(cursor + 1, |(_, end)| cursor + end);
    }
    if let Some(regex) = &config.word_regex {
        let remaining = &bytes[cursor..limit];
        let valid_len =
            std::str::from_utf8(remaining).map_or_else(|error| error.valid_up_to(), str::len);
        if valid_len > 0 {
            let segment =
                std::str::from_utf8(&remaining[..valid_len]).expect("validated UTF-8 prefix");
            return regex
                .find(segment)
                .filter(|&(start, end)| start == 0 && end > 0)
                .map_or(cursor + 1, |(_, end)| cursor + end);
        }
        return cursor + 1;
    }

    let mut next = cursor;
    if ptx_is_default_word_byte(config, bytes[next]) {
        while next < limit && ptx_is_default_word_byte(config, bytes[next]) {
            next += 1;
        }
    } else {
        next += 1;
    }
    next
}

fn ptx_bytes_to_vec(bytes: &[u8], start: usize, end: usize) -> Vec<u8> {
    if start >= end || start >= bytes.len() {
        return Vec::new();
    }
    bytes[start..end.min(bytes.len())].to_vec()
}

fn ptx_define_output_fields_bytes_for_width(
    all_before: &[u8],
    keyword: &[u8],
    all_after: &[u8],
    left_context_after_keyword: usize,
    config: &PtxConfig,
    line_width: usize,
    maximum_word_length: usize,
) -> PtxOutputFieldsBytes {
    let (half_line_width, before_max_width, keyafter_max_width, _) =
        ptx_field_dimensions_bytes(config, line_width);
    let truncation_enabled = !config.trunc_bytes.is_empty();

    let mut bytes = Vec::with_capacity(all_before.len() + keyword.len() + all_after.len());
    bytes.extend_from_slice(all_before);
    bytes.extend_from_slice(keyword);
    bytes.extend_from_slice(all_after);

    let context_start = 0usize;
    let context_end = bytes.len();
    let key_start = all_before.len();
    let key_end = key_start + keyword.len();
    // GNU stores the left-context boundary as a signed displacement from the
    // keyword.  In input-reference mode, leading whitespace can itself be a
    // custom-regexp keyword before the reference prefix is removed, leaving
    // that boundary after the keyword start.
    let left_context_start = if left_context_after_keyword == 0 {
        context_start
    } else {
        key_start + left_context_after_keyword
    };
    let right_context_end = context_end;
    let left_field_start = if key_start.saturating_sub(left_context_start)
        > half_line_width.saturating_add(maximum_word_length)
    {
        let jump = half_line_width.saturating_add(maximum_word_length);
        let mut start = key_start.saturating_sub(jump);
        start = ptx_skip_something_bytes(&bytes, start, key_start, config);
        start
    } else {
        left_context_start
    };

    let keyafter_start = key_start;
    let mut keyafter_end = key_end;
    let mut cursor = keyafter_end;
    let keyafter_limit = keyafter_start.saturating_add(keyafter_max_width);
    while cursor < right_context_end && cursor <= keyafter_limit {
        keyafter_end = cursor;
        cursor = ptx_skip_something_bytes(&bytes, cursor, right_context_end, config);
    }
    if cursor <= keyafter_limit {
        keyafter_end = cursor;
    }
    let mut keyafter_truncation = truncation_enabled && keyafter_end < right_context_end;
    keyafter_end = ptx_skip_white_backwards_bytes(&bytes, keyafter_end, keyafter_start);

    let mut before_start = left_field_start;
    let mut before_end = keyafter_start;
    before_end = ptx_skip_white_backwards_bytes(&bytes, before_end, before_start);
    while before_start.saturating_add(before_max_width) < before_end {
        let next = ptx_skip_something_bytes(&bytes, before_start, before_end, config);
        if next <= before_start {
            break;
        }
        before_start = next;
    }

    let mut before_truncation = if truncation_enabled {
        ptx_skip_white_backwards_bytes(&bytes, before_start, context_start) > left_context_start
    } else {
        false
    };
    before_start = ptx_skip_white_bytes(&bytes, before_start, context_end);

    let before_len = before_end.saturating_sub(before_start);
    let before_padding_adjustment = before_start.saturating_sub(before_end);
    let tail_max_width = before_max_width
        .saturating_add(before_padding_adjustment)
        .saturating_sub(before_len)
        .saturating_sub(config.gap_size);
    let (tail_start, tail_end, tail_truncation) = if tail_max_width > 0 {
        let tail_start = ptx_skip_white_bytes(&bytes, keyafter_end, context_end);
        let mut tail_end = tail_start;
        let mut cursor = tail_end;
        let tail_limit = tail_start.saturating_add(tail_max_width);
        while cursor < right_context_end && cursor < tail_limit {
            tail_end = cursor;
            cursor = ptx_skip_something_bytes(&bytes, cursor, right_context_end, config);
        }
        if cursor < tail_limit {
            tail_end = cursor;
        }

        let mut tail_truncation = false;
        if tail_end > tail_start {
            keyafter_truncation = false;
            tail_truncation = truncation_enabled && tail_end < right_context_end;
        }
        tail_end = ptx_skip_white_backwards_bytes(&bytes, tail_end, tail_start);
        (tail_start, tail_end, tail_truncation)
    } else {
        (0, 0, false)
    };

    let keyafter_len = keyafter_end.saturating_sub(keyafter_start);
    let head_max_width = keyafter_max_width
        .saturating_sub(keyafter_len)
        .saturating_sub(config.gap_size);
    let (head_start, head_end, head_truncation) = if head_max_width > 0 {
        let head_end = ptx_skip_white_backwards_bytes(&bytes, before_start, context_start);
        let mut head_start = left_field_start;
        while head_start.saturating_add(head_max_width) < head_end {
            let next = ptx_skip_something_bytes(&bytes, head_start, head_end, config);
            if next <= head_start {
                break;
            }
            head_start = next;
        }

        let mut head_truncation = false;
        if head_end > head_start {
            before_truncation = false;
            head_truncation = truncation_enabled && head_start > left_context_start;
        }
        head_start = ptx_skip_white_bytes(&bytes, head_start, head_end);
        (head_start, head_end, head_truncation)
    } else {
        (0, 0, false)
    };

    PtxOutputFieldsBytes {
        tail: ptx_bytes_to_vec(&bytes, tail_start, tail_end),
        before: ptx_bytes_to_vec(&bytes, before_start, before_end),
        keyafter: ptx_bytes_to_vec(&bytes, keyafter_start, keyafter_end),
        head: ptx_bytes_to_vec(&bytes, head_start, head_end),
        before_padding_adjustment,
        tail_truncation,
        before_truncation,
        keyafter_truncation,
        head_truncation,
    }
}

/// 获取格式化的输出文本块
///
/// 该函数基于 GNU ptx 源码实现，将输入文本分割成四个部分：
/// - tail: 右侧上下文的尾部
/// - before: 关键词前的文本
/// - after: 关键词后的文本
/// - head: 左侧上下文的头部
///
/// 每个部分的大小受以下因素限制：
/// - line_width: 总行宽度
/// - gap_size: 部分之间的间隔大小
/// - trunc_str: 截断标记字符串
///
/// # 参数
/// * `all_before` - 关键词前的所有字符
/// * `keyword` - 关键词字符串
/// * `all_after` - 关键词后的所有字符
/// * `config` - PTX 配置参数
///
/// # 返回值
/// 返回一个元组 (tail, before, after, head)，每个部分都是格式化后的字符串
fn ptx_get_output_chunks_for_width_with_max(
    all_before: &[char],
    keyword: &str,
    all_after: &[char],
    config: &PtxConfig,
    line_width: usize,
    maximum_word_length: usize,
) -> (String, String, String, String, usize) {
    let fields = ptx_define_output_fields_for_width(
        all_before,
        keyword,
        all_after,
        config,
        line_width,
        maximum_word_length,
    );
    let mut tail = fields.tail;
    if fields.tail_truncation {
        tail.push_str(&config.trunc_str);
    }
    let before = if fields.before_truncation {
        format!("{}{}", config.trunc_str, fields.before)
    } else {
        fields.before
    };
    let mut after: String = fields.keyafter.chars().skip(fields.keyword_len).collect();
    if fields.keyafter_truncation {
        after.push_str(&config.trunc_str);
    }
    let head = if fields.head_truncation {
        format!("{}{}", config.trunc_str, fields.head)
    } else {
        fields.head
    };
    (tail, before, after, head, fields.before_padding_adjustment)
}

fn tex_mapper(x: char) -> String {
    match x {
        c if ptx_is_space_char(c) => " ".to_string(),
        '\\' => "\\backslash{}".to_owned(),
        '$' | '%' | '#' | '&' | '_' => format!("\\{x}"),
        '}' | '{' => format!("$\\{x}$"),
        _ => x.to_string(),
    }
}

/// Escape special characters for TeX.
fn format_tex_field(s: &str) -> String {
    let mapped_chunks: Vec<String> = s.chars().map(tex_mapper).collect();
    mapped_chunks.join("")
}

/// 格式化输出为 TeX 格式
#[allow(clippy::too_many_arguments)]
fn ptx_format_tex_line(
    config: &PtxConfig,
    word_ref: &WordRef,
    line: &str,
    chars_line: &[char],
    text: &str,
    chars_text: &[char],
    context_reg: &Regex,
    reference: &str,
    line_width: usize,
    maximum_word_length: usize,
) -> String {
    let mut output = String::with_capacity(line.len() * 2);

    let (keyword, all_before, all_after, _) = ptx_context_slices(
        config,
        word_ref,
        text,
        chars_text,
        line,
        chars_line,
        context_reg,
    );

    let fields = ptx_define_output_fields_for_width(
        all_before,
        keyword,
        all_after,
        config,
        line_width,
        maximum_word_length,
    );
    let keyafter_chars: Vec<char> = fields.keyafter.chars().collect();
    let (key, after): (String, String) = if keyafter_chars.is_empty() && config.word_regex.is_some()
    {
        (keyword.chars().next().into_iter().collect(), String::new())
    } else {
        let key_end = ptx_skip_something(&keyafter_chars, 0, keyafter_chars.len(), config);
        (
            keyafter_chars[..key_end].iter().collect(),
            keyafter_chars[key_end..].iter().collect(),
        )
    };

    write!(
        output,
        "\\{} {{{}}}{{{}}}{{{}}}{{{}}}{{{}}}",
        config.macro_name,
        format_tex_field(&fields.tail),
        format_tex_field(&fields.before),
        format_tex_field(&key),
        format_tex_field(&after),
        format_tex_field(&fields.head),
    )
    .unwrap();

    // 添加引用信息
    if config.is_auto_ref || config.is_input_ref {
        write!(output, "{{{}}}", format_tex_field(reference)).unwrap();
    }

    output
}

fn ptx_format_roff_field(s: &str) -> String {
    s.chars()
        .map(|c| {
            if ptx_is_space_char(c) {
                " ".to_string()
            } else if c == '"' {
                "\"\"".to_string()
            } else {
                c.to_string()
            }
        })
        .collect::<String>()
}

fn ptx_context_slices<'a>(
    config: &PtxConfig,
    word_ref: &WordRef,
    text: &'a str,
    chars_text: &'a [char],
    line: &'a str,
    chars_line: &'a [char],
    context_reg: &Regex,
) -> (&'a str, &'a [char], &'a [char], &'a [char]) {
    if word_ref.context_end > word_ref.context_start
        && word_ref.global_position_end <= text.len()
        && word_ref.context_end <= text.len()
    {
        let keyword = &text[word_ref.global_position..word_ref.global_position_end];
        let all_before = &chars_text[word_ref.context_char_start..word_ref.global_char_position];
        let all_after = &chars_text[word_ref.global_char_position_end..word_ref.context_char_end];
        return (keyword, all_before, all_after, chars_text);
    }

    let before_start = context_base_start(config, line, chars_line, context_reg);
    let (context_left, context_right) = context_bounds(
        config,
        line,
        context_reg,
        word_ref.position,
        word_ref.position_end,
        before_start,
    );
    let keyword = &line[word_ref.position..word_ref.position_end];
    (
        keyword,
        &chars_line[context_left..word_ref.position],
        &chars_line[word_ref.position_end..context_right],
        chars_line,
    )
}

/// 格式化输出为 Roff 格式
#[allow(clippy::too_many_arguments)]
fn ptx_format_roff_line(
    config: &PtxConfig,
    word_ref: &WordRef,
    line: &str,
    chars_line: &[char],
    text: &str,
    chars_text: &[char],
    context_reg: &Regex,
    reference: &str,
    line_width: usize,
    maximum_word_length: usize,
) -> String {
    let mut output = String::with_capacity(line.len() * 2);
    write!(output, ".{}", config.macro_name).unwrap();

    let (keyword, all_before, all_after, _) = ptx_context_slices(
        config,
        word_ref,
        text,
        chars_text,
        line,
        chars_line,
        context_reg,
    );

    // 获取格式化后的文本块
    let (tail, before, after, head, _) = ptx_get_output_chunks_for_width_with_max(
        all_before,
        keyword,
        all_after,
        config,
        line_width,
        maximum_word_length,
    );

    // 转义特殊字符并构建输出
    write!(
        output,
        " \"{}\" \"{}\" \"{}{}\" \"{}\"",
        ptx_format_roff_field(&tail),
        ptx_format_roff_field(&before),
        ptx_format_roff_field(keyword),
        ptx_format_roff_field(&after),
        ptx_format_roff_field(&head)
    )
    .unwrap();

    // 添加引用信息
    if config.is_auto_ref || config.is_input_ref {
        write!(output, " \"{}\"", ptx_format_roff_field(reference)).unwrap();
    }

    output
}

fn str_cols(s: &str) -> usize {
    s.chars().count()
}

fn ptx_effective_line_width(config: &PtxConfig, reference_max_width: usize) -> usize {
    if (config.is_auto_ref || config.is_input_ref) && !config.is_right_ref {
        config
            .line_width
            .saturating_sub(reference_max_width + config.gap_size)
    } else {
        config.line_width
    }
}

fn ptx_auto_reference_max_width(settings: &PtxSettings) -> usize {
    settings
        .file_map
        .iter()
        .map(|content| {
            let line_ordinal = content.raw_lines.len().max(1) + 1;
            content.raw_filename.len() + 1 + line_ordinal.to_string().len()
        })
        .max()
        .unwrap_or(0)
}

fn ptx_reference_max_width_bytes(settings: &PtxSettings) -> usize {
    if settings.config.is_auto_ref {
        return ptx_auto_reference_max_width(settings);
    }
    if !settings.config.is_input_ref {
        return 0;
    }

    settings
        .words
        .iter()
        .map(|word_ref| {
            let content = &settings.file_map[word_ref.file_index];
            ptx_get_reference_bytes(&settings.config, word_ref, content).len()
        })
        .max()
        .unwrap_or(0)
}

fn ptx_display_field(s: &str) -> String {
    s.chars()
        .map(|c| if ptx_is_space_char(c) { ' ' } else { c })
        .collect()
}

fn context_base_start(
    config: &PtxConfig,
    line: &str,
    _chars_line: &[char],
    _context_reg: &Regex,
) -> usize {
    if config.is_input_ref {
        ptx_input_reference_content_start(line)
    } else {
        0
    }
}

fn context_bounds(
    config: &PtxConfig,
    line: &str,
    context_reg: &Regex,
    keyword_beg: usize,
    keyword_end: usize,
    base_start: usize,
) -> (usize, usize) {
    if !config.is_gnu_ext || config.context_regex == "\\w+" {
        return (base_start, line.len());
    }

    let mut left = base_start;
    for (_, end) in context_reg.find_iter(line) {
        if end <= keyword_beg {
            left = end;
        } else {
            break;
        }
    }
    left = left.max(base_start);

    let mut right = line.len();
    for (start, end) in context_reg.find_iter(line) {
        if start >= keyword_end {
            right = end;
            break;
        }
    }

    (left, right)
}

#[allow(clippy::too_many_arguments)]
fn ptx_format_dumb_line(
    config: &PtxConfig,
    word_ref: &WordRef,
    line: &str,
    chars_line: &[char],
    text: &str,
    chars_text: &[char],
    context_reg: &Regex,
    reference: &str,
    reference_max_width: usize,
    maximum_word_length: usize,
) -> String {
    let mut output = String::with_capacity(line.len() * 2);
    let (keyword, all_before, all_after, _) = ptx_context_slices(
        config,
        word_ref,
        text,
        chars_text,
        line,
        chars_line,
        context_reg,
    );
    let gap_size = config.gap_size;
    let mut effective_line_width = config.line_width;
    if (config.is_auto_ref || config.is_input_ref) && !config.is_right_ref {
        effective_line_width = effective_line_width.saturating_sub(reference_max_width + gap_size);
    }
    let (tail, before, after, head, before_padding_adjustment) =
        ptx_get_output_chunks_for_width_with_max(
            all_before,
            keyword,
            all_after,
            config,
            effective_line_width,
            maximum_word_length,
        );
    let keyafter = format!("{keyword}{after}");
    let half_line_width = effective_line_width / 2;

    let reference_len = str_cols(reference);
    if !config.is_right_ref {
        if config.is_auto_ref {
            output.push_str(reference);
            output.push(':');
            let pad = reference_max_width
                .saturating_add(gap_size)
                .saturating_sub(reference_len.saturating_add(1));
            output.push_str(&" ".repeat(pad));
        } else {
            output.push_str(reference);
            let pad = reference_max_width
                .saturating_add(gap_size)
                .saturating_sub(reference_len);
            output.push_str(&" ".repeat(pad));
        }
    }

    let before_len = str_cols(&before);
    let tail_len = str_cols(&tail);
    if !tail.is_empty() {
        output.push_str(&ptx_display_field(&tail));
        let pad = half_line_width
            .saturating_add(before_padding_adjustment)
            .saturating_sub(gap_size)
            .saturating_sub(before_len)
            .saturating_sub(tail_len);
        output.push_str(&" ".repeat(pad));
    } else {
        let whitespace_before_adjust =
            if config.is_gnu_ext && !before.is_empty() && before.chars().all(ptx_is_space_char) {
                1
            } else {
                0
            };
        let pad = half_line_width
            .saturating_add(before_padding_adjustment)
            .saturating_sub(gap_size)
            .saturating_sub(before_len)
            .saturating_add(whitespace_before_adjust);
        output.push_str(&" ".repeat(pad));
    }

    output.push_str(&ptx_display_field(&before));
    output.push_str(&" ".repeat(gap_size));
    output.push_str(&ptx_display_field(&keyafter));

    let keyafter_len = str_cols(&keyafter);
    let head_len = str_cols(&head);
    if !head.is_empty() {
        let pad = half_line_width
            .saturating_sub(keyafter_len)
            .saturating_sub(head_len);
        output.push_str(&" ".repeat(pad));
        output.push_str(&ptx_display_field(&head));
    } else if (config.is_auto_ref || config.is_input_ref) && config.is_right_ref {
        let pad = half_line_width.saturating_sub(keyafter_len);
        output.push_str(&" ".repeat(pad));
    }

    if (config.is_auto_ref || config.is_input_ref) && config.is_right_ref {
        output.push_str(&" ".repeat(gap_size));
        output.push_str(reference);
    }

    output
}

fn ptx_format_roff_field_bytes(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for &b in s {
        if ptx_is_space_byte(b) {
            out.push(b' ');
        } else if b == b'"' {
            out.push(b'"');
            out.push(b'"');
        } else {
            out.push(b);
        }
    }
    out
}

fn ptx_format_tex_field_bytes(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for &byte in s {
        match byte {
            byte if ptx_is_space_byte(byte) => out.push(b' '),
            b'\\' => out.extend_from_slice(b"\\backslash{}"),
            b'$' | b'%' | b'#' | b'&' | b'_' => {
                out.push(b'\\');
                out.push(byte);
            }
            b'{' | b'}' => {
                out.extend_from_slice(b"$\\");
                out.push(byte);
                out.push(b'$');
            }
            _ => out.push(byte),
        }
    }
    out
}

fn ptx_display_field_bytes(s: &[u8]) -> Vec<u8> {
    s.iter()
        .map(|&b| if ptx_is_space_byte(b) { b' ' } else { b })
        .collect()
}

fn ptx_context_slices_bytes<'a>(
    word_ref: &WordRef,
    content: &'a FileContent,
) -> (&'a [u8], &'a [u8], &'a [u8], usize) {
    let bytes = &content.raw_text;
    if word_ref.context_end > word_ref.context_start
        && word_ref.global_position_end <= bytes.len()
        && word_ref.context_end <= bytes.len()
    {
        let before_start = word_ref.context_start.min(word_ref.global_position);
        return (
            &bytes[word_ref.global_position..word_ref.global_position_end],
            &bytes[before_start..word_ref.global_position],
            &bytes[word_ref.global_position_end..word_ref.context_end],
            word_ref
                .context_start
                .saturating_sub(word_ref.global_position),
        );
    }

    let line = &content.raw_lines[word_ref.local_line_nr];
    (
        &line[word_ref.position..word_ref.position_end],
        &line[..word_ref.position],
        &line[word_ref.position_end..],
        0,
    )
}

fn ptx_format_dumb_line_bytes(
    config: &PtxConfig,
    word_ref: &WordRef,
    content: &FileContent,
    reference: &[u8],
    reference_max_width: usize,
    maximum_word_length: usize,
) -> Vec<u8> {
    let (keyword, all_before, all_after, left_context_after_keyword) =
        ptx_context_slices_bytes(word_ref, content);
    let gap_size = config.gap_size;
    let mut effective_line_width = config.line_width;
    if (config.is_auto_ref || config.is_input_ref) && !config.is_right_ref {
        effective_line_width = effective_line_width.saturating_sub(reference_max_width + gap_size);
    }
    let fields = ptx_define_output_fields_bytes_for_width(
        all_before,
        keyword,
        all_after,
        left_context_after_keyword,
        config,
        effective_line_width,
        maximum_word_length,
    );

    let mut output = Vec::new();
    if !config.is_right_ref {
        if config.is_auto_ref {
            output.extend_from_slice(reference);
            output.push(b':');
            let pad = reference_max_width
                .saturating_add(gap_size)
                .saturating_sub(reference.len().saturating_add(1));
            output.extend(std::iter::repeat_n(b' ', pad));
        } else {
            output.extend_from_slice(reference);
            let pad = reference_max_width
                .saturating_add(gap_size)
                .saturating_sub(reference.len());
            output.extend(std::iter::repeat_n(b' ', pad));
        }
    }

    let half_line_width = effective_line_width / 2;
    let trunc_len = config.trunc_bytes.len();

    if !fields.tail.is_empty() {
        output.extend_from_slice(&ptx_display_field_bytes(&fields.tail));
        if fields.tail_truncation {
            output.extend_from_slice(&config.trunc_bytes);
        }
        let pad = half_line_width
            .saturating_add(fields.before_padding_adjustment)
            .saturating_sub(gap_size)
            .saturating_sub(fields.before.len())
            .saturating_sub(if fields.before_truncation {
                trunc_len
            } else {
                0
            })
            .saturating_sub(fields.tail.len())
            .saturating_sub(if fields.tail_truncation { trunc_len } else { 0 });
        output.extend(std::iter::repeat_n(b' ', pad));
    } else {
        let whitespace_before_adjust = if config.is_gnu_ext
            && !fields.before.is_empty()
            && fields.before.iter().all(|&byte| ptx_is_space_byte(byte))
        {
            1
        } else {
            0
        };
        let pad = half_line_width
            .saturating_add(fields.before_padding_adjustment)
            .saturating_sub(gap_size)
            .saturating_sub(fields.before.len())
            .saturating_sub(if fields.before_truncation {
                trunc_len
            } else {
                0
            })
            .saturating_add(whitespace_before_adjust);
        output.extend(std::iter::repeat_n(b' ', pad));
    }

    if fields.before_truncation {
        output.extend_from_slice(&config.trunc_bytes);
    }
    output.extend_from_slice(&ptx_display_field_bytes(&fields.before));
    output.extend(std::iter::repeat_n(b' ', gap_size));
    output.extend_from_slice(&ptx_display_field_bytes(&fields.keyafter));
    if fields.keyafter_truncation {
        output.extend_from_slice(&config.trunc_bytes);
    }

    if !fields.head.is_empty() {
        let pad = half_line_width
            .saturating_sub(fields.keyafter.len())
            .saturating_sub(if fields.keyafter_truncation {
                trunc_len
            } else {
                0
            })
            .saturating_sub(fields.head.len())
            .saturating_sub(if fields.head_truncation { trunc_len } else { 0 });
        output.extend(std::iter::repeat_n(b' ', pad));
        if fields.head_truncation {
            output.extend_from_slice(&config.trunc_bytes);
        }
        output.extend_from_slice(&ptx_display_field_bytes(&fields.head));
    } else if (config.is_auto_ref || config.is_input_ref) && config.is_right_ref {
        let pad = half_line_width
            .saturating_sub(fields.keyafter.len())
            .saturating_sub(if fields.keyafter_truncation {
                trunc_len
            } else {
                0
            });
        output.extend(std::iter::repeat_n(b' ', pad));
    }

    if (config.is_auto_ref || config.is_input_ref) && config.is_right_ref {
        output.extend(std::iter::repeat_n(b' ', gap_size));
        output.extend_from_slice(reference);
    }

    output
}

fn ptx_format_roff_line_bytes(
    config: &PtxConfig,
    word_ref: &WordRef,
    content: &FileContent,
    reference: &[u8],
    line_width: usize,
    maximum_word_length: usize,
) -> Vec<u8> {
    let (keyword, all_before, all_after, left_context_after_keyword) =
        ptx_context_slices_bytes(word_ref, content);
    let fields = ptx_define_output_fields_bytes_for_width(
        all_before,
        keyword,
        all_after,
        left_context_after_keyword,
        config,
        line_width,
        maximum_word_length,
    );

    let mut output = Vec::new();
    output.push(b'.');
    output.extend_from_slice(&config.macro_bytes);
    output.extend_from_slice(b" \"");
    output.extend_from_slice(&ptx_format_roff_field_bytes(&fields.tail));
    if fields.tail_truncation {
        output.extend_from_slice(&config.trunc_bytes);
    }
    output.extend_from_slice(b"\" \"");
    if fields.before_truncation {
        output.extend_from_slice(&config.trunc_bytes);
    }
    output.extend_from_slice(&ptx_format_roff_field_bytes(&fields.before));
    output.extend_from_slice(b"\" \"");
    output.extend_from_slice(&ptx_format_roff_field_bytes(&fields.keyafter));
    if fields.keyafter_truncation {
        output.extend_from_slice(&config.trunc_bytes);
    }
    output.extend_from_slice(b"\" \"");
    if fields.head_truncation {
        output.extend_from_slice(&config.trunc_bytes);
    }
    output.extend_from_slice(&ptx_format_roff_field_bytes(&fields.head));
    output.push(b'"');
    if config.is_auto_ref || config.is_input_ref {
        output.extend_from_slice(b" \"");
        output.extend_from_slice(&ptx_format_roff_field_bytes(reference));
        output.push(b'"');
    }
    output
}

fn ptx_format_tex_line_bytes(
    config: &PtxConfig,
    word_ref: &WordRef,
    content: &FileContent,
    reference: &[u8],
    line_width: usize,
    maximum_word_length: usize,
) -> Vec<u8> {
    let (keyword, all_before, all_after, left_context_after_keyword) =
        ptx_context_slices_bytes(word_ref, content);
    let fields = ptx_define_output_fields_bytes_for_width(
        all_before,
        keyword,
        all_after,
        left_context_after_keyword,
        config,
        line_width,
        maximum_word_length,
    );
    let (key, after) = if fields.keyafter.is_empty() && config.word_regex.is_some() {
        (&keyword[..keyword.len().min(1)], &[][..])
    } else {
        let key_end = ptx_skip_something_bytes(&fields.keyafter, 0, fields.keyafter.len(), config);
        (&fields.keyafter[..key_end], &fields.keyafter[key_end..])
    };

    let mut output = Vec::new();
    output.push(b'\\');
    output.extend_from_slice(&config.macro_bytes);
    output.extend_from_slice(b" {");
    output.extend_from_slice(&ptx_format_tex_field_bytes(&fields.tail));
    output.extend_from_slice(b"}{");
    output.extend_from_slice(&ptx_format_tex_field_bytes(&fields.before));
    output.extend_from_slice(b"}{");
    output.extend_from_slice(&ptx_format_tex_field_bytes(key));
    output.extend_from_slice(b"}{");
    output.extend_from_slice(&ptx_format_tex_field_bytes(after));
    output.extend_from_slice(b"}{");
    output.extend_from_slice(&ptx_format_tex_field_bytes(&fields.head));
    output.push(b'}');
    if config.is_auto_ref || config.is_input_ref {
        output.push(b'{');
        output.extend_from_slice(&ptx_format_tex_field_bytes(reference));
        output.push(b'}');
    }
    output
}

/// 执行 PTX 命令的核心逻辑
fn ptx_exec(settings: &mut PtxSettings) -> CTResult<()> {
    let mut writer: BufWriter<Box<dyn Write>> =
        BufWriter::new(if let Some(file) = settings.output_file.take() {
            Box::new(file)
        } else {
            Box::new(stdout())
        });

    let reference_max_width = ptx_reference_max_width_bytes(settings);
    let effective_line_width = ptx_effective_line_width(&settings.config, reference_max_width);
    let maximum_word_length =
        ptx_global_maximum_word_length_bytes(&settings.file_map, &settings.config);

    for word_ref in &settings.words {
        // 通过索引直接获取文件内容
        let content = &settings.file_map[word_ref.file_index];

        let reference = ptx_get_reference_bytes(&settings.config, word_ref, content);

        let output_line = match settings.config.format {
            OutFormat::Tex => ptx_format_tex_line_bytes(
                &settings.config,
                word_ref,
                content,
                &reference,
                effective_line_width,
                maximum_word_length,
            ),
            OutFormat::Roff => ptx_format_roff_line_bytes(
                &settings.config,
                word_ref,
                content,
                &reference,
                effective_line_width,
                maximum_word_length,
            ),
            OutFormat::Dumb => ptx_format_dumb_line_bytes(
                &settings.config,
                word_ref,
                content,
                &reference,
                reference_max_width,
                maximum_word_length,
            ),
        };

        writer
            .write_all(&output_line)
            .map_err_context(ptx_write_error_context)?;
        writer
            .write_all(b"\n")
            .map_err_context(ptx_write_error_context)?;
    }
    writer.flush().map_err_context(ptx_write_error_context)
}

fn ptx_reference_max_width(settings: &PtxSettings, context_reg: &Regex) -> usize {
    if settings.config.is_auto_ref {
        return ptx_auto_reference_max_width(settings);
    }
    let mut reference_max_width = 0usize;
    if settings.config.is_input_ref {
        for word_ref in &settings.words {
            let content = &settings.file_map[word_ref.file_index];
            let reference = ptx_get_reference(
                &settings.config,
                word_ref,
                &content.filename,
                &content.lines[word_ref.local_line_nr],
                context_reg,
            );
            reference_max_width = reference_max_width.max(str_cols(&reference));
        }
    }
    reference_max_width
}

fn ptx_render_row(
    settings: &PtxSettings,
    word_ref: &WordRef,
    context_reg: &Regex,
    reference_max_width: usize,
    maximum_word_length: usize,
) -> PtxSemanticRow {
    let content = &settings.file_map[word_ref.file_index];
    let line = &content.lines[word_ref.local_line_nr];
    let chars_line = &content.chars_lines[word_ref.local_line_nr];
    let reference = ptx_get_reference(
        &settings.config,
        word_ref,
        &content.filename,
        line,
        context_reg,
    );
    let (keyword, all_before, all_after, _) = ptx_context_slices(
        &settings.config,
        word_ref,
        &content.text,
        &content.chars_text,
        line,
        chars_line,
        context_reg,
    );
    let effective_line_width = ptx_effective_line_width(&settings.config, reference_max_width);
    let (tail, before, after, head, _) = ptx_get_output_chunks_for_width_with_max(
        all_before,
        keyword,
        all_after,
        &settings.config,
        effective_line_width,
        maximum_word_length,
    );
    let rendered_text = match settings.config.format {
        OutFormat::Tex => ptx_format_tex_line(
            &settings.config,
            word_ref,
            line,
            chars_line,
            &content.text,
            &content.chars_text,
            context_reg,
            &reference,
            effective_line_width,
            maximum_word_length,
        ),
        OutFormat::Roff => ptx_format_roff_line(
            &settings.config,
            word_ref,
            line,
            chars_line,
            &content.text,
            &content.chars_text,
            context_reg,
            &reference,
            effective_line_width,
            maximum_word_length,
        ),
        OutFormat::Dumb => ptx_format_dumb_line(
            &settings.config,
            word_ref,
            line,
            chars_line,
            &content.text,
            &content.chars_text,
            context_reg,
            &reference,
            reference_max_width,
            maximum_word_length,
        ),
    };

    PtxSemanticRow {
        row_index: 0,
        keyword: keyword.to_string(),
        before,
        after,
        head,
        tail,
        reference,
        file: content.filename.clone(),
        line_index: word_ref.local_line_nr + 1,
        global_line_index: word_ref.global_line_nr + 1,
        rendered_text,
        format: ptx_format_name(&settings.config.format).to_string(),
    }
}

fn ptx_collect_semantic_rows(settings: &PtxSettings) -> Vec<PtxSemanticRow> {
    let context_reg = compile_regex_case_lossy(
        &settings.config.context_regex,
        settings.config.is_ignore_case,
    );
    let reference_max_width = ptx_reference_max_width(settings, &context_reg);
    let maximum_word_length = ptx_global_maximum_word_length(&settings.file_map, &settings.config);
    let mut rows: Vec<PtxSemanticRow> = settings
        .words
        .iter()
        .map(|word_ref| {
            ptx_render_row(
                settings,
                word_ref,
                &context_reg,
                reference_max_width,
                maximum_word_length,
            )
        })
        .collect();

    for (index, row) in rows.iter_mut().enumerate() {
        row.row_index = index + 1;
    }

    rows
}

fn ptx_exec_to_writer(settings: &PtxSettings, writer: &mut impl Write) -> CTResult<()> {
    let reference_max_width = ptx_reference_max_width_bytes(settings);
    let effective_line_width = ptx_effective_line_width(&settings.config, reference_max_width);
    let maximum_word_length =
        ptx_global_maximum_word_length_bytes(&settings.file_map, &settings.config);

    for word_ref in &settings.words {
        let file_map_value = &settings.file_map[word_ref.file_index];

        let reference = ptx_get_reference_bytes(&settings.config, word_ref, file_map_value);

        let output_line = match settings.config.format {
            OutFormat::Tex => ptx_format_tex_line_bytes(
                &settings.config,
                word_ref,
                file_map_value,
                &reference,
                effective_line_width,
                maximum_word_length,
            ),
            OutFormat::Roff => ptx_format_roff_line_bytes(
                &settings.config,
                word_ref,
                file_map_value,
                &reference,
                effective_line_width,
                maximum_word_length,
            ),
            OutFormat::Dumb => ptx_format_dumb_line_bytes(
                &settings.config,
                word_ref,
                file_map_value,
                &reference,
                reference_max_width,
                maximum_word_length,
            ),
        };

        writer
            .write_all(&output_line)
            .map_err_context(ptx_write_error_context)?;
        writer
            .write_all(b"\n")
            .map_err_context(ptx_write_error_context)?;
    }
    writer.flush().map_err_context(ptx_write_error_context)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PtxCoreOutput {
    pub bytes: Vec<u8>,
}

pub fn ptx_main(args: impl ctcore::Args) -> CTResult<()> {
    let mut out = stdout().lock();
    ptx_main_with_writer(args, &mut out)
}

pub fn ptx_core_output(args: impl ctcore::Args) -> CTResult<PtxCoreOutput> {
    let mut out = Vec::new();
    ptx_main_with_writer(args, &mut out)?;
    Ok(PtxCoreOutput { bytes: out })
}

fn ptx_render_help_text() -> String {
    let mut command = ct_app();
    command.render_help().to_string()
}

fn ptx_render_version_text() -> String {
    ct_app().render_version()
}

pub fn ptx_main_with_writer<W: Write>(args: impl ctcore::Args, out: &mut W) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let args = ptx_args(args)?;
    let matches = match ct_app().try_get_matches_from(args) {
        Ok(matches) => matches,
        Err(err) => {
            return match err.kind() {
                ErrorKind::DisplayHelp => {
                    out.write_all(ptx_render_help_text().as_bytes())
                        .map_err_context(ptx_write_error_context)?;
                    out.flush().map_err_context(ptx_write_error_context)?;
                    Ok(())
                }
                ErrorKind::DisplayVersion => {
                    out.write_all(ptx_render_version_text().as_bytes())
                        .map_err_context(ptx_write_error_context)?;
                    out.flush().map_err_context(ptx_write_error_context)?;
                    Ok(())
                }
                _ => Err(err.into()),
            };
        }
    };
    let mut settings = PtxSettings::from_matches(matches)?;
    if settings.output_file.is_some() {
        ptx_exec(&mut settings)?;
        return Ok(());
    }

    ptx_exec_to_writer(&settings, out)
}

pub fn ptx_core_output_from_args(args: Vec<OsString>) -> CTResult<PtxCoreOutput> {
    ptx_core_output(args.into_iter())
}

pub fn ptx_native_semantic(args: impl ctcore::Args) -> CTResult<PtxSemantic> {
    ptx_native_semantic_with_stdin(args, None)
}

pub fn ptx_native_semantic_with_stdin(
    args: impl ctcore::Args,
    stdin_bytes: Option<Vec<u8>>,
) -> CTResult<PtxSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);

    let argv = ptx_args(args)?;
    let direct = run_ptx_direct_process(&argv, stdin_bytes.as_deref())?;
    let classic_text = String::from_utf8_lossy(&direct.stdout).into_owned();
    let stderr_text = String::from_utf8_lossy(&direct.stderr).into_owned();

    let rows = if direct.exit_code == 0 {
        ptx_collect_semantic_rows_from_argv(argv, stdin_bytes)?
    } else {
        Vec::new()
    };

    Ok(PtxSemantic {
        rows,
        classic_text,
        stderr_text,
        exit_code: direct.exit_code,
    })
}

fn ptx_args(args: impl ctcore::Args) -> CTResult<Vec<OsString>> {
    let mut args: Vec<OsString> = args.collect();
    validate_ptx_width_occurrences(&args)?;
    if std::env::var_os("POSIXLY_CORRECT").is_none() {
        return Ok(args);
    }

    let mut index = 1usize;
    while index < args.len() {
        let bytes = args[index].as_encoded_bytes();
        if bytes == b"--" {
            break;
        }
        if bytes.is_empty() || bytes == b"-" || bytes[0] != b'-' {
            args.insert(index, OsString::from("--"));
            break;
        }

        let takes_next_value = if let Some(long) = bytes.strip_prefix(b"--") {
            !long.contains(&b'=') && ptx_long_option_takes_value(long)
        } else {
            ptx_short_option_takes_next_value(&bytes[1..])
        };
        index += 1 + usize::from(takes_next_value && index + 1 < args.len());
    }
    Ok(args)
}

fn validate_ptx_width_occurrences(args: &[OsString]) -> CTResult<()> {
    let posix = std::env::var_os("POSIXLY_CORRECT").is_some();
    let mut index = 1usize;
    while index < args.len() {
        let bytes = args[index].as_encoded_bytes();
        if bytes == b"--" {
            break;
        }
        if bytes.is_empty() || bytes == b"-" || bytes[0] != b'-' {
            if posix {
                break;
            }
            index += 1;
            continue;
        }

        if let Some(long) = bytes.strip_prefix(b"--") {
            let (name, attached) = long
                .iter()
                .position(|byte| *byte == b'=')
                .map_or((long, None), |separator| {
                    (&long[..separator], Some(&long[separator + 1..]))
                });
            let Some(option) = ptx_canonical_long_option(name) else {
                break;
            };
            // GNU exits as soon as getopt encounters either terminal option,
            // so later numeric options must not be prevalidated.
            if matches!(option, b"help" | b"version") {
                return Ok(());
            }
            if matches!(option, b"width" | b"gap-size") {
                let value = match attached {
                    Some(value) => value,
                    None if index + 1 < args.len() => {
                        index += 1;
                        args[index].as_encoded_bytes()
                    }
                    None => break,
                };
                let description = if option == b"width" {
                    "line width"
                } else {
                    "gap width"
                };
                match std::str::from_utf8(value) {
                    Ok(value) => {
                        parse_positive_base0(value, description)?;
                    }
                    Err(_) => return Err(ptx_invalid_numeric_arg_error(value, description)),
                }
            } else if attached.is_none()
                && matches!(
                    option,
                    b"break-file"
                        | b"flag-truncation"
                        | b"ignore-file"
                        | b"macro-name"
                        | b"only-file"
                        | b"format"
                        | b"sentence-regexp"
                        | b"word-regexp"
                )
                && index + 1 < args.len()
            {
                index += 1;
            }
            index += 1;
            continue;
        }

        let options = &bytes[1..];
        let mut option_index = 0usize;
        while option_index < options.len() {
            let option = options[option_index];
            if matches!(option, b'w' | b'g') {
                let value = if option_index + 1 < options.len() {
                    &options[option_index + 1..]
                } else if index + 1 < args.len() {
                    index += 1;
                    args[index].as_encoded_bytes()
                } else {
                    break;
                };
                let description = if option == b'w' {
                    "line width"
                } else {
                    "gap width"
                };
                match std::str::from_utf8(value) {
                    Ok(value) => {
                        parse_positive_base0(value, description)?;
                    }
                    Err(_) => return Err(ptx_invalid_numeric_arg_error(value, description)),
                }
                break;
            }
            if matches!(option, b'F' | b'M' | b'S' | b'W' | b'b' | b'i' | b'o') {
                if option_index + 1 == options.len() && index + 1 < args.len() {
                    index += 1;
                }
                break;
            }
            if !matches!(
                option,
                b'A' | b'G' | b'O' | b'R' | b'T' | b'f' | b'r' | b't'
            ) {
                return Ok(());
            }
            option_index += 1;
        }
        index += 1;
    }
    Ok(())
}

fn ptx_canonical_long_option(option: &[u8]) -> Option<&'static [u8]> {
    const LONG_OPTIONS: [&[u8]; 18] = [
        b"auto-reference",
        b"break-file",
        b"flag-truncation",
        b"ignore-case",
        b"gap-size",
        b"ignore-file",
        b"macro-name",
        b"only-file",
        b"references",
        b"right-side-refs",
        b"format",
        b"sentence-regexp",
        b"traditional",
        b"typeset-mode",
        b"width",
        b"word-regexp",
        b"help",
        b"version",
    ];
    if let Some(exact) = LONG_OPTIONS
        .into_iter()
        .find(|candidate| *candidate == option)
    {
        return Some(exact);
    }
    let mut matches = LONG_OPTIONS
        .into_iter()
        .filter(|candidate| candidate.starts_with(option));
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

fn ptx_long_option_takes_value(option: &[u8]) -> bool {
    ptx_canonical_long_option(option).is_some_and(|option| {
        matches!(
            option,
            b"break-file"
                | b"flag-truncation"
                | b"gap-size"
                | b"ignore-file"
                | b"macro-name"
                | b"only-file"
                | b"format"
                | b"sentence-regexp"
                | b"width"
                | b"word-regexp"
        )
    })
}

fn ptx_short_option_takes_next_value(options: &[u8]) -> bool {
    for (index, option) in options.iter().copied().enumerate() {
        if matches!(
            option,
            b'F' | b'M' | b'S' | b'W' | b'b' | b'g' | b'i' | b'o' | b'w'
        ) {
            return index + 1 == options.len();
        }
    }
    false
}

fn ptx_collect_semantic_rows_from_argv(
    argv: Vec<OsString>,
    stdin_bytes: Option<Vec<u8>>,
) -> CTResult<Vec<PtxSemanticRow>> {
    let matches = match ct_app().try_get_matches_from(argv) {
        Ok(matches) => matches,
        Err(_) => return Ok(Vec::new()),
    };

    if let Some(bytes) = stdin_bytes {
        return ctcore::ct_io::with_injected_stdin(bytes, move || {
            ptx_collect_semantic_rows_from_matches(matches)
        });
    }

    ptx_collect_semantic_rows_from_matches(matches)
}

fn ptx_collect_semantic_rows_from_matches(
    matches: clap::ArgMatches,
) -> CTResult<Vec<PtxSemanticRow>> {
    let settings = match PtxSettings::from_matches(matches) {
        Ok(settings) => settings,
        Err(_) => return Ok(Vec::new()),
    };

    Ok(ptx_semantic_rows_for_settings(&settings))
}

fn ptx_semantic_from_clap_error(error: clap::Error) -> PtxSemantic {
    match error.kind() {
        ErrorKind::DisplayHelp => PtxSemantic {
            rows: Vec::new(),
            classic_text: ptx_render_help_text(),
            stderr_text: String::new(),
            exit_code: 0,
        },
        ErrorKind::DisplayVersion => PtxSemantic {
            rows: Vec::new(),
            classic_text: ptx_render_version_text(),
            stderr_text: String::new(),
            exit_code: 0,
        },
        _ => PtxSemantic {
            rows: Vec::new(),
            classic_text: String::new(),
            stderr_text: error.to_string(),
            exit_code: 1,
        },
    }
}

fn ptx_render_error_text(err: &dyn CTError) -> String {
    let mut stderr = format!("ptx: {err}\n");
    if err.usage() {
        stderr.push_str("Try 'ptx --help' for more information.\n");
    }
    stderr
}

fn ptx_push_octal_escape(output: &mut Vec<u8>, byte: u8) {
    output.push(b'\\');
    output.push(b'0' + ((byte >> 6) & 7));
    output.push(b'0' + ((byte >> 3) & 7));
    output.push(b'0' + (byte & 7));
}

fn ptx_quote_pattern(
    pattern: &[u8],
    byte_ctype: &LocaleByteCtype,
    single_byte_locale: bool,
) -> Vec<u8> {
    const UTF8_LEFT_QUOTE: &[u8] = "‘".as_bytes();
    const UTF8_RIGHT_QUOTE: &[u8] = "’".as_bytes();

    let (left_quote, right_quote): (&[u8], &[u8]) =
        if ptx_locale_codeset().eq_ignore_ascii_case("UTF-8") {
            (UTF8_LEFT_QUOTE, UTF8_RIGHT_QUOTE)
        } else {
            (b"'", b"'")
        };
    let mut quoted = Vec::with_capacity(pattern.len() + left_quote.len() + right_quote.len());
    quoted.extend_from_slice(left_quote);
    let printable = ptx_printable_byte_mask(pattern, byte_ctype, single_byte_locale);
    let mut index = 0usize;
    while index < pattern.len() {
        if pattern[index..].starts_with(right_quote) {
            quoted.push(b'\\');
            quoted.extend_from_slice(right_quote);
            index += right_quote.len();
            continue;
        }
        let byte = pattern[index];
        let escape = match byte {
            b'\x07' => Some(b'a'),
            b'\x08' => Some(b'b'),
            b'\t' => Some(b't'),
            b'\n' => Some(b'n'),
            b'\x0b' => Some(b'v'),
            b'\x0c' => Some(b'f'),
            b'\r' => Some(b'r'),
            b'\\' => Some(b'\\'),
            _ => None,
        };
        if let Some(escaped) = escape {
            quoted.extend_from_slice(&[b'\\', escaped]);
            index += 1;
            continue;
        }
        if printable[index] {
            quoted.push(byte);
        } else {
            ptx_push_octal_escape(&mut quoted, byte);
        }
        index += 1;
    }
    quoted.extend_from_slice(right_quote);
    quoted
}

fn ptx_invalid_numeric_arg_error(value: &[u8], description: &str) -> Box<dyn CTError> {
    let byte_ctype = LocaleByteCtype::from_environment();
    let quoted = ptx_quote_pattern(value, &byte_ctype, ptx_is_single_byte_locale());
    let mut stderr = std::io::stderr().lock();
    let _ = stderr.write_all(ctcore::ct_util_name().as_bytes());
    let _ = stderr.write_all(b": invalid ");
    let _ = stderr.write_all(description.as_bytes());
    let _ = stderr.write_all(b": ");
    let _ = stderr.write_all(&quoted);
    let _ = stderr.write_all(b"\n");
    CtSimpleError::new(1, "")
}

fn ptx_printable_byte_mask(
    bytes: &[u8],
    byte_ctype: &LocaleByteCtype,
    single_byte_locale: bool,
) -> Vec<bool> {
    let mut printable = vec![false; bytes.len()];
    let locale_encoding = LocaleRegexEncoding::from_environment();
    if locale_encoding.is_non_utf8_multibyte() {
        for (index, &byte) in bytes.iter().enumerate() {
            if byte.is_ascii() {
                printable[index] = byte_ctype.is_print(byte);
            }
        }
        locale_encoding.mark_valid_multibyte_sequences(bytes, &mut printable);
        return printable;
    }
    let mut index = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii() || single_byte_locale {
            printable[index] = byte_ctype.is_print(byte);
            index += 1;
            continue;
        }
        let valid_len = match std::str::from_utf8(&bytes[index..]) {
            Ok(valid) => valid.chars().next().map_or(0, char::len_utf8),
            Err(error) if error.valid_up_to() > 0 => {
                std::str::from_utf8(&bytes[index..index + error.valid_up_to()])
                    .expect("validated UTF-8 prefix")
                    .chars()
                    .next()
                    .map_or(0, char::len_utf8)
            }
            Err(_) => 0,
        };
        if valid_len == 0 {
            index += 1;
        } else {
            printable[index..index + valid_len].fill(true);
            index += valid_len;
        }
    }
    printable
}

fn ptx_push_single_quoted(output: &mut Vec<u8>, bytes: &[u8]) {
    output.push(b'\'');
    for &byte in bytes {
        if byte == b'\'' {
            output.extend_from_slice(b"'\\''");
        } else {
            output.push(byte);
        }
    }
    output.push(b'\'');
}

fn ptx_push_dollar_quoted(output: &mut Vec<u8>, bytes: &[u8]) {
    output.extend_from_slice(b"$'");
    for &byte in bytes {
        match byte {
            b'\x07' => output.extend_from_slice(b"\\a"),
            b'\x08' => output.extend_from_slice(b"\\b"),
            b'\t' => output.extend_from_slice(b"\\t"),
            b'\n' => output.extend_from_slice(b"\\n"),
            b'\x0b' => output.extend_from_slice(b"\\v"),
            b'\x0c' => output.extend_from_slice(b"\\f"),
            b'\r' => output.extend_from_slice(b"\\r"),
            b'\\' | b'\'' => output.extend_from_slice(&[b'\\', byte]),
            _ => ptx_push_octal_escape(output, byte),
        }
    }
    output.push(b'\'');
}

fn ptx_quote_file_name(path: &OsStr) -> Vec<u8> {
    const SHELL_SPECIAL: &[u8] = b"|&;<>()$`\\\"'*?[]=^{} ";
    let bytes = path.as_bytes();
    let byte_ctype = LocaleByteCtype::from_environment();
    let printable = ptx_printable_byte_mask(bytes, &byte_ctype, ptx_is_single_byte_locale());
    if printable.iter().all(|&value| value) {
        let needs_quote = bytes.is_empty()
            || bytes
                .first()
                .is_some_and(|byte| matches!(byte, b'~' | b'#' | b'!'))
            || bytes.iter().any(|byte| SHELL_SPECIAL.contains(byte));
        if !needs_quote {
            return bytes.to_vec();
        }
        if bytes.contains(&b'\'')
            && !bytes
                .iter()
                .any(|byte| matches!(byte, b'"' | b'`' | b'$' | b'\\'))
        {
            let mut quoted = Vec::with_capacity(bytes.len() + 2);
            quoted.push(b'"');
            quoted.extend_from_slice(bytes);
            quoted.push(b'"');
            return quoted;
        }
        let mut quoted = Vec::with_capacity(bytes.len() + 2);
        ptx_push_single_quoted(&mut quoted, bytes);
        return quoted;
    }

    let mut quoted = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        let is_printable = printable[index];
        let start = index;
        while index < bytes.len() && printable[index] == is_printable {
            index += 1;
        }
        if is_printable {
            ptx_push_single_quoted(&mut quoted, &bytes[start..index]);
        } else {
            ptx_push_dollar_quoted(&mut quoted, &bytes[start..index]);
        }
    }
    quoted
}

fn ptx_file_io_error(path: &OsStr, error: std::io::Error) -> Box<dyn CTError> {
    let quoted = ptx_quote_file_name(path);
    let mut stderr = std::io::stderr().lock();
    let _ = stderr.write_all(ctcore::ct_util_name().as_bytes());
    let _ = stderr.write_all(b": ");
    let _ = stderr.write_all(&quoted);
    let _ = stderr.write_all(b": ");
    let _ = stderr.write_all(strip_errno(&error).as_bytes());
    let _ = stderr.write_all(b"\n");
    CtSimpleError::new(1, "")
}

fn ptx_invalid_regex_error(pattern: &[u8]) -> Box<dyn CTError> {
    ptx_regex_error(b"Invalid regular expression", pattern)
}

fn ptx_invalid_range_end_error(pattern: &[u8]) -> Box<dyn CTError> {
    ptx_regex_error(b"Invalid range end", pattern)
}

fn ptx_regex_error(message: &[u8], pattern: &[u8]) -> Box<dyn CTError> {
    let byte_ctype = LocaleByteCtype::from_environment();
    let quoted = ptx_quote_pattern(pattern, &byte_ctype, ptx_is_single_byte_locale());
    let mut stderr = std::io::stderr().lock();
    let _ = stderr.write_all(ctcore::ct_util_name().as_bytes());
    let _ = stderr.write_all(b": ");
    let _ = stderr.write_all(message);
    let _ = stderr.write_all(b" (for regexp ");
    let _ = stderr.write_all(&quoted);
    let _ = stderr.write_all(b")\n");
    CtSimpleError::new(1, "")
}

fn validate_ptx_locale_regexp(pattern: &[u8], config: &PtxConfig) -> CTResult<()> {
    if !ptx_pattern_needs_locale_class_matching(pattern) {
        return Ok(());
    }
    let Some(locale_collation) = config.locale_collation.as_deref() else {
        return Ok(());
    };
    if let Some(message) = locale_collation.compile_error(
        pattern,
        config.is_ignore_case.then_some(&config.byte_ctype.upper),
    ) {
        return Err(ptx_regex_error(&message, pattern));
    }
    Ok(())
}

fn ptx_extra_operand_error(operand: &OsStr) -> Box<dyn CTError> {
    let byte_ctype = LocaleByteCtype::from_environment();
    let quoted = ptx_quote_pattern(operand.as_bytes(), &byte_ctype, ptx_is_single_byte_locale());
    let mut stderr = std::io::stderr().lock();
    let _ = stderr.write_all(ctcore::ct_util_name().as_bytes());
    let _ = stderr.write_all(b": extra operand ");
    let _ = stderr.write_all(&quoted);
    let _ = stderr.write_all(b"\n");
    CTsageError::new(1, "")
}

fn ptx_zero_length_regex_error(
    pattern: &[u8],
    byte_ctype: &LocaleByteCtype,
    single_byte_locale: bool,
) -> Box<dyn CTError> {
    let quoted = ptx_quote_pattern(pattern, byte_ctype, single_byte_locale);
    let mut stderr = std::io::stderr().lock();
    let _ = stderr.write_all(ctcore::ct_util_name().as_bytes());
    let _ = stderr.write_all(b": error: regular expression has a match of length zero: ");
    let _ = stderr.write_all(&quoted);
    let _ = stderr.write_all(b"\n");
    CtSimpleError::new(1, "")
}

fn ptx_semantic_rows_for_settings(settings: &PtxSettings) -> Vec<PtxSemanticRow> {
    ptx_collect_semantic_rows(settings)
}

pub fn ptx_native_semantic_rows_only(args: impl ctcore::Args) -> CTResult<PtxSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);

    let argv: Vec<OsString> = args.collect();
    let matches = match ct_app().try_get_matches_from(argv) {
        Ok(matches) => matches,
        Err(error) => return Ok(ptx_semantic_from_clap_error(error)),
    };

    let settings = match PtxSettings::from_matches(matches) {
        Ok(settings) => settings,
        Err(error) => {
            return Ok(PtxSemantic {
                rows: Vec::new(),
                classic_text: String::new(),
                stderr_text: ptx_render_error_text(error.as_ref()),
                exit_code: error.code(),
            });
        }
    };

    Ok(PtxSemantic {
        rows: ptx_semantic_rows_for_settings(&settings),
        classic_text: String::new(),
        stderr_text: String::new(),
        exit_code: 0,
    })
}

mod ptx_options {
    pub static PTX_FILE: &str = "file";
    pub static PTX_AUTO_REFERENCE: &str = "auto-reference";
    pub static PTX_TRADITIONAL: &str = "traditional";
    pub static PTX_FLAG_TRUNCATION: &str = "flag-truncation";
    pub static PTX_MACRO_NAME: &str = "macro-name";
    pub static PTX_FORMAT: &str = "format";
    pub static PTX_FORMAT_ROFF: &str = "format-roff";
    pub static PTX_RIGHT_SIDE_REFS: &str = "right-side-refs";
    pub static PTX_SENTENCE_REGEXP: &str = "sentence-regexp";
    pub static PTX_FORMAT_TEX: &str = "format-tex";
    pub static PTX_WORD_REGEXP: &str = "word-regexp";
    pub static PTX_BREAK_FILE: &str = "break-file";
    pub static PTX_IGNORE_CASE: &str = "ignore-case";
    pub static PTX_GAP_SIZE: &str = "gap-size";
    pub static PTX_IGNORE_FILE: &str = "ignore-file";
    pub static PTX_ONLY_FILE: &str = "only-file";
    pub static PTX_REFERENCES: &str = "references";
    pub static PTX_TYPESET_MODE: &str = "typeset-mode";
    pub static PTX_WIDTH: &str = "width";
}

/// PTX 命令的运行配置
#[derive(Debug, Default)]
struct PtxSettings {
    /// 基础配置选项
    config: PtxConfig,
    /// 文件内容映射
    file_map: FileMap,
    /// 单词引用集合
    words: BTreeSet<WordRef>,
    /// 传统模式解析操作数时已经创建的输出文件。
    output_file: Option<File>,
}

impl PtxSettings {
    fn from_matches(matches: clap::ArgMatches) -> CTResult<Self> {
        // 获取输入文件列表
        let input_files: Vec<OsString> = match &matches.get_many::<OsString>(ptx_options::PTX_FILE)
        {
            Some(v) => v.clone().cloned().collect(),
            None => vec![OsString::from("-")],
        };

        // 获取配置
        let mut config = get_config(&matches)?;
        let output_file = if !config.is_gnu_ext && input_files.len() >= 2 {
            let output_filename = &input_files[1];
            Some(
                File::create(output_filename)
                    .map_err(|error| ptx_file_io_error(output_filename.as_os_str(), error))?,
            )
        } else {
            None
        };
        if !config.is_gnu_ext && input_files.len() > 2 {
            return Err(ptx_extra_operand_error(&input_files[2]));
        }
        if matches.contains_id(ptx_options::PTX_SENTENCE_REGEXP)
            && config.context_regex != NEVER_MATCH_REGEX
        {
            validate_ptx_locale_regexp(
                config
                    .context_pattern_bytes
                    .as_deref()
                    .expect("user context regexp has original bytes"),
                &config,
            )?;
            if config
                .context_pattern_bytes
                .as_deref()
                .is_some_and(ptx_pattern_has_equivalence_range_endpoint)
            {
                return Err(ptx_invalid_range_end_error(
                    config
                        .context_pattern_bytes
                        .as_deref()
                        .expect("checked context regexp"),
                ));
            }
            if config
                .context_pattern_bytes
                .as_deref()
                .is_some_and(ptx_has_chained_character_class_range)
            {
                return Err(ptx_invalid_regex_error(
                    config
                        .context_pattern_bytes
                        .as_deref()
                        .expect("checked context regexp"),
                ));
            }
            if let Some(pattern) = &config.context_byte_pattern {
                config.context_byte_regex = Some(compile_user_byte_regex(
                    pattern,
                    config.is_ignore_case,
                    &config.byte_ctype,
                    config.locale_regex_encoding,
                    config.locale_validator.clone(),
                    config.single_byte_locale,
                )?);
            } else {
                compile_user_regex(&config.context_regex, config.is_ignore_case)?;
            }
        }
        validate_ptx_word_regexp(&matches, &config)?;

        // 创建单词过滤器
        let mut word_filter = WordFilter::new(&matches, &config)?;
        config.word_break_bytes = word_filter.break_set.clone();
        if word_filter.uses_custom_regex {
            config.word_regex = Some(compile_user_regex(
                &word_filter.word_regex,
                config.is_ignore_case,
            )?);
            if let Some(pattern) = &word_filter.word_byte_pattern {
                config.word_byte_regex = Some(compile_user_byte_regex(
                    pattern,
                    config.is_ignore_case,
                    &config.byte_ctype,
                    config.locale_regex_encoding,
                    config.locale_validator.clone(),
                    config.single_byte_locale,
                )?);
                config.force_byte_mode = true;
            }
        }

        // 读取输入文件
        let file_map = ptx_read_input(&input_files, &config)?;
        ptx_recompile_locale_range_regexps(&mut config, &mut word_filter, &file_map)?;
        // 创建单词集合
        let word_set = ptx_create_word_set(&config, &word_filter, &file_map);

        // 创建设置
        let settings = Self {
            config,
            file_map,
            words: word_set,
            output_file,
        };

        Ok(settings)
    }
}

fn validate_ptx_word_regexp(matches: &clap::ArgMatches, config: &PtxConfig) -> CTResult<()> {
    let Some(value) = matches.get_one::<OsString>(ptx_options::PTX_WORD_REGEXP) else {
        return Ok(());
    };
    let bytes = ptx_unescape_bytes(value.as_os_str().as_bytes());
    if bytes.is_empty() {
        return Ok(());
    }
    validate_ptx_locale_regexp(&bytes, config)?;
    if ptx_pattern_has_equivalence_range_endpoint(&bytes) {
        return Err(ptx_invalid_range_end_error(&bytes));
    }
    if ptx_has_chained_character_class_range(&bytes) {
        return Err(ptx_invalid_regex_error(&bytes));
    }
    if config.is_ignore_case
        || config.single_byte_locale
        || config.locale_regex_encoding.is_non_utf8_multibyte()
        || std::str::from_utf8(&bytes).is_err()
    {
        let pattern = gnu_emacs_regex_to_onig_bytes(&bytes, config);
        compile_user_byte_regex(
            &pattern,
            config.is_ignore_case,
            &config.byte_ctype,
            config.locale_regex_encoding,
            config.locale_validator.clone(),
            config.single_byte_locale,
        )?;
    } else {
        let pattern = gnu_emacs_regex_to_rust(
            std::str::from_utf8(&bytes).expect("validated UTF-8 word regexp"),
            config,
        );
        compile_user_regex(&pattern, config.is_ignore_case)?;
    }
    Ok(())
}

pub fn ct_app() -> Command {
    let args = vec![
        Arg::new(ptx_options::PTX_FILE)
            .hide(true)
            .action(ArgAction::Append)
            .value_parser(OsStringValueParser::new())
            .value_hint(clap::ValueHint::FilePath),
        Arg::new(ptx_options::PTX_AUTO_REFERENCE)
            .short('A')
            .long(ptx_options::PTX_AUTO_REFERENCE)
            .help(t!("ptx.clap.ptx_auto_reference"))
            .action(ArgAction::SetTrue),
        Arg::new(ptx_options::PTX_TRADITIONAL)
            .short('G')
            .long(ptx_options::PTX_TRADITIONAL)
            .help(t!("ptx.clap.ptx_traditional"))
            .action(ArgAction::SetTrue),
        Arg::new(ptx_options::PTX_FLAG_TRUNCATION)
            .short('F')
            .long(ptx_options::PTX_FLAG_TRUNCATION)
            .help(t!("ptx.clap.ptx_flag_truncation"))
            .value_name("STRING")
            .value_parser(OsStringValueParser::new()),
        Arg::new(ptx_options::PTX_MACRO_NAME)
            .short('M')
            .long(ptx_options::PTX_MACRO_NAME)
            .help(t!("ptx.clap.ptx_macro_name"))
            .value_name("STRING")
            .value_parser(OsStringValueParser::new()),
        Arg::new(ptx_options::PTX_FORMAT_ROFF)
            .short('O')
            .help(t!("ptx.clap.ptx_format_roff"))
            .action(ArgAction::SetTrue),
        Arg::new(ptx_options::PTX_FORMAT)
            .long(ptx_options::PTX_FORMAT)
            .value_name("FORMAT")
            .value_parser(CtShortcutValueParser::new(["roff", "tex"])),
        Arg::new(ptx_options::PTX_RIGHT_SIDE_REFS)
            .short('R')
            .long(ptx_options::PTX_RIGHT_SIDE_REFS)
            .help(t!("ptx.clap.ptx_right_side_refs"))
            .action(ArgAction::SetTrue),
        Arg::new(ptx_options::PTX_SENTENCE_REGEXP)
            .short('S')
            .long(ptx_options::PTX_SENTENCE_REGEXP)
            .help(t!("ptx.clap.ptx_sentence_regexp"))
            .value_name("REGEXP")
            .value_parser(OsStringValueParser::new()),
        Arg::new(ptx_options::PTX_FORMAT_TEX)
            .short('T')
            .help(t!("ptx.clap.ptx_format_tex"))
            .action(ArgAction::SetTrue),
        Arg::new(ptx_options::PTX_WORD_REGEXP)
            .short('W')
            .long(ptx_options::PTX_WORD_REGEXP)
            .help(t!("ptx.clap.ptx_word_regexp"))
            .value_name("REGEXP")
            .value_parser(OsStringValueParser::new()),
        Arg::new(ptx_options::PTX_BREAK_FILE)
            .short('b')
            .long(ptx_options::PTX_BREAK_FILE)
            .help(t!("ptx.clap.ptx_break_file"))
            .value_name("FILE")
            .value_parser(OsStringValueParser::new())
            .value_hint(clap::ValueHint::FilePath),
        Arg::new(ptx_options::PTX_IGNORE_CASE)
            .short('f')
            .long(ptx_options::PTX_IGNORE_CASE)
            .help(t!("ptx.clap.ptx_ignore_case"))
            .action(ArgAction::SetTrue),
        Arg::new(ptx_options::PTX_GAP_SIZE)
            .short('g')
            .long(ptx_options::PTX_GAP_SIZE)
            .help(t!("ptx.clap.ptx_gap_size"))
            .allow_hyphen_values(true)
            .value_name("NUMBER"),
        Arg::new(ptx_options::PTX_IGNORE_FILE)
            .short('i')
            .long(ptx_options::PTX_IGNORE_FILE)
            .help(t!("ptx.clap.ptx_ignore_file"))
            .value_name("FILE")
            .value_parser(OsStringValueParser::new())
            .value_hint(clap::ValueHint::FilePath),
        Arg::new(ptx_options::PTX_ONLY_FILE)
            .short('o')
            .long(ptx_options::PTX_ONLY_FILE)
            .help(t!("ptx.clap.ptx_only_file"))
            .value_name("FILE")
            .value_parser(OsStringValueParser::new())
            .value_hint(clap::ValueHint::FilePath),
        Arg::new(ptx_options::PTX_REFERENCES)
            .short('r')
            .long(ptx_options::PTX_REFERENCES)
            .help(t!("ptx.clap.ptx_references"))
            .action(ArgAction::SetTrue),
        Arg::new(ptx_options::PTX_TYPESET_MODE)
            .short('t')
            .long(ptx_options::PTX_TYPESET_MODE)
            .help("not implemented.")
            .action(ArgAction::SetTrue),
        Arg::new(ptx_options::PTX_WIDTH)
            .short('w')
            .long(ptx_options::PTX_WIDTH)
            .help(t!("ptx.clap.ptx_width"))
            .allow_hyphen_values(true)
            .value_name("NUMBER"),
    ];

    Command::new(ctcore::ct_util_name())
        .about(t!("ptx.about"))
        .version(crate_version!())
        .override_usage(t!("ptx.usage"))
        .disable_help_flag(true)
        .disable_version_flag(true)
        .infer_long_args(true)
        .args_override_self(true)
        .arg(
            Arg::new("help")
                .long("help")
                .help("display this help and exit")
                .action(ArgAction::Help),
        )
        .arg(
            Arg::new("version")
                .long("version")
                .help("output version information and exit")
                .action(ArgAction::Version),
        )
        .args(args)
}

#[derive(Default)]
pub struct Ptx;
impl Tool for Ptx {
    fn name(&self) -> &'static str {
        "ptx"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        ptx_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn test_word_ref(
        word: &str,
        local_line_nr: usize,
        position: usize,
        position_end: usize,
    ) -> WordRef {
        WordRef {
            raw_word: word.as_bytes().to_vec(),
            word: word.to_string(),
            global_line_nr: local_line_nr,
            local_line_nr,
            position,
            position_end,
            global_position: position,
            global_position_end: position_end,
            context_start: 0,
            context_end: 0,
            global_char_position: position,
            global_char_position_end: position_end,
            context_char_start: 0,
            context_char_end: 0,
            file_index: 0,
            input_reference: Vec::new(),
            reference_line_nr: local_line_nr,
        }
    }

    fn test_file_content(filename: &str, lines: Vec<String>, offset: usize) -> FileContent {
        let mut text = String::new();
        let mut line_starts = Vec::with_capacity(lines.len());
        for (index, line) in lines.iter().enumerate() {
            if index > 0 {
                text.push('\n');
            }
            line_starts.push(text.len());
            text.push_str(line);
        }
        let raw_text = text.as_bytes().to_vec();
        let invalid_utf8_bytes = ptx_invalid_utf8_mask(&raw_text);
        let raw_lines = lines.iter().map(|line| line.as_bytes().to_vec()).collect();
        FileContent {
            filename: filename.to_string(),
            raw_filename: filename.as_bytes().to_vec(),
            chars_text: text.chars().collect(),
            byte_to_char: build_byte_to_char_map(&text),
            text,
            raw_text,
            invalid_utf8_bytes,
            line_starts,
            chars_lines: lines.iter().map(|line| line.chars().collect()).collect(),
            lines,
            raw_lines,
            offset,
        }
    }

    #[test]
    fn test_tool_implementation() {
        let tool = Ptx;

        // 测试 name 方法
        assert_eq!(tool.name(), "ptx");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("ptx"));

        // 测试 execute 方法
        let args = vec![
            OsString::from("ptx"),
            OsString::from("--definitely-invalid-flag"),
        ];
        assert!(tool.execute(&args).is_err());
    }

    #[test]
    fn test_version_uses_syskits_package_version() {
        let mut out = Vec::new();
        ptx_main_with_writer(
            [OsString::from("ptx"), OsString::from("--version")].into_iter(),
            &mut out,
        )
        .unwrap();

        let text = String::from_utf8(out).unwrap();
        assert!(text.ends_with(&format!(" {}\n", crate_version!())));
        assert!(!text.contains("GNU coreutils"));
    }

    #[test]
    fn test_help_uses_syskits_command_definition() {
        let mut out = Vec::new();
        ptx_main_with_writer(
            [OsString::from("ptx"), OsString::from("--help")].into_iter(),
            &mut out,
        )
        .unwrap();

        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("Usage:"));
        assert!(text.contains("--auto-reference"));
        assert!(!text.contains("GNU coreutils"));
        assert!(!text.contains("translationproject.org"));

        let options = text
            .split_once("Options:")
            .map(|(_, options)| options)
            .expect("help should include options");
        let option_lines: Vec<&str> = options
            .lines()
            .filter(|line| line.trim_start().starts_with('-'))
            .collect();
        assert!(option_lines.len() > 1);
        for window in option_lines.windows(2) {
            let first = options.find(window[0]).unwrap();
            let second = options.find(window[1]).unwrap();
            assert!(
                !options[first..second].contains("\n\n"),
                "options should be rendered in compact help format"
            );
        }
    }

    #[test]
    fn test_cli_dumb_formatter_w10_two_tokens_alignment() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), b"foo bar\n").unwrap();

        let mut out = Vec::new();
        ptx_main_with_writer(
            [
                OsString::from("ptx"),
                OsString::from("-w10"),
                OsString::from(temp.path()),
            ]
            .into_iter(),
            &mut out,
        )
        .unwrap();

        assert_eq!(out, b"     /   bar\n        foo/\n");
    }

    mod config_tests {
        use super::*;

        #[test]
        fn test_get_config_default() {
            let matches = ct_app().try_get_matches_from(vec!["ptx"]).unwrap();
            let config = get_config(&matches).unwrap();
            assert!(config.is_gnu_ext);
            assert!(matches!(config.format, OutFormat::Dumb));
        }

        #[test]
        fn test_get_config_sentence_regexp_supported() {
            let matches = ct_app()
                .try_get_matches_from(vec!["ptx", "-G", "-S", "[A-Z].*"])
                .unwrap();
            let config = get_config(&matches).unwrap();
            assert_eq!(
                config.context_pattern_bytes.as_deref(),
                Some(b"[A-Z].*".as_slice())
            );
            assert_eq!(config.context_regex, "(?:(?!)\\x00).*");
        }

        #[test]
        fn test_get_config_sentence_regexp_zero_len_accepted() {
            // Zero-length regex check is now deferred to execution time
            // to match GNU ptx behavior (only errors when processing non-empty content)
            let matches = ct_app()
                .try_get_matches_from(vec!["ptx", "-S", "^"])
                .unwrap();
            let config = get_config(&matches).unwrap();
            assert_eq!(config.context_regex, "^");
        }

        #[test]
        fn test_get_config_traditional() {
            let matches = ct_app().try_get_matches_from(vec!["ptx", "-G"]).unwrap();
            let config = get_config(&matches).unwrap();
            assert!(!config.is_gnu_ext);
            assert!(matches!(config.format, OutFormat::Roff));
            assert_eq!(config.context_regex, "\n");
        }

        #[test]
        fn test_get_config_with_options() {
            let matches = ct_app()
                .try_get_matches_from(vec![
                    "ptx", "-G", "-w", "80", "-g", "4", "-M", "test", "-F", "*", "-O",
                ])
                .unwrap();
            let config = get_config(&matches).unwrap();
            assert_eq!(config.line_width, 80);
            assert_eq!(config.gap_size, 4);
            assert_eq!(config.macro_name, "test");
            assert_eq!(config.trunc_str, "*");
            assert!(matches!(config.format, OutFormat::Roff));
        }
    }

    mod filter_tests {
        use super::*;

        fn create_temp_file_with_content(content: &str) -> NamedTempFile {
            let mut file = NamedTempFile::new().unwrap();
            write!(file, "{content}").unwrap();
            file
        }

        #[test]
        fn test_read_word_filter_file() {
            let file = create_temp_file_with_content("word1\nword2\nword3");
            let matches = ct_app()
                .try_get_matches_from(vec!["ptx", "-o", file.path().to_str().unwrap()])
                .unwrap();

            let words = read_word_filter_file(&matches, ptx_options::PTX_ONLY_FILE).unwrap();
            assert_eq!(words.len(), 3);
            assert!(words.contains(b"word1".as_slice()));
            assert!(words.contains(b"word2".as_slice()));
            assert!(words.contains(b"word3".as_slice()));
        }

        #[test]
        fn test_read_char_filter_file() {
            let file = create_temp_file_with_content("abc");
            let matches = ct_app()
                .try_get_matches_from(vec!["ptx", "-b", file.path().to_str().unwrap()])
                .unwrap();

            let chars = read_char_filter_file(&matches, ptx_options::PTX_BREAK_FILE).unwrap();
            assert_eq!(chars.len(), 3);
            assert!(chars.contains(&b'a'));
            assert!(chars.contains(&b'b'));
            assert!(chars.contains(&b'c'));
        }

        #[test]
        fn test_word_filter_new() {
            let config = PtxConfig::default();
            let matches = ct_app().try_get_matches_from(vec!["ptx"]).unwrap();

            let filter = WordFilter::new(&matches, &config).unwrap();
            assert!(!filter.is_only_specified);
            assert!(!filter.is_ignore_specified);
            assert_eq!(filter.word_regex, "[A-Za-z]+");
        }

        #[test]
        fn test_word_filter_break_file_generates_regex() {
            let breaker = create_temp_file_with_content("/");
            let matches = ct_app()
                .try_get_matches_from(vec!["ptx", "-G", "-b", breaker.path().to_str().unwrap()])
                .unwrap();
            let config = PtxConfig::default();
            let filter = WordFilter::new(&matches, &config).unwrap();
            assert_eq!(filter.word_regex, "[^/]+");
        }

        #[test]
        fn test_word_filter_custom_word_regex() {
            let matches = ct_app()
                .try_get_matches_from(vec!["ptx", "-G", "-W", "[A-Z]+"])
                .unwrap();
            let config = PtxConfig::default();
            let filter = WordFilter::new(&matches, &config).unwrap();
            assert_eq!(filter.word_regex, "[A-Z]+");
        }

        #[test]
        fn test_word_filter_uses_gnu_emacs_bracket_semantics() {
            let matches = ct_app()
                .try_get_matches_from(vec!["ptx", "-W", "[[:alpha:]]+"])
                .unwrap();
            let config = PtxConfig::default();

            let filter = WordFilter::new(&matches, &config).unwrap();

            assert_eq!(filter.word_regex, r"[\[:alpha:]]+");
        }
    }

    mod word_ref_tests {
        use super::*;

        #[test]
        fn test_word_ref_ordering() {
            let word1 = test_word_ref("test", 1, 0, 4);
            let word2 = WordRef {
                global_line_nr: 2,
                ..test_word_ref("test", 1, 0, 4)
            };

            assert!(word1 < word2);
        }
    }

    mod formatting_tests {
        use super::*;

        #[test]
        fn test_format_roff_line() {
            let config = PtxConfig {
                format: OutFormat::Roff,
                macro_name: "xx".to_string(),
                ..Default::default()
            };

            let word_ref = test_word_ref("test", 1, 6, 10);

            let line = "hello test world";
            let chars_line: Vec<char> = line.chars().collect();
            let maximum_word_length = ptx_maximum_word_length_in_chars(&chars_line, &config);
            let reference = "1";
            let context_reg = compile_regex_case_lossy(&config.context_regex, false);

            let result = ptx_format_roff_line(
                &config,
                &word_ref,
                line,
                &chars_line,
                line,
                &chars_line,
                &context_reg,
                reference,
                config.line_width,
                maximum_word_length,
            );
            assert!(result.starts_with(".xx"));
            assert!(result.contains("test"));
        }

        #[test]
        fn test_format_dumb_line_w10_two_tokens_alignment() {
            let config = PtxConfig {
                line_width: 10,
                gap_size: 3,
                trunc_str: "/".to_string(),
                ..Default::default()
            };
            let line = "foo bar";
            let chars_line: Vec<char> = line.chars().collect();
            let maximum_word_length = ptx_maximum_word_length_in_chars(&chars_line, &config);
            let word_ref = test_word_ref("bar", 0, 4, 7);
            let context_reg = compile_regex_case_lossy(&config.context_regex, false);
            let got = ptx_format_dumb_line(
                &config,
                &word_ref,
                line,
                &chars_line,
                line,
                &chars_line,
                &context_reg,
                "",
                0,
                maximum_word_length,
            );
            assert_eq!(got, "     /   bar");
        }

        #[test]
        fn test_format_dumb_line_sentence_regex_alignment() {
            let config = PtxConfig {
                context_regex: "[.!?]".to_string(),
                ..Default::default()
            };
            let line = "alpha. beta! gamma?";
            let chars_line: Vec<char> = line.chars().collect();
            let maximum_word_length = ptx_maximum_word_length_in_chars(&chars_line, &config);
            let word_ref = test_word_ref("beta", 0, 7, 11);
            let context_reg = compile_regex_case_lossy(&config.context_regex, false);
            let got = ptx_format_dumb_line(
                &config,
                &word_ref,
                line,
                &chars_line,
                line,
                &chars_line,
                &context_reg,
                "",
                0,
                maximum_word_length,
            );
            assert_eq!(got, "                                        beta!");
        }
    }

    mod execution_tests {
        use super::*;
        use tempfile::NamedTempFile;

        #[test]
        fn test_ptx_exec() {
            // 创建测试配置
            let mut settings = PtxSettings {
                config: PtxConfig {
                    format: OutFormat::Roff,
                    is_gnu_ext: false,
                    ..Default::default()
                },
                file_map: {
                    vec![test_file_content(
                        "test.txt",
                        vec!["hello test world".to_string()],
                        0,
                    )]
                },
                words: {
                    let mut set = BTreeSet::new();
                    set.insert(test_word_ref("test", 0, 6, 10));
                    set
                },
                output_file: Some(NamedTempFile::new().unwrap().reopen().unwrap()),
            };

            let result = ptx_exec(&mut settings);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ptx_exec_dumb_format() {
            let mut settings = PtxSettings {
                config: PtxConfig {
                    format: OutFormat::Dumb,
                    ..Default::default()
                },
                file_map: { vec![test_file_content("test.txt", vec!["test".to_string()], 0)] },
                words: {
                    let mut set = BTreeSet::new();
                    set.insert(test_word_ref("test", 0, 0, 4));
                    set
                },
                output_file: None,
            };

            let result = ptx_exec(&mut settings);
            assert!(result.is_ok());
        }
    }

    mod output_chunk_tests {
        use super::*;

        #[test]
        fn test_get_output_chunks_basic() {
            let config = PtxConfig {
                line_width: 20,
                gap_size: 3,
                trunc_str: "/".to_string(),
                ..Default::default()
            };

            let before = &['h', 'e', 'l', 'l', 'o', ' '];
            let keyword = "test";
            let after = &[' ', 'w', 'o', 'r', 'l', 'd'];
            let max_word_len = 5;

            let (tail, before_out, after_out, head, _) = ptx_get_output_chunks_for_width_with_max(
                before,
                keyword,
                after,
                &config,
                config.line_width,
                max_word_len,
            );

            assert_eq!(tail, "");
            assert_eq!(before_out, "hello");
            assert_eq!(after_out, "/");
            assert_eq!(head, "");
        }

        #[test]
        fn test_get_output_chunks_long_line() {
            let config = PtxConfig {
                line_width: 5, // 设置更小的宽度以确保触发截断
                gap_size: 2,
                trunc_str: "*".to_string(),
                ..Default::default()
            };

            // 使用更长的文本
            let before = &[
                'v', 'e', 'r', 'y', ' ', 'l', 'o', 'n', 'g', ' ', 't', 'e', 'x', 't', ' ',
            ];
            let keyword = "test";
            let after = &[
                ' ', 'h', 'e', 'r', 'e', ' ', 'a', 'n', 'd', ' ', 't', 'h', 'e', 'r', 'e',
            ];
            let max_word_len = 5;

            let (_tail, before_out, after_out, _head, _) = ptx_get_output_chunks_for_width_with_max(
                before,
                keyword,
                after,
                &config,
                config.line_width,
                max_word_len,
            );

            // 验证长文本被适当截断
            assert!(!before_out.is_empty());
            assert!(!after_out.is_empty());
            assert!(before_out.len() + after_out.len() <= config.line_width);
            assert!(before_out.contains('*') || after_out.contains('*')); // 修改断言检查实际输出部分
        }

        #[test]
        fn test_get_output_chunks_empty_context() {
            let config = PtxConfig::default();

            let before = &[];
            let keyword = "test";
            let after = &[];
            let max_word_len = 4;

            let (tail, before_out, after_out, head, _) = ptx_get_output_chunks_for_width_with_max(
                before,
                keyword,
                after,
                &config,
                config.line_width,
                max_word_len,
            );

            assert_eq!(tail, "");
            assert_eq!(before_out, "");
            assert_eq!(after_out, "");
            assert_eq!(head, "");
        }

        #[test]
        fn test_get_output_chunks_whitespace() {
            let config = PtxConfig {
                trunc_str: "/".to_string(),
                ..Default::default()
            };

            let before = &[' ', ' ', ' '];
            let keyword = "test";
            let after = &[' ', ' ', ' '];
            let max_word_len = 4;

            let (tail, before_out, after_out, head, _) = ptx_get_output_chunks_for_width_with_max(
                before,
                keyword,
                after,
                &config,
                config.line_width,
                max_word_len,
            );

            // 验证空白字符被正确处理
            assert_eq!(tail, "");
            assert_eq!(before_out, "");
            assert_eq!(after_out, "");
            assert_eq!(head, "");
        }
    }

    mod input_processing_tests {
        use super::*;
        use tempfile::NamedTempFile;

        #[test]
        fn test_ptx_read_input() {
            // 创建测试文件
            let mut file = NamedTempFile::new().unwrap();
            writeln!(file, "line one\nline two").unwrap();

            let config = PtxConfig {
                is_gnu_ext: false,
                ..Default::default()
            };

            let input_files = vec![file.path().as_os_str().to_os_string()];
            let result = ptx_read_input(&input_files, &config).unwrap();

            assert_eq!(result.len(), 1);
            let content = &result[0];
            assert_eq!(content.lines, vec!["line one", "line two"]);
            assert_eq!(content.offset, 0);
        }

        #[test]
        fn test_ptx_read_input_multiple_files() {
            let mut file1 = NamedTempFile::new().unwrap();
            let mut file2 = NamedTempFile::new().unwrap();
            writeln!(file1, "file1").unwrap();
            writeln!(file2, "file2").unwrap();

            let config = PtxConfig {
                is_gnu_ext: true, // 允许多文件
                ..Default::default()
            };

            let input_files = vec![
                file1.path().as_os_str().to_os_string(),
                file2.path().as_os_str().to_os_string(),
            ];
            let result = ptx_read_input(&input_files, &config).unwrap();

            assert_eq!(result.len(), 2);
        }
    }

    mod word_set_tests {
        use super::*;

        #[test]
        fn test_ptx_create_word_set() {
            let config = PtxConfig {
                is_ignore_case: false,
                is_input_ref: false,
                ..Default::default()
            };

            let filter = WordFilter {
                is_only_specified: false,
                is_ignore_specified: false,
                only_set: HashSet::new(),
                ignore_set: HashSet::new(),
                word_regex: r"\w+".to_string(),
                word_byte_pattern: None,
                word_pattern_bytes: None,
                break_set: None,
                uses_custom_regex: false,
            };

            let file_map = vec![test_file_content(
                "test.txt",
                vec!["hello world".to_string()],
                0,
            )];

            let word_set = ptx_create_word_set(&config, &filter, &file_map);

            assert_eq!(word_set.len(), 2); // "hello" 和 "world"
            assert!(word_set.iter().any(|w| w.word == "hello"));
            assert!(word_set.iter().any(|w| w.word == "world"));
        }

        #[test]
        fn test_ptx_create_word_set_with_ignore_case() {
            let config = PtxConfig {
                is_ignore_case: true,
                ..Default::default()
            };

            let filter = WordFilter {
                word_regex: r"\w+".to_string(),
                ..Default::default()
            };

            let file_map = vec![test_file_content(
                "test.txt",
                vec!["Hello WORLD".to_string()],
                0,
            )];

            let word_set = ptx_create_word_set(&config, &filter, &file_map);

            assert!(word_set.iter().any(|w| w.word == "hello"));
            assert!(word_set.iter().any(|w| w.word == "world"));
        }

        #[test]
        fn test_ptx_create_word_set_skips_input_reference_field() {
            let config = PtxConfig {
                is_input_ref: true,
                context_regex: "\n".to_string(),
                ..Default::default()
            };
            let filter = WordFilter {
                word_regex: r"[A-Za-z]+".to_string(),
                ..Default::default()
            };
            let file_map = vec![test_file_content(
                "test.txt",
                vec!["source openssl".to_string(), "ref hello world".to_string()],
                0,
            )];

            let words: Vec<String> = ptx_create_word_set(&config, &filter, &file_map)
                .into_iter()
                .map(|word_ref| word_ref.word)
                .collect();

            assert!(!words.iter().any(|word| word == "source"));
            assert!(!words.iter().any(|word| word == "ref"));
            assert!(words.iter().any(|word| word == "openssl"));
            assert!(words.iter().any(|word| word == "hello"));
            assert!(words.iter().any(|word| word == "world"));
        }
    }

    mod reference_tests {
        use super::*;

        #[test]
        fn test_ptx_get_reference_auto_ref() {
            let config = PtxConfig {
                is_auto_ref: true,
                is_input_ref: false,
                ..Default::default()
            };

            let word_ref = test_word_ref("test", 0, 0, 4);

            let context_reg = Regex::new(&config.context_regex).unwrap();
            let reference =
                ptx_get_reference(&config, &word_ref, "test.txt", "test line", &context_reg);

            assert_eq!(reference, "test.txt:1");
        }

        #[test]
        fn test_ptx_get_reference_input_ref() {
            let config = PtxConfig {
                is_auto_ref: false,
                is_input_ref: true,
                ..Default::default()
            };

            let word_ref = WordRef {
                input_reference: b"123".to_vec(),
                ..WordRef::default()
            };
            let context_reg = Regex::new(&config.context_regex).unwrap();
            let reference = ptx_get_reference(
                &config,
                &word_ref,
                "test.txt",
                "123 word text",
                &context_reg,
            );

            assert_eq!(reference, "123");
        }
    }
}
