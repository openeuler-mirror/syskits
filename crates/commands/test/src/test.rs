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

pub(crate) mod error;
mod parser;

use clap::Command;
use clap::crate_version;
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CtSimpleError};
#[cfg(not(windows))]
use ctcore::ct_process::{getegid, geteuid};
use error::{ParseError, ParseResult};
use parser::{TestOperator, TestSymbol, TestUnaryOperator, test_parse};
use rust_i18n::t;
use std::cmp::Ordering;
use std::ffi::{OsStr, OsString};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
rust_i18n::i18n!("locales", fallback = "en-US");
use sys_locale::get_locale;

#[derive(Default)]
pub struct Test;
impl Tool for Test {
    fn name(&self) -> &'static str {
        "test"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        test_main(args.iter().cloned())
    }
}

#[derive(Default)]
pub struct LeftBracket;
impl Tool for LeftBracket {
    fn name(&self) -> &'static str {
        "["
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        test_main(args.iter().cloned())
    }
}

/// 创建和配置 clap 命令应用
pub fn ct_app() -> Command {
    // Disable printing of -h and -v as valid alternatives for --help and --version,
    // since we don't recognize -h and -v as help/version flags.
    // 中文注释: 禁用打印 -h 和 -v 作为 --help 和 --version 的有效替代方式，
    // 因为我们不将 -h 和 -v 识别为帮助/版本标志。
    Command::new(ctcore::ct_util_name())
        .version(crate_version!())
        .about(t!("test.about"))
        .override_usage(t!("test.usage"))
        .after_help(t!("test.after_help"))
}

/// 处理 help 和 version 标志
fn handle_help_version(program: &OsString, args: &[OsString]) -> Option<CTResult<()>> {
    let prog_name = program.to_string_lossy();

    // 如果是 test 命令，严格遵循 POSIX/GNU 规范，不拦截 --help 和 --version，
    // 把它们留给后端作为普通非空字符串处理（会静默返回 true）
    if prog_name.ends_with("test") {
        return None;
    }

    // 如果是 `[` 命令，则允许输出帮助信息
    if !args.is_empty() && (args[0] == "--help" || args[0] == "--version") {
        // 允许匹配 `[ --help` 或者正规的 `[ --help ]`
        if args.len() == 1 || (args.len() == 2 && args[1] == "]") {
            // 交给 clap 触发默认的帮助和版本打印
            ct_app().get_matches_from(vec![program.clone(), args[0].clone()]);
            return Some(Ok(()));
        }
    }
    None
}

/// 处理 [ 命令的结束括号
fn handle_closing_bracket(binary_name: &str, args: &mut Vec<OsString>) -> CTResult<()> {
    if binary_name.ends_with('[') {
        let last = args.pop();
        if last.as_deref() != Some(OsStr::new("]")) {
            return Err(CtSimpleError::new(2, "missing ']'"));
        }
    }
    Ok(())
}

/// test 命令的主要入口函数
pub fn test_main(mut args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    // Get program name and collect arguments
    let program = args.next().unwrap_or_else(|| OsString::from("test"));
    let mut args: Vec<_> = args.collect();

    // Handle help and version flags
    if let Some(result) = handle_help_version(&program, &args) {
        return result;
    }

    // Handle closing bracket for [ command
    handle_closing_bracket(ctcore::ct_util_name(), &mut args)?;

    // Parse and evaluate the expression
    let result = test_parse(args).map(|mut stack| test_eval(&mut stack))??;

    if result { Ok(()) } else { Err(1.into()) }
}

/// 评估符号栈并返回布尔结果
fn test_eval(stack: &mut Vec<TestSymbol>) -> ParseResult<bool> {
    macro_rules! pop_literal {
        () => {
            match stack.pop() {
                Some(TestSymbol::Literal(s)) => s,
                Some(TestSymbol::LengthOperand(s)) => s,
                _ => panic!(),
            }
        };
    }

    let s = stack.pop();

    match s {
        Some(TestSymbol::Bang) => {
            let result = test_eval(stack)?;

            Ok(!result)
        }
        Some(TestSymbol::Op(TestOperator::String(op))) => {
            let b = pop_literal!();
            let a = pop_literal!();
            match op.to_string_lossy().as_ref() {
                "!=" => Ok(a != b),
                "<" => Ok(a < b),
                ">" => Ok(a > b),
                _ => Ok(a == b),
            }
        }
        Some(TestSymbol::Op(TestOperator::Int(op))) => {
            let b = pop_integer_operand(stack)?;
            let a = pop_integer_operand(stack)?;

            Ok(test_integers(&a, &b, &op)?)
        }
        Some(TestSymbol::Op(TestOperator::File(op))) => {
            let b = pop_file_operand(stack, &op);
            let a = pop_file_operand(stack, &op);
            Ok(files(&a?, &b?, &op)?)
        }
        Some(TestSymbol::UnaryOp(TestUnaryOperator::StrlenOp(op))) => {
            let s = match stack.pop() {
                Some(TestSymbol::Literal(s)) => s,
                Some(TestSymbol::None) => OsString::from(""),
                None => {
                    return Ok(true);
                }
                _ => {
                    return Err(ParseError::MissingArgument(op.quote().to_string()));
                }
            };

            Ok(if op == "-z" {
                s.is_empty()
            } else {
                !s.is_empty()
            })
        }
        Some(TestSymbol::UnaryOp(TestUnaryOperator::FiletestOp(op))) => {
            let op = op.to_str().unwrap();

            let f = pop_literal!();

            Ok(match op {
                "-b" => path(&f, &TestPathCondition::BlockSpecial),
                "-c" => path(&f, &TestPathCondition::CharacterSpecial),
                "-d" => path(&f, &TestPathCondition::Directory),
                "-e" => path(&f, &TestPathCondition::Exists),
                "-f" => path(&f, &TestPathCondition::Regular),
                "-g" => path(&f, &TestPathCondition::GroupIdFlag),
                "-G" => path(&f, &TestPathCondition::GroupOwns),
                "-h" => path(&f, &TestPathCondition::SymLink),
                "-k" => path(&f, &TestPathCondition::Sticky),
                "-L" => path(&f, &TestPathCondition::SymLink),
                "-N" => path(&f, &TestPathCondition::ExistsModifiedLastRead),
                "-O" => path(&f, &TestPathCondition::UserOwns),
                "-p" => path(&f, &TestPathCondition::Fifo),
                "-r" => path(&f, &TestPathCondition::Readable),
                "-S" => path(&f, &TestPathCondition::Socket),
                "-s" => path(&f, &TestPathCondition::NonEmpty),
                "-t" => isatty(&f)?,
                "-u" => path(&f, &TestPathCondition::UserIdFlag),
                "-w" => path(&f, &TestPathCondition::Writable),
                "-x" => path(&f, &TestPathCondition::Executable),
                _ => panic!(),
            })
        }
        Some(TestSymbol::Literal(s)) => Ok(!s.is_empty()),
        Some(TestSymbol::LengthOperand(s)) => Ok(!s.is_empty()),
        Some(TestSymbol::None) | None => Ok(false),
        Some(TestSymbol::BoolOp(op)) => {
            if (op == "-a" || op == "-o") && stack.len() < 2 {
                return Err(ParseError::UnaryOperatorExpected(op.quote().to_string()));
            }

            let b = test_eval(stack)?;
            let a = test_eval(stack)?;

            Ok(if op == "-a" { a && b } else { a || b })
        }
        _ => Err(ParseError::ExpectedValue),
    }
}

fn pop_integer_operand(stack: &mut Vec<TestSymbol>) -> ParseResult<OsString> {
    match stack.pop() {
        Some(TestSymbol::LengthOperand(s)) => Ok(OsString::from(s.len().to_string())),
        Some(TestSymbol::Literal(s)) => Ok(s),
        _ => panic!(),
    }
}

fn pop_file_operand(stack: &mut Vec<TestSymbol>, op: &OsStr) -> ParseResult<OsString> {
    match stack.pop() {
        Some(TestSymbol::LengthOperand(_)) => Err(ParseError::OperatorDoesNotAcceptLength(
            op.to_string_lossy().into_owned(),
        )),
        Some(TestSymbol::Literal(s)) => Ok(s),
        _ => panic!(),
    }
}

fn integer_is_blank(byte: u8) -> bool {
    unsafe { libc::isblank(libc::c_int::from(byte)) != 0 }
}

struct ParsedInteger {
    negative: bool,
    digits: Vec<u8>,
}

/// Parse GNU test's signed decimal integer syntax without imposing a size limit.
fn parse_integer(value: &OsStr) -> ParseResult<ParsedInteger> {
    let bytes = value.as_encoded_bytes();
    let mut start = 0;
    let mut end = bytes.len();

    while start < end && integer_is_blank(bytes[start]) {
        start += 1;
    }
    while start < end && integer_is_blank(bytes[end - 1]) {
        end -= 1;
    }

    let mut negative = false;
    if start < end {
        match bytes[start] {
            b'+' => start += 1,
            b'-' => {
                negative = true;
                start += 1;
            }
            _ => {}
        }
    }

    let digits = &bytes[start..end];
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return Err(ParseError::InvalidInteger(value.quote().to_string()));
    }

    let first_nonzero = digits
        .iter()
        .position(|digit| *digit != b'0')
        .unwrap_or(digits.len() - 1);
    let digits = digits[first_nonzero..].to_vec();
    if digits == b"0" {
        negative = false;
    }

    Ok(ParsedInteger { negative, digits })
}

/// 根据给定的操作符比较两个整数
fn compare_integers(a: &ParsedInteger, b: &ParsedInteger, op: &str) -> ParseResult<bool> {
    let magnitude_ordering = a
        .digits
        .len()
        .cmp(&b.digits.len())
        .then_with(|| a.digits.cmp(&b.digits));
    let ordering = match (a.negative, b.negative) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (true, true) => magnitude_ordering.reverse(),
        (false, false) => magnitude_ordering,
    };

    match op {
        "-eq" => Ok(ordering == Ordering::Equal),
        "-ne" => Ok(ordering != Ordering::Equal),
        "-gt" => Ok(ordering == Ordering::Greater),
        "-ge" => Ok(ordering != Ordering::Less),
        "-lt" => Ok(ordering == Ordering::Less),
        "-le" => Ok(ordering != Ordering::Greater),
        _ => Err(ParseError::UnknownOperator(op.to_string())),
    }
}

/// 进行整数比较操作
fn test_integers(a: &OsStr, b: &OsStr, op: &OsStr) -> ParseResult<bool> {
    // Parse input values
    let left = parse_integer(a)?;
    let right = parse_integer(b)?;

    // Get operator as string
    let operator = op
        .to_str()
        .ok_or_else(|| ParseError::UnknownOperator(op.quote().to_string()))?;

    // Compare values
    compare_integers(&left, &right, operator)
}

/// 比较文件元数据的操作
fn files(a: &OsStr, b: &OsStr, op: &OsStr) -> ParseResult<bool> {
    // Don't manage the error. GNU doesn't show error when doing
    // test foo -nt bar
    let f_a = fs::metadata(a).ok();
    let f_b = fs::metadata(b).ok();

    Ok(match op.to_str() {
        #[cfg(unix)]
        Some("-ef") => match (f_a.as_ref(), f_b.as_ref()) {
            (Some(f_a), Some(f_b)) => f_a.ino() == f_b.ino() && f_a.dev() == f_b.dev(),
            _ => false,
        },
        #[cfg(not(unix))]
        Some("-ef") => unimplemented!(),
        Some("-nt") => newer_than(f_a.as_ref(), f_b.as_ref()),
        Some("-ot") => older_than(f_a.as_ref(), f_b.as_ref()),
        _ => return Err(ParseError::UnknownOperator(op.quote().to_string())),
    })
}

fn mtime(metadata: Option<&fs::Metadata>) -> Option<std::time::SystemTime> {
    metadata.and_then(|metadata| metadata.modified().ok())
}

fn newer_than(a: Option<&fs::Metadata>, b: Option<&fs::Metadata>) -> bool {
    match (mtime(a), mtime(b)) {
        (Some(a), Some(b)) => a > b,
        (Some(_), None) => true,
        _ => false,
    }
}

fn older_than(a: Option<&fs::Metadata>, b: Option<&fs::Metadata>) -> bool {
    match (mtime(a), mtime(b)) {
        (Some(a), Some(b)) => a < b,
        (None, Some(_)) => true,
        _ => false,
    }
}

/// 检查文件描述符是否为终端
fn isatty(fd: &OsStr) -> ParseResult<bool> {
    fd.to_str()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| ParseError::InvalidInteger(fd.quote().to_string()))
        .map(|i| unsafe { libc::isatty(i) == 1 })
}

#[derive(Eq, PartialEq)]
enum TestPathCondition {
    BlockSpecial,
    CharacterSpecial,
    Directory,
    Exists,
    ExistsModifiedLastRead,
    Regular,
    GroupIdFlag,
    GroupOwns,
    SymLink,
    Sticky,
    UserOwns,
    Fifo,
    Readable,
    Socket,
    NonEmpty,
    UserIdFlag,
    Writable,
    Executable,
}

/// 在非Windows平台上检查文件路径条件
#[cfg(not(windows))]
fn path(path: &OsStr, condition: &TestPathCondition) -> bool {
    use std::os::unix::fs::FileTypeExt;

    const S_ISUID: u32 = 0o4000;
    const S_ISGID: u32 = 0o2000;
    const S_ISVTX: u32 = 0o1000;

    match condition {
        TestPathCondition::Readable => return effective_access(path, libc::R_OK),
        TestPathCondition::Writable => return effective_access(path, libc::W_OK),
        TestPathCondition::Executable => return effective_access(path, libc::X_OK),
        _ => {}
    }

    let metadata = if condition == &TestPathCondition::SymLink {
        fs::symlink_metadata(path)
    } else {
        fs::metadata(path)
    };

    let Ok(metadata) = metadata else {
        return false;
    };

    let file_type = metadata.file_type();

    match condition {
        TestPathCondition::BlockSpecial => file_type.is_block_device(),
        TestPathCondition::CharacterSpecial => file_type.is_char_device(),
        TestPathCondition::Directory => file_type.is_dir(),
        TestPathCondition::Exists => true,
        TestPathCondition::ExistsModifiedLastRead => {
            metadata.accessed().unwrap() < metadata.modified().unwrap()
        }
        TestPathCondition::Regular => file_type.is_file(),
        TestPathCondition::GroupIdFlag => metadata.mode() & S_ISGID != 0,
        TestPathCondition::GroupOwns => metadata.gid() == getegid(),
        TestPathCondition::SymLink => metadata.file_type().is_symlink(),
        TestPathCondition::Sticky => metadata.mode() & S_ISVTX != 0,
        TestPathCondition::UserOwns => metadata.uid() == geteuid(),
        TestPathCondition::Fifo => file_type.is_fifo(),
        TestPathCondition::Socket => file_type.is_socket(),
        TestPathCondition::NonEmpty => metadata.size() > 0,
        TestPathCondition::UserIdFlag => metadata.mode() & S_ISUID != 0,
        TestPathCondition::Readable
        | TestPathCondition::Writable
        | TestPathCondition::Executable => unreachable!(),
    }
}

#[cfg(not(windows))]
fn effective_access(path: &OsStr, mode: libc::c_int) -> bool {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let Ok(path) = CString::new(path.as_bytes()) else {
        return false;
    };

    // GNU test uses euidaccess for -r/-w/-x; AT_EACCESS checks effective IDs.
    unsafe { libc::faccessat(libc::AT_FDCWD, path.as_ptr(), mode, libc::AT_EACCESS) == 0 }
}

/// 在Windows平台上检查文件路径条件
#[cfg(windows)]
fn path(path: &OsStr, condition: &TestPathCondition) -> bool {
    use std::fs::metadata;

    let stat = match metadata(path) {
        Ok(s) => s,
        _ => return false,
    };

    match condition {
        TestPathCondition::BlockSpecial => false,
        TestPathCondition::CharacterSpecial => false,
        TestPathCondition::Directory => stat.is_dir(),
        TestPathCondition::Exists => true,
        TestPathCondition::ExistsModifiedLastRead => unimplemented!(),
        TestPathCondition::Regular => stat.is_file(),
        TestPathCondition::GroupIdFlag => false,
        TestPathCondition::GroupOwns => unimplemented!(),
        TestPathCondition::SymLink => false,
        TestPathCondition::Sticky => false,
        TestPathCondition::UserOwns => unimplemented!(),
        TestPathCondition::Fifo => false,
        TestPathCondition::Readable => false, // TODO
        TestPathCondition::Socket => false,
        TestPathCondition::NonEmpty => stat.len() > 0,
        TestPathCondition::UserIdFlag => false,
        TestPathCondition::Writable => false,   // TODO
        TestPathCondition::Executable => false, // TODO
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::ffi::OsString;

    #[test]
    fn test_integer_op() {
        let a = OsStr::new("18446744073709551616");
        let b = OsStr::new("0");
        assert!(!test_integers(a, b, OsStr::new("-lt")).unwrap());
        let a = OsStr::new("18446744073709551616");
        let b = OsStr::new("0");
        assert!(test_integers(a, b, OsStr::new("-gt")).unwrap());
        let a = OsStr::new("-1");
        let b = OsStr::new("0");
        assert!(test_integers(a, b, OsStr::new("-lt")).unwrap());
        let a = OsStr::new("42");
        let b = OsStr::new("42");
        assert!(test_integers(a, b, OsStr::new("-eq")).unwrap());
        let a = OsStr::new("42");
        let b = OsStr::new("42");
        assert!(!test_integers(a, b, OsStr::new("-ne")).unwrap());
    }

    #[test]
    fn integer_operands_accept_surrounding_blanks() {
        for value in [" 42", "42 ", "\t42\t", " \t+00042\t "] {
            assert!(test_integers(OsStr::new(value), OsStr::new("42"), OsStr::new("-eq")).unwrap());
        }
        assert!(
            test_integers(
                OsStr::new(" \t-00042\t"),
                OsStr::new("-41"),
                OsStr::new("-lt")
            )
            .unwrap()
        );

        for value in ["", " ", "+", "-", "4 2", "42\n"] {
            assert!(test_integers(OsStr::new(value), OsStr::new("0"), OsStr::new("-eq")).is_err());
        }
    }

    #[test]
    fn integer_comparisons_support_arbitrary_precision() {
        let above_i128 = OsStr::new("170141183460469231731687303715884105728");
        let below_i128 = OsStr::new("-170141183460469231731687303715884105729");
        assert!(test_integers(above_i128, OsStr::new("0"), OsStr::new("-gt")).unwrap());
        assert!(test_integers(below_i128, OsStr::new("0"), OsStr::new("-lt")).unwrap());

        let huge = "184467440737095516160000000000000000000000000";
        let padded = format!("+000{huge}");
        assert!(test_integers(OsStr::new(huge), OsStr::new(&padded), OsStr::new("-eq")).unwrap());
        assert!(test_integers(OsStr::new("-0"), OsStr::new("+000"), OsStr::new("-eq")).unwrap());
    }

    #[test]
    fn test_tool_implementation() {
        let tool = Test;

        // 测试 name 方法
        assert_eq!(tool.name(), "test");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("test"));

        // 测试 execute 方法
        let args = vec![OsString::from("test"), OsString::from("--version")];
        assert!(tool.execute(&args).is_ok());
    }
}

#[cfg(test)]
mod tests_all {
    use super::*;
    use std::ffi::OsStr;
    use std::ffi::OsString;
    use std::fs::File;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use tempfile::tempdir;

    // Helper function to create test files with specific permissions
    fn create_test_file(path: &Path, mode: u32) -> std::io::Result<()> {
        let file = File::create(path)?;
        let mut perms = file.metadata()?.permissions();
        perms.set_mode(mode);
        file.set_permissions(perms)?;
        Ok(())
    }

    #[test]
    fn test_integer_operations() {
        // Test all integer comparison operators
        let test_cases = vec![
            ("0", "0", "-eq", true),
            ("1", "0", "-eq", false),
            ("1", "0", "-ne", true),
            ("0", "1", "-ne", true),
            ("1", "0", "-gt", true),
            ("0", "1", "-gt", false),
            ("0", "1", "-lt", true),
            ("1", "0", "-lt", false),
            ("1", "1", "-ge", true),
            ("2", "1", "-ge", true),
            ("1", "2", "-ge", false),
            ("1", "1", "-le", true),
            ("1", "2", "-le", true),
            ("2", "1", "-le", false),
        ];

        for (a, b, op, expected) in test_cases {
            let result = test_integers(OsStr::new(a), OsStr::new(b), OsStr::new(op)).unwrap();
            assert_eq!(result, expected, "Failed: {a} {op} {b}");
        }

        // Test invalid integer
        assert!(
            test_integers(
                OsStr::new("not_a_number"),
                OsStr::new("0"),
                OsStr::new("-eq")
            )
            .is_err()
        );

        // Test invalid operator
        assert!(test_integers(OsStr::new("0"), OsStr::new("0"), OsStr::new("-invalid")).is_err());
    }

    #[test]
    fn test_string_operations() {
        let mut stack = Vec::new();

        // Test string equality
        stack.push(TestSymbol::Literal(OsString::from("abc")));
        stack.push(TestSymbol::Literal(OsString::from("abc")));
        stack.push(TestSymbol::Op(TestOperator::String(OsString::from("="))));
        assert!(test_eval(&mut stack).unwrap());

        // Test string inequality
        let mut stack = Vec::new();
        stack.push(TestSymbol::Literal(OsString::from("abc")));
        stack.push(TestSymbol::Literal(OsString::from("def")));
        stack.push(TestSymbol::Op(TestOperator::String(OsString::from("!="))));
        assert!(test_eval(&mut stack).unwrap());

        // Test string comparison
        let mut stack = Vec::new();
        stack.push(TestSymbol::Literal(OsString::from("abc")));
        stack.push(TestSymbol::Literal(OsString::from("def")));
        stack.push(TestSymbol::Op(TestOperator::String(OsString::from("<"))));
        assert!(test_eval(&mut stack).unwrap());

        // Test empty string
        let mut stack = Vec::new();
        stack.push(TestSymbol::Literal(OsString::from("")));
        stack.push(TestSymbol::UnaryOp(TestUnaryOperator::StrlenOp(
            OsString::from("-z"),
        )));
        assert!(test_eval(&mut stack).unwrap());
    }

    #[test]
    fn test_file_operations() {
        let temp_dir = tempdir().unwrap();
        let test_file = temp_dir.path().join("test_file");
        let test_dir = temp_dir.path().join("test_dir");
        let test_symlink = temp_dir.path().join("test_symlink");

        // Create test files
        create_test_file(&test_file, 0o644).unwrap();
        std::fs::create_dir(&test_dir).unwrap();
        std::os::unix::fs::symlink(&test_file, &test_symlink).unwrap();

        // Test file existence (-e)
        assert!(path(test_file.as_os_str(), &TestPathCondition::Exists));

        // Test regular file (-f)
        assert!(path(test_file.as_os_str(), &TestPathCondition::Regular));

        // Test directory (-d)
        assert!(path(test_dir.as_os_str(), &TestPathCondition::Directory));

        // Test symbolic link (-h or -L)
        assert!(path(test_symlink.as_os_str(), &TestPathCondition::SymLink));

        // Test readable (-r)
        assert!(path(test_file.as_os_str(), &TestPathCondition::Readable));

        // Test writable (-w)
        assert!(path(test_file.as_os_str(), &TestPathCondition::Writable));

        // Test non-existent file
        assert!(!path(
            OsStr::new("nonexistent_file"),
            &TestPathCondition::Exists
        ));
    }

    #[test]
    fn test_access_checks_use_effective_permissions() {
        let temp_dir = tempdir().unwrap();
        let test_file = temp_dir.path().join("mode000");

        create_test_file(&test_file, 0o000).unwrap();

        if geteuid() == 0 {
            assert!(path(test_file.as_os_str(), &TestPathCondition::Readable));
            assert!(path(test_file.as_os_str(), &TestPathCondition::Writable));
        } else {
            assert!(!path(test_file.as_os_str(), &TestPathCondition::Readable));
            assert!(!path(test_file.as_os_str(), &TestPathCondition::Writable));
        }

        assert!(!path(test_file.as_os_str(), &TestPathCondition::Executable));
    }

    #[test]
    fn test_file_age_operations_with_missing_files() {
        let temp_dir = tempdir().unwrap();
        let existing = temp_dir.path().join("existing");
        let missing = temp_dir.path().join("missing");
        let also_missing = temp_dir.path().join("also_missing");

        create_test_file(&existing, 0o644).unwrap();

        assert!(files(existing.as_os_str(), missing.as_os_str(), OsStr::new("-nt")).unwrap());
        assert!(files(missing.as_os_str(), existing.as_os_str(), OsStr::new("-ot")).unwrap());
        assert!(!files(existing.as_os_str(), missing.as_os_str(), OsStr::new("-ot")).unwrap());
        assert!(!files(missing.as_os_str(), existing.as_os_str(), OsStr::new("-nt")).unwrap());
        assert!(
            !files(
                missing.as_os_str(),
                also_missing.as_os_str(),
                OsStr::new("-nt")
            )
            .unwrap()
        );
        assert!(
            !files(
                missing.as_os_str(),
                also_missing.as_os_str(),
                OsStr::new("-ot")
            )
            .unwrap()
        );
    }

    #[test]
    fn test_logical_operations() {
        // Test AND operation
        let mut stack = Vec::new();
        stack.push(TestSymbol::Literal(OsString::from("true")));
        stack.push(TestSymbol::Literal(OsString::from("true")));
        stack.push(TestSymbol::BoolOp(OsString::from("-a")));
        assert!(test_eval(&mut stack).unwrap());

        // Test OR operation
        let mut stack = Vec::new();
        stack.push(TestSymbol::Literal(OsString::from("true")));
        stack.push(TestSymbol::Literal(OsString::from("false")));
        stack.push(TestSymbol::BoolOp(OsString::from("-o")));
        assert!(test_eval(&mut stack).unwrap());
    }

    #[test]
    fn test_command_line_args() {
        // Test valid expression
        let args = vec![
            OsString::from("test"),
            OsString::from("1"),
            OsString::from("-eq"),
            OsString::from("1"),
        ];
        assert!(test_main(args.into_iter()).is_ok());

        // Test invalid expression
        let args = vec![
            OsString::from("test"),
            OsString::from("1"),
            OsString::from("-invalid"),
            OsString::from("1"),
        ];
        assert!(test_main(args.into_iter()).is_err());
    }

    #[test]
    fn test_tool_implementation() {
        let tool = Test;

        // Test name method
        assert_eq!(tool.name(), "test");

        // Test command method
        let command = tool.command();
        assert!(command.get_name().contains("test"));

        // Test execute method with valid expression
        let args = vec![
            OsString::from("test"),
            OsString::from("1"),
            OsString::from("-eq"),
            OsString::from("1"),
        ];
        assert!(tool.execute(&args).is_ok());
    }

    #[test]
    fn test_error_handling() {
        // Test invalid integer
        let args = vec![
            OsString::from("test"),
            OsString::from("not_a_number"),
            OsString::from("-eq"),
            OsString::from("0"),
        ];
        assert!(test_main(args.into_iter()).is_err());

        // Test invalid operator
        let args = vec![
            OsString::from("test"),
            OsString::from("1"),
            OsString::from("-invalid"),
            OsString::from("1"),
        ];
        assert!(test_main(args.into_iter()).is_err());
    }
}
