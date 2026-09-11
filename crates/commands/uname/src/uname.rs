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

//! uname 是一个 Linux 系统命令，用于显示系统的基本信息。
//! 它提供了关于操作系统的内核名称、版本、主机名、硬件平台（体系结构）和操作系统发行版等信息。

extern crate rust_i18n;
use clap::{Arg, ArgAction, Command, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::ct_error::{CTError, CTResult, strip_errno};
use ctcore::libc::{SIG_DFL, SIG_ERR, SIG_IGN, SIGPIPE, getppid, sighandler_t, signal};
use platform_info::*;

use ctcore::Tool;
use std::borrow::Cow;
use std::error::Error;
use std::ffi::{CStr, CString, OsStr, OsString};
use std::fmt::{Display, Formatter};
use std::io::Write;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(target_os = "linux")]
static INHERITED_SIGPIPE_HANDLER: AtomicUsize = AtomicUsize::new(SIG_ERR);

#[cfg(target_os = "linux")]
#[used]
#[unsafe(link_section = ".init_array")]
static CAPTURE_INHERITED_SIGPIPE: unsafe extern "C" fn() = capture_inherited_sigpipe;

#[cfg(target_os = "linux")]
unsafe extern "C" fn capture_inherited_sigpipe() {
    let mut action = std::mem::MaybeUninit::<ctcore::libc::sigaction>::uninit();
    if unsafe { ctcore::libc::sigaction(SIGPIPE, std::ptr::null(), action.as_mut_ptr()) } == 0 {
        let action = unsafe { action.assume_init() };
        INHERITED_SIGPIPE_HANDLER.store(action.sa_sigaction, Ordering::Relaxed);
    }
}

pub mod uname_flags {
    pub static UNAME_ALL: &str = "all";
    pub static UNAME_KERNEL_NAME: &str = "kernel-name";
    pub static UNAME_NODE_NAME: &str = "nodename";
    pub static UNAME_KERNEL_VERSION: &str = "kernel-version";
    pub static UNAME_KERNEL_RELEASE: &str = "kernel-release";
    pub static UNAME_MACHINE: &str = "machine";
    pub static UNAME_PROCESSOR: &str = "processor";
    pub static UNAME_HARDWARE_PLATFORM: &str = "hardware-platform";
    pub static UNAME_OS: &str = "operating-system";
}

pub struct UNameOutput {
    pub kernel_name: Option<OsString>,
    pub node_name: Option<OsString>,
    pub kernel_release: Option<OsString>,
    pub kernel_version: Option<OsString>,
    pub machine: Option<OsString>,
    pub os: Option<OsString>,
    pub processor: Option<OsString>,
    pub hardware_platform: Option<OsString>,
}

impl UNameOutput {
    fn display_bytes(&self) -> Vec<u8> {
        let mut output = Vec::new();
        let mut names = [
            self.kernel_name.as_ref(),
            self.node_name.as_ref(),
            self.kernel_release.as_ref(),
            self.kernel_version.as_ref(),
            self.machine.as_ref(),
            self.processor.as_ref(),
            self.hardware_platform.as_ref(),
            self.os.as_ref(),
        ]
        .into_iter()
        .flatten();
        if let Some(name) = names.next() {
            output.extend_from_slice(name.as_bytes());
        }
        for name in names {
            output.push(b' ');
            output.extend_from_slice(name.as_bytes());
        }
        output
    }

    pub fn new(opts: &UnameFlags) -> CTResult<Self> {
        Self::new_with_platform_provider(opts, || PlatformInfo::new().map_err(platform_info_error))
    }

    fn new_with_platform_provider(
        opts: &UnameFlags,
        platform_provider: impl FnOnce() -> CTResult<PlatformInfo>,
    ) -> CTResult<Self> {
        let is_none = !(opts.is_all
            || opts.is_kernel_name
            || opts.is_node_name
            || opts.is_kernel_release
            || opts.is_kernel_version
            || opts.is_machine
            || opts.is_os
            || opts.is_processor
            || opts.is_hardware_platform);
        let requires_platform_info = is_none
            || opts.is_all
            || opts.is_kernel_name
            || opts.is_node_name
            || opts.is_kernel_release
            || opts.is_kernel_version
            || opts.is_machine
            || cfg!(not(target_os = "linux")) && opts.is_os;
        let queries_platform_info =
            requires_platform_info || opts.is_processor || opts.is_hardware_platform;
        let platform_info = if queries_platform_info {
            match platform_provider() {
                Ok(platform_info) => Some(platform_info),
                Err(error) if requires_platform_info => return Err(error),
                Err(_) => None,
            }
        } else {
            None
        };
        let uname = platform_info.as_ref();

        let kernel_name = (opts.is_kernel_name || opts.is_all || is_none).then(|| {
            uname
                .expect("platform info is required")
                .sysname()
                .to_os_string()
        });

        let node_name = (opts.is_node_name || opts.is_all).then(|| {
            uname
                .expect("platform info is required")
                .nodename()
                .to_os_string()
        });

        let kernel_release = (opts.is_kernel_release || opts.is_all).then(|| {
            uname
                .expect("platform info is required")
                .release()
                .to_os_string()
        });

        let kernel_version = (opts.is_kernel_version || opts.is_all).then(|| {
            uname
                .expect("platform info is required")
                .version()
                .to_os_string()
        });

        let machine = (opts.is_machine || opts.is_all).then(|| {
            uname
                .expect("platform info is required")
                .machine()
                .to_os_string()
        });

        let processor = (opts.is_processor || opts.is_all).then(|| {
            uname
                .map(|uname| uname.machine().to_os_string())
                .unwrap_or_default()
        });

        let hardware_platform = (opts.is_hardware_platform || opts.is_all).then(|| {
            uname
                .map(|uname| hardware_platform_from_machine(uname.machine()))
                .unwrap_or_default()
        });

        let os = (opts.is_os || opts.is_all).then(|| operating_system_name(uname));

        Ok(Self {
            kernel_name,
            node_name,
            kernel_release,
            kernel_version,
            machine,
            processor,
            hardware_platform,
            os,
        })
    }
}

fn platform_info_error(error: PlatformInfoError) -> Box<dyn CTError> {
    let context = t!("uname.errors.cannot_get_system_name");
    match error.downcast::<std::io::Error>() {
        Ok(error) => localized_io_error(&context, &error),
        Err(error) => localized_runtime_error(&context, &error.to_string()),
    }
}

#[derive(Debug)]
struct UnameRuntimeError {
    diagnostic: Vec<u8>,
}

impl Display for UnameRuntimeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        String::from_utf8_lossy(&self.diagnostic).fmt(formatter)
    }
}

impl Error for UnameRuntimeError {}

impl CTError for UnameRuntimeError {
    fn diagnostic_bytes(&self) -> Cow<'_, [u8]> {
        Cow::Borrowed(&self.diagnostic)
    }
}

fn io_error_diagnostic(context: &[u8], error: &std::io::Error) -> Vec<u8> {
    let mut diagnostic = Vec::with_capacity(context.len() + 2 + error.to_string().len());
    diagnostic.extend_from_slice(context);
    diagnostic.extend_from_slice(b": ");
    diagnostic.extend_from_slice(strip_errno(error).as_bytes());
    diagnostic
}

fn localized_io_error(context: &str, error: &std::io::Error) -> Box<dyn CTError> {
    Box::new(UnameRuntimeError {
        diagnostic: io_error_diagnostic(&locale_text_bytes(context), error),
    })
}

fn localized_runtime_error(context: &str, detail: &str) -> Box<dyn CTError> {
    let mut diagnostic = locale_text_bytes(context);
    diagnostic.extend_from_slice(b": ");
    diagnostic.extend_from_slice(detail.as_bytes());
    Box::new(UnameRuntimeError { diagnostic })
}

#[cfg(target_os = "linux")]
fn operating_system_name(_platform_info: Option<&PlatformInfo>) -> OsString {
    OsString::from("GNU/Linux")
}

#[cfg(not(target_os = "linux"))]
fn operating_system_name(platform_info: Option<&PlatformInfo>) -> OsString {
    platform_info
        .expect("platform info is required")
        .osname()
        .to_os_string()
}

fn hardware_platform_from_machine(machine: &OsStr) -> OsString {
    let bytes = machine.as_bytes();
    if bytes.len() == 4 && bytes[0] == b'i' && bytes[2] == b'8' && bytes[3] == b'6' {
        let mut normalized = bytes.to_vec();
        normalized[1] = b'3';
        OsString::from_vec(normalized)
    } else {
        machine.to_os_string()
    }
}

pub struct UnameFlags {
    pub is_all: bool,
    pub is_kernel_name: bool,
    pub is_node_name: bool,
    pub is_kernel_version: bool,
    pub is_kernel_release: bool,
    pub is_machine: bool,
    pub is_processor: bool,
    pub is_hardware_platform: bool,
    pub is_os: bool,
}

#[derive(Default)]
pub struct Uname;
impl Tool for Uname {
    fn name(&self) -> &'static str {
        "uname"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        uname_main(args.iter().cloned())
    }
}

pub fn uname_main(args: impl ctcore::Args) -> CTResult<()> {
    let _sigpipe_guard = SigpipeGuard::for_cli();

    rust_i18n::set_locale(message_locale().rust_i18n_name());
    let matches = ct_app().try_get_matches_from(prepare_uname_args(args)?)?;

    let flags = UnameFlags {
        is_all: matches.get_flag(uname_flags::UNAME_ALL),
        is_kernel_name: matches.get_flag(uname_flags::UNAME_KERNEL_NAME),
        is_node_name: matches.get_flag(uname_flags::UNAME_NODE_NAME),
        is_kernel_release: matches.get_flag(uname_flags::UNAME_KERNEL_RELEASE),
        is_kernel_version: matches.get_flag(uname_flags::UNAME_KERNEL_VERSION),
        is_machine: matches.get_flag(uname_flags::UNAME_MACHINE),
        is_processor: matches.get_flag(uname_flags::UNAME_PROCESSOR),
        is_hardware_platform: matches.get_flag(uname_flags::UNAME_HARDWARE_PLATFORM),
        is_os: matches.get_flag(uname_flags::UNAME_OS),
    };
    let output = UNameOutput::new(&flags)?;
    let mut rendered = output.display_bytes();
    rendered.push(b'\n');
    std::io::stdout()
        .lock()
        .write_all(&rendered)
        .map_err(|error| localized_io_error(&t!("uname.errors.write_error"), &error))
}

struct SigpipeGuard {
    previous: sighandler_t,
}

impl SigpipeGuard {
    fn for_cli() -> Option<Self> {
        let target = sigpipe_restore_target(inherited_sigpipe_handler(), parent_ignores_sigpipe());
        if target == SIG_IGN {
            return None;
        }

        let previous = unsafe { signal(SIGPIPE, target) };
        (previous != SIG_ERR).then_some(Self { previous })
    }
}

impl Drop for SigpipeGuard {
    fn drop(&mut self) {
        unsafe {
            signal(SIGPIPE, self.previous);
        }
    }
}

#[cfg(target_os = "linux")]
fn inherited_sigpipe_handler() -> sighandler_t {
    INHERITED_SIGPIPE_HANDLER.load(Ordering::Relaxed)
}

#[cfg(not(target_os = "linux"))]
fn inherited_sigpipe_handler() -> sighandler_t {
    SIG_ERR
}

fn sigpipe_restore_target(captured: sighandler_t, parent_ignored: bool) -> sighandler_t {
    if captured != SIG_ERR {
        captured
    } else if parent_ignored {
        SIG_IGN
    } else {
        SIG_DFL
    }
}

fn parent_ignores_sigpipe() -> bool {
    let parent = unsafe { getppid() };
    let Ok(status) = std::fs::read_to_string(format!("/proc/{parent}/status")) else {
        return false;
    };
    sigpipe_is_ignored_in_status(&status)
}

fn sigpipe_is_ignored_in_status(status: &str) -> bool {
    let Some(mask) = status
        .lines()
        .find_map(|line| line.strip_prefix("SigIgn:\t"))
        .and_then(|mask| u64::from_str_radix(mask, 16).ok())
    else {
        return false;
    };
    mask & (1_u64 << (SIGPIPE - 1)) != 0
}

fn prepare_uname_args(args: impl ctcore::Args) -> CTResult<Vec<OsString>> {
    prepare_uname_args_with_mode(args, std::env::var_os("POSIXLY_CORRECT").is_some())
}

fn prepare_uname_args_with_mode(
    args: impl ctcore::Args,
    posixly_correct: bool,
) -> CTResult<Vec<OsString>> {
    let args = args.collect::<Vec<_>>();
    let mut parse_options = true;
    let mut options = Vec::new();
    let mut operands = Vec::new();

    for argument in args.iter().skip(1) {
        let bytes = argument.as_encoded_bytes();
        if parse_options && bytes == b"--" {
            parse_options = false;
            continue;
        }
        if parse_options && bytes.len() > 1 && bytes[0] == b'-' {
            validate_attached_value(bytes)?;
            validate_ambiguous_long_option(bytes)?;
            validate_unknown_option(bytes)?;
            options.push(argument.clone());
            if is_terminal_option(bytes) {
                let mut prepared = Vec::with_capacity(args.len());
                prepared.extend(args.first().cloned());
                prepared.extend(options);
                prepared.extend(operands);
                return Ok(prepared);
            }
            continue;
        }

        operands.push(argument.clone());
        if posixly_correct {
            parse_options = false;
        }
    }

    if let Some(operand) = operands.first() {
        let mut message = locale_text_bytes(&t!("uname.errors.extra_operand"));
        message.push(b' ');
        message.extend(quote_locale_operand(operand));
        return Err(UnameUsageError::boxed(message));
    }

    Ok(args)
}

const UNAME_LONG_OPTIONS: &[&str] = &[
    "all",
    "kernel-name",
    "sysname",
    "nodename",
    "kernel-release",
    "release",
    "kernel-version",
    "machine",
    "processor",
    "hardware-platform",
    "operating-system",
    "help",
    "version",
];

enum LongOptionMatch {
    None,
    Recognized(&'static str),
    Ambiguous(Vec<&'static str>),
}

fn match_long_option(name: &[u8]) -> LongOptionMatch {
    if let Some(option) = UNAME_LONG_OPTIONS
        .iter()
        .find(|option| option.as_bytes() == name)
    {
        return LongOptionMatch::Recognized(option);
    }

    let matches = UNAME_LONG_OPTIONS
        .iter()
        .copied()
        .filter(|option| option.as_bytes().starts_with(name))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => LongOptionMatch::None,
        [option] => LongOptionMatch::Recognized(option),
        _ => LongOptionMatch::Ambiguous(matches),
    }
}

const UNAME_SHORT_OPTIONS: &[u8] = b"asnrvmpiohV";

fn validate_unknown_option(argument: &[u8]) -> CTResult<()> {
    if let Some(long) = argument.strip_prefix(b"--") {
        let name = &long[..long
            .iter()
            .position(|byte| *byte == b'=')
            .unwrap_or(long.len())];
        if matches!(match_long_option(name), LongOptionMatch::None) {
            let mut message = b"unrecognized option '".to_vec();
            message.extend_from_slice(argument);
            message.push(b'\'');
            return Err(UnameUsageError::boxed(message));
        }
        return Ok(());
    }

    if let Some(unknown) = argument[1..]
        .iter()
        .find(|option| !UNAME_SHORT_OPTIONS.contains(option))
    {
        let mut message = b"invalid option -- '".to_vec();
        message.push(*unknown);
        message.push(b'\'');
        return Err(UnameUsageError::boxed(message));
    }
    Ok(())
}

fn validate_ambiguous_long_option(argument: &[u8]) -> CTResult<()> {
    let Some(long) = argument.strip_prefix(b"--") else {
        return Ok(());
    };
    let name = &long[..long
        .iter()
        .position(|byte| *byte == b'=')
        .unwrap_or(long.len())];

    let LongOptionMatch::Ambiguous(matches) = match_long_option(name) else {
        return Ok(());
    };
    let possibilities = matches
        .into_iter()
        .map(|option| format!("'--{option}'"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut message = b"option '".to_vec();
    message.extend_from_slice(argument);
    message.extend_from_slice(b"' is ambiguous; possibilities: ");
    message.extend_from_slice(possibilities.as_bytes());
    Err(UnameUsageError::boxed(message))
}

fn validate_attached_value(argument: &[u8]) -> CTResult<()> {
    let Some(long) = argument.strip_prefix(b"--") else {
        return Ok(());
    };
    let Some(separator) = long.iter().position(|byte| *byte == b'=') else {
        return Ok(());
    };
    let name = &long[..separator];

    match match_long_option(name) {
        LongOptionMatch::Recognized(canonical) => Err(UnameUsageError::boxed(
            format!("option '--{canonical}' doesn't allow an argument").into_bytes(),
        )),
        LongOptionMatch::Ambiguous(matches) => {
            let _ = matches.len();
            Ok(())
        }
        LongOptionMatch::None => Ok(()),
    }
}

#[derive(Debug)]
struct UnameUsageError {
    message: Vec<u8>,
    usage_hint: Vec<u8>,
}

impl UnameUsageError {
    fn boxed(message: Vec<u8>) -> Box<dyn CTError> {
        Box::new(Self {
            message,
            usage_hint: locale_text_bytes(&t!("uname.errors.try_help")),
        })
    }
}

impl Display for UnameUsageError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        String::from_utf8_lossy(&self.message).fmt(formatter)
    }
}

impl Error for UnameUsageError {}

impl CTError for UnameUsageError {
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

fn locale_name() -> String {
    if !environment_locale_is_valid() {
        return "C".to_string();
    }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MessageLocale {
    EnUs,
    ZhCn,
}

impl MessageLocale {
    fn rust_i18n_name(self) -> &'static str {
        match self {
            Self::EnUs => "en-US",
            Self::ZhCn => "zh-CN",
        }
    }
}

fn message_locale() -> MessageLocale {
    if !environment_locale_is_valid() {
        return MessageLocale::EnUs;
    }
    message_locale_from_values(
        std::env::var("LC_ALL").ok().as_deref(),
        std::env::var("LC_MESSAGES").ok().as_deref(),
        std::env::var("LANG").ok().as_deref(),
        std::env::var("LANGUAGE").ok().as_deref(),
    )
}

fn environment_locale_is_valid() -> bool {
    let locale = unsafe {
        ctcore::libc::newlocale(
            ctcore::libc::LC_ALL_MASK,
            c"".as_ptr(),
            std::ptr::null_mut(),
        )
    };
    if locale.is_null() {
        return false;
    }
    unsafe { ctcore::libc::freelocale(locale) };
    true
}

fn message_locale_from_values(
    lc_all: Option<&str>,
    lc_messages: Option<&str>,
    lang: Option<&str>,
    language: Option<&str>,
) -> MessageLocale {
    let base = [lc_all, lc_messages, lang]
        .into_iter()
        .flatten()
        .find(|locale| !locale.is_empty())
        .unwrap_or("C");

    if base.eq_ignore_ascii_case("C") || base.eq_ignore_ascii_case("POSIX") {
        return MessageLocale::EnUs;
    }

    if let Some(locale) = language
        .filter(|value| !value.is_empty())
        .and_then(|value| value.split(':').find_map(known_message_locale))
    {
        return locale;
    }

    known_message_locale(base).unwrap_or(MessageLocale::EnUs)
}

fn known_message_locale(locale: &str) -> Option<MessageLocale> {
    let language = locale
        .split(['.', '@'])
        .next()
        .unwrap_or(locale)
        .replace('-', "_");
    if language.eq_ignore_ascii_case("C")
        || language.eq_ignore_ascii_case("POSIX")
        || language.eq_ignore_ascii_case("en")
        || language.to_ascii_lowercase().starts_with("en_")
    {
        Some(MessageLocale::EnUs)
    } else if language.eq_ignore_ascii_case("zh")
        || language.to_ascii_lowercase().starts_with("zh_cn")
    {
        Some(MessageLocale::ZhCn)
    } else {
        None
    }
}

fn locale_codeset() -> Option<CString> {
    let locale_name = CString::new(locale_name()).ok()?;
    let locale = unsafe {
        ctcore::libc::newlocale(
            ctcore::libc::LC_CTYPE_MASK,
            locale_name.as_ptr(),
            std::ptr::null_mut(),
        )
    };
    if locale.is_null() {
        return None;
    }

    unsafe {
        let codeset = ctcore::libc::nl_langinfo_l(ctcore::libc::CODESET, locale);
        let result = (!codeset.is_null())
            .then(|| CStr::from_ptr(codeset).to_bytes())
            .and_then(|bytes| CString::new(bytes).ok());
        ctcore::libc::freelocale(locale);
        result
    }
}

fn transcode_utf8(text: &str, codeset: &CStr) -> Option<Vec<u8>> {
    let descriptor = unsafe { ctcore::libc::iconv_open(codeset.as_ptr(), c"UTF-8".as_ptr()) };
    if descriptor as usize == usize::MAX {
        return None;
    }

    let mut input = text.as_ptr().cast_mut().cast();
    let mut input_left = text.len();
    let mut output = vec![0_u8; text.len().saturating_mul(4).max(32)];
    let mut output_used = 0;
    let converted = loop {
        let mut output_pointer = unsafe { output.as_mut_ptr().add(output_used) }.cast();
        let mut output_left = output.len() - output_used;
        let result = unsafe {
            ctcore::libc::iconv(
                descriptor,
                &mut input,
                &mut input_left,
                &mut output_pointer,
                &mut output_left,
            )
        };
        output_used = output.len() - output_left;
        if result != usize::MAX {
            output.truncate(output_used);
            break Some(output);
        }
        if std::io::Error::last_os_error().raw_os_error() != Some(ctcore::libc::E2BIG) {
            break None;
        }
        output.resize(output.len().saturating_mul(2), 0);
    };

    unsafe { ctcore::libc::iconv_close(descriptor) };
    converted
}

fn locale_text_bytes(text: &str) -> Vec<u8> {
    if let Some(codeset) = locale_codeset() {
        let normalized = codeset.to_string_lossy().to_ascii_uppercase();
        if normalized == "UTF-8" || normalized == "UTF8" {
            return text.as_bytes().to_vec();
        }
        if let Some(converted) = transcode_utf8(text, &codeset) {
            return converted;
        }
    }

    text.as_bytes().to_vec()
}

fn normalized_locale_codeset() -> Option<String> {
    locale_codeset().map(|codeset| codeset.to_string_lossy().to_ascii_uppercase())
}

struct LocaleCtype {
    raw: ctcore::libc::locale_t,
}

impl LocaleCtype {
    fn from_environment() -> Option<Self> {
        let name = CString::new(locale_name()).ok()?;
        Self::from_name(&name)
    }

    fn from_name(name: &CStr) -> Option<Self> {
        let raw = unsafe {
            ctcore::libc::newlocale(
                ctcore::libc::LC_CTYPE_MASK,
                name.as_ptr(),
                std::ptr::null_mut(),
            )
        };
        (!raw.is_null()).then(|| Self { raw })
    }

    fn activate(&self) -> Option<LocaleCtypeGuard> {
        let previous = unsafe { ctcore::libc::uselocale(self.raw) };
        (!previous.is_null()).then_some(LocaleCtypeGuard { previous })
    }

    fn classify_character(&self, bytes: &[u8]) -> (usize, bool) {
        let mut state: ctcore::libc::mbstate_t = unsafe { std::mem::zeroed() };
        let mut wide: ctcore::libc::wchar_t = 0;
        let length = unsafe { mbrtowc(&mut wide, bytes.as_ptr().cast(), bytes.len(), &mut state) };
        if length == usize::MAX || length == usize::MAX - 1 {
            return (1, false);
        }

        let length = if length == 0 { 1 } else { length };
        let printable = unsafe { iswprint_l(wide as ctcore::libc::c_uint, self.raw) != 0 };
        (length, printable)
    }
}

impl Drop for LocaleCtype {
    fn drop(&mut self) {
        unsafe { ctcore::libc::freelocale(self.raw) };
    }
}

struct LocaleCtypeGuard {
    previous: ctcore::libc::locale_t,
}

impl Drop for LocaleCtypeGuard {
    fn drop(&mut self) {
        unsafe { ctcore::libc::uselocale(self.previous) };
    }
}

unsafe extern "C" {
    fn mbrtowc(
        wide: *mut ctcore::libc::wchar_t,
        bytes: *const ctcore::libc::c_char,
        length: usize,
        state: *mut ctcore::libc::mbstate_t,
    ) -> usize;
    fn iswprint_l(
        character: ctcore::libc::c_uint,
        locale: ctcore::libc::locale_t,
    ) -> ctcore::libc::c_int;
}

fn quote_locale_operand(operand: &OsStr) -> Vec<u8> {
    let codeset = normalized_locale_codeset();
    let (left_quote, right_quote, quote_to_escape) =
        quote_marks_for_codeset(message_locale(), codeset.as_deref());

    LocaleCtype::from_environment().map_or_else(
        || quote_utf8_locale_operand(operand, left_quote, right_quote, quote_to_escape),
        |locale| {
            quote_encoded_locale_operand(operand, left_quote, right_quote, quote_to_escape, &locale)
        },
    )
}

fn quote_marks_for_codeset(
    message_locale: MessageLocale,
    codeset: Option<&str>,
) -> (&'static [u8], &'static [u8], Option<u8>) {
    if message_locale == MessageLocale::ZhCn {
        return (b"\"", b"\"", Some(b'\"'));
    }

    let normalized = codeset.map(str::to_ascii_uppercase);
    match normalized.as_deref() {
        Some("UTF-8" | "UTF8") => ("‘".as_bytes(), "’".as_bytes(), None),
        Some("GB18030") => (b"\xa1\x07e", b"\xa1\xaf", None),
        _ => (b"'", b"'", Some(b'\'')),
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
        if !right_quote.is_empty() && input[index..].starts_with(right_quote) {
            quoted.push(b'\\');
            quoted.extend_from_slice(right_quote);
            index += right_quote.len();
            continue;
        }
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

fn quote_encoded_locale_operand(
    operand: &OsStr,
    left_quote: &[u8],
    right_quote: &[u8],
    quote_to_escape: Option<u8>,
    locale: &LocaleCtype,
) -> Vec<u8> {
    let Some(_locale_guard) = locale.activate() else {
        return quote_c_locale_operand(operand);
    };
    let input = operand.as_bytes();
    let mut quoted = Vec::with_capacity(input.len() + left_quote.len() + right_quote.len());
    quoted.extend_from_slice(left_quote);

    let mut index = 0;
    while index < input.len() {
        if !right_quote.is_empty() && input[index..].starts_with(right_quote) {
            quoted.push(b'\\');
            quoted.extend_from_slice(right_quote);
            index += right_quote.len();
            continue;
        }
        let byte = input[index];
        if byte.is_ascii() {
            push_quoted_ascii(&mut quoted, byte, quote_to_escape);
            index += 1;
            continue;
        }

        let (length, printable) = locale.classify_character(&input[index..]);
        let end = index.saturating_add(length).min(input.len());
        if printable {
            quoted.extend_from_slice(&input[index..end]);
        } else {
            for byte in &input[index..end] {
                push_octal_escape(&mut quoted, *byte);
            }
        }
        index = end;
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

fn is_terminal_option(argument: &[u8]) -> bool {
    if let Some(long) = argument.strip_prefix(b"--") {
        return b"help".starts_with(long) || b"version".starts_with(long);
    }

    argument[1..]
        .iter()
        .any(|option| matches!(option, b'h' | b'V'))
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("uname.about");
    let usage_description = t!("uname.usage");
    let args = [
        Arg::new(uname_flags::UNAME_ALL)
            .short('a')
            .long(uname_flags::UNAME_ALL)
            .help(t!("uname.clap.uname_all"))
            .action(ArgAction::SetTrue),
        Arg::new(uname_flags::UNAME_KERNEL_NAME)
            .short('s')
            .long(uname_flags::UNAME_KERNEL_NAME)
            .alias("sysname") // Obsolescent option in GNU uname
            .help("print the kernel name.")
            .action(ArgAction::SetTrue),
        Arg::new(uname_flags::UNAME_NODE_NAME)
            .short('n')
            .long(uname_flags::UNAME_NODE_NAME)
            .help(
                "print the nodename (the nodename may be a name that the system \
                is known by to a communications network).",
            )
            .action(ArgAction::SetTrue),
        Arg::new(uname_flags::UNAME_KERNEL_RELEASE)
            .short('r')
            .long(uname_flags::UNAME_KERNEL_RELEASE)
            .alias("release") // Obsolescent option in GNU uname
            .help("print the operating system release.")
            .action(ArgAction::SetTrue),
        Arg::new(uname_flags::UNAME_KERNEL_VERSION)
            .short('v')
            .long(uname_flags::UNAME_KERNEL_VERSION)
            .help(t!("uname.clap.uname_kernel_version"))
            .action(ArgAction::SetTrue),
        Arg::new(uname_flags::UNAME_MACHINE)
            .short('m')
            .long(uname_flags::UNAME_MACHINE)
            .help(t!("uname.clap.uname_machine"))
            .action(ArgAction::SetTrue),
        Arg::new(uname_flags::UNAME_OS)
            .short('o')
            .long(uname_flags::UNAME_OS)
            .help(t!("uname.clap.uname_os"))
            .action(ArgAction::SetTrue),
        Arg::new(uname_flags::UNAME_PROCESSOR)
            .short('p')
            .long(uname_flags::UNAME_PROCESSOR)
            .help(t!("uname.clap.uname_processor"))
            .action(ArgAction::SetTrue),
        Arg::new(uname_flags::UNAME_HARDWARE_PLATFORM)
            .short('i')
            .long(uname_flags::UNAME_HARDWARE_PLATFORM)
            .help(t!("uname.clap.uname_hardware_platform"))
            .action(ArgAction::SetTrue),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args_override_self(true)
        .args(args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    #[test]
    fn extra_operand_uses_gnu_diagnostic() {
        let error =
            prepare_uname_args(["uname", "extra"].map(OsString::from).into_iter()).unwrap_err();

        assert!(error.to_string().starts_with("extra operand "));
        assert!(error.usage());
    }

    #[test]
    fn non_utf8_extra_operand_preserves_original_bytes() {
        assert_eq!(
            quote_c_locale_operand(&OsString::from_vec(vec![0xff])),
            b"'\\377'"
        );
    }

    #[test]
    fn attached_value_reports_canonical_long_option() {
        let error = prepare_uname_args(["uname", "--oper=value"].map(OsString::from).into_iter())
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "option '--operating-system' doesn't allow an argument"
        );
    }

    #[test]
    fn ambiguous_long_option_lists_all_possibilities() {
        let error =
            prepare_uname_args(["uname", "--kernel"].map(OsString::from).into_iter()).unwrap_err();

        assert_eq!(
            error.to_string(),
            "option '--kernel' is ambiguous; possibilities: '--kernel-name' '--kernel-release' '--kernel-version'"
        );
    }

    #[test]
    fn ambiguous_long_option_preserves_non_utf8_bytes() {
        let error = prepare_uname_args(
            [
                OsString::from("uname"),
                OsString::from_vec(b"--kernel=\xff".to_vec()),
            ]
            .into_iter(),
        )
        .unwrap_err();

        assert_eq!(
            error.diagnostic_bytes().as_ref(),
            b"option '--kernel=\xff' is ambiguous; possibilities: '--kernel-name' '--kernel-release' '--kernel-version'"
        );
    }

    #[test]
    fn unknown_options_use_gnu_diagnostics() {
        let short =
            prepare_uname_args(["uname", "-z"].map(OsString::from).into_iter()).unwrap_err();
        assert_eq!(short.to_string(), "invalid option -- 'z'");

        let long = prepare_uname_args(
            ["uname", "--does-not-exist"]
                .map(OsString::from)
                .into_iter(),
        )
        .unwrap_err();
        assert_eq!(long.to_string(), "unrecognized option '--does-not-exist'");
    }

    #[test]
    fn non_utf8_short_option_preserves_original_byte() {
        let error = prepare_uname_args(
            [
                OsString::from("uname"),
                OsString::from_vec(vec![b'-', 0xff]),
            ]
            .into_iter(),
        )
        .unwrap_err();

        assert_eq!(
            error.diagnostic_bytes().as_ref(),
            b"invalid option -- '\xff'"
        );
    }

    #[test]
    fn default_mode_parses_options_after_operands() {
        let error = prepare_uname_args_with_mode(
            ["uname", "extra", "-z"].map(OsString::from).into_iter(),
            false,
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "invalid option -- 'z'");

        let prepared = prepare_uname_args_with_mode(
            ["uname", "extra", "--help"].map(OsString::from).into_iter(),
            false,
        )
        .unwrap();
        assert_eq!(prepared, ["uname", "--help", "extra"].map(OsString::from));
    }

    #[test]
    fn posix_mode_stops_option_parsing_at_first_operand() {
        let error = prepare_uname_args_with_mode(
            ["uname", "extra", "-z"].map(OsString::from).into_iter(),
            true,
        )
        .unwrap_err();

        assert!(error.to_string().starts_with("extra operand "));
    }

    #[test]
    fn sigpipe_ignore_mask_is_parsed_from_proc_status() {
        assert!(sigpipe_is_ignored_in_status(
            "Name:\tbash\nSigIgn:\t0000000000001000\n"
        ));
        assert!(!sigpipe_is_ignored_in_status(
            "Name:\tbash\nSigIgn:\t0000000000000000\n"
        ));
        assert!(!sigpipe_is_ignored_in_status("SigIgn:\tnot-hex\n"));
    }

    #[test]
    fn captured_sigpipe_disposition_takes_priority_over_parent_state() {
        assert_eq!(sigpipe_restore_target(SIG_IGN, false), SIG_IGN);
        assert_eq!(sigpipe_restore_target(SIG_DFL, true), SIG_DFL);
        assert_eq!(sigpipe_restore_target(SIG_ERR, true), SIG_IGN);
        assert_eq!(sigpipe_restore_target(SIG_ERR, false), SIG_DFL);
    }

    #[test]
    fn utf8_locale_operand_quoting_preserves_valid_and_invalid_bytes() {
        assert_eq!(
            quote_utf8_locale_operand(OsStr::new("x"), "‘".as_bytes(), "’".as_bytes(), None),
            "‘x’".as_bytes()
        );
        assert_eq!(
            quote_utf8_locale_operand(
                &OsString::from_vec(vec![0xff]),
                "‘".as_bytes(),
                "’".as_bytes(),
                None
            ),
            "‘\\377’".as_bytes()
        );
        assert_eq!(
            quote_utf8_locale_operand(OsStr::new("x"), b"\"", b"\"", Some(b'\"')),
            b"\"x\""
        );
        assert_eq!(
            quote_utf8_locale_operand(OsStr::new("x’y"), "‘".as_bytes(), "’".as_bytes(), None),
            "‘x\\’y’".as_bytes()
        );
    }

    #[test]
    fn gbk_locale_operand_quoting_uses_gbk_character_boundaries() {
        let locale = LocaleCtype::from_name(c"zh_CN.GBK").expect("zh_CN.GBK locale is available");

        assert_eq!(
            quote_encoded_locale_operand(
                &OsString::from_vec(vec![0xd6, 0xd0]),
                b"\"",
                b"\"",
                Some(b'\"'),
                &locale
            ),
            b"\"\xd6\xd0\""
        );
        assert_eq!(
            quote_encoded_locale_operand(
                &OsString::from_vec(vec![0xe4, 0xb8, 0xad]),
                b"\"",
                b"\"",
                Some(b'\"'),
                &locale
            ),
            b"\"\xe4\xb8\\255\""
        );
    }

    #[test]
    fn locale_text_can_be_transcoded_to_gbk() {
        assert_eq!(
            transcode_utf8("多余的操作对象", c"GBK").as_deref(),
            Some(&b"\xb6\xe0\xd3\xe0\xb5\xc4\xb2\xd9\xd7\xf7\xb6\xd4\xcf\xf3"[..])
        );
    }

    #[test]
    fn message_locale_respects_gnu_category_and_language_priority() {
        assert_eq!(
            message_locale_from_values(None, Some("zh_CN.UTF-8"), Some("en_US.UTF-8"), None),
            MessageLocale::ZhCn
        );
        assert_eq!(
            message_locale_from_values(None, None, Some("zh_CN.UTF-8"), Some("C")),
            MessageLocale::EnUs
        );
        assert_eq!(
            message_locale_from_values(Some("C"), None, Some("zh_CN.UTF-8"), Some("zh_CN")),
            MessageLocale::EnUs
        );
        assert_eq!(
            message_locale_from_values(None, None, Some("en_US.UTF-8"), Some("missing:zh_CN:C")),
            MessageLocale::ZhCn
        );
    }

    #[test]
    fn gb18030_english_locale_uses_gnu_fallback_quote_bytes() {
        let (left, right, quote_to_escape) =
            quote_marks_for_codeset(MessageLocale::EnUs, Some("GB18030"));

        assert_eq!(left, b"\xa1\x07e");
        assert_eq!(right, b"\xa1\xaf");
        assert_eq!(quote_to_escape, None);
    }

    #[test]
    fn invalid_locale_name_is_rejected_without_freeing_null_locale() {
        assert!(LocaleCtype::from_name(c"not_A_locale").is_none());
    }

    #[test]
    fn test_tool_implementation() {
        let tool = Uname;

        // 测试 name 方法
        assert_eq!(tool.name(), "uname");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("uname"));

        // 测试 execute 方法
        let args = [OsString::from("uname")];
        assert!(tool.execute(&args).is_ok());
    }

    #[cfg(test)]
    mod uname_output_tests {
        use super::*;
        use std::cell::Cell;

        #[allow(clippy::too_many_arguments)]
        fn generate_uname_flags(
            is_all: bool,
            is_kernel_name: bool,
            is_node_name: bool,
            is_kernel_release: bool,
            is_kernel_version: bool,
            is_machine: bool,
            is_processor: bool,
            is_hardware_platform: bool,
            is_os: bool,
        ) -> UnameFlags {
            UnameFlags {
                is_all,
                is_kernel_name,
                is_node_name,
                is_kernel_release,
                is_kernel_version,
                is_machine,
                is_processor,
                is_hardware_platform,
                is_os,
            }
        }

        #[test]
        fn test_uname_output_all_flags() {
            let flags =
                generate_uname_flags(true, false, false, false, false, false, false, false, false);
            let uname_output = UNameOutput::new(&flags).unwrap();

            assert_eq!(uname_output.kernel_name, Some(OsString::from("Linux")));
            assert!(uname_output.node_name.is_some());
            assert!(uname_output.kernel_release.is_some());
            assert!(uname_output.kernel_version.is_some());
            assert!(uname_output.machine.is_some());
            assert!(uname_output.os.is_some());
            assert!(uname_output.processor.is_some());
            assert!(uname_output.hardware_platform.is_some());
            assert!(!uname_output.display_bytes().is_empty());
        }

        #[test]
        fn test_uname_output_individual_flags() {
            let flags = generate_uname_flags(false, true, true, true, true, true, true, true, true);
            let uname_output = UNameOutput::new(&flags).unwrap();

            assert!(uname_output.kernel_name.is_some());
            assert!(uname_output.node_name.is_some());
            assert!(uname_output.kernel_release.is_some());
            assert!(uname_output.kernel_version.is_some());
            assert!(uname_output.machine.is_some());
            assert!(uname_output.os.is_some());
            assert!(uname_output.processor.is_some());
            assert!(uname_output.hardware_platform.is_some());
            assert!(!uname_output.display_bytes().is_empty());
        }

        #[test]
        fn test_uname_output_no_flags() {
            let flags = generate_uname_flags(
                false, false, false, false, false, false, false, false, false,
            );
            let uname_output = UNameOutput::new(&flags).unwrap();

            assert!(uname_output.kernel_name.is_some());
            assert!(uname_output.node_name.is_none());
            assert!(uname_output.kernel_release.is_none());
            assert!(uname_output.kernel_version.is_none());
            assert!(uname_output.machine.is_none());
            assert!(uname_output.os.is_none());
            assert!(uname_output.processor.is_none());
            assert!(uname_output.hardware_platform.is_none());

            let expected_output = "Linux ";
            assert_eq!(
                uname_output.display_bytes(),
                expected_output.trim().as_bytes()
            );
        }

        #[test]
        fn test_uname_output_some_flags() {
            let flags =
                generate_uname_flags(false, true, false, true, false, true, false, true, false);
            let uname_output = UNameOutput::new(&flags).unwrap();

            assert!(uname_output.kernel_name.is_some());
            assert!(uname_output.node_name.is_none());
            assert!(uname_output.kernel_release.is_some());
            assert!(uname_output.kernel_version.is_none());
            assert!(uname_output.machine.is_some());
            assert!(uname_output.os.is_none());
            assert!(uname_output.processor.is_none());
            assert!(uname_output.hardware_platform.is_some());
            assert!(!uname_output.display_bytes().is_empty());
        }

        #[test]
        fn display_preserves_trailing_whitespace_inside_last_field() {
            let output = UNameOutput {
                kernel_name: None,
                node_name: Some(OsString::from("node ")),
                kernel_release: None,
                kernel_version: None,
                machine: None,
                os: None,
                processor: None,
                hardware_platform: None,
            };

            assert_eq!(output.display_bytes(), b"node ");
        }

        #[test]
        fn display_preserves_non_utf8_kernel_field_bytes() {
            let output = UNameOutput {
                kernel_name: Some(OsString::from("Linux")),
                node_name: Some(OsString::from_vec(vec![b'n', 0xff])),
                kernel_release: None,
                kernel_version: None,
                machine: None,
                os: None,
                processor: None,
                hardware_platform: None,
            };

            assert_eq!(output.display_bytes(), b"Linux n\xff");
        }

        #[test]
        fn i686_hardware_platform_is_normalized_to_i386() {
            assert_eq!(
                hardware_platform_from_machine(OsStr::new("i686")),
                OsString::from("i386")
            );
            assert_eq!(
                hardware_platform_from_machine(OsStr::new("x86_64")),
                OsString::from("x86_64")
            );
        }

        #[test]
        fn operating_system_only_does_not_query_platform_info() {
            let flags =
                generate_uname_flags(false, false, false, false, false, false, false, false, true);
            let provider_called = Cell::new(false);

            let output = UNameOutput::new_with_platform_provider(&flags, || {
                provider_called.set(true);
                PlatformInfo::new().map_err(|_error| {
                    ctcore::ct_error::CtSimpleError::new(1, "cannot get system name")
                })
            })
            .unwrap();

            assert!(!provider_called.get());
            assert_eq!(output.os, Some(OsString::from("GNU/Linux")));
        }

        #[test]
        fn processor_and_hardware_only_ignore_platform_query_failure() {
            let flags =
                generate_uname_flags(false, false, false, false, false, false, true, true, false);

            let output = UNameOutput::new_with_platform_provider(&flags, || {
                Err(ctcore::ct_error::CtSimpleError::new(
                    1,
                    "cannot get system name",
                ))
            })
            .unwrap();

            assert_eq!(output.processor, Some(OsString::new()));
            assert_eq!(output.hardware_platform, Some(OsString::new()));
            assert_eq!(output.display_bytes(), b" ");
        }

        #[test]
        fn platform_query_error_preserves_errno_message() {
            let source: PlatformInfoError =
                Box::new(std::io::Error::from_raw_os_error(ctcore::libc::EPERM));

            let error = platform_info_error(source);

            assert_eq!(error.code(), 1);
            assert_eq!(
                error.to_string(),
                "cannot get system name: Operation not permitted"
            );
        }

        #[test]
        fn io_error_diagnostic_preserves_localized_context_bytes() {
            let error = std::io::Error::from_raw_os_error(ctcore::libc::ENOSPC);

            let diagnostic = io_error_diagnostic("写入错误".as_bytes(), &error);

            assert_eq!(diagnostic, "写入错误: No space left on device".as_bytes());
        }
    }

    #[cfg(test)]
    mod ct_main_tests {
        use std::ffi::OsString;

        use super::*;

        #[test]
        fn test_uname_main_execution_version() {
            let args = [ctcore::ct_util_name(), "--version"];
            let result = uname_main(args.iter().map(OsString::from));
            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
        }

        #[test]
        fn test_uname_main_execution_other_version() {
            let args = [ctcore::ct_util_name(), "-V"];
            let result = uname_main(args.iter().map(OsString::from));
            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
        }

        #[test]
        fn test_uname_main_execution_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = uname_main(args.iter().map(OsString::from));
            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
        }

        #[test]
        fn test_uname_main_execution_unsupport_help() {
            let args = [ctcore::ct_util_name(), "-H"];
            let result = uname_main(args.iter().map(OsString::from));
            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
        }

        #[test]
        fn test_uname_main_invalid_argument() {
            let args = [ctcore::ct_util_name(), "--invalid-argument"];
            let result = uname_main(args.iter().map(OsString::from));
            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
        }

        #[test]
        fn test_uname_main_support_missing_argument() {
            let args = [ctcore::ct_util_name()]; // 缺少任何参数
            let result = uname_main(args.iter().map(OsString::from));
            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
        }

        #[test]
        fn test_uname_main_long_option_all() {
            let args = [ctcore::ct_util_name(), "--all"];
            let result = uname_main(args.iter().map(OsString::from));
            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
        }

        #[test]
        fn test_uname_main_long_option_kernel_name() {
            let args = [ctcore::ct_util_name(), "--kernel-name"];
            let result = uname_main(args.iter().map(OsString::from));
            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
        }

        #[test]
        fn test_uname_main_long_option_nodename() {
            let args = [ctcore::ct_util_name(), "--nodename"];
            let result = uname_main(args.iter().map(OsString::from));
            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
        }

        #[test]
        fn test_uname_main_long_option_kernel_release() {
            let args = [ctcore::ct_util_name(), "--kernel-release"];
            let result = uname_main(args.iter().map(OsString::from));
            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
        }

        #[test]
        fn test_uname_main_long_option_kernel_version() {
            let args = [ctcore::ct_util_name(), "--kernel-version"];
            let result = uname_main(args.iter().map(OsString::from));
            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
        }

        #[test]
        fn test_uname_main_long_option_machine() {
            let args = [ctcore::ct_util_name(), "--machine"];
            let result = uname_main(args.iter().map(OsString::from));
            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
        }

        #[test]
        fn test_uname_main_long_option_os() {
            let args = [ctcore::ct_util_name(), "--operating-system"];
            let result = uname_main(args.iter().map(OsString::from));
            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
        }

        #[test]
        fn test_uname_main_long_option_processor() {
            let args = [ctcore::ct_util_name(), "--processor"];
            let result = uname_main(args.iter().map(OsString::from));
            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
        }

        #[test]
        fn test_uname_main_long_option_hardware_platform() {
            let args = [ctcore::ct_util_name(), "--hardware-platform"];
            let result = uname_main(args.iter().map(OsString::from));
            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
        }

        #[test]
        fn test_uname_main_short_option_a() {
            let args = [ctcore::ct_util_name(), "-a"];
            let result = uname_main(args.iter().map(OsString::from));
            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
        }

        #[test]
        fn test_uname_main_short_option_s() {
            let args = [ctcore::ct_util_name(), "-s"];
            let result = uname_main(args.iter().map(OsString::from));
            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
        }

        #[test]
        fn test_uname_main_short_option_n() {
            let args = [ctcore::ct_util_name(), "-n"];
            let result = uname_main(args.iter().map(OsString::from));
            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
        }

        #[test]
        fn test_uname_main_short_option_r() {
            let args = [ctcore::ct_util_name(), "-r"];
            let result = uname_main(args.iter().map(OsString::from));
            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
        }

        #[test]
        fn test_uname_main_short_option_v() {
            let args = [ctcore::ct_util_name(), "-v"];
            let result = uname_main(args.iter().map(OsString::from));
            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
        }

        #[test]
        fn test_uname_main_short_option_m() {
            let args = [ctcore::ct_util_name(), "-m"];
            let result = uname_main(args.iter().map(OsString::from));
            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
        }

        #[test]
        fn test_uname_main_short_option_o() {
            let args = [ctcore::ct_util_name(), "-o"];
            let result = uname_main(args.iter().map(OsString::from));
            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
        }

        #[test]
        fn test_uname_main_short_option_p() {
            let args = [ctcore::ct_util_name(), "-p"];
            let result = uname_main(args.iter().map(OsString::from));
            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
        }

        #[test]
        fn test_uname_main_short_option_i() {
            let args = [ctcore::ct_util_name(), "-i"];
            let result = uname_main(args.iter().map(OsString::from));
            if let Err(output) = result {
                let code = output.code();
                let message = output.usage();
                println!("Error code: {code}");
                println!("Error message: {message}");
            }
        }
    }

    #[cfg(test)]
    mod ct_app_tests {
        use clap::error::ErrorKind;

        use super::*;

        // uname 接口测试: uname [OPTION]...
        //   -a, --all                print all information, in the following order,
        //                              except omit -p and -i if unknown:
        //   -s, --kernel-name        print the kernel name
        //   -n, --nodename           print the network node hostname
        //   -r, --kernel-release     print the kernel release
        //   -v, --kernel-version     print the kernel version
        //   -m, --machine            print the machine hardware name
        //   -p, --processor          print the processor type (non-portable)
        //   -i, --hardware-platform  print the hardware platform (non-portable)
        //   -o, --operating-system   print the operating system
        //       --help     display this help and exit
        //       --version  output version information and exit

        #[test]
        fn test_ct_app_execution_version() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "--version"];

            // Assuming `command` has a method to retrieve the executable name, replace it with the actual one
            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_other_version() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "-V"];
            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_help() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "--help"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_execution_unsupport_help() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-H"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_invalid_argument() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_support_missing_argument() {
            let command = ct_app();

            let missing_args = vec![ctcore::ct_util_name()]; // 缺少任何参数
            let result = command.try_get_matches_from(missing_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_all() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "--all"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_kernel_name() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "--kernel-name"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_repeated_kernel_name_options() {
            let matches = ct_app()
                .try_get_matches_from([ctcore::ct_util_name(), "-s", "-s", "--kernel-name"])
                .unwrap();

            assert!(matches.get_flag(uname_flags::UNAME_KERNEL_NAME));
            assert!(!matches.get_flag(uname_flags::UNAME_ALL));
        }

        #[test]
        fn test_ct_app_long_option_nodename() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "--nodename"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_kernel_release() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "--kernel-release"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_kernel_version() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "--kernel-version"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_machine() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "--machine"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_os() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "--operating-system"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_processor() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "--processor"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_hardware_platform() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "--hardware-platform"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_short_option_a() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "-a"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_short_option_s() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "-s"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_short_option_n() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "-n"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_short_option_r() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "-r"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_short_option_v() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "-v"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_short_option_m() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "-m"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_short_option_o() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "-o"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_short_option_p() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "-p"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_short_option_i() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "-i"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }
    }
}
