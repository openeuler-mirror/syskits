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
rust_i18n::i18n!("locales", fallback = "zh-CN");
use ctcore::Tool;
use ctcore::ct_error::{CTResult, CtSimpleError, FromIo};
use ctcore::ct_line_ending::CtLineEnding;
use std::ffi::OsString;
use std::fs::File;
use std::io::{BufRead, BufReader, Write, stdin, stdout};
use std::path::Path;
use sys_locale::get_locale;

mod paste_flags {
    pub const PASTE_DELIMITER: &str = "delimiters";
    pub const PASTE_SERIAL: &str = "serial";
    pub const PASTE_FILE: &str = "file";
    pub const PASTE_ZERO_TERMINATED: &str = "zero-terminated";
}

/// 表示一个输入源，可以是文件或标准输入
type InputSource = Option<BufReader<File>>;

/// 存储粘贴操作的配置参数
#[derive(Debug)]
struct PasteFlags {
    is_serial: bool,
    delimiters: String,
    files: Vec<String>,
    line_ending: CtLineEnding,
}

/// 表示粘贴操作的上下文
struct PasteContext<W: Write> {
    files: Vec<InputSource>,
    delimiters: Vec<char>,
    line_ending: CtLineEnding,
    output: Vec<u8>,
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
                .get_one::<String>(paste_flags::PASTE_DELIMITER)
                .unwrap()
                .clone(),
            files: matches
                .get_many::<String>(paste_flags::PASTE_FILE)
                .unwrap()
                .cloned()
                .collect(),
            line_ending: CtLineEnding::from_zero_flag(
                matches.get_flag(paste_flags::PASTE_ZERO_TERMINATED),
            ),
        })
    }

    fn validate_delimiters(&self) -> CTResult<()> {
        if self.delimiters.ends_with('\\') && !self.delimiters.ends_with("\\\\") {
            return Err(CtSimpleError::new(
                1,
                format!(
                    "delimiter list ends with an unescaped backslash: {}",
                    self.delimiters
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
            let file = if name == "-" {
                None
            } else {
                let path = Path::new(name);
                let r = File::open(path).map_err_context(String::new)?;
                Some(BufReader::new(r))
            };
            files.push(file);
        }

        Ok(Self {
            files,
            delimiters: paste_unescape(&flags.delimiters).chars().collect(),
            line_ending: flags.line_ending,
            output: Vec::new(),
            writer,
        })
    }

    fn process_line(&mut self, file: &mut InputSource) -> CTResult<bool> {
        match file {
            Some(reader) => match reader.read_until(self.line_ending as u8, &mut self.output) {
                Ok(0) => Ok(false),
                Ok(_) => {
                    if self.output.ends_with(&[self.line_ending as u8]) {
                        self.output.pop();
                    }
                    Ok(true)
                }
                Err(e) => Err(e.map_err_context(String::new)),
            },
            None => {
                match stdin()
                    .lock()
                    .read_until(self.line_ending as u8, &mut self.output)
                {
                    Ok(0) => Ok(false),
                    Ok(_) => {
                        if self.output.ends_with(&[self.line_ending as u8]) {
                            self.output.pop();
                        }
                        Ok(true)
                    }
                    Err(e) => Err(e.map_err_context(String::new)),
                }
            }
        }
    }

    fn write_output(&mut self, delim_length: usize) -> CTResult<()> {
        if !self.output.is_empty() {
            self.output.truncate(self.output.len() - delim_length);
            write!(
                self.writer,
                "{}{}",
                String::from_utf8_lossy(&self.output),
                self.line_ending
            )?;
        }
        Ok(())
    }

    fn add_delimiter(&mut self, delim_count: usize) -> usize {
        let delimiter = self.delimiters[delim_count % self.delimiters.len()];
        let mut buf = [0; 4];
        let ch = delimiter.encode_utf8(&mut buf);
        let delim_length = ch.len();
        self.output.extend_from_slice(&buf[..delim_length]);
        delim_length
    }

    fn paste_serial(&mut self) -> CTResult<()> {
        let files = std::mem::take(&mut self.files);
        for mut file in files {
            self.output.clear();
            let mut delim_length = 1;
            while self.process_line(&mut file)? {
                delim_length = self.add_delimiter(0);
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

                let mut current_file = self.files[i].take();
                if !self.process_line(&mut current_file)? {
                    *eof_item = true;
                    eof_count += 1;
                } else {
                    has_content = true;
                }
                self.files[i] = current_file;
            }

            if self.files.len() == eof_count {
                break;
            }

            if has_content {
                write!(
                    self.writer,
                    "{}{}",
                    String::from_utf8_lossy(&self.output),
                    self.line_ending
                )?;
            }
            delim_count = 0;
        }
        Ok(())
    }
}

fn paste_unescape(s: &str) -> String {
    s.replace("\\\\", "\u{FFFF}")
        .replace("\\n", "\n")
        .replace("\\t", "\t")
        .replace("\u{FFFF}", "\\")
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    let mut stdout = stdout().lock();
    paste_main(&mut stdout, args)
}

pub fn paste_main<W: Write>(writer: &mut W, args: impl ctcore::Args) -> CTResult<()> {
    // 设置语言
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let matches = ct_app().try_get_matches_from(args)?;
    let flags = PasteFlags::new(&matches)?;
    flags.validate_delimiters()?;

    paste_exec(writer, flags)
}

fn paste_exec<W: Write>(writer: &mut W, flags: PasteFlags) -> CTResult<()> {
    let mut context = PasteContext::new(&flags, writer)?;
    if flags.is_serial {
        context.paste_serial()
    } else {
        context.paste_parallel()
    }
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
            .hide_default_value(true),
        Arg::new(paste_flags::PASTE_FILE)
            .value_name("FILE")
            .action(ArgAction::Append)
            .default_value("-")
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
            assert_eq!(flags.delimiters, ",");
            assert_eq!(flags.files, vec!["file1.txt", "file2.txt"]);
            assert_eq!(flags.line_ending, CtLineEnding::Newline);
        }

        #[test]
        fn test_validate_delimiters_valid() {
            let flags = PasteFlags {
                is_serial: false,
                delimiters: ",:".to_string(),
                files: vec!["file.txt".to_string()],
                line_ending: CtLineEnding::Newline,
            };
            assert!(flags.validate_delimiters().is_ok());
        }

        #[test]
        fn test_validate_delimiters_invalid() {
            let flags = PasteFlags {
                is_serial: false,
                delimiters: "\\".to_string(),
                files: vec!["file.txt".to_string()],
                line_ending: CtLineEnding::Newline,
            };
            assert!(flags.validate_delimiters().is_err());
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
            assert_eq!(flags.delimiters, ":|");
        }

        #[test]
        fn test_validate_delimiters_with_escaped_backslash() {
            let flags = PasteFlags {
                is_serial: false,
                delimiters: "\\\\".to_string(),
                files: vec!["file.txt".to_string()],
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
                delimiters: "\t".to_string(),
                files: vec![file_path],
                line_ending: CtLineEnding::Newline,
            };

            let mut output = Vec::new();
            let mut context = PasteContext::new(&flags, &mut output)?;
            let mut file = context.files.pop().unwrap();

            assert!(context.process_line(&mut file)?);
            assert_eq!(String::from_utf8_lossy(&context.output), "test");

            assert!(context.process_line(&mut file)?);
            assert_eq!(String::from_utf8_lossy(&context.output), "testline");

            assert!(!context.process_line(&mut file)?);
            Ok(())
        }

        #[test]
        fn test_add_delimiter() {
            let flags = PasteFlags {
                is_serial: false,
                delimiters: ",".to_string(),
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
                delimiters: ",".to_string(),
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
                delimiters: "\t".to_string(),
                files: vec!["-".to_string()],
                line_ending: CtLineEnding::Newline,
            };

            let mut output = Vec::new();
            let mut context = PasteContext::new(&flags, &mut output)?;
            let mut _file = context.files.pop().unwrap();

            // 注意：这个测试需要模拟标准输入，可能需要特殊处理
            Ok(())
        }

        #[test]
        fn test_process_line_with_zero_terminator() -> CTResult<()> {
            let (_temp_dir, file_path) = create_test_file("test\0line\0");
            let flags = PasteFlags {
                is_serial: false,
                delimiters: "\t".to_string(),
                files: vec![file_path],
                line_ending: CtLineEnding::Nul,
            };

            let mut output = Vec::new();
            let mut context = PasteContext::new(&flags, &mut output)?;
            let mut file = context.files.pop().unwrap();

            assert!(context.process_line(&mut file)?);
            assert_eq!(String::from_utf8_lossy(&context.output), "test");
            Ok(())
        }

        #[test]
        fn test_add_delimiter_with_multiple_chars() {
            let flags = PasteFlags {
                is_serial: false,
                delimiters: ":|".to_string(),
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
        fn test_write_output_empty() -> CTResult<()> {
            let flags = PasteFlags {
                is_serial: false,
                delimiters: ",".to_string(),
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
            let args = vec![ctcore::ct_util_name(), &file_path];

            let mut output = Vec::new();
            paste_main(&mut output, args.iter().map(|s| OsString::from(s)))?;

            assert_eq!(String::from_utf8_lossy(&output), "test\n");
            Ok(())
        }

        #[test]
        fn test_paste_main_invalid_args() {
            let args = vec![ctcore::ct_util_name(), "--invalid-flag"];
            let mut output = Vec::new();
            assert!(paste_main(&mut output, args.iter().map(|s| OsString::from(s))).is_err());
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
                    .get_one::<String>(paste_flags::PASTE_DELIMITER)
                    .unwrap(),
                ","
            );
        }

        #[test]
        fn test_ct_app_combined_options() {
            let args = vec![ctcore::ct_util_name(), "-sd", ",", "file.txt"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            assert!(matches.get_flag(paste_flags::PASTE_SERIAL));
            assert_eq!(
                matches
                    .get_one::<String>(paste_flags::PASTE_DELIMITER)
                    .unwrap(),
                ","
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
                .get_many::<String>(paste_flags::PASTE_FILE)
                .unwrap()
                .collect();
            assert_eq!(files.len(), 3);
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
                delimiters: ",".to_string(),
                files: vec![file_path],
                line_ending: CtLineEnding::Newline,
            };

            let mut output = Vec::new();
            paste_exec(&mut output, flags)?;

            assert_eq!(String::from_utf8_lossy(&output), expected);
            Ok(())
        }

        #[test]
        fn test_paste_with_zero_terminator() -> CTResult<()> {
            let test_input = "a\0b\0c\0";
            let expected = "a,b,c\0";

            let (_temp_dir, file_path) = create_test_file(test_input);
            let flags = PasteFlags {
                is_serial: true,
                delimiters: ",".to_string(),
                files: vec![file_path],
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
                delimiters: "\t".to_string(),
                files: vec![
                    file1_path.to_str().unwrap().to_string(),
                    file2_path.to_str().unwrap().to_string(),
                    file3_path.to_str().unwrap().to_string(),
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
                delimiters: "\\n".to_string(),
                files: vec![file_path],
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
                delimiters: "\\\\".to_string(),
                files: vec![file_path],
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
                delimiters: "\t".to_string(),
                files: vec![file_path],
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
                delimiters: "\t".to_string(),
                files: vec!["nonexistent.txt".to_string()],
                line_ending: CtLineEnding::Newline,
            };

            let mut output = Vec::new();
            assert!(paste_exec(&mut output, flags).is_err());
        }
    }

    #[test]
    fn test_paste_unescape() {
        assert_eq!(paste_unescape("\\n"), "\n");
        assert_eq!(paste_unescape("\\t"), "\t");
        assert_eq!(paste_unescape("\\\\"), "\\");
        assert_eq!(paste_unescape("\\\\n"), "\\n");
        assert_eq!(paste_unescape("a\\\\nb"), "a\\nb");
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
        let tool = Paste::default();

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
