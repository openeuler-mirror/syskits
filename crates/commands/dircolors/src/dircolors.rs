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
use rust_i18n::t;
//dircolors 命令在Linux系统中主要用于控制 ls 命令显示目录和文件时使用的颜色
rust_i18n::i18n!("locales", fallback = "en-US");
use std::env;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use sys_locale::get_locale;

use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use ctcore::Tool;
use ctcore::ct_colors::{CT_FILE_ATTRIBUTE_CODES, CT_FILE_COLORS, CT_FILE_TYPES, CT_TERMS};
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CTsageError, CtSimpleError};

use std::ffi::{CString, OsString};

mod opt_flags {
    pub const BOURNE_SHELL: &str = "bourne-shell";
    pub const C_SHELL: &str = "c-shell";
    pub const PRINT_DATABASE: &str = "print-database";
    pub const PRINT_LS_COLORS: &str = "print-ls-colors";
    pub const FILE: &str = "FILE";
}

#[derive(PartialEq, Eq, Debug)]
pub enum DircolorsOutputFmt {
    Shell,
    CShell,
    Display,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DircolorsSemantic {
    pub output_kind: String,
    pub output: String,
    pub classic_bytes: Vec<u8>,
}

pub fn dircolors_guess_syntax() -> DircolorsOutputFmt {
    match env::var("SHELL") {
        Ok(ref s) if !s.is_empty() => {
            let shell_path: &Path = s.as_ref();
            if let Some(name) = shell_path.file_name() {
                if name == "csh" || name == "tcsh" {
                    DircolorsOutputFmt::CShell
                } else {
                    DircolorsOutputFmt::Shell
                }
            } else {
                DircolorsOutputFmt::Shell
            }
        }
        _ => DircolorsOutputFmt::Unknown,
    }
}

fn dircolors_get_colors_format_strings(fmt: &DircolorsOutputFmt) -> (&'static [u8], &'static [u8]) {
    let prefix: &'static [u8] = match fmt {
        DircolorsOutputFmt::Shell => b"LS_COLORS='",
        DircolorsOutputFmt::CShell => b"setenv LS_COLORS '",
        DircolorsOutputFmt::Display => b"",
        DircolorsOutputFmt::Unknown => unreachable!(),
    };

    let suffix: &'static [u8] = match fmt {
        DircolorsOutputFmt::Shell => b"';\nexport LS_COLORS",
        DircolorsOutputFmt::CShell => b"'",
        DircolorsOutputFmt::Display => b"",
        DircolorsOutputFmt::Unknown => unreachable!(),
    };

    (prefix, suffix)
}

/**
 * 命令行应用程序的主函数，用于解析参数并执行相应的操作，如打印目录颜色配置。
 *
 * # 参数
 * `args`: 实现了 `ctcore::Args` 的类型，提供访问命令行参数的方法。
 *
 * # 返回值
 * 一个 `CTResult<()>`，其中 `Ok(())` 表示成功，`Err(_)` 表示错误，并带有描述性消息。
 */
pub fn dircolors_main(args: impl ctcore::Args) -> CTResult<()> {
    let semantic = dircolors_native_semantic(args)?;
    if !semantic.classic_bytes.is_empty() {
        let stdout = std::io::stdout();
        let mut stdout = stdout.lock();
        stdout
            .write_all(&semantic.classic_bytes)
            .map_err(|error| CtSimpleError::new(1, error.to_string()))?;
        stdout
            .write_all(b"\n")
            .map_err(|error| CtSimpleError::new(1, error.to_string()))?;
    }
    Ok(())
}

pub fn dircolors_native_semantic(args: impl ctcore::Args) -> CTResult<DircolorsSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let args_match = ct_app().try_get_matches_from(args)?;

    let files = args_match
        .get_many::<String>(opt_flags::FILE)
        .map_or(vec![], |file_values| file_values.collect());

    if let Some(err) = dircolors_parm_conflict_check(&args_match) {
        return Err(err);
    }

    if let Some(err) = dircolors_print_parm_check(&args_match) {
        return Err(err);
    }

    if args_match.get_flag(opt_flags::PRINT_DATABASE) {
        if !files.is_empty() {
            return Err(CTsageError::new(
                1,
                format!(
                    "extra operand {}\nfile operands cannot be combined with \
                     --print-database (-p)",
                    files[0].quote()
                ),
            ));
        }

        let classic_bytes = generate_dircolors_config().into_bytes();
        return Ok(DircolorsSemantic {
            output_kind: "database".into(),
            output: String::from_utf8_lossy(&classic_bytes).into_owned(),
            classic_bytes,
        });
    }

    let mut out_format = dircolors_out_format_check(&args_match);
    if out_format == DircolorsOutputFmt::Unknown {
        match dircolors_guess_syntax() {
            DircolorsOutputFmt::Unknown => {
                return Err(CtSimpleError::new(
                    1,
                    "no SHELL environment variable, and no shell type option given",
                ));
            }
            fmt => out_format = fmt,
        }
    }

    let output = dircolors_collect_output(files, &out_format)?;
    Ok(DircolorsSemantic {
        output_kind: dircolors_output_kind(&out_format).into(),
        output: String::from_utf8_lossy(&output).into_owned(),
        classic_bytes: output,
    })
}

fn dircolors_output_kind(fmt: &DircolorsOutputFmt) -> &'static str {
    match fmt {
        DircolorsOutputFmt::Shell => "shell",
        DircolorsOutputFmt::CShell => "c-shell",
        DircolorsOutputFmt::Display => "display",
        DircolorsOutputFmt::Unknown => "unknown",
    }
}

fn dircolors_collect_output(
    files: Vec<&String>,
    out_format: &DircolorsOutputFmt,
) -> CTResult<Vec<u8>> {
    let (result, parse_ok) = if files.is_empty() {
        let default_config = generate_dircolors_config();
        dircolors_parse(
            default_config
                .as_bytes()
                .split(|byte| *byte == b'\n')
                .map(Ok::<_, std::io::Error>),
            out_format,
            "<internal>",
        )?
    } else if files.len() > 1 {
        return Err(CTsageError::new(
            1,
            format!("extra operand {}", files[1].quote()),
        ));
    } else if files[0].eq("-") {
        let fin = BufReader::new(ctcore::ct_io::stdin_reader_box());
        dircolors_parse(fin.split(b'\n'), out_format, files[0])?
    } else {
        let path = Path::new(files[0]);
        match File::open(path) {
            Ok(f) => {
                let fin = BufReader::new(f);
                dircolors_parse(fin.split(b'\n'), out_format, &path.to_string_lossy())?
            }
            Err(e) => {
                return Err(CtSimpleError::new(
                    1,
                    format!("{}: {}", path.maybe_quote(), path_error_message(&e)),
                ));
            }
        }
    };

    if !parse_ok {
        ctcore::ct_error::set_ct_exit_code(1);
        return Ok(Vec::new());
    }
    Ok(result)
}

fn path_error_message(error: &std::io::Error) -> String {
    #[cfg(unix)]
    if let Some(error_code) = error.raw_os_error() {
        let message = unsafe {
            std::ffi::CStr::from_ptr(ctcore::libc::strerror(error_code))
                .to_string_lossy()
                .into_owned()
        };
        return message;
    }

    error.kind().to_string()
}

fn dircolors_out_format_check(args_match: &ArgMatches) -> DircolorsOutputFmt {
    if args_match.get_flag(opt_flags::C_SHELL) {
        DircolorsOutputFmt::CShell
    } else if args_match.get_flag(opt_flags::BOURNE_SHELL) {
        DircolorsOutputFmt::Shell
    } else if args_match.get_flag(opt_flags::PRINT_LS_COLORS) {
        DircolorsOutputFmt::Display
    } else {
        DircolorsOutputFmt::Unknown
    }
}

fn dircolors_print_parm_check(
    args_match: &ArgMatches,
) -> Option<Box<dyn ctcore::ct_error::CTError>> {
    if args_match.get_flag(opt_flags::PRINT_DATABASE)
        && args_match.get_flag(opt_flags::PRINT_LS_COLORS)
    {
        return Some(CTsageError::new(
            1,
            "options --print-database and --print-ls-colors are mutually exclusive",
        ));
    }
    None
}

fn dircolors_parm_conflict_check(
    args_match: &ArgMatches,
) -> Option<Box<dyn ctcore::ct_error::CTError>> {
    if (args_match.get_flag(opt_flags::C_SHELL) || args_match.get_flag(opt_flags::BOURNE_SHELL))
        && (args_match.get_flag(opt_flags::PRINT_DATABASE)
            || args_match.get_flag(opt_flags::PRINT_LS_COLORS))
    {
        return Some(CTsageError::new(
            1,
            "the options to output non shell syntax,\n\
             and to select a shell syntax are mutually exclusive",
        ));
    }
    None
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("dircolors.about");
    let usage_description = t!("dircolors.usage");

    let args = dircolors_args_init();

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .after_help(t!("dircolors.after_help"))
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(&args)
}

fn dircolors_args_init() -> Vec<Arg> {
    let args = vec![
        Arg::new(opt_flags::BOURNE_SHELL)
            .long("sh")
            .short('b')
            .visible_alias("bourne-shell")
            .overrides_with(opt_flags::C_SHELL)
            .help(t!("dircolors.clap.bourne_shell"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::C_SHELL)
            .long("csh")
            .short('c')
            .visible_alias("c-shell")
            .overrides_with(opt_flags::BOURNE_SHELL)
            .help(t!("dircolors.clap.c_shell"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::PRINT_DATABASE)
            .long("print-database")
            .short('p')
            .help(t!("dircolors.clap.print_database"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::PRINT_LS_COLORS)
            .long("print-ls-colors")
            .help(t!("dircolors.clap.print_ls_colors"))
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::FILE)
            .hide(true)
            .value_hint(clap::ValueHint::FilePath)
            .action(ArgAction::Append),
    ];
    args
}

#[derive(PartialEq)]
enum DircolorsParseState {
    Global,
    Matched,
    Continue,
    Pass,
}

#[allow(clippy::cognitive_complexity)]
/// 解析dircolors格式的输入，生成相应的配置字符串。
///
/// # 参数
/// - `user_input`: 输入的数据，需要能够迭代器遍历字符串。
/// - `fmt`: 指定输出格式，决定如何处理解析结果。
/// - `fp`: 文件路径或者名称，用于在错误信息中引用。
///
/// # 返回值
/// - `Result<Vec<u8>, String>`: 成功时返回原始输出字节，失败时返回错误信息字符串。
#[allow(clippy::cognitive_complexity)]
fn dircolors_parse<T, S>(
    user_input: T,
    fmt: &DircolorsOutputFmt,
    fp: &str,
) -> CTResult<(Vec<u8>, bool)>
where
    T: IntoIterator<Item = Result<S, std::io::Error>>,
    S: AsRef<[u8]>,
{
    let mut result = Vec::with_capacity(1790);
    let (prefix, suffix) = dircolors_get_colors_format_strings(fmt);

    result.extend_from_slice(prefix);

    let term = env::var("TERM").unwrap_or_else(|_| "none".to_owned());
    let colorterm = env::var("COLORTERM").unwrap_or_default();

    let mut parse_state = DircolorsParseState::Global;
    let mut ok = true;

    for (num, line) in user_input.into_iter().enumerate() {
        let num = num + 1;
        let line = line.map_err(|error| {
            CtSimpleError::new(
                1,
                format!(
                    "{}: read error: {}",
                    fp.maybe_quote(),
                    path_error_message(&error)
                ),
            )
        })?;
        let line = line.as_ref();
        let line = line.split(|byte| *byte == b'\0').next().unwrap();
        let Some((key, value)) = dircolors_parse_line(line) else {
            continue;
        };
        let Some(value) = value else {
            ok = false;
            eprintln!(
                "{}: {}:{}: invalid line;  missing second token",
                ctcore::ct_util_name(),
                fp.maybe_quote(),
                num
            );
            continue;
        };

        let mut lower = key.to_vec();
        lower.make_ascii_lowercase();
        if lower.as_slice() == b"term" {
            if dircolors_fnmatch(value, term.as_bytes()) {
                parse_state = DircolorsParseState::Matched;
            } else if parse_state != DircolorsParseState::Matched {
                parse_state = DircolorsParseState::Pass;
            }
        } else if lower.as_slice() == b"colorterm" {
            if dircolors_fnmatch(value, colorterm.as_bytes()) {
                parse_state = DircolorsParseState::Matched;
            } else if parse_state != DircolorsParseState::Matched {
                parse_state = DircolorsParseState::Pass;
            }
        } else {
            if parse_state == DircolorsParseState::Matched {
                parse_state = DircolorsParseState::Continue;
            }
            if parse_state != DircolorsParseState::Pass {
                if key.starts_with(b".") {
                    dircolors_append_entry(&mut result, Some(b'*'), key, value, fmt);
                } else if key.starts_with(b"*") {
                    dircolors_append_entry(&mut result, None, key, value, fmt);
                } else if lower.as_slice() == b"options"
                    || lower.as_slice() == b"color"
                    || lower.as_slice() == b"eightbit"
                {
                } else if let Some((_, s)) = CT_FILE_ATTRIBUTE_CODES
                    .iter()
                    .find(|&&(attribute, _)| attribute.as_bytes() == lower.as_slice())
                {
                    dircolors_append_entry(&mut result, None, s.as_bytes(), value, fmt);
                } else if parse_state == DircolorsParseState::Matched
                    || parse_state == DircolorsParseState::Continue
                {
                    ok = false;
                    eprintln!(
                        "{}: {}:{}: unrecognized keyword {}",
                        ctcore::ct_util_name(),
                        fp.maybe_quote(),
                        num,
                        String::from_utf8_lossy(key)
                    );
                }
            }
        }
    }

    if fmt == &DircolorsOutputFmt::Display {
        result.pop();
    }
    result.extend_from_slice(suffix);

    Ok((result, ok))
}

fn dircolors_parse_line(line: &[u8]) -> Option<(&[u8], Option<&[u8]>)> {
    let line = line.trim_ascii_start();
    if line.is_empty() || line.starts_with(b"#") {
        return None;
    }

    let key_end = line
        .iter()
        .position(|byte| byte.is_ascii_whitespace())
        .unwrap_or(line.len());
    let key = &line[..key_end];
    let argument = line[key_end..].trim_ascii_start();
    if argument.is_empty() || argument.starts_with(b"#") {
        return Some((key, None));
    }

    let argument_end = argument
        .iter()
        .position(|byte| *byte == b'#')
        .unwrap_or(argument.len());
    let argument = argument[..argument_end].trim_ascii_end();
    Some((key, Some(argument)))
}

fn dircolors_append_entry(
    result: &mut Vec<u8>,
    prefix: Option<u8>,
    item: &[u8],
    value: &[u8],
    fmt: &DircolorsOutputFmt,
) {
    let display = *fmt == DircolorsOutputFmt::Display;
    if display {
        result.extend_from_slice(b"\x1b[");
        result.extend_from_slice(value);
        result.push(b'm');
    }
    if let Some(prefix) = prefix {
        result.push(prefix);
    }
    result.extend_from_slice(&dircolors_escape(item, !display));
    result.push(if display { b'\t' } else { b'=' });
    result.extend_from_slice(&dircolors_escape(value, !display));
    if display {
        result.extend_from_slice(b"\x1b[0m\n");
    } else {
        result.push(b':');
    }
}

fn dircolors_fnmatch(pattern: &[u8], value: &[u8]) -> bool {
    let Ok(pattern) = CString::new(pattern) else {
        return false;
    };
    let Ok(value) = CString::new(value) else {
        return false;
    };

    unsafe { ctcore::libc::fnmatch(pattern.as_ptr(), value.as_ptr(), 0) == 0 }
}

fn dircolors_escape(s: &[u8], escape_for_shell: bool) -> Vec<u8> {
    if !escape_for_shell {
        return s.to_vec();
    }

    let mut result = Vec::with_capacity(s.len());
    let mut need_backslash = true;

    for &c in s {
        match c {
            b'\'' => {
                result.extend_from_slice(b"'\\''");
                need_backslash = true;
            }
            b'\\' | b'^' => {
                result.push(c);
                need_backslash = !need_backslash;
            }
            b':' | b'=' => {
                if need_backslash {
                    result.push(b'\\');
                }
                result.push(c);
                need_backslash = true;
            }
            _ => {
                result.push(c);
                need_backslash = true;
            }
        }
    }

    result
}

pub fn generate_dircolors_config() -> String {
    let mut config = String::new();

    config.push_str(
        "\
        # Configuration file for dircolors, a utility to help you set the\n\
        # LS_COLORS environment variable used by GNU ls with the --color option.\n\
        # Copyright (C) 1996-2023 Free Software Foundation, Inc.\n\
        # Copying and distribution of this file, with or without modification,\n\
        # are permitted provided the copyright notice and this notice are preserved.\n\
        #\n\
        # The keywords COLOR, OPTIONS, and EIGHTBIT (honored by the\n\
        # slackware version of dircolors) are recognized but ignored.\n\
        # Global config options can be specified before TERM or COLORTERM entries\n\
        # ===================================================================\n\
        # Terminal filters\n\
        # ===================================================================\n\
        # Below are TERM or COLORTERM entries, which can be glob patterns, which\n\
        # restrict following config to systems with matching environment variables.\n\
        ",
    );
    config.push_str("COLORTERM ?*\n");
    for term in CT_TERMS {
        config.push_str(&format!("TERM {term}\n"));
    }

    config.push_str(
        "\
        # ===================================================================\n\
        # Basic file attributes\n\
        # ===================================================================\n\
        # Below are the color init strings for the basic file types.\n\
        # One can use codes for 256 or more colors supported by modern terminals.\n\
        # The default color codes use the capabilities of an 8 color terminal\n\
        # with some additional attributes as per the following codes:\n\
        # Attribute codes:\n\
        # 00=none 01=bold 04=underscore 05=blink 07=reverse 08=concealed\n\
        # Text color codes:\n\
        # 30=black 31=red 32=green 33=yellow 34=blue 35=magenta 36=cyan 37=white\n\
        # Background color codes:\n\
        # 40=black 41=red 42=green 43=yellow 44=blue 45=magenta 46=cyan 47=white\n\
        #NORMAL 00 # no color code at all\n\
        #FILE 00 # regular file: use no color at all\n\
        ",
    );

    for (name, _, code) in CT_FILE_TYPES {
        let comment = match *name {
            "RESET" => " # reset to \"normal\" color",
            "DIR" => " # directory",
            "LINK" => {
                " # symbolic link. (If you set this to 'target' instead of a\n # numerical value, the color is as for the file pointed to.)"
            }
            "MULTIHARDLINK" => " # regular file with more than one link",
            "FIFO" => " # pipe",
            "SOCK" => " # socket",
            "DOOR" => " # door",
            "BLK" => " # block device driver",
            "CHR" => " # character device driver",
            "ORPHAN" => " # symlink to nonexistent file, or non-stat'able file ...",
            "MISSING" => " # ... and the files they point to",
            "SETUID" => " # file that is setuid (u+s)",
            "SETGID" => " # file that is setgid (g+s)",
            "CAPABILITY" => " # file with capability (very expensive to lookup)",
            "STICKY_OTHER_WRITABLE" => " # dir that is sticky and other-writable (+t,o+w)",
            "OTHER_WRITABLE" => " # dir that is other-writable (o+w) and not sticky",
            "STICKY" => " # dir with the sticky bit set (+t) and not other-writable",
            "EXEC" => "",
            _ => "",
        };
        if *name == "EXEC" {
            config.push_str("# This is for files with execute permission:\n");
        }
        config.push_str(&format!("{name} {code}{comment}\n"));
    }

    config.push_str("# ===================================================================\n");
    config.push_str("# File extension attributes\n");
    config.push_str("# ===================================================================\n");
    config.push_str("# List any file extensions like '.gz' or '.tar' that you would like ls\n");
    config.push_str("# to color below. Put the suffix, a space, and the color init string.\n");
    config.push_str("# (and any comments you want to add after a '#').\n");
    config.push_str("# Suffixes are matched case insensitively, but if you define different\n");
    config.push_str("# init strings for separate cases, those will be honored.\n");
    config.push_str("#\n");
    config.push_str("# If you use DOS-style suffixes, you may want to uncomment the following:\n");
    config.push_str("#.cmd 01;32 # executables (bright green)\n");
    config.push_str("#.exe 01;32\n");
    config.push_str("#.com 01;32\n");
    config.push_str("#.btm 01;32\n");
    config.push_str("#.bat 01;32\n");
    config.push_str("# Or if you want to color scripts even if they do not have the\n");
    config.push_str("# executable bit actually set.\n");
    config.push_str("#.sh 01;32\n");
    config.push_str("#.csh 01;32\n");
    config.push_str("# archives or compressed (bright red)\n");

    for (ext, color) in CT_FILE_COLORS {
        if *ext == ".avif" {
            config.push_str("# image formats\n");
        } else if *ext == ".aac" {
            config.push_str("# audio formats\n");
        } else if *ext == "*~" {
            config.push_str("# backup files\n");
        } else if *ext == ".ogv" || *ext == ".oga" {
            config.push_str("# https://wiki.xiph.org/MIME_Types_and_File_Extensions\n");
        }
        config.push_str(&format!("{ext} {color}\n"));
    }
    config.push_str("#\n");
    config.push_str("# Subsequent TERM or COLORTERM entries, can be used to add / override\n");
    config.push_str("# config specific to those matching environment variables.");

    config
}

#[derive(Default)]
pub struct Dircolors;
impl Tool for Dircolors {
    fn name(&self) -> &'static str {
        "dircolors"
    }

    fn command(&self) -> Command {
        ct_app()
    }
    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        // 直接调用原有的 dircolors_main 函数
        dircolors_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Dircolors;

        // 测试 name 方法
        assert_eq!(tool.name(), "dircolors");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("dircolors"));

        // 测试 execute 方法
        let args = vec![OsString::from("dircolors"), OsString::from("-p")];
        assert!(tool.execute(&args).is_ok());
    }

    use super::dircolors_escape;

    #[test]
    fn test_escape() {
        assert_eq!(Vec::<u8>::new(), dircolors_escape(b"", true));
        assert_eq!(b"'\\''".to_vec(), dircolors_escape(b"'", true));
        assert_eq!(b"\\:".to_vec(), dircolors_escape(b":", true));
        assert_eq!(b"\\:".to_vec(), dircolors_escape(b"\\:", true));
        assert_eq!(b"\\=".to_vec(), dircolors_escape(b"=", true));
        assert_eq!(br"\\\\\:".to_vec(), dircolors_escape(br"\\\\:", true));
        assert_eq!(br"^:".to_vec(), dircolors_escape(br"^:", true));
        assert_eq!(b"01=34:56".to_vec(), dircolors_escape(b"01=34:56", false));
    }

    mod tests_dircolors_main {
        use crate::dircolors_main;

        use std::ffi::OsString;

        #[test]
        fn test_dircolors_main_version() {
            let args = [ctcore::ct_util_name(), "--version"];

            let result = dircolors_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_dircolors_main_v() {
            let args = [ctcore::ct_util_name(), "-V"];
            let result = dircolors_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_dircolors_main_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = dircolors_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_dircolors_main_h() {
            let args = [ctcore::ct_util_name(), "-h"];
            let result = dircolors_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_dircolors_main_b() {
            let args = [ctcore::ct_util_name(), "-b"];
            let result = dircolors_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_dircolors_main_sh() {
            let args = [ctcore::ct_util_name(), "--sh"];
            let result = dircolors_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_dircolors_main_c() {
            let args = [ctcore::ct_util_name(), "-c"];
            let result = dircolors_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_dircolors_main_csh() {
            let args = [ctcore::ct_util_name(), "--csh"];
            let result = dircolors_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_dircolors_main_p() {
            let args = [ctcore::ct_util_name(), "-p"];
            let result = dircolors_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_dircolors_main_print_database() {
            let args = [ctcore::ct_util_name(), "--print-database"];
            let result = dircolors_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_dircolors_main_print_ls_colors() {
            let args = [ctcore::ct_util_name(), "--print-ls-colors"];
            let result = dircolors_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }
    }

    mod tests_ct_app {
        use crate::ct_app;

        use crate::opt_flags::{BOURNE_SHELL, C_SHELL, PRINT_DATABASE, PRINT_LS_COLORS};
        use clap::error::ErrorKind;

        #[test]
        fn test_dircolors_main_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }
        #[test]
        fn test_dircolors_main_v() {
            let args = vec![ctcore::ct_util_name(), "-V"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_dircolors_main_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_dircolors_main_h() {
            let args = vec![ctcore::ct_util_name(), "-h"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_dircolors_main_b() {
            let args = vec![ctcore::ct_util_name(), "-b"];
            let command = ct_app();

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.unwrap().get_flag(BOURNE_SHELL));
        }

        #[test]
        fn test_dircolors_main_sh() {
            let args = vec![ctcore::ct_util_name(), "--sh"];
            let command = ct_app();

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.unwrap().get_flag(BOURNE_SHELL));
        }

        #[test]
        fn test_dircolors_main_c() {
            let args = vec![ctcore::ct_util_name(), "-c"];
            let command = ct_app();

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.unwrap().get_flag(C_SHELL));
        }

        #[test]
        fn test_dircolors_main_csh() {
            let args = vec![ctcore::ct_util_name(), "--csh"];
            let command = ct_app();

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.unwrap().get_flag(C_SHELL));
        }

        #[test]
        fn test_dircolors_main_p() {
            let args = vec![ctcore::ct_util_name(), "-p"];
            let command = ct_app();

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.unwrap().get_flag(PRINT_DATABASE));
        }

        #[test]
        fn test_dircolors_main_print_database() {
            let args = vec![ctcore::ct_util_name(), "--print-database"];
            let command = ct_app();

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.unwrap().get_flag(PRINT_DATABASE));
        }

        #[test]
        fn test_dircolors_main_print_ls_colors() {
            let args = vec![ctcore::ct_util_name(), "--print-ls-colors"];
            let command = ct_app();

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.unwrap().get_flag(PRINT_LS_COLORS));
        }
    }

    mod native_semantic_tests {
        use super::*;
        use std::ffi::OsString;
        use tempfile::TempDir;

        #[test]
        fn dircolors_native_semantic_shell_output_uses_fixture() {
            let dir = TempDir::new().unwrap();
            let path = dir.path().join("dircolors.conf");
            std::fs::write(&path, "DIR 01;34\n*.txt 01;32\n").unwrap();
            let path = path.display().to_string();

            let args = [ctcore::ct_util_name(), "-b", path.as_str()];
            let semantic = dircolors_native_semantic(args.iter().map(OsString::from)).unwrap();

            assert_eq!(semantic.output_kind, "shell");
            assert_eq!(
                semantic.output,
                "LS_COLORS='di=01;34:*.txt=01;32:';\nexport LS_COLORS"
            );
        }

        #[test]
        fn dircolors_native_semantic_database_output_reports_mode() {
            let args = [ctcore::ct_util_name(), "-p"];
            let semantic = dircolors_native_semantic(args.iter().map(OsString::from)).unwrap();

            assert_eq!(semantic.output_kind, "database");
            assert!(
                semantic
                    .output
                    .starts_with("# Configuration file for dircolors")
            );
        }
    }
}
