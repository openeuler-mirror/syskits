// This file is part of the uutils coreutils package.
//
// For the full copyright and license information, please view the LICENSE
// file that was distributed with this source code.

// spell-checker:ignore (vars) egid euid FiletestOp StrlenOp

pub(crate) mod error;
mod parser;

use clap::Command;
use error::{ParseError, ParseResult};
use parser::{TestOperator, TestSymbol, TestUnaryOperator, test_parse};
use std::ffi::{OsStr, OsString};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CtSimpleError};
#[cfg(not(windows))]
use ctcore::ct_process::{getegid, geteuid};
use ctcore::{ct_format_usage, ct_help_about, ct_help_section, ct_help_usage};
use clap::crate_version;
use ctcore::Tool;

const CT_ABOUT: &str = ct_help_about!("test.md");

// The help_usage method replaces util name (the first word) with {}.
// And, The format_usage method replaces {} with execution_phrase ( e.g. test or [ ).
// However, This test command has two util names.
// So, we use test or [ instead of {} so that the usage string is correct.
//"\
//test EXPRESSION
//[
//[ EXPRESSION ]
//[ ]
//[ OPTION
//]";
const CT_USAGE: &str = ct_help_usage!("test.md");

// We use after_help so that this comes after the usage string (it would come before if we used about)
const CT_AFTER_HELP: &str = ct_help_section!("after help", "test.md");

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

pub fn ct_app() -> Command {
    // Disable printing of -h and -v as valid alternatives for --help and --version,
    // since we don't recognize -h and -v as help/version flags.
    Command::new(ctcore::ct_util_name())
        .version(crate_version!())
        .about(CT_ABOUT)
        .override_usage(ct_format_usage(CT_USAGE))
        .after_help(CT_AFTER_HELP)
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    test_main(args)
}

/// Handle help and version flags for both test and [ commands
fn handle_help_version(program: &OsString, args: &[OsString]) -> Option<CTResult<()>> {
    if args.len() == 1 && (args[0] == "--help" || args[0] == "--version") {
        ct_app().get_matches_from(std::iter::once(program.clone()).chain(args.iter().cloned()));
        return Some(Ok(()));
    }
    None
}

/// Handle the closing bracket for [ command
fn handle_closing_bracket(binary_name: &str, args: &mut Vec<OsString>) -> CTResult<()> {
    if binary_name.ends_with('[') {
        let last = args.pop();
        if last.as_deref() != Some(OsStr::new("]")) {
            return Err(CtSimpleError::new(2, "missing ']'"));
        }
    }
    Ok(())
}

pub fn test_main(mut args: impl ctcore::Args) -> CTResult<()> {
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

/// Evaluate a stack of Symbols, returning the result of the evaluation or
/// an error message if evaluation failed.
fn test_eval(stack: &mut Vec<TestSymbol>) -> ParseResult<bool> {
    macro_rules! pop_literal {
        () => {
            match stack.pop() {
                Some(TestSymbol::Literal(s)) => s,
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
            let b = pop_literal!();
            let a = pop_literal!();

            Ok(test_integers(&a, &b, &op)?)
        }
        Some(TestSymbol::Op(TestOperator::File(op))) => {
            let b = pop_literal!();
            let a = pop_literal!();
            Ok(files(&a, &b, &op)?)
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

/// Parse string to i128 with proper error handling
fn parse_integer(value: &OsStr) -> ParseResult<i128> {
    value
        .to_str()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| ParseError::InvalidInteger(value.quote().to_string()))
}

/// Compare two integers based on the given operator
fn compare_integers(a: i128, b: i128, op: &str) -> ParseResult<bool> {
    match op {
        "-eq" => Ok(a == b),
        "-ne" => Ok(a != b),
        "-gt" => Ok(a > b),
        "-ge" => Ok(a >= b),
        "-lt" => Ok(a < b),
        "-le" => Ok(a <= b),
        _ => Err(ParseError::UnknownOperator(op.to_string())),
    }
}

/// Operations to compare integers
/// `a` is the left hand side
/// `b` is the left hand side
/// `op` the operation (ex: -eq, -lt, etc)
fn test_integers(a: &OsStr, b: &OsStr, op: &OsStr) -> ParseResult<bool> {
    // Parse input values
    let left = parse_integer(a)?;
    let right = parse_integer(b)?;

    // Get operator as string
    let operator = op
        .to_str()
        .ok_or_else(|| ParseError::UnknownOperator(op.quote().to_string()))?;

    // Compare values
    compare_integers(left, right, operator)
}

/// Operations to compare files metadata
/// `a` is the left hand side
/// `b` is the left hand side
/// `op` the operation (ex: -ef, -nt, etc)
fn files(a: &OsStr, b: &OsStr, op: &OsStr) -> ParseResult<bool> {
    // Don't manage the error. GNU doesn't show error when doing
    // test foo -nt bar
    let (Ok(f_a), Ok(f_b)) = (fs::metadata(a), fs::metadata(b)) else {
        return Ok(false);
    };

    Ok(match op.to_str() {
        #[cfg(unix)]
        Some("-ef") => f_a.ino() == f_b.ino() && f_a.dev() == f_b.dev(),
        #[cfg(not(unix))]
        Some("-ef") => unimplemented!(),
        Some("-nt") => f_a.modified().unwrap() > f_b.modified().unwrap(),
        Some("-ot") => f_a.modified().unwrap() < f_b.modified().unwrap(),
        _ => return Err(ParseError::UnknownOperator(op.quote().to_string())),
    })
}

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

#[cfg(not(windows))]
fn path(path: &OsStr, condition: &TestPathCondition) -> bool {
    use std::fs::Metadata;
    use std::os::unix::fs::FileTypeExt;

    const S_ISUID: u32 = 0o4000;
    const S_ISGID: u32 = 0o2000;
    const S_ISVTX: u32 = 0o1000;

    enum Permission {
        Read = 0o4,
        Write = 0o2,
        Execute = 0o1,
    }

    let perm = |metadata: Metadata, p: Permission| {
        if geteuid() == metadata.uid() {
            metadata.mode() & ((p as u32) << 6) != 0
        } else if getegid() == metadata.gid() {
            metadata.mode() & ((p as u32) << 3) != 0
        } else {
            metadata.mode() & (p as u32) != 0
        }
    };

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
        TestPathCondition::Readable => perm(metadata, Permission::Read),
        TestPathCondition::Socket => file_type.is_socket(),
        TestPathCondition::NonEmpty => metadata.size() > 0,
        TestPathCondition::UserIdFlag => metadata.mode() & S_ISUID != 0,
        TestPathCondition::Writable => perm(metadata, Permission::Write),
        TestPathCondition::Executable => perm(metadata, Permission::Execute),
    }
}

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
