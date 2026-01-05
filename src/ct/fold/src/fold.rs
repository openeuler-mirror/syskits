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

//! 对每个指定的文件设置自动换行（折行），并将重新排版后的结果输出到标准输出。

use clap::{Arg, ArgAction, Command, crate_version};
use ctcore::ct_error::{CTResult, FromIo};
use ctcore::{ct_format_usage, ct_help_about, ct_help_usage};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write, stdin};
use std::path::Path;

const FOLD_TAB_WIDTH: usize = 8;
const FOLD_USAGE: &str = ct_help_usage!("fold.md");
const FOLD_ABOUT: &str = ct_help_about!("fold.md");

mod fold_flags {
    pub const FOLD_BYTES: &str = "bytes";
    pub const FOLD_SPACES: &str = "spaces";
    pub const FOLD_WIDTH: &str = "width";
    pub const FOLD_FILE: &str = "file";
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    fold_main(&mut out, args)
}

struct FoldFlags {
    bytes: bool,
    spaces: bool,
    width: usize,
    files: Vec<String>,
}

/// 主折叠函数，用于处理命令行参数并输出结果
///
/// # Parameters
///
/// - `writer`: 一个实现了Write trait的可变引用，用于输出结果
/// - `args`: 一个实现了ctcore::Args trait的参数源，用于提供命令行参数
///
/// # Returns
///
/// 返回一个Result，表示操作成功或失败
pub fn fold_main<W: Write>(writer: &mut W, args: impl ctcore::Args) -> CTResult<()> {
    let args = args.collect_lossy();

    let (args, obs_width) = handle_obsolete(&args[..]);
    let matches = ct_app().try_get_matches_from(args)?;

    let flags = FoldFlags {
        bytes: matches.get_flag(fold_flags::FOLD_BYTES),
        spaces: matches.get_flag(fold_flags::FOLD_SPACES),
        width: match matches.get_one::<String>(fold_flags::FOLD_WIDTH) {
            Some(v) => Some(v.clone()),
            None => obs_width,
        }
        .and_then(|inp_width| inp_width.parse::<usize>().ok())
        .unwrap_or(80),
        files: match matches.get_many::<String>(fold_flags::FOLD_FILE) {
            Some(v) => v.cloned().collect(),
            None => vec!["-".to_owned()],
        },
    };

    fold(writer, &flags)
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = FOLD_ABOUT;
    let usage_description = ct_format_usage(FOLD_USAGE);
    let args = vec![
        Arg::new(fold_flags::FOLD_BYTES)
            .long(fold_flags::FOLD_BYTES)
            .short('b')
            .help(
                "count using bytes rather than columns (meaning control characters \
                     such as newline are not treated specially)",
            )
            .action(ArgAction::SetTrue),
        Arg::new(fold_flags::FOLD_SPACES)
            .long(fold_flags::FOLD_SPACES)
            .short('s')
            .help("break lines at word boundaries rather than a hard cut-off")
            .action(ArgAction::SetTrue),
        Arg::new(fold_flags::FOLD_WIDTH)
            .long(fold_flags::FOLD_WIDTH)
            .short('w')
            .help("set WIDTH as the maximum line width rather than 80")
            .value_name("WIDTH")
            .allow_hyphen_values(true),
        Arg::new(fold_flags::FOLD_FILE)
            .hide(true)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::FilePath),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(&args)
}

/// 处理过时的参数。
///
/// 该函数检查命令行参数列表，查找以单个连字符（-）开头且后跟数字的参数。
/// 如果找到这样的参数，则将其从参数列表中移除，并将其值作为第二个返回值返回。
///
/// # 参数
///
/// - `args`: 命令行参数列表。
///
/// # 返回值
///
/// - 一个包含处理后参数的向量。
/// - 一个可选的字符串，表示找到的过时参数的值。
fn handle_obsolete(args: &[String]) -> (Vec<String>, Option<String>) {
    for (i, arg) in args.iter().enumerate() {
        // 检查参数是否以单个连字符（-）开头且后跟数字。
        let slice = &arg;
        if slice.starts_with('-') && slice.chars().nth(1).is_some_and(|c| c.is_ascii_digit()) {
            // 如果找到过时的参数，则将其从参数列表中移除。
            let mut v = args.to_vec();
            v.remove(i);
            // 返回处理后的参数列表和过时参数的值。
            return (v, Some(slice[1..].to_owned()));
        }
    }
    // 如果没有找到过时的参数，则返回原始参数列表和 None。
    (args.to_vec(), None)
}

/// 对文件内容进行折叠处理
///
/// 该函数根据提供的折叠标志（`FoldFlags`）对指定的文件进行处理。
/// 如果文件名是`-`，则从标准输入读取内容；否则，从指定的文件中读取内容。
/// 然后根据`bytes`标志决定是按字节还是按列进行折叠，并根据`spaces`标志决定是否在空格处进行换行。
///
/// # 参数
///
/// - `fold_flags`: 包含折叠标志的结构体。
///
/// # 返回值
///
/// - 如果折叠成功，返回`Ok(())`；如果发生错误，返回`Err`。
fn fold<W: Write>(writer: &mut W, fold_flags: &FoldFlags) -> CTResult<()> {
    // filenames
    // fn fold(filenames: &[String], bytes: bool, spaces: bool, width: usize) -> CTResult<()> {
    for filename in &fold_flags.files {
        let filename: &str = filename;
        let mut stdin_buf;
        let mut file_buf;
        let buffer = BufReader::new(if filename == "-" {
            // 如果文件名是`-`，则从标准输入读取内容
            stdin_buf = stdin();
            &mut stdin_buf as &mut dyn Read
        } else {
            // 否则，从指定的文件中读取内容
            file_buf = File::open(Path::new(filename)).map_err_context(|| filename.to_string())?;
            &mut file_buf as &mut dyn Read
        });

        let spaces = fold_flags.spaces;
        let width = fold_flags.width;
        if fold_flags.bytes {
            // 如果`bytes`标志为真，则按字节进行折叠
            fold_file_bytewise(writer, buffer, spaces, width)?;
        } else {
            // 否则，按列进行折叠
            fold_file(writer, buffer, spaces, width)?;
        }
    }
    Ok(())
}

/// 逐字节折叠文件内容，以适应指定的宽度。
///
/// 此函数处理 `-b`/`--bytes` 选项的折叠，将所有字符（包括制表符、退格符和回车符）视为占用一列。
/// 如果 `spaces` 为 `true`，则尝试在空白字符边界处换行。
fn fold_file_bytewise<T: Read, W: Write>(
    writer: &mut W,
    mut file: BufReader<T>,
    is_spaces: bool,
    width: usize,
) -> CTResult<()> {
    let mut line = String::new();

    loop {
        if file
            .read_line(&mut line)
            .map_err_context(|| "failed to read line".to_string())?
            == 0
        {
            break;
        }

        if line == "\n" {
            writeln!(writer)?;
            line.truncate(0);
            continue;
        }

        let len = line.len();
        let mut i = 0;

        while i < len {
            let width = if len - i >= width { width } else { len - i };
            let slice = {
                let slice = &line[i..i + width];
                if is_spaces && i + width < len {
                    match slice.rfind(|c: char| c.is_whitespace() && c != '\r') {
                        Some(m) => &slice[..=m],
                        None => slice,
                    }
                } else {
                    slice
                }
            };

            // 不重复换行符：如果子字符串是 "\n"，则上一次迭代已经在行尾折叠并打印了该换行符。
            if slice == "\n" {
                break;
            }

            i += slice.len();
            let at_eol = i >= len;

            if at_eol {
                write!(writer, "{slice}")?;
            } else {
                writeln!(writer, "{slice}")?;
            }
        }

        line.truncate(0);
    }

    Ok(())
}

/// 打印输出行，重置列数和字符数。
///
/// 如果 `spaces` 为 `true`，打印输出行直到上一个遇到的字符（包括空格），并将剩余字符设置为下一行的开头。
fn emit_output<W: Write>(
    writer: &mut W,
    output: &mut String,
    last_space: &mut Option<usize>,
    col_count: &mut usize,
) -> CTResult<()> {
    let consume = match *last_space {
        Some(i) => i + 1,
        None => output.len(),
    }
    .min(output.len());

    // println!("{}", &output[..consume]);
    writeln!(writer, "{}", &output[..consume])?;
    output.replace_range(..consume, "");

    // 我们知道输出中没有制表符了，所以每个字符计为 1 列
    *col_count = output.len();

    *last_space = None;

    Ok(())
}

/// 按列折叠文件内容，以适应指定的宽度。
///
/// 此函数处理默认的折叠选项，将制表符视为 8列，退格符减少列数，回车符重置列数。
/// 如果 `spaces` 为 `true`，则尝试在空白字符边界处换行。
fn fold_file<T: Read, W: Write>(
    writer: &mut W,
    mut file: BufReader<T>,
    is_spaces: bool,
    width: usize,
) -> CTResult<()> {
    let mut line = String::new();
    let mut output = String::new();
    let mut col_count = 0; // 当前行的列数
    let mut last_space = None; // 上一个空格字符的位置

    loop {
        // 读取文件的一行内容
        if file
            .read_line(&mut line)
            .map_err_context(|| "failed to read line".to_string())?
            == 0
        {
            break;
        }

        // 遍历当前行的每个字符
        for ch in line.chars() {
            if ch == '\n' {
                // 确保不拆分输出中的空格，因为我们知道整个输出将适合
                last_space = None;
                emit_output(writer, &mut output, &mut last_space, &mut col_count)?;
                break;
            }

            if col_count >= width {
                emit_output(writer, &mut output, &mut last_space, &mut col_count)?;
            }

            match ch {
                '\r' => col_count = 0,
                '\t' => {
                    let next_tab_stop = col_count + FOLD_TAB_WIDTH - col_count % FOLD_TAB_WIDTH;

                    if next_tab_stop > width && !output.is_empty() {
                        emit_output(writer, &mut output, &mut last_space, &mut col_count)?;
                    }

                    col_count = next_tab_stop;
                    last_space = if is_spaces { Some(output.len()) } else { None };
                }
                '\x08' => {
                    col_count = col_count.saturating_sub(1);
                }
                _ if is_spaces && ch.is_whitespace() => {
                    last_space = Some(output.len());
                    col_count += 1;
                }
                _ => col_count += 1,
            };

            output.push(ch);
        }

        if !output.is_empty() {
            write!(writer, "{output}")?;
            output.truncate(0);
        }

        line.truncate(0);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(test)]
    mod fold_main_tests {
        use super::*;
        use std::ffi::OsString;
        use std::fs::File;
        use tempfile::tempdir;

        #[test]
        fn test_ctmain_version() {
            let mut writer = Vec::new();
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = fold_main(&mut writer, args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    assert_eq!(output.code(), 0);
                }
                Ok(output) => {
                    println!("{:?}", output);
                }
            }
        }

        #[test]
        fn test_ctmain_v() {
            let mut writer = Vec::new();
            let args = vec![ctcore::ct_util_name(), "-V"];
            let result = fold_main(&mut writer, args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    assert_eq!(output.code(), 0);
                }
                Ok(output) => {
                    println!("{:?}", output);
                }
            }
        }

        #[test]
        fn test_ctmain_help() {
            let mut writer = Vec::new();
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = fold_main(&mut writer, args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    assert_eq!(output.code(), 0);
                }
                Ok(output) => {
                    println!("{:?}", output);
                }
            }
        }

        #[test]
        fn test_ctmain_h() {
            let mut writer = Vec::new();
            let args = vec![ctcore::ct_util_name(), "-h"];
            let result = fold_main(&mut writer, args.iter().map(|s| OsString::from(s)));
            match result {
                Err(output) => {
                    assert_eq!(output.code(), 0);
                }
                Ok(output) => {
                    println!("{:?}", output);
                }
            }
        }

        #[test]
        fn test_ct_main_long_option_b_short() {
            let mut writer = Vec::new();
            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let temp_file_path = temp_dir.path().join("fold_temp_file1.txt");
            let mut temp_file =
                File::create(&temp_file_path).expect("Failed to create temporary file");
            temp_file
                .write_all(b"aaaaaaaaaaaaaaaaaaaaaaaaa\n")
                .expect("Failed to write to temporary file");
            let binding = temp_file_path.to_string_lossy().into_owned();
            let args = vec![ctcore::ct_util_name(), "-b", &binding];
            let result = fold_main(&mut writer, args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_long_option_b_long() {
            let mut writer = Vec::new();

            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let temp_file_path = temp_dir.path().join("fold_temp_file1.txt");
            let mut temp_file =
                File::create(&temp_file_path).expect("Failed to create temporary file");
            temp_file
                .write_all(b"File 1\n")
                .expect("Failed to write to temporary file");
            let binding = temp_file_path.to_string_lossy().into_owned();

            let args = vec![ctcore::ct_util_name(), "--bytes", &binding];
            let result = fold_main(&mut writer, args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_long_option_s_short() {
            let mut writer = Vec::new();

            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let temp_file_path = temp_dir.path().join("fold_temp_file1.txt");
            let mut temp_file =
                File::create(&temp_file_path).expect("Failed to create temporary file");
            temp_file
                .write_all(b"File 1\n")
                .expect("Failed to write to temporary file");
            let binding = temp_file_path.to_string_lossy().into_owned();

            let args = vec![ctcore::ct_util_name(), "-s", &binding];
            let result = fold_main(&mut writer, args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_long_option_s_long() {
            let mut writer = Vec::new();

            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let temp_file_path = temp_dir.path().join("fold_temp_file1.txt");
            let mut temp_file =
                File::create(&temp_file_path).expect("Failed to create temporary file");
            temp_file
                .write_all(b"File 1\n")
                .expect("Failed to write to temporary file");
            let binding = temp_file_path.to_string_lossy().into_owned();

            let args = vec![ctcore::ct_util_name(), "--spaces", &binding];
            let result = fold_main(&mut writer, args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_long_option_w_short() {
            let mut writer = Vec::new();

            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let temp_file_path = temp_dir.path().join("fold_temp_file1.txt");
            let mut temp_file =
                File::create(&temp_file_path).expect("Failed to create temporary file");
            temp_file
                .write_all(b"File 1\n")
                .expect("Failed to write to temporary file");
            let binding = temp_file_path.to_string_lossy().into_owned();

            let args = vec![ctcore::ct_util_name(), "-w", "10", &binding];
            let result = fold_main(&mut writer, args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_main_long_option_w_long() {
            let mut writer = Vec::new();

            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let temp_file_path = temp_dir.path().join("fold_temp_file1.txt");
            let mut temp_file =
                File::create(&temp_file_path).expect("Failed to create temporary file");
            temp_file
                .write_all(b"File 1\n")
                .expect("Failed to write to temporary file");
            let binding = temp_file_path.to_string_lossy().into_owned();

            let args = vec![ctcore::ct_util_name(), "--width", "10", &binding];
            let result = fold_main(&mut writer, args.iter().map(|s| OsString::from(s)));

            assert!(result.is_ok());
        }
    }

    #[cfg(test)]
    mod ct_app_tests {
        use clap::error::ErrorKind;

        use super::*;

        // fold 接口测试: fold [OPTION]... [FILE]...
        //
        // Options:
        //   -b, --bytes          count using bytes rather than columns (meaning control characters such as newline are not treated specially)
        //   -s, --spaces         break lines at word boundaries rather than a hard cut-off
        //   -w, --width <WIDTH>  set WIDTH as the maximum line width rather than 80
        //   -h, --help           Print help
        //   -V, --version        Print version

        #[test]
        fn test_ct_app_execution_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];

            // Assuming `command` has a method to retrieve the executable name, replace it with the actual one
            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_other_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];
            let executable = command.try_get_matches_from(args);

            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_help() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "--help"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_execution_unsupport_help() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-H"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_invalid_argument() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_support_missing_argument() {
            let command = ct_app();

            let missing_args = vec![ctcore::ct_util_name()]; // 缺少任何参数
            let result = command.try_get_matches_from(missing_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_b_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-b"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
            assert!(executable.unwrap().contains_id(fold_flags::FOLD_BYTES));
        }

        #[test]
        fn test_ct_app_long_option_b_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--bytes"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
            assert!(executable.unwrap().contains_id(fold_flags::FOLD_BYTES));
        }

        #[test]
        fn test_ct_app_long_option_s_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-s"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
            assert!(executable.unwrap().contains_id(fold_flags::FOLD_SPACES));
        }

        #[test]
        fn test_ct_app_long_option_s_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--spaces"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
            assert!(executable.unwrap().contains_id(fold_flags::FOLD_SPACES));
        }

        #[test]
        fn test_ct_app_long_option_w_short_err() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-w"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_err());
        }

        #[test]
        fn test_ct_app_long_option_w_long_err() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--width"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_err());
        }

        #[test]
        fn test_ct_app_long_option_w_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-w", "10"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
            assert!(executable.unwrap().contains_id(fold_flags::FOLD_WIDTH));
        }

        #[test]
        fn test_ct_app_long_option_w_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--width", "10"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
            assert!(executable.unwrap().contains_id(fold_flags::FOLD_WIDTH));
        }
    }

    #[cfg(test)]
    mod handle_obsolete_tests {
        /*
        分支 1：参数以单个连字符（-）开头并后跟一个数字。
            测试用例 1：参数列表中包含一个过时参数（例如，"-1"）。
            测试用例 2：参数列表中包含多个过时参数，确保只处理第一个（例如，"-1", "-2"）。
        分支 2：参数不以单个连字符（-）开头或不后跟数字。
            测试用例 3：参数列表中不包含过时参数（例如，"foo", "bar"）。
            测试用例 4：参数列表中包含以连字符开头但不后跟数字的参数（例如，"-foo"）
        */
        use super::*;
        #[test]
        fn handle_obsolete_with_obsolete_parameter_removes_and_returns_value() {
            let args = vec!["foo".to_string(), "-1".to_string(), "bar".to_string()];
            let (result, obsolete) = handle_obsolete(&args);
            assert_eq!(result, vec!["foo".to_string(), "bar".to_string()]);
            assert_eq!(obsolete, Some("1".to_string()));
        }

        #[test]
        fn handle_obsolete_with_multiple_obsolete_parameters_removes_first_and_returns_value() {
            let args = vec![
                "foo".to_string(),
                "-1".to_string(),
                "-2".to_string(),
                "bar".to_string(),
            ];
            let (result, obsolete) = handle_obsolete(&args);
            assert_eq!(
                result,
                vec!["foo".to_string(), "-2".to_string(), "bar".to_string()]
            );
            assert_eq!(obsolete, Some("1".to_string()));
        }

        #[test]
        fn handle_obsolete_without_obsolete_parameters_returns_original_list_and_none() {
            let args = vec!["foo".to_string(), "bar".to_string()];
            let (result, obsolete) = handle_obsolete(&args);
            assert_eq!(result, vec!["foo".to_string(), "bar".to_string()]);
            assert_eq!(obsolete, None);
        }

        #[test]
        fn handle_obsolete_with_non_numeric_parameter_returns_original_list_and_none() {
            let args = vec!["foo".to_string(), "-foo".to_string(), "bar".to_string()];
            let (result, obsolete) = handle_obsolete(&args);
            assert_eq!(
                result,
                vec!["foo".to_string(), "-foo".to_string(), "bar".to_string()]
            );
            assert_eq!(obsolete, None);
        }

        #[test]
        fn handle_obsolete_with_empty_args_returns_empty_list_and_none() {
            let args: Vec<String> = vec![];
            let (result, obsolete) = handle_obsolete(&args);
            assert_eq!(result, Vec::<String>::new());
            assert_eq!(obsolete, None);
        }

        #[test]
        fn handle_obsolete_with_only_obsolete_parameters_removes_all_and_returns_first_value() {
            let args = vec!["-1".to_string(), "-2".to_string(), "-3".to_string()];
            let (result, obsolete) = handle_obsolete(&args);
            assert_eq!(result, vec!["-2".to_string(), "-3".to_string()]);
            assert_eq!(obsolete, Some("1".to_string()));
        }
    }
}