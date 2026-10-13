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

use crate::csplit_error::{CsplitError, PatternText};
#[cfg(test)]
use ctcore::ct_display::locale_quote;
use ctcore::ct_show_warning;
use onig::{EncodedBytes, Error as OnigError, Regex as OnigRegex, RegexOptions, Syntax};
use std::fmt;

#[cfg(all(target_os = "linux", target_env = "gnu"))]
use std::cell::RefCell;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
use std::ffi::CStr;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
use std::sync::{Mutex, OnceLock};

#[cfg(all(target_os = "linux", target_env = "gnu"))]
const GNU_CSPLIT_REGEX_SYNTAX: libc::c_ulong = (1 << 1) | (1 << 2) | (1 << 6) | (1 << 7) | (1 << 9);

#[cfg(all(target_os = "linux", target_env = "gnu"))]
static GNU_REGEX_CONFIGURATION_LOCK: Mutex<()> = Mutex::new(());

#[cfg(all(target_os = "linux", target_env = "gnu"))]
type ReSetSyntax = unsafe extern "C" fn(libc::c_ulong) -> libc::c_ulong;

#[cfg(all(target_os = "linux", target_env = "gnu"))]
type ReCompilePattern = unsafe extern "C" fn(
    *const libc::c_char,
    libc::size_t,
    *mut libc::regex_t,
) -> *const libc::c_char;

#[cfg(all(target_os = "linux", target_env = "gnu"))]
type ReSearch = unsafe extern "C" fn(
    *mut libc::regex_t,
    *const libc::c_char,
    libc::regoff_t,
    libc::regoff_t,
    libc::regoff_t,
    *mut libc::c_void,
) -> libc::regoff_t;

#[cfg(all(target_os = "linux", target_env = "gnu"))]
struct GnuRegexApi {
    set_syntax: ReSetSyntax,
    compile_pattern: ReCompilePattern,
    search: ReSearch,
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn gnu_regex_api() -> Result<&'static GnuRegexApi, String> {
    static API: OnceLock<Result<GnuRegexApi, String>> = OnceLock::new();
    API.get_or_init(load_gnu_regex_api)
        .as_ref()
        .map_err(Clone::clone)
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn load_gnu_regex_api() -> Result<GnuRegexApi, String> {
    unsafe {
        let handle = libc::dlopen(c"libc.so.6".as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
        if handle.is_null() {
            return Err(dynamic_loader_error());
        }

        let set_syntax = libc::dlsym(handle, c"re_set_syntax".as_ptr());
        if set_syntax.is_null() {
            return Err(dynamic_loader_error());
        }
        let compile_pattern = libc::dlsym(handle, c"re_compile_pattern".as_ptr());
        if compile_pattern.is_null() {
            return Err(dynamic_loader_error());
        }
        let search = libc::dlsym(handle, c"re_search".as_ptr());
        if search.is_null() {
            return Err(dynamic_loader_error());
        }

        Ok(GnuRegexApi {
            set_syntax: std::mem::transmute::<*mut libc::c_void, ReSetSyntax>(set_syntax),
            compile_pattern: std::mem::transmute::<*mut libc::c_void, ReCompilePattern>(
                compile_pattern,
            ),
            search: std::mem::transmute::<*mut libc::c_void, ReSearch>(search),
        })
    }
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn dynamic_loader_error() -> String {
    let error = unsafe { libc::dlerror() };
    if error.is_null() {
        "unable to load GNU regular expression API".to_string()
    } else {
        unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .into_owned()
    }
}

#[derive(Debug)]
enum RegexEngine {
    Oniguruma(OnigRegex),
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    Libc(LibcRegex),
}

impl RegexEngine {
    fn is_match(&self, text: &[u8]) -> bool {
        match self {
            Self::Oniguruma(regex) => regex
                .find_with_encoding(EncodedBytes::ascii(text))
                .is_some(),
            #[cfg(all(target_os = "linux", target_env = "gnu"))]
            Self::Libc(regex) => regex.is_match(text),
        }
    }
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
struct LibcRegex {
    compiled: RefCell<libc::regex_t>,
    search: ReSearch,
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
impl fmt::Debug for LibcRegex {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("LibcRegex").finish_non_exhaustive()
    }
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
impl LibcRegex {
    fn new(expression: &[u8]) -> Result<Self, String> {
        let api = gnu_regex_api()?;
        let _configuration_guard = GNU_REGEX_CONFIGURATION_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut compiled = unsafe { std::mem::zeroed::<libc::regex_t>() };

        unsafe {
            libc::setlocale(libc::LC_ALL, c"".as_ptr());
        }

        let previous_syntax = unsafe { (api.set_syntax)(GNU_CSPLIT_REGEX_SYNTAX) };
        let error = unsafe {
            (api.compile_pattern)(expression.as_ptr().cast(), expression.len(), &mut compiled)
        };
        unsafe {
            (api.set_syntax)(previous_syntax);
        }

        if !error.is_null() {
            let message = unsafe { CStr::from_ptr(error) }
                .to_string_lossy()
                .into_owned();
            unsafe {
                libc::regfree(&mut compiled);
            }
            return Err(message);
        }

        Ok(Self {
            compiled: RefCell::new(compiled),
            search: api.search,
        })
    }

    fn is_match(&self, text: &[u8]) -> bool {
        let length = text
            .len()
            .try_into()
            .expect("input line length fits in regoff_t");
        let mut compiled = self.compiled.borrow_mut();
        unsafe {
            (self.search)(
                &mut *compiled,
                text.as_ptr().cast(),
                length,
                0,
                length,
                std::ptr::null_mut(),
            ) >= 0
        }
    }
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
impl Drop for LibcRegex {
    fn drop(&mut self) {
        unsafe {
            libc::regfree(self.compiled.get_mut());
        }
    }
}

#[derive(Debug)]
pub struct CsplitRegex {
    compiled: RegexEngine,
    expression: Vec<u8>,
    source: PatternText,
}

impl CsplitRegex {
    fn new(expression: &[u8], source: PatternText) -> Result<Self, CsplitError> {
        #[cfg(all(target_os = "linux", target_env = "gnu"))]
        if should_use_libc_regex(expression) {
            let compiled = LibcRegex::new(expression)
                .map(RegexEngine::Libc)
                .map_err(|error| CsplitError::InvalidRegularExpression(source.clone(), error))?;
            return Ok(Self {
                compiled,
                expression: expression.to_owned(),
                source,
            });
        }

        let expression_for_compilation = normalize_empty_bre_intervals(expression);
        validate_bre_interval_ranges(&expression_for_compilation, &source)?;
        let compiled = compile_regex(&expression_for_compilation, &source)?;
        validate_bre_backreferences(expression, &source)?;
        Ok(Self {
            compiled,
            expression: expression.to_owned(),
            source,
        })
    }

    pub fn is_match(&self, text: &[u8]) -> bool {
        self.compiled.is_match(text)
    }
}

fn compile_regex(expression: &[u8], source: &PatternText) -> Result<RegexEngine, CsplitError> {
    OnigRegex::with_options_and_encoding(
        EncodedBytes::ascii(expression),
        RegexOptions::REGEX_OPTION_NONE,
        Syntax::grep(),
    )
    .map_err(|error| {
        CsplitError::InvalidRegularExpression(
            source.clone(),
            gnu_regular_expression_error(&error, expression).to_string(),
        )
    })
    .map(RegexEngine::Oniguruma)
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn should_use_libc_regex(expression: &[u8]) -> bool {
    requires_libc_regex(expression) || locale_has_non_c_character_rules()
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn locale_has_non_c_character_rules() -> bool {
    let _configuration_guard = GNU_REGEX_CONFIGURATION_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    unsafe {
        if libc::setlocale(libc::LC_ALL, c"".as_ptr()).is_null() {
            return false;
        }

        [libc::LC_CTYPE, libc::LC_COLLATE]
            .into_iter()
            .any(|category| {
                let locale = libc::setlocale(category, std::ptr::null());
                if locale.is_null() {
                    return false;
                }
                !matches!(CStr::from_ptr(locale).to_bytes(), b"C" | b"POSIX")
            })
    }
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn requires_libc_regex(expression: &[u8]) -> bool {
    let mut index = 0;
    let mut in_bracket_expression = false;
    let mut character_class_seen = false;

    while index < expression.len() {
        if expression[index] == b'\\' {
            index += 2;
            continue;
        }

        if !in_bracket_expression {
            if expression[index] == b'[' {
                in_bracket_expression = true;
            }
            index += 1;
            continue;
        }

        if expression[index] == b'[' {
            match expression.get(index + 1) {
                Some(b'.' | b'=') => return true,
                Some(b':') => {
                    character_class_seen = true;
                    let class_name_start = index + 2;
                    index += 2;
                    while index + 1 < expression.len()
                        && (expression[index] != b':' || expression[index + 1] != b']')
                    {
                        index += 1;
                    }
                    if index + 1 >= expression.len()
                        || !expression[class_name_start..index]
                            .iter()
                            .all(u8::is_ascii_alphabetic)
                    {
                        return true;
                    }
                    index += 2;
                    continue;
                }
                _ => {}
            }
        }

        if expression[index] == b']' {
            in_bracket_expression = false;
        }
        index += 1;
    }

    character_class_seen && in_bracket_expression
}

fn unterminated_bre_bracket_has_content(expression: &[u8]) -> bool {
    let mut index = 0;
    let mut in_bracket_expression = false;
    let mut has_content = false;

    while index < expression.len() {
        if !in_bracket_expression {
            if expression[index] == b'\\' {
                index += 2;
            } else if expression[index] == b'[' {
                in_bracket_expression = true;
                has_content = false;
                index += 1;
                if expression.get(index) == Some(&b'^') {
                    index += 1;
                }
            } else {
                index += 1;
            }
            continue;
        }

        if expression[index] == b']' && has_content {
            in_bracket_expression = false;
            index += 1;
            continue;
        }

        has_content = true;
        if expression[index] == b'\\' {
            index += 2;
        } else {
            index += 1;
        }
    }

    in_bracket_expression && has_content
}

fn gnu_regular_expression_error(error: &OnigError, expression: &[u8]) -> &'static str {
    match error.code() {
        onig_sys::ONIGERR_END_PATTERN_AT_LEFT_BRACE => "Unmatched \\{",
        onig_sys::ONIGERR_EMPTY_CHAR_CLASS => "Unmatched [, [^, [:, [., or [=",
        onig_sys::ONIGERR_PREMATURE_END_OF_CHAR_CLASS
            if unterminated_bre_bracket_has_content(expression) =>
        {
            "Unmatched [, [^, [:, [., or [="
        }
        onig_sys::ONIGERR_UNMATCHED_CLOSE_PARENTHESIS => "Unmatched ) or \\)",
        onig_sys::ONIGERR_END_PATTERN_WITH_UNMATCHED_PARENTHESIS
        | onig_sys::ONIGERR_END_PATTERN_IN_GROUP => "Unmatched ( or \\(",
        onig_sys::ONIGERR_UNMATCHED_RANGE_SPECIFIER_IN_CHAR_CLASS => "Invalid range end",
        onig_sys::ONIGERR_INVALID_BACKREF => "Invalid back reference",
        onig_sys::ONIGERR_INVALID_POSIX_BRACKET_TYPE => "Invalid character class name",
        onig_sys::ONIGERR_TOO_BIG_NUMBER_FOR_REPEAT_RANGE => "Regular expression too big",
        onig_sys::ONIGERR_INVALID_REPEAT_RANGE_PATTERN
        | onig_sys::ONIGERR_UPPER_SMALLER_THAN_LOWER_IN_REPEAT_RANGE => "Invalid content of \\{\\}",
        _ => "Invalid regular expression",
    }
}

fn validate_bre_backreferences(expression: &[u8], source: &PatternText) -> Result<(), CsplitError> {
    let mut index = 0;
    let mut in_bracket_expression = false;
    let mut bracket_has_content = false;
    let mut group_stack = Vec::new();
    let mut closed_groups = Vec::new();

    while index < expression.len() {
        if in_bracket_expression {
            if expression[index] == b'['
                && let Some(marker @ (b':' | b'.' | b'=')) = expression.get(index + 1)
            {
                bracket_has_content = true;
                index += 2;
                while index + 1 < expression.len()
                    && (expression[index] != *marker || expression[index + 1] != b']')
                {
                    index += 1;
                }
                index = (index + 2).min(expression.len());
                continue;
            }
            if expression[index] == b']' && bracket_has_content {
                in_bracket_expression = false;
                index += 1;
                continue;
            }
            bracket_has_content = true;
            if expression[index] == b'\\' {
                index += 1;
                if index < expression.len() {
                    index += 1;
                }
            } else {
                index += 1;
            }
            continue;
        }

        if expression[index] == b'[' {
            in_bracket_expression = true;
            bracket_has_content = false;
            index += 1;
            if expression.get(index) == Some(&b'^') {
                index += 1;
            }
            continue;
        }

        if expression[index] != b'\\' {
            index += 1;
            continue;
        }

        let slash_start = index;
        while expression.get(index) == Some(&b'\\') {
            index += 1;
        }
        if (index - slash_start) % 2 == 0 {
            continue;
        }

        match expression.get(index) {
            Some(b'(') => {
                let group = closed_groups.len();
                closed_groups.push(false);
                group_stack.push(group);
            }
            Some(b')') => {
                if let Some(group) = group_stack.pop() {
                    closed_groups[group] = true;
                }
            }
            Some(digit @ b'1'..=b'9') => {
                let group = usize::from(*digit - b'1');
                if !closed_groups.get(group).copied().unwrap_or(false) {
                    return Err(CsplitError::InvalidRegularExpression(
                        source.clone(),
                        "Invalid back reference".to_string(),
                    ));
                }
            }
            _ => {}
        }
        if index < expression.len() {
            index += 1;
        }
    }

    Ok(())
}

fn normalize_empty_bre_intervals(expression: &[u8]) -> Vec<u8> {
    let mut index = 0;
    let mut copied = 0;
    let mut normalized = Vec::with_capacity(expression.len() + 1);
    let mut in_bracket_expression = false;

    while index < expression.len() {
        if in_bracket_expression
            && expression[index] == b'['
            && let Some(marker @ (b':' | b'.' | b'=')) = expression.get(index + 1)
        {
            index += 2;
            while index + 1 < expression.len()
                && (expression[index] != *marker || expression[index + 1] != b']')
            {
                index += 1;
            }
            index += 2;
            continue;
        }

        if expression[index] == b'\\' {
            let slash_start = index;
            while expression.get(index) == Some(&b'\\') {
                index += 1;
            }
            let slash_count = index - slash_start;

            if !in_bracket_expression
                && slash_count % 2 == 1
                && expression.get(index) == Some(&b'{')
            {
                let content_start = index + 1;
                if let Some((content_end, after_interval)) =
                    find_bre_interval_closing(expression, content_start)
                {
                    if slash_start == 1 && expression[0] == b'^' {
                        // GNU treats \{...\} immediately after an anchor as literal text.
                        normalized.extend_from_slice(&expression[copied..slash_start]);
                        normalized.extend_from_slice(&expression[index..content_end]);
                        normalized.push(b'}');
                        copied = after_interval;
                    } else if expression[content_start] == b','
                        && expression[content_start + 1..content_end]
                            .iter()
                            .all(u8::is_ascii_digit)
                    {
                        if slash_start == 0 && slash_count == 1 {
                            // GNU treats a leading \{...\} as literal text.
                            normalized.extend_from_slice(&expression[copied..slash_start]);
                            normalized.extend_from_slice(&expression[index..content_end]);
                            normalized.push(b'}');
                            copied = after_interval;
                        } else if slash_start > 0 {
                            // Oniguruma requires the GNU empty lower bound to be written as zero.
                            normalized.extend_from_slice(&expression[copied..content_start]);
                            normalized.push(b'0');
                            copied = content_start;
                        }
                    }
                    index = after_interval;
                } else {
                    index = expression.len();
                }
                continue;
            }

            if slash_count % 2 == 1 && index < expression.len() {
                index += 1;
            }
            continue;
        }

        match expression[index] {
            b'[' if !in_bracket_expression => in_bracket_expression = true,
            b']' if in_bracket_expression => in_bracket_expression = false,
            _ => {}
        }
        index += 1;
    }

    normalized.extend_from_slice(&expression[copied..]);
    normalized
}

fn find_bre_interval_closing(bytes: &[u8], mut index: usize) -> Option<(usize, usize)> {
    while index < bytes.len() {
        if bytes[index] != b'\\' {
            index += 1;
            continue;
        }

        let slash_start = index;
        while bytes.get(index) == Some(&b'\\') {
            index += 1;
        }
        if (index - slash_start) % 2 == 1 && bytes.get(index) == Some(&b'}') {
            return Some((index - 1, index + 1));
        }
        if (index - slash_start) % 2 == 1 {
            index += 1;
        }
    }

    None
}

fn parse_bre_repeat_count(value: &[u8]) -> Option<u64> {
    if value.is_empty() || !value.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let mut count = 0_u64;
    for digit in value {
        count = count
            .saturating_mul(10)
            .saturating_add(u64::from(*digit - b'0'));
    }
    Some(count)
}

fn bre_repeat_count_exceeds_gnu_limit(value: &[u8]) -> bool {
    const GNU_RE_DUP_MAX: u64 = 0x7fff;

    parse_bre_repeat_count(value).is_some_and(|count| count > GNU_RE_DUP_MAX)
}

fn validate_bre_interval_ranges(
    expression: &[u8],
    source: &PatternText,
) -> Result<(), CsplitError> {
    let bytes = expression;
    let mut index = 0;
    let mut in_bracket_expression = false;

    while index < bytes.len() {
        if in_bracket_expression
            && bytes[index] == b'['
            && let Some(marker @ (b':' | b'.' | b'=')) = bytes.get(index + 1)
        {
            index += 2;
            while index + 1 < bytes.len() && (bytes[index] != *marker || bytes[index + 1] != b']') {
                index += 1;
            }
            index += 2;
            continue;
        }

        if bytes[index] == b'\\' {
            let slash_start = index;
            while bytes.get(index) == Some(&b'\\') {
                index += 1;
            }
            let slash_count = index - slash_start;

            // The last slash in an odd-length run begins a BRE interval delimiter.
            if !in_bracket_expression
                && slash_start > 0
                && slash_count % 2 == 1
                && bytes.get(index) == Some(&b'{')
            {
                let content_start = index + 1;
                let Some((closing_backslash, after_interval)) =
                    find_bre_interval_closing(bytes, content_start)
                else {
                    return Err(CsplitError::InvalidRegularExpression(
                        source.clone(),
                        "Unmatched \\{".to_string(),
                    ));
                };

                let interval = &bytes[content_start..closing_backslash];
                if let Some(comma) = interval.iter().position(|byte| *byte == b',') {
                    let (lower, upper) = interval.split_at(comma);
                    let upper = &upper[1..];
                    if let (Some(lower), Some(upper)) =
                        (parse_bre_repeat_count(lower), parse_bre_repeat_count(upper))
                        && lower > upper
                    {
                        return Err(CsplitError::InvalidRegularExpression(
                            source.clone(),
                            "Invalid content of \\{\\}".to_string(),
                        ));
                    }
                    if bre_repeat_count_exceeds_gnu_limit(lower)
                        || bre_repeat_count_exceeds_gnu_limit(upper)
                    {
                        return Err(CsplitError::InvalidRegularExpression(
                            source.clone(),
                            "Regular expression too big".to_string(),
                        ));
                    }
                } else if bre_repeat_count_exceeds_gnu_limit(interval) {
                    return Err(CsplitError::InvalidRegularExpression(
                        source.clone(),
                        "Regular expression too big".to_string(),
                    ));
                }
                index = after_interval;
                continue;
            }

            if slash_count % 2 == 1 && index < bytes.len() {
                index += 1;
            }
            continue;
        }

        match bytes[index] {
            b'[' if !in_bracket_expression => {
                in_bracket_expression = true;
                index += 1;
            }
            b']' if in_bracket_expression => {
                in_bracket_expression = false;
                index += 1;
            }
            _ => index += 1,
        }
    }

    Ok(())
}

impl fmt::Display for CsplitRegex {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&String::from_utf8_lossy(&self.expression))
    }
}

/// The definition of a pattern to match on a line.
#[derive(Debug)]
pub enum CsplitPattern {
    /// Copy the file's content to a split up to, not including, the given line number. The number
    /// of times the pattern is executed is detailed in [`CsplitExecutePattern`].
    UpToLine(usize, CsplitExecutePattern),
    /// Copy the file's content to a split up to, not including, the line matching the regex. The
    /// integer is an offset relative to the matched line of what to include (if positive) or
    /// to exclude (if negative). The number of times the pattern is executed is detailed in
    /// [`CsplitExecutePattern`].
    UpToMatch(CsplitRegex, i64, CsplitExecutePattern),
    /// Skip the file's content up to, not including, the line matching the regex. The integer
    /// is an offset relative to the matched line of what to include (if positive) or to exclude
    /// (if negative). The number of times the pattern is executed is detailed in [`CsplitExecutePattern`].
    SkipToMatch(CsplitRegex, i64, CsplitExecutePattern),
}

#[allow(clippy::to_string_trait_impl)]
impl ToString for CsplitPattern {
    fn to_string(&self) -> String {
        match self {
            Self::UpToLine(n, _) => n.to_string(),
            Self::UpToMatch(regex, _, _) | Self::SkipToMatch(regex, _, _) => {
                regex.source.to_lossy_string()
            }
        }
    }
}

impl CsplitPattern {
    fn set_execute_n_times(&mut self, execute_n_times: CsplitExecutePattern) {
        match self {
            Self::UpToLine(_, execute)
            | Self::UpToMatch(_, _, execute)
            | Self::SkipToMatch(_, _, execute) => *execute = execute_n_times,
        }
    }

    pub fn source(&self) -> PatternText {
        match self {
            Self::UpToLine(n, _) => n.to_string().into(),
            Self::UpToMatch(regex, _, _) | Self::SkipToMatch(regex, _, _) => regex.source.clone(),
        }
    }
}

/// The number of times a pattern can be used.
#[derive(Debug)]
pub enum CsplitExecutePattern {
    /// Execute the pattern as many times as possible
    Always,
    /// Execute the pattern a fixed number of times
    Times(u64),
}

impl CsplitExecutePattern {
    pub fn iter(&self) -> CsplitExecutePatternIter {
        match self {
            Self::Times(n) => CsplitExecutePatternIter::new(Some(*n)),
            Self::Always => CsplitExecutePatternIter::new(None),
        }
    }
}

pub struct CsplitExecutePatternIter {
    max: Option<u64>,
    cur: u64,
}

impl CsplitExecutePatternIter {
    fn new(max: Option<u64>) -> Self {
        Self { max, cur: 0 }
    }
}

impl Iterator for CsplitExecutePatternIter {
    type Item = (Option<u64>, u64);

    fn next(&mut self) -> Option<(Option<u64>, u64)> {
        match self.max {
            // iterate until m is reached
            Some(m) => {
                if self.cur == m {
                    None
                } else {
                    self.cur += 1;
                    Some((self.max, self.cur))
                }
            }
            // no limit, just increment a counter
            None => {
                self.cur += 1;
                Some((None, self.cur))
            }
        }
    }
}

/// Parses the definitions of patterns given on the command line into a list of [`CsplitPattern`]s.
///
/// # Errors
///
/// If a pattern is incorrect, a [`CsplitError::InvalidPattern`] error is returned, which may be
/// due to, e.g.,:
/// - an invalid regular expression;
/// - an invalid number for, e.g., the offset.
pub fn get_patterns<T: AsRef<[u8]>>(args: &[T]) -> Result<Vec<CsplitPattern>, CsplitError> {
    extract_patterns(args)
}

/**
 * 从命令行参数中提取CSplit模式。
 *
 * 此函数解析一系列字符串参数，并将它们转换为CSplit操作所需的模式集合。
 * 每个参数可以是一个正则表达式模式、行号或者控制重复次数的特殊格式字符串。
 *
 * @param args 需要解析的命令行参数，为一个字符串切片。
 * @return Result<Vec<CsplitPattern>, CsplitError> 如果解析成功，返回一个包含CSplit模式的向量；
 *         如果遇到错误，返回一个包含错误信息的CsplitError。
 */
fn extract_patterns<T: AsRef<[u8]>>(args: &[T]) -> Result<Vec<CsplitPattern>, CsplitError> {
    // 初始化用于储存解析后模式的向量
    let mut csplit_patterns = Vec::with_capacity(args.len());
    let mut previous_line_number = 0;

    // 使用peekable迭代器以便于向前查看下一个参数
    let mut iter = args.iter().peekable();

    while let Some(arg) = iter.next() {
        let arg = arg.as_ref();
        let source = PatternText::from(arg.to_owned());

        // 解析模式定义
        if matches!(arg.first(), Some(b'/') | Some(b'%')) {
            let delimiter = arg[0];
            let closing_delimiter = arg[1..]
                .iter()
                .rposition(|byte| *byte == delimiter)
                .map(|position| position + 1);
            let Some(closing_delimiter) = closing_delimiter else {
                return Err(CsplitError::ClosingDelimiterMissing(
                    source,
                    delimiter as char,
                ));
            };
            let expression = &arg[1..closing_delimiter];
            let offset_text = &arg[closing_delimiter + 1..];
            let offset = if offset_text.is_empty() {
                0
            } else {
                let offset_text = offset_text
                    .iter()
                    .skip_while(|byte| byte.is_ascii_whitespace())
                    .copied()
                    .collect::<Vec<_>>();
                std::str::from_utf8(&offset_text)
                    .ok()
                    .and_then(|offset_text| offset_text.parse::<i64>().ok())
                    .ok_or_else(|| CsplitError::IntegerExpectedAfterDelimiter(source.clone()))?
            };
            let pattern = CsplitRegex::new(expression, source)?;
            if delimiter == b'/' {
                csplit_patterns.push(CsplitPattern::UpToMatch(
                    pattern,
                    offset,
                    CsplitExecutePattern::Times(1),
                ));
            } else {
                csplit_patterns.push(CsplitPattern::SkipToMatch(
                    pattern,
                    offset,
                    CsplitExecutePattern::Times(1),
                ));
            }
        } else if let Ok(argument) = std::str::from_utf8(arg)
            && let Ok(line_number) = argument
                .trim_start_matches(|character: char| character.is_ascii_whitespace())
                .parse::<u64>()
            && line_number <= i64::MAX as u64
            && let Ok(line_number) = usize::try_from(line_number)
        {
            if line_number == 0 {
                return Err(CsplitError::LineNumberIsZero(argument.to_string()));
            }
            if line_number < previous_line_number {
                return Err(CsplitError::LineNumberSmallerThanPrevious(
                    source,
                    previous_line_number,
                ));
            }
            if line_number == previous_line_number {
                ct_show_warning!(
                    "line number {} is the same as preceding line number",
                    source.locale_quote()
                );
            }
            previous_line_number = line_number;
            // 如果参数是行号，构建相应的行号模式
            csplit_patterns.push(CsplitPattern::UpToLine(
                line_number,
                CsplitExecutePattern::Times(1),
            ));
        } else {
            // 如果参数无法解析为有效的模式，返回错误
            return Err(CsplitError::InvalidPattern(source));
        }

        if let Some(next_item) = iter.peek()
            && next_item.as_ref().starts_with(b"{")
        {
            let repeat_count = parse_repeat_count(next_item.as_ref())?;
            iter.next();
            csplit_patterns
                .last_mut()
                .expect("parsed pattern must be present")
                .set_execute_n_times(repeat_count);
        }
    }
    // 成功解析所有参数后，返回模式向量
    Ok(csplit_patterns)
}

fn parse_repeat_count(argument: impl AsRef<[u8]>) -> Result<CsplitExecutePattern, CsplitError> {
    let argument = argument.as_ref();
    let Some(without_closing_brace) = argument.strip_suffix(b"}") else {
        return Err(CsplitError::RepeatCountMissingClosingBrace(
            PatternText::from(argument.to_owned()),
        ));
    };
    let value = without_closing_brace.strip_prefix(b"{").unwrap_or(argument);
    if value == b"*" {
        return Ok(CsplitExecutePattern::Always);
    }

    let value = value.trim_ascii_start();
    match std::str::from_utf8(&value)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
    {
        Some(repeat_count) if repeat_count <= i64::MAX as u64 => {
            Ok(CsplitExecutePattern::Times(repeat_count + 1))
        }
        _ => Err(CsplitError::RepeatCountIntegerRequired(format!(
            "{}}}",
            PatternText::from(without_closing_brace.to_owned()).locale_quote()
        ))),
    }
}

/// Asserts the line numbers are in increasing order, starting at 1.
/**
 * 验证拆分模式中涉及的行号是否有效。
 *
 * 此函数遍历提供的拆分模式列表，检查行号是否满足以下条件：
 * 1. 行号不能为零。
 * 2. 相邻的行号不能相等。
 * 3. 行号必须按升序出现。
 *
 * @param patterns 指向CsplitPattern结构体 slice的引用，表示待验证的拆分模式。
 * @return Result<(), CsplitError> 如果所有行号都有效，则返回Ok(())；如果发现任何无效行号，则返回Err，携带相应的错误信息。
 */
#[cfg(test)]
fn validate_line_numbers(csplit_patterns: &[CsplitPattern]) -> Result<(), CsplitError> {
    // 过滤并映射出所有以行号为条件的拆分模式，忽略其他类型。
    csplit_patterns
        .iter()
        .filter_map(|pattern| match pattern {
            CsplitPattern::UpToLine(line_number, _) => Some(line_number),
            _ => None,
        })
        // 通过try_fold迭代验证行号，确保它们按照升序且没有重复。
        .try_fold(0, |prev_ln, &current_ln| match (prev_ln, current_ln) {
            // 行号不能为零。
            (_, 0) => Err(CsplitError::LineNumberIsZero("0".to_string())),
            // 相邻的行号不能相等。
            (n, m) if n == m => {
                ct_show_warning!("line number '{}' is the same as preceding line number", n);
                Ok(n)
            }
            // 行号必须按升序出现。
            (n, m) if n > m => Err(CsplitError::LineNumberSmallerThanPrevious(
                m.to_string().into(),
                n,
            )),
            (_, m) => Ok(m),
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bad_pattern() {
        let input = vec!["bad".to_string()];
        assert!(get_patterns(input.as_slice()).is_err());
    }

    #[test]
    fn test_up_to_line_pattern() {
        let input: Vec<String> = vec!["24", "42", "{*}", "50", "{4}"]
            .into_iter()
            .map(|v| v.to_string())
            .collect();
        let patterns = get_patterns(input.as_slice()).unwrap();
        assert_eq!(patterns.len(), 3);
        match patterns.first() {
            Some(CsplitPattern::UpToLine(24, CsplitExecutePattern::Times(1))) => (),
            _ => panic!("expected UpToLine pattern"),
        };
        match patterns.get(1) {
            Some(CsplitPattern::UpToLine(42, CsplitExecutePattern::Always)) => (),
            _ => panic!("expected UpToLine pattern"),
        };
        match patterns.get(2) {
            Some(CsplitPattern::UpToLine(50, CsplitExecutePattern::Times(5))) => (),
            _ => panic!("expected UpToLine pattern"),
        };
    }

    #[test]
    #[allow(clippy::cognitive_complexity)]
    fn test_up_to_match_pattern() {
        let input: Vec<String> = vec![
            "/test1.*end$/",
            "/test2.*end$/",
            "{*}",
            "/test3.*end$/",
            "{4}",
            "/test4.*end$/+3",
            "/test5.*end$/-3",
        ]
        .into_iter()
        .map(|v| v.to_string())
        .collect();
        let patterns = get_patterns(input.as_slice()).unwrap();
        assert_eq!(patterns.len(), 5);
        match patterns.first() {
            Some(CsplitPattern::UpToMatch(reg, 0, CsplitExecutePattern::Times(1))) => {
                let parsed_reg = format!("{reg}");
                assert_eq!(parsed_reg, "test1.*end$");
            }
            _ => panic!("expected UpToMatch pattern"),
        };
        match patterns.get(1) {
            Some(CsplitPattern::UpToMatch(reg, 0, CsplitExecutePattern::Always)) => {
                let parsed_reg = format!("{reg}");
                assert_eq!(parsed_reg, "test2.*end$");
            }
            _ => panic!("expected UpToMatch pattern"),
        };
        match patterns.get(2) {
            Some(CsplitPattern::UpToMatch(reg, 0, CsplitExecutePattern::Times(5))) => {
                let parsed_reg = format!("{reg}");
                assert_eq!(parsed_reg, "test3.*end$");
            }
            _ => panic!("expected UpToMatch pattern"),
        };
        match patterns.get(3) {
            Some(CsplitPattern::UpToMatch(reg, 3, CsplitExecutePattern::Times(1))) => {
                let parsed_reg = format!("{reg}");
                assert_eq!(parsed_reg, "test4.*end$");
            }
            _ => panic!("expected UpToMatch pattern"),
        };
        match patterns.get(4) {
            Some(CsplitPattern::UpToMatch(reg, -3, CsplitExecutePattern::Times(1))) => {
                let parsed_reg = format!("{reg}");
                assert_eq!(parsed_reg, "test5.*end$");
            }
            _ => panic!("expected UpToMatch pattern"),
        };
    }

    #[test]
    #[allow(clippy::cognitive_complexity)]
    fn test_skip_to_match_pattern() {
        let input: Vec<String> = vec![
            "%test1.*end$%",
            "%test2.*end$%",
            "{*}",
            "%test3.*end$%",
            "{4}",
            "%test4.*end$%+3",
            "%test5.*end$%-3",
        ]
        .into_iter()
        .map(|v| v.to_string())
        .collect();
        let patterns = get_patterns(input.as_slice()).unwrap();
        assert_eq!(patterns.len(), 5);
        match patterns.first() {
            Some(CsplitPattern::SkipToMatch(reg, 0, CsplitExecutePattern::Times(1))) => {
                let parsed_reg = format!("{reg}");
                assert_eq!(parsed_reg, "test1.*end$");
            }
            _ => panic!("expected SkipToMatch pattern"),
        };
        match patterns.get(1) {
            Some(CsplitPattern::SkipToMatch(reg, 0, CsplitExecutePattern::Always)) => {
                let parsed_reg = format!("{reg}");
                assert_eq!(parsed_reg, "test2.*end$");
            }
            _ => panic!("expected SkipToMatch pattern"),
        };
        match patterns.get(2) {
            Some(CsplitPattern::SkipToMatch(reg, 0, CsplitExecutePattern::Times(5))) => {
                let parsed_reg = format!("{reg}");
                assert_eq!(parsed_reg, "test3.*end$");
            }
            _ => panic!("expected SkipToMatch pattern"),
        };
        match patterns.get(3) {
            Some(CsplitPattern::SkipToMatch(reg, 3, CsplitExecutePattern::Times(1))) => {
                let parsed_reg = format!("{reg}");
                assert_eq!(parsed_reg, "test4.*end$");
            }
            _ => panic!("expected SkipToMatch pattern"),
        };
        match patterns.get(4) {
            Some(CsplitPattern::SkipToMatch(reg, -3, CsplitExecutePattern::Times(1))) => {
                let parsed_reg = format!("{reg}");
                assert_eq!(parsed_reg, "test5.*end$");
            }
            _ => panic!("expected SkipToMatch pattern"),
        };
    }

    #[test]
    fn test_skip_to_match_pattern_test1() {
        let input: Vec<String> = vec!["%test1.*end$%".parse().unwrap()];
        let patterns = get_patterns(input.as_slice()).unwrap();
        assert_eq!(patterns.len(), 1);
        match patterns.first() {
            Some(CsplitPattern::SkipToMatch(reg, 0, CsplitExecutePattern::Times(1))) => {
                let parsed_reg = format!("{reg}");
                assert_eq!(parsed_reg, "test1.*end$");
            }
            _ => panic!("expected SkipToMatch pattern"),
        };
    }

    #[test]
    fn test_bre_unescaped_gnu_extensions_are_literals() {
        let input = vec!["/a+b/".to_string()];
        let patterns = get_patterns(input.as_slice()).unwrap();
        match patterns.first() {
            Some(CsplitPattern::UpToMatch(reg, 0, CsplitExecutePattern::Times(1))) => {
                assert!(!reg.is_match(b"aaab"));
                assert!(reg.is_match(b"a+b"));
                assert_eq!(format!("{reg}"), "a+b");
            }
            _ => panic!("expected UpToMatch pattern"),
        }
    }

    #[test]
    fn test_bre_escaped_gnu_extensions_remain_operators() {
        let input = vec![r"/a\+b/".to_string()];
        let patterns = get_patterns(input.as_slice()).unwrap();
        match patterns.first() {
            Some(CsplitPattern::UpToMatch(reg, 0, CsplitExecutePattern::Times(1))) => {
                assert!(reg.is_match(b"aaab"));
                assert!(!reg.is_match(b"a+b"));
                assert_eq!(format!("{reg}"), r"a\+b");
            }
            _ => panic!("expected UpToMatch pattern"),
        }
    }

    #[test]
    fn test_skip_to_match_pattern_test2() {
        let input: Vec<String> = vec!["%test2.*end$%".parse().unwrap()];
        let patterns = get_patterns(input.as_slice()).unwrap();
        assert_eq!(patterns.len(), 1);

        // match patterns.first() {
        //     Some(Pattern::SkipToMatch(reg, 0, ExecutePattern::Always)) => {
        //         let parsed_reg = format!("{reg}");
        //         assert_eq!(parsed_reg, "test2.*end$");
        //     }
        //     _ => panic!("expected SkipToMatch pattern"),
        // };
    }
    #[test]
    fn test_skip_to_match_pattern_test3() {
        let input: Vec<String> = vec!["%test3.*end$%".parse().unwrap()];
        let patterns = get_patterns(input.as_slice()).unwrap();
        assert_eq!(patterns.len(), 1);
        // match patterns.first() {
        //     Some(Pattern::SkipToMatch(reg, 0, ExecutePattern::Times(5))) => {
        //         let parsed_reg = format!("{reg}");
        //         assert_eq!(parsed_reg, "test3.*end$");
        //     }
        //     _ => panic!("expected SkipToMatch pattern"),
        // };
    }
    #[test]
    fn test_skip_to_match_pattern_test4() {
        let input: Vec<String> = vec!["%test4.*end$%+3".parse().unwrap()];
        let patterns = get_patterns(input.as_slice()).unwrap();
        assert_eq!(patterns.len(), 1);
        match patterns.first() {
            Some(CsplitPattern::SkipToMatch(reg, 3, CsplitExecutePattern::Times(1))) => {
                let parsed_reg = format!("{reg}");
                assert_eq!(parsed_reg, "test4.*end$");
            }
            _ => panic!("expected SkipToMatch pattern"),
        };
    }

    #[test]
    fn test_skip_to_match_pattern_test5() {
        let input: Vec<String> = vec!["%test5.*end$%-3".to_string()];
        let patterns = get_patterns(input.as_slice()).unwrap();
        assert_eq!(patterns.len(), 1);
        match patterns.first() {
            Some(CsplitPattern::SkipToMatch(reg, -3, CsplitExecutePattern::Times(1))) => {
                let parsed_reg = format!("{reg}");
                assert_eq!(parsed_reg, "test5.*end$");
            }
            _ => panic!("expected SkipToMatch pattern"),
        };
    }

    #[test]
    fn test_invalid_offset_reports_integer_expected() {
        let input = vec!["/test/not-a-number".to_string()];
        assert!(matches!(
            get_patterns(&input),
            Err(CsplitError::IntegerExpectedAfterDelimiter(pattern))
                if pattern == "/test/not-a-number"
        ));
    }

    #[test]
    fn test_offset_uses_intmax_width() {
        let input = vec!["/test/2147483648".to_string()];
        let patterns = get_patterns(&input).unwrap();
        assert!(matches!(
            patterns.first(),
            Some(CsplitPattern::UpToMatch(_, 2_147_483_648, _))
        ));
    }

    #[test]
    fn test_empty_regular_expression_is_valid() {
        let input = vec!["//".to_string()];
        let patterns = get_patterns(&input).unwrap();
        assert!(matches!(
            patterns.first(),
            Some(CsplitPattern::UpToMatch(pattern, 0, _)) if pattern.to_string().is_empty()
        ));
    }

    #[test]
    fn test_line_number_allows_leading_whitespace() {
        let input = vec![" \t+24".to_string()];
        let patterns = get_patterns(&input).unwrap();
        assert!(matches!(
            patterns.first(),
            Some(CsplitPattern::UpToLine(24, _))
        ));
    }

    #[test]
    fn test_missing_closing_delimiter() {
        for input in ["/test", "%test"] {
            let delimiter = input.chars().next().unwrap();
            assert!(matches!(
                get_patterns(&[input.to_string()]),
                Err(CsplitError::ClosingDelimiterMissing(pattern, actual_delimiter))
                    if pattern == input && actual_delimiter == delimiter
            ));
        }
    }

    #[test]
    fn test_repeat_count_diagnostics_and_range() {
        assert!(matches!(
            parse_repeat_count("{2"),
            Err(CsplitError::RepeatCountMissingClosingBrace(value)) if value == "{2"
        ));
        assert!(matches!(
            parse_repeat_count("{x}"),
            Err(CsplitError::RepeatCountIntegerRequired(value))
                if value == format!("{}}}", locale_quote("{x"))
        ));
        assert!(matches!(
            parse_repeat_count("{9223372036854775807}"),
            Ok(CsplitExecutePattern::Times(9_223_372_036_854_775_808))
        ));
        assert!(matches!(
            parse_repeat_count("{9223372036854775808}"),
            Err(CsplitError::RepeatCountIntegerRequired(_))
        ));
    }

    #[test]
    fn test_line_number_zero() {
        let patterns = vec![CsplitPattern::UpToLine(0, CsplitExecutePattern::Times(1))];
        match validate_line_numbers(&patterns) {
            Err(CsplitError::LineNumberIsZero(_)) => (),
            _ => panic!("expected LineNumberIsZero error"),
        }
    }

    #[test]
    fn test_line_number_smaller_than_previous() {
        let input: Vec<String> = vec!["10".to_string(), "5".to_string()];
        match get_patterns(input.as_slice()) {
            Err(CsplitError::LineNumberSmallerThanPrevious(value, 10)) if value == "5" => (),
            _ => panic!("expected LineNumberSmallerThanPrevious error"),
        }
    }

    #[test]
    fn test_line_number_smaller_than_previous_separate() {
        let input: Vec<String> = vec!["10".to_string(), "/20/".to_string(), "5".to_string()];
        match get_patterns(input.as_slice()) {
            Err(CsplitError::LineNumberSmallerThanPrevious(value, 10)) if value == "5" => (),
            _ => panic!("expected LineNumberSmallerThanPrevious error"),
        }
    }

    #[test]
    fn test_line_number_zero_separate() {
        let input: Vec<String> = vec!["10".to_string(), "/20/".to_string(), "0".to_string()];
        match get_patterns(input.as_slice()) {
            Err(CsplitError::LineNumberIsZero(_)) => (),
            _ => panic!("expected LineNumberIsZero error"),
        }
    }
}
