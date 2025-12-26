/*
 *    Copyright(c) 2022-2024 China Telecom Cloud Technologies co., Ltd. All rights reserved
 *     syskits is licensed under Mulan PSL v2.
 *    You can use this software according to the terms and conditions of the Mulan PSL V2
 *    You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 *    THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 *    KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 *    NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 *    See the Mulan PSL v2 for more details.
 *
 */

// spell-checker:ignore (ToDO) delim mkdelim

use ctcore::ct_error::{CTResult, FromIo};
use ctcore::ct_line_ending::CtLineEnding;
use ctcore::{ct_format_usage, ct_help_about, ct_help_usage};
use std::cmp::Ordering;
use std::fs::File;
use std::io::{self, stdin, BufRead, BufReader, Stdin};
use std::path::Path;

use clap::{crate_version, Arg, ArgAction, ArgMatches, Command};

const COMM_ABOUT: &str = ct_help_about!("comm.md");
const COMM_USAGE: &str = ct_help_usage!("comm.md");

mod opt_flags {
    pub const COLUMN_1: &str = "1";
    pub const COLUMN_2: &str = "2";
    pub const COLUMN_3: &str = "3";
    pub const DELIMITER: &str = "output-delimiter";
    pub const DELIMITER_DEFAULT: &str = "\t";
    pub const FILE_1: &str = "FILE1";
    pub const FILE_2: &str = "FILE2";
    pub const TOTAL: &str = "total";
    pub const ZERO_TERMINATED: &str = "zero-terminated";
}
#[derive(Debug)]
enum CommInput {
    Stdin(Stdin),
    FileIn(BufReader<File>),
}
#[derive(Debug)]
struct CommLineReader {
    line_ending: CtLineEnding,
    input: CommInput,
}

impl CommLineReader {
    fn new(comm_input: CommInput, line_ending: CtLineEnding) -> Self {
        Self {
            input: comm_input,
            line_ending,
        }
    }

    fn read_line(&mut self, buf: &mut Vec<u8>) -> io::Result<usize> {
        let comm_line_ending = self.line_ending.into();

        let result = match &mut self.input {
            CommInput::Stdin(r) => r.lock().read_until(comm_line_ending, buf),
            CommInput::FileIn(r) => r.read_until(comm_line_ending, buf),
        };

        if !buf.ends_with(&[comm_line_ending]) {
            buf.push(comm_line_ending);
        }

        result
    }
}
/**
 * 对两个命令行读取器中的数据行进行比较，并根据选项输出结果。
 *
 * @param a 第一个命令行读取器的引用，用于读取第一组数据。
 * @param b 第二个命令行读取器的引用，用于读取第二组数据。
 * @param opts 包含各种选项的参数匹配器，用于定制比较和输出的行为。
 */
fn comm(a: &mut CommLineReader, b: &mut CommLineReader, opts: &ArgMatches) {
    // 根据选项获取分隔符
    let delim = comm_get_del_im(opts);

    // 通过选项确定第一、第二列的宽度
    let width_col_1 = usize::from(!opts.get_flag(opt_flags::COLUMN_1));
    let width_col_2 = usize::from(!opts.get_flag(opt_flags::COLUMN_2));

    // 根据列宽度计算第二、第三列的分隔符
    let delim_col_2 = delim.repeat(width_col_1);
    let delim_col_3 = delim.repeat(width_col_1 + width_col_2);

    // 初始化用于读取数据的缓冲区及读取状态
    let ra = &mut Vec::new();
    let mut na = a.read_line(ra);
    let rb = &mut Vec::new();
    let mut nb = b.read_line(rb);

    // 初始化用于计数的变量
    let mut total_col_1 = 0;
    let mut total_col_2 = 0;
    let mut total_col_3 = 0;

    // 循环读取并比较两组数据，直到其中一组读取完毕
    while na.is_ok() || nb.is_ok() {
        // 根据两行数据的状态进行比较
        let ord = match (na.is_ok(), nb.is_ok()) {
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
            (true, true) => match (&na, &nb) {
                (&Ok(0), &Ok(0)) => break, // 两行都读取完毕，退出循环
                (&Ok(0), _) => Ordering::Greater,
                (_, &Ok(0)) => Ordering::Less,
                _ => ra.cmp(&rb),
            },
            _ => unreachable!(), // 理论上不应到达此处
        };

        // 根据比较结果输出相应行数据，并准备下一次读取
        if ord == Ordering::Less {
            if !opts.get_flag(opt_flags::COLUMN_1) {
                print!("{}", String::from_utf8_lossy(ra));
            }
            ra.clear();
            na = a.read_line(ra);
            total_col_1 += 1;
        } else if ord == Ordering::Greater {
            if !opts.get_flag(opt_flags::COLUMN_2) {
                print!("{delim_col_2}{}", String::from_utf8_lossy(rb));
            }
            rb.clear();
            nb = b.read_line(rb);
            total_col_2 += 1;
        } else if ord == Ordering::Equal {
            if !opts.get_flag(opt_flags::COLUMN_3) {
                print!("{delim_col_3}{}", String::from_utf8_lossy(ra));
            }
            ra.clear();
            rb.clear();
            na = a.read_line(ra);
            nb = b.read_line(rb);
            total_col_3 += 1;
        }
    }

    // 根据选项输出总计数行
    if opts.get_flag(opt_flags::TOTAL) {
        let line_ending = CtLineEnding::from_zero_flag(opts.get_flag(opt_flags::ZERO_TERMINATED));
        print!("{total_col_1}{delim}{total_col_2}{delim}{total_col_3}{delim}total{line_ending}");
    }
}

fn comm_get_del_im(options: &ArgMatches) -> &str {
    let del_im = match options
        .get_one::<String>(opt_flags::DELIMITER)
        .unwrap()
        .as_str()
    {
        "" => "\0",
        delim => delim,
    };
    del_im
}

fn open_file(file_name: &str, line_ending: CtLineEnding) -> io::Result<CommLineReader> {
    match file_name {
        "-" => Ok(CommLineReader::new(CommInput::Stdin(stdin()), line_ending)),
        _ => {
            let f = File::open(Path::new(file_name))?;
            Ok(CommLineReader::new(
                CommInput::FileIn(BufReader::new(f)),
                line_ending,
            ))
        }
    }
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    comm_main(args).map(|_| ())
}

pub fn comm_main(args: impl ctcore::Args) -> CTResult<i32> {
    let matches = ct_app().try_get_matches_from(args)?;
    let line_ending = CtLineEnding::from_zero_flag(matches.get_flag(opt_flags::ZERO_TERMINATED));
    let tmp_file1 = matches.get_one::<String>(opt_flags::FILE_1).unwrap();
    let tmp_file2 = matches.get_one::<String>(opt_flags::FILE_2).unwrap();
    let mut f1 = open_file(tmp_file1, line_ending).map_err_context(|| tmp_file1.to_string())?;
    let mut f2 = open_file(tmp_file2, line_ending).map_err_context(|| tmp_file2.to_string())?;

    comm(&mut f1, &mut f2, &matches);
    Ok(0)
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = COMM_ABOUT;
    let usage_description = ct_format_usage(COMM_USAGE);

    let args = args_init();

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(&args)
}

fn args_init() -> Vec<Arg> {
    let args = vec![
        Arg::new(opt_flags::COLUMN_1)
            .short('1')
            .help("suppress column 1 (lines unique to FILE1)")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::COLUMN_2)
            .short('2')
            .help("suppress column 2 (lines unique to FILE2)")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::COLUMN_3)
            .short('3')
            .help("suppress column 3 (lines that appear in both files)")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::DELIMITER)
            .long(opt_flags::DELIMITER)
            .help("separate columns with STR")
            .value_name("STR")
            .default_value(opt_flags::DELIMITER_DEFAULT)
            .hide_default_value(true),
        Arg::new(opt_flags::ZERO_TERMINATED)
            .long(opt_flags::ZERO_TERMINATED)
            .short('z')
            .overrides_with(opt_flags::ZERO_TERMINATED)
            .help("line delimiter is NUL, not newline")
            .action(ArgAction::SetTrue),
        Arg::new(opt_flags::FILE_1)
            .required(true)
            .value_hint(clap::ValueHint::FilePath),
        Arg::new(opt_flags::FILE_2)
            .required(true)
            .value_hint(clap::ValueHint::FilePath),
        Arg::new(opt_flags::TOTAL)
            .long(opt_flags::TOTAL)
            .help("output a summary")
            .action(ArgAction::SetTrue),
    ];
    args
}

#[cfg(test)]
mod tests {
    // 这里是 `comm` 命令的使用说明和示例：
    //
    // 1. **基本用法**：
    // ```bash
    // comm FILE1 FILE2
    // ```
    // 比较已排序的文件 FILE1 和 FILE2，并按行比较它们。
    //
    // 2. **选项**：
    //
    // - `-1`：不显示 FILE1 中独有的行。
    // - `-2`：不显示 FILE2 中独有的行。
    // - `-3`：不显示两个文件共有的行。
    //
    // - `--output-delimiter=STR`：使用 STR 分隔列。
    // - `--total`：输出摘要信息。
    // - `-z, --zero-terminated`：行分隔符为 NUL 而不是换行符。
    // - `--help`：显示帮助信息并退出。
    // - `--version`：显示版本信息并退出。
    //
    // 3. **示例**：
    //
    // - 只打印同时存在于 file1 和 file2 中的行：
    // ```bash
    // comm -12 file1 file2
    // ```
    //
    // - 打印 file1 中不在 file2 中出现的行，以及 file2 中不在 file1 中出现的行：
    // ```bash
    // comm -3 file1 file2
    // ```
    //
    // 这些是 `comm` 命令的基本用法和选项示例。

    #[cfg(test)]
    mod tests_ct_app {
        use crate::ct_app;
        use crate::opt_flags;
        use crate::opt_flags::DELIMITER;
        use crate::opt_flags::DELIMITER_DEFAULT;
        use crate::opt_flags::FILE_1;
        use crate::opt_flags::FILE_2;

        use clap::error::ErrorKind;

        #[test]
        fn test_ct_app_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_v() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_help() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_h() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-h"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_column_1() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-1", "file1", "file2"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result
                .unwrap()
                .get_one::<bool>(opt_flags::COLUMN_1)
                .unwrap());
        }

        #[test]
        fn test_ct_app_column_2() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-2", "file1", "file2"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result
                .unwrap()
                .get_one::<bool>(opt_flags::COLUMN_2)
                .unwrap());
        }

        #[test]
        fn test_ct_app_column_3() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-3", "file1", "file2"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result
                .unwrap()
                .get_one::<bool>(opt_flags::COLUMN_3)
                .unwrap());
        }

        #[test]
        fn test_ct_app_column_12() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-12", "file1", "file2"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result
                .unwrap()
                .get_one::<bool>(opt_flags::COLUMN_1)
                .unwrap());
        }

        #[test]
        fn test_ct_app_column_13() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-13", "file1", "file2"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result
                .unwrap()
                .get_one::<bool>(opt_flags::COLUMN_3)
                .unwrap());
        }

        #[test]
        fn test_ct_app_column_23() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-23", "file1", "file2"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result
                .unwrap()
                .get_one::<bool>(opt_flags::COLUMN_3)
                .unwrap());
        }

        #[test]
        fn test_ct_app_column_11() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-11", "file1", "file2"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::ArgumentConflict);
        }

        #[test]
        fn test_ct_app_column_22() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-22", "file1", "file2"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::ArgumentConflict);
        }

        #[test]
        fn test_ct_app_column_33() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-33", "file1", "file2"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::ArgumentConflict);
        }

        #[test]
        fn test_ct_app_column_123() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-123", "file1", "file2"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result
                .unwrap()
                .get_one::<bool>(opt_flags::COLUMN_1)
                .unwrap());
            // assert!(result.unwrap().get_one::<bool>(opt_flags::COLUMN_2).unwrap());
            // assert!(result.unwrap().get_one::<bool>(opt_flags::COLUMN_3).unwrap());
        }

        #[test]
        fn test_ct_app_zero_terminated() {
            let command = ct_app();
            let args = vec![
                ctcore::ct_util_name(),
                "--zero-terminated",
                "file1",
                "file2",
            ];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result
                .unwrap()
                .get_one::<bool>(opt_flags::ZERO_TERMINATED)
                .unwrap());
        }

        #[test]
        fn test_ct_app_zero() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-z", "file1", "file2"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result
                .unwrap()
                .get_one::<bool>(opt_flags::ZERO_TERMINATED)
                .unwrap());
        }

        #[test]
        fn test_ct_app_total() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--total", "file1", "file2"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert!(result.unwrap().get_one::<bool>(opt_flags::TOTAL).unwrap());
        }
        #[test]
        fn test_ct_app_file_1() {
            let command = ct_app();
            let test_file_path = "test_ct_app_file_1.txt"; // 测试文件路径
            let expected_result = FILE_1;
            let flag = opt_flags::FILE_1.to_string();
            let files = test_file_path.to_string();
            let args = vec![ctcore::ct_util_name(), &flag, &files];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(
                result
                    .unwrap()
                    .get_one::<String>(opt_flags::FILE_1)
                    .unwrap(),
                expected_result
            );
        }

        #[test]
        fn test_ct_app_file_2() {
            let command = ct_app();
            let test_file_path = "test_ct_app_file_2.txt"; // 测试文件路径
                                                           // let expected_result = FILE_2;
            let flag = FILE_2.to_string();
            let files = test_file_path.to_string();
            let args = vec![ctcore::ct_util_name(), &flag, &files];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(
                result
                    .unwrap()
                    .get_one::<String>(opt_flags::FILE_2)
                    .unwrap(),
                test_file_path
            );
        }

        #[test]
        fn test_ct_app_delimiter() {
            let command = ct_app();
            let test_file = "test_ct_app_delimiter.txt"; // 测试文件路径

            let expected_result = DELIMITER_DEFAULT;
            let flag = DELIMITER.to_string();
            let file1 = test_file.to_string();

            let args = vec![ctcore::ct_util_name(), &flag, &file1];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(
                result
                    .unwrap()
                    .get_one::<String>(opt_flags::DELIMITER)
                    .unwrap(),
                expected_result
            );
        }

        #[test]
        fn test_ct_app_delimiter_default() {
            let command = ct_app();
            let test_file = "test_ct_app_delimiter_default.txt"; // 测试文件路径

            let expected_result = DELIMITER_DEFAULT;
            let flag = DELIMITER_DEFAULT.to_string();
            let file1 = test_file.to_string();

            let args = vec![ctcore::ct_util_name(), &flag, &file1];
            let result = command.try_get_matches_from(args);

            assert!(result.is_ok());
            assert_eq!(
                result
                    .unwrap()
                    .get_one::<String>(opt_flags::DELIMITER)
                    .unwrap(),
                expected_result
            );
        }
    }

    #[cfg(test)]
    mod tests_ct_main {
        use crate::opt_flags;
        use crate::opt_flags::DELIMITER;
        use crate::opt_flags::DELIMITER_DEFAULT;

        use crate::comm_main;
        use crate::opt_flags::FILE_2;
        use std::ffi::OsString;

        #[test]
        fn test_ct_main_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = comm_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 0);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_v() {
            let args = vec![ctcore::ct_util_name(), "-V"];
            let result = comm_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 0);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = comm_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 0);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_h() {
            let args = vec![ctcore::ct_util_name(), "-h"];
            let result = comm_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 0);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_column_1() {
            let args = vec![ctcore::ct_util_name(), "-1", "file1", "file2"];
            let result = comm_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_column_2() {
            let args = vec![ctcore::ct_util_name(), "-2", "file1", "file2"];
            let result = comm_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_column_3() {
            let args = vec![ctcore::ct_util_name(), "-3", "file1", "file2"];
            let result = comm_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_column_12() {
            let args = vec![ctcore::ct_util_name(), "-12", "file1", "file2"];
            let result = comm_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_column_13() {
            let args = vec![ctcore::ct_util_name(), "-13", "file1", "file2"];
            let result = comm_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_column_23() {
            let args = vec![ctcore::ct_util_name(), "-23", "file1", "file2"];
            let result = comm_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_column_11() {
            let args = vec![ctcore::ct_util_name(), "-11", "file1", "file2"];
            let result = comm_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_column_22() {
            let args = vec![ctcore::ct_util_name(), "-22", "file1", "file2"];
            let result = comm_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_column_33() {
            let args = vec![ctcore::ct_util_name(), "-33", "file1", "file2"];
            let result = comm_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_column_123() {
            let args = vec![ctcore::ct_util_name(), "-123", "file1", "file2"];
            let result = comm_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_zero_terminated() {
            let args = vec![
                ctcore::ct_util_name(),
                "--zero-terminated",
                "file1",
                "file2",
            ];
            let result = comm_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_zero() {
            let args = vec![ctcore::ct_util_name(), "-z", "file1", "file2"];
            let result = comm_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_total() {
            let args = vec![ctcore::ct_util_name(), "--total", "file1", "file2"];
            let result = comm_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }
        #[test]
        fn test_ct_main_file_1() {
            let test_file_path = "test_ct_main_file_1.txt"; // 测试文件路径

            let flag = opt_flags::FILE_1.to_string();
            let files = test_file_path.to_string();
            let args = vec![ctcore::ct_util_name(), &flag, &files];
            let result = comm_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_file_2() {
            let test_file_path = "test_ct_main_file_2.txt"; // 测试文件路径
                                                            // let expected_result = FILE_2;
            let flag = FILE_2.to_string();
            let files = test_file_path.to_string();
            let args = vec![ctcore::ct_util_name(), &flag, &files];
            let result = comm_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_delimiter() {
            let test_file = "test_ct_main_delimiter.txt"; // 测试文件路径

            let flag = DELIMITER.to_string();
            let file1 = test_file.to_string();

            let args = vec![ctcore::ct_util_name(), &flag, &file1];
            let result = comm_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }

        #[test]
        fn test_ct_main_delimiter_default() {
            let test_file = "test_ct_main_delimiter_default.txt"; // 测试文件路径

            let flag = DELIMITER_DEFAULT.to_string();
            let file1 = test_file.to_string();

            let args = vec![ctcore::ct_util_name(), &flag, &file1];
            let result = comm_main(args.iter().map(|s| OsString::from(s)));

            match result {
                Err(output) => {
                    assert_eq!(output.code(), 1);
                }
                Ok(output) => {
                    assert_eq!(output, 0);
                }
            }
        }
    }
    #[cfg(test)]
    mod tests_ct_line_reader {
        use crate::ct_app;
        use crate::opt_flags::COLUMN_1;
        use crate::opt_flags::COLUMN_2;
        use crate::opt_flags::COLUMN_3;
        use crate::opt_flags::DELIMITER;
        use crate::opt_flags::DELIMITER_DEFAULT;
        use crate::opt_flags::FILE_1;
        use crate::opt_flags::FILE_2;
        use crate::opt_flags::TOTAL;
        use crate::opt_flags::ZERO_TERMINATED;
        use ctcore::ct_line_ending::CtLineEnding;

        #[test]
        fn test_ct_line_ending_from_zero_flag() {
            let command = ct_app();
            let test_file = "test_ct_line_ending_from_zero_flag.txt"; // 测试文件路径
            let expected_result = CtLineEnding::Newline;
            let flag = ZERO_TERMINATED.to_string();
            let file1 = test_file.to_string();

            let args = vec![ctcore::ct_util_name(), &flag, &file1];

            let matches = command.try_get_matches_from(args);
            let line_ending =
                CtLineEnding::from_zero_flag(matches.expect("REASON").get_flag(ZERO_TERMINATED));
            assert_eq!(line_ending, expected_result);

            // let filename1 = matches.get_one::<String>(FILE_1).unwrap();
            // let filename2 = matches.get_one::<String>(FILE_2).unwrap();
            // let mut f1 = open_file(filename1, line_ending).map_err_context(|| filename1.to_string())?;
            // let mut f2 = open_file(filename2, line_ending).map_err_context(|| filename2.to_string())?;
        }

        #[test]
        fn test_ct_line_ending_from_delimiter_flag() {
            let command = ct_app();
            let test_file = "test_ct_line_ending_from_delimiter_flag.txt"; // 测试文件路径
            let expected_result = CtLineEnding::Newline;
            let flag = DELIMITER.to_string();
            let file1 = test_file.to_string();

            let args = vec![ctcore::ct_util_name(), &flag, &file1];
            let matches = command.try_get_matches_from(args);
            let line_ending =
                CtLineEnding::from_zero_flag(matches.expect("REASON").get_flag(ZERO_TERMINATED));

            assert_eq!(line_ending, expected_result);
        }

        #[test]
        fn test_ct_line_ending_from_delimiter_default_flag() {
            let command = ct_app();
            let test_file = "test_ct_line_ending_from_delimiter_default_flag.txt"; // 测试文件路径
            let expected_result = CtLineEnding::Newline;
            let flag = DELIMITER_DEFAULT.to_string();
            let file1 = test_file.to_string();

            let args = vec![ctcore::ct_util_name(), &flag, &file1];
            let matches = command.try_get_matches_from(args);
            let line_ending =
                CtLineEnding::from_zero_flag(matches.expect("REASON").get_flag(ZERO_TERMINATED));

            assert_eq!(line_ending, expected_result);
        }

        #[test]
        fn test_ct_line_ending_from_total_flag() {
            let command = ct_app();
            let test_file = "test_ct_line_ending_from_total_flag.txt"; // 测试文件路径
            let expected_result = CtLineEnding::Newline;
            let flag = TOTAL.to_string();
            let file1 = test_file.to_string();

            let args = vec![ctcore::ct_util_name(), &flag, &file1];

            let matches = command.try_get_matches_from(args);
            let line_ending =
                CtLineEnding::from_zero_flag(matches.expect("REASON").get_flag(ZERO_TERMINATED));
            assert_eq!(line_ending, expected_result);
        }

        #[test]
        fn test_ct_line_ending_from_file1_flag() {
            let command = ct_app();
            let test_file = "test_ct_line_ending_from_file1_flag.txt"; // 测试文件路径
            let expected_result = CtLineEnding::Newline;
            let flag = FILE_1.to_string();
            let file1 = test_file.to_string();

            let args = vec![ctcore::ct_util_name(), &flag, &file1];

            let matches = command.try_get_matches_from(args);
            let line_ending =
                CtLineEnding::from_zero_flag(matches.expect("REASON").get_flag(ZERO_TERMINATED));
            assert_eq!(line_ending, expected_result);
        }

        #[test]
        fn test_ct_line_ending_from_file2_flag() {
            let command = ct_app();
            let test_file = "test_ct_line_ending_from_file2_flag.txt"; // 测试文件路径
            let expected_result = CtLineEnding::Newline;
            let flag = FILE_2.to_string();
            let file1 = test_file.to_string();

            let args = vec![ctcore::ct_util_name(), &flag, &file1];

            let matches = command.try_get_matches_from(args);
            let line_ending =
                CtLineEnding::from_zero_flag(matches.expect("REASON").get_flag(ZERO_TERMINATED));
            assert_eq!(line_ending, expected_result);
        }

        #[test]
        fn test_ct_line_ending_from_column_1_flag() {
            let command = ct_app();
            let test_file = "test_ct_line_ending_from_column_1_flag.txt"; // 测试文件路径
            let expected_result = CtLineEnding::Newline;
            let flag = COLUMN_1.to_string();
            let file1 = test_file.to_string();

            let args = vec![ctcore::ct_util_name(), &flag, &file1];

            let matches = command.try_get_matches_from(args);
            let line_ending =
                CtLineEnding::from_zero_flag(matches.expect("REASON").get_flag(ZERO_TERMINATED));
            assert_eq!(line_ending, expected_result);
        }

        #[test]
        fn test_ct_line_ending_from_column_2_flag() {
            let command = ct_app();
            let test_file = "test_ct_line_ending_from_column_2_flag.txt"; // 测试文件路径
            let expected_result = CtLineEnding::Newline;
            let flag = COLUMN_2.to_string();
            let file1 = test_file.to_string();

            let args = vec![ctcore::ct_util_name(), &flag, &file1];

            let matches = command.try_get_matches_from(args);
            let line_ending =
                CtLineEnding::from_zero_flag(matches.expect("REASON").get_flag(ZERO_TERMINATED));
            assert_eq!(line_ending, expected_result);
        }

        #[test]
        fn test_ct_line_ending_from_column_3_flag() {
            let command = ct_app();
            let test_file = "test_ct_line_ending_from_column_3_flag.txt"; // 测试文件路径
            let expected_result = CtLineEnding::Newline;
            let flag = COLUMN_3.to_string();
            let file1 = test_file.to_string();

            let args = vec![ctcore::ct_util_name(), &flag, &file1];

            let matches = command.try_get_matches_from(args);
            let line_ending =
                CtLineEnding::from_zero_flag(matches.expect("REASON").get_flag(ZERO_TERMINATED));
            assert_eq!(line_ending, expected_result);
        }
    }
    #[cfg(test)]
    mod tests_ct_opt_flags {
        use crate::ct_app;

        use crate::opt_flags::COLUMN_1;
        use crate::opt_flags::COLUMN_2;
        use crate::opt_flags::COLUMN_3;
        use crate::opt_flags::DELIMITER;
        use crate::opt_flags::DELIMITER_DEFAULT;
        use crate::opt_flags::FILE_1;
        use crate::opt_flags::FILE_2;
        use crate::opt_flags::TOTAL;
        use crate::opt_flags::ZERO_TERMINATED;

        #[test]
        fn tests_ct_opt_flags_column_1() {
            let test_file = "tests_ct_opt_flags_column_1.txt"; // 测试文件路径
            let expected_result = COLUMN_1;
            let flag = COLUMN_1.to_string();

            let file1 = test_file.to_string();
            let args = vec![ctcore::ct_util_name(), &flag, &file1];

            let matches = ct_app().try_get_matches_from(args);

            let binding = matches.expect("REASON");
            let filename1 = binding.get_one::<String>(FILE_1).unwrap();

            assert_eq!(filename1, expected_result);
        }

        #[test]
        fn tests_ct_opt_flags_column_2() {
            let test_file = "tests_ct_opt_flags_column_2.txt"; // 测试文件路径
            let expected_result = COLUMN_2;
            let flag = COLUMN_2.to_string();

            let file1 = test_file.to_string();
            let args = vec![ctcore::ct_util_name(), &flag, &file1];

            let matches = ct_app().try_get_matches_from(args);

            let binding = matches.expect("REASON");
            let filename1 = binding.get_one::<String>(FILE_1).unwrap();

            assert_eq!(filename1, expected_result);
        }

        #[test]
        fn tests_ct_opt_flags_column_3() {
            let test_file = "tests_ct_opt_flags_column_3.txt"; // 测试文件路径
            let expected_result = COLUMN_3;
            let flag = COLUMN_3.to_string();

            let file1 = test_file.to_string();
            let args = vec![ctcore::ct_util_name(), &flag, &file1];

            let matches = ct_app().try_get_matches_from(args);

            let binding = matches.expect("REASON");
            let filename1 = binding.get_one::<String>(FILE_1).unwrap();

            assert_eq!(filename1, expected_result);
        }

        #[test]
        fn tests_ct_opt_flags_delimiter() {
            let test_file = "tests_ct_opt_flags_delimter.txt"; // 测试文件路径
            let expected_result = DELIMITER;
            let flag = DELIMITER.to_string();

            let file1 = test_file.to_string();
            let args = vec![ctcore::ct_util_name(), &flag, &file1];

            let matches = ct_app().try_get_matches_from(args);

            let binding = matches.expect("REASON");
            let filename1 = binding.get_one::<String>(FILE_1).unwrap();

            assert_eq!(filename1, expected_result);
        }

        #[test]
        fn tests_ct_opt_flags_delimiter_default() {
            let test_file = "tests_ct_opt_flags_delimiter_default.txt"; // 测试文件路径
            let expected_result = DELIMITER_DEFAULT;
            let flag = DELIMITER_DEFAULT.to_string();

            let file1 = test_file.to_string();
            let args = vec![ctcore::ct_util_name(), &flag, &file1];

            let matches = ct_app().try_get_matches_from(args);

            let binding = matches.expect("REASON");
            let filename1 = binding.get_one::<String>(FILE_1).unwrap();

            assert_eq!(filename1, expected_result);
        }

        #[test]
        fn tests_ct_opt_flags_file_1() {
            let test_file = "tests_ct_opt_flags_file_1.txt"; // 测试文件路径
            let expected_result = FILE_1;
            let flag = FILE_1.to_string();

            let file1 = test_file.to_string();
            let args = vec![ctcore::ct_util_name(), &flag, &file1];

            let matches = ct_app().try_get_matches_from(args);

            let binding = matches.expect("REASON");
            let filename1 = binding.get_one::<String>(FILE_1).unwrap();

            assert_eq!(filename1, expected_result);
        }
        #[test]
        fn tests_ct_opt_flags_file_2() {
            let test_file = "tests_ct_opt_flags_file_2.txt"; // 测试文件路径
            let expected_result = FILE_2;
            let flag = FILE_2.to_string();

            let file1 = test_file.to_string();
            let args = vec![ctcore::ct_util_name(), &flag, &file1];

            let matches = ct_app().try_get_matches_from(args);

            let binding = matches.expect("REASON");
            let filename1 = binding.get_one::<String>(FILE_1).unwrap();

            assert_eq!(filename1, expected_result);
        }

        #[test]
        fn tests_ct_opt_flags_total() {
            let test_file = "tests_ct_opt_flags_total.txt"; // 测试文件路径
            let expected_result = TOTAL;
            let flag = TOTAL.to_string();

            let file1 = test_file.to_string();
            let args = vec![ctcore::ct_util_name(), &flag, &file1];

            let matches = ct_app().try_get_matches_from(args);

            let binding = matches.expect("REASON");
            let filename1 = binding.get_one::<String>(FILE_1).unwrap();

            assert_eq!(filename1, expected_result);
        }
        #[test]
        fn tests_ct_opt_flags_zero_terminated() {
            let test_file = "tests_ct_opt_flags_zero_terminated.txt"; // 测试文件路径
            let expected_result = ZERO_TERMINATED;
            let flag = ZERO_TERMINATED.to_string();

            let file1 = test_file.to_string();
            let args = vec![ctcore::ct_util_name(), &flag, &file1];

            let matches = ct_app().try_get_matches_from(args);

            let binding = matches.expect("REASON");
            let filename1 = binding.get_one::<String>(FILE_1).unwrap();

            assert_eq!(filename1, expected_result);
        }
    }

    #[cfg(test)]
    mod tests_ct_open_file {
        use crate::ct_app;
        use crate::open_file;
        use crate::opt_flags;
        use ctcore::ct_error::FromIo;
        use ctcore::ct_line_ending::CtLineEnding;
        use std::fs;
        use std::fs::File;
        use tempfile::Builder;

        use crate::opt_flags::COLUMN_1;
        use crate::opt_flags::COLUMN_2;
        use crate::opt_flags::COLUMN_3;
        use crate::opt_flags::DELIMITER;
        use crate::opt_flags::DELIMITER_DEFAULT;
        use crate::opt_flags::FILE_1;
        use crate::opt_flags::FILE_2;
        use crate::opt_flags::TOTAL;
        use crate::opt_flags::ZERO_TERMINATED;

        #[test]
        fn tests_ct_open_file_zero_terminated() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("tests_ct_open_file_zero_terminated_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ct_open_file_zero_terminated_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let flag = ZERO_TERMINATED.to_string();

            let file1 = filename1.to_string();
            let args = vec![ctcore::ct_util_name(), &flag, &file1];

            let matches = ct_app().try_get_matches_from(args);
            let line_ending = CtLineEnding::from_zero_flag(
                matches
                    .expect("REASON")
                    .get_flag(opt_flags::ZERO_TERMINATED),
            );

            let f1 = open_file(filename1, line_ending).map_err_context(|| filename1.to_string());

            assert_eq!(f1.is_ok(), true);
        }

        #[test]
        fn tests_ct_open_file_total() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("tests_ct_open_file_total")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ct_open_file_total.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let flag = TOTAL.to_string();

            let file1 = filename1.to_string();
            let args = vec![ctcore::ct_util_name(), &flag, &file1];

            let matches = ct_app().try_get_matches_from(args);
            let line_ending =
                CtLineEnding::from_zero_flag(matches.expect("REASON").get_flag(opt_flags::TOTAL));

            let f1 = open_file(filename1, line_ending).map_err_context(|| filename1.to_string());

            assert_eq!(f1.is_ok(), true);
        }

        #[test]
        fn tests_ct_open_file_file_1() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("tests_ct_open_file_file_1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ct_open_file_file_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let flag = FILE_1.to_string();

            let file1 = filename1.to_string();
            let args = vec![ctcore::ct_util_name(), &flag, &file1];

            let matches = ct_app().try_get_matches_from(args);
            let line_ending =
                CtLineEnding::from_zero_flag(matches.expect("REASON").get_flag(opt_flags::TOTAL));

            let f1 = open_file(filename1, line_ending).map_err_context(|| filename1.to_string());

            assert_eq!(f1.is_ok(), true);
        }

        #[test]
        fn tests_ct_open_file_file_2() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("tests_ct_open_file_file_2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ct_open_file_file_2.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let flag = FILE_2.to_string();

            let file1 = filename1.to_string();
            let args = vec![ctcore::ct_util_name(), &flag, &file1];

            let matches = ct_app().try_get_matches_from(args);
            let line_ending =
                CtLineEnding::from_zero_flag(matches.expect("REASON").get_flag(opt_flags::TOTAL));

            let f1 = open_file(filename1, line_ending).map_err_context(|| filename1.to_string());

            assert_eq!(f1.is_ok(), true);
        }

        #[test]
        fn tests_ct_open_file_delimiter() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("tests_ct_open_file_delimiter")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ct_open_file_delimiter.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let flag = DELIMITER.to_string();

            let file1 = filename1.to_string();
            let args = vec![ctcore::ct_util_name(), &flag, &file1];

            let matches = ct_app().try_get_matches_from(args);
            let line_ending =
                CtLineEnding::from_zero_flag(matches.expect("REASON").get_flag(opt_flags::TOTAL));

            let f1 = open_file(filename1, line_ending).map_err_context(|| filename1.to_string());

            assert_eq!(f1.is_ok(), true);
        }
        #[test]
        fn tests_ct_open_file_delimiter_default() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("tests_ct_open_file_delimiter_default")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ct_open_file_delimiter_default.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let flag = DELIMITER_DEFAULT.to_string();

            let file1 = filename1.to_string();
            let args = vec![ctcore::ct_util_name(), &flag, &file1];

            let matches = ct_app().try_get_matches_from(args);
            let line_ending =
                CtLineEnding::from_zero_flag(matches.expect("REASON").get_flag(opt_flags::TOTAL));

            let f1 = open_file(filename1, line_ending).map_err_context(|| filename1.to_string());

            assert_eq!(f1.is_ok(), true);
        }

        #[test]
        fn tests_ct_open_file_column_1() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("tests_ct_open_file_column_1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ct_open_file_column_1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let flag = COLUMN_1.to_string();

            let file1 = filename1.to_string();
            let args = vec![ctcore::ct_util_name(), &flag, &file1];

            let matches = ct_app().try_get_matches_from(args);
            let line_ending = CtLineEnding::from_zero_flag(
                matches.expect("REASON").get_flag(opt_flags::COLUMN_1),
            );

            let f1 = open_file(filename1, line_ending).map_err_context(|| filename1.to_string());

            assert_eq!(f1.is_ok(), true);
        }

        #[test]
        fn tests_ct_open_file_column_2() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("tests_ct_open_file_column_2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ct_open_file_column_2.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let flag = COLUMN_2.to_string();

            let file1 = filename1.to_string();
            let args = vec![ctcore::ct_util_name(), &flag, &file1];

            let matches = ct_app().try_get_matches_from(args);
            let line_ending = CtLineEnding::from_zero_flag(
                matches.expect("REASON").get_flag(opt_flags::COLUMN_2),
            );

            let f1 = open_file(filename1, line_ending).map_err_context(|| filename1.to_string());

            assert_eq!(f1.is_ok(), true);
        }

        #[test]
        fn tests_ct_open_file_column_3() {
            // 创建临时目录结构
            let temp_dir = Builder::new()
                .prefix("tests_ct_open_file_column_3")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ct_open_file_column_3.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let flag = COLUMN_3.to_string();

            let file1 = filename1.to_string();
            let args = vec![ctcore::ct_util_name(), &flag, &file1];

            let matches = ct_app().try_get_matches_from(args);
            let line_ending = CtLineEnding::from_zero_flag(
                matches.expect("REASON").get_flag(opt_flags::COLUMN_3),
            );

            let f1 = open_file(filename1, line_ending).map_err_context(|| filename1.to_string());

            assert_eq!(f1.is_ok(), true);
        }
    }

    #[cfg(test)]
    mod tests_ctmain {
        use crate::ctmain;
        use std::ffi::OsString;
        use std::fs;
        use std::fs::File;
        use tempfile::Builder;

        #[test]
        fn tests_ctmain_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_h() {
            let args = vec![ctcore::ct_util_name(), "--h"];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_v() {
            let args = vec![ctcore::ct_util_name(), "-V"];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_column_1() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_file2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-1", filename1, filename2];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_column_2() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_2_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_flag_column_2_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_2_file2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_flag_column_2_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-2", filename1, filename2];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_column_3() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_3_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_flag_column_3_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_3_file2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_flag_column_3_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-3", filename1, filename2];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_column_12() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_12_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_flag_column_12_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_12_file2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_flag_column_12_file1.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-12", filename1, filename2];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_column_13() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_13_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_flag_column_13_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_13_file2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_flag_column_13_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-13", filename1, filename2];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_column_23() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_23_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_flag_column_23_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_23_file2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_flag_column_23_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-23", filename1, filename2];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }
        #[test]
        fn tests_ctmain_flag_column_123() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_123_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_flag_column_123_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_123_file2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_flag_column_123_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-123", filename1, filename2];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_zero() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_zero_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_flag_zero_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_zero_file2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_flag_zero_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "-z", filename1, filename2];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_zero_terminated() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_zero_terminated_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_flag_zero_terminated_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_zero_terminated_file2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_flag_zero_terminated_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--zero-terminated",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_total_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_flag_total_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_total_file2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_flag_total_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![ctcore::ct_util_name(), "--total", filename1, filename2];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }
        #[test]
        fn tests_ctmain_flag_zero_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_zero_total_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_flag_zero_total_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_zero_total_file2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_flag_zero_total_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-z",
                "--total",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_zero_terminated_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_zero_terminated_total_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 =
                sub_dir_path.join("tests_ctmain_flag_zero_terminated_total_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_zero_terminated_total_file2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 =
                sub_dir_path.join("tests_ctmain_flag_zero_terminated_total_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--zero-terminated",
                "--total",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_output_delimiter() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_output_delimiter_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_flag_output_delimiter_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_output_delimiter_file2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_flag_output_delimiter_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--output-delimiter=STR",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_output_delimiter_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_output_delimiter_total_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 =
                sub_dir_path.join("tests_ctmain_flag_output_delimiter_total_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_output_delimiter_total_file2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 =
                sub_dir_path.join("tests_ctmain_flag_output_delimiter_total_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--output-delimiter=STR",
                "--total",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_output_delimiter_zero() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_output_delimiter_zero_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 =
                sub_dir_path.join("tests_ctmain_flag_output_delimiter_zero_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_output_delimiter_zero_file2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 =
                sub_dir_path.join("tests_ctmain_flag_output_delimiter_zero_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--output-delimiter=STR",
                "--zero-terminated",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_output_delimiter_zero_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_output_delimiter_zero_total_file1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 =
                sub_dir_path.join("tests_ctmain_flag_output_delimiter_zero_total_file1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_output_delimiter_zero_total_file2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 =
                sub_dir_path.join("tests_ctmain_flag_output_delimiter_zero_total_file2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "--output-delimiter=STR",
                "--zero-terminated",
                "--total",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        //////////////////////////////////////////////////////////////////
        #[test]
        fn tests_ctmain_flag_column_1_zero() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_1_zero1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_flag_column_1_zero.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_1_zero2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_flag_column_1_zero.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-1",
                "--zero-terminated",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_column_2_zero() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_2_zero1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_flag_column_2_zero1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_2_zero2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_flag_column_2_zero.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-2",
                "--zero-terminated",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_column_3_zero() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_3_zero1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_flag_column_3_zero1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_3_zero2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_flag_column_3_zero.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-3",
                "--zero-terminated",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_column_12_zero() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_12_zero1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_flag_column_12_zero1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_12_zero2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_flag_column_12_zero.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-12",
                "--zero-terminated",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_column_13_zero() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_13_zero1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_flag_column_13_zero1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_13_zero2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_flag_column_13_zero2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-13",
                "--zero-terminated",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_column_23_zero() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_23_zero1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_flag_column_23_zero1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_23_zero2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_flag_column_23_zero2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-23",
                "--zero-terminated",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }
        #[test]
        fn tests_ctmain_flag_column_123_zero() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_123_zero1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_flag_column_123_zero.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_123_zero2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_flag_column_123_zero2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-123",
                "--zero-terminated",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_column_1_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_1_total1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_flag_column_1_total1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_1_total2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_flag_column_1_total2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-1",
                "--total",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_column_2_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_2_total1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_flag_column_2_total1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_2_total2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_flag_column_2_total2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-2",
                "--total",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_column_3_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_3_total1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_flag_column_3_total1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_3_total2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_flag_column_3_total2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-3",
                "--total",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_column_12_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_12_total1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_flag_column_12_total1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_12_total2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_flag_column_12_total2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-12",
                "--total",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_column_13_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_13_total1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_flag_column_13_total1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_13_total2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_flag_column_13_total2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-13",
                "--total",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_column_23_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_23_total1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_flag_column_23_total1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_23_total2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_flag_column_23_total2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-23",
                "--total",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }
        #[test]
        fn tests_ctmain_flag_column_123_total() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_123_total1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_flag_column_123_total1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_123_total2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_flag_column_123_total2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-123",
                "--total",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_column_1_output_delimiter() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_1_output_delimiter1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_flag_column_1_output_delimiter1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_1_output_delimiter2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_flag_column_1_output_delimiter2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-1",
                "--output-delimiter=STR",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_column_2_output_delimiter() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_2_output_delimiter1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_flag_column_2_output_delimiter1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_2_output_delimiter2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_flag_column_2_output_delimiter2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-2",
                "--output-delimiter=STR",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_column_3_output_delimiter() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_3_output_delimiter1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_flag_column_3_output_delimiter1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_3_output_delimiter2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_flag_column_3_output_delimiter2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-3",
                "--output-delimiter=STR",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_column_12_output_delimiter() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_12_output_delimiter1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 =
                sub_dir_path.join("tests_ctmain_flag_column_12_output_delimiter1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_12_output_delimiter2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 =
                sub_dir_path.join("tests_ctmain_flag_column_12_output_delimiter2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-12",
                "--output-delimiter=STR",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_column_13_output_delimiter() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_13_output_delimiter1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 =
                sub_dir_path.join("tests_ctmain_flag_column_13_output_delimiter1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_13_output_delimiter2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 =
                sub_dir_path.join("tests_ctmain_flag_column_13_output_delimiter2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-13",
                "--output-delimiter=STR",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_column_23_output_delimiter() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_23_output_delimiter1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 =
                sub_dir_path.join("tests_ctmain_flag_column_23_output_delimiter1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_23_output_delimiter2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 =
                sub_dir_path.join("tests_ctmain_flag_column_23_output_delimiter2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-23",
                "--output-delimiter=STR",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }
        #[test]
        fn tests_ctmain_flag_column_123_output_delimiter() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_123_output_delimiter1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 =
                sub_dir_path.join("tests_ctmain_flag_column_123_output_delimiter1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_123_output_delimiter2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 =
                sub_dir_path.join("tests_ctmain_flag_column_123_output_delimiter2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-123",
                "--output-delimiter=STR",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_column_1_output_delimiter_total_lines() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_1_output_delimiter_total_lines1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 =
                sub_dir_path.join("tests_ctmain_flag_column_1_output_delimiter_total_lines1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_1_output_delimiter_total_lines2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 =
                sub_dir_path.join("tests_ctmain_flag_column_1_output_delimiter_total_lines2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-1",
                "--output-delimiter=STR",
                "--total",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_column_2_output_delimiter_total_lines() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_2_output_delimiter_total_lines1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 =
                sub_dir_path.join("tests_ctmain_flag_column_2_output_delimiter_total_lines1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_2_output_delimiter_total_lines2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 =
                sub_dir_path.join("tests_ctmain_flag_column_2_output_delimiter_total_lines2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-2",
                "--output-delimiter=STR",
                "--total",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_column_3_output_delimiter_total_lines() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_3_output_delimiter_total_lines1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 =
                sub_dir_path.join("tests_ctmain_flag_column_3_output_delimiter_total_lines1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_3_output_delimiter_total_lines2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 =
                sub_dir_path.join("tests_ctmain_flag_column_3_output_delimiter_total_lines2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-3",
                "--output-delimiter=STR",
                "--total",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_column_12_output_delimiter_total_lines() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_12_output_delimiter_total_lines1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 =
                sub_dir_path.join("tests_ctmain_flag_column_12_output_delimiter_total_lines1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_12_output_delimiter_total_lines2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 =
                sub_dir_path.join("tests_ctmain_flag_column_12_output_delimiter_total_lines2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-12",
                "--output-delimiter=STR",
                "--total",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_column_13_output_delimiter_total_lines() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_13_output_delimiter_total_lines1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 =
                sub_dir_path.join("tests_ctmain_flag_column_13_output_delimiter_total_lines1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_13_output_delimiter_total_lines2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 =
                sub_dir_path.join("tests_ctmain_flag_column_13_output_delimiter_total_lines2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-13",
                "--output-delimiter=STR",
                "--total",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_flag_column_23_output_delimiter_total_lines() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_23_output_delimiter_total_lines1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 =
                sub_dir_path.join("tests_ctmain_flag_column_23_output_delimiter_total_lines1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_23_output_delimiter_total_lines2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 =
                sub_dir_path.join("tests_ctmain_flag_column_23_output_delimiter_total_lines2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-23",
                "--output-delimiter=STR",
                "--total",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }
        #[test]
        fn tests_ctmain_flag_column_123_output_delimiter_total_lines() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_123_output_delimiter_total_lines1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 =
                sub_dir_path.join("tests_ctmain_flag_column_123_output_delimiter_total_lines1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_flag_column_123_output_delimiter_total_lines2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 =
                sub_dir_path.join("tests_ctmain_flag_column_123_output_delimiter_total_lines2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-123",
                "--output-delimiter=STR",
                "--total",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_all_args_12() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_all_args_12")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_all_args_12.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_all_args_122")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_all_args_122.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-12",
                "--output-delimiter=STR",
                "--total",
                "--zero",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_all_args_13() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_all_args_131")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_all_args_131.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_all_args_132")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_all_args_132.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-13",
                "--output-delimiter=STR",
                "--total",
                "--zero",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }

        #[test]
        fn tests_ctmain_all_args_23() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_all_args_231")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_all_args_231.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_all_args_232")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_all_args_232.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-23",
                "--output-delimiter=STR",
                "--total",
                "--zero",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }
        #[test]
        fn tests_ctmain_all_args_123() {
            let temp_dir = Builder::new()
                .prefix("tests_ctmain_all_args1")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_1 = sub_dir_path.join("tests_ctmain_all_args1.txt");
            File::create(&test_file_1).unwrap();
            let filename1 = test_file_1.to_str().unwrap();

            let temp_dir = Builder::new()
                .prefix("tests_ctmain_all_args2")
                .tempdir()
                .unwrap();
            let sub_dir_path = temp_dir.path().join("sub_dir");
            fs::create_dir(&sub_dir_path).unwrap();
            let test_file_2 = sub_dir_path.join("tests_ctmain_all_args2.txt");
            File::create(&test_file_2).unwrap();
            let filename2 = test_file_2.to_str().unwrap();

            let args = vec![
                ctcore::ct_util_name(),
                "-123",
                "--output-delimiter=STR",
                "--total",
                "--zero",
                filename1,
                filename2,
            ];
            let result = ctmain(args.iter().map(|s| OsString::from(s)));
            assert_eq!(result, 0);
        }
    }

}