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
pub mod native_int_str;
pub mod parse_error;
pub mod split_iterator;
pub mod string_expander;
pub mod string_parser;
pub mod variable_parser;

use clap::builder::ValueParser;
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use clap::Arg;
use clap::ArgAction;
use clap::Command;
use clap::crate_version;
use sys_locale::get_locale;

use ini::Ini;
use native_int_str::{
    EnvConvert, NCvt, NativeIntStr, NativeIntString, NativeStr, from_native_int_representation,
    from_native_int_representation_owned, get_single_native_int_value,
};
#[cfg(unix)]
use nix::sys::signal::Signal;

#[cfg(unix)]
use nix::libc;

use std::borrow::Cow;
use std::env;
#[cfg(unix)]
use std::ffi::CStr;
#[cfg(target_os = "linux")]
use std::ffi::CString;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::io::{self, Write};
use std::ops::Deref;

#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStrExt;

use ctcore::ct_display::Quotable;
use ctcore::ct_error::CTError;
use ctcore::ct_error::CTResult;
use ctcore::ct_error::CTsageError;
use ctcore::ct_error::CtSimpleError;
use ctcore::ct_error::ExitCode;

use ctcore::Tool;
use ctcore::ct_line_ending::CtLineEnding;
#[cfg(not(target_os = "linux"))]
use ctcore::ct_show_warning;

#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::process::{self};
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(unix)]
type EnvSignal = libc::c_int;

#[cfg(unix)]
type SignalDispositions = (Option<Vec<EnvSignal>>, Option<Vec<EnvSignal>>);

#[cfg(unix)]
#[derive(Clone, Copy)]
enum SignalDisposition {
    Default,
    Ignore,
}

#[cfg(unix)]
#[derive(Debug, PartialEq)]
struct SignalMasks {
    block: Vec<EnvSignal>,
    unblock: Vec<EnvSignal>,
}

#[cfg(unix)]
const MISSING_SIGNAL_ARGUMENT: &str = "\0";

#[cfg(target_os = "linux")]
unsafe extern "C" {
    static mut environ: *mut *mut libc::c_char;
}

#[cfg(target_os = "linux")]
static INHERITED_SIGPIPE_HANDLER: AtomicUsize = AtomicUsize::new(libc::SIG_ERR);

#[cfg(target_os = "linux")]
#[used]
#[unsafe(link_section = ".init_array")]
static CAPTURE_INHERITED_SIGPIPE: unsafe extern "C" fn() = capture_inherited_sigpipe;

#[cfg(target_os = "linux")]
unsafe extern "C" fn capture_inherited_sigpipe() {
    let mut action = std::mem::MaybeUninit::<libc::sigaction>::uninit();
    if unsafe { libc::sigaction(libc::SIGPIPE, std::ptr::null(), action.as_mut_ptr()) } == 0 {
        let action = unsafe { action.assume_init() };
        INHERITED_SIGPIPE_HANDLER.store(action.sa_sigaction, Ordering::Relaxed);
    }
}

#[derive(Debug, PartialEq)]
struct EnvOptions<'a> {
    ignore_env: bool,
    line_ending: CtLineEnding,
    running_directory: Option<&'a OsStr>,
    files: Vec<&'a OsStr>,
    unsets: Vec<&'a OsStr>,
    sets: Vec<(Cow<'a, OsStr>, Cow<'a, OsStr>)>,
    program: Vec<&'a OsStr>,
    #[cfg(unix)]
    default_signals: Option<Vec<EnvSignal>>,
    #[cfg(unix)]
    ignore_signals: Option<Vec<EnvSignal>>,
    #[cfg(unix)]
    block_signals: Option<SignalMasks>,
    #[cfg(unix)]
    list_signal_handling: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvRow {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvSemantic {
    pub rows: Vec<EnvRow>,
    pub classic_text: String,
    pub stderr_text: String,
    pub exit_code: i32,
}

fn print_env(line_ending: CtLineEnding) {
    #[cfg(target_os = "linux")]
    {
        let stdout_raw = io::stdout();
        let mut stdout = stdout_raw.lock();
        let mut entry_ptr = unsafe { environ };
        while !entry_ptr.is_null() && unsafe { !(*entry_ptr).is_null() } {
            let entry = unsafe { CStr::from_ptr(*entry_ptr) };
            write_environment_entry(&mut stdout, entry, line_ending)
                .expect("write environment entry");
            entry_ptr = unsafe { entry_ptr.add(1) };
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        let stdout_raw = io::stdout();
        let mut stdout = stdout_raw.lock();
        for (n, v) in env::vars() {
            write!(stdout, "{n}={v}{line_ending}").unwrap();
        }
    }
}

#[cfg(target_os = "linux")]
fn write_environment_entry(
    stdout: &mut impl Write,
    entry: &CStr,
    line_ending: CtLineEnding,
) -> io::Result<()> {
    stdout.write_all(entry.to_bytes())?;
    stdout.write_all(&[line_ending.into()])
}

fn env_parse_name_value_opt<'a>(options: &mut EnvOptions<'a>, option: &'a OsStr) -> CTResult<bool> {
    let wrap = NativeStr::<'a>::new(option);
    let split_o = wrap.split_once(&'=');
    if let Some((name, value)) = split_o {
        options.sets.push((name, value));
        Ok(false)
    } else {
        env_parse_program_opt(options, option).map(|_| true)
    }
}

fn env_parse_program_opt<'a>(options: &mut EnvOptions<'a>, option: &'a OsStr) -> CTResult<()> {
    options.program.push(option);
    Ok(())
}

fn env_validate_null_output(options: &EnvOptions<'_>) -> CTResult<()> {
    if options.line_ending == CtLineEnding::Nul && !options.program.is_empty() {
        return Err(CTsageError::new(
            125,
            "cannot specify --null (-0) with command".to_string(),
        ));
    }
    Ok(())
}

fn env_load_config_file(options: &mut EnvOptions) -> CTResult<()> {
    for &file in &options.files {
        let config = if file == "-" {
            let stdin = io::stdin();
            let mut stdin_locked = stdin.lock();
            Ini::read_from(&mut stdin_locked)
        } else {
            Ini::load_from_file(file)
        };

        let config =
            config.map_err(|e| CtSimpleError::new(1, format!("{}: {}", file.maybe_quote(), e)))?;

        for (_, prop) in &config {
            for (key, value) in prop.iter() {
                unsafe { env::set_var(key, value) };
            }
        }
    }
    Ok(())
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("env.about");
    let usage_description = t!("env.usage");
    let args = env_args_init();

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .after_help(t!("env.after_help"))
        .infer_long_args(true)
        .trailing_var_arg(true)
        .args(&args)
}

fn env_args_init() -> Vec<Arg> {
    let args = vec![
        Arg::new("ignore-environment")
            .short('i')
            .long("ignore-environment")
            .help(t!("env.clap.ignore-environment"))
            .action(ArgAction::Count),
        Arg::new("chdir")
            .short('C')
            .long("chdir")
            .number_of_values(1)
            .value_name("DIR")
            .value_parser(ValueParser::os_string())
            .value_hint(clap::ValueHint::DirPath)
            .action(ArgAction::Append)
            .help("change working directory to DIR"),
        Arg::new("null")
            .short('0')
            .long("null")
            .help("end each output line with a 0 byte rather than a newline")
            .action(ArgAction::Count),
        Arg::new("file")
            .short('f')
            .long("file")
            .value_name("PATH")
            .value_hint(clap::ValueHint::FilePath)
            .value_parser(ValueParser::os_string())
            .action(ArgAction::Append)
            .help("read and set variables from a \".env\"-style configuration file"),
        Arg::new("unset")
            .short('u')
            .long("unset")
            .value_name("NAME")
            .action(ArgAction::Append)
            .value_parser(ValueParser::os_string())
            .help("remove variable from the environment"),
        Arg::new("debug")
            .short('v')
            .long("debug")
            .action(ArgAction::Count)
            .help(t!("env.clap.debug")),
        Arg::new("split-string")
            .short('S')
            .long("split-string")
            .value_name("S")
            .action(ArgAction::Set)
            .value_parser(ValueParser::os_string())
            .help("process and split S into separate arguments"),
        // --- 核心新增：四个用于系统信号掩码拦截和查询的 Flag ---
        Arg::new("default-signal")
            .long("default-signal")
            .value_name("SIG")
            .num_args(0..=1)
            .require_equals(true)
            .default_missing_value(MISSING_SIGNAL_ARGUMENT)
            .action(ArgAction::Append)
            .help("reset handling of SIG to its default"),
        Arg::new("ignore-signal")
            .long("ignore-signal")
            .value_name("SIG")
            .num_args(0..=1)
            .require_equals(true)
            .default_missing_value(MISSING_SIGNAL_ARGUMENT)
            .action(ArgAction::Append)
            .help("set handling of SIG to do nothing"),
        Arg::new("block-signal")
            .long("block-signal")
            .value_name("SIG")
            .num_args(0..=1)
            .require_equals(true)
            .default_missing_value(MISSING_SIGNAL_ARGUMENT)
            .action(ArgAction::Append)
            .help("block delivery of SIG"),
        Arg::new("list-signal-handling")
            .long("list-signal-handling")
            .action(ArgAction::Count)
            .help("list nondefault signal handling to stderr"),
        Arg::new("vars")
            .action(ArgAction::Append)
            .value_parser(ValueParser::os_string()),
    ];
    args
}

pub fn env_parse_args_from_str(native_text: &NativeIntStr) -> CTResult<Vec<NativeIntString>> {
    split_iterator::split(native_text).map_err(|e| match e {
        parse_error::EnvParseError::BackslashCNotAllowedInDoubleQuotes { pos: _ } => {
            CtSimpleError::new(125, "'\\c' must not appear in double-quoted -S string")
        }
        parse_error::EnvParseError::InvalidBackslashAtEndOfStringInMinusS {
            pos: _,
            quoting: _,
        } => CtSimpleError::new(125, "invalid backslash at end of string in -S"),
        parse_error::EnvParseError::InvalidSequenceBackslashXInMinusS { pos: _, c } => {
            CtSimpleError::new(125, format!("invalid sequence '\\{c}' in -S"))
        }
        parse_error::EnvParseError::MissingClosingQuote { pos: _, c: _ } => {
            CtSimpleError::new(125, "no terminating quote in -S string")
        }
        parse_error::EnvParseError::ParsingOfVariableNameFailed { pos, msg: _ } => {
            let scan_end = pos.min(native_text.len());
            let dollar = get_single_native_int_value(&'$').expect("dollar has native encoding");
            let start = native_text[..scan_end]
                .iter()
                .rposition(|character| *character == dollar)
                .unwrap_or_else(|| scan_end.saturating_sub(1));
            let fragment_os = from_native_int_representation(Cow::Borrowed(&native_text[start..]));
            let fragment = fragment_os.to_string_lossy();
            CtSimpleError::new(
                125,
                format!("only ${{VARNAME}} expansion is supported, error at: {fragment}"),
            )
        }
        _ => CtSimpleError::new(125, format!("Error: {e:?}")),
    })
}

fn env_extract_combined_short_split_string(
    arg: &OsStr,
) -> Option<(Option<OsString>, Option<OsString>)> {
    let native_arg = NCvt::convert(arg);
    let dash = get_single_native_int_value(&'-').expect("dash has native encoding");
    let split_string = get_single_native_int_value(&'S').expect("S has native encoding");
    let no_argument_options = [
        get_single_native_int_value(&'i').expect("i has native encoding"),
        get_single_native_int_value(&'v').expect("v has native encoding"),
        get_single_native_int_value(&'0').expect("0 has native encoding"),
    ];

    if native_arg.first() != Some(&dash) {
        return None;
    }

    let mut prefix = vec![dash];
    for (index, &option) in native_arg.iter().enumerate().skip(1) {
        if no_argument_options.contains(&option) {
            prefix.push(option);
            continue;
        }
        if option != split_string {
            return None;
        }

        let options = (prefix.len() > 1).then(|| from_native_int_representation_owned(prefix));
        let split_argument = (index + 1 < native_arg.len())
            .then(|| from_native_int_representation_owned(native_arg[index + 1..].to_vec()));
        return Some((options, split_argument));
    }

    None
}

fn env_short_options_enable_debug(arg: &OsStr) -> bool {
    let native_arg = NCvt::convert(arg);
    let dash = get_single_native_int_value(&'-').expect("dash has native encoding");
    let debug = get_single_native_int_value(&'v').expect("v has native encoding");
    let split_string = get_single_native_int_value(&'S').expect("S has native encoding");
    let no_argument_options = [
        get_single_native_int_value(&'i').expect("i has native encoding"),
        debug,
        get_single_native_int_value(&'0').expect("0 has native encoding"),
    ];
    let argument_options = [
        get_single_native_int_value(&'u').expect("u has native encoding"),
        get_single_native_int_value(&'C').expect("C has native encoding"),
    ];

    if native_arg.first() != Some(&dash) {
        return false;
    }

    let mut debug_enabled = false;
    for &option in native_arg.iter().skip(1) {
        if option == debug {
            debug_enabled = true;
        } else if no_argument_options.contains(&option) {
            continue;
        } else if option == split_string || argument_options.contains(&option) {
            return debug_enabled;
        } else {
            return false;
        }
    }

    debug_enabled
}

fn env_is_c_whitespace_byte(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

fn env_shebang_whitespace_option(arg: &OsStr) -> Option<u8> {
    let bytes = arg.as_encoded_bytes();
    if !bytes.starts_with(b"-") || bytes.get(1) == Some(&b'-') {
        return None;
    }

    for &option in &bytes[1..] {
        if env_is_c_whitespace_byte(option) {
            return Some(option);
        }
        if !matches!(option, b'i' | b'v' | b'0') {
            return None;
        }
    }
    None
}

fn env_split_string_argument(
    split_arg: &OsStr,
    all_args: &mut Vec<OsString>,
    is_debug_printing: bool,
) -> CTResult<()> {
    let arg_strings = env_parse_args_from_str(&NCvt::convert(split_arg))?
        .into_iter()
        .map(from_native_int_representation_owned)
        .collect::<Vec<_>>();

    if is_debug_printing && !arg_strings.is_empty() {
        eprintln!("split -S:  {}", split_arg.quote());
        eprintln!(" into:    {}", arg_strings[0].quote());
        for argument in arg_strings.iter().skip(1) {
            eprintln!("     &    {}", argument.quote());
        }
    }

    all_args.extend(arg_strings);
    Ok(())
}

fn env_is_name_value_operand(arg: &OsStr) -> bool {
    NativeStr::new(arg).strip_prefix(OsStr::new("-")).is_none()
        && NativeStr::new(arg).split_once(&'=').is_some()
}

fn env_is_option_like(arg: &OsStr) -> bool {
    NativeStr::new(arg).strip_prefix(OsStr::new("-")).is_some()
}

fn env_has_program_name(args: &[OsString]) -> bool {
    args.first()
        .map(|arg| arg.as_os_str() == OsStr::new(ctcore::ct_util_name()))
        .unwrap_or(false)
}

#[derive(Default)]
struct EnvAppData {
    do_debug_printing: bool,
    had_string_argument: bool,
}

impl EnvAppData {
    fn make_error_no_such_file_or_dir(&self, program: &OsStr) -> Box<dyn CTError> {
        ctcore::ct_show_error!("{}: No such file or directory", program.quote());
        if env_contains_c_whitespace(program) {
            ctcore::ct_show_error!("use -[v]S to pass options in shebang lines");
        }
        ExitCode::new(127)
    }

    fn process_all_string_arguments(
        &mut self,
        source_args: &[OsString],
    ) -> CTResult<Vec<std::ffi::OsString>> {
        let Some(first_arg) = source_args.first() else {
            return Ok(Vec::new());
        };
        let has_program_name = env_has_program_name(source_args);
        let program_name = has_program_name.then(|| first_arg.clone());
        let mut args = if has_program_name {
            source_args[1..].to_vec()
        } else {
            source_args.to_vec()
        };
        loop {
            let mut all_args: Vec<std::ffi::OsString> =
                program_name.iter().cloned().collect::<Vec<_>>();
            let mut next_args: Vec<std::ffi::OsString> = Vec::new();
            let mut iter = args.iter().peekable();
            let mut expanded_split_string = false;

            while let Some(arg) = iter.next() {
                if arg == "--" || arg == "-" {
                    next_args.push(arg.clone());
                    next_args.extend(iter.cloned());
                    break;
                }

                if arg == "--split-string" {
                    let Some(split_arg) = iter.next() else {
                        return Err(CTsageError::new(
                            125,
                            "option '--split-string' requires an argument".to_string(),
                        ));
                    };
                    env_split_string_argument(split_arg, &mut next_args, self.do_debug_printing)?;
                    self.had_string_argument = true;
                    expanded_split_string = true;
                    next_args.extend(iter.cloned());
                    break;
                }

                if let Some(split_arg) =
                    NativeStr::new(arg).strip_prefix(OsStr::new("--split-string="))
                {
                    env_split_string_argument(&split_arg, &mut next_args, self.do_debug_printing)?;
                    self.had_string_argument = true;
                    expanded_split_string = true;
                    next_args.extend(iter.cloned());
                    break;
                }

                if env_is_name_value_operand(arg) {
                    next_args.push(arg.clone());
                    next_args.extend(iter.cloned());
                    break;
                }

                if !env_is_option_like(arg) {
                    next_args.push(arg.clone());
                    next_args.extend(iter.cloned());
                    break;
                }

                if let Some(option) = env_shebang_whitespace_option(arg) {
                    ctcore::ct_show_error!("invalid option -- '{}'", char::from(option));
                    return Err(CTsageError::new(
                        125,
                        "use -[v]S to pass options in shebang lines".to_string(),
                    ));
                }

                if arg == "--debug" || env_short_options_enable_debug(arg) {
                    self.do_debug_printing = true;
                }

                if arg == "-S" {
                    let Some(split_arg) = iter.next() else {
                        return Err(CTsageError::new(
                            125,
                            "option requires an argument -- 'S'".to_string(),
                        ));
                    };
                    env_split_string_argument(split_arg, &mut next_args, self.do_debug_printing)?;
                    self.had_string_argument = true;
                    expanded_split_string = true;
                    next_args.extend(iter.cloned());
                    break;
                }

                if let Some(split_arg) = NativeStr::new(arg).strip_prefix(OsStr::new("-S")) {
                    env_split_string_argument(&split_arg, &mut next_args, self.do_debug_printing)?;
                    self.had_string_argument = true;
                    expanded_split_string = true;
                    next_args.extend(iter.cloned());
                    break;
                }

                if let Some((options, split_argument)) =
                    env_extract_combined_short_split_string(arg)
                {
                    if let Some(options) = options {
                        next_args.push(options);
                    }
                    let split_argument = match split_argument {
                        Some(split_argument) => split_argument,
                        None => iter.next().cloned().ok_or_else(|| {
                            CTsageError::new(125, "option requires an argument -- 'S'".to_string())
                        })?,
                    };
                    env_split_string_argument(
                        &split_argument,
                        &mut next_args,
                        self.do_debug_printing,
                    )?;
                    self.had_string_argument = true;
                    expanded_split_string = true;
                    next_args.extend(iter.cloned());
                    break;
                }

                next_args.push(arg.clone());
            }

            all_args.extend(next_args.clone());
            if !expanded_split_string {
                return Ok(all_args);
            }

            args = next_args;
        }
    }

    fn parse_arguments(
        &mut self,
        source_args: impl ctcore::Args,
    ) -> Result<(Vec<OsString>, clap::ArgMatches), Box<dyn CTError>> {
        let sources_args: Vec<OsString> = source_args.collect();
        let args = self.process_all_string_arguments(&sources_args)?;
        let args_for_clap = if env_has_program_name(&sources_args) || !self.had_string_argument {
            args
        } else {
            let mut args_for_clap = Vec::with_capacity(args.len() + 1);
            args_for_clap.push(OsString::from(ctcore::ct_util_name()));
            args_for_clap.extend(args);
            args_for_clap
        };
        let app = ct_app();
        let args_match =
            app.try_get_matches_from(args_for_clap)
                .map_err(|e| -> Box<dyn CTError> {
                    match e.kind() {
                        clap::error::ErrorKind::DisplayHelp
                        | clap::error::ErrorKind::DisplayVersion => e.into(),
                        _ => {
                            let s = format!("{e}");
                            if !s.is_empty() {
                                eprintln!("{}", s.trim_end());
                            }
                            ctcore::ct_error::ExitCode::new(125)
                        }
                    }
                })?;
        Ok((sources_args, args_match))
    }

    fn run_env(&mut self, source_args: impl ctcore::Args) -> CTResult<()> {
        let (_sources_args, matches) = self.parse_arguments(source_args)?;

        let is_debug_printing = self.do_debug_printing || matches.get_count("debug") > 0;

        let mut options = env_make_options(&matches)?;

        apply_removal_of_all_env_vars(&options, is_debug_printing);

        env_load_config_file(&mut options)?;

        env_apply_unset_env_vars(&options, is_debug_printing)?;

        env_apply_specified_env_vars(&options, is_debug_printing)?;

        env_validate_null_output(&options)?;

        if options.program.is_empty() {
            env_apply_change_directory(&options)?;
            print_env(options.line_ending);
        } else {
            #[cfg(unix)]
            if options.list_signal_handling {
                apply_signal_handlers(&options, is_debug_printing)?;
                list_signal_handling(&options);
                return self.run_program_with_signal_handling(options, is_debug_printing, false);
            }

            return self.run_program(options, is_debug_printing);
        }

        Ok(())
    }

    fn run_program(
        &mut self,
        options: EnvOptions<'_>,
        is_do_debug_printing: bool,
    ) -> Result<(), Box<dyn CTError>> {
        self.run_program_with_signal_handling(options, is_do_debug_printing, true)
    }

    fn run_program_with_signal_handling(
        &mut self,
        options: EnvOptions<'_>,
        is_do_debug_printing: bool,
        apply_signal_handling: bool,
    ) -> Result<(), Box<dyn CTError>> {
        let prog = Cow::from(options.program[0]);
        let args = &options.program[1..];

        let mut command = process::Command::new(&*prog);
        command.args(args);

        #[cfg(unix)]
        {
            if apply_signal_handling {
                apply_signal_handlers(&options, is_do_debug_printing)?;
            }
        }

        if let Some(directory) = options.running_directory
            && is_do_debug_printing
        {
            eprintln!("{}", env_change_directory_debug_message(directory));
        }
        env_apply_change_directory(&options)?;

        if is_do_debug_printing {
            eprintln!("executing: {}", prog.to_string_lossy());
            for (i, arg) in options.program.iter().enumerate() {
                eprintln!("   arg[{i}]= {}", arg.quote());
            }
        }

        #[cfg(unix)]
        {
            let error = command.exec();
            Err(self.command_execution_error(prog.deref(), error))
        }

        #[cfg(not(unix))]
        {
            let _ = apply_signal_handling;
            match command.status() {
                Ok(exit) if !exit.success() => return Err(exit.code().unwrap_or(1).into()),
                Err(error) => return Err(self.command_execution_error(prog.deref(), error)),
                Ok(_) => {}
            }
            Ok(())
        }
    }

    fn command_execution_error(&self, program: &OsStr, error: io::Error) -> Box<dyn CTError> {
        match command_execution_error_exit_code(&error) {
            127 => self.make_error_no_such_file_or_dir(program),
            126 => {
                ctcore::ct_show_error!("{}", command_execution_error_message(program, &error));
                126.into()
            }
            _ => unreachable!("command execution errors map only to 126 or 127"),
        }
    }
}

fn command_execution_error_message(program: &OsStr, error: &io::Error) -> String {
    format!(
        "{}: {}",
        program.quote(),
        command_execution_error_text(error)
    )
}

fn command_execution_error_text(error: &io::Error) -> String {
    #[cfg(unix)]
    if let Some(errno) = error.raw_os_error() {
        return unsafe { CStr::from_ptr(libc::strerror(errno)) }
            .to_string_lossy()
            .into_owned();
    }

    error.to_string()
}

fn env_contains_c_whitespace(value: &OsStr) -> bool {
    value
        .as_encoded_bytes()
        .iter()
        .copied()
        .any(env_is_c_whitespace_byte)
}

fn command_execution_error_exit_code(error: &io::Error) -> i32 {
    match error.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::InvalidInput => 127,
        _ => 126,
    }
}

fn apply_removal_of_all_env_vars(options: &EnvOptions<'_>, is_debug_printing: bool) {
    if options.ignore_env {
        if is_debug_printing {
            eprintln!("cleaning environ");
        }
        for (ref name, _) in env::vars_os() {
            unsafe { env::remove_var(name) };
        }
    }
}

// --- Unix 系统下的核心信号解析器 ---
#[cfg(unix)]
fn parse_signal(sig_str: &str) -> CTResult<EnvSignal> {
    let sig_str = sig_str.to_uppercase();
    #[cfg(target_os = "linux")]
    if let Some(signal_number) = ctcore::ct_signals::get_ct_signal_by_name_or_value(&sig_str)
        && let Ok(signal_number) = i32::try_from(signal_number)
        && signal_number > 0
    {
        return Ok(signal_number);
    }

    let s = if sig_str.starts_with("SIG") {
        sig_str.clone()
    } else {
        format!("SIG{sig_str}")
    };
    for sig in Signal::iterator() {
        if sig.as_str() == s {
            return Ok(sig as EnvSignal);
        }
    }
    if let Ok(num) = sig_str.parse::<i32>() {
        if let Ok(sig) = Signal::try_from(num) {
            return Ok(sig as EnvSignal);
        }
    }
    Err(CtSimpleError::new(
        125,
        format!("invalid signal {}", sig_str.quote()),
    ))
}

#[cfg(unix)]
fn parse_signal_list(val: &str) -> CTResult<Vec<EnvSignal>> {
    let mut sigs = Vec::new();
    for p in val.split(',') {
        if !p.is_empty() {
            sigs.push(parse_signal(p)?);
        }
    }
    Ok(sigs)
}

#[cfg(unix)]
fn invalid_argument_message() -> String {
    unsafe { CStr::from_ptr(libc::strerror(libc::EINVAL)) }
        .to_string_lossy()
        .into_owned()
}

#[cfg(unix)]
fn known_env_signals() -> Vec<EnvSignal> {
    let mut signals: Vec<_> = Signal::iterator()
        .map(|signal| signal as EnvSignal)
        .collect();
    #[cfg(target_os = "linux")]
    signals.extend(libc::SIGRTMIN()..=libc::SIGRTMAX());
    signals
}

#[cfg(unix)]
fn signal_name(signal: EnvSignal) -> Option<String> {
    #[cfg(target_os = "linux")]
    if signal == libc::SIGIO {
        return Some("POLL".to_owned());
    }

    if let Ok(signal) = Signal::try_from(signal) {
        return Some(
            signal
                .as_str()
                .strip_prefix("SIG")
                .unwrap_or(signal.as_str())
                .to_owned(),
        );
    }

    #[cfg(target_os = "linux")]
    {
        let rtmin = libc::SIGRTMIN();
        let rtmax = libc::SIGRTMAX();
        if (rtmin..=rtmax).contains(&signal) {
            let (base, name) = if signal <= rtmin + (rtmax - rtmin) / 2 {
                (rtmin, "RTMIN")
            } else {
                (rtmax, "RTMAX")
            };
            let offset = signal - base;
            return Some(if offset == 0 {
                name.to_owned()
            } else {
                format!("{name}{offset:+}")
            });
        }
    }

    None
}

#[cfg(unix)]
fn signal_disposition_debug_message(signal: EnvSignal, action: &str) -> String {
    let name = signal_name(signal).expect("known signal has a display name");
    format!("Reset signal {name} ({signal}) to {action}")
}

#[cfg(unix)]
fn signal_mask_debug_message(signal: EnvSignal, action: &str) -> String {
    let name = signal_name(signal).expect("known signal has a display name");
    format!("signal {name} ({signal}) mask set to {action}")
}

#[cfg(unix)]
fn get_signal_masks(args_match: &clap::ArgMatches) -> CTResult<Option<SignalMasks>> {
    if !args_match.contains_id("block-signal") && !args_match.contains_id("default-signal") {
        return Ok(None);
    }

    let mut operations = Vec::new();
    if let (Some(indices), Some(values)) = (
        args_match.indices_of("block-signal"),
        args_match.get_many::<String>("block-signal"),
    ) {
        operations.extend(
            indices
                .zip(values)
                .map(|(index, value)| (index, false, value)),
        );
    }
    if let (Some(indices), Some(values)) = (
        args_match.indices_of("default-signal"),
        args_match.get_many::<String>("default-signal"),
    ) {
        operations.extend(
            indices
                .zip(values)
                .map(|(index, value)| (index, true, value)),
        );
    }
    operations.sort_unstable_by_key(|(index, _, _)| *index);

    let mut block = Vec::new();
    let mut unblock = Vec::new();
    for (_, should_unblock, value) in operations {
        let signals = if value == MISSING_SIGNAL_ARGUMENT {
            known_env_signals()
        } else {
            parse_signal_list(value)?
        };

        for signal in signals {
            if should_unblock {
                block.retain(|candidate| *candidate != signal);
                if !unblock.contains(&signal) {
                    unblock.push(signal);
                }
            } else {
                unblock.retain(|candidate| *candidate != signal);
                if !block.contains(&signal) {
                    block.push(signal);
                }
            }
        }
    }
    Ok(Some(SignalMasks { block, unblock }))
}

#[cfg(all(unix, test))]
fn get_signal_dispositions(args_match: &clap::ArgMatches) -> CTResult<SignalDispositions> {
    get_signal_dispositions_for_program(args_match, true)
}

#[cfg(unix)]
fn get_signal_dispositions_for_program(
    args_match: &clap::ArgMatches,
    program_specified: bool,
) -> CTResult<SignalDispositions> {
    let mut operations = Vec::new();
    for (name, set_default) in [("default-signal", true), ("ignore-signal", false)] {
        if let (Some(indices), Some(values)) = (
            args_match.indices_of(name),
            args_match.get_many::<String>(name),
        ) {
            operations.extend(
                indices
                    .zip(values)
                    .map(|(index, value)| (index, set_default, value)),
            );
        }
    }
    operations.sort_unstable_by_key(|(index, _, _)| *index);

    let saw_default = operations.iter().any(|(_, set_default, _)| *set_default);
    let saw_ignore = operations.iter().any(|(_, set_default, _)| !set_default);
    let mut default_signals = Vec::new();
    let mut ignore_signals = Vec::new();

    for (_, set_default, value) in operations {
        let ignore_immutable_signal_errors = value == MISSING_SIGNAL_ARGUMENT;
        let signals = if ignore_immutable_signal_errors {
            known_env_signals()
        } else {
            parse_signal_list(value)?
        };

        for signal in signals {
            if program_specified
                && (signal == libc::SIGKILL || signal == libc::SIGSTOP)
                && !ignore_immutable_signal_errors
            {
                return Err(CtSimpleError::new(
                    125,
                    format!(
                        "failed to set signal action for signal {}: {}",
                        signal,
                        invalid_argument_message()
                    ),
                ));
            }
            let (selected, overridden) = if set_default {
                (&mut default_signals, &mut ignore_signals)
            } else {
                (&mut ignore_signals, &mut default_signals)
            };
            overridden.retain(|candidate| *candidate != signal);
            if !selected.contains(&signal) {
                selected.push(signal);
            }
        }
    }

    Ok((
        saw_default.then_some(default_signals),
        saw_ignore.then_some(ignore_signals),
    ))
}

#[cfg(unix)]
fn apply_signal_disposition(signal: EnvSignal, disposition: SignalDisposition) -> bool {
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        if libc::sigaction(signal, std::ptr::null(), &mut action) != 0 {
            return false;
        }
        action.sa_sigaction = match disposition {
            SignalDisposition::Default => libc::SIG_DFL,
            SignalDisposition::Ignore => libc::SIG_IGN,
        };
        libc::sigaction(signal, &action, std::ptr::null_mut()) == 0
    }
}

// --- 处理和打印系统级信号状态 ---
#[cfg(unix)]
fn apply_signal_handlers(options: &EnvOptions, is_debug_printing: bool) -> CTResult<()> {
    apply_signal_handlers_to_process(
        options.default_signals.as_deref(),
        options.ignore_signals.as_deref(),
        options.block_signals.as_ref(),
        is_debug_printing,
    );
    Ok(())
}

#[cfg(unix)]
fn apply_signal_handlers_to_process(
    default_signals: Option<&[EnvSignal]>,
    ignore_signals: Option<&[EnvSignal]>,
    signal_masks: Option<&SignalMasks>,
    is_debug_printing: bool,
) {
    let mut disposition_changes = Vec::new();
    if let Some(sigs) = default_signals {
        for &sig in sigs {
            if apply_signal_disposition(sig, SignalDisposition::Default) {
                disposition_changes.push((sig, "DEFAULT"));
            } else if sig == libc::SIGKILL || sig == libc::SIGSTOP {
                disposition_changes.push((sig, "DEFAULT (failure ignored)"));
            }
        }
    }
    if let Some(sigs) = ignore_signals {
        for &sig in sigs {
            if apply_signal_disposition(sig, SignalDisposition::Ignore) {
                disposition_changes.push((sig, "IGNORE"));
            } else if sig == libc::SIGKILL || sig == libc::SIGSTOP {
                disposition_changes.push((sig, "IGNORE (failure ignored)"));
            }
        }
    }
    if is_debug_printing {
        disposition_changes.sort_unstable_by_key(|(sig, _)| *sig);
        for (sig, action) in disposition_changes {
            eprintln!("{}", signal_disposition_debug_message(sig, action));
        }
    }

    let mut mask_changes = Vec::new();
    if let Some(signal_masks) = signal_masks {
        let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
        unsafe { libc::sigemptyset(&mut set) };
        for &sig in &signal_masks.block {
            unsafe { libc::sigaddset(&mut set, sig) };
            mask_changes.push((sig, "BLOCK"));
        }
        unsafe { libc::sigprocmask(libc::SIG_BLOCK, &set, std::ptr::null_mut()) };
    }
    if let Some(signal_masks) = signal_masks {
        let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
        unsafe { libc::sigemptyset(&mut set) };
        for &sig in &signal_masks.unblock {
            unsafe { libc::sigaddset(&mut set, sig) };
            mask_changes.push((sig, "UNBLOCK"));
        }
        unsafe { libc::sigprocmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut()) };
    }
    if is_debug_printing {
        mask_changes.sort_unstable_by_key(|(sig, _)| *sig);
        for (sig, action) in mask_changes {
            eprintln!("{}", signal_mask_debug_message(sig, action));
        }
    }
}

#[cfg(unix)]
fn signal_handling_line(signal: EnvSignal, is_blocked: bool, is_ignored: bool) -> Option<String> {
    if !is_ignored && !is_blocked {
        return None;
    }

    let name = signal_name(signal).expect("known signal has a display name");
    let ignored = if is_ignored { "IGNORE" } else { "" };
    let blocked = if is_blocked { "BLOCK" } else { "" };
    let separator = if is_ignored && is_blocked { "," } else { "" };
    Some(format!(
        "{name:<10} ({signal:2}): {blocked}{separator}{ignored}"
    ))
}

#[cfg(unix)]
fn inherited_sigpipe_is_ignored() -> bool {
    INHERITED_SIGPIPE_HANDLER.load(Ordering::Relaxed) == libc::SIG_IGN
}

#[cfg(not(target_os = "linux"))]
fn inherited_sigpipe_is_ignored() -> bool {
    false
}

#[cfg(unix)]
fn is_ignored_signal_reportable(
    signal: EnvSignal,
    is_ignored: bool,
    explicitly_ignored: bool,
    inherited_sigpipe_ignored: bool,
) -> bool {
    is_ignored && (signal != libc::SIGPIPE || explicitly_ignored || inherited_sigpipe_ignored)
}

#[cfg(unix)]
fn list_signal_handling(options: &EnvOptions<'_>) {
    let mut old_set: libc::sigset_t = unsafe { std::mem::zeroed() };
    let _ = unsafe { libc::sigprocmask(0, std::ptr::null(), &mut old_set) };

    for sig in known_env_signals() {
        if sig == libc::SIGKILL || sig == libc::SIGSTOP {
            continue;
        }

        let is_blocked = unsafe { libc::sigismember(&old_set, sig) == 1 };

        let mut old_act: libc::sigaction = unsafe { std::mem::zeroed() };
        if unsafe { libc::sigaction(sig, std::ptr::null(), &mut old_act) } == 0 {
            let is_ignored = is_ignored_signal_reportable(
                sig,
                old_act.sa_sigaction == libc::SIG_IGN,
                options
                    .ignore_signals
                    .as_ref()
                    .is_some_and(|signals| signals.contains(&sig)),
                inherited_sigpipe_is_ignored(),
            );
            if let Some(line) = signal_handling_line(sig, is_blocked, is_ignored) {
                eprintln!("{line}");
            }
        }
    }
}

fn env_make_options(args_match: &clap::ArgMatches) -> CTResult<EnvOptions<'_>> {
    let ignore_env = args_match.get_count("ignore-environment") > 0;
    let line_ending = CtLineEnding::from_zero_flag(args_match.get_count("null") > 0);
    let running_directory = args_match
        .get_many::<OsString>("chdir")
        .and_then(Iterator::last)
        .map(|s| s.as_os_str());
    let files = match args_match.get_many::<OsString>("file") {
        Some(v) => v.map(|s| s.as_os_str()).collect(),
        None => Vec::with_capacity(0),
    };
    let unsets = match args_match.get_many::<OsString>("unset") {
        Some(v) => v.map(|s| s.as_os_str()).collect(),
        None => Vec::with_capacity(0),
    };

    #[cfg(unix)]
    let block_signals = get_signal_masks(args_match)?;
    #[cfg(unix)]
    let list_signal_handling = args_match.get_count("list-signal-handling") > 0;

    let mut opts = EnvOptions {
        ignore_env,
        line_ending,
        running_directory,
        files,
        unsets,
        sets: vec![],
        program: vec![],
        #[cfg(unix)]
        default_signals: None,
        #[cfg(unix)]
        ignore_signals: None,
        #[cfg(unix)]
        block_signals,
        #[cfg(unix)]
        list_signal_handling,
    };

    let mut begin_prog_opts = false;
    let mut dash_operand_seen = false;
    if let Some(mut iter) = args_match.get_many::<OsString>("vars") {
        while !begin_prog_opts {
            if let Some(opt) = iter.next() {
                if opt == "-" && !dash_operand_seen && opts.sets.is_empty() {
                    opts.ignore_env = true;
                    dash_operand_seen = true;
                } else {
                    begin_prog_opts = env_parse_name_value_opt(&mut opts, opt)?;
                }
            } else {
                break;
            }
        }
        for opt in iter {
            env_parse_program_opt(&mut opts, opt)?;
        }
    }
    #[cfg(unix)]
    {
        (opts.default_signals, opts.ignore_signals) =
            get_signal_dispositions_for_program(args_match, !opts.program.is_empty())?;
    }

    Ok(opts)
}

fn env_unset_debug_message(name: &OsStr) -> String {
    format!("unset:    {}", name.to_string_lossy())
}

fn env_apply_unset_env_vars(
    options: &EnvOptions<'_>,
    is_debug_printing: bool,
) -> Result<(), Box<dyn CTError>> {
    if options.ignore_env {
        return Ok(());
    }

    for opt_name in &options.unsets {
        if is_debug_printing {
            eprintln!("{}", env_unset_debug_message(opt_name));
        }
        let native_name = NativeStr::new(opt_name);
        if opt_name.is_empty()
            || native_name.contains(&'\0').unwrap()
            || native_name.contains(&'=').unwrap()
        {
            return Err(CtSimpleError::new(
                125,
                format!("cannot unset {}: Invalid argument", opt_name.quote()),
            ));
        }

        unsafe { env::remove_var(opt_name) };
    }
    Ok(())
}

fn env_validate_change_directory(options: &EnvOptions<'_>) -> Result<(), Box<dyn CTError>> {
    if options.program.is_empty() && options.running_directory.is_some() {
        return Err(CTsageError::new(
            125,
            "must specify command with --chdir (-C)".to_string(),
        ));
    }

    Ok(())
}

fn env_change_directory_debug_message(directory: &OsStr) -> String {
    format!("chdir:    {}", directory.quote())
}

fn env_apply_change_directory(options: &EnvOptions<'_>) -> Result<(), Box<dyn CTError>> {
    env_validate_change_directory(options)?;

    if let Some(d) = options.running_directory {
        match env::set_current_dir(d) {
            Ok(()) => d,
            Err(error) => {
                let err_msg = if error.kind() == std::io::ErrorKind::NotFound {
                    "No such file or directory".to_string()
                } else {
                    error.to_string()
                };

                return Err(CtSimpleError::new(
                    125,
                    format!("cannot change directory to {}: {}", d.quote(), err_msg),
                ));
            }
        };
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn env_set_empty_name_value(value: &OsStr) -> CTResult<()> {
    let mut assignment = Vec::with_capacity(value.as_bytes().len() + 1);
    assignment.push(b'=');
    assignment.extend_from_slice(value.as_bytes());
    let assignment = CString::new(assignment).expect("environment value must not contain NUL");
    let assignment = assignment.into_raw();

    if unsafe { libc::putenv(assignment) } == 0 {
        return Ok(());
    }

    unsafe {
        drop(CString::from_raw(assignment));
    }
    Err(CtSimpleError::new(
        125,
        format!(
            "cannot set empty environment variable name: {}",
            io::Error::last_os_error()
        ),
    ))
}

fn env_apply_specified_env_vars(options: &EnvOptions<'_>, is_debug_printing: bool) -> CTResult<()> {
    for (name, val) in &options.sets {
        if name.is_empty() {
            if is_debug_printing {
                eprintln!("setenv:   ={}", val.to_string_lossy());
            }
            #[cfg(target_os = "linux")]
            env_set_empty_name_value(val)?;
            #[cfg(not(target_os = "linux"))]
            ct_show_warning!("no name specified for value {}", val.quote());
            continue;
        }
        if is_debug_printing {
            eprintln!(
                "setenv:   {}={}",
                name.to_string_lossy(),
                val.to_string_lossy()
            );
        }
        unsafe { env::set_var(name, val) };
    }
    Ok(())
}

fn env_line_ending_string(line_ending: CtLineEnding) -> String {
    format!("{line_ending}")
}

fn env_snapshot_upsert(vars: &mut Vec<(OsString, OsString)>, name: &OsStr, value: &OsStr) {
    if let Some((_, existing)) = vars.iter_mut().find(|(existing, _)| existing == name) {
        *existing = value.to_os_string();
    } else {
        vars.push((name.to_os_string(), value.to_os_string()));
    }
}

fn env_snapshot_remove(vars: &mut Vec<(OsString, OsString)>, name: &OsStr) {
    vars.retain(|(existing, _)| existing != name);
}

fn env_snapshot_load_config_file(
    vars: &mut Vec<(OsString, OsString)>,
    options: &EnvOptions<'_>,
) -> CTResult<()> {
    for &file in &options.files {
        let config = if file == "-" {
            let stdin = io::stdin();
            let mut stdin_locked = stdin.lock();
            Ini::read_from(&mut stdin_locked)
        } else {
            Ini::load_from_file(file)
        };

        let config =
            config.map_err(|e| CtSimpleError::new(1, format!("{}: {}", file.quote(), e)))?;

        for (_, prop) in &config {
            for (key, value) in prop.iter() {
                env_snapshot_upsert(vars, OsStr::new(key), OsStr::new(value));
            }
        }
    }
    Ok(())
}

fn env_collect_snapshot_semantic(options: &EnvOptions<'_>) -> CTResult<EnvSemantic> {
    if options.running_directory.is_some() {
        env_apply_change_directory(options)?;
    }

    if !options.program.is_empty() {
        return Err(CTsageError::new(
            125,
            "data env does not yet support command execution",
        ));
    }

    let mut vars = if options.ignore_env {
        Vec::new()
    } else {
        env::vars_os().collect::<Vec<_>>()
    };

    env_snapshot_load_config_file(&mut vars, options)?;

    for opt_name in &options.unsets {
        let native_name = NativeStr::new(opt_name);
        if opt_name.is_empty()
            || native_name.contains(&'\0').unwrap()
            || native_name.contains(&'=').unwrap()
        {
            return Err(CtSimpleError::new(
                125,
                format!("cannot unset {}: Invalid argument", opt_name.quote()),
            ));
        }
        env_snapshot_remove(&mut vars, opt_name);
    }

    #[cfg(target_os = "linux")]
    let stderr_text = String::new();
    #[cfg(not(target_os = "linux"))]
    let mut stderr_text = String::new();
    for (name, val) in &options.sets {
        if name.is_empty() {
            #[cfg(target_os = "linux")]
            env_snapshot_upsert(&mut vars, name, val);
            #[cfg(not(target_os = "linux"))]
            stderr_text.push_str(&format!(
                "env: warning: no name specified for value {}\n",
                val.quote()
            ));
            continue;
        }
        env_snapshot_upsert(&mut vars, name, val);
    }

    let line_ending = env_line_ending_string(options.line_ending);
    let rows = vars
        .iter()
        .map(|(name, value)| EnvRow {
            name: name.to_string_lossy().into_owned(),
            value: value.to_string_lossy().into_owned(),
        })
        .collect::<Vec<_>>();

    let mut classic_text = String::new();
    for row in &rows {
        classic_text.push_str(&row.name);
        classic_text.push('=');
        classic_text.push_str(&row.value);
        classic_text.push_str(&line_ending);
    }

    Ok(EnvSemantic {
        rows,
        classic_text,
        stderr_text,
        exit_code: 0,
    })
}

#[derive(Default)]
pub struct Env;
impl Tool for Env {
    fn name(&self) -> &'static str {
        "env"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        env_main(args.iter().cloned())
    }
}

pub fn env_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    EnvAppData::default().run_env(args)
}

pub fn env_native_semantic(args: impl ctcore::Args) -> CTResult<EnvSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let mut app_data = EnvAppData::default();
    let (_source_args, matches) = app_data.parse_arguments(args)?;
    let options = env_make_options(&matches)?;
    env_collect_snapshot_semantic(&options)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[cfg(target_os = "linux")]
    #[test]
    fn test_write_environment_entry_preserves_non_utf8_bytes() {
        let entry = c"GOOD=value\xff";
        let mut output = Vec::new();

        write_environment_entry(&mut output, entry, CtLineEnding::Newline).unwrap();

        assert_eq!(output, b"GOOD=value\xff\n");
    }

    #[test]
    fn test_chdir_debug_message_uses_gnu_quote_layout() {
        assert_eq!(
            env_change_directory_debug_message(OsStr::new("/tmp/env-directory")),
            "chdir:    '/tmp/env-directory'"
        );
    }

    #[test]
    fn test_null_with_command_is_validated_after_option_parsing() {
        let matches = ct_app()
            .try_get_matches_from([ctcore::ct_util_name(), "-0", "A=1", "/usr/bin/true"])
            .unwrap();

        let options = env_make_options(&matches).unwrap();

        assert_eq!(options.line_ending, CtLineEnding::Nul);
        assert_eq!(
            options.sets,
            vec![(
                Cow::Borrowed(OsStr::new("A")),
                Cow::Borrowed(OsStr::new("1"))
            )]
        );
        assert_eq!(options.program, vec![OsStr::new("/usr/bin/true")]);
    }

    #[test]
    fn test_short_options_enable_debug_before_split_string() {
        assert!(env_short_options_enable_debug(OsStr::new("-vS")));
        assert!(env_short_options_enable_debug(OsStr::new("-viS")));
        assert!(env_short_options_enable_debug(OsStr::new("-vuNAME")));
        assert!(!env_short_options_enable_debug(OsStr::new("-S")));
        assert!(!env_short_options_enable_debug(OsStr::new("-vinvalid")));
    }

    #[test]
    fn test_shebang_whitespace_option_detection() {
        assert_eq!(
            env_shebang_whitespace_option(OsStr::new("-iv /usr/bin/true")),
            Some(b' ')
        );
        assert_eq!(
            env_shebang_whitespace_option(OsStr::new("-0\t/usr/bin/true")),
            Some(b'\t')
        );
        assert_eq!(env_shebang_whitespace_option(OsStr::new("-S arg")), None);
        assert_eq!(env_shebang_whitespace_option(OsStr::new("-q arg")), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_parse_signal_accepts_poll_alias() {
        assert_eq!(parse_signal("POLL").unwrap(), libc::SIGIO);
        assert_eq!(parse_signal("SIGPOLL").unwrap(), libc::SIGIO);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_parse_signal_accepts_linux_realtime_signal_names() {
        assert_eq!(parse_signal("RTMIN").unwrap(), libc::SIGRTMIN());
        assert_eq!(parse_signal("SIGRTMIN+1").unwrap(), libc::SIGRTMIN() + 1);
        assert_eq!(parse_signal("RTMAX").unwrap(), libc::SIGRTMAX());
        assert_eq!(parse_signal("SIGRTMAX-1").unwrap(), libc::SIGRTMAX() - 1);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_signal_debug_messages_use_gnu_signal_names() {
        assert_eq!(
            signal_disposition_debug_message(libc::SIGHUP, "IGNORE"),
            "Reset signal HUP (1) to IGNORE"
        );
        assert_eq!(
            signal_mask_debug_message(libc::SIGRTMIN(), "BLOCK"),
            format!("signal RTMIN ({}) mask set to BLOCK", libc::SIGRTMIN())
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_signal_name_uses_poll_for_linux_signal_29() {
        assert_eq!(signal_name(libc::SIGIO), Some("POLL".to_owned()));
        assert_eq!(
            signal_disposition_debug_message(libc::SIGIO, "IGNORE"),
            "Reset signal POLL (29) to IGNORE"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_signal_handling_line_reports_inherited_ignored_sigpipe() {
        assert_eq!(
            signal_handling_line(libc::SIGPIPE, false, true),
            Some("PIPE       (13): IGNORE".to_owned())
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_inherited_ignored_sigpipe_is_reported_but_rust_runtime_state_is_not() {
        assert!(!is_ignored_signal_reportable(
            libc::SIGPIPE,
            true,
            false,
            false
        ));
        assert!(is_ignored_signal_reportable(
            libc::SIGPIPE,
            true,
            false,
            true
        ));
        assert!(is_ignored_signal_reportable(
            libc::SIGPIPE,
            true,
            true,
            false
        ));
    }

    #[cfg(unix)]
    #[test]
    fn test_parse_signal_list_rejects_whitespace_in_operand() {
        let error = parse_signal_list("HUP, INT").unwrap_err();

        assert_eq!(error.code(), 125);
        assert_eq!(error.to_string(), "invalid signal ' INT'");
    }

    #[cfg(unix)]
    #[test]
    fn test_explicit_empty_signal_arguments_are_noops() {
        let matches = ct_app()
            .try_get_matches_from([ctcore::ct_util_name(), "--ignore-signal=", "true"])
            .unwrap();
        let (default_signals, ignore_signals) = get_signal_dispositions(&matches).unwrap();

        assert_eq!(default_signals, None);
        assert_eq!(ignore_signals, Some(vec![]));

        let matches = ct_app()
            .try_get_matches_from([ctcore::ct_util_name(), "--block-signal=", "true"])
            .unwrap();

        assert_eq!(
            get_signal_masks(&matches).unwrap(),
            Some(SignalMasks {
                block: vec![],
                unblock: vec![],
            })
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_signal_options_without_arguments_include_realtime_signals() {
        let matches = ct_app()
            .try_get_matches_from([ctcore::ct_util_name(), "--block-signal", "true"])
            .unwrap();
        let blocked = get_signal_masks(&matches).unwrap().unwrap();
        assert!(blocked.block.contains(&libc::SIGRTMIN()));
        assert!(blocked.block.contains(&libc::SIGRTMAX()));
        assert!(blocked.block.contains(&libc::SIGKILL));
        assert!(blocked.block.contains(&libc::SIGSTOP));

        let matches = ct_app()
            .try_get_matches_from([ctcore::ct_util_name(), "--ignore-signal", "true"])
            .unwrap();
        let (_, ignored) = get_signal_dispositions(&matches).unwrap();
        let ignored = ignored.unwrap();
        assert!(ignored.contains(&libc::SIGRTMIN()));
        assert!(ignored.contains(&libc::SIGRTMAX()));
        assert!(ignored.contains(&libc::SIGKILL));
        assert!(ignored.contains(&libc::SIGSTOP));

        let matches = ct_app()
            .try_get_matches_from([ctcore::ct_util_name(), "--default-signal", "true"])
            .unwrap();
        let (defaults, _) = get_signal_dispositions(&matches).unwrap();
        let defaults = defaults.unwrap();
        assert!(defaults.contains(&libc::SIGKILL));
        assert!(defaults.contains(&libc::SIGSTOP));
    }

    #[cfg(unix)]
    #[test]
    fn test_default_signal_removes_prior_signal_from_block_mask() {
        let matches = ct_app()
            .try_get_matches_from([
                ctcore::ct_util_name(),
                "--block-signal=HUP",
                "--default-signal=HUP",
                "true",
            ])
            .unwrap();

        assert_eq!(
            get_signal_masks(&matches).unwrap(),
            Some(SignalMasks {
                block: vec![],
                unblock: vec![libc::SIGHUP],
            })
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_default_signal_keeps_unblock_mask_when_ignore_overrides_disposition() {
        let matches = ct_app()
            .try_get_matches_from([
                ctcore::ct_util_name(),
                "--default-signal=HUP",
                "--ignore-signal=HUP",
                "true",
            ])
            .unwrap();

        assert_eq!(
            get_signal_masks(&matches).unwrap(),
            Some(SignalMasks {
                block: vec![],
                unblock: vec![libc::SIGHUP],
            })
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_explicit_immutable_signal_actions_fail() {
        for (option, number) in [("--ignore-signal=KILL", 9), ("--default-signal=STOP", 19)] {
            let matches = ct_app()
                .try_get_matches_from([ctcore::ct_util_name(), option, "true"])
                .unwrap();
            let error = get_signal_dispositions(&matches).unwrap_err();

            assert_eq!(error.code(), 125, "{option}");
            assert_eq!(
                error.to_string(),
                format!("failed to set signal action for signal {number}: Invalid argument"),
                "{option}"
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_immutable_signal_actions_are_ignored_without_a_program() {
        let matches = ct_app()
            .try_get_matches_from([ctcore::ct_util_name(), "--ignore-signal=KILL"])
            .unwrap();

        let (_, ignored) = get_signal_dispositions_for_program(&matches, false).unwrap();

        assert_eq!(ignored, Some(vec![libc::SIGKILL]));
    }

    #[test]
    fn test_ignore_environment_skips_unset_validation() {
        let options = EnvOptions {
            ignore_env: true,
            line_ending: CtLineEnding::Newline,
            running_directory: None,
            files: vec![],
            unsets: vec![OsStr::new("A=B")],
            sets: vec![],
            program: vec![],
            #[cfg(unix)]
            default_signals: None,
            #[cfg(unix)]
            ignore_signals: None,
            #[cfg(unix)]
            block_signals: None,
            #[cfg(unix)]
            list_signal_handling: false,
        };

        assert!(env_apply_unset_env_vars(&options, false).is_ok());
    }

    #[test]
    fn test_unset_debug_message_uses_gnu_layout() {
        assert_eq!(
            env_unset_debug_message(OsStr::new("TEST_VAR")),
            "unset:    TEST_VAR"
        );
    }

    #[test]
    fn test_semantic_accepts_empty_environment_name() {
        let matches = ct_app()
            .try_get_matches_from([ctcore::ct_util_name(), "-i", "=value"])
            .unwrap();
        let options = env_make_options(&matches).unwrap();

        let semantic = env_collect_snapshot_semantic(&options).unwrap();

        assert_eq!(semantic.classic_text, "=value\n");
        assert_eq!(semantic.stderr_text, "");
    }

    #[cfg(unix)]
    #[test]
    fn test_repeated_standard_options_are_accepted() {
        let matches = ct_app()
            .try_get_matches_from([
                ctcore::ct_util_name(),
                "-i",
                "-i",
                "-0",
                "-0",
                "-v",
                "-v",
                "--list-signal-handling",
                "--list-signal-handling",
                "-C",
                "/",
                "-C",
                "/tmp",
            ])
            .unwrap();

        let options = env_make_options(&matches).unwrap();

        assert!(options.ignore_env);
        assert_eq!(options.line_ending, CtLineEnding::Nul);
        assert_eq!(options.running_directory, Some(OsStr::new("/tmp")));
        assert!(options.list_signal_handling);
    }

    #[test]
    fn test_only_first_dash_operand_clears_environment() {
        let matches = ct_app()
            .try_get_matches_from([ctcore::ct_util_name(), "-", "A=B", "-"])
            .unwrap();
        let options = env_make_options(&matches).unwrap();

        assert!(options.ignore_env);
        assert_eq!(
            options.sets,
            vec![(
                Cow::Borrowed(OsStr::new("A")),
                Cow::Borrowed(OsStr::new("B"))
            )]
        );
        assert_eq!(options.program, vec![OsStr::new("-")]);
    }

    #[test]
    fn test_split_string_rejects_invalid_gnu_variable_syntax() {
        for input in ["A=$FOO", "A=${MISSING:default}", "A=${}", "A=${é}"] {
            assert!(
                env_parse_args_from_str(&NCvt::convert(input)).is_err(),
                "{input}"
            );
        }
    }

    #[test]
    fn test_command_execution_error_exit_codes() {
        assert_eq!(
            command_execution_error_exit_code(&io::Error::from(io::ErrorKind::NotFound)),
            127
        );
        assert_eq!(
            command_execution_error_exit_code(&io::Error::from(io::ErrorKind::InvalidInput)),
            127
        );
        assert_eq!(
            command_execution_error_exit_code(&io::Error::from(io::ErrorKind::PermissionDenied)),
            126
        );
    }

    #[test]
    fn test_command_execution_error_message_uses_errno_text() {
        let error = io::Error::from_raw_os_error(libc::ENOTDIR);

        assert_eq!(
            command_execution_error_message(OsStr::new("/tmp/not-a-directory/command"), &error),
            "'/tmp/not-a-directory/command': Not a directory"
        );
    }

    #[test]
    fn test_command_name_c_whitespace_detection() {
        assert!(env_contains_c_whitespace(OsStr::new("command with space")));
        assert!(env_contains_c_whitespace(OsStr::new("command\twith-tab")));
        assert!(!env_contains_c_whitespace(OsStr::new(
            "command-without-whitespace"
        )));
    }

    #[cfg(unix)]
    #[test]
    fn test_signal_dispositions_follow_argument_order() {
        let matches = ct_app()
            .try_get_matches_from([
                ctcore::ct_util_name(),
                "--ignore-signal=HUP",
                "--default-signal=HUP",
                "true",
            ])
            .unwrap();
        let (default_signals, ignore_signals) = get_signal_dispositions(&matches).unwrap();
        assert_eq!(default_signals, Some(vec![libc::SIGHUP]));
        assert_eq!(ignore_signals, Some(vec![]));

        let matches = ct_app()
            .try_get_matches_from([
                ctcore::ct_util_name(),
                "--default-signal=HUP",
                "--ignore-signal=HUP",
                "true",
            ])
            .unwrap();
        let (default_signals, ignore_signals) = get_signal_dispositions(&matches).unwrap();
        assert_eq!(default_signals, Some(vec![]));
        assert_eq!(ignore_signals, Some(vec![libc::SIGHUP]));
    }

    #[test]
    fn test_tool_implementation() {
        let tool = Env;

        // 测试 name 方法
        assert_eq!(tool.name(), "env");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("env"));

        // 测试 execute 方法
        let args = [OsString::from("env"), OsString::from("--version")];
        assert!(tool.execute(&args).is_err());
    }
    mod tests_env_main {
        use crate::env_main;

        use std::ffi::OsString;
        use std::fs;
        use std::fs::File;
        use std::io::Write;

        use tempfile::Builder;

        #[test]
        fn test_env_main_version() {
            let args = [ctcore::ct_util_name(), "--version"];
            let result = env_main(args.iter().map(OsString::from));
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 0);
        }

        #[test]
        fn test_env_main_v() {
            let args = [ctcore::ct_util_name(), "-V"];
            let result = env_main(args.iter().map(OsString::from));
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 0);
        }

        #[test]
        fn test_env_main_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = env_main(args.iter().map(OsString::from));
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 0);
        }

        #[test]
        fn test_env_main_h() {
            let args = [ctcore::ct_util_name(), "-h"];
            let result = env_main(args.iter().map(OsString::from));
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 0);
        }

        #[test]
        fn test_env_main_i() {
            let args = [ctcore::ct_util_name(), "-i", "arch"];
            let result = env_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_env_main_ignore_environment() {
            let args = [ctcore::ct_util_name(), "--ignore-environment", "arch"];
            let result = env_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_env_main_bad_args() {
            let args = [ctcore::ct_util_name(), "--bad-arg"];
            let result = env_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_env_main_chdir_args() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--chdir", env_dir, "ls"];

            let result = env_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_env_main_c_args() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-C", env_dir, "ls"];

            let result = env_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_env_main_0() {
            let args = [ctcore::ct_util_name(), "-0"];
            let result = env_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_env_main_null() {
            let args = [ctcore::ct_util_name(), "--null"];
            let result = env_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        use crate::env;

        #[test]
        fn test_env_main_f() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let f = test_file_1.to_str().unwrap();

            let content = "FVAR1=hello\n\
           FVAR2=CtyunOS\n\
           FVAR3=Rust\n\
           FVAR4=Syskit\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "-f", f];

            let result = env_main(args.iter().map(OsString::from));
            assert!(result.is_ok());

            match env::var("FVAR1") {
                Ok(val) => {
                    assert_eq!(val, "hello");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("FVAR2") {
                Ok(val) => {
                    assert_eq!(val, "CtyunOS");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("FVAR3") {
                Ok(val) => {
                    assert_eq!(val, "Rust");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("FVAR4") {
                Ok(val) => {
                    assert_eq!(val, "Syskit");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }
        }

        #[test]
        fn test_env_main_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let content = "ENV_VAR1=hello\n\
           ENV_VAR2=CtyunOS\n\
           ENV_VAR3=Rust\n\
           ENV_VAR4=Syskit\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "--file", filename, "ls"];

            let result = env_main(args.iter().map(OsString::from));
            assert!(result.is_ok());

            match env::var("ENV_VAR1") {
                Ok(val) => {
                    assert_eq!(val, "hello");
                }
                Err(e) => {
                    println!("env set fail:{e}")
                }
            }

            match env::var("ENV_VAR2") {
                Ok(val) => {
                    assert_eq!(val, "CtyunOS");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("ENV_VAR3") {
                Ok(val) => {
                    assert_eq!(val, "Rust");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("ENV_VAR4") {
                Ok(val) => {
                    assert_eq!(val, "Syskit");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }
        }

        #[test]
        fn test_env_main_file_unset() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "Y_UNSET_VAR1=hello\n\
           Y_UNSET_VAR2=CtyunOS\n\
           Y_UNSET_VAR3=Rust\n\
           Y_UNSET_VAR4=Syskit\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "--file", unset_filename, "env"];

            let result = env_main(args.iter().map(OsString::from));
            assert!(result.is_ok());

            match env::var("Y_UNSET_VAR1") {
                Ok(val) => {
                    assert_eq!(val, "hello");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("Y_UNSET_VAR2") {
                Ok(val) => {
                    assert_eq!(val, "CtyunOS");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("Y_UNSET_VAR3") {
                Ok(val) => {
                    assert_eq!(val, "Rust");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("Y_UNSET_VAR4") {
                Ok(val) => {
                    assert_eq!(val, "Syskit");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            let args = [
                ctcore::ct_util_name(),
                "-i",
                "env",
                "-u",
                unset_filename,
                "env",
            ];

            let result = env_main(args.iter().map(OsString::from));
            assert!(result.is_ok());

            match env::var("Y_UNSET_VAR1") {
                Ok(val) => {
                    assert_eq!(val, "hello");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("Y_UNSET_VAR2") {
                Ok(val) => {
                    assert_eq!(val, "CtyunOS");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("Y_UNSET_VAR3") {
                Ok(val) => {
                    assert_eq!(val, "Rust");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("Y_UNSET_VAR4") {
                Ok(val) => {
                    assert_eq!(val, "Syskit");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }
        }

        #[test]
        fn test_env_main_file_unset_whole() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "Q_UNSET_VAR1=hello\n\
           Q_UNSET_VAR2=CtyunOS\n\
           Q_UNSET_VAR3=Rust\n\
           Q_UNSET_VAR4=Syskit\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "--file", unset_filename, "env"];

            let result = env_main(args.iter().map(OsString::from));
            assert!(result.is_ok());

            match env::var("Q_UNSET_VAR1") {
                Ok(val) => {
                    assert_eq!(val, "hello");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("Q_UNSET_VAR2") {
                Ok(val) => {
                    assert_eq!(val, "CtyunOS");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("Q_UNSET_VAR3") {
                Ok(val) => {
                    assert_eq!(val, "Rust");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("Q_UNSET_VAR4") {
                Ok(val) => {
                    assert_eq!(val, "Syskit");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            let args = [
                ctcore::ct_util_name(),
                "--ignore-environment",
                "env",
                "--unset",
                unset_filename,
                "env",
            ];

            let result = env_main(args.iter().map(OsString::from));
            assert!(result.is_ok());

            match env::var("Q_UNSET_VAR1") {
                Ok(val) => {
                    assert_eq!(val, "hello");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("Q_UNSET_VAR2") {
                Ok(val) => {
                    assert_eq!(val, "CtyunOS");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("Q_UNSET_VAR3") {
                Ok(val) => {
                    assert_eq!(val, "Rust");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("Q_UNSET_VAR4") {
                Ok(val) => {
                    assert_eq!(val, "Syskit");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }
        }

        #[test]
        fn test_env_main_debug() {
            let args = [ctcore::ct_util_name(), "-v", "arch"];
            let result = env_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_env_main_debug_whole() {
            let args = [ctcore::ct_util_name(), "--debug", "arch"];
            let result = env_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_env_main_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "--split-string=", "cat", file_path];

            let result = env_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_env_main_split_s() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "-S", "", "cat", file_path];

            let result = env_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_env_main_s_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                "-S",
                "--split-string=",
                "cat",
                file_path,
            ];

            let result = env_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }
    }

    mod tests_run_env {
        use crate::EnvAppData;

        use std::ffi::OsString;
        use std::fs;
        use std::fs::File;
        use std::io::Write;

        use tempfile::Builder;

        #[test]
        fn test_run_env_version() {
            let cmd = [ctcore::ct_util_name(), "--version"];
            let args = cmd.iter().map(OsString::from);

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 0);
        }

        #[test]
        fn test_run_env_v() {
            let cmd = [ctcore::ct_util_name(), "-V"];
            let args = cmd.iter().map(OsString::from);

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 0);
        }

        #[test]
        fn test_run_env_help() {
            let cmd = [ctcore::ct_util_name(), "--help"];
            let args = cmd.iter().map(OsString::from);

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 0);
        }

        #[test]
        fn test_run_env_h() {
            let cmd = [ctcore::ct_util_name(), "-h"];
            let args = cmd.iter().map(OsString::from);

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 0);
        }

        #[test]
        fn test_run_env_i() {
            let cmd = [ctcore::ct_util_name(), "-i", "arch"];
            let args = cmd.iter().map(OsString::from);

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_run_env_ignore_environment() {
            let cmd = [ctcore::ct_util_name(), "--ignore-environment", "arch"];
            let args = cmd.iter().map(OsString::from);

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_run_env_bad_cmd() {
            let cmd = [ctcore::ct_util_name(), "--bad-arg"];
            let args = cmd.iter().map(OsString::from);

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_err());
        }

        #[test]
        fn test_run_env_chdir_cmd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let cmd = [ctcore::ct_util_name(), "--chdir", env_dir, "ls"];

            let args = cmd.iter().map(OsString::from);

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_run_env_c_cmd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let cmd = [ctcore::ct_util_name(), "-C", env_dir, "ls"];

            let args = cmd.iter().map(OsString::from);

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_run_env_0() {
            let cmd = [ctcore::ct_util_name(), "-0"];
            let args = cmd.iter().map(OsString::from);

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_run_env_null() {
            let cmd = [ctcore::ct_util_name(), "--null"];
            let args = cmd.iter().map(OsString::from);

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());
        }

        use crate::env;

        #[test]
        fn test_run_env_f() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let f = test_file_1.to_str().unwrap();

            let content = "FVAR1=hello\n\
           FVAR2=CtyunOS\n\
           FVAR3=Rust\n\
           FVAR4=Syskit\n";
            file.write_all(content.as_bytes()).unwrap();

            let cmd = [ctcore::ct_util_name(), "-f", f];

            let args = cmd.iter().map(OsString::from);

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());

            match env::var("FVAR1") {
                Ok(val) => {
                    assert_eq!(val, "hello");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("FVAR2") {
                Ok(val) => {
                    assert_eq!(val, "CtyunOS");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("FVAR3") {
                Ok(val) => {
                    assert_eq!(val, "Rust");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("FVAR4") {
                Ok(val) => {
                    assert_eq!(val, "Syskit");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }
        }

        #[test]
        fn test_run_env_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let content = "ENV_VAR1=hello\n\
           ENV_VAR2=CtyunOS\n\
           ENV_VAR3=Rust\n\
           ENV_VAR4=Syskit\n";
            file.write_all(content.as_bytes()).unwrap();

            let cmd = [ctcore::ct_util_name(), "--file", filename, "ls"];

            let args = cmd.iter().map(OsString::from);

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());

            match env::var("ENV_VAR1") {
                Ok(val) => {
                    assert_eq!(val, "hello");
                }
                Err(e) => {
                    println!("env set fail:{e}")
                }
            }

            match env::var("ENV_VAR2") {
                Ok(val) => {
                    assert_eq!(val, "CtyunOS");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("ENV_VAR3") {
                Ok(val) => {
                    assert_eq!(val, "Rust");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("ENV_VAR4") {
                Ok(val) => {
                    assert_eq!(val, "Syskit");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }
        }

        #[test]
        fn test_run_env_file_unset() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "QQ_UNSET_VAR1=hello\n\
           QQ_UNSET_VAR2=CtyunOS\n\
           QQ_UNSET_VAR3=Rust\n\
           QQ_UNSET_VAR4=Syskit\n";
            file.write_all(content.as_bytes()).unwrap();

            let cmd = [ctcore::ct_util_name(), "--file", unset_filename, "env"];

            let args = cmd.iter().map(OsString::from);

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());

            match env::var("QQ_UNSET_VAR1") {
                Ok(val) => {
                    assert_eq!(val, "hello");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("QQ_UNSET_VAR2") {
                Ok(val) => {
                    assert_eq!(val, "CtyunOS");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("QQ_UNSET_VAR3") {
                Ok(val) => {
                    assert_eq!(val, "Rust");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("QQ_UNSET_VAR4") {
                Ok(val) => {
                    assert_eq!(val, "Syskit");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            let cmd = [
                ctcore::ct_util_name(),
                "-i",
                "env",
                "-u",
                unset_filename,
                "env",
            ];

            let args = cmd.iter().map(OsString::from);

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());

            match env::var("QQ_UNSET_VAR1") {
                Ok(val) => {
                    assert_ne!(val, "hello");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("QQ_UNSET_VAR2") {
                Ok(val) => {
                    assert_ne!(val, "CtyunOS");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("QQ_UNSET_VAR3") {
                Ok(val) => {
                    assert_ne!(val, "Rust");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("QQ_UNSET_VAR4") {
                Ok(val) => {
                    assert_ne!(val, "Syskit");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }
        }

        #[test]
        fn test_run_env_file_unset_whole() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "YY_UNSET_VAR1=hello\n\
           YY_UNSET_VAR2=CtyunOS\n\
           YY_UNSET_VAR3=Rust\n\
           YY_UNSET_VAR4=Syskit\n";
            file.write_all(content.as_bytes()).unwrap();

            let cmd = [ctcore::ct_util_name(), "--file", unset_filename, "env"];

            let args = cmd.iter().map(OsString::from);

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());

            match env::var("YY_UNSET_VAR1") {
                Ok(val) => {
                    assert_eq!(val, "hello");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("YY_UNSET_VAR2") {
                Ok(val) => {
                    assert_eq!(val, "CtyunOS");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("YY_UNSET_VAR3") {
                Ok(val) => {
                    assert_eq!(val, "Rust");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("YY_UNSET_VAR4") {
                Ok(val) => {
                    assert_eq!(val, "Syskit");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            let cmd = [
                ctcore::ct_util_name(),
                "--ignore-environment",
                "env",
                "--unset",
                unset_filename,
                "env",
            ];

            let args = cmd.iter().map(OsString::from);

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());

            match env::var("YY_UNSET_VAR1") {
                Ok(val) => {
                    assert_ne!(val, "hello");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("YY_UNSET_VAR2") {
                Ok(val) => {
                    assert_ne!(val, "CtyunOS");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("YY_UNSET_VAR3") {
                Ok(val) => {
                    assert_ne!(val, "Rust");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("YY_UNSET_VAR4") {
                Ok(val) => {
                    assert_ne!(val, "Syskit");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }
        }

        #[test]
        fn test_run_env_debug() {
            let cmd = [ctcore::ct_util_name(), "-v", "arch"];
            let args = cmd.iter().map(OsString::from);

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_run_env_debug_whole() {
            let cmd = [ctcore::ct_util_name(), "--debug", "arch"];
            let args = cmd.iter().map(OsString::from);

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_run_env_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let cmd = [ctcore::ct_util_name(), "--split-string=", "cat", file_path];

            let args = cmd.iter().map(OsString::from);

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_run_env_split_s() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let cmd = [ctcore::ct_util_name(), "-S", "", "cat", file_path];

            let args = cmd.iter().map(OsString::from);

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_run_env_s_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let cmd = [
                ctcore::ct_util_name(),
                "-S",
                "--split-string=",
                "cat",
                file_path,
            ];

            let args = cmd.iter().map(OsString::from);

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.run_env(args);
            assert!(result.is_ok());
        }
    }

    mod tests_parse_arguments {
        use crate::EnvAppData;

        use std::ffi::OsString;
        use std::fs;
        use std::fs::File;
        use std::io::Write;

        use tempfile::Builder;

        #[test]
        fn test_parse_arguments_version() {
            let cmd = [ctcore::ct_util_name(), "--version"];
            let args = cmd.iter().map(OsString::from);

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.parse_arguments(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 0);
        }

        #[test]
        fn test_parse_arguments_v() {
            let cmd = [ctcore::ct_util_name(), "-V"];
            let args = cmd.iter().map(OsString::from);

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.parse_arguments(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 0);
        }

        #[test]
        fn test_parse_arguments_help() {
            let cmd = [ctcore::ct_util_name(), "--help"];
            let args = cmd.iter().map(OsString::from);

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.parse_arguments(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 0);
        }

        #[test]
        fn test_parse_arguments_h() {
            let cmd = [ctcore::ct_util_name(), "-h"];
            let args = cmd.iter().map(OsString::from);

            let mut env_app_data = EnvAppData::default();

            let result = env_app_data.parse_arguments(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 0);
        }

        #[test]
        fn test_parse_arguments_i() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-i"), OsString::from("arch")];

            let expected_args = vec![OsString::from("-i"), OsString::from("arch")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }

        #[test]
        fn test_parse_arguments_ignore_environment() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--ignore-environment"),
                OsString::from("arch"),
            ];

            let expected_args = vec![
                OsString::from("--ignore-environment"),
                OsString::from("arch"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }

        #[test]
        fn test_parse_arguments_split_string_args() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("-S"),
                OsString::from("arg2"),
                OsString::from("-vS"),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("-S"),
                OsString::from("arg2"),
                OsString::from("-vS"),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }

        #[test]
        fn test_parse_arguments_chdir_cmd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--chdir"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let expected_args = vec![
                OsString::from("--chdir"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }

        #[test]
        fn test_parse_arguments_c_cmd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-C"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let expected_args = vec![
                OsString::from("-C"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }

        #[test]
        fn test_parse_arguments_0() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-0")];

            let expected_args = vec![OsString::from("-0")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }

        #[test]
        fn test_parse_arguments_null() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("--null")];

            let expected_args = vec![OsString::from("--null")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }

        #[test]
        fn test_parse_arguments_f() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let f = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-f"), OsString::from(f)];

            let expected_args = vec![OsString::from("-f"), OsString::from(f)];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }

        #[test]
        fn test_parse_arguments_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let f = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("--file"), OsString::from(f)];

            let expected_args = vec![OsString::from("--file"), OsString::from(f)];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }

        #[test]
        fn test_parse_arguments_unset() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--unset"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("--unset"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }

        #[test]
        fn test_parse_arguments_u() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-u"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("-u"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }

        #[test]
        fn test_parse_arguments_debug_whole() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--debug"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("--debug"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }

        #[test]
        fn test_parse_arguments_debug() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-v"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("-v"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }
        #[test]
        fn test_parse_arguments_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }
        #[test]
        fn test_parse_arguments_s_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-S"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("-S"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }
        #[test]
        fn test_parse_arguments_s_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-S"),
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("-S"),
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);
        }
    }

    mod tests_process_all_string_arguments {
        use crate::EnvAppData;

        use std::ffi::OsString;
        use std::fs;
        use std::fs::File;
        use std::io::Write;

        use tempfile::Builder;

        #[test]
        fn test_process_all_string_arguments_combined_i_s() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-iS"), OsString::from("A=1")];

            assert_eq!(
                env_app_data
                    .process_all_string_arguments(&original_args)
                    .unwrap(),
                vec![OsString::from("-i"), OsString::from("A=1")]
            );
        }

        #[test]
        fn test_process_all_string_arguments_i() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-i"), OsString::from("arch")];

            let expected_args = vec![OsString::from("-i"), OsString::from("arch")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args);
        }

        #[test]
        fn test_process_all_string_arguments_ignore_environment() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--ignore-environment"),
                OsString::from("arch"),
            ];

            let expected_args = vec![
                OsString::from("--ignore-environment"),
                OsString::from("arch"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args);
        }

        #[test]
        fn test_process_all_string_arguments_split_string_args() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("-S"),
                OsString::from("arg2"),
                OsString::from("-vS"),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("-S"),
                OsString::from("arg2"),
                OsString::from("-vS"),
                OsString::from("arg3"),
            ];

            let expected_args_2 = vec![
                OsString::from("arg1"),
                OsString::from("-S"),
                OsString::from("arg2"),
                OsString::from("-vS"),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args_2);
        }

        #[test]
        fn test_process_all_string_arguments_chdir_cmd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--chdir"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let expected_args = vec![
                OsString::from("--chdir"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args);
        }

        #[test]
        fn test_process_all_string_arguments_c_cmd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-C"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let expected_args = vec![
                OsString::from("-C"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args);
        }

        #[test]
        fn test_process_all_string_arguments_0() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-0")];

            let expected_args = vec![OsString::from("-0")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args);
        }

        #[test]
        fn test_process_all_string_arguments_null() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("--null")];

            let expected_args = vec![OsString::from("--null")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args);
        }

        #[test]
        fn test_process_all_string_arguments_f() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let f = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-f"), OsString::from(f)];

            let expected_args = vec![OsString::from("-f"), OsString::from(f)];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args);
        }

        #[test]
        fn test_process_all_string_arguments_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let f = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("--file"), OsString::from(f)];

            let expected_args = vec![OsString::from("--file"), OsString::from(f)];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args);
        }

        #[test]
        fn test_process_all_string_arguments_unset() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--unset"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("--unset"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args);
        }

        #[test]
        fn test_process_all_string_arguments_u() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-u"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("-u"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args);
        }

        #[test]
        fn test_process_all_string_arguments_debug_whole() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--debug"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("--debug"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args);
        }

        #[test]
        fn test_process_all_string_arguments_debug() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-v"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("-v"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args);
        }
        #[test]
        fn test_process_all_string_arguments_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args_2 = vec![
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args_2);
        }
        #[test]
        fn test_process_all_string_arguments_s_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-S"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("-S"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args_2 = vec![
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args_2);
        }
        #[test]
        fn test_process_all_string_arguments_s_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-S"),
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("-S"),
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args_2 = vec![
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, _) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let result = env_app_data.process_all_string_arguments(&original_args);

            assert_eq!(result.unwrap(), expected_args_2);
        }
    }

    mod tests_make_error_no_such_file_or_dir {
        use crate::EnvAppData;
        use std::ffi::OsString;

        #[test]
        fn test_make_error_no_such_file_or_dir() {
            let env_app_data = EnvAppData::default();
            let prog = OsString::from("test_program");
            let error = env_app_data.make_error_no_such_file_or_dir(&prog);
            //println!("{:#?}", error.code());
            assert_eq!(error.code(), 127);
        }
    }

    mod tests_make_options {
        use crate::EnvAppData;
        use crate::EnvOptions;
        use crate::env_make_options;
        use ctcore::ct_line_ending::CtLineEnding::Newline;
        use std::ffi::{OsStr, OsString};
        use std::fs;
        use std::fs::File;
        use std::io::Write;

        use tempfile::Builder;

        #[test]
        fn test_make_options_i() {
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch"].iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-i"), OsString::from("arch")];

            let expected_args = vec![OsString::from("-i"), OsString::from("arch")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }

        #[test]
        fn test_make_options_ignore_environment() {
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch"].iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--ignore-environment"),
                OsString::from("arch"),
            ];

            let expected_args = vec![
                OsString::from("--ignore-environment"),
                OsString::from("arch"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();
            assert_eq!(original_args, expected_args);
            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }

        #[test]
        fn test_make_options_split_string_args() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("-S"),
                OsString::from("arg2"),
                OsString::from("-vS"),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("-S"),
                OsString::from("arg2"),
                OsString::from("-vS"),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arg1", "-S", "arg2", "-vS", "arg3"]
                    .iter()
                    .map(OsStr::new)
                    .collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }

        #[test]
        fn test_make_options_chdir_cmd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--chdir"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let expected_args = vec![
                OsString::from("--chdir"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let binding = [env_dir, "ls"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }

        #[test]
        fn test_make_options_c_cmd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-C"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let expected_args = vec![
                OsString::from("-C"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [env_dir, "ls"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }

        #[test]
        fn test_make_options_0() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-0")];

            let expected_args = vec![OsString::from("-0")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: [].to_vec(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }

        #[test]
        fn test_make_options_null() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("--null")];

            let expected_args = vec![OsString::from("--null")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: [].to_vec(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }

        #[test]
        fn test_make_options_f() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let f = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-f"), OsString::from(f)];

            let expected_args = vec![OsString::from("-f"), OsString::from(f)];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let binding = [f];
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }

        #[test]
        fn test_make_options_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let f = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("--file"), OsString::from(f)];

            let expected_args = vec![OsString::from("--file"), OsString::from(f)];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [f];
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }

        #[test]
        fn test_make_options_unset() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--unset"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("--unset"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [unset_filename, "env"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }

        #[test]
        fn test_make_options_u() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-u"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("-u"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [unset_filename, "env"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }

        #[test]
        fn test_make_options_debug_whole() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--debug"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("--debug"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch", "env"].iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }

        #[test]
        fn test_make_options_debug() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-v"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("-v"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch", "env"].iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }
        #[test]
        fn test_make_options_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = ["arg1", "cat", file_path, "arg3"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }
        #[test]
        fn test_make_options_s_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-S"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("-S"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = ["arg1", "cat", file_path, "arg3"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }
        #[test]
        fn test_make_options_s_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-S"),
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("-S"),
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let binding = ["arg1", "cat", file_path, "arg3"];
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);
        }
    }

    mod tests_apply_change_directory {
        use crate::EnvAppData;
        use crate::EnvOptions;
        use crate::env_apply_change_directory;
        use crate::env_make_options;

        use ctcore::ct_line_ending::CtLineEnding::Newline;
        use std::ffi::{OsStr, OsString};
        use std::fs;
        use std::fs::File;
        use std::io::Write;

        use tempfile::Builder;

        #[test]
        fn test_apply_change_directory_i() {
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch"].iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-i"), OsString::from("arch")];

            let expected_args = vec![OsString::from("-i"), OsString::from("arch")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }
        #[test]
        fn test_apply_change_directory_ignore_environment() {
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch"].iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--ignore-environment"),
                OsString::from("arch"),
            ];

            let expected_args = vec![
                OsString::from("--ignore-environment"),
                OsString::from("arch"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();
            assert_eq!(original_args, expected_args);
            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_apply_change_directory_split_string_args() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("-S"),
                OsString::from("arg2"),
                OsString::from("-vS"),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("-S"),
                OsString::from("arg2"),
                OsString::from("-vS"),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arg1", "-S", "arg2", "-vS", "arg3"]
                    .iter()
                    .map(OsStr::new)
                    .collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_apply_change_directory_chdir_cmd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--chdir"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let expected_args = vec![
                OsString::from("--chdir"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let binding = [env_dir, "ls"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_apply_change_directory_c_cmd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-C"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let expected_args = vec![
                OsString::from("-C"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [env_dir, "ls"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_apply_change_directory_0() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-0")];

            let expected_args = vec![OsString::from("-0")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: [].to_vec(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_apply_change_directory_null() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("--null")];

            let expected_args = vec![OsString::from("--null")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: [].to_vec(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_apply_change_directory_f() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let f = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-f"), OsString::from(f)];

            let expected_args = vec![OsString::from("-f"), OsString::from(f)];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let binding = [f];
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_apply_change_directory_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let f = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("--file"), OsString::from(f)];

            let expected_args = vec![OsString::from("--file"), OsString::from(f)];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [f];
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_apply_change_directory_unset() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--unset"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("--unset"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [unset_filename, "env"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_apply_change_directory_u() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-u"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("-u"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [unset_filename, "env"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_apply_change_directory_debug_whole() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--debug"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("--debug"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch", "env"].iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_apply_change_directory_debug() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-v"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("-v"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch", "env"].iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }
        #[test]
        fn test_apply_change_directory_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = ["arg1", "cat", file_path, "arg3"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }
        #[test]
        fn test_apply_change_directory_s_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-S"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("-S"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = ["arg1", "cat", file_path, "arg3"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }
        #[test]
        fn test_apply_change_directory_s_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-S"),
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("-S"),
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let binding = ["arg1", "cat", file_path, "arg3"];
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_apply_change_directory(&opts);

            assert!(ret.is_ok());
        }
    }

    mod tests_load_config_file {
        use crate::EnvAppData;
        use crate::EnvOptions;
        use crate::env_load_config_file;
        use crate::env_make_options;

        use ctcore::ct_line_ending::CtLineEnding::Newline;
        use std::ffi::{OsStr, OsString};
        use std::fs;
        use std::fs::File;
        use std::io::Write;

        use tempfile::Builder;

        #[test]
        fn test_load_config_file_i() {
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch"].iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-i"), OsString::from("arch")];

            let expected_args = vec![OsString::from("-i"), OsString::from("arch")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }
        #[test]
        fn test_load_config_file_ignore_environment() {
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch"].iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--ignore-environment"),
                OsString::from("arch"),
            ];

            let expected_args = vec![
                OsString::from("--ignore-environment"),
                OsString::from("arch"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();
            assert_eq!(original_args, expected_args);
            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_load_config_file_split_string_args() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("-S"),
                OsString::from("arg2"),
                OsString::from("-vS"),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("-S"),
                OsString::from("arg2"),
                OsString::from("-vS"),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arg1", "-S", "arg2", "-vS", "arg3"]
                    .iter()
                    .map(OsStr::new)
                    .collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_load_config_file_chdir_cmd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--chdir"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let expected_args = vec![
                OsString::from("--chdir"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let binding = [env_dir, "ls"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_load_config_file_c_cmd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-C"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let expected_args = vec![
                OsString::from("-C"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [env_dir, "ls"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_load_config_file_0() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-0")];

            let expected_args = vec![OsString::from("-0")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: [].to_vec(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_load_config_file_null() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("--null")];

            let expected_args = vec![OsString::from("--null")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: [].to_vec(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_load_config_file_f() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let f = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-f"), OsString::from(f)];

            let expected_args = vec![OsString::from("-f"), OsString::from(f)];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let binding = [f];
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_load_config_file_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let f = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("--file"), OsString::from(f)];

            let expected_args = vec![OsString::from("--file"), OsString::from(f)];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [f];
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_load_config_file_unset() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--unset"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("--unset"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [unset_filename, "env"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_load_config_file_u() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-u"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("-u"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [unset_filename, "env"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_load_config_file_debug_whole() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--debug"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("--debug"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch", "env"].iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_load_config_file_debug() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-v"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("-v"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch", "env"].iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }
        #[test]
        fn test_load_config_file_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = ["arg1", "cat", file_path, "arg3"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }
        #[test]
        fn test_load_config_file_s_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-S"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("-S"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = ["arg1", "cat", file_path, "arg3"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }
        #[test]
        fn test_load_config_file_s_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-S"),
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("-S"),
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let binding = ["arg1", "cat", file_path, "arg3"];
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let ret = env_load_config_file(&mut opts);

            assert!(ret.is_ok());
        }
    }

    mod tests_parse_name_value_opt {
        use crate::EnvAppData;
        use crate::EnvOptions;
        use crate::env_make_options;
        use crate::env_parse_name_value_opt;

        use ctcore::ct_line_ending::CtLineEnding::Newline;
        use std::ffi::{OsStr, OsString};
        use std::fs;
        use std::fs::File;
        use std::io::Write;

        use tempfile::Builder;

        #[test]
        fn test_parse_name_value_opt_i() {
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch"].iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-i"), OsString::from("arch")];

            let expected_args = vec![OsString::from("-i"), OsString::from("arch")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let opt = OsStr::new("arch");
            let ret = env_parse_name_value_opt(&mut opts, opt).unwrap();

            assert!(ret);
            assert_eq!(opts.sets.len(), 0);
        }
        #[test]
        fn test_parse_name_value_opt_ignore_environment() {
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch"].iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--ignore-environment"),
                OsString::from("arch"),
            ];

            let expected_args = vec![
                OsString::from("--ignore-environment"),
                OsString::from("arch"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();
            assert_eq!(original_args, expected_args);
            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let opt = OsStr::new("arch");
            let ret = env_parse_name_value_opt(&mut opts, opt).unwrap();

            assert!(ret);
            assert_eq!(opts.sets.len(), 0);
        }

        #[test]
        fn test_parse_name_value_opt_split_string_args() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("-S"),
                OsString::from("arg2"),
                OsString::from("-vS"),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("-S"),
                OsString::from("arg2"),
                OsString::from("-vS"),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arg1", "-S", "arg2", "-vS", "arg3"]
                    .iter()
                    .map(OsStr::new)
                    .collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let opt = OsStr::new("arch");
            let ret = env_parse_name_value_opt(&mut opts, opt).unwrap();

            assert!(ret);
            assert_eq!(opts.sets.len(), 0);
        }

        #[test]
        fn test_parse_name_value_opt_chdir_cmd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--chdir"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let expected_args = vec![
                OsString::from("--chdir"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let binding = [env_dir, "ls"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let opt = OsStr::new("arch");
            let ret = env_parse_name_value_opt(&mut opts, opt).unwrap();

            assert!(ret);
            assert_eq!(opts.sets.len(), 0);
        }

        #[test]
        fn test_parse_name_value_opt_c_cmd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-C"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let expected_args = vec![
                OsString::from("-C"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [env_dir, "ls"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let opt = OsStr::new("arch");
            let ret = env_parse_name_value_opt(&mut opts, opt).unwrap();

            assert!(ret);
            assert_eq!(opts.sets.len(), 0);
        }

        #[test]
        fn test_parse_name_value_opt_0() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-0")];

            let expected_args = vec![OsString::from("-0")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: [].to_vec(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let opt = OsStr::new("arch");
            let ret = env_parse_name_value_opt(&mut opts, opt).unwrap();

            assert!(ret);
            assert_eq!(opts.sets.len(), 0);
        }

        #[test]
        fn test_parse_name_value_opt_null() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("--null")];

            let expected_args = vec![OsString::from("--null")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: [].to_vec(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let opt = OsStr::new("arch");
            let ret = env_parse_name_value_opt(&mut opts, opt).unwrap();

            assert!(ret);
            assert_eq!(opts.sets.len(), 0);
        }

        #[test]
        fn test_parse_name_value_opt_f() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let f = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-f"), OsString::from(f)];

            let expected_args = vec![OsString::from("-f"), OsString::from(f)];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let binding = [f];
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let opt = OsStr::new("arch");
            let ret = env_parse_name_value_opt(&mut opts, opt).unwrap();

            assert!(ret);
            assert_eq!(opts.sets.len(), 0);
        }

        #[test]
        fn test_parse_name_value_opt_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let f = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("--file"), OsString::from(f)];

            let expected_args = vec![OsString::from("--file"), OsString::from(f)];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [f];
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let opt = OsStr::new("arch");
            let ret = env_parse_name_value_opt(&mut opts, opt).unwrap();

            assert!(ret);
            assert_eq!(opts.sets.len(), 0);
        }

        #[test]
        fn test_parse_name_value_opt_unset() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--unset"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("--unset"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [unset_filename, "env"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let opt = OsStr::new("arch");
            let ret = env_parse_name_value_opt(&mut opts, opt).unwrap();

            assert!(ret);
            assert_eq!(opts.sets.len(), 0);
        }

        #[test]
        fn test_parse_name_value_opt_u() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-u"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("-u"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [unset_filename, "env"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let opt = OsStr::new("arch");
            let ret = env_parse_name_value_opt(&mut opts, opt).unwrap();

            assert!(ret);
            assert_eq!(opts.sets.len(), 0);
        }

        #[test]
        fn test_parse_name_value_opt_debug_whole() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--debug"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("--debug"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch", "env"].iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let opt = OsStr::new("arch");
            let ret = env_parse_name_value_opt(&mut opts, opt).unwrap();

            assert!(ret);
            assert_eq!(opts.sets.len(), 0);
        }

        #[test]
        fn test_parse_name_value_opt_debug() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-v"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("-v"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch", "env"].iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let opt = OsStr::new("arch");
            let ret = env_parse_name_value_opt(&mut opts, opt).unwrap();

            assert!(ret);
            assert_eq!(opts.sets.len(), 0);
        }
        #[test]
        fn test_parse_name_value_opt_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = ["arg1", "cat", file_path, "arg3"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let opt = OsStr::new("arch");
            let ret = env_parse_name_value_opt(&mut opts, opt).unwrap();

            assert!(ret);
            assert_eq!(opts.sets.len(), 0);
        }
        #[test]
        fn test_parse_name_value_opt_s_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-S"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("-S"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = ["arg1", "cat", file_path, "arg3"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let opt = OsStr::new("arch");
            let ret = env_parse_name_value_opt(&mut opts, opt).unwrap();

            assert!(ret);
            assert_eq!(opts.sets.len(), 0);
        }
        #[test]
        fn test_parse_name_value_opt_s_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-S"),
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("-S"),
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let binding = ["arg1", "cat", file_path, "arg3"];
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let opt = OsStr::new("arch");
            let ret = env_parse_name_value_opt(&mut opts, opt).unwrap();

            assert!(ret);
            assert_eq!(opts.sets.len(), 0);
        }
    }

    #[cfg(not(unix))]
    mod tests_run_program {
        use crate::EnvAppData;
        use crate::EnvOptions;
        use crate::env_make_options;

        use ctcore::ct_line_ending::CtLineEnding::Newline;
        use std::ffi::OsStr;
        use std::ffi::OsString;
        use std::fs;
        use std::fs::File;
        use std::io::Write;

        use tempfile::Builder;

        #[test]
        fn test_run_program_i() {
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch"].iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-i"), OsString::from("arch")];

            let expected_args = vec![OsString::from("-i"), OsString::from("arch")];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let mut env_app_data = EnvAppData::default();

            let do_debug_printing = true;
            let ret = env_app_data.run_program(opts, do_debug_printing);

            assert!(ret.is_ok());
        }
        #[test]
        fn test_run_program_ignore_environment() {
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch"].iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--ignore-environment"),
                OsString::from("arch"),
            ];

            let expected_args = vec![
                OsString::from("--ignore-environment"),
                OsString::from("arch"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();
            assert_eq!(original_args, expected_args);
            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let mut env_app_data = EnvAppData::default();

            let do_debug_printing = true;
            let ret = env_app_data.run_program(opts, do_debug_printing);

            assert!(ret.is_ok());
        }

        #[test]
        fn test_run_program_split_string_args() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("-S"),
                OsString::from("arg2"),
                OsString::from("-vS"),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("-S"),
                OsString::from("arg2"),
                OsString::from("-vS"),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arg1", "-S", "arg2", "-vS", "arg3"]
                    .iter()
                    .map(OsStr::new)
                    .collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let mut env_app_data = EnvAppData::default();

            let do_debug_printing = true;
            let ret = env_app_data.run_program(opts, do_debug_printing);

            assert!(ret.is_err());
        }

        #[test]
        fn test_run_program_chdir_cmd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--chdir"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let expected_args = vec![
                OsString::from("--chdir"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let binding = [env_dir, "ls"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let mut env_app_data = EnvAppData::default();

            let do_debug_printing = true;
            let ret = env_app_data.run_program(opts, do_debug_printing);

            assert!(ret.is_err());
        }

        #[test]
        fn test_run_program_c_cmd() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-C"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let expected_args = vec![
                OsString::from("-C"),
                OsString::from(env_dir),
                OsString::from("ls"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [env_dir, "ls"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let mut env_app_data = EnvAppData::default();

            let do_debug_printing = true;
            let ret = env_app_data.run_program(opts, do_debug_printing);

            assert!(ret.is_err());
        }

        #[test]
        fn test_run_program_f() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let f = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("-f"), OsString::from(f)];

            let expected_args = vec![OsString::from("-f"), OsString::from(f)];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let binding = [f];
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let mut env_app_data = EnvAppData::default();

            let do_debug_printing = true;
            let ret = env_app_data.run_program(opts, do_debug_printing);

            assert!(ret.is_err());
        }

        #[test]
        fn test_run_program_file() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let f = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![OsString::from("--file"), OsString::from(f)];

            let expected_args = vec![OsString::from("--file"), OsString::from(f)];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [f];
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let mut env_app_data = EnvAppData::default();

            let do_debug_printing = true;
            let ret = env_app_data.run_program(opts, do_debug_printing);

            assert!(ret.is_err());
        }

        #[test]
        fn test_run_program_unset() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--unset"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("--unset"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [unset_filename, "env"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let mut env_app_data = EnvAppData::default();

            let do_debug_printing = true;
            let ret = env_app_data.run_program(opts, do_debug_printing);

            assert!(ret.is_err());
        }

        #[test]
        fn test_run_program_u() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let _ = sub_dir_path.to_str().unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-u"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("-u"),
                OsString::from(unset_filename),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = [unset_filename, "env"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let mut env_app_data = EnvAppData::default();

            let do_debug_printing = true;
            let ret = env_app_data.run_program(opts, do_debug_printing);

            assert!(ret.is_err());
        }

        #[test]
        fn test_run_program_debug_whole() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--debug"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("--debug"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch", "env"].iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let mut env_app_data = EnvAppData::default();

            let do_debug_printing = true;
            let ret = env_app_data.run_program(opts, do_debug_printing);

            assert!(ret.is_err());
        }

        #[test]
        fn test_run_program_debug() {
            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-v"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let expected_args = vec![
                OsString::from("-v"),
                OsString::from("arch"),
                OsString::from("env"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: ["arch", "env"].iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let mut env_app_data = EnvAppData::default();

            let do_debug_printing = true;
            let ret = env_app_data.run_program(opts, do_debug_printing);

            assert!(ret.is_err());
        }
        #[test]
        fn test_run_program_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = ["arg1", "cat", file_path, "arg3"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let mut env_app_data = EnvAppData::default();

            let do_debug_printing = true;
            let ret = env_app_data.run_program(opts, do_debug_printing);

            assert!(ret.is_err());
        }
        #[test]
        fn test_run_program_s_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-S"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("-S"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);

            let binding = ["arg1", "cat", file_path, "arg3"];

            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let mut env_app_data = EnvAppData::default();

            let do_debug_printing = true;
            let ret = env_app_data.run_program(opts, do_debug_printing);

            assert!(ret.is_err());
        }
        #[test]
        fn test_run_program_s_split_string() {
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let mut env_app_data = EnvAppData::default();
            let original_args = vec![
                OsString::from("-S"),
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let expected_args = vec![
                OsString::from("-S"),
                OsString::from("--split-string"),
                OsString::from("arg1"),
                OsString::from("cat"),
                OsString::from(file_path),
                OsString::from("arg3"),
            ];

            let original_args = original_args.into_iter();
            let result = env_app_data.parse_arguments(original_args);
            assert!(result.is_ok());
            let (original_args, matches) = result.unwrap();

            assert_eq!(original_args, expected_args);
            let binding = ["arg1", "cat", file_path, "arg3"];
            let expected_opts = EnvOptions {
                ignore_env: false,
                line_ending: Newline,
                running_directory: None,
                files: [].to_vec(),
                unsets: [].to_vec(),
                sets: [].to_vec(),
                program: binding.iter().map(OsStr::new).collect::<Vec<_>>(),
                #[cfg(unix)]
                default_signals: None,
                #[cfg(unix)]
                ignore_signals: None,
                #[cfg(unix)]
                block_signals: None,
                #[cfg(unix)]
                list_signal_handling: false,
            };

            let opts = env_make_options(&matches).unwrap();

            assert_eq!(opts, expected_opts);

            let mut env_app_data = EnvAppData::default();

            let do_debug_printing = true;
            let ret = env_app_data.run_program(opts, do_debug_printing);

            assert!(ret.is_err());
        }
    }

    mod tests_ct_app {
        use crate::ct_app;
        use clap::error::ErrorKind;

        use std::fs;
        use std::fs::File;
        use std::io::Write;

        use tempfile::Builder;

        #[test]
        fn test_ct_app_version() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "--version"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_v() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "-V"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_help() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "--help"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_h() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "-h"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_i() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "-i", "arch"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_ignore_environment() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "--ignore-environment", "arch"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_bad_args() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "--bad-arg"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
        }

        #[test]
        fn test_ct_app_chdir_args() {
            let command = ct_app();
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "--chdir", env_dir, "ls"];

            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_c_args() {
            let command = ct_app();
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let _ = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let env_dir = sub_dir_path.to_str().unwrap();

            let args = [ctcore::ct_util_name(), "-C", env_dir, "ls"];

            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_0() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "-0"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_null() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "--null"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        use crate::env;

        #[test]
        fn test_ct_app_f() {
            let command = ct_app();
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let f = test_file_1.to_str().unwrap();

            let content = "FVAR1=hello\n\
           FVAR2=CtyunOS\n\
           FVAR3=Rust\n\
           FVAR4=Syskit\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "-f", f];

            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());

            match env::var("FVAR1") {
                Ok(val) => {
                    assert_eq!(val, "hello");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("FVAR2") {
                Ok(val) => {
                    assert_eq!(val, "CtyunOS");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("FVAR3") {
                Ok(val) => {
                    assert_eq!(val, "Rust");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("FVAR4") {
                Ok(val) => {
                    assert_eq!(val, "Syskit");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }
        }

        #[test]
        fn test_ct_app_file() {
            let command = ct_app();
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let filename = test_file_1.to_str().unwrap();

            let content = "ENV_VAR1=hello\n\
           ENV_VAR2=CtyunOS\n\
           ENV_VAR3=Rust\n\
           ENV_VAR4=Syskit\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "--file", filename, "ls"];

            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());

            match env::var("ENV_VAR1") {
                Ok(val) => {
                    assert_eq!(val, "hello");
                }
                Err(e) => {
                    println!("env set fail:{e}")
                }
            }

            match env::var("ENV_VAR2") {
                Ok(val) => {
                    assert_eq!(val, "CtyunOS");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("ENV_VAR3") {
                Ok(val) => {
                    assert_eq!(val, "Rust");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("ENV_VAR4") {
                Ok(val) => {
                    assert_eq!(val, "Syskit");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }
        }

        #[test]
        fn test_ct_app_file_unset() {
            let command = ct_app();
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "QQQ_UNSET_VAR1=hello\n\
           QQQ_UNSET_VAR2=CtyunOS\n\
           QQQ_UNSET_VAR3=Rust\n\
           QQQ_UNSET_VAR4=Syskit\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "--file", unset_filename, "env"];

            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());

            match env::var("QQQ_UNSET_VAR1") {
                Ok(val) => {
                    assert_eq!(val, "hello");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("QQQ_UNSET_VAR2") {
                Ok(val) => {
                    assert_eq!(val, "CtyunOS");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("QQQ_UNSET_VAR3") {
                Ok(val) => {
                    assert_eq!(val, "Rust");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("QQQ_UNSET_VAR4") {
                Ok(val) => {
                    assert_eq!(val, "Syskit");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            let args = [
                ctcore::ct_util_name(),
                "-i",
                "env",
                "-u",
                unset_filename,
                "env",
            ];
            let command = ct_app();
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());

            match env::var("QQQ_UNSET_VAR1") {
                Ok(val) => {
                    assert_eq!(val, "hello");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("QQQ_UNSET_VAR2") {
                Ok(val) => {
                    assert_eq!(val, "CtyunOS");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("QQQ_UNSET_VAR3") {
                Ok(val) => {
                    assert_eq!(val, "Rust");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("QQQ_UNSET_VAR4") {
                Ok(val) => {
                    assert_eq!(val, "Syskit");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }
        }

        #[test]
        fn test_ct_app_file_unset_whole() {
            let command = ct_app();
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let unset_filename = test_file_1.to_str().unwrap();

            let content = "UNSET_VAR1=hello\n\
           UNSET_VAR2=CtyunOS\n\
           UNSET_VAR3=Rust\n\
           UNSET_VAR4=Syskit\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "--file", unset_filename, "env"];

            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());

            match env::var("UNSET_VAR1") {
                Ok(val) => {
                    assert_eq!(val, "hello");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("UNSET_VAR2") {
                Ok(val) => {
                    assert_eq!(val, "CtyunOS");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("UNSET_VAR3") {
                Ok(val) => {
                    assert_eq!(val, "Rust");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("UNSET_VAR4") {
                Ok(val) => {
                    assert_eq!(val, "Syskit");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            let args = [
                ctcore::ct_util_name(),
                "--ignore-environment",
                "env",
                "--unset",
                unset_filename,
                "env",
            ];
            let command = ct_app();
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());

            match env::var("UNSET_VAR1") {
                Ok(val) => {
                    assert_eq!(val, "hello");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("UNSET_VAR2") {
                Ok(val) => {
                    assert_eq!(val, "CtyunOS");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("UNSET_VAR3") {
                Ok(val) => {
                    assert_eq!(val, "Rust");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }

            match env::var("UNSET_VAR4") {
                Ok(val) => {
                    assert_eq!(val, "Syskit");
                }
                Err(e) => {
                    println!("env set fail,{e}")
                }
            }
        }

        #[test]
        fn test_ct_app_debug() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "-v", "arch"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_debug_whole() {
            let command = ct_app();
            let args = [ctcore::ct_util_name(), "--debug", "arch"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_split_string() {
            let command = ct_app();
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                "--split-string=''",
                "cat",
                file_path,
            ];

            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_split_s() {
            let command = ct_app();
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [ctcore::ct_util_name(), "-S", "", "cat", file_path];

            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_s_split_string() {
            let command = ct_app();
            let temp_dir = Builder::new()
                .prefix("tests_ct_get_filesystem_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("test_file_1.txt");
            File::create(&test_file_1).unwrap();
            let mut file = File::create(&test_file_1).unwrap();
            let file_path = test_file_1.to_str().unwrap();

            let content = "aaaa.\n\
           bbbb.\n\
           cccc.\n\
           dddd.\n";
            file.write_all(content.as_bytes()).unwrap();

            let args = [
                ctcore::ct_util_name(),
                "-S",
                "--split-string=''",
                "cat",
                file_path,
            ];

            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
        }
    }
}
