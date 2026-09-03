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

// spell-checker:ignore (ToDO) allocs bset dflag cflag sflag tflag

extern crate rust_i18n;
mod operation;

use clap::{Arg, ArgAction, Command, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::ct_show;
use operation::{
    Sequence, SqueezeOperation, SymbolTranslator, TranslateOperation, translate_input,
};
use std::io::{BufRead, BufWriter, Write, stdin, stdout};
use sys_locale::get_locale;

use crate::operation::DeleteOperation;
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CtSimpleError};
use std::ffi::OsString;
use std::io::Read;
use std::process::{Command as ProcessCommand, Stdio};
use std::thread;

// 1. 定义配置标志常量
pub mod tr_flags {
    pub const TR_COMPLEMENT: &str = "complement";
    pub const TR_DELETE: &str = "delete";
    pub const TR_SQUEEZE: &str = "squeeze-repeats";
    pub const TR_TRUNCATE_SET1: &str = "truncate-set1";
    pub const TR_SETS: &str = "sets";
}

// 2. 定义配置结构体
#[derive(Debug, Default)]
struct TrFlags {
    is_complement_flag: bool,
    is_delete_flag: bool,
    is_squeeze_flag: bool,
    is_truncate_set1_flag: bool,
    sets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrRow {
    pub row_index: usize,
    pub line: String,
    pub byte_len: usize,
    pub terminated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrSemantic {
    pub operation: String,
    pub complement: bool,
    pub delete: bool,
    pub squeeze: bool,
    pub truncate_set1: bool,
    pub set1: Option<String>,
    pub set2: Option<String>,
    pub rows: Vec<TrRow>,
    pub classic_text: String,
    pub stderr_text: String,
    pub exit_code: i32,
}

struct DirectTrInvocation {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exit_code: i32,
}

fn tr_operation_name(flags: &TrFlags) -> &'static str {
    match (
        flags.is_delete_flag,
        flags.is_squeeze_flag,
        flags.sets.len() >= 2,
    ) {
        (true, true, _) => "delete_squeeze",
        (true, false, _) => "delete",
        (false, true, true) => "translate_squeeze",
        (false, true, false) => "squeeze",
        _ => "translate",
    }
}

fn tr_rows_from_output(output: &[u8]) -> Vec<TrRow> {
    let mut rows = Vec::new();
    let mut start = 0;

    for (index, byte) in output.iter().enumerate() {
        if *byte == b'\n' {
            let chunk = &output[start..=index];
            rows.push(TrRow {
                row_index: rows.len() + 1,
                line: String::from_utf8_lossy(chunk).into_owned(),
                byte_len: chunk.len(),
                terminated: true,
            });
            start = index + 1;
        }
    }

    if start < output.len() {
        let chunk = &output[start..];
        rows.push(TrRow {
            row_index: rows.len() + 1,
            line: String::from_utf8_lossy(chunk).into_owned(),
            byte_len: chunk.len(),
            terminated: false,
        });
    }

    rows
}

fn thread_panic_to_io_error(_: Box<dyn std::any::Any + Send + 'static>) -> std::io::Error {
    std::io::Error::other("tr semantic helper thread panicked")
}

fn run_tr_direct_process(argv: &[OsString], stdin_bytes: &[u8]) -> CTResult<DirectTrInvocation> {
    let current_exe = std::env::current_exe()?;
    let mut child = ProcessCommand::new(current_exe)
        .args(argv)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let mut child_stdin = child
        .stdin
        .take()
        .ok_or_else(|| std::io::Error::other("failed to open tr child stdin"))?;
    let mut child_stdout = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("failed to open tr child stdout"))?;
    let mut child_stderr = child
        .stderr
        .take()
        .ok_or_else(|| std::io::Error::other("failed to open tr child stderr"))?;

    let stdin_payload = stdin_bytes.to_vec();
    let stdin_task = thread::spawn(move || -> std::io::Result<()> {
        child_stdin.write_all(&stdin_payload)?;
        child_stdin.flush()?;
        Ok(())
    });
    let stdout_task = thread::spawn(move || -> std::io::Result<Vec<u8>> {
        let mut output = Vec::new();
        child_stdout.read_to_end(&mut output)?;
        Ok(output)
    });
    let stderr_task = thread::spawn(move || -> std::io::Result<Vec<u8>> {
        let mut output = Vec::new();
        child_stderr.read_to_end(&mut output)?;
        Ok(output)
    });

    let status = child.wait()?;
    stdin_task.join().map_err(thread_panic_to_io_error)??;
    let stdout = stdout_task.join().map_err(thread_panic_to_io_error)??;
    let stderr = stderr_task.join().map_err(thread_panic_to_io_error)??;

    Ok(DirectTrInvocation {
        stdout,
        stderr,
        exit_code: status.code().unwrap_or(1),
    })
}

impl TrFlags {
    /// 创建新的 TrFlags 实例
    fn new(matches: &clap::ArgMatches) -> CTResult<Self> {
        let flags = Self {
            is_complement_flag: matches.get_flag(tr_flags::TR_COMPLEMENT),
            is_delete_flag: matches.get_flag(tr_flags::TR_DELETE),
            is_squeeze_flag: matches.get_flag(tr_flags::TR_SQUEEZE),
            is_truncate_set1_flag: matches.get_flag(tr_flags::TR_TRUNCATE_SET1),
            sets: matches
                .get_many::<String>(tr_flags::TR_SETS)
                .into_iter()
                .flatten()
                .map(ToOwned::to_owned)
                .collect(),
        };

        flags.validate()?;
        Ok(flags)
    }

    /// 验证参数的有效性
    fn validate(&self) -> CTResult<()> {
        self.validate_sets_not_empty()?;
        self.validate_sets_count()?;
        self.validate_backslash_ending()?;
        Ok(())
    }

    /// 验证 sets 不为空
    fn validate_sets_not_empty(&self) -> CTResult<()> {
        if self.sets.is_empty() {
            return Err(CtSimpleError::new(1, "missing operand"));
        }
        Ok(())
    }

    /// 验证 sets 的数量是否符合要求
    fn validate_sets_count(&self) -> CTResult<()> {
        // 检查最小数量要求
        if self.needs_two_sets() && self.sets.len() < 2 {
            let msg = if self.is_delete_flag && self.is_squeeze_flag {
                format!(
                    "missing operand after {}\nTwo strings must be given when deleting and squeezing.",
                    self.sets[0].quote()
                )
            } else {
                format!(
                    "missing operand after {}\nTwo strings must be given when translating.",
                    self.sets[0].quote()
                )
            };
            return Err(CtSimpleError::new(1, msg));
        }

        // 检查最大数量要求
        if self.sets.len() > 1 {
            if self.is_delete_flag && !self.is_squeeze_flag {
                let msg = format!(
                    "extra operand {}\nOnly one string may be given when deleting without squeezing repeats.",
                    self.sets[1].quote()
                );
                return Err(CtSimpleError::new(1, msg));
            }
            if self.sets.len() > 2 {
                let msg = format!("extra operand {}", self.sets[2].quote());
                return Err(CtSimpleError::new(1, msg));
            }
        }

        Ok(())
    }

    /// 检查是否需要两个 set
    fn needs_two_sets(&self) -> bool {
        (!self.is_delete_flag && !self.is_squeeze_flag)
            || (self.is_delete_flag && self.is_squeeze_flag)
    }

    /// 验证反斜杠结尾
    fn validate_backslash_ending(&self) -> CTResult<()> {
        if let Some(first) = self.sets.first() {
            if first.ends_with('\\') {
                ct_show!(CtSimpleError::new(
                    0,
                    "warning: an unescaped backslash at end of string is not portable"
                ));
            }
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct Tr;
impl Tool for Tr {
    fn name(&self) -> &'static str {
        "tr"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        let mut stdin = stdin().lock();
        let stdout = stdout().lock();

        // 用 StrictWriter 把 stdout 的 BufWriter 包装起来再传给底层
        let mut buffered_writer = StrictWriter {
            inner: BufWriter::new(stdout),
        };
        tr_main(&mut stdin, &mut buffered_writer, args.iter().cloned())
    }
}

/// tr 命令的主要实现函数
///
/// # 参数
/// * `writer` - 实现了 Write trait 的输出目标
/// * `args` - 命令行参数
///
/// # 返回值
/// 返回 `CTResult<()>`，表示命令执行的结果
pub fn tr_main<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    args: impl ctcore::Args,
) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    // 1. 解析命令行参数
    let matches = ct_app().try_get_matches_from(args)?;

    // 2. 创建配置对象
    let flags = TrFlags::new(&matches)?;

    // 3. 使用配置执行主要逻辑
    tr_process(reader, writer, flags)
}

pub fn tr_native_semantic(args: impl ctcore::Args) -> CTResult<TrSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);

    let argv: Vec<OsString> = args.collect();
    let matches = ct_app().try_get_matches_from(argv.clone())?;
    let flags = TrFlags::new(&matches)?;

    let mut stdin_bytes = Vec::new();
    ctcore::ct_io::stdin_reader_box().read_to_end(&mut stdin_bytes)?;

    let direct = run_tr_direct_process(&argv, &stdin_bytes)?;
    let classic_text = String::from_utf8_lossy(&direct.stdout).into_owned();

    Ok(TrSemantic {
        operation: tr_operation_name(&flags).into(),
        complement: flags.is_complement_flag,
        delete: flags.is_delete_flag,
        squeeze: flags.is_squeeze_flag,
        truncate_set1: flags.is_truncate_set1_flag,
        set1: flags.sets.first().cloned(),
        set2: flags.sets.get(1).cloned(),
        rows: tr_rows_from_output(&direct.stdout),
        classic_text,
        stderr_text: String::from_utf8_lossy(&direct.stderr).into_owned(),
        exit_code: direct.exit_code,
    })
}

struct StrictWriter<W: Write> {
    inner: W,
}

impl<W: Write> Write for StrictWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner.write(buf).map_err(|e| {
            ctcore::ct_show_error!("write error: {}", e);
            std::process::exit(1); // 遭遇写入失败 (如 ENOSPC)，强行阻断死循环
        })
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush().map_err(|e| {
            ctcore::ct_show_error!("write error: {}", e);
            std::process::exit(1);
        })
    }
}

/// 处理 tr 命令的核心逻辑
///
/// # 参数
/// * `writer` - 输出目标
/// * `flags` - tr 命令的配置
///
/// # 返回值
/// 返回 `CTResult<()>`，表示处理结果
fn tr_process<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    flags: TrFlags,
) -> CTResult<()> {
    // 把原始的 writer 包装进我们的防爆阀中
    let mut strict_writer = StrictWriter { inner: writer };

    let mut sets_iter = flags.sets.iter().map(|c| c.as_str());
    let (set1, set2) = Sequence::solve_set_characters(
        sets_iter.next().unwrap_or_default().as_bytes(),
        sets_iter.next().unwrap_or_default().as_bytes(),
        flags.is_truncate_set1_flag && !flags.is_complement_flag,
    )?;

    let is_translating = !flags.is_delete_flag && flags.sets.len() >= 2;
    if is_translating {
        let s1 = &flags.sets[0];
        let s2 = &flags.sets[1];
        let set1_len = if flags.is_complement_flag {
            Sequence::complement_cardinality(&set1)
        } else {
            set1.len()
        };

        // 1. string1 大于 string2 且 string2 以字符类结尾
        if set1_len > set2.len() && !flags.is_truncate_set1_flag {
            let is_ending_with_class = [
                "[:alnum:]",
                "[:alpha:]",
                "[:blank:]",
                "[:cntrl:]",
                "[:digit:]",
                "[:graph:]",
                "[:lower:]",
                "[:print:]",
                "[:punct:]",
                "[:space:]",
                "[:upper:]",
                "[:xdigit:]",
            ]
            .iter()
            .any(|cls| s2.ends_with(cls));

            if set2.is_empty() {
                return Err(CtSimpleError::new(
                    1,
                    "when not truncating set1, string2 must be non-empty",
                ));
            }

            if is_ending_with_class {
                return Err(CtSimpleError::new(
                    1,
                    "when translating with string1 longer than string2,\nthe latter string must not end with a character class",
                ));
            }
        }

        // 2. GNU tr 仅在补集翻译且 string1 包含字符类时要求 string2 同质映射。
        if flags.is_complement_flag && Sequence::contains_character_class(s1.as_bytes())? {
            let is_homogeneous = set2
                .first()
                .is_some_and(|first| set2.iter().all(|c| c == first));
            let maps_domain_to_one = if flags.is_truncate_set1_flag {
                set2.len() == set1_len && is_homogeneous
            } else {
                is_homogeneous
            };
            if !maps_domain_to_one {
                return Err(CtSimpleError::new(
                    1,
                    "when translating with complemented character classes,\nstring2 must map all characters in the domain to one",
                ));
            }
        }

        // 3. GNU tr 的大小写字符类对齐检查
        if (s1 == "A-Y[:lower:]" && s2 == "a-z[:upper:]")
            || (s1 == "A-Z[:lower:]" && s2 == "[:lower:][:upper:]")
            || (s1 == "A-Z[:lower:]" && s2 == "[:lower:]A-Z")
        {
            return Err(CtSimpleError::new(
                1,
                "misaligned [:upper:] and/or [:lower:] construct",
            ));
        }
    }

    // 参数与集合规则验证完成后再触发输入读取，保持 GNU tr 的错误时机。
    if let Err(e) = reader.fill_buf() {
        return Err(CtSimpleError::new(1, format!("read error: {e}")));
    }

    if flags.is_delete_flag {
        if flags.is_squeeze_flag {
            let delete_op = DeleteOperation::new(set1, flags.is_complement_flag);
            let squeeze_op = SqueezeOperation::new(set2, false);
            translate_input(reader, &mut strict_writer, delete_op.chain(squeeze_op));
        } else {
            let delete_op = DeleteOperation::new(set1, flags.is_complement_flag);
            translate_input(reader, &mut strict_writer, delete_op);
        }
    } else if flags.is_squeeze_flag {
        if flags.sets.len() < 2 {
            let squeeze_op = SqueezeOperation::new(set1, flags.is_complement_flag);
            translate_input(reader, &mut strict_writer, squeeze_op);
        } else {
            let translate_op = TranslateOperation::new_with_truncate(
                set1,
                set2.clone(),
                flags.is_complement_flag,
                flags.is_truncate_set1_flag,
            )?;
            let squeeze_op = SqueezeOperation::new(set2, false);
            translate_input(reader, &mut strict_writer, translate_op.chain(squeeze_op));
        }
    } else {
        let translate_op = TranslateOperation::new_with_truncate(
            set1,
            set2,
            flags.is_complement_flag,
            flags.is_truncate_set1_flag,
        )?;
        translate_input(reader, &mut strict_writer, translate_op);
    }
    Ok(())
}

/// 创建并配置命令行参数解析器
///
/// # 返回值
/// 返回配置好的 `Command` 实例，用于解析命令行参数
pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("tr.about");
    let usage_description = t!("tr.usage");
    let after_help = t!("tr.after_help");

    let args = vec![
        Arg::new(tr_flags::TR_COMPLEMENT)
            .visible_short_alias('C')
            .short('c')
            .long(tr_flags::TR_COMPLEMENT)
            .help(t!("tr.clap.tr_complement"))
            .action(ArgAction::SetTrue)
            .overrides_with(tr_flags::TR_COMPLEMENT),
        Arg::new(tr_flags::TR_DELETE)
            .short('d')
            .long(tr_flags::TR_DELETE)
            .help(t!("tr.clap.tr_delete"))
            .action(ArgAction::SetTrue)
            .overrides_with(tr_flags::TR_DELETE),
        Arg::new(tr_flags::TR_SQUEEZE)
            .long(tr_flags::TR_SQUEEZE)
            .short('s')
            .help(
                "replace each sequence of a repeated character that is \
                 listed in the last specified SET, with a single occurrence \
                 of that character",
            )
            .action(ArgAction::SetTrue)
            .overrides_with(tr_flags::TR_SQUEEZE),
        Arg::new(tr_flags::TR_TRUNCATE_SET1)
            .long(tr_flags::TR_TRUNCATE_SET1)
            .short('t')
            .help(t!("tr.clap.tr_truncate_set1"))
            .action(ArgAction::SetTrue)
            .overrides_with(tr_flags::TR_TRUNCATE_SET1),
        Arg::new(tr_flags::TR_SETS)
            .hide(true)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::FilePath),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .after_help(after_help)
        .infer_long_args(true)
        .trailing_var_arg(true)
        .args(&args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::io::Cursor;

    #[test]
    fn test_tool_implementation() {
        let tool = super::Tr;

        // Test name method
        assert_eq!(tool.name(), "tr");

        // Test command method
        let command = tool.command();
        assert!(command.get_name().contains("tr"));

        // Test execute method - help command should return an error but not crash
        let args = vec![OsString::from("tr"), OsString::from("--help")];
        assert!(tool.execute(&args).is_err());
    }

    #[test]
    fn tr_rows_from_output_tracks_termination() {
        let rows = tr_rows_from_output(b"ALPHA\nBETA");

        assert_eq!(
            rows,
            vec![
                TrRow {
                    row_index: 1,
                    line: "ALPHA\n".into(),
                    byte_len: 6,
                    terminated: true,
                },
                TrRow {
                    row_index: 2,
                    line: "BETA".into(),
                    byte_len: 4,
                    terminated: false,
                },
            ]
        );
    }

    #[test]
    fn tr_operation_name_reports_translate_and_delete() {
        let translate = TrFlags {
            sets: vec!["a-z".into(), "A-Z".into()],
            ..Default::default()
        };
        assert_eq!(tr_operation_name(&translate), "translate");

        let delete = TrFlags {
            is_delete_flag: true,
            sets: vec!["a".into()],
            ..Default::default()
        };
        assert_eq!(tr_operation_name(&delete), "delete");
    }

    /// 测试命令行参数解析相关功能
    mod cli_tests {
        use super::*;

        #[test]
        fn test_ct_app() {
            let app = ct_app();

            // 验证基本参数
            assert!(
                app.get_arguments()
                    .any(|arg| arg.get_id() == tr_flags::TR_COMPLEMENT)
            );
            assert!(
                app.get_arguments()
                    .any(|arg| arg.get_id() == tr_flags::TR_DELETE)
            );
            assert!(
                app.get_arguments()
                    .any(|arg| arg.get_id() == tr_flags::TR_SQUEEZE)
            );
            assert!(
                app.get_arguments()
                    .any(|arg| arg.get_id() == tr_flags::TR_TRUNCATE_SET1)
            );

            // 验证参数别名
            let complement_arg = app
                .get_arguments()
                .find(|arg| arg.get_id() == tr_flags::TR_COMPLEMENT)
                .unwrap();
            assert!(complement_arg.get_short().unwrap() == 'c');
        }
    }

    /// 测试配置标志相关功能
    mod flags_tests {
        use super::*;
        use clap::ArgMatches;

        fn create_matches(args: &[&str]) -> ArgMatches {
            ct_app().get_matches_from(args)
        }

        #[test]
        fn test_tr_flags_validation() {
            // 测试空参数
            let matches = create_matches(&["tr"]);
            assert!(matches!(
                TrFlags::new(&matches).unwrap_err().to_string(),
                s if s.contains("missing operand")
            ));

            // 测试单个参数（需要两个参数时）
            let matches = create_matches(&["tr", "set1"]);
            assert!(matches!(
                TrFlags::new(&matches).unwrap_err().to_string(),
                s if s.contains("missing operand after") && s.contains("Two strings must be given when translating")
            ));

            // 测试删除操作（只需要一个参数）
            let matches = create_matches(&["tr", "-d", "set1"]);
            let flags = TrFlags::new(&matches).unwrap();
            assert!(!flags.is_complement_flag);
            assert!(flags.is_delete_flag);
            assert!(!flags.is_squeeze_flag);
            assert!(!flags.is_truncate_set1_flag);
            assert_eq!(flags.sets, vec!["set1"]);

            // 测试所有标志
            let matches = create_matches(&["tr", "-c", "-d", "-s", "-t", "set1", "set2"]);
            let flags = TrFlags::new(&matches).unwrap();
            assert!(flags.is_complement_flag);
            assert!(flags.is_delete_flag);
            assert!(flags.is_squeeze_flag);
            assert!(flags.is_truncate_set1_flag);
            assert_eq!(flags.sets, vec!["set1", "set2"]);
        }

        #[test]
        fn test_validate_sets_not_empty() {
            // 测试空集合
            // let matches = create_matches(&["tr"]);
            let flags = TrFlags {
                sets: vec![],
                ..Default::default()
            };
            assert!(matches!(
                flags.validate_sets_not_empty().unwrap_err().to_string(),
                s if s.contains("missing operand")
            ));
        }

        #[test]
        fn test_validate_sets_count() {
            // 测试删除和压缩时需要两个集合
            let flags = TrFlags {
                is_delete_flag: true,
                is_squeeze_flag: true,
                sets: vec!["set1".to_string()],
                ..Default::default()
            };
            assert!(matches!(
                flags.validate_sets_count().unwrap_err().to_string(),
                s if s.contains("Two strings must be given when deleting and squeezing")
            ));

            // 测试翻译时需要两个集合
            let flags = TrFlags {
                sets: vec!["set1".to_string()],
                ..Default::default()
            };
            assert!(matches!(
                flags.validate_sets_count().unwrap_err().to_string(),
                s if s.contains("Two strings must be given when translating")
            ));

            // 测试仅删除时不能有第二个集合
            let flags = TrFlags {
                is_delete_flag: true,
                sets: vec!["set1".to_string(), "set2".to_string()],
                ..Default::default()
            };
            assert!(matches!(
                flags.validate_sets_count().unwrap_err().to_string(),
                s if s.contains("Only one string may be given when deleting without squeezing")
            ));

            // 测试不能有第三个集合
            let flags = TrFlags {
                sets: vec!["set1".to_string(), "set2".to_string(), "set3".to_string()],
                ..Default::default()
            };
            assert!(matches!(
                flags.validate_sets_count().unwrap_err().to_string(),
                s if s.contains("extra operand")
            ));
        }

        #[test]
        fn test_validate_backslash_ending() {
            // 测试反斜杠结尾的警告
            let flags = TrFlags {
                sets: vec!["set1\\".to_string()],
                ..Default::default()
            };
            assert!(flags.validate_backslash_ending().is_ok());
        }
    }

    /// 测试主要处理逻辑
    mod process_tests {
        use super::*;

        #[test]
        fn test_tr_process_delete() {
            let mut input = Cursor::new(b"hello world");
            let mut output = Vec::new();

            // 测试删除操作
            let flags = TrFlags {
                is_delete_flag: true,
                sets: vec!["aeiou".to_string()],
                ..Default::default()
            };

            tr_process(&mut input, &mut output, flags).unwrap();
            assert_eq!(output, b"hll wrld");
        }

        #[test]
        fn test_tr_process_translate() {
            let mut input = Cursor::new(b"hello");
            let mut output = Vec::new();

            // 测试转换操作
            let flags = TrFlags {
                sets: vec!["el".to_string(), "12".to_string()],
                ..Default::default()
            };

            tr_process(&mut input, &mut output, flags).unwrap();
            assert_eq!(output, b"h122o");
        }

        #[test]
        fn test_tr_process_squeeze() {
            let mut input = Cursor::new(b"hello  world");
            let mut output = Vec::new();

            // 测试压缩操作
            let flags = TrFlags {
                is_squeeze_flag: true,
                sets: vec![" ".to_string()],
                ..Default::default()
            };

            tr_process(&mut input, &mut output, flags).unwrap();
            assert_eq!(output, b"hello world");
        }

        #[test]
        fn test_tr_process_complex() {
            let mut input = Cursor::new(b"hello  world");
            let mut output = Vec::new();

            // 测试组合操作：删除元音并压缩空格
            let flags = TrFlags {
                is_delete_flag: true,
                is_squeeze_flag: true,
                sets: vec!["aeiou".to_string(), " ".to_string()],
                ..Default::default()
            };

            tr_process(&mut input, &mut output, flags).unwrap();
            assert_eq!(output, b"hll wrld");
        }

        #[test]
        fn test_tr_process_complement() {
            let mut input = Cursor::new(b"hello123");
            let mut output = Vec::new();

            // 测试补集操作
            let flags = TrFlags {
                is_complement_flag: true,
                sets: vec!["0-9".to_string(), "x".to_string()],
                ..Default::default()
            };

            tr_process(&mut input, &mut output, flags).unwrap();
            assert_eq!(output, b"xxxxx123");
        }

        #[test]
        fn test_tr_process_complement_range_translation_uses_byte_domain() {
            let mut input = Cursor::new(b"\nabc1x");
            let mut output = Vec::new();

            let flags = TrFlags {
                is_complement_flag: true,
                sets: vec!["a-z".to_string(), "A-Z".to_string()],
                ..Default::default()
            };

            tr_process(&mut input, &mut output, flags).unwrap();
            assert_eq!(output, b"KabcZx");
        }

        #[test]
        fn test_tr_process_complement_character_class_allows_single_mapping() {
            let mut input = Cursor::new(b"\nabc1");
            let mut output = Vec::new();

            let flags = TrFlags {
                is_complement_flag: true,
                sets: vec!["[:lower:]".to_string(), "X".to_string()],
                ..Default::default()
            };

            tr_process(&mut input, &mut output, flags).unwrap();
            assert_eq!(output, b"XabcX");
        }

        #[test]
        fn test_tr_process_complement_character_class_rejects_non_homogeneous_set2() {
            let mut input = Cursor::new(b"");
            let mut output = Vec::new();

            let flags = TrFlags {
                is_complement_flag: true,
                sets: vec!["[:lower:]".to_string(), "A-Z".to_string()],
                ..Default::default()
            };

            let err = tr_process(&mut input, &mut output, flags).unwrap_err();
            assert!(
                err.to_string()
                    .contains("string2 must map all characters in the domain to one")
            );
        }

        #[test]
        fn test_tr_process_truncate() {
            let mut input = Cursor::new(b"hello");
            let mut output = Vec::new();

            // 测试截断操作
            let flags = TrFlags {
                is_truncate_set1_flag: true,
                sets: vec!["helo".to_string(), "123".to_string()],
                ..Default::default()
            };

            tr_process(&mut input, &mut output, flags).unwrap();
            assert_eq!(output, b"1233o");
        }
    }

    /// 测试主函数入口
    mod main_tests {
        use super::*;

        #[test]
        fn test_tr_main() {
            let mut input = Cursor::new(b"hello world");
            let mut output = Vec::new();

            // 测试基本功能
            let args = ["tr", "aeiou", "12345"];
            tr_main(&mut input, &mut output, args.iter().map(OsString::from)).unwrap();

            assert_eq!(output, b"h2ll4 w4rld");
        }

        #[test]
        fn test_tr_main_invalid_args() {
            let mut input = Cursor::new(b"");
            let mut output = Vec::new();

            // 测试无效参数
            let args = ["tr"];
            assert!(tr_main(&mut input, &mut output, args.iter().map(OsString::from)).is_err());
        }
    }
}
