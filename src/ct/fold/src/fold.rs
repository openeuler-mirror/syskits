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

        #[test]
        fn handle_obsolete_with_obsolete_parameter_at_beginning_removes_and_returns_value() {
            let args = vec!["-1".to_string(), "foo".to_string(), "bar".to_string()];
            let (result, obsolete) = handle_obsolete(&args);
            assert_eq!(result, vec!["foo".to_string(), "bar".to_string()]);
            assert_eq!(obsolete, Some("1".to_string()));
        }

        #[test]
        fn handle_obsolete_with_obsolete_parameter_in_middle_removes_and_returns_value() {
            let args = vec!["foo".to_string(), "-2".to_string(), "bar".to_string()];
            let (result, obsolete) = handle_obsolete(&args);
            assert_eq!(result, vec!["foo".to_string(), "bar".to_string()]);
            assert_eq!(obsolete, Some("2".to_string()));
        }

        #[test]
        fn handle_obsolete_with_obsolete_parameter_at_end_removes_and_returns_value() {
            let args = vec!["foo".to_string(), "bar".to_string(), "-3".to_string()];
            let (result, obsolete) = handle_obsolete(&args);
            assert_eq!(result, vec!["foo".to_string(), "bar".to_string()]);
            assert_eq!(obsolete, Some("3".to_string()));
        }

        #[test]
        fn handle_obsolete_with_double_dash_parameter_does_not_remove() {
            let args = vec!["foo".to_string(), "--2".to_string(), "bar".to_string()];
            let (result, obsolete) = handle_obsolete(&args);
            assert_eq!(
                result,
                vec!["foo".to_string(), "--2".to_string(), "bar".to_string()]
            );
            assert_eq!(obsolete, None);
        }

        #[test]
        fn handle_obsolete_with_negative_number_parameter_removes_and_returns_value() {
            let args = vec!["foo".to_string(), "-123".to_string(), "bar".to_string()];
            let (result, obsolete) = handle_obsolete(&args);
            assert_eq!(result, vec!["foo".to_string(), "bar".to_string()]);
            assert_eq!(obsolete, Some("123".to_string()));
        }

        #[test]
        fn handle_obsolete_with_leading_or_trailing_spaces_does_not_remove() {
            let args = vec!["foo".to_string(), " -1 ".to_string(), "bar".to_string()];
            let (result, obsolete) = handle_obsolete(&args);
            assert_eq!(
                result,
                vec!["foo".to_string(), " -1 ".to_string(), "bar".to_string()]
            );
            assert_eq!(obsolete, None);
        }
    }

    #[cfg(test)]
    mod fold_tests {
        /*
            文件名是 "-"： 测试从标准输入读取内容。
            文件名不是 "-"：测试从指定文件读取内容。
            fold_flags.bytes 为 true：测试按字节进行折叠。
            fold_flags.bytes 为 false：测试按列进行折叠。
            文件读取错误：测试文件不存在或无法读取的情况。
        */
        use super::*;
        use ctcore::ct_error::CTResult;
        use std::io::{BufWriter, Write};
        use tempfile::NamedTempFile;

        /// 写入临时文件的辅助函数
        fn write_temp_file(content: &str) -> NamedTempFile {
            let mut temp_file = NamedTempFile::new().expect("Failed to create temp file");
            write!(temp_file, "{}", content).expect("Failed to write to temp file");
            temp_file
        }

        #[test]
        fn test_fold_single_file_bytewise_no_spaces() -> CTResult<()> {
            // 创建一个临时文件，并写入内容
            let content = "Hello Rust World!";
            let temp_file = write_temp_file(content);

            // 构造 FoldFlags：开启 bytes 模式，不在空格处折行，设定行宽 5
            let fold_flags = FoldFlags {
                bytes: true,
                spaces: false,
                width: 5,
                files: vec![temp_file.path().to_string_lossy().to_string()],
            };

            // 创建一个内存 writer
            let mut writer = Vec::new();

            // 调用 fold 函数
            fold(&mut writer, &fold_flags)?;

            // 读取输出并断言
            let output = String::from_utf8(writer).unwrap();
            assert_eq!(output, "Hello\n Rust\n Worl\nd!");

            Ok(())
        }

        #[test]
        fn test_fold_single_file_bytewise_spaces() -> CTResult<()> {
            // 在空白处断行，当 `spaces=true` 时，折叠时优先寻找空格位置
            let content = "Hello Rust World!";
            let temp_file = write_temp_file(content);

            // 设定行宽较小，观察空格折行效果
            let fold_flags = FoldFlags {
                bytes: true,
                spaces: true,
                width: 6,
                files: vec![temp_file.path().to_string_lossy().to_string()],
            };

            let mut writer = Vec::new();
            fold(&mut writer, &fold_flags)?;

            let output = String::from_utf8(writer).unwrap();
            // 断行逻辑示例（以 6 为上限，优先在空格处分割）：
            // "Hello " -> (空格前就能折行)
            // "Rust "  -> ...
            // "World!"
            assert_eq!(output, "Hello \nRust \nWorld!");

            Ok(())
        }

        #[test]
        fn test_fold_single_file_columnwise_spaces() -> CTResult<()> {
            // 不开启 bytewise，而是使用默认列宽模式
            // （列宽模式会把制表符视为多列、退格符减少列数等，但此处仅测试普通字符+空格折行）
            let content = "Hello Rust World!";
            let temp_file = write_temp_file(content);

            let fold_flags = FoldFlags {
                bytes: false,
                spaces: true,
                width: 6,
                files: vec![temp_file.path().to_string_lossy().to_string()],
            };

            let mut writer = Vec::new();
            fold(&mut writer, &fold_flags)?;

            let output = String::from_utf8(writer).unwrap();
            // 在列模式下，遇到 6 列后会折行。由于 spaces=true，会在最后一次空格处断开
            // 可能结果与 bytewise 类似，但内部对制表符等有不同处理方式
            assert_eq!(output, "Hello \nRust \nWorld!");

            Ok(())
        }

        #[test]
        fn test_fold_single_file_columnwise_no_spaces() -> CTResult<()> {
            let content = "Hello Rust World!";
            let temp_file = write_temp_file(content);

            let fold_flags = FoldFlags {
                bytes: false,
                spaces: false,
                width: 6,
                files: vec![temp_file.path().to_string_lossy().to_string()],
            };

            let mut writer = Vec::new();
            fold(&mut writer, &fold_flags)?;

            let output = String::from_utf8(writer).unwrap();
            // 不在空格处优先断行，直接按列宽截断
            // "Hello " -> 6列
            // "Rust W" -> 6列
            // "orld!"
            // 注意列宽模式下，对 \t 等可能处理不同，但此例中不存在 \t
            assert_eq!(output, "Hello \nRust W\norld!");

            Ok(())
        }

        #[test]
        fn test_fold_multiple_files() -> CTResult<()> {
            let content1 = "File1 content.\nNext line in file1.";
            let content2 = "File2 content.\nNext line in file2.";
            let temp_file1 = write_temp_file(content1);
            let temp_file2 = write_temp_file(content2);

            let fold_flags = FoldFlags {
                bytes: false,
                spaces: false,
                width: 10,
                files: vec![
                    temp_file1.path().to_string_lossy().to_string(),
                    temp_file2.path().to_string_lossy().to_string(),
                ],
            };

            let mut writer = Vec::new();
            fold(&mut writer, &fold_flags)?;

            let output = String::from_utf8(writer).unwrap();
            // 简单断言一下，里面应包含来自 file1 与 file2 的折行结果
            assert!(output.contains("File1"));
            assert!(output.contains("File2"));

            Ok(())
        }

        #[test]
        fn test_fold_file_not_found() {
            // 指定一个不存在的文件路径
            let fold_flags = FoldFlags {
                bytes: false,
                spaces: false,
                width: 10,
                files: vec!["this_file_does_not_exist.xyz".to_owned()],
            };

            let mut writer = BufWriter::new(Vec::new());
            // fold 应该返回错误
            let result = fold(&mut writer, &fold_flags);
            assert!(result.is_err());
        }

        #[test]
        fn fold_files_file_not_found_returns_error() {
            let mut output = Vec::new();
            let fold_flags = FoldFlags {
                files: vec!["nonexistent.txt".to_string()],
                bytes: false,
                spaces: true,
                width: 80,
            };

            // 尝试读取不存在的文件
            let result = fold(&mut output, &fold_flags);

            // 验证错误
            assert!(result.is_err());
        }
    }

    #[cfg(test)]
    mod fold_file_bytewise_tests {
        /*
            空输入：测试输入为空的情况。
            单行输入：测试单行输入，确保其正确处理。
            多行输入：测试多行输入，确保行宽限制被正确应用。
            空行：测试包含空行的输入，确保空行被正确处理。
            行尾空格：测试行尾有空格的情况，确保空格被正确处理。
            行宽限制：测试行宽限制，确保行被正确截断。
            行尾换行符：测试行尾换行符的处理，确保没有重复的换行符。
            特殊测试：
        包含空格分隔符的长行：
            测试了 spaces 参数为 true 时，优先在空格处分割行。
            包含制表符的行：验证了制表符是否被正确处理。
            包含非ASCII字符的行：确保非ASCII字符被正确处理。
            超长单行输入：测试了非常长的单行输入是否按宽度正确折叠。
            不同宽度限制：验证了不同的宽度限制对输出的影响。
            启用 spaces 参数：测试了 spaces 参数为 true 时的行为。
            混合换行符：测试了不同类型的换行符是否被正确处理。
        */

        use super::*;
        use std::io::{BufReader, Cursor};

        #[test]
        fn test_fold_file_bytewise_empty_input() {
            let input = "";
            let mut output = Vec::new();
            let reader = BufReader::new(Cursor::new(input));
            fold_file_bytewise(&mut output, reader, false, 10).unwrap();
            assert_eq!(String::from_utf8(output).unwrap(), "");
        }

        #[test]
        fn test_fold_file_bytewise_single_line_input() {
            let input = "This is a single line.";
            let mut output = Vec::new();
            let reader = BufReader::new(Cursor::new(input));
            fold_file_bytewise(&mut output, reader, false, 10).unwrap();
            assert_eq!(
                String::from_utf8(output).unwrap(),
                "This is a \nsingle lin\ne."
            );
        }

        #[test]
        fn test_fold_file_bytewise_multiple_lines_input() {
            let input = "This is a line.\nThis is another line.";
            let mut output = Vec::new();
            let reader = BufReader::new(Cursor::new(input));
            fold_file_bytewise(&mut output, reader, false, 10).unwrap();
            assert_eq!(
                String::from_utf8(output).unwrap(),
                "This is a \nline.\nThis is an\nother line\n."
            );
        }

        #[test]
        fn test_fold_file_bytewise_empty_lines_input() {
            let input = "\n\n";
            let mut output = Vec::new();
            let reader = BufReader::new(Cursor::new(input));
            fold_file_bytewise(&mut output, reader, false, 10).unwrap();
            assert_eq!(String::from_utf8(output).unwrap(), "\n\n");
        }

        #[test]
        fn test_fold_file_bytewise_line_ends_with_space() {
            let input = "This line ends with a space. ";
            let mut output = Vec::new();
            let reader = BufReader::new(Cursor::new(input));
            fold_file_bytewise(&mut output, reader, false, 10).unwrap();
            assert_eq!(
                String::from_utf8(output).unwrap(),
                "This line \nends with \na space. "
            );
        }

        #[test]
        fn test_fold_file_bytewise_line_width_limit() {
            let input = "This line is too long and should be folded.";
            let mut output = Vec::new();
            let reader = BufReader::new(Cursor::new(input));
            fold_file_bytewise(&mut output, reader, false, 10).unwrap();
            assert_eq!(
                String::from_utf8(output).unwrap(),
                "This line \nis too lon\ng and shou\nld be fold\ned."
            );
        }

        #[test]
        fn test_fold_file_bytewise_line_ends_with_newline() {
            let input = "This line ends with a newline.\n";
            let mut output = Vec::new();
            let reader = BufReader::new(Cursor::new(input));
            fold_file_bytewise(&mut output, reader, false, 10).unwrap();
            assert_eq!(
                String::from_utf8(output).unwrap(),
                "This line \nends with \na newline.\n"
            );
        }

        #[test]
        fn test_fold_file_bytewise_line_with_spaces_for_splitting() {
            let input = "This is a very long line that should be split at spaces.";
            let mut output = Vec::new();
            let reader = BufReader::new(Cursor::new(input));
            fold_file_bytewise(&mut output, reader, true, 10).unwrap();
            assert_eq!(
                String::from_utf8(output).unwrap(),
                "This is a \nvery long \nline that \nshould be \nsplit at \nspaces."
            );
        }

        #[test]
        fn test_fold_file_bytewise_line_with_tabs() {
            let input = "This\tis\ta\tline\twith\ttabs.";
            let mut output = Vec::new();
            let reader = BufReader::new(Cursor::new(input));
            fold_file_bytewise(&mut output, reader, false, 10).unwrap();
            assert_eq!(
                String::from_utf8(output).unwrap(),
                "This\tis\ta\t\nline\twith\t\ntabs."
            );
        }

        #[test]
        fn test_fold_file_bytewise_non_ascii_characters() {
            let input = "你好，这是一个包含非ASCII字符的测试。";
            let mut output = Vec::new();
            let reader = BufReader::new(Cursor::new(input));
            fold_file_bytewise(&mut output, reader, false, 50).unwrap();
            assert_eq!(
                String::from_utf8(output).unwrap(),
                "你好，这是一个包含非ASCII字符的测试\n。"
            );
        }

        #[test]
        fn test_fold_file_bytewise_very_long_single_line() {
            let input = "This is a very very very very very very very very very very very very very very very long line.";
            let mut output = Vec::new();
            let reader = BufReader::new(Cursor::new(input));
            fold_file_bytewise(&mut output, reader, false, 20).unwrap();
            assert_eq!(
                String::from_utf8(output).unwrap(),
                "This is a very very \nvery very very very \nvery very very very \nvery very very very \nvery long line."
            );
        }

        #[test]
        fn test_fold_file_bytewise_different_width_limits() {
            let input = "This is a line.";
            let mut output = Vec::new();
            let reader = BufReader::new(Cursor::new(input));
            fold_file_bytewise(&mut output, reader, false, 5).unwrap();
            assert_eq!(String::from_utf8(output).unwrap(), "This \nis a \nline.");
        }

        #[test]
        fn test_fold_file_bytewise_spaces_enabled() {
            let input = "This is a very long line that should be split at spaces.";
            let mut output = Vec::new();
            let reader = BufReader::new(Cursor::new(input));
            fold_file_bytewise(&mut output, reader, true, 10).unwrap();
            assert_eq!(
                String::from_utf8(output).unwrap(),
                "This is a \nvery long \nline that \nshould be \nsplit at \nspaces."
            );
        }

        #[test]
        fn test_fold_file_bytewise_mixed_line_endings() {
            let input = "Line with CRLF\r\nLine with LF\n";
            let mut output = Vec::new();
            let reader = BufReader::new(Cursor::new(input));
            fold_file_bytewise(&mut output, reader, false, 10).unwrap();
            assert_eq!(
                String::from_utf8(output).unwrap(),
                "Line with \nCRLF\r\nLine with \nLF\n"
            );
        }
    }

    #[cfg(test)]
    mod emit_output_tests {
        /*
        通用测试用例：
            验证 output 是否正确更新，移除了已写入的部分。
            验证 col_count 是否正确更新为 output 中剩余字符的数量。
            验证 last_space 是否被重置为 None。
        边界情况：
            output 为空字符串。
            last_space 的值大于 output 的长度。
            output 只有一个字符
        */
        use super::*;
        use std::io::Cursor;
        #[test]
        fn test_emit_output_last_space_is_some_writes_up_to_last_space() {
            let mut writer = Cursor::new(Vec::new());
            let mut output = String::from("Hello World");
            let mut last_space = Some(5);
            let mut col_count = 0;

            emit_output(&mut writer, &mut output, &mut last_space, &mut col_count).unwrap();

            assert_eq!(writer.into_inner(), b"Hello \n");
            assert_eq!(output, "World");
            assert_eq!(col_count, 5);
            assert_eq!(last_space, None);
        }

        #[test]
        fn test_emit_output_last_space_is_none_writes_whole_output() {
            let mut writer = Cursor::new(Vec::new());
            let mut output = String::from("Hello World");
            let mut last_space = None;
            let mut col_count = 0;

            emit_output(&mut writer, &mut output, &mut last_space, &mut col_count).unwrap();

            assert_eq!(writer.into_inner(), b"Hello World\n");
            assert_eq!(output, "");
            assert_eq!(col_count, 0);
            assert_eq!(last_space, None);
        }

        #[test]
        fn test_emit_output_output_is_empty() {
            let mut writer = Cursor::new(Vec::new());
            let mut output = String::from("");
            let mut last_space = Some(5);
            let mut col_count = 0;

            emit_output(&mut writer, &mut output, &mut last_space, &mut col_count).unwrap();
            assert_eq!(writer.into_inner(), b"\n");
            assert_eq!(output, "");
            assert_eq!(col_count, 0);
            assert_eq!(last_space, None);
        }

        #[test]
        fn test_emit_output_last_space_exceeds_output_length() {
            let mut writer = Cursor::new(Vec::new());
            let mut output = String::from("Hello");
            let mut last_space = Some(10);
            let mut col_count = 0;

            emit_output(&mut writer, &mut output, &mut last_space, &mut col_count).unwrap();

            assert_eq!(writer.into_inner(), b"Hello\n");
            assert_eq!(output, "");
            assert_eq!(col_count, 0);
            assert_eq!(last_space, None);
        }

        #[test]
        fn test_emit_output_output_has_one_character() {
            let mut writer = Cursor::new(Vec::new());
            let mut output = String::from("A");
            let mut last_space = Some(0);
            let mut col_count = 0;

            emit_output(&mut writer, &mut output, &mut last_space, &mut col_count).unwrap();

            assert_eq!(writer.into_inner(), b"A\n");
            assert_eq!(output, "");
            assert_eq!(col_count, 0);
            assert_eq!(last_space, None);
        }

        #[test]
        fn test_emit_output_last_space_is_zero() {
            let mut writer = Cursor::new(Vec::new());
            let mut output = String::from("Hello World");
            let mut last_space = Some(0);
            let mut col_count = 0;

            emit_output(&mut writer, &mut output, &mut last_space, &mut col_count).unwrap();

            assert_eq!(writer.into_inner(), b"H\n");
            assert_eq!(output, "ello World");
            assert_eq!(col_count, 10);
            assert_eq!(last_space, None);
        }
    }
    #[cfg(test)]
    mod fold_file_tests {
        /*
        空输入：测试输入为空的情况。
        单行输入：测试单行输入，确保它被正确格式化。
        多行输入：测试多行输入，确保每行都被正确处理。
        换行符处理：测试包含换行符的输入，确保它们被正确处理。
        制表符处理：测试包含制表符的输入，确保它们被正确转换。
        退格符处理：测试包含退格符的输入，确保它们被正确处理。
        空格保留：测试 spaces 标志设置为 true 和 false 的情况。
        行宽处理：测试不同宽度的输入，确保行被正确换行
        */
        use std::io::BufReader;

        use super::*;

        #[test]
        fn fold_file_empty_input_no_output() {
            let input = "";
            let reader = BufReader::new(input.as_bytes());
            let mut writer = Vec::new();

            fold_file(&mut writer, reader, false, 10).unwrap();
            let output_str = String::from_utf8(writer).unwrap();
            assert_eq!(output_str, "");
        }

        #[test]
        fn fold_file_single_line_input_no_wrap() {
            let input = "This is a single line.";

            let reader = BufReader::new(input.as_bytes());
            let mut writer = Vec::new();

            fold_file(&mut writer, reader, false, 22).unwrap();
            let output_str = String::from_utf8(writer).unwrap();
            assert_eq!(output_str, "This is a single line.");
        }

        #[test]
        fn fold_file_multiple_lines_wrap_correctly() {
            let input = "This is line one.\nThis is line two.\n";

            let reader = BufReader::new(input.as_bytes());
            let mut writer = Vec::new();

            fold_file(&mut writer, reader, false, 17).unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            assert_eq!(output_str, "This is line one.\nThis is line two.\n");
        }

        #[test]
        fn fold_file_newline_handling_correct_output() {
            let input = "Line one\nLine two\n";
            let reader = BufReader::new(input.as_bytes());
            let mut writer = Vec::new();

            fold_file(&mut writer, reader, false, 10).unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            assert_eq!(output_str, "Line one\nLine two\n");
        }

        #[test]
        fn fold_file_tab_handling_correct_output() {
            let input = "Line\twith\ttabs";

            let reader = BufReader::new(input.as_bytes());
            let mut writer = Vec::new();

            fold_file(&mut writer, reader, false, 20).unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            assert_eq!(output_str, "Line\twith\ttabs");
        }

        #[test]
        fn fold_file_backspace_handling_correct_output() {
            let input = "Line\\bBackspace";

            let reader = BufReader::new(input.as_bytes());
            let mut writer = Vec::new();

            fold_file(&mut writer, reader, false, 10).unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            assert_eq!(output_str, "Line\\bBack\nspace");
        }

        #[test]
        fn fold_file_space_handling_with_spaces() {
            let input = "Line with spaces";

            let reader = BufReader::new(input.as_bytes());
            let mut writer = Vec::new();

            fold_file(&mut writer, reader, true, 10).unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            assert_eq!(output_str, "Line with \nspaces");
        }

        #[test]
        fn fold_file_line_width_handling_correct_wrap() {
            let input = "This line is too long and should be wrapped.";

            let reader = BufReader::new(input.as_bytes());
            let mut writer = Vec::new();

            fold_file(&mut writer, reader, false, 10).unwrap();

            let output_str = String::from_utf8(writer).unwrap();
            assert_eq!(
                output_str,
                "This line \nis too lon\ng and shou\nld be wrap\nped."
            );
        }
    }
}