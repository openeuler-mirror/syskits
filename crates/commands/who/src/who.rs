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

extern crate rust_i18n;
use clap::{Arg, ArgAction, Command, builder::OsStringValueParser, crate_version};
use ctcore::Tool;
use ctcore::ct_error::{CTError, CTResult};

use std::borrow::Cow;
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt::{Display, Formatter};
use std::os::unix::ffi::OsStrExt;
use sys_locale::get_locale;

use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");

mod platform;
pub use platform::{WhoRow, WhoSemantic, who_native_semantic};

mod who_flags {
    pub const WHO_ALL: &str = "all";
    pub const WHO_BOOT: &str = "boot";
    pub const WHO_DEAD: &str = "dead";
    pub const WHO_HEADING: &str = "heading";
    pub const WHO_LOGIN: &str = "login";
    pub const WHO_LOOKUP: &str = "lookup";
    pub const WHO_ONLY_HOSTNAME_USER: &str = "only_hostname_user";
    pub const WHO_PROCESS: &str = "process";
    pub const WHO_COUNT: &str = "count";
    pub const WHO_RUNLEVEL: &str = "runlevel";
    pub const WHO_SHORT: &str = "short";
    pub const WHO_TIME: &str = "time";
    pub const WHO_USERS: &str = "users";
    pub const WHO_MESG: &str = "mesg";
    pub const WHO_FILE: &str = "FILE"; // if length=1: FILE, if length=2: ARG1 ARG2
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("who.about");
    let usage_description = t!("who.usage");
    let args = vec![
        Arg::new(who_flags::WHO_ALL)
            .long(who_flags::WHO_ALL)
            .short('a')
            .help(t!("who.clap.options.all"))
            .action(ArgAction::SetTrue)
            .overrides_with(who_flags::WHO_ALL),
        Arg::new(who_flags::WHO_BOOT)
            .long(who_flags::WHO_BOOT)
            .short('b')
            .help(t!("who.clap.options.boot"))
            .action(ArgAction::SetTrue)
            .overrides_with(who_flags::WHO_BOOT),
        Arg::new(who_flags::WHO_DEAD)
            .long(who_flags::WHO_DEAD)
            .short('d')
            .help(t!("who.clap.options.dead"))
            .action(ArgAction::SetTrue)
            .overrides_with(who_flags::WHO_DEAD),
        Arg::new(who_flags::WHO_HEADING)
            .long(who_flags::WHO_HEADING)
            .short('H')
            .help(t!("who.clap.options.heading"))
            .action(ArgAction::SetTrue)
            .overrides_with(who_flags::WHO_HEADING),
        Arg::new(who_flags::WHO_LOGIN)
            .long(who_flags::WHO_LOGIN)
            .short('l')
            .help(t!("who.clap.options.login"))
            .action(ArgAction::SetTrue)
            .overrides_with(who_flags::WHO_LOGIN),
        Arg::new(who_flags::WHO_LOOKUP)
            .long(who_flags::WHO_LOOKUP)
            .help(t!("who.clap.options.lookup"))
            .action(ArgAction::SetTrue)
            .overrides_with(who_flags::WHO_LOOKUP),
        Arg::new(who_flags::WHO_ONLY_HOSTNAME_USER)
            .short('m')
            .help(t!("who.clap.options.only_hostname_user"))
            .action(ArgAction::SetTrue)
            .overrides_with(who_flags::WHO_ONLY_HOSTNAME_USER),
        Arg::new(who_flags::WHO_PROCESS)
            .long(who_flags::WHO_PROCESS)
            .short('p')
            .help(t!("who.clap.options.process"))
            .action(ArgAction::SetTrue)
            .overrides_with(who_flags::WHO_PROCESS),
        Arg::new(who_flags::WHO_COUNT)
            .long(who_flags::WHO_COUNT)
            .short('q')
            .help(t!("who.clap.options.count"))
            .action(ArgAction::SetTrue)
            .overrides_with(who_flags::WHO_COUNT),
        Arg::new(who_flags::WHO_RUNLEVEL)
            .long(who_flags::WHO_RUNLEVEL)
            .short('r')
            .help(t!("who.clap.options.runlevel"))
            .action(ArgAction::SetTrue)
            .overrides_with(who_flags::WHO_RUNLEVEL),
        Arg::new(who_flags::WHO_SHORT)
            .long(who_flags::WHO_SHORT)
            .short('s')
            .help(t!("who.clap.options.short"))
            .action(ArgAction::SetTrue)
            .overrides_with(who_flags::WHO_SHORT),
        Arg::new(who_flags::WHO_TIME)
            .long(who_flags::WHO_TIME)
            .short('t')
            .help(t!("who.clap.options.time"))
            .action(ArgAction::SetTrue)
            .overrides_with(who_flags::WHO_TIME),
        Arg::new(who_flags::WHO_USERS)
            .long(who_flags::WHO_USERS)
            .short('u')
            .help(t!("who.clap.options.users"))
            .action(ArgAction::SetTrue)
            .overrides_with(who_flags::WHO_USERS),
        Arg::new(who_flags::WHO_MESG)
            .long(who_flags::WHO_MESG)
            .short('T')
            .visible_short_alias('w')
            .visible_aliases(["message", "writable"])
            .help(t!("who.clap.options.mesg"))
            .action(ArgAction::SetTrue)
            .overrides_with(who_flags::WHO_MESG),
        Arg::new(who_flags::WHO_FILE)
            .num_args(1..=2)
            .value_parser(OsStringValueParser::new())
            .value_hint(clap::ValueHint::FilePath)
            .help(t!("who.clap.options.file")),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .arg(
            Arg::new("help")
                .short('h')
                .long("help")
                .help(t!("who.clap.help"))
                .action(ArgAction::Help),
        )
        .arg(
            Arg::new("version")
                .short('V')
                .long("version")
                .help(t!("who.clap.version"))
                .action(ArgAction::Version),
        )
        .args(&args)
}

pub(crate) fn ct_app_for_parse(posixly_correct: bool) -> Command {
    ct_app().trailing_var_arg(posixly_correct)
}

pub(crate) fn prepare_who_args(args: impl ctcore::Args) -> CTResult<Vec<OsString>> {
    let args = args.collect::<Vec<_>>();
    let posixly_correct = std::env::var_os("POSIXLY_CORRECT").is_some();
    let mut parse_options = true;
    let mut operand_count = 0;
    let mut options = Vec::new();
    let mut operands = Vec::new();
    let mut saw_option_terminator = false;
    let mut terminal_option = false;

    for argument in args.iter().skip(1) {
        let bytes = argument.as_encoded_bytes();
        if parse_options && bytes == b"--" {
            parse_options = false;
            saw_option_terminator = true;
            continue;
        }
        if parse_options && bytes.len() > 1 && bytes[0] == b'-' {
            validate_option(bytes)?;
            options.push(argument.clone());
            if is_terminal_option(bytes) {
                terminal_option = true;
                break;
            }
            continue;
        }

        operand_count += 1;
        operands.push(argument.clone());
        if posixly_correct {
            parse_options = false;
        }
    }

    if operand_count > 2 && !terminal_option {
        let mut message = t!("who.errors.extra_operand").as_bytes().to_vec();
        message.push(b' ');
        message.extend(quote_locale_operand(operands[2].as_os_str()));
        return Err(WhoUsageError::boxed(message));
    }

    if posixly_correct {
        return Ok(args);
    }

    let mut prepared = Vec::with_capacity(args.len());
    prepared.extend(args.first().cloned());
    prepared.extend(options);
    if saw_option_terminator {
        prepared.push(OsString::from("--"));
    }
    prepared.extend(operands);
    Ok(prepared)
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum WhoLongOptionKind {
    All,
    Boot,
    Count,
    Dead,
    Heading,
    Help,
    Login,
    Lookup,
    Mesg,
    Process,
    Runlevel,
    Short,
    Time,
    Users,
    Version,
}

const WHO_LONG_OPTIONS: [(&str, WhoLongOptionKind); 17] = [
    ("all", WhoLongOptionKind::All),
    ("boot", WhoLongOptionKind::Boot),
    ("count", WhoLongOptionKind::Count),
    ("dead", WhoLongOptionKind::Dead),
    ("heading", WhoLongOptionKind::Heading),
    ("login", WhoLongOptionKind::Login),
    ("lookup", WhoLongOptionKind::Lookup),
    ("message", WhoLongOptionKind::Mesg),
    ("mesg", WhoLongOptionKind::Mesg),
    ("process", WhoLongOptionKind::Process),
    ("runlevel", WhoLongOptionKind::Runlevel),
    ("short", WhoLongOptionKind::Short),
    ("time", WhoLongOptionKind::Time),
    ("users", WhoLongOptionKind::Users),
    ("writable", WhoLongOptionKind::Mesg),
    ("help", WhoLongOptionKind::Help),
    ("version", WhoLongOptionKind::Version),
];

enum LongOptionMatch {
    None,
    Recognized(&'static str),
    Ambiguous(Vec<&'static str>),
}

fn match_long_option(name: &[u8]) -> LongOptionMatch {
    if let Some((option, _)) = WHO_LONG_OPTIONS
        .iter()
        .find(|(option, _)| option.as_bytes() == name)
    {
        return LongOptionMatch::Recognized(option);
    }

    let matches = WHO_LONG_OPTIONS
        .iter()
        .filter(|(option, _)| option.as_bytes().starts_with(name))
        .collect::<Vec<_>>();
    let Some((first_name, first_kind)) = matches.first().copied() else {
        return LongOptionMatch::None;
    };
    if matches.iter().all(|(_, kind)| kind == first_kind) {
        LongOptionMatch::Recognized(first_name)
    } else {
        LongOptionMatch::Ambiguous(matches.into_iter().map(|(name, _)| *name).collect())
    }
}

const WHO_SHORT_OPTIONS: &[u8] = b"abdlmpqrstuwHThV";

#[derive(Debug)]
struct WhoUsageError {
    message: Vec<u8>,
    usage_hint: Vec<u8>,
}

fn locale_name() -> String {
    for variable in ["LC_ALL", "LC_CTYPE", "LANG"] {
        let Some(locale) = std::env::var_os(variable) else {
            continue;
        };
        if locale.is_empty() {
            continue;
        }
        return locale.to_string_lossy().into_owned();
    }
    "C".to_string()
}

fn quote_locale_operand(operand: &OsStr) -> Vec<u8> {
    let locale = locale_name().to_ascii_uppercase();
    let is_utf8 = locale.contains("UTF-8") || locale.contains("UTF8");
    if !is_utf8 {
        return quote_c_locale_operand(operand);
    }

    if locale.starts_with("ZH_") || locale.starts_with("ZH-") {
        quote_utf8_locale_operand(operand, b"\"", b"\"", Some(b'"'))
    } else {
        quote_utf8_locale_operand(operand, "‘".as_bytes(), "’".as_bytes(), None)
    }
}

fn quote_c_locale_operand(operand: &OsStr) -> Vec<u8> {
    let mut quoted = Vec::with_capacity(operand.as_bytes().len() + 2);
    quoted.push(b'\'');
    for byte in operand.as_bytes() {
        match *byte {
            b'\x07' => quoted.extend_from_slice(b"\\a"),
            b'\x08' => quoted.extend_from_slice(b"\\b"),
            b'\t' => quoted.extend_from_slice(b"\\t"),
            b'\n' => quoted.extend_from_slice(b"\\n"),
            b'\x0b' => quoted.extend_from_slice(b"\\v"),
            b'\x0c' => quoted.extend_from_slice(b"\\f"),
            b'\r' => quoted.extend_from_slice(b"\\r"),
            b'\\' => quoted.extend_from_slice(b"\\\\"),
            b'\'' => quoted.extend_from_slice(b"\\'"),
            b' '..=b'~' => quoted.push(*byte),
            _ => {
                quoted.push(b'\\');
                quoted.push(b'0' + (byte >> 6));
                quoted.push(b'0' + ((byte >> 3) & 7));
                quoted.push(b'0' + (byte & 7));
            }
        }
    }
    quoted.push(b'\'');
    quoted
}

fn quote_utf8_locale_operand(
    operand: &OsStr,
    left_quote: &[u8],
    right_quote: &[u8],
    quote_to_escape: Option<u8>,
) -> Vec<u8> {
    let input = operand.as_bytes();
    let mut quoted = Vec::with_capacity(input.len() + left_quote.len() + right_quote.len());
    quoted.extend_from_slice(left_quote);

    let mut index = 0;
    while index < input.len() {
        let byte = input[index];
        if byte.is_ascii() {
            push_quoted_ascii(&mut quoted, byte, quote_to_escape);
            index += 1;
            continue;
        }

        match std::str::from_utf8(&input[index..]) {
            Ok(_) => {
                quoted.extend_from_slice(&input[index..]);
                break;
            }
            Err(error) if error.valid_up_to() > 0 => {
                let end = index + error.valid_up_to();
                quoted.extend_from_slice(&input[index..end]);
                index = end;
            }
            Err(error) => {
                let invalid_length = error.error_len().unwrap_or(input.len() - index);
                for invalid in &input[index..index + invalid_length] {
                    push_octal_escape(&mut quoted, *invalid);
                }
                index += invalid_length;
            }
        }
    }

    quoted.extend_from_slice(right_quote);
    quoted
}

fn push_quoted_ascii(output: &mut Vec<u8>, byte: u8, quote_to_escape: Option<u8>) {
    match byte {
        b'\x07' => output.extend_from_slice(b"\\a"),
        b'\x08' => output.extend_from_slice(b"\\b"),
        b'\t' => output.extend_from_slice(b"\\t"),
        b'\n' => output.extend_from_slice(b"\\n"),
        b'\x0b' => output.extend_from_slice(b"\\v"),
        b'\x0c' => output.extend_from_slice(b"\\f"),
        b'\r' => output.extend_from_slice(b"\\r"),
        b'\\' => output.extend_from_slice(b"\\\\"),
        escaped if quote_to_escape == Some(escaped) => {
            output.push(b'\\');
            output.push(escaped);
        }
        b' '..=b'~' => output.push(byte),
        _ => push_octal_escape(output, byte),
    }
}

fn push_octal_escape(output: &mut Vec<u8>, byte: u8) {
    output.push(b'\\');
    output.push(b'0' + (byte >> 6));
    output.push(b'0' + ((byte >> 3) & 7));
    output.push(b'0' + (byte & 7));
}

impl WhoUsageError {
    fn boxed(message: Vec<u8>) -> Box<dyn CTError> {
        Box::new(Self {
            message,
            usage_hint: t!("who.errors.try_help").as_bytes().to_vec(),
        })
    }
}

impl Display for WhoUsageError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        String::from_utf8_lossy(&self.message).fmt(formatter)
    }
}

impl Error for WhoUsageError {}

impl CTError for WhoUsageError {
    fn diagnostic_bytes(&self) -> Cow<'_, [u8]> {
        Cow::Borrowed(&self.message)
    }

    fn usage_hint_bytes(&self) -> Option<Cow<'_, [u8]>> {
        Some(Cow::Borrowed(&self.usage_hint))
    }

    fn usage(&self) -> bool {
        true
    }
}

fn is_terminal_option(argument: &[u8]) -> bool {
    if let Some(long) = argument.strip_prefix(b"--") {
        let name = &long[..long
            .iter()
            .position(|byte| *byte == b'=')
            .unwrap_or(long.len())];
        return matches!(
            match_long_option(name),
            LongOptionMatch::Recognized("help" | "version")
        );
    }

    argument[1..]
        .iter()
        .any(|option| matches!(option, b'h' | b'V'))
}

fn validate_option(argument: &[u8]) -> CTResult<()> {
    if argument.starts_with(b"--") {
        validate_long_option(argument)
    } else if let Some(unknown) = argument[1..]
        .iter()
        .find(|option| !WHO_SHORT_OPTIONS.contains(option))
    {
        let mut message = b"invalid option -- '".to_vec();
        message.push(*unknown);
        message.push(b'\'');
        Err(WhoUsageError::boxed(message))
    } else {
        Ok(())
    }
}

fn validate_long_option(argument: &[u8]) -> CTResult<()> {
    let Some(long) = argument.strip_prefix(b"--") else {
        return Ok(());
    };
    let separator = long.iter().position(|byte| *byte == b'=');
    let name = &long[..separator.unwrap_or(long.len())];

    match match_long_option(name) {
        LongOptionMatch::Recognized(canonical) if separator.is_some() => Err(WhoUsageError::boxed(
            format!("option '--{canonical}' doesn't allow an argument").into_bytes(),
        )),
        LongOptionMatch::Ambiguous(matches) => {
            let argument = String::from_utf8_lossy(argument);
            let possibilities = matches
                .into_iter()
                .map(|option| format!("'--{option}'"))
                .collect::<Vec<_>>()
                .join(" ");
            Err(WhoUsageError::boxed(
                format!("option '{argument}' is ambiguous; possibilities: {possibilities}")
                    .into_bytes(),
            ))
        }
        LongOptionMatch::None => {
            let mut message = b"unrecognized option '".to_vec();
            message.extend_from_slice(argument);
            message.push(b'\'');
            Err(WhoUsageError::boxed(message))
        }
        LongOptionMatch::Recognized(_) => Ok(()),
    }
}

#[derive(Default)]
pub struct Who;
impl Tool for Who {
    fn name(&self) -> &'static str {
        "who"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        // Set locale based on system settings
        let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
        rust_i18n::set_locale(&lang_code);

        platform::who_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use clap::error::ErrorKind;
    use ctcore::Tool;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    use super::*;

    static LOCALE_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct LocaleRestore(Option<OsString>);

    struct I18nRestore(String);

    impl Drop for LocaleRestore {
        fn drop(&mut self) {
            // SAFETY: LOCALE_ENV_LOCK remains held while this guard restores LC_ALL.
            unsafe {
                match self.0.take() {
                    Some(value) => std::env::set_var("LC_ALL", value),
                    None => std::env::remove_var("LC_ALL"),
                }
            }
        }
    }

    impl Drop for I18nRestore {
        fn drop(&mut self) {
            rust_i18n::set_locale(&self.0);
        }
    }

    fn with_lc_all<T>(locale: &str, test: impl FnOnce() -> T) -> T {
        let _lock = LOCALE_ENV_LOCK.lock().unwrap();
        let restore = LocaleRestore(std::env::var_os("LC_ALL"));
        // SAFETY: LOCALE_ENV_LOCK serializes test mutations and LocaleRestore restores LC_ALL.
        unsafe { std::env::set_var("LC_ALL", locale) };
        let result = test();
        drop(restore);
        result
    }

    fn with_i18n_locale<T>(locale: &str, test: impl FnOnce() -> T) -> T {
        let restore = I18nRestore(rust_i18n::locale().to_string());
        rust_i18n::set_locale(locale);
        let result = test();
        drop(restore);
        result
    }

    #[test]
    fn file_operand_accepts_non_utf8_paths() {
        let path = OsString::from_vec(vec![b'u', b't', b'm', b'p', b'-', 0xff]);
        let matches = ct_app()
            .try_get_matches_from([OsString::from(ctcore::ct_util_name()), path.clone()])
            .unwrap();
        assert_eq!(
            matches
                .get_one::<OsString>(who_flags::WHO_FILE)
                .map(OsString::as_os_str),
            Some(path.as_os_str())
        );
    }

    #[test]
    fn posix_parsing_stops_at_the_first_operand() {
        let matches = ct_app_for_parse(true)
            .try_get_matches_from(["who", "utmp", "-q"])
            .unwrap();
        assert!(!matches.get_flag(who_flags::WHO_COUNT));
        assert_eq!(
            matches
                .get_many::<OsString>(who_flags::WHO_FILE)
                .unwrap()
                .map(OsString::as_os_str)
                .collect::<Vec<_>>(),
            [std::ffi::OsStr::new("utmp"), std::ffi::OsStr::new("-q")]
        );
    }

    #[test]
    fn default_parsing_permutes_options_before_operands() {
        let args = ["who", "a", "-H", "b"].map(OsString::from);
        let prepared = prepare_who_args(args.into_iter()).unwrap();
        let matches = ct_app_for_parse(false)
            .try_get_matches_from(prepared)
            .unwrap();
        assert!(matches.get_flag(who_flags::WHO_HEADING));
        assert_eq!(
            matches
                .get_many::<OsString>(who_flags::WHO_FILE)
                .unwrap()
                .map(OsString::as_os_str)
                .collect::<Vec<_>>(),
            [std::ffi::OsStr::new("a"), std::ffi::OsStr::new("b")]
        );
    }

    #[test]
    fn third_operand_uses_the_gnu_extra_operand_diagnostic() {
        with_lc_all("C", || {
            with_i18n_locale("en-US", || {
                let args = ["who", "a", "b", "c"].map(OsString::from);
                let error = prepare_who_args(args.into_iter()).unwrap_err();
                assert_eq!(error.to_string(), "extra operand 'c'");
                assert!(error.usage());
            });
        });
    }

    #[test]
    fn non_utf8_extra_operand_uses_gnu_locale_quoting() {
        let operand = OsString::from_vec(vec![0xff]);
        assert_eq!(quote_c_locale_operand(&operand), b"'\\377'");
        assert_eq!(
            quote_c_locale_operand(OsStr::new("a'b\\c\n")),
            b"'a\\'b\\\\c\\n'"
        );
    }

    #[test]
    fn utf8_extra_operand_uses_gnu_locale_quoting() {
        with_lc_all("C.UTF-8", || {
            let ascii_args = ["who", "a", "b", "c"].map(OsString::from);
            let ascii_error = prepare_who_args(ascii_args.into_iter()).unwrap_err();
            assert_eq!(
                ascii_error.diagnostic_bytes().as_ref(),
                "extra operand ‘c’".as_bytes()
            );

            let raw_args = [
                OsString::from("who"),
                OsString::from("a"),
                OsString::from("b"),
                OsString::from_vec(vec![0xff]),
            ];
            let raw_error = prepare_who_args(raw_args.into_iter()).unwrap_err();
            assert_eq!(
                raw_error.diagnostic_bytes().as_ref(),
                "extra operand ‘\\377’".as_bytes()
            );
        });

        with_lc_all("zh_CN.UTF-8", || {
            assert_eq!(quote_locale_operand(OsStr::new("c")), b"\"c\"");
        });
    }

    #[test]
    fn chinese_usage_errors_match_gnu_diagnostics() {
        with_lc_all("zh_CN.UTF-8", || {
            with_i18n_locale("zh-CN", || {
                let extra_args = ["who", "a", "b", "c"].map(OsString::from);
                let extra_error = prepare_who_args(extra_args.into_iter()).unwrap_err();
                assert_eq!(
                    extra_error.diagnostic_bytes().as_ref(),
                    "多余的操作对象 \"c\"".as_bytes()
                );
                assert_eq!(
                    extra_error.usage_hint_bytes().as_deref(),
                    Some("请尝试执行 \"who --help\" 来获取更多信息。".as_bytes())
                );

                let option_args = ["who", "-z"].map(OsString::from);
                let option_error = prepare_who_args(option_args.into_iter()).unwrap_err();
                assert_eq!(option_error.to_string(), "invalid option -- 'z'");
                assert_eq!(
                    option_error.usage_hint_bytes().as_deref(),
                    Some("请尝试执行 \"who --help\" 来获取更多信息。".as_bytes())
                );
            });
        });
    }

    #[test]
    fn option_error_after_third_operand_takes_precedence() {
        let args = ["who", "a", "b", "c", "-z"].map(OsString::from);
        let error = prepare_who_args(args.into_iter()).unwrap_err();
        assert_eq!(error.to_string(), "invalid option -- 'z'");
        assert!(error.usage());
    }

    #[test]
    fn terminal_option_after_operands_takes_precedence() {
        let args = ["who", "a", "b", "c", "--version"].map(OsString::from);
        let prepared = prepare_who_args(args.into_iter()).unwrap();
        assert_eq!(prepared[1], OsString::from("--version"));
    }

    #[test]
    fn attached_value_reports_the_canonical_no_argument_option() {
        let args = ["who", "--bo=value"].map(OsString::from);
        let error = prepare_who_args(args.into_iter()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "option '--boot' doesn't allow an argument"
        );
        assert!(error.usage());
    }

    #[test]
    fn ambiguous_long_option_reports_all_distinct_possibilities() {
        let args = ["who", "--l"].map(OsString::from);
        let error = prepare_who_args(args.into_iter()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "option '--l' is ambiguous; possibilities: '--login' '--lookup'"
        );
        assert!(error.usage());
    }

    #[test]
    fn empty_long_option_name_is_ambiguous_with_all_options() {
        let args = ["who", "--="].map(OsString::from);
        let error = prepare_who_args(args.into_iter()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "option '--=' is ambiguous; possibilities: '--all' '--boot' '--count' '--dead' '--heading' '--login' '--lookup' '--message' '--mesg' '--process' '--runlevel' '--short' '--time' '--users' '--writable' '--help' '--version'"
        );
        assert!(error.usage());
    }

    #[test]
    fn unknown_long_option_uses_the_gnu_diagnostic() {
        let args = ["who", "--not-an-option"].map(OsString::from);
        let error = prepare_who_args(args.into_iter()).unwrap_err();
        assert_eq!(error.to_string(), "unrecognized option '--not-an-option'");
        assert!(error.usage());
    }

    #[test]
    fn unknown_short_option_uses_the_gnu_diagnostic() {
        let args = ["who", "-az"].map(OsString::from);
        let error = prepare_who_args(args.into_iter()).unwrap_err();
        assert_eq!(error.to_string(), "invalid option -- 'z'");
        assert!(error.usage());
    }

    #[test]
    fn unknown_non_utf8_short_option_preserves_the_original_byte() {
        let args = [OsString::from("who"), OsString::from_vec(vec![b'-', 0xff])];
        let error = prepare_who_args(args.into_iter()).unwrap_err();
        assert_eq!(
            error.diagnostic_bytes().as_ref(),
            b"invalid option -- '\xff'"
        );
        assert!(error.usage());
    }

    #[test]
    fn unknown_non_utf8_long_option_preserves_the_original_byte() {
        let args = [
            OsString::from("who"),
            OsString::from_vec(b"--bad\xff".to_vec()),
        ];
        let error = prepare_who_args(args.into_iter()).unwrap_err();
        assert_eq!(
            error.diagnostic_bytes().as_ref(),
            b"unrecognized option '--bad\xff'"
        );
        assert!(error.usage());
    }

    #[test]
    fn test_tool_implementation() {
        let tool = Who;

        // 测试 name 方法
        assert_eq!(tool.name(), "who");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("who"));

        // 测试 execute 方法
        let args = vec![OsString::from("who"), OsString::from("--version")];
        assert!(tool.execute(&args).is_err());
    }

    // who 接口: who [OPTION]... [ FILE | ARG1 ARG2 ]
    //   -a, --all         same as -b -d --login -p -r -t -T -u
    //   -b, --boot        time of last system boot
    //   -d, --dead        print dead processes
    //   -H, --heading     print line of column headings
    //   -l, --login       print system login processes
    //       --lookup      attempt to canonicalize hostnames via DNS
    //   -m                only hostname and user associated with stdin
    //   -p, --process     print active processes spawned by init
    //   -q, --count       all login names and number of users logged on
    //   -r, --runlevel    print current runlevel
    //   -s, --short       print only name, line, and time (default)
    //   -t, --time        print last system clock change
    //   -T, -w, --mesg    add user's message status as +, - or ?
    //   -u, --users       list users logged in
    //       --message     same as -T
    //       --writable    same as -T
    //       --help     display this help and exit
    #[test]
    fn test_ct_app_execution_version() {
        let command = ct_app();

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), "--version"];

        // Assuming `command` has a method to retrieve the executable name, replace it with the actual one
        let executable = command.try_get_matches_from(args);

        assert!(executable.is_err());
        assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
    }

    #[test]
    fn test_ct_app_execution_other_version() {
        let command = ct_app();

        // 测试用例1：有效输入
        let args = vec![ctcore::ct_util_name(), "-V"];

        // Assuming `command` has a method to retrieve the executable name, replace it with the actual one
        let executable = command.try_get_matches_from(args);

        assert!(executable.is_err());
        assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
    }

    #[test]
    fn test_ct_app_execution_help() {
        let command = ct_app();

        // 测试用例2：验证 --help 参数是否正确处理
        let help_args = vec![ctcore::ct_util_name(), "--help"];
        let result = command.try_get_matches_from(help_args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
    }

    #[test]
    fn test_ct_app_execution_unsupport_help() {
        let command = ct_app();

        // 测试用例2：验证 --help 参数是否正确处理
        let help_args = vec![ctcore::ct_util_name(), "-H"];
        let result = command.try_get_matches_from(help_args);
        assert!(result.is_ok());
        // assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn test_ct_app_invalid_argument() {
        let command = ct_app();

        // 测试用例3：验证当提供未知参数时是否正确报错
        let invalid_args = vec![ctcore::ct_util_name(), "--invalid-argument"];
        let result = command.try_get_matches_from(invalid_args);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn test_ct_app_support_missing_argument() {
        let command = ct_app();

        // 测试用例4：验证当缺少必需的参数时是否正确报错
        let missing_args = vec![ctcore::ct_util_name()]; // 缺少任何参数
        let result = command.try_get_matches_from(missing_args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_all() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--all"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_boot() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--boot"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_dead() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--dead"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_heading() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--heading"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_login() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--login"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_lookup() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--lookup"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_process() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--process"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_count() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--count"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_runlevel() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--runlevel"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_short() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--short"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_time() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--time"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_users() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--users"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_long_option_mesg() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--mesg"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_repeated_mesg_aliases() {
        let command = ct_app();
        let args = vec![
            ctcore::ct_util_name(),
            "-T",
            "-w",
            "--message",
            "--writable",
            "utmp",
        ];

        let matches = command.try_get_matches_from(args).unwrap();

        assert!(matches.get_flag(who_flags::WHO_MESG));
    }

    #[test]
    fn test_ct_app_repeated_boolean_options_are_idempotent() {
        let repeated_options = [
            ["-a", "--all"],
            ["-b", "--boot"],
            ["-d", "--dead"],
            ["-H", "--heading"],
            ["-l", "--login"],
            ["--lookup", "--lookup"],
            ["-m", "-m"],
            ["-p", "--process"],
            ["-q", "--count"],
            ["-r", "--runlevel"],
            ["-s", "--short"],
            ["-t", "--time"],
            ["-u", "--users"],
        ];

        for options in repeated_options {
            let matches = ct_app().try_get_matches_from([
                ctcore::ct_util_name(),
                options[0],
                options[1],
                "utmp",
            ]);
            assert!(
                matches.is_ok(),
                "repeated options {options:?} were rejected"
            );
        }
    }

    #[test]
    fn test_ct_app_long_option_file() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--file"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_err());
        assert_eq!(executable.unwrap_err().kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn test_ct_app_long_option_file2() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "--file"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_err());
        assert_eq!(executable.unwrap_err().kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn test_ct_app_short_option_a() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-a"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_b() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-b"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_d() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-d"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_uppercase_h() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-H"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_l() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-l"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_m() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-m"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_p() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-p"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_q() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-q"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_r() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-r"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_s() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-s"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_t() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-t"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_u() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-u"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_uppercase_t() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-T"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }

    #[test]
    fn test_ct_app_short_option_w() {
        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), "-w"];
        let executable = command.try_get_matches_from(args);
        assert!(executable.is_ok());
    }
}
