/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
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

use clap::{Arg, ArgAction, Command, crate_version};
use ctcore::ct_error::{CTResult, CtSimpleError, FromIo};
use ctcore::ct_line_ending::CtLineEnding;
use ctcore::{ct_format_usage, ct_help_about, ct_help_usage};
use std::fs::File;
use std::io::{BufRead, BufReader, Write, stdin, stdout};
use std::path::Path;

const PASTE_ABOUT: &str = ct_help_about!("paste.md");
const PASTE_USAGE: &str = ct_help_usage!("paste.md");

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
    let application_info = PASTE_ABOUT;
    let usage_description = ct_format_usage(PASTE_USAGE);
    let args = vec![
        Arg::new(paste_flags::PASTE_SERIAL)
            .long(paste_flags::PASTE_SERIAL)
            .short('s')
            .help("paste one file at a time instead of in parallel")
            .action(ArgAction::SetTrue),
        Arg::new(paste_flags::PASTE_DELIMITER)
            .long(paste_flags::PASTE_DELIMITER)
            .short('d')
            .help("reuse characters from LIST instead of TABs")
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
            .help("line delimiter is NUL, not newline")
            .action(ArgAction::SetTrue),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(&args)
}
