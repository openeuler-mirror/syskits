/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

//! tee命令行工具，它允许用户将标准输入重定向到一个或多个文件，同时保持在标准输出上显示。
//! 这在需要将输出保存到文件并同时查看终端输出时非常有用。

use clap::{Arg, ArgAction, Command, builder::PossibleValue, crate_version};
use ctcore::ct_display::Quotable;
use ctcore::ct_error::CTResult;
use ctcore::{ct_format_usage, ct_help_about, ct_help_section, ct_help_usage, ct_show_error};
use std::fs::OpenOptions;
use std::io::{Error, ErrorKind, Read, Result, Write, copy, sink, stdin, stdout};
use std::path::PathBuf;

// spell-checker:ignore nopipe

#[cfg(unix)]
use ctcore::ct_signals::{enable_pipe_errors, ignore_interrupts};

const TEE_ABOUT: &str = ct_help_about!("tee.md");
const TEE_USAGE: &str = ct_help_usage!("tee.md");
const TEE_AFTER_HELP: &str = ct_help_section!("after help", "tee.md");

mod stat_flags {
    pub const TEE_APPEND: &str = "append";
    pub const TEE_IGNORE_INTERRUPTS: &str = "ignore-interrupts";
    pub const TEE_FILE: &str = "file";
    pub const TEE_IGNORE_PIPE_ERRORS: &str = "ignore-pipe-errors";
    pub const TEE_OUTPUT_ERROR: &str = "output-error";
}

#[allow(dead_code)]
#[derive(Default)]
struct StatOptions {
    is_append: bool,
    is_ignore_interrupts: bool,
    files: Vec<String>,
    output_error: Option<OutputErrorMode>,
}

impl StatOptions {
    fn new(matches: &clap::ArgMatches) -> Self {
        Self {
            is_append: matches.get_flag(stat_flags::TEE_APPEND),
            is_ignore_interrupts: matches.get_flag(stat_flags::TEE_IGNORE_INTERRUPTS),
            files: get_file_list(matches),
            output_error: get_output_error_mode(matches),
        }
    }
}

#[derive(Clone, Debug)]
enum OutputErrorMode {
    Warn,
    WarnNoPipe,
    Exit,
    ExitNoPipe,
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    tee_main(args)
}

pub fn tee_main(args: impl ctcore::Args) -> CTResult<()> {
    let matches = ct_app().try_get_matches_from(args)?;
    let options = StatOptions::new(&matches);

    match run_tee(&options) {
        Ok(_) => Ok(()),
        Err(_) => Err(1.into()),
    }
}

fn get_file_list(matches: &clap::ArgMatches) -> Vec<String> {
    matches
        .get_many::<String>(stat_flags::TEE_FILE)
        .map(|v| v.map(ToString::to_string).collect())
        .unwrap_or_default()
}

fn get_output_error_mode(matches: &clap::ArgMatches) -> Option<OutputErrorMode> {
    if matches.get_flag(stat_flags::TEE_IGNORE_PIPE_ERRORS) {
        return Some(OutputErrorMode::WarnNoPipe);
    }

    matches
        .get_one::<String>(stat_flags::TEE_OUTPUT_ERROR)
        .map(|v| {
            match v.as_str() {
                "warn" => OutputErrorMode::Warn,
                "warn-nopipe" => OutputErrorMode::WarnNoPipe,
                "exit" => OutputErrorMode::Exit,
                "exit-nopipe" => OutputErrorMode::ExitNoPipe,
                _ => OutputErrorMode::WarnNoPipe, // 默认行为
            }
        })
}

fn run_tee(options: &StatOptions) -> Result<()> {
    #[cfg(unix)]
    setup_signal_handlers(options)?;

    let writers = create_writers(options)?;
    let mut input = NamedReader {
        inner: Box::new(stdin()) as Box<dyn Read>,
    };

    let mut output = MultiWriter::new(writers, options.output_error.clone());
    match copy(&mut input, &mut output) {
        Err(e) if e.kind() != ErrorKind::Other => return Err(e),
        _ => (),
    }

    if output.flush().is_err() || output.error_occurred() {
        Err(Error::from(ErrorKind::Other))
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn setup_signal_handlers(options: &StatOptions) -> Result<()> {
    if options.is_ignore_interrupts {
        ignore_interrupts().map_err(|_| Error::from(ErrorKind::Other))?;
    }
    if options.output_error.is_none() {
        enable_pipe_errors().map_err(|_| Error::from(ErrorKind::Other))?;
    }
    Ok(())
}

fn create_writers(options: &StatOptions) -> Result<Vec<NamedWriter>> {
    let mut writers = options
        .files
        .iter()
        .map(|file| {
            Ok(NamedWriter {
                name: file.clone(),
                inner: open(
                    file.clone(),
                    options.is_append,
                    options.output_error.as_ref(),
                )?,
            })
        })
        .collect::<Result<Vec<NamedWriter>>>()?;

    // 添加标准输出作为第一个写入器
    writers.insert(
        0,
        NamedWriter {
            name: "'standard output'".to_owned(),
            inner: Box::new(stdout()),
        },
    );

    Ok(writers)
}

pub fn ct_app() -> Command {
    let args = vec![
        Arg::new(stat_flags::TEE_APPEND)
                .long(stat_flags::TEE_APPEND)
                .short('a')
                .help("append to the given FILEs, do not overwrite")
                .action(ArgAction::SetTrue),

        Arg::new(stat_flags::TEE_IGNORE_INTERRUPTS)
            .long(stat_flags::TEE_IGNORE_INTERRUPTS)
            .short('i')
            .help("ignore interrupt signals (ignored on non-Unix platforms)")
            .action(ArgAction::SetTrue),

        Arg::new(stat_flags::TEE_FILE)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::FilePath),

        Arg::new(stat_flags::TEE_IGNORE_PIPE_ERRORS)
            .short('p')
            .help("set write error behavior (ignored on non-Unix platforms)")
            .action(ArgAction::SetTrue),

        Arg::new(stat_flags::TEE_OUTPUT_ERROR)
            .long(stat_flags::TEE_OUTPUT_ERROR)
            .require_equals(true)
            .num_args(0..=1)
            .value_parser([
                PossibleValue::new("warn")
                    .help("produce warnings for errors writing to any output"),
                PossibleValue::new("warn-nopipe")
                    .help("produce warnings for errors that are not pipe errors (ignored on non-unix platforms)"),
                PossibleValue::new("exit").help("exit on write errors to any output"),
                PossibleValue::new("exit-nopipe")
                    .help("exit on write errors to any output that are not pipe errors (equivalent to exit on non-unix platforms)"),
            ])
            .help("set write error behavior")
            .conflicts_with(stat_flags::TEE_IGNORE_PIPE_ERRORS),

    ];

    Command::new(ctcore::ct_util_name())
        .version(crate_version!())
        .about(TEE_ABOUT)
        .override_usage(ct_format_usage(TEE_USAGE))
        .after_help(TEE_AFTER_HELP)
        .infer_long_args(true)
        .args(args)
}

fn open(
    name: String,
    append: bool,
    output_error: Option<&OutputErrorMode>,
) -> Result<Box<dyn Write>> {
    let path = PathBuf::from(name.clone());
    let inner: Box<dyn Write> = {
        let mut options = OpenOptions::new();
        let mode = if append {
            options.append(true)
        } else {
            options.truncate(true)
        };
        match mode.write(true).create(true).open(path.as_path()) {
            Ok(file) => Box::new(file),
            Err(f) => {
                ct_show_error!("{}: {}", name.maybe_quote(), f);
                match output_error {
                    Some(OutputErrorMode::Exit | OutputErrorMode::ExitNoPipe) => return Err(f),
                    _ => Box::new(sink()),
                }
            }
        }
    };
    Ok(Box::new(NamedWriter { inner, name }) as Box<dyn Write>)
}

struct MultiWriter {
    writers: Vec<NamedWriter>,
    output_error_mode: Option<OutputErrorMode>,
    ignored_errors: usize,
}

impl MultiWriter {
    fn new(writers: Vec<NamedWriter>, output_error_mode: Option<OutputErrorMode>) -> Self {
        Self {
            writers,
            output_error_mode,
            ignored_errors: 0,
        }
    }

    fn error_occurred(&self) -> bool {
        self.ignored_errors != 0
    }
}

fn process_error(
    mode: Option<&OutputErrorMode>,
    f: Error,
    writer: &NamedWriter,
    ignored_errors: &mut usize,
) -> Result<()> {
    match mode {
        Some(OutputErrorMode::Warn) => {
            ct_show_error!("{}: {}", writer.name.maybe_quote(), f);
            *ignored_errors += 1;
            Ok(())
        }
        Some(OutputErrorMode::WarnNoPipe) | None => {
            if f.kind() != ErrorKind::BrokenPipe {
                ct_show_error!("{}: {}", writer.name.maybe_quote(), f);
                *ignored_errors += 1;
            }
            Ok(())
        }
        Some(OutputErrorMode::Exit) => {
            ct_show_error!("{}: {}", writer.name.maybe_quote(), f);
            Err(f)
        }
        Some(OutputErrorMode::ExitNoPipe) => {
            if f.kind() == ErrorKind::BrokenPipe {
                Ok(())
            } else {
                ct_show_error!("{}: {}", writer.name.maybe_quote(), f);
                Err(f)
            }
        }
    }
}

impl Write for MultiWriter {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        let mut aborted = None;
        let mode = self.output_error_mode.clone();
        let mut errors = 0;
        self.writers.retain_mut(|writer| {
            let result = writer.write_all(buf);
            match result {
                Err(f) => {
                    if let Err(e) = process_error(mode.as_ref(), f, writer, &mut errors) {
                        if aborted.is_none() {
                            aborted = Some(e);
                        }
                    }
                    false
                }
                _ => true,
            }
        });
        self.ignored_errors += errors;
        if let Some(e) = aborted {
            Err(e)
        } else if self.writers.is_empty() {
            // 标准库永远不会引发此错误类型，因此我们可以使用它来提前终止 `copy`
            Err(Error::from(ErrorKind::Other))
        } else {
            Ok(buf.len())
        }
    }

    fn flush(&mut self) -> Result<()> {
        let mut aborted = None;
        let mode = self.output_error_mode.clone();
        let mut errors = 0;
        self.writers.retain_mut(|writer| {
            let result = writer.flush();
            match result {
                Err(f) => {
                    if let Err(e) = process_error(mode.as_ref(), f, writer, &mut errors) {
                        if aborted.is_none() {
                            aborted = Some(e);
                        }
                    }
                    false
                }
                _ => true,
            }
        });
        self.ignored_errors += errors;
        if let Some(e) = aborted {
            Err(e)
        } else {
            Ok(())
        }
    }
}

struct NamedWriter {
    inner: Box<dyn Write>,
    pub name: String,
}

impl Write for NamedWriter {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> Result<()> {
        self.inner.flush()
    }
}

struct NamedReader {
    inner: Box<dyn Read>,
}

impl Read for NamedReader {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        match self.inner.read(buf) {
            Err(f) => {
                ct_show_error!("stdin: {}", f);
                Err(f)
            }
            okay => okay,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stat_options_default() {
        let options = StatOptions::default();
        assert!(!options.is_append);
        assert!(!options.is_ignore_interrupts);
        assert!(options.files.is_empty());
        assert!(options.output_error.is_none());
    }

    #[test]
    fn test_stat_options_new() {
        let matches = ct_app()
            .try_get_matches_from(["tee", "-a", "file.txt"])
            .unwrap();
        let options = StatOptions::new(&matches);

        assert!(options.is_append);
        assert!(!options.is_ignore_interrupts);
        assert_eq!(options.files, vec!["file.txt"]);
        assert!(options.output_error.is_none());
    }
}
