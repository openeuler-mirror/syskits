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

//! 将各个 <文件> 里相同行号的行合并成一行，中间用制表符分隔，并输出到标准输出。
//! 如果没有指定 <文件>，或者 <文件> 为 "-"，则从标准输入读取。
//! 如果指定了 -s 选项，则将各个 <文件> 里的行按顺序合并成一行，中间用制表符分隔，并输出到标准输出。
//! 如果指定了 -d 选项，则使用指定的字符代替制表符分隔各个行。
//! 如果指定了 -z 选项，则使用 NUL 字符代替换行符作为行分隔符。

extern crate rust_i18n;
use clap::{Arg, ArgAction, Command, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::Tool;
use ctcore::ct_error::{CTResult, CTsageError, CtSimpleError, FromIo};
use ctcore::ct_line_ending::CtLineEnding;
use std::ffi::OsString;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write, stdout};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use sys_locale::get_locale;

mod paste_flags {
    pub const PASTE_DELIMITER: &str = "delimiters";
    pub const PASTE_SERIAL: &str = "serial";
    pub const PASTE_FILE: &str = "file";
    pub const PASTE_ZERO_TERMINATED: &str = "zero-terminated";
}

const GNU_PASTE_LONG_OPTIONS: &[&str] =
    &["serial", "delimiters", "zero-terminated", "help", "version"];

fn validate_gnu_paste_options(args: &[OsString]) -> CTResult<()> {
    let mut index = 1;
    while index < args.len() {
        let argument = args[index].as_os_str().as_bytes();
        if argument == b"--" {
            break;
        }

        if let Some(long_argument) = argument.strip_prefix(b"--") {
            if long_argument.is_empty() {
                break;
            }

            let option_end = long_argument
                .iter()
                .position(|byte| *byte == b'=')
                .unwrap_or(long_argument.len());
            let option_name = String::from_utf8_lossy(&long_argument[..option_end]);
            let supplied_value = option_end < long_argument.len();
            let mut candidates = GNU_PASTE_LONG_OPTIONS
                .iter()
                .copied()
                .filter(|candidate| candidate.starts_with(option_name.as_ref()))
                .collect::<Vec<_>>();

            if let Some(exact) = candidates
                .iter()
                .position(|candidate| *candidate == option_name)
            {
                candidates = vec![candidates[exact]];
            }

            if candidates.is_empty() {
                return Err(CTsageError::new(
                    1,
                    format!(
                        "unrecognized option '{}'",
                        args[index].as_os_str().to_string_lossy()
                    ),
                ));
            }

            if candidates.len() > 1 {
                let possibilities = candidates
                    .iter()
                    .map(|candidate| format!("'--{candidate}'"))
                    .collect::<Vec<_>>()
                    .join(" ");
                return Err(CTsageError::new(
                    1,
                    format!(
                        "option '{}' is ambiguous; possibilities: {possibilities}",
                        args[index].as_os_str().to_string_lossy()
                    ),
                ));
            }

            let option = candidates[0];
            if option == "delimiters" {
                if !supplied_value {
                    index += 1;
                    if index == args.len() {
                        return Err(CTsageError::new(
                            1,
                            "option '--delimiters' requires an argument",
                        ));
                    }
                }
            } else if supplied_value {
                return Err(CTsageError::new(
                    1,
                    format!("option '--{option}' doesn't allow an argument"),
                ));
            } else if matches!(option, "help" | "version") {
                return Ok(());
            }
        } else if argument.starts_with(b"-") && argument.len() > 1 {
            let mut short_index = 1;
            while short_index < argument.len() {
                let option = argument[short_index];
                if option == b'd' {
                    if short_index + 1 == argument.len() {
                        index += 1;
                        if index == args.len() {
                            return Err(CTsageError::new(1, "option requires an argument -- 'd'"));
                        }
                    }
                    break;
                }

                // -h and -V are retained as syskits extensions.
                if !matches!(option, b's' | b'z' | b'h' | b'V') {
                    return Err(CTsageError::new(
                        1,
                        format!("invalid option -- '{}'", char::from(option)),
                    ));
                }
                if matches!(option, b'h' | b'V') {
                    return Ok(());
                }
                short_index += 1;
            }
        }

        index += 1;
    }

    Ok(())
}

fn apply_posix_option_order(args: &mut Vec<OsString>, posixly_correct: bool) {
    if !posixly_correct {
        return;
    }

    let mut index = 1;
    while index < args.len() {
        let argument = args[index].as_os_str().as_bytes();
        if argument == b"--" {
            return;
        }
        if argument == b"-" || !argument.starts_with(b"-") {
            args.insert(index, OsString::from("--"));
            return;
        }

        if let Some(long_argument) = argument.strip_prefix(b"--") {
            let option = long_argument
                .split(|byte| *byte == b'=')
                .next()
                .unwrap_or_default();
            if !option.is_empty()
                && b"delimiters".starts_with(option)
                && !long_argument.contains(&b'=')
            {
                index += 2;
                continue;
            }
        } else if let Some(short_options) = argument.strip_prefix(b"-") {
            if let Some(delimiter_index) = short_options.iter().position(|byte| *byte == b'd') {
                if delimiter_index + 1 == short_options.len() {
                    index += 2;
                    continue;
                }
            }
        }

        index += 1;
    }
}

/// 表示一个输入源，可以是文件或标准输入
enum InputSource {
    File(BufReader<File>),
    Stdin,
}

/// 存储粘贴操作的配置参数
#[derive(Debug)]
struct PasteFlags {
    is_serial: bool,
    delimiters: OsString,
    files: Vec<OsString>,
    line_ending: CtLineEnding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteMode {
    Parallel,
    Serial,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasteRow {
    pub row_index: usize,
    pub line: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasteSemantic {
    pub mode: PasteMode,
    pub rows: Vec<PasteRow>,
    pub classic_text: String,
    pub stderr_text: String,
    pub exit_code: i32,
}

/// 表示粘贴操作的上下文
struct PasteContext<W: Write> {
    files: Vec<InputSource>,
    stdin: BufReader<Box<dyn Read>>,
    delimiters: Vec<u8>,
    line_ending: CtLineEnding,
    output: Vec<u8>,
    delimiter_positions: Vec<(usize, u8)>,
    rows: Vec<PasteRow>,
    emitted_rows: usize,
    writer: W,
}

impl PasteFlags {
    /// 从命令行参数创建 PasteFlags 实例
    ///
    /// # Arguments
    /// * `matches` - 解析后的命令行参数
    ///
    /// # Returns
    /// * `CTResult<Self>` - 成功则返回 PasteFlags 实例，失败则返回错误
    fn new(matches: &clap::ArgMatches) -> CTResult<Self> {
        Ok(Self {
            is_serial: matches.get_flag(paste_flags::PASTE_SERIAL),
            delimiters: matches
                .get_one::<OsString>(paste_flags::PASTE_DELIMITER)
                .unwrap()
                .clone(),
            files: matches
                .get_many::<OsString>(paste_flags::PASTE_FILE)
                .unwrap()
                .cloned()
                .collect(),
            line_ending: CtLineEnding::from_zero_flag(
                matches.get_flag(paste_flags::PASTE_ZERO_TERMINATED),
            ),
        })
    }

    fn validate_delimiters(&self) -> CTResult<()> {
        let trailing_backslashes = self
            .delimiters
            .as_os_str()
            .as_bytes()
            .iter()
            .rev()
            .take_while(|byte| **byte == b'\\')
            .count();
        if trailing_backslashes % 2 == 1 {
            return Err(CtSimpleError::new(
                1,
                format!(
                    "delimiter list ends with an unescaped backslash: {}",
                    self.delimiters.to_string_lossy()
                ),
            ));
        }
        Ok(())
    }
}

impl<W: Write> PasteContext<W> {
    fn new(flags: &PasteFlags, writer: W) -> CTResult<Self> {
        let mut files = Vec::with_capacity(flags.files.len());
        for name in &flags.files {
            let file = if name.as_os_str().as_bytes() == b"-" {
                InputSource::Stdin
            } else {
                let path = Path::new(name);
                let r = File::open(path).map_err_context(String::new)?;
                InputSource::File(BufReader::new(r))
            };
            files.push(file);
        }

        Ok(Self {
            files,
            stdin: BufReader::new(ctcore::ct_io::stdin_reader_box()),
            delimiters: paste_unescape(flags.delimiters.as_os_str().as_bytes()),
            line_ending: flags.line_ending,
            output: Vec::new(),
            delimiter_positions: Vec::new(),
            rows: Vec::new(),
            emitted_rows: 0,
            writer,
        })
    }

    fn write_row_bytes(&mut self, row: &[u8]) -> CTResult<()> {
        let mut classic_bytes = Vec::with_capacity(row.len());
        let mut segment_start = 0;
        for &(position, delimiter) in &self.delimiter_positions {
            if position >= row.len() {
                break;
            }
            classic_bytes.extend_from_slice(
                String::from_utf8_lossy(&row[segment_start..position]).as_bytes(),
            );
            classic_bytes.push(delimiter);
            segment_start = position + 1;
        }
        classic_bytes
            .extend_from_slice(String::from_utf8_lossy(&row[segment_start..]).as_bytes());

        let rendered = String::from_utf8_lossy(&classic_bytes).into_owned();
        self.emitted_rows += 1;
        self.rows.push(PasteRow {
            row_index: self.emitted_rows,
            line: rendered.clone(),
        });
        self.writer
            .write_all(&classic_bytes)
            .map_err(|e| CtSimpleError::new(1, e.to_string()))?;
        self.writer
            .write_all(&[self.line_ending as u8])
            .map_err(|e| CtSimpleError::new(1, e.to_string()))?;
        Ok(())
    }

    fn process_stream(
        reader: &mut dyn BufRead,
        output: &mut Vec<u8>,
        delimiter_positions: &mut Vec<(usize, u8)>,
        writer: &mut W,
        line_ending: u8,
    ) -> CTResult<bool> {
        let mut has_content = false;

        loop {
            let buf = reader
                .fill_buf()
                .map_err(|e| CtSimpleError::new(1, e.to_string()))?;
            if buf.is_empty() {
                return Ok(has_content);
            }
            let (consumed, hit_eol) = match buf.iter().position(|&b| b == line_ending) {
                Some(i) => (i + 1, true),
                None => (buf.len(), false),
            };
            output.extend_from_slice(&buf[..consumed]);
            reader.consume(consumed);
            has_content = true;

            // 一旦缓冲区超过 64KB，强制提前刷入底层 writer。
            // 如果底层是 /dev/full，会立即在此处触发 ENOSPC 错误并安全退出。
            if output.len() > 65536 {
                writer
                    .write_all(output)
                    .map_err(|e| CtSimpleError::new(1, e.to_string()))?;
                output.clear();
                delimiter_positions.clear();
            }

            if hit_eol {
                if output.ends_with(&[line_ending]) {
                    output.pop();
                }
                break;
            }
        }
        Ok(has_content)
    }

    fn process_line_at(&mut self, index: usize) -> CTResult<bool> {
        let line_ending = self.line_ending as u8;
        match &mut self.files[index] {
            InputSource::File(reader) => {
                Self::process_stream(
                    reader,
                    &mut self.output,
                    &mut self.delimiter_positions,
                    &mut self.writer,
                    line_ending,
                )
            }
            InputSource::Stdin => Self::process_stream(
                &mut self.stdin,
                &mut self.output,
                &mut self.delimiter_positions,
                &mut self.writer,
                line_ending,
            ),
        }
    }

    fn write_output(&mut self, delim_length: usize) -> CTResult<()> {
        if !self.output.is_empty() {
            let row_end = self.output.len().saturating_sub(delim_length);
            let row = self.output[..row_end].to_vec();
            self.write_row_bytes(&row)?;
        }
        Ok(())
    }

    fn add_delimiter(&mut self, delim_count: usize) -> usize {
        // GNU 规范：如果定界符列表为空（例如 -d ''），则不添加任何定界符
        if self.delimiters.is_empty() {
            return 0;
        }

        let delimiter = self.delimiters[delim_count % self.delimiters.len()];
        if delimiter == b'\0' {
            return 0;
        }

        self.delimiter_positions
            .push((self.output.len(), delimiter));
        self.output.push(delimiter);
        1
    }

    fn paste_serial(&mut self) -> CTResult<()> {
        for i in 0..self.files.len() {
            self.output.clear();
            self.delimiter_positions.clear();
            let mut delim_length = 1;
            let mut delim_count = 0;
            while self.process_line_at(i)? {
                delim_length = self.add_delimiter(delim_count);
                delim_count += 1;
            }
            self.write_output(delim_length)?;
        }
        Ok(())
    }

    fn paste_parallel(&mut self) -> CTResult<()> {
        let mut eof = vec![false; self.files.len()];
        let mut delim_count = 0;

        loop {
            self.output.clear();
            self.delimiter_positions.clear();
            let mut eof_count = 0;
            let mut has_content = false;

            for (i, eof_item) in eof.iter_mut().enumerate() {
                if i > 0 {
                    self.add_delimiter(delim_count);
                    delim_count += 1;
                }

                if *eof_item {
                    eof_count += 1;
                    continue;
                }

                if !self.process_line_at(i)? {
                    *eof_item = true;
                    eof_count += 1;
                } else {
                    has_content = true;
                }
            }

            if self.files.len() == eof_count {
                break;
            }

            if has_content {
                let row = self.output.clone();
                self.write_row_bytes(&row)?;
            }
            delim_count = 0;
        }
        Ok(())
    }

    fn into_rows(self) -> Vec<PasteRow> {
        self.rows
    }
}

fn paste_unescape(s: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(s.len());
    let mut bytes = s.iter().copied();
    while let Some(byte) = bytes.next() {
        if byte != b'\\' {
            output.push(byte);
            continue;
        }

        match bytes.next() {
            Some(b'0') => output.push(0),
            Some(b'b') => output.push(0x08),
            Some(b'f') => output.push(0x0c),
            Some(b'n') => output.push(b'\n'),
            Some(b'r') => output.push(b'\r'),
            Some(b't') => output.push(b'\t'),
            Some(b'v') => output.push(0x0b),
            Some(escaped_byte) => output.push(escaped_byte),
            None => output.push(b'\\'),
        }
    }
    output
}

pub fn paste_main<W: Write>(writer: &mut W, args: impl ctcore::Args) -> CTResult<()> {
    // 设置语言
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let mut args = args.into_iter().collect::<Vec<_>>();
    apply_posix_option_order(&mut args, std::env::var_os("POSIXLY_CORRECT").is_some());
    validate_gnu_paste_options(&args)?;
    let matches = ct_app().try_get_matches_from(args)?;
    let flags = PasteFlags::new(&matches)?;
    flags.validate_delimiters()?;

    paste_exec(writer, flags)
}

fn paste_exec<W: Write>(writer: &mut W, flags: PasteFlags) -> CTResult<()> {
    paste_exec_collect(writer, flags).map(|_| ())
}

fn paste_exec_collect<W: Write>(writer: &mut W, flags: PasteFlags) -> CTResult<Vec<PasteRow>> {
    let mut context = PasteContext::new(&flags, writer)?;
    if flags.is_serial {
        context.paste_serial()?;
    } else {
        context.paste_parallel()?;
    }
    Ok(context.into_rows())
}

pub fn paste_native_semantic(args: impl ctcore::Args) -> CTResult<PasteSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let mut args = args.into_iter().collect::<Vec<_>>();
    apply_posix_option_order(&mut args, std::env::var_os("POSIXLY_CORRECT").is_some());
    validate_gnu_paste_options(&args)?;
    let matches = ct_app().try_get_matches_from(args)?;
    let flags = PasteFlags::new(&matches)?;
    flags.validate_delimiters()?;

    let mode = if flags.is_serial {
        PasteMode::Serial
    } else {
        PasteMode::Parallel
    };

    let mut classic_output = Vec::new();
    let rows = paste_exec_collect(&mut classic_output, flags)?;

    Ok(PasteSemantic {
        mode,
        rows,
        classic_text: String::from_utf8_lossy(&classic_output).into_owned(),
        stderr_text: String::new(),
        exit_code: 0,
    })
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("paste.about");
    let usage_description = t!("paste.usage");
    let args = vec![
        Arg::new(paste_flags::PASTE_SERIAL)
            .long(paste_flags::PASTE_SERIAL)
            .short('s')
            .help(t!("paste.clap.paste_serial"))
            .action(ArgAction::SetTrue),
        Arg::new(paste_flags::PASTE_DELIMITER)
            .long(paste_flags::PASTE_DELIMITER)
            .short('d')
            .help(t!("paste.clap.paste_delimiter"))
            .value_name("LIST")
            .default_value("\t")
            .hide_default_value(true)
            .value_parser(clap::builder::ValueParser::os_string()),
        Arg::new(paste_flags::PASTE_FILE)
            .value_name("FILE")
            .action(ArgAction::Append)
            .default_value("-")
            .value_parser(clap::builder::ValueParser::os_string())
            .value_hint(clap::ValueHint::FilePath),
        Arg::new(paste_flags::PASTE_ZERO_TERMINATED)
            .long(paste_flags::PASTE_ZERO_TERMINATED)
            .short('z')
            .help(t!("paste.clap.paste_zero_terminated"))
            .action(ArgAction::SetTrue),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args_override_self(true)
        .args(&args)
}

#[derive(Default)]
pub struct Paste;
impl Tool for Paste {
    fn name(&self) -> &'static str {
        "paste"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        let mut stdout = stdout().lock();
        paste_main(&mut stdout, args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use tempfile::Builder;

    mod paste_flags_tests {
        use super::*;

        #[test]
        fn test_paste_flags_new() {
            let args = vec![
                ctcore::ct_util_name(),
                "-s",
                "-d",
                ",",
                "file1.txt",
                "file2.txt",
            ];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let flags = PasteFlags::new(&matches).unwrap();

            assert!(flags.is_serial);
            assert_eq!(flags.delimiters, OsString::from(","));
            assert_eq!(
                flags.files,
                vec![OsString::from("file1.txt"), OsString::from("file2.txt")]
            );
            assert_eq!(flags.line_ending, CtLineEnding::Newline);
        }

        #[test]
        fn test_validate_delimiters_valid() {
            let flags = PasteFlags {
                is_serial: false,
                delimiters: ",:".into(),
                files: vec!["file.txt".into()],
                line_ending: CtLineEnding::Newline,
            };
            assert!(flags.validate_delimiters().is_ok());
        }

        #[test]
        fn test_validate_delimiters_invalid() {
            for delimiters in ["\\", "\\\\\\", "\\\\\\\\\\"] {
                let flags = PasteFlags {
                    is_serial: false,
                    delimiters: delimiters.into(),
                    files: vec!["file.txt".into()],
                    line_ending: CtLineEnding::Newline,
                };
                assert!(flags.validate_delimiters().is_err());
            }
        }

        #[test]
        fn test_paste_flags_with_zero_terminator() {
            let args = vec![ctcore::ct_util_name(), "-z", "file.txt"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let flags = PasteFlags::new(&matches).unwrap();
            assert_eq!(flags.line_ending, CtLineEnding::Nul);
        }

        #[test]
        fn test_paste_flags_with_custom_delimiter() {
            let args = vec![ctcore::ct_util_name(), "-d", ":|", "file.txt"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let flags = PasteFlags::new(&matches).unwrap();
            assert_eq!(flags.delimiters, OsString::from(":|"));
        }

        #[test]
        fn test_validate_delimiters_with_escaped_backslash() {
            let flags = PasteFlags {
                is_serial: false,
                delimiters: "\\\\".into(),
                files: vec!["file.txt".into()],
                line_ending: CtLineEnding::Newline,
            };
            assert!(flags.validate_delimiters().is_ok());
        }
    }

    mod paste_context_tests {
        use super::*;

        fn create_test_file(content: &str) -> (tempfile::TempDir, String) {
            let temp_dir = Builder::new().prefix("paste_test").tempdir().unwrap();
            let file_path = temp_dir.path().join("test.txt");
            std::fs::write(&file_path, content).unwrap();
            (temp_dir, file_path.to_str().unwrap().to_string())
        }

        #[test]
        fn test_process_line() -> CTResult<()> {
            let (_temp_dir, file_path) = create_test_file("test\nline\n");
            let flags = PasteFlags {
                is_serial: false,
                delimiters: "\t".into(),
                files: vec![file_path.into()],
                line_ending: CtLineEnding::Newline,
            };

            let mut output = Vec::new();
            let mut context = PasteContext::new(&flags, &mut output)?;

            assert!(context.process_line_at(0)?);
            assert_eq!(String::from_utf8_lossy(&context.output), "test");

            assert!(context.process_line_at(0)?);
            assert_eq!(String::from_utf8_lossy(&context.output), "testline");

            assert!(!context.process_line_at(0)?);
            Ok(())
        }

        #[test]
        fn test_add_delimiter() {
            let flags = PasteFlags {
                is_serial: false,
                delimiters: ",".into(),
                files: vec![],
                line_ending: CtLineEnding::Newline,
            };

            let mut output = Vec::new();
            let mut context = PasteContext::new(&flags, &mut output).unwrap();
            context.output.extend_from_slice(b"test");

            let len = context.add_delimiter(0);
            assert_eq!(len, 1);
            assert_eq!(String::from_utf8_lossy(&context.output), "test,");
        }

        #[test]
        fn test_write_output() -> CTResult<()> {
            let flags = PasteFlags {
                is_serial: false,
                delimiters: ",".into(),
                files: vec![],
                line_ending: CtLineEnding::Newline,
            };

            let mut output = Vec::new();
            let mut context = PasteContext::new(&flags, &mut output)?;
            context.output.extend_from_slice(b"test,");
            context.write_output(1)?;

            assert_eq!(String::from_utf8_lossy(&output), "test\n");
            Ok(())
        }

        #[test]
        fn test_process_line_with_stdin() -> CTResult<()> {
            let flags = PasteFlags {
                is_serial: false,
                delimiters: "\t".into(),
                files: vec!["-".into()],
                line_ending: CtLineEnding::Newline,
            };

            let mut output = Vec::new();
            let mut context = PasteContext::new(&flags, &mut output)?;

            ctcore::ct_io::with_injected_stdin(b"1\n2\n".to_vec(), || -> CTResult<()> {
                context.stdin = BufReader::new(ctcore::ct_io::stdin_reader_box());
                assert!(context.process_line_at(0)?);
                assert_eq!(String::from_utf8_lossy(&context.output), "1");

                assert!(context.process_line_at(0)?);
                assert_eq!(String::from_utf8_lossy(&context.output), "12");

                assert!(!context.process_line_at(0)?);
                Ok(())
            })?;
            Ok(())
        }

        #[test]
        fn test_process_line_with_zero_terminator() -> CTResult<()> {
            let (_temp_dir, file_path) = create_test_file("test\0line\0");
            let flags = PasteFlags {
                is_serial: false,
                delimiters: "\t".into(),
                files: vec![file_path.into()],
                line_ending: CtLineEnding::Nul,
            };

            let mut output = Vec::new();
            let mut context = PasteContext::new(&flags, &mut output)?;

            assert!(context.process_line_at(0)?);
            assert_eq!(String::from_utf8_lossy(&context.output), "test");
            Ok(())
        }

        #[test]
        fn test_add_delimiter_with_multiple_chars() {
            let flags = PasteFlags {
                is_serial: false,
                delimiters: ":|".into(),
                files: vec![],
                line_ending: CtLineEnding::Newline,
            };

            let mut output = Vec::new();
            let mut context = PasteContext::new(&flags, &mut output).unwrap();
            context.output.extend_from_slice(b"test");

            let len1 = context.add_delimiter(0);
            let len2 = context.add_delimiter(1);
            assert_eq!(len1, 1); // ':'
            assert_eq!(len2, 1); // '|'
            assert_eq!(String::from_utf8_lossy(&context.output), "test:|");
        }

        #[test]
        fn test_add_delimiter_cycles_utf8_bytes() {
            let flags = PasteFlags {
                is_serial: false,
                delimiters: "é".into(),
                files: vec![],
                line_ending: CtLineEnding::Newline,
            };

            let mut output = Vec::new();
            let mut context = PasteContext::new(&flags, &mut output).unwrap();
            context.output.extend_from_slice(b"test");

            assert_eq!(context.add_delimiter(0), 1);
            assert_eq!(context.add_delimiter(1), 1);
            assert_eq!(context.output, b"test\xc3\xa9");
        }

        #[test]
        fn test_write_output_empty() -> CTResult<()> {
            let flags = PasteFlags {
                is_serial: false,
                delimiters: ",".into(),
                files: vec![],
                line_ending: CtLineEnding::Newline,
            };

            let mut output = Vec::new();
            let mut context = PasteContext::new(&flags, &mut output)?;
            context.write_output(0)?;

            assert_eq!(output.len(), 0);
            Ok(())
        }
    }

    mod paste_main_tests {
        use super::*;

        #[test]
        fn test_paste_main_basic() -> CTResult<()> {
            let (_temp_dir, file_path) = create_test_file("test\n");
            let args = [ctcore::ct_util_name(), &file_path];

            let mut output = Vec::new();
            paste_main(&mut output, args.iter().map(OsString::from))?;

            assert_eq!(String::from_utf8_lossy(&output), "test\n");
            Ok(())
        }

        #[test]
        fn test_paste_main_invalid_args() {
            let args = [ctcore::ct_util_name(), "--invalid-flag"];
            let mut output = Vec::new();
            assert!(paste_main(&mut output, args.iter().map(OsString::from)).is_err());
        }

        #[test]
        fn test_paste_main_non_utf8_arguments() -> CTResult<()> {
            let temp_dir = tempfile::tempdir()?;
            let file_path = temp_dir
                .path()
                .join(OsString::from_vec(b"input-\xff".to_vec()));
            std::fs::write(&file_path, b"N\n")?;

            let args = vec![
                OsString::from(ctcore::ct_util_name()),
                OsString::from("-d"),
                OsString::from_vec(vec![0xfe, b',']),
                file_path.clone().into_os_string(),
                file_path.into_os_string(),
            ];
            let mut output = Vec::new();
            paste_main(&mut output, args.into_iter())?;

            assert_eq!(output, b"N\xfeN\n");
            Ok(())
        }
    }

    mod ct_app_tests {
        use super::*;
        use clap::error::ErrorKind;

        #[test]
        fn test_ct_app_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = ct_app().try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = ct_app().try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_serial_option() {
            let args = vec![ctcore::ct_util_name(), "-s", "file.txt"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            assert!(matches.get_flag(paste_flags::PASTE_SERIAL));
        }

        #[test]
        fn test_ct_app_delimiter_option() {
            let args = vec![ctcore::ct_util_name(), "-d", ",", "file.txt"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            assert_eq!(
                matches
                    .get_one::<OsString>(paste_flags::PASTE_DELIMITER)
                    .unwrap(),
                &OsString::from(",")
            );
        }

        #[test]
        fn test_ct_app_combined_options() {
            let args = vec![ctcore::ct_util_name(), "-sd", ",", "file.txt"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            assert!(matches.get_flag(paste_flags::PASTE_SERIAL));
            assert_eq!(
                matches
                    .get_one::<OsString>(paste_flags::PASTE_DELIMITER)
                    .unwrap(),
                &OsString::from(",")
            );
        }

        #[test]
        fn test_ct_app_zero_terminated_option() {
            let args = vec![ctcore::ct_util_name(), "-z", "file.txt"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            assert!(matches.get_flag(paste_flags::PASTE_ZERO_TERMINATED));
        }

        #[test]
        fn test_ct_app_multiple_files() {
            let args = vec![
                ctcore::ct_util_name(),
                "file1.txt",
                "file2.txt",
                "file3.txt",
            ];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let files: Vec<_> = matches
                .get_many::<OsString>(paste_flags::PASTE_FILE)
                .unwrap()
                .collect();
            assert_eq!(files.len(), 3);
        }

        #[test]
        fn test_ct_app_repeated_options() {
            let matches = ct_app()
                .try_get_matches_from([
                    ctcore::ct_util_name(),
                    "-s",
                    "-s",
                    "-z",
                    "-z",
                    "-d",
                    ":",
                    "-d",
                    ",",
                    "file.txt",
                ])
                .unwrap();

            assert!(matches.get_flag(paste_flags::PASTE_SERIAL));
            assert!(matches.get_flag(paste_flags::PASTE_ZERO_TERMINATED));
            assert_eq!(
                matches
                    .get_one::<OsString>(paste_flags::PASTE_DELIMITER)
                    .unwrap(),
                &OsString::from(",")
            );
        }

        #[test]
        fn test_gnu_option_diagnostics() {
            for (args, expected) in [
                (
                    vec!["paste", "--definitely-unknown"],
                    "unrecognized option '--definitely-unknown'",
                ),
                (
                    vec!["paste", "--serial=yes"],
                    "option '--serial' doesn't allow an argument",
                ),
                (vec!["paste", "-d"], "option requires an argument -- 'd'"),
                (vec!["paste", "-sx"], "invalid option -- 'x'"),
            ] {
                let args = args.into_iter().map(OsString::from).collect::<Vec<_>>();
                let error = validate_gnu_paste_options(&args).unwrap_err();
                assert_eq!(error.to_string(), expected);
            }
        }

        #[test]
        fn test_posix_option_order_stops_at_first_operand() {
            let mut args = ["paste", "-d", ",", "input", "-s"]
                .into_iter()
                .map(OsString::from)
                .collect::<Vec<_>>();

            apply_posix_option_order(&mut args, true);

            assert_eq!(
                args,
                ["paste", "-d", ",", "--", "input", "-s"]
                    .into_iter()
                    .map(OsString::from)
                    .collect::<Vec<_>>()
            );
        }
    }

    mod integration_tests {
        use super::*;

        #[test]
        fn test_paste_serial_mode() -> CTResult<()> {
            let test_input = "1\n2\n3\n";
            let expected = "1,2,3\n";

            let (_temp_dir, file_path) = create_test_file(test_input);
            let flags = PasteFlags {
                is_serial: true,
                delimiters: ",".into(),
                files: vec![file_path.into()],
                line_ending: CtLineEnding::Newline,
            };

            let mut output = Vec::new();
            paste_exec(&mut output, flags)?;

            assert_eq!(String::from_utf8_lossy(&output), expected);
            Ok(())
        }

        #[test]
        fn test_paste_serial_cycles_delimiters() -> CTResult<()> {
            let (_temp_dir, file_path) = create_test_file("1\n2\n3\n4\n");
            let flags = PasteFlags {
                is_serial: true,
                delimiters: ":,".into(),
                files: vec![file_path.into()],
                line_ending: CtLineEnding::Newline,
            };

            let mut output = Vec::new();
            paste_exec(&mut output, flags)?;

            assert_eq!(output, b"1:2,3:4\n");
            Ok(())
        }

        #[test]
        fn test_paste_with_zero_terminator() -> CTResult<()> {
            let test_input = "a\0b\0c\0";
            let expected = "a,b,c\0";

            let (_temp_dir, file_path) = create_test_file(test_input);
            let flags = PasteFlags {
                is_serial: true,
                delimiters: ",".into(),
                files: vec![file_path.into()],
                line_ending: CtLineEnding::Nul,
            };

            let mut output = Vec::new();
            paste_exec(&mut output, flags)?;

            assert_eq!(String::from_utf8_lossy(&output), expected);
            Ok(())
        }

        #[test]
        fn test_paste_parallel_with_multiple_files() -> CTResult<()> {
            let test_input1 = "1\n2\n3\n";
            let test_input2 = "a\nb\nc\n";
            let test_input3 = "x\ny\nz\n";
            let expected = "1\ta\tx\n2\tb\ty\n3\tc\tz\n";

            let temp_dir = tempfile::tempdir()?;
            let file1_path = temp_dir.path().join("file1.txt");
            let file2_path = temp_dir.path().join("file2.txt");
            let file3_path = temp_dir.path().join("file3.txt");

            std::fs::write(&file1_path, test_input1)?;
            std::fs::write(&file2_path, test_input2)?;
            std::fs::write(&file3_path, test_input3)?;

            let flags = PasteFlags {
                is_serial: false,
                delimiters: "\t".into(),
                files: vec![
                    file1_path.into_os_string(),
                    file2_path.into_os_string(),
                    file3_path.into_os_string(),
                ],
                line_ending: CtLineEnding::Newline,
            };

            let mut output = Vec::new();
            paste_exec(&mut output, flags)?;

            assert_eq!(String::from_utf8_lossy(&output), expected);
            Ok(())
        }

        #[test]
        fn test_paste_with_escaped_delimiters() -> CTResult<()> {
            let test_input = "1\n2\n3\n";
            let expected = "1\n2\n3\n";

            let (_temp_dir, file_path) = create_test_file(test_input);
            let flags = PasteFlags {
                is_serial: true,
                delimiters: "\\n".into(),
                files: vec![file_path.into()],
                line_ending: CtLineEnding::Newline,
            };

            let mut output = Vec::new();
            paste_exec(&mut output, flags)?;

            assert_eq!(String::from_utf8_lossy(&output), expected);
            Ok(())
        }

        #[test]
        fn test_paste_with_double_backslash() -> CTResult<()> {
            let test_input = "1\n2\n3\n";
            let expected = "1\\2\\3\n";

            let (_temp_dir, file_path) = create_test_file(test_input);
            let flags = PasteFlags {
                is_serial: true,
                delimiters: "\\\\".into(),
                files: vec![file_path.into()],
                line_ending: CtLineEnding::Newline,
            };

            let mut output = Vec::new();
            paste_exec(&mut output, flags)?;

            assert_eq!(String::from_utf8_lossy(&output), expected);
            Ok(())
        }

        #[test]
        fn test_paste_with_empty_file() -> CTResult<()> {
            let (_temp_dir, file_path) = create_test_file("");
            let flags = PasteFlags {
                is_serial: false,
                delimiters: "\t".into(),
                files: vec![file_path.into()],
                line_ending: CtLineEnding::Newline,
            };

            let mut output = Vec::new();
            paste_exec(&mut output, flags)?;

            assert_eq!(String::from_utf8_lossy(&output), "");
            Ok(())
        }

        #[test]
        fn test_paste_with_nonexistent_file() {
            let flags = PasteFlags {
                is_serial: false,
                delimiters: "\t".into(),
                files: vec!["nonexistent.txt".into()],
                line_ending: CtLineEnding::Newline,
            };

            let mut output = Vec::new();
            assert!(paste_exec(&mut output, flags).is_err());
        }
    }

    #[test]
    fn test_paste_unescape() {
        assert_eq!(paste_unescape(b"\\b"), b"\x08");
        assert_eq!(paste_unescape(b"\\f"), b"\x0c");
        assert_eq!(paste_unescape(b"\\n"), b"\n");
        assert_eq!(paste_unescape(b"\\r"), b"\r");
        assert_eq!(paste_unescape(b"\\t"), b"\t");
        assert_eq!(paste_unescape(b"\\v"), b"\x0b");
        assert_eq!(paste_unescape(b"\\\\"), b"\\");
        assert_eq!(paste_unescape(b"\\\\n"), b"\\n");
        assert_eq!(paste_unescape(b"a\\\\nb"), b"a\\nb");
        assert_eq!(paste_unescape(b"\\q"), b"q");
        assert_eq!(paste_unescape(b"a\\qb"), b"aqb");
        assert_eq!(paste_unescape("\u{FFFF}".as_bytes()), "\u{FFFF}".as_bytes());
        assert_eq!(paste_unescape(&[0xfe, b',']), [0xfe, b',']);
    }

    #[test]
    fn test_paste_unescape_empty_delimiter() {
        assert_eq!(paste_unescape(b"\\0"), b"\0");
        assert_eq!(paste_unescape(b"\\0,"), b"\0,");
        assert_eq!(paste_unescape(b"\\\\0"), b"\\0");
    }

    // Helper function for creating test files
    fn create_test_file(content: &str) -> (tempfile::TempDir, String) {
        let temp_dir = Builder::new().prefix("paste_test").tempdir().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        std::fs::write(&file_path, content).unwrap();
        (temp_dir, file_path.to_str().unwrap().to_string())
    }

    #[test]
    fn test_tool_implementation() {
        let tool = Paste;

        // 测试 name 方法
        assert_eq!(tool.name(), "paste");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("paste"));

        // 测试 execute 方法 - 帮助命令应该返回错误，但不会崩溃
        let args = vec![OsString::from("paste"), OsString::from("--help")];
        assert!(tool.execute(&args).is_err());
    }
}
