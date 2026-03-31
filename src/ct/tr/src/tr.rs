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

// spell-checker:ignore (ToDO) allocs bset dflag cflag sflag tflag

mod operation;

use clap::{Arg, ArgAction, Command, crate_version};
use ctcore::{ct_format_usage, ct_help_about, ct_help_section, ct_help_usage, ct_show};
use operation::{
    Sequence, SqueezeOperation, SymbolTranslator, TranslateOperation, translate_input,
};
use std::io::{BufRead, BufWriter, Write, stdin, stdout};

use crate::operation::DeleteOperation;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CtSimpleError};

const TR_ABOUT: &str = ct_help_about!("tr.md");
const TR_USAGE: &str = ct_help_usage!("tr.md");
const TR_AFTER_HELP: &str = ct_help_section!("after help", "tr.md");

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

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    let stdin = stdin();
    let mut locked_stdin = stdin.lock();

    let stdout = stdout();
    let out = stdout.lock();
    let mut buffered_writer = BufWriter::new(out);

    tr_main(&mut locked_stdin, &mut buffered_writer, args)
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
    // 1. 解析命令行参数
    let matches = ct_app().try_get_matches_from(args)?;

    // 2. 创建配置对象
    let flags = TrFlags::new(&matches)?;

    // 3. 使用配置执行主要逻辑
    tr_process(reader, writer, flags)
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
    let mut sets_iter = flags.sets.iter().map(|c| c.as_str());
    let (set1, set2) = Sequence::solve_set_characters(
        sets_iter.next().unwrap_or_default().as_bytes(),
        sets_iter.next().unwrap_or_default().as_bytes(),
        flags.is_truncate_set1_flag,
    )?;

    // '*_op' are the operations that need to be applied, in order.
    if flags.is_delete_flag {
        if flags.is_squeeze_flag {
            let delete_op = DeleteOperation::new(set1, flags.is_complement_flag);
            let squeeze_op = SqueezeOperation::new(set2, false);
            translate_input(reader, writer, delete_op.chain(squeeze_op));
        } else {
            let delete_op = DeleteOperation::new(set1, flags.is_complement_flag);
            translate_input(reader, writer, delete_op);
        }
    } else if flags.is_squeeze_flag {
        if flags.sets.len() < 2 {
            let squeeze_op = SqueezeOperation::new(set1, flags.is_complement_flag);
            translate_input(reader, writer, squeeze_op);
        } else {
            let translate_op =
                TranslateOperation::new(set1, set2.clone(), flags.is_complement_flag)?;
            let squeeze_op = SqueezeOperation::new(set2, false);
            translate_input(reader, writer, translate_op.chain(squeeze_op));
        }
    } else {
        let translate_op = TranslateOperation::new(set1, set2, flags.is_complement_flag)?;
        translate_input(reader, writer, translate_op);
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
    let application_info = TR_ABOUT;
    let usage_description = ct_format_usage(TR_USAGE);
    let after_help = TR_AFTER_HELP;

    let args = vec![
        Arg::new(tr_flags::TR_COMPLEMENT)
            .visible_short_alias('C')
            .short('c')
            .long(tr_flags::TR_COMPLEMENT)
            .help("use the complement of SET1")
            .action(ArgAction::SetTrue)
            .overrides_with(tr_flags::TR_COMPLEMENT),
        Arg::new(tr_flags::TR_DELETE)
            .short('d')
            .long(tr_flags::TR_DELETE)
            .help("delete characters in SET1, do not translate")
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
            .help("first truncate SET1 to length of SET2")
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
            let args = vec!["tr", "aeiou", "12345"];
            tr_main(
                &mut input,
                &mut output,
                args.iter().map(|s| OsString::from(s)),
            )
            .unwrap();

            assert_eq!(output, b"h2ll4 w4rld");
        }

        #[test]
        fn test_tr_main_invalid_args() {
            let mut input = Cursor::new(b"");
            let mut output = Vec::new();

            // 测试无效参数
            let args = vec!["tr"];
            assert!(
                tr_main(
                    &mut input,
                    &mut output,
                    args.iter().map(|s| OsString::from(s))
                )
                .is_err()
            );
        }
    }
}
