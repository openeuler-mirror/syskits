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

//dirname命令主要用于从给定的文件或目录路径中剥离出目录部分，去掉路径末尾的文件名（或最后一个组件），仅保留上级目录的路径。

extern crate rust_i18n;
use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::Tool;
use ctcore::ct_error::{CTResult, CTsageError, FromIo};
use ctcore::ct_line_ending::CtLineEnding;
use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use sys_locale::get_locale;

mod opt_flags {
    pub const ZERO: &str = "zero";
    pub const DIR: &str = "dir";
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirnameRow {
    pub input: String,
    pub directory_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirnameSemantic {
    pub rows: Vec<DirnameRow>,
    pub classic_text: String,
}

struct DirnameOptions {
    line_ending: CtLineEnding,
    inputs: Vec<OsString>,
}

impl DirnameOptions {
    fn from_matches(args_match: &ArgMatches) -> CTResult<Self> {
        let line_ending = CtLineEnding::from_zero_flag(args_match.get_flag(opt_flags::ZERO));
        let inputs: Vec<OsString> = args_match
            .get_many::<OsString>(opt_flags::DIR)
            .unwrap_or_default()
            .cloned()
            .collect();

        if inputs.is_empty() {
            return Err(CTsageError::new(1, "missing operand"));
        }

        Ok(Self {
            line_ending,
            inputs,
        })
    }
}

#[derive(Default)]
pub struct Dirname;
impl Tool for Dirname {
    fn name(&self) -> &'static str {
        "dirname"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        dirname_main(args.iter().cloned())
    }
}

pub fn dirname_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let args_match = ct_app()
        .after_help(t!("dirname.after_help"))
        .try_get_matches_from(dirname_args(args)?)?;

    let options = DirnameOptions::from_matches(&args_match)?;
    let stdout = io::stdout();
    dirname_classic_from_options(&options, &mut stdout.lock())?;

    Ok(())
}

pub fn dirname_native_semantic(args: impl ctcore::Args) -> CTResult<DirnameSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let args_match = ct_app()
        .after_help(t!("dirname.after_help"))
        .try_get_matches_from(dirname_args(args)?)?;
    let options = DirnameOptions::from_matches(&args_match)?;
    dirname_semantic_from_options(&options)
}

fn dirname_args(args: impl ctcore::Args) -> CTResult<Vec<OsString>> {
    let mut args: Vec<OsString> = args.collect();
    if std::env::var_os("POSIXLY_CORRECT").is_some() {
        // GNU getopt stops option processing at the first operand in POSIX mode.
        for index in 1..args.len() {
            let bytes = args[index].as_encoded_bytes();
            if bytes == b"--" {
                break;
            }
            if bytes.is_empty() || bytes == b"-" || bytes[0] != b'-' {
                args.insert(index, OsString::from("--"));
                break;
            }
        }
    }

    validate_dirname_options(&args)?;
    Ok(args)
}

fn validate_dirname_options(args: &[OsString]) -> CTResult<()> {
    // dirname has no value-taking options, so prevalidation can preserve GNU's
    // concise diagnostics while clap still owns successful option parsing.
    for argument in args.iter().skip(1) {
        let bytes = argument.as_encoded_bytes();
        if bytes == b"--" {
            break;
        }
        if bytes.len() <= 1 || bytes[0] != b'-' {
            continue;
        }

        if bytes[1] == b'-' {
            let option = &bytes[2..];
            let (name, has_value) = option
                .iter()
                .position(|byte| *byte == b'=')
                .map_or((option, false), |equals| (&option[..equals], true));
            let canonical = canonical_long_option(name);
            if has_value {
                if let Some(canonical) = canonical {
                    return Err(CTsageError::new(
                        1,
                        format!("option '--{canonical}' doesn't allow an argument"),
                    ));
                }
                return Err(CTsageError::new(
                    1,
                    format!("unrecognized option '{}'", String::from_utf8_lossy(bytes)),
                ));
            }
            match canonical {
                Some("help" | "version") => return Ok(()),
                Some(_) => continue,
                None => {
                    return Err(CTsageError::new(
                        1,
                        format!("unrecognized option '{}'", String::from_utf8_lossy(bytes)),
                    ));
                }
            }
        }

        for option in &bytes[1..] {
            match option {
                b'z' => {}
                b'h' | b'V' => return Ok(()),
                invalid => {
                    return Err(CTsageError::new(
                        1,
                        format!("invalid option -- '{}'", char::from(*invalid)),
                    ));
                }
            }
        }
    }

    Ok(())
}

fn canonical_long_option(option: &[u8]) -> Option<&'static str> {
    let mut matches = ["zero", "help", "version"].into_iter().filter(|candidate| {
        !option.is_empty()
            && option.len() <= candidate.len()
            && candidate.as_bytes().starts_with(option)
    });
    let canonical = matches.next()?;
    matches.next().is_none().then_some(canonical)
}

fn dirname_classic_from_options<W: Write>(
    options: &DirnameOptions,
    output: &mut W,
) -> CTResult<()> {
    let write_error = || String::from("write error");

    for input in &options.inputs {
        write_verbatim(output, &compute_dirname_os(input)).map_err_context(write_error)?;
        output
            .write_all(&[u8::from(options.line_ending)])
            .map_err_context(write_error)?;
    }
    output.flush().map_err_context(write_error)?;

    Ok(())
}

fn write_verbatim<W: Write>(output: &mut W, text: &OsStr) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        output.write_all(text.as_bytes())
    }
    #[cfg(windows)]
    {
        write!(output, "{}", std::path::Path::new(text).display())
    }
    #[cfg(not(any(unix, windows)))]
    {
        write!(output, "{}", text.to_string_lossy())
    }
}

fn dirname_semantic_from_options(options: &DirnameOptions) -> CTResult<DirnameSemantic> {
    let mut rows = Vec::with_capacity(options.inputs.len());
    let mut classic_text = String::new();

    for input in &options.inputs {
        let input = input.to_str().ok_or_else(|| {
            CTsageError::new(1, "dirname native semantics require UTF-8 arguments")
        })?;
        let directory_path = compute_dirname(input);
        classic_text.push_str(&directory_path);
        classic_text.push_str(&options.line_ending.to_string());
        rows.push(DirnameRow {
            input: input.to_owned(),
            directory_path,
        });
    }

    Ok(DirnameSemantic { rows, classic_text })
}

#[cfg(test)]
fn dirname_process(line_ending: CtLineEnding, dirnames: &Vec<String>) -> Option<CTResult<()>> {
    if dirnames.is_empty() {
        return Some(Err(CTsageError::new(1, "missing operand")));
    } else {
        let stdout = io::stdout();
        let mut output = stdout.lock();
        for item in dirnames {
            let dirname = compute_dirname(item);
            write!(output, "{dirname}{line_ending}").unwrap();
        }
    }
    None
}

pub fn compute_dirname(path: &str) -> String {
    compute_dirname_os(OsStr::new(path))
        .into_string()
        .expect("valid UTF-8 input must produce valid UTF-8 output")
}

pub fn compute_dirname_os(path: &OsStr) -> OsString {
    let bytes = path.as_encoded_bytes();
    if bytes.is_empty() {
        return OsString::from(".");
    }

    let Some(last_non_slash) = bytes.iter().rposition(|byte| *byte != b'/') else {
        return OsString::from("/");
    };
    let path_without_trailing_slashes = &bytes[..=last_non_slash];

    let Some(last_slash) = path_without_trailing_slashes
        .iter()
        .rposition(|byte| *byte == b'/')
    else {
        return OsString::from(".");
    };

    let directory = &path_without_trailing_slashes[..last_slash];
    let directory_end = directory
        .iter()
        .rposition(|byte| *byte != b'/')
        .map_or(0, |position| position + 1);
    if directory_end == 0 {
        return OsString::from("/");
    }

    // The slice is split only at ASCII '/' boundaries, so it remains a valid
    // subset of the platform-specific encoding returned by as_encoded_bytes.
    unsafe { OsString::from_encoded_bytes_unchecked(directory[..directory_end].to_vec()) }
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("dirname.about");
    let usage_description = t!("dirname.usage");

    let args = vec![
        Arg::new(opt_flags::ZERO)
            .long(opt_flags::ZERO)
            .short('z')
            .help(t!("dirname.clap.zero"))
            .action(ArgAction::SetTrue)
            .overrides_with(opt_flags::ZERO),
        Arg::new(opt_flags::DIR)
            .hide(true)
            .action(ArgAction::Append)
            .value_parser(clap::builder::ValueParser::os_string())
            .value_hint(clap::ValueHint::AnyPath),
    ];

    Command::new(utility_name)
        .about(application_info)
        .version(command_version)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(&args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[cfg(unix)]
    #[test]
    fn test_compute_dirname_preserves_non_utf8_bytes() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let path = OsString::from_vec(b"/tmp/missing-\xff/file".to_vec());
        let directory = compute_dirname_os(&path);

        assert_eq!(directory.as_os_str().as_bytes(), b"/tmp/missing-\xff");
    }

    #[test]
    fn test_tool_implementation() {
        let tool = Dirname;

        // Test name method
        assert_eq!(tool.name(), "dirname");

        // Test command method
        let command = tool.command();
        assert!(command.get_name().contains("dirname"));

        // Test execute method - should fail without arguments
        let args = vec![OsString::from("dirname"), OsString::from("--version")];
        assert!(tool.execute(&args).is_err());

        // Test execute method with valid argument
        let args = vec![OsString::from("dirname"), OsString::from("/path/to/file")];
        assert!(tool.execute(&args).is_ok());
    }

    mod tests_dirname_main {
        use crate::dirname_main;

        use std::ffi::OsString;

        #[test]
        fn test_dirname_main_version() {
            let args = [ctcore::ct_util_name(), "--version"];

            let result = dirname_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_dirname_main_v() {
            let args = [ctcore::ct_util_name(), "-V"];
            let result = dirname_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_dirname_main_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = dirname_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_dirname_main_h() {
            let args = [ctcore::ct_util_name(), "-h"];
            let result = dirname_main(args.iter().map(OsString::from));

            assert!(result.is_err());
        }

        #[test]
        fn test_dirname_main_z() {
            let args = [ctcore::ct_util_name(), "-z", "3/etc/audi-efwe/few/35/2"];
            let result = dirname_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }

        #[test]
        fn test_dirname_main_zero() {
            let args = [
                ctcore::ct_util_name(),
                "--zero",
                " 3/etc/audi-efwe/few/35/2",
            ];
            let result = dirname_main(args.iter().map(OsString::from));

            assert!(result.is_ok());
        }
    }

    mod tests_ct_app {
        use crate::ct_app;

        use crate::opt_flags::ZERO;
        use clap::error::ErrorKind;

        #[test]
        fn test_dirname_app_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }
        #[test]
        fn test_dirname_zpp_v() {
            let args = vec![ctcore::ct_util_name(), "-V"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_dirname_app_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_dirname_app_h() {
            let args = vec![ctcore::ct_util_name(), "-h"];
            let command = ct_app();
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_dirname_app_z() {
            let args = vec![ctcore::ct_util_name(), "-z", "3/etc/audi-efwe/few/35/2"];
            let command = ct_app();

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.unwrap().get_flag(ZERO));
        }

        #[test]
        fn test_dirname_app_zero() {
            let args = vec![ctcore::ct_util_name(), "--zero", "3/etc/audi-efwe/few/35/2"];
            let command = ct_app();

            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.unwrap().get_flag(ZERO));
        }
    }

    mod tests_dirname_process {
        use crate::dirname_process;
        use ctcore::ct_line_ending::CtLineEnding;

        use std::vec;

        #[test]
        fn test_dirname_process_with_empty_dirnames() {
            let line_ending = CtLineEnding::default();
            let dirnames = vec![];
            let result = dirname_process(line_ending, &dirnames);

            assert!(result.is_some());
            assert!(result.unwrap().is_err());
        }

        #[test]
        fn test_dirname_process_with_non_empty_dirnames() {
            let line_ending = CtLineEnding::default();
            let dirnames = ["dir1", "dir2", "dir3"]
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<String>>();

            let result = dirname_process(line_ending, &dirnames);

            assert!(result.is_none());
        }
    }
}
