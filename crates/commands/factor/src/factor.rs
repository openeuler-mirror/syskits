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

//! 因式分解的命令行接口

extern crate rust_i18n;
use ctcore::Tool;
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use std::collections::BTreeMap;
#[cfg(target_os = "linux")]
use std::ffi::{CStr, CString, c_char};
use std::ffi::{OsStr, OsString};
use std::io::BufRead;
use std::io::{self, BufReader, Write, stdout};
mod factor_algorithm;
use clap::builder::OsStringValueParser;
use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use ctcore::ct_display::locale_quote_marks;
use ctcore::ct_error::{CTResult, CtSimpleError, FromIo, set_ct_exit_code, strip_errno};
use ctcore::ct_show_error;
pub use factor_algorithm::*;
use num_bigint::BigUint;
use num_prime::nt_funcs::factorize;
use num_traits::{One, ToPrimitive, Zero};
use sys_locale::get_locale;

pub mod miller_rabin;
pub mod numeric;
pub mod rho;
pub mod table;

/// 定义配置标志常量
pub mod factor_flags {
    /// 使用指数表示法标志
    pub const EXPONENTS: &str = "exponents";
    /// 帮助标志
    pub const HELP: &str = "help";
    /// 版本标志
    pub const VERSION: &str = "version";
    /// 要分解的数字参数
    pub const NUMBER: &str = "NUMBER";
}

/// 因式分解命令的配置结构体
struct FactorFlags {
    /// 是否使用指数表示法
    print_exponents: bool,
    /// 要分解的数字列表
    numbers: Vec<OsString>,
}

/// 为 `FactorFlags` 实现默认值
impl Default for FactorFlags {
    fn default() -> Self {
        Self {
            print_exponents: false,
            numbers: Vec::new(),
        }
    }
}

impl FactorFlags {
    /// 从命令行参数创建配置结构体
    fn new(matches: &ArgMatches) -> Self {
        // 布尔标志提取模式
        let print_exponents = matches.get_count(factor_flags::EXPONENTS) > 0;

        // 向量类型参数提取模式
        let numbers = matches
            .get_many::<OsString>(factor_flags::NUMBER)
            .map_or_else(Vec::new, |v| v.cloned().collect());

        // 构建并返回结构体
        Self {
            print_exponents,
            numbers,
        }
    }
}

#[derive(Debug)]
enum ParsedNumber {
    Small(u64),
    Big(BigUint),
}

impl ParsedNumber {}

fn os_str_bytes(value: &OsStr) -> &[u8] {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        value.as_bytes()
    }

    #[cfg(not(unix))]
    value.to_str().map_or(&[], str::as_bytes)
}

fn c_string_token(token: &[u8]) -> &[u8] {
    token
        .iter()
        .position(|&byte| byte == 0)
        .map_or(token, |end| &token[..end])
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactorPower {
    pub prime: String,
    pub exponent: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactorRow {
    pub number: String,
    pub factors: Vec<String>,
    pub factor_powers: Vec<FactorPower>,
    pub print_exponents: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactorSemantic {
    pub rows: Vec<FactorRow>,
    pub classic_text: String,
    pub stderr_text: String,
    pub exit_code: i32,
}

fn parse_number_token(token: &[u8]) -> Option<ParsedNumber> {
    let trimmed = &token[token.iter().take_while(|&&byte| byte == b' ').count()..];
    if trimmed.is_empty() {
        return None;
    }

    let stripped = trimmed.strip_prefix(b"+").unwrap_or(trimmed);
    if stripped.is_empty() || stripped.starts_with(b"-") {
        return None;
    }

    if !stripped.iter().all(u8::is_ascii_digit) {
        return None;
    }

    let value = BigUint::parse_bytes(stripped, 10)?;
    if let Some(n) = value.to_u64() {
        return Some(ParsedNumber::Small(n));
    }

    Some(ParsedNumber::Big(value))
}

fn push_octal_escape(quoted: &mut String, byte: u8) {
    quoted.push_str(&format!("\\{byte:03o}"));
}

fn factor_quote(token: &[u8]) -> String {
    let (left_quote, right_quote) = locale_quote_marks();
    let unicode_locale = right_quote != "'";
    let mut quoted = left_quote;
    let mut index = 0;

    while index < token.len() {
        let byte = token[index];
        match byte {
            b'\x07' => quoted.push_str("\\a"),
            b'\x08' => quoted.push_str("\\b"),
            b'\t' => quoted.push_str("\\t"),
            b'\n' => quoted.push_str("\\n"),
            b'\x0b' => quoted.push_str("\\v"),
            b'\x0c' => quoted.push_str("\\f"),
            b'\r' => quoted.push_str("\\r"),
            b'\\' => quoted.push_str("\\\\"),
            b'\'' if !unicode_locale => quoted.push_str("\\'"),
            b' '..=b'~' => quoted.push(char::from(byte)),
            byte if !unicode_locale || byte.is_ascii() => push_octal_escape(&mut quoted, byte),
            byte => {
                let remaining = &token[index..];
                let valid_length = std::str::from_utf8(remaining)
                    .map_or_else(|error| error.valid_up_to(), str::len);
                if valid_length == 0 {
                    push_octal_escape(&mut quoted, byte);
                } else {
                    let valid = std::str::from_utf8(&remaining[..valid_length])
                        .expect("validated UTF-8 prefix");
                    let character = valid.chars().next().expect("nonempty UTF-8 prefix");
                    quoted.push(character);
                    index += character.len_utf8();
                    continue;
                }
            }
        }
        index += 1;
    }
    quoted.push_str(&right_quote);
    quoted
}

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn setlocale(category: ctcore::libc::c_int, locale: *const c_char) -> *mut c_char;
    fn bindtextdomain(domainname: *const c_char, dirname: *const c_char) -> *mut c_char;
    fn dgettext(domainname: *const c_char, msgid: *const c_char) -> *mut c_char;
}

fn factor_init_locale() {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);

    #[cfg(target_os = "linux")]
    unsafe {
        setlocale(ctcore::libc::LC_ALL, c"".as_ptr());
        bindtextdomain(c"coreutils".as_ptr(), c"/usr/share/locale".as_ptr());
    }
}

fn factor_gettext(message: &str) -> String {
    #[cfg(target_os = "linux")]
    {
        let message_id = CString::new(message).expect("gettext message has no NUL");
        let translated = unsafe { dgettext(c"coreutils".as_ptr(), message_id.as_ptr()) };
        if !translated.is_null() {
            return unsafe { CStr::from_ptr(translated) }
                .to_string_lossy()
                .into_owned();
        }
    }

    message.to_owned()
}

fn invalid_number_message(token: &[u8]) -> String {
    factor_gettext("%s is not a valid positive integer").replacen("%s", &factor_quote(token), 1)
}

fn recognized_long_option(candidate: &str) -> Option<&'static str> {
    if candidate.is_empty() {
        return None;
    }

    let mut matches = ["exponents", "-debug", "help", "version"]
        .iter()
        .copied()
        .filter(|option| option.starts_with(candidate));
    let option = matches.next()?;
    matches.next().is_none().then_some(option)
}

fn is_recognized_long_option(arg: &str) -> bool {
    arg.strip_prefix("--")
        .and_then(recognized_long_option)
        .is_some()
}

fn long_option_with_unexpected_argument(arg: &str) -> Option<&'static str> {
    let (candidate, _) = arg.strip_prefix("--")?.split_once('=')?;
    recognized_long_option(candidate)
}

fn is_developer_debug_option(arg: &str) -> bool {
    arg.strip_prefix("--")
        .and_then(recognized_long_option)
        .is_some_and(|option| option == "-debug")
}

fn filter_developer_debug_options_with_mode(
    args: &[OsString],
    posixly_correct: bool,
) -> (Vec<OsString>, bool) {
    let mut filtered = Vec::with_capacity(args.len());
    let mut parse_options = true;
    let mut dev_debug = false;

    for (index, arg) in args.iter().enumerate() {
        if index == 0 {
            filtered.push(arg.clone());
            continue;
        }

        let arg_str = arg.to_string_lossy();
        if parse_options {
            if arg_str == "--" {
                parse_options = false;
            } else if is_developer_debug_option(&arg_str) {
                dev_debug = true;
                continue;
            } else if posixly_correct && (!arg_str.starts_with('-') || arg_str == "-") {
                parse_options = false;
            }
        }

        filtered.push(arg.clone());
    }

    (filtered, dev_debug)
}

fn factor_option_error(args: &[OsString]) -> Option<String> {
    factor_option_error_with_mode(args, std::env::var_os("POSIXLY_CORRECT").is_some())
}

fn factor_option_error_with_mode(args: &[OsString], posixly_correct: bool) -> Option<String> {
    let mut options_done = false;
    for arg in args.iter().skip(1) {
        let arg_str = arg.to_string_lossy();
        if options_done {
            continue;
        }

        if arg_str == "--" {
            options_done = true;
            continue;
        }

        if arg_str.starts_with("--") {
            if let Some(option) = long_option_with_unexpected_argument(&arg_str) {
                return Some(format!(
                    "factor: option '--{option}' doesn't allow an argument\nTry 'factor --help' for more information.\n"
                ));
            }
            if is_recognized_long_option(&arg_str) {
                continue;
            }
            return Some(format!(
                "factor: unrecognized option '{arg_str}'\nTry 'factor --help' for more information.\n"
            ));
        }

        if arg_str.starts_with('-') && arg_str != "-" {
            if is_exponents_short_option_cluster(&arg_str) || arg_str == "-V" {
                continue;
            }
            let invalid = invalid_short_option_character(&arg_str);
            return Some(format!(
                "factor: invalid option -- '{invalid}'\nTry 'factor --help' for more information.\n"
            ));
        }

        if posixly_correct {
            break;
        }
    }

    None
}

fn is_exponents_short_option_cluster(arg: &str) -> bool {
    arg.starts_with('-') && arg.len() > 1 && arg[1..].chars().all(|c| c == 'h')
}

fn invalid_short_option_character(arg: &str) -> char {
    arg.strip_prefix('-')
        .and_then(|options| options.chars().find(|option| *option != 'h'))
        .unwrap_or('-')
}

fn invalid_short_option_byte(arg: &OsStr) -> u8 {
    #[cfg(unix)]
    use std::os::unix::ffi::OsStrExt;

    #[cfg(unix)]
    let bytes = arg.as_bytes();
    #[cfg(not(unix))]
    let bytes = arg.to_string_lossy();
    #[cfg(not(unix))]
    let bytes = bytes.as_bytes();

    bytes
        .iter()
        .copied()
        .skip(1)
        .find(|option| *option != b'h')
        .unwrap_or(b'-')
}

fn write_invalid_option_diagnostic(
    writer: &mut impl Write,
    utility_name: &str,
    invalid: u8,
) -> io::Result<()> {
    write!(writer, "{utility_name}: invalid option -- '")?;
    writer.write_all(&[invalid])?;
    writeln!(writer, "'")?;
    writeln!(writer, "Try '{utility_name} --help' for more information.")
}

fn factor_row_from_powers(
    number: String,
    factor_powers: Vec<FactorPower>,
    print_exponents: bool,
) -> FactorRow {
    let factors = factor_powers
        .iter()
        .flat_map(|power| {
            std::iter::repeat_n(
                power.prime.clone(),
                usize::try_from(power.exponent).unwrap(),
            )
        })
        .collect();

    FactorRow {
        number,
        factors,
        factor_powers,
        print_exponents,
    }
}

fn factor_row_from_token(token: &[u8], print_exponents: bool) -> Result<FactorRow, String> {
    let token = c_string_token(token);
    let parsed = parse_number_token(token)
        .ok_or_else(|| format!("factor: {}\n", invalid_number_message(token)))?;

    Ok(match parsed {
        ParsedNumber::Small(value) => {
            let factor_powers = factor(value)
                .prime_powers()
                .into_iter()
                .map(|(prime, exponent)| FactorPower {
                    prime: prime.to_string(),
                    exponent: u32::from(exponent),
                })
                .collect();
            factor_row_from_powers(value.to_string(), factor_powers, print_exponents)
        }
        ParsedNumber::Big(value) => {
            let number = value.to_string();
            let factor_powers = if value.is_zero() || value.is_one() {
                Vec::new()
            } else {
                factorize_biguint(&value)
                    .into_iter()
                    .map(|(prime, exponent)| FactorPower {
                        prime: prime.to_string(),
                        exponent: u32::try_from(exponent).expect("exponent fits in u32"),
                    })
                    .collect()
            };
            factor_row_from_powers(number, factor_powers, print_exponents)
        }
    })
}

fn render_factor_row_classic(row: &FactorRow) -> String {
    let mut output = String::new();
    output.push_str(&row.number);
    output.push(':');
    if row.print_exponents {
        for power in &row.factor_powers {
            output.push(' ');
            output.push_str(&power.prime);
            if power.exponent > 1 {
                output.push('^');
                output.push_str(&power.exponent.to_string());
            }
        }
    } else {
        for factor in &row.factors {
            output.push(' ');
            output.push_str(factor);
        }
    }
    output
}

fn factor_semantic_from_tokens<'a>(
    tokens: impl IntoIterator<Item = &'a [u8]>,
    print_exponents: bool,
) -> FactorSemantic {
    let mut rows = Vec::new();
    let mut classic_text = String::new();
    let mut stderr_text = String::new();
    let mut exit_code = 0;

    for token in tokens {
        match factor_row_from_token(token, print_exponents) {
            Ok(row) => {
                classic_text.push_str(&render_factor_row_classic(&row));
                classic_text.push('\n');
                rows.push(row);
            }
            Err(message) => {
                exit_code = 1;
                stderr_text.push_str(&message);
            }
        }
    }

    FactorSemantic {
        rows,
        classic_text,
        stderr_text,
        exit_code,
    }
}

fn stdin_tokens(line: &[u8]) -> impl Iterator<Item = &[u8]> {
    line.split(|byte| matches!(byte, b' ' | b'\t' | b'\n'))
        .filter(|token| !token.is_empty())
}

fn read_error_message(error: &io::Error) -> String {
    format!("error reading input: {}", strip_errno(error))
}

fn closed_stdin_error() -> Option<io::Error> {
    (ctcore::ct_stdin_was_closed() && !ctcore::ct_io::has_injected_stdin())
        .then(|| io::Error::from_raw_os_error(ctcore::libc::EBADF))
}

pub fn factor_native_semantic(args: impl ctcore::Args) -> CTResult<FactorSemantic> {
    factor_init_locale();

    let args_vec: Vec<OsString> = args.into_iter().collect();
    if let Some(stderr_text) = factor_option_error(&args_vec) {
        return Ok(FactorSemantic {
            rows: Vec::new(),
            classic_text: String::new(),
            stderr_text,
            exit_code: 1,
        });
    }

    let (args_vec, _) = filter_developer_debug_options_with_mode(
        &args_vec,
        std::env::var_os("POSIXLY_CORRECT").is_some(),
    );
    let matches = ct_app().try_get_matches_from(&args_vec)?;
    let settings = FactorFlags::new(&matches);

    if settings.numbers.is_empty() {
        if let Some(err) = closed_stdin_error() {
            return Ok(FactorSemantic {
                rows: Vec::new(),
                classic_text: String::new(),
                stderr_text: format!("factor: {}\n", read_error_message(&err)),
                exit_code: 1,
            });
        }

        let mut stdin = BufReader::new(ctcore::ct_io::stdin_reader_box());
        let mut semantic = FactorSemantic {
            rows: Vec::new(),
            classic_text: String::new(),
            stderr_text: String::new(),
            exit_code: 0,
        };
        let mut line = Vec::new();

        loop {
            line.clear();
            match stdin.read_until(b'\n', &mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let line_semantic =
                        factor_semantic_from_tokens(stdin_tokens(&line), settings.print_exponents);
                    semantic.rows.extend(line_semantic.rows);
                    semantic.classic_text.push_str(&line_semantic.classic_text);
                    semantic.stderr_text.push_str(&line_semantic.stderr_text);
                    semantic.exit_code = semantic.exit_code.max(line_semantic.exit_code);
                }
                Err(err) => {
                    semantic.exit_code = 1;
                    semantic
                        .stderr_text
                        .push_str(&format!("factor: {}\n", read_error_message(&err)));
                    break;
                }
            }
        }

        return Ok(semantic);
    }

    Ok(factor_semantic_from_tokens(
        settings
            .numbers
            .iter()
            .map(|number| os_str_bytes(number.as_os_str())),
        settings.print_exponents,
    ))
}

fn format_big_factors(factors: &BTreeMap<BigUint, usize>, print_exponents: bool) -> String {
    let mut output = String::new();
    for (prime, exp) in factors {
        if print_exponents && *exp > 1 {
            output.push(' ');
            output.push_str(&prime.to_string());
            output.push('^');
            output.push_str(&exp.to_string());
        } else {
            let prime_str = prime.to_string();
            for _ in 0..*exp {
                output.push(' ');
                output.push_str(&prime_str);
            }
        }
    }

    output
}

fn factorize_biguint(value: &BigUint) -> BTreeMap<BigUint, usize> {
    let mut n = value.clone();
    let mut factors = BTreeMap::new();
    if n.is_zero() || n.is_one() {
        return factors;
    }

    let one = BigUint::one();
    let two = BigUint::from(2u8);
    let mut count = 0usize;
    while (&n & &one).is_zero() {
        n >>= 1;
        count += 1;
    }
    if count > 0 {
        factors.insert(two, count);
    }

    if n.is_one() {
        return factors;
    }

    let rest = factorize(n);
    for (prime, exp) in rest {
        *factors.entry(prime).or_insert(0) += exp;
    }

    factors
}

fn validate_options(args: &[OsString]) -> CTResult<()> {
    validate_options_with_mode(args, std::env::var_os("POSIXLY_CORRECT").is_some())
}

fn validate_options_with_mode(args: &[OsString], posixly_correct: bool) -> CTResult<()> {
    let mut options_done = false;
    for arg in args.iter().skip(1) {
        let arg_str = arg.to_string_lossy();
        if options_done {
            continue;
        }

        if arg_str == "--" {
            options_done = true;
            continue;
        }

        if arg_str.starts_with("--") {
            if let Some(option) = long_option_with_unexpected_argument(&arg_str) {
                ct_show_error!("option '--{}' doesn't allow an argument", option);
                eprintln!(
                    "Try '{} --help' for more information.",
                    ctcore::ct_util_name()
                );
                return Err(CtSimpleError::new(1, ""));
            }
            if is_recognized_long_option(&arg_str) {
                continue;
            }
            ct_show_error!("unrecognized option '{}'", arg_str);
            eprintln!(
                "Try '{} --help' for more information.",
                ctcore::ct_util_name()
            );
            return Err(CtSimpleError::new(1, ""));
        }

        if arg_str.starts_with('-') && arg_str != "-" {
            if is_exponents_short_option_cluster(&arg_str) || arg_str == "-V" {
                continue;
            }
            let invalid = invalid_short_option_byte(arg.as_os_str());
            let stderr = io::stderr();
            write_invalid_option_diagnostic(&mut stderr.lock(), ctcore::ct_util_name(), invalid)
                .expect("failed to write invalid option diagnostic");
            return Err(CtSimpleError::new(1, ""));
        }

        if posixly_correct {
            break;
        }
    }

    Ok(())
}

/// 处理单个数字的因式分解并输出结果
fn factors_print_bytes(
    token: &[u8],
    w: &mut io::BufWriter<impl io::Write>,
    is_print_exponents: bool,
    dev_debug: bool,
) -> io::Result<()> {
    let token = c_string_token(token);
    let parsed = match parse_number_token(token) {
        Some(parsed) => parsed,
        None => {
            ct_show_error!("{}", invalid_number_message(token));
            set_ct_exit_code(1);
            return Ok(());
        }
    };

    if dev_debug {
        let precision = match &parsed {
            ParsedNumber::Small(_) => "single",
            ParsedNumber::Big(value) if value.bits() < 128 => "single",
            ParsedNumber::Big(_) => "arbitrary",
        };
        eprint!("[using {precision}-precision arithmetic] ");
    }

    match parsed {
        ParsedNumber::Small(x) => {
            if is_print_exponents {
                writeln!(w, "{}:{:#}", x, factor(x))?;
            } else {
                writeln!(w, "{}:{}", x, factor(x))?;
            }
        }
        ParsedNumber::Big(x) => {
            let output = if x.is_zero() || x.is_one() {
                String::new()
            } else {
                let factors = factorize_biguint(&x);
                format_big_factors(&factors, is_print_exponents)
            };
            writeln!(w, "{x}:{output}")?;
        }
    }

    w.flush()
}

/// 处理从标准输入读取的数字
fn process_stdin(
    w: &mut io::BufWriter<impl io::Write>,
    print_exponents: bool,
    dev_debug: bool,
) -> CTResult<()> {
    if let Some(error) = closed_stdin_error() {
        set_ct_exit_code(1);
        ct_show_error!("{}", read_error_message(&error));
        return Ok(());
    }

    let mut stdin = BufReader::new(ctcore::ct_io::stdin_reader_box());
    let mut line = Vec::new();
    loop {
        line.clear();
        match stdin.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(_) => {
                for number in stdin_tokens(&line) {
                    factors_print_bytes(number, w, print_exponents, dev_debug)
                        .map_err_context(|| "write error".into())?;
                }
            }
            Err(e) => {
                set_ct_exit_code(1);
                ct_show_error!("{}", read_error_message(&e));
                return Ok(());
            }
        }
    }
    Ok(())
}

/// 处理命令行参数中提供的数字
fn process_numbers(
    numbers: &[OsString],
    w: &mut io::BufWriter<impl io::Write>,
    print_exponents: bool,
    dev_debug: bool,
) -> CTResult<()> {
    for number in numbers {
        factors_print_bytes(
            os_str_bytes(number.as_os_str()),
            w,
            print_exponents,
            dev_debug,
        )
        .map_err_context(|| "write error".into())?;
    }
    Ok(())
}

#[derive(Default)]
pub struct Factor;
impl Tool for Factor {
    fn name(&self) -> &'static str {
        "factor"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        let stdout = stdout();
        let mut w = io::BufWriter::with_capacity(4 * 1024, stdout.lock());
        factor_main(args.iter().cloned(), &mut w)
    }
}
/// 因式分解命令的主函数
///
/// # Errors
///
/// 返回错误如果：
/// - 命令行参数解析失败
/// - 写入输出时发生 I/O 错误
/// - 处理输入数字时发生错误
pub fn factor_main(args: impl ctcore::Args, w: &mut io::BufWriter<impl io::Write>) -> CTResult<()> {
    factor_init_locale();
    // 1. 解析命令行参数
    let args_vec: Vec<OsString> = args.into_iter().collect();
    validate_options(&args_vec)?;
    let (args_vec, dev_debug) = filter_developer_debug_options_with_mode(
        &args_vec,
        std::env::var_os("POSIXLY_CORRECT").is_some(),
    );
    let matches = ct_app().try_get_matches_from(&args_vec)?;

    // 2. 创建配置对象
    let settings = FactorFlags::new(&matches);

    // 3. 使用配置执行主要逻辑
    if settings.numbers.is_empty() {
        // 处理从标准输入读取的数字
        process_stdin(w, settings.print_exponents, dev_debug)?;
    } else {
        // 处理命令行参数中提供的数字
        process_numbers(&settings.numbers, w, settings.print_exponents, dev_debug)?;
    }

    // 确保所有输出都被刷新
    if let Err(e) = w.flush() {
        ct_show_error!("{}", e);
    }

    Ok(())
}

/// 创建命令行应用程序
#[must_use]
pub fn ct_app() -> Command {
    ct_app_with_mode(std::env::var_os("POSIXLY_CORRECT").is_some())
}

fn ct_app_with_mode(posixly_correct: bool) -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("factor.about");
    let usage_description = t!("factor.usage");
    let args = vec![
        Arg::new(factor_flags::NUMBER)
            .action(ArgAction::Append)
            .value_parser(OsStringValueParser::new())
            .trailing_var_arg(posixly_correct),
        Arg::new(factor_flags::EXPONENTS)
            .short('h')
            .long(factor_flags::EXPONENTS)
            .help(t!("factor.clap.exponents"))
            .action(ArgAction::Count),
        Arg::new(factor_flags::HELP)
            .long(factor_flags::HELP)
            .help(t!("factor.clap.help"))
            .action(ArgAction::Help),
        Arg::new(factor_flags::VERSION)
            .short('V')
            .long(factor_flags::VERSION)
            .help(t!("factor.clap.version"))
            .action(ArgAction::Version),
    ];

    // 构建并配置命令行解析器
    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .args(&args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Factor;

        // Test name method
        assert_eq!(tool.name(), "factor");

        // Test command method
        let command = tool.command();
        assert!(command.get_name().contains("factor"));

        // Test execute method - should work with default arguments
        let args = vec![OsString::from("factor"), OsString::from("--help")];
        assert!(tool.execute(&args).is_err());

        // Test with specific number
        let args = vec![OsString::from("factor"), OsString::from("42")];
        assert!(tool.execute(&args).is_ok());
    }

    /// 测试 FactorFlags 的默认值
    #[test]
    fn test_factor_flags_default() {
        let flags = FactorFlags::default();
        assert!(!flags.print_exponents);
        assert!(flags.numbers.is_empty());
    }

    /// 测试从 ArgMatches 创建 FactorFlags
    #[test]
    fn test_factor_flags_new() {
        // 创建一个带有指数标志的 ArgMatches
        let matches = ct_app().try_get_matches_from(vec!["factor", "-h"]).unwrap();
        let flags = FactorFlags::new(&matches);
        assert!(flags.print_exponents);
        assert!(flags.numbers.is_empty());

        // 创建一个带有数字参数的 ArgMatches
        let matches = ct_app()
            .try_get_matches_from(vec!["factor", "12", "24"])
            .unwrap();
        let flags = FactorFlags::new(&matches);
        assert!(!flags.print_exponents);
        assert_eq!(flags.numbers, vec!["12", "24"]);

        // 创建一个同时带有指数标志和数字参数的 ArgMatches
        let matches = ct_app()
            .try_get_matches_from(vec!["factor", "-h", "12", "24"])
            .unwrap();
        let flags = FactorFlags::new(&matches);
        assert!(flags.print_exponents);
        assert_eq!(flags.numbers, vec!["12", "24"]);

        let matches = ct_app()
            .try_get_matches_from(vec!["factor", "-h", "-h", "360"])
            .unwrap();
        let flags = FactorFlags::new(&matches);
        assert!(flags.print_exponents);
        assert_eq!(flags.numbers, vec!["360"]);

        let matches = ct_app()
            .try_get_matches_from(vec!["factor", "--exponents", "--exponents", "360"])
            .unwrap();
        let flags = FactorFlags::new(&matches);
        assert!(flags.print_exponents);
        assert_eq!(flags.numbers, vec!["360"]);

        let matches = ct_app()
            .try_get_matches_from(vec!["factor", "-hh", "360"])
            .unwrap();
        let flags = FactorFlags::new(&matches);
        assert!(flags.print_exponents);
        assert_eq!(flags.numbers, vec!["360"]);
    }

    #[test]
    fn test_posix_mode_stops_options_at_first_operand() {
        let args = vec![
            OsString::from("factor"),
            OsString::from("12"),
            OsString::from("-h"),
            OsString::from("18"),
        ];

        assert!(factor_option_error_with_mode(&args, true).is_none());
        assert!(validate_options_with_mode(&args, true).is_ok());

        let matches = ct_app_with_mode(true).try_get_matches_from(&args).unwrap();
        let flags = FactorFlags::new(&matches);
        assert!(!flags.print_exponents);
        assert_eq!(flags.numbers, vec!["12", "-h", "18"]);
    }

    #[test]
    fn test_developer_debug_option_filtering() {
        let args = vec![
            OsString::from("factor"),
            OsString::from("12"),
            OsString::from("---d"),
        ];

        assert!(factor_option_error_with_mode(&args, false).is_none());
        let (filtered, dev_debug) = filter_developer_debug_options_with_mode(&args, false);
        assert!(dev_debug);
        assert_eq!(
            filtered,
            vec![OsString::from("factor"), OsString::from("12")]
        );

        let (filtered, dev_debug) = filter_developer_debug_options_with_mode(&args, true);
        assert!(!dev_debug);
        assert_eq!(filtered, args);

        let after_delimiter = vec![
            OsString::from("factor"),
            OsString::from("--"),
            OsString::from("---debug"),
        ];
        let (filtered, dev_debug) =
            filter_developer_debug_options_with_mode(&after_delimiter, false);
        assert!(!dev_debug);
        assert_eq!(filtered, after_delimiter);
    }

    use std::io::BufWriter;

    /// 测试 factors_print_bytes 函数 - 普通格式
    #[test]
    fn test_factors_print_str_normal_format() {
        let buffer = Vec::new();
        let mut writer = BufWriter::new(buffer);

        // 测试数字 12 的因式分解（普通格式）
        factors_print_bytes(b"12", &mut writer, false, false).unwrap();

        // 获取 writer 中的 buffer
        let buffer = writer.into_inner().unwrap();
        let output = String::from_utf8(buffer).unwrap();
        assert_eq!(output, "12: 2 2 3\n");
    }

    /// 测试 factors_print_bytes 函数 - 指数格式
    #[test]
    fn test_factors_print_str_exponent_format() {
        let buffer = Vec::new();
        let mut writer = BufWriter::new(buffer);

        // 测试数字 12 的因式分解（指数格式）
        factors_print_bytes(b"12", &mut writer, true, false).unwrap();

        // 获取 writer 中的 buffer
        let buffer = writer.into_inner().unwrap();
        let output = String::from_utf8(buffer).unwrap();
        assert_eq!(output, "12: 2^2 3\n");
    }

    #[test]
    fn test_operand_skips_only_leading_ascii_spaces() {
        assert!(parse_number_token(b"  12").is_some());
        assert!(parse_number_token(b"12 ").is_none());
        assert!(parse_number_token(b"\t12").is_none());
        assert!(parse_number_token("\u{a0}12".as_bytes()).is_none());
    }

    #[test]
    fn test_invalid_operand_diagnostic_preserves_whitespace() {
        let message = factor_row_from_token(b"abc ", false).unwrap_err();

        assert!(message.contains("abc "));
        assert!(!message.contains("'abc'"));
    }

    #[test]
    fn test_invalid_operand_quoting_escapes_control_characters() {
        let quoted = factor_quote(b"a\n\x01b");

        assert!(quoted.contains("a\\n\\001b"));
        assert!(!quoted.contains('\n'));
        assert!(!quoted.contains('\x01'));
    }

    #[test]
    fn test_stdin_uses_only_gnu_delimiters() {
        let input = "12\r\n15\x0b21\x0c25\u{a0}27\n".as_bytes();

        assert_eq!(
            stdin_tokens(input).collect::<Vec<_>>(),
            vec![b"12\r".as_slice(), "15\x0b21\x0c25\u{a0}27".as_bytes()]
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_read_error_message_omits_rust_errno_suffix() {
        let error = io::Error::from_raw_os_error(21);

        assert_eq!(
            read_error_message(&error),
            "error reading input: Is a directory"
        );
    }

    #[test]
    fn test_non_utf8_stdin_token_is_quoted_and_processing_continues() {
        assert!(parse_number_token(b"\xff").is_none());
        assert!(factor_quote(b"\xff").contains("\\377"));
        assert!(parse_number_token(b"15").is_some());
    }

    #[cfg(unix)]
    #[test]
    fn test_number_arguments_accept_non_utf8_bytes() {
        use std::os::unix::ffi::OsStringExt;

        let invalid = OsString::from_vec(vec![b'1', b'2', 0xff]);
        let matches = ct_app()
            .try_get_matches_from(vec![
                OsString::from("factor"),
                OsString::from("--"),
                invalid.clone(),
                OsString::from("15"),
            ])
            .unwrap();
        let flags = FactorFlags::new(&matches);

        assert_eq!(flags.numbers, vec![invalid, OsString::from("15")]);
    }

    #[test]
    fn test_nul_terminates_stdin_token_like_c_string() {
        let row = factor_row_from_token(b"12\0bad", false).unwrap();

        assert_eq!(row.number, "12");
        assert_eq!(row.factors, vec!["2", "2", "3"]);
    }

    #[test]
    fn test_factor_main_accepts_repeated_exponents_option() {
        let buffer = Vec::new();
        let mut writer = BufWriter::new(buffer);
        let args = vec![
            OsString::from("factor"),
            OsString::from("-h"),
            OsString::from("-h"),
            OsString::from("360"),
        ];

        factor_main(args.into_iter(), &mut writer).unwrap();

        let buffer = writer.into_inner().unwrap();
        let output = String::from_utf8(buffer).unwrap();
        assert_eq!(output, "360: 2^3 3^2 5\n");
    }

    #[test]
    fn test_factor_main_accepts_abbreviated_exponents_option() {
        let buffer = Vec::new();
        let mut writer = BufWriter::new(buffer);
        let args = vec![
            OsString::from("factor"),
            OsString::from("--exp"),
            OsString::from("360"),
        ];

        factor_main(args.into_iter(), &mut writer).unwrap();

        let buffer = writer.into_inner().unwrap();
        let output = String::from_utf8(buffer).unwrap();
        assert_eq!(output, "360: 2^3 3^2 5\n");
    }

    #[test]
    fn test_exponents_option_rejects_attached_argument() {
        let args = vec![OsString::from("factor"), OsString::from("--exp=value")];

        assert_eq!(
            factor_option_error(&args),
            Some(String::from(
                "factor: option '--exponents' doesn't allow an argument\n\
                 Try 'factor --help' for more information.\n"
            ))
        );
    }

    #[test]
    fn test_short_option_cluster_reports_invalid_character() {
        let args = vec![OsString::from("factor"), OsString::from("-hhx")];

        assert_eq!(
            factor_option_error(&args),
            Some(String::from(
                "factor: invalid option -- 'x'\n\
                 Try 'factor --help' for more information.\n"
            ))
        );
    }

    #[test]
    fn test_multibyte_short_option_diagnostic_uses_first_byte() {
        let option = OsString::from("-\u{e9}");
        let invalid = invalid_short_option_byte(option.as_os_str());
        let mut diagnostic = Vec::new();

        write_invalid_option_diagnostic(&mut diagnostic, "factor", invalid).unwrap();

        assert_eq!(invalid, 0xc3);
        assert_eq!(
            diagnostic,
            b"factor: invalid option -- '\xc3'\n\
              Try 'factor --help' for more information.\n"
        );
    }

    #[test]
    fn test_factor_main_accepts_clustered_exponents_option() {
        let buffer = Vec::new();
        let mut writer = BufWriter::new(buffer);
        let args = vec![
            OsString::from("factor"),
            OsString::from("-hh"),
            OsString::from("360"),
        ];

        factor_main(args.into_iter(), &mut writer).unwrap();

        let buffer = writer.into_inner().unwrap();
        let output = String::from_utf8(buffer).unwrap();
        assert_eq!(output, "360: 2^3 3^2 5\n");
    }

    /// 测试 process_numbers 函数
    #[test]
    fn test_process_numbers() {
        let buffer = Vec::new();
        let mut writer = BufWriter::new(buffer);

        // 测试处理多个数字
        let numbers = vec![OsString::from("12"), OsString::from("24")];
        process_numbers(&numbers, &mut writer, false, false).unwrap();

        // 获取 writer 中的 buffer
        let buffer = writer.into_inner().unwrap();
        let output = String::from_utf8(buffer).unwrap();
        assert_eq!(output, "12: 2 2 3\n24: 2 2 2 3\n");
    }

    /// 测试无效输入的处理
    #[test]
    fn test_invalid_input() {
        let buffer = Vec::new();
        let mut writer = BufWriter::new(buffer);

        // 测试无效输入（非数字）
        // 注意：这个测试会产生警告消息，但不会失败
        factors_print_bytes(b"abc", &mut writer, false, false).unwrap();

        // 获取 writer 中的 buffer
        let buffer = writer.into_inner().unwrap();
        let output = String::from_utf8(buffer).unwrap();
        assert_eq!(output, ""); // 无效输入不会产生输出
    }

    /// 测试边界情况
    #[test]
    fn test_edge_cases() {
        let buffer = Vec::new();
        let mut writer = BufWriter::new(buffer);

        // 测试 0
        factors_print_bytes(b"0", &mut writer, false, false).unwrap();

        // 测试 1
        factors_print_bytes(b"1", &mut writer, false, false).unwrap();

        // 获取 writer 中的 buffer
        let buffer = writer.into_inner().unwrap();
        let output = String::from_utf8(buffer).unwrap();
        assert_eq!(output, "0:\n1:\n"); // 0 和 1 都没有素因子
    }

    /// 测试大数
    #[test]
    fn test_large_numbers() {
        let buffer = Vec::new();
        let mut writer = BufWriter::new(buffer);

        // 测试较大的数字
        factors_print_bytes(b"1234567", &mut writer, true, false).unwrap();

        // 获取 writer 中的 buffer
        let buffer = writer.into_inner().unwrap();
        let output = String::from_utf8(buffer).unwrap();
        // 1234567 = 127 * 9721
        assert_eq!(output, "1234567: 127 9721\n");
    }

    /// 测试命令行应用程序配置
    #[test]
    fn test_ct_app_configuration() {
        let app = ct_app();

        // 验证应用程序名称和版本
        assert_eq!(app.get_name(), ctcore::ct_util_name());

        // 验证参数配置
        let args: Vec<_> = app.get_arguments().collect();
        assert!(args.iter().any(|arg| arg.get_id() == factor_flags::NUMBER));
        assert!(
            args.iter()
                .any(|arg| arg.get_id() == factor_flags::EXPONENTS)
        );
        assert!(args.iter().any(|arg| arg.get_id() == factor_flags::HELP));
    }
}
