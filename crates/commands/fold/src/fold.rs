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

//! 对每个指定的文件设置自动换行（折行），并将重新排版后的结果输出到标准输出。

extern crate rust_i18n;
use clap::error::ErrorKind;
use clap::{Arg, ArgAction, Command, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "zh-CN");
use ctcore::Tool;
use ctcore::ct_error::{CTResult, FromIo, set_ct_exit_code};
use std::ffi::OsString;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write, stdin};
use std::path::Path;
use sys_locale::get_locale;
const FOLD_TAB_WIDTH: usize = 8;

mod fold_flags {
    pub const FOLD_BYTES: &str = "bytes";
    pub const FOLD_CHARACTERS: &str = "characters";
    pub const FOLD_SPACES: &str = "spaces";
    pub const FOLD_WIDTH: &str = "width";
    pub const FOLD_FILE: &str = "file";
}

struct FoldFlags {
    bytes: bool,
    characters: bool,
    spaces: bool,
    width: usize,
    files: Vec<String>,
}

/// 主折叠函数，用于处理命令行参数并输出结果
///
/// # Parameters
///
/// - `writer`: 一个实现了Write trait的可变引用，用于输出结果
/// - `args`: 命令行参数的切片
///
/// # Returns
///
/// 返回一个Result，表示操作成功或失败
pub fn fold_main<W: Write>(writer: &mut W, args: impl ctcore::Args) -> CTResult<()> {
    // 设置语言
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    // 将OsString参数转换为String
    let string_args: Vec<String> = args.collect_lossy();

    let (args, obs_width) = handle_obsolete(&string_args[..]);
    let matches = match ct_app().try_get_matches_from(args) {
        Ok(m) => m,
        Err(e) => {
            if e.kind() == ErrorKind::ArgumentConflict {
                set_ct_exit_code(2); // 检查是否是参数冲突错误，如果是则返回退出码2
                return Ok(());
            }
            return Err(e.into());
        }
    };

    let flags = FoldFlags {
        bytes: matches.get_flag(fold_flags::FOLD_BYTES),
        characters: matches.get_flag(fold_flags::FOLD_CHARACTERS),
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
    let application_info = t!("fold.about");
    let usage_description = t!("fold.usage");
    let args = vec![
        Arg::new(fold_flags::FOLD_BYTES)
            .long(fold_flags::FOLD_BYTES)
            .short('b')
            .help(
                "count using bytes rather than columns (meaning control characters \
                     such as newline are not treated specially)",
            )
            .action(ArgAction::SetTrue)
            .conflicts_with(fold_flags::FOLD_CHARACTERS),
        Arg::new(fold_flags::FOLD_CHARACTERS)
            .long(fold_flags::FOLD_CHARACTERS)
            .short('c')
            .help(t!("fold.clap.fold_characters"))
            .action(ArgAction::SetTrue)
            .conflicts_with(fold_flags::FOLD_BYTES),
        Arg::new(fold_flags::FOLD_SPACES)
            .long(fold_flags::FOLD_SPACES)
            .short('s')
            .help(t!("fold.clap.fold_spaces"))
            .action(ArgAction::SetTrue),
        Arg::new(fold_flags::FOLD_WIDTH)
            .long(fold_flags::FOLD_WIDTH)
            .short('w')
            .help(t!("fold.clap.fold_width"))
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
            match File::open(Path::new(filename)) {
                Ok(f) => {
                    file_buf = f;
                    &mut file_buf as &mut dyn Read
                }
                Err(e) => {
                    let error_msg = match e.kind() {
                        std::io::ErrorKind::NotFound => "No such file or directory".to_string(),
                        std::io::ErrorKind::PermissionDenied => "Permission denied".to_string(),
                        _ => e.to_string(),
                    };
                    eprintln!("fold: {}: {}", filename, error_msg);
                    continue;
                }
            }
        });

        let spaces = fold_flags.spaces;
        let width = fold_flags.width;
        if fold_flags.bytes {
            // 如果`bytes`标志为真，则按字节进行折叠
            fold_file_bytewise(writer, buffer, spaces, width)?;
        } else if fold_flags.characters {
            // 如果`characters`标志为真，则按字符进行折叠
            fold_file_characterwise(writer, buffer, spaces, width)?;
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

        let bytes = line.as_bytes();
        let len = bytes.len();
        let mut i = 0;

        while i < len {
            let mut end_byte = (i + width).min(len);

            // 确保我们不在UTF-8字符中间断开
            // UTF-8字节的特征：
            // - 单字节字符：0xxxxxxx (0-127)
            // - 多字节字符的后续字节：10xxxxxx (128-191)
            // - 多字节字符的首字节：11xxxxxx (192-255)
            while end_byte < len && end_byte > i {
                let byte = bytes[end_byte];
                if byte & 0x80 == 0 || byte & 0xC0 != 0x80 {
                    // 这是ASCII字符或UTF-8多字节字符的首字节，可以安全断开
                    break;
                }
                // 这是UTF-8多字节字符的续字节，向前移动
                end_byte -= 1;
            }

            // 如果无法找到有效的断点，强制输出至少一个字符以避免无限循环
            if end_byte == i {
                // 找到下一个字符的边界
                end_byte = i + 1;
                while end_byte < len && (bytes[end_byte] & 0xC0) == 0x80 {
                    end_byte += 1;
                }
            }

            let mut actual_end = end_byte;

            // 如果启用了spaces选项，尝试在最后一个空白字符处断行
            if is_spaces && end_byte < len {
                // 从end_byte往前找最后一个空白字符
                for j in (i..end_byte).rev() {
                    let byte = bytes[j];
                    // ASCII空白字符：空格(32), 制表符(9), 换行符(10), 回车符(13)
                    if byte == b' ' || byte == b'\t' || byte == b'\n' || byte == b'\r' {
                        actual_end = j + 1;
                        break;
                    }
                }
            }

            let slice_bytes = &bytes[i..actual_end];

            // 不重复换行符：如果子字符串是 "\n"，则上一次迭代已经在行尾折叠并打印了该换行符。
            if slice_bytes == b"\n" {
                break;
            }

            i = actual_end;
            let at_eol = i >= len;

            // 输出字节切片，现在应该是有效的UTF-8
            if at_eol {
                writer.write_all(slice_bytes)?;
            } else {
                writer.write_all(slice_bytes)?;
                writeln!(writer)?;
            }
        }

        line.truncate(0);
    }

    Ok(())
}

/// 按字符折叠文件内容
///
/// 该函数按字符计数进行折叠，与字节模式不同，这里一个 Unicode 字符视为一个计数单位。
/// 例如，对于多字节UTF-8字符如中文字符，按字符模式计数时只算作1个字符，而不是多个字节。
///
/// # 参数
///
/// - `writer`: 输出写入器
/// - `file`: 输入文件读取器
/// - `is_spaces`: 是否在空格处优先换行
/// - `width`: 每行最大字符数
///
/// # 返回值
///
/// - 如果折叠成功，返回`Ok(())`；如果发生错误，返回`Err`。
fn fold_file_characterwise<T: Read, W: Write>(
    writer: &mut W,
    mut file: BufReader<T>,
    is_spaces: bool,
    width: usize,
) -> CTResult<()> {
    let mut line = String::new();
    let mut first_line = true;

    loop {
        if file
            .read_line(&mut line)
            .map_err_context(|| "failed to read line".to_string())?
            == 0
        {
            break;
        }

        // 从第二行开始，在处理前添加空行
        if !first_line {
            writeln!(writer)?;
        }
        first_line = false;

        if line == "\n" {
            writeln!(writer)?;
            line.truncate(0);
            continue;
        }

        let chars: Vec<char> = line.chars().collect();
        let len = chars.len();
        let mut i = 0;

        while i < len {
            let end_idx = if len - i >= width { i + width } else { len };
            let slice_chars = &chars[i..end_idx];

            let actual_slice = if is_spaces && end_idx < len {
                // 查找最后一个空白字符的位置
                if let Some(last_space_pos) = slice_chars
                    .iter()
                    .rposition(|&c| c.is_whitespace() && c != '\r')
                {
                    &slice_chars[..=last_space_pos]
                } else {
                    slice_chars
                }
            } else {
                slice_chars
            };

            let slice_str: String = actual_slice.iter().collect();

            // 不重复换行符：如果子字符串是 "\n"，则上一次迭代已经在行尾折叠并打印了该换行符。
            if slice_str == "\n" {
                break;
            }

            i += actual_slice.len();
            let at_eol = i >= len;

            if at_eol {
                write!(writer, "{slice_str}")?;
            } else {
                writeln!(writer, "{slice_str}")?;
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

#[derive(Default)]
pub struct Fold;
impl Tool for Fold {
    fn name(&self) -> &'static str {
        "fold"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        // 调用原有的 fold_main 函数，传入 stdout 作为 writer
        let mut stdout = std::io::stdout();
        fold_main(&mut stdout, args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    // 新增：测试 Tool trait 的基本实现
    #[test]
    fn test_tool_implementation() {
        let tool = Fold::default();

        // 测试 name 方法
        assert_eq!(tool.name(), "fold");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("fold"));

        // 测试 execute 方法
        let args = vec![OsString::from("fold"), OsString::from("--version")];
        assert!(tool.execute(&args).is_err());
    }

    #[cfg(test)]
    mod fold_main_tests {
        use super::*;
        use std::ffi::OsString;
        use std::fs::File;
        use tempfile::tempdir;

        #[test]
        fn test_ctmain_version() {
            let mut writer = Vec::new();
            let args = vec![
                OsString::from(ctcore::ct_util_name()),
                OsString::from("--version"),
            ];
            let result = fold_main(&mut writer, args.iter().cloned());
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
            // 使用临时文件而不是标准输入来避免阻塞
            let content = "test content for width option";
            let temp_file = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(temp_file.path(), content).unwrap();

            let args = vec![
                "fold".to_string(),
                "--width".to_string(),
                "10".to_string(),
                temp_file.path().to_string_lossy().to_string(),
            ];

            let mut writer = Vec::new();
            let result = fold_main(
                &mut writer,
                args.iter().map(|s| std::ffi::OsString::from(s)),
            );

            assert!(result.is_ok());
            let output = String::from_utf8(writer).unwrap();
            assert!(output.contains("test"));
        }

        #[test]
        fn test_ct_main_long_option_c_short() {
            // 使用临时文件而不是标准输入来避免阻塞
            let content = "测试字符abc";
            let temp_file = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(temp_file.path(), content).unwrap();

            let args = vec![
                "fold".to_string(),
                "-c".to_string(),
                temp_file.path().to_string_lossy().to_string(),
            ];

            let mut writer = Vec::new();
            let result = fold_main(
                &mut writer,
                args.iter().map(|s| std::ffi::OsString::from(s)),
            );

            assert!(result.is_ok());
            let output = String::from_utf8(writer).unwrap();
            assert!(output.contains("测试"));
        }

        #[test]
        fn test_ct_main_long_option_c_long() {
            // 使用临时文件而不是标准输入来避免阻塞
            let content = "测试字符abc";
            let temp_file = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(temp_file.path(), content).unwrap();

            let args = vec![
                "fold".to_string(),
                "--characters".to_string(),
                temp_file.path().to_string_lossy().to_string(),
            ];

            let mut writer = Vec::new();
            let result = fold_main(
                &mut writer,
                args.iter().map(|s| std::ffi::OsString::from(s)),
            );

            assert!(result.is_ok());
            let output = String::from_utf8(writer).unwrap();
            assert!(output.contains("测试"));
        }

        #[test]
        fn test_ct_main_mutually_exclusive_b_c_exit_code() {
            // 测试互斥选项-b和-c返回退出码2
            use ctcore::ct_error::{get_ct_exit_code, set_ct_exit_code};

            // 重置退出码
            set_ct_exit_code(0);

            let mut writer = Vec::new();
            let args = vec![ctcore::ct_util_name(), "-b", "-c", "/etc/passwd"];
            let result = fold_main(&mut writer, args.iter().map(|s| OsString::from(s)));

            // 应该返回Ok()，因为我们设置了exit_code并返回Ok
            assert!(result.is_ok());
            // 检查退出码应该是2
            assert_eq!(get_ct_exit_code(), 2);

            // 重置退出码
            set_ct_exit_code(0);
        }

        #[test]
        fn test_ct_main_mutually_exclusive_c_b_exit_code() {
            // 测试互斥选项-c和-b返回退出码2（顺序相反）
            use ctcore::ct_error::{get_ct_exit_code, set_ct_exit_code};

            // 重置退出码
            set_ct_exit_code(0);

            let mut writer = Vec::new();
            let args = vec![ctcore::ct_util_name(), "-c", "-b", "/etc/passwd"];
            let result = fold_main(&mut writer, args.iter().map(|s| OsString::from(s)));

            // 应该返回Ok()，因为我们设置了exit_code并返回Ok
            assert!(result.is_ok());
            // 检查退出码应该是2
            assert_eq!(get_ct_exit_code(), 2);

            // 重置退出码
            set_ct_exit_code(0);
        }

        #[test]
        fn test_ct_main_characters_functionality() {
            // 测试-c选项的字符计数功能
            let mut writer = Vec::new();

            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let temp_file_path = temp_dir.path().join("fold_temp_utf8.txt");
            let mut temp_file =
                File::create(&temp_file_path).expect("Failed to create temporary file");
            // 写入包含UTF-8字符的内容，确保字符计数正确
            temp_file
                .write_all("这是中文字符测试abcd".as_bytes())
                .expect("Failed to write to temporary file");
            let binding = temp_file_path.to_string_lossy().into_owned();

            // 测试字符模式 -c，设置宽度为5字符
            let args = vec![ctcore::ct_util_name(), "-c", "-w", "5", &binding];
            let result = fold_main(&mut writer, args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());

            let output = String::from_utf8(writer).unwrap();
            // 验证输出包含折行
            assert!(!output.is_empty());
            // 验证输出包含中文字符
            assert!(output.contains("这是中文"));
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

        #[test]
        fn test_ct_app_long_option_c_short() {
            let command = ct_app();
            let args = vec!["fold", "-c"];
            let matches = command.try_get_matches_from(args).unwrap();
            assert!(matches.get_flag("characters"));
        }

        #[test]
        fn test_ct_app_long_option_c_long() {
            let command = ct_app();
            let args = vec!["fold", "--characters"];
            let matches = command.try_get_matches_from(args).unwrap();
            assert!(matches.get_flag("characters"));
        }

        #[test]
        fn test_ct_app_combined_options_c_s() {
            let command = ct_app();
            let args = vec!["fold", "-c", "-s", "-w", "20"];
            let matches = command.try_get_matches_from(args).unwrap();
            assert!(matches.get_flag("characters"));
            assert!(matches.get_flag("spaces"));
            assert_eq!(matches.get_one::<String>("width").unwrap(), "20");
        }

        #[test]
        fn test_ct_app_mutually_exclusive_b_c() {
            // 测试-b和-c选项互斥，应该返回错误
            let command = ct_app();
            let args = vec!["fold", "-b", "-c"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            if let Err(e) = result {
                assert_eq!(e.kind(), clap::error::ErrorKind::ArgumentConflict);
            }
        }

        #[test]
        fn test_ct_app_mutually_exclusive_c_b() {
            // 测试-c和-b选项互斥，应该返回错误（顺序相反）
            let command = ct_app();
            let args = vec!["fold", "-c", "-b"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_err());
            if let Err(e) = result {
                assert_eq!(e.kind(), clap::error::ErrorKind::ArgumentConflict);
            }
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
        fn test_fold_nonexistent_file_with_width() {
            let fold_flags = FoldFlags {
                bytes: false,
                characters: false,
                spaces: false,
                width: 20,
                files: vec!["nonexistent_file.txt".to_string()],
            };

            let mut writer = Vec::new();
            let result = fold(&mut writer, &fold_flags);

            // 验证执行成功（因为错误被处理了）
            assert!(result.is_ok());

            // 验证输出为空（因为文件不存在）
            assert_eq!(String::from_utf8(writer).unwrap(), "");
        }

        #[test]
        fn test_fold_existing_file_with_width() -> CTResult<()> {
            // 创建一个临时文件并写入测试内容
            let content = "This is a test file with content that should be folded at width 20.\nThis is another line that should also be folded properly.";
            let temp_file = write_temp_file(content);

            let fold_flags = FoldFlags {
                bytes: false,
                characters: false,
                spaces: false,
                width: 20,
                files: vec![temp_file.path().to_string_lossy().to_string()],
            };

            let mut writer = Vec::new();
            fold(&mut writer, &fold_flags)?;

            let output = String::from_utf8(writer).unwrap();
            println!("output: {}", output);
            // 验证输出是否按照20个字符宽度正确折行
            assert!(output.contains("This is a test file"));
            assert!(output.contains("with content that sh"));
            assert!(output.contains("ould be folded at wi"));
            assert!(output.contains("dth 20."));
            assert!(output.contains("This is another line"));
            assert!(output.contains(" that should also be"));
            assert!(output.contains(" folded properly."));

            Ok(())
        }

        #[test]
        fn test_fold_single_file_bytewise_no_spaces() -> CTResult<()> {
            // 创建一个临时文件，并写入内容
            let content = "Hello Rust World!";
            let temp_file = write_temp_file(content);

            // 构造 FoldFlags：开启 bytes 模式，不在空格处折行，设定行宽 5
            let fold_flags = FoldFlags {
                bytes: true,
                characters: false,
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
                characters: false,
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
                characters: false,
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
                characters: false,
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
                characters: false,
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
                characters: false,
                spaces: false,
                width: 10,
                files: vec!["this_file_does_not_exist.xyz".to_owned()],
            };

            let mut writer = BufWriter::new(Vec::new());
            // fold 应该返回错误
            let result = fold(&mut writer, &fold_flags);
            assert!(result.is_ok());
        }

        #[test]
        fn fold_files_file_not_found_returns_ok() {
            let mut output = Vec::new();
            let fold_flags = FoldFlags {
                files: vec!["nonexistent.txt".to_string()],
                bytes: false,
                characters: false,
                spaces: true,
                width: 80,
            };

            // 尝试读取不存在的文件
            let result = fold(&mut output, &fold_flags);

            // 验证错误
            assert!(result.is_ok());
        }

        #[test]
        fn test_fold_single_file_characterwise_no_spaces() -> CTResult<()> {
            // 测试字符计数模式：中文字符每个算作1个字符
            let content = "这是测试中文字符串abc";
            let temp_file = write_temp_file(content);

            let fold_flags = FoldFlags {
                bytes: false,
                characters: true,
                spaces: false,
                width: 8,
                files: vec![temp_file.path().to_string_lossy().to_string()],
            };

            let mut writer = Vec::new();
            fold(&mut writer, &fold_flags)?;

            let output = String::from_utf8(writer).unwrap();
            // 按字符计数：前8个字符是"这是测试中文字符"，剩下"串abc"
            assert_eq!(output, "这是测试中文字符\n串abc");

            Ok(())
        }

        #[test]
        fn test_fold_single_file_characterwise_spaces() -> CTResult<()> {
            // 测试字符计数模式配合空格断行
            let content = "这是 测试 中文字符 串";
            let temp_file = write_temp_file(content);

            let fold_flags = FoldFlags {
                bytes: false,
                characters: true,
                spaces: true,
                width: 6,
                files: vec![temp_file.path().to_string_lossy().to_string()],
            };

            let mut writer = Vec::new();
            fold(&mut writer, &fold_flags)?;

            let output = String::from_utf8(writer).unwrap();
            // 按字符计数且在空格处断行：
            // "这是 测试 " 中，"这是 " 3个字符， "测试 " 3个字符，"中文字符 " 5个字符，"串" 1个字符
            // 实际会是"这是 测试 "，"中文字符 串"
            assert_eq!(output, "这是 测试 \n中文字符 串");

            Ok(())
        }

        #[test]
        fn test_fold_bytewise_utf8_character_boundaries() -> CTResult<()> {
            // 测试字节模式正确处理UTF-8字符边界
            let content = "中文测试abc";
            let temp_file = write_temp_file(content);

            let fold_flags = FoldFlags {
                bytes: true,
                characters: false,
                spaces: false,
                width: 10, // 10字节：每个中文字符3字节，"中文测"=9字节，"试"会在下一行
                files: vec![temp_file.path().to_string_lossy().to_string()],
            };

            let mut writer = Vec::new();
            fold(&mut writer, &fold_flags)?;

            let output = String::from_utf8(writer).unwrap();
            // 字节模式但保证UTF-8字符完整性
            assert_eq!(output, "中文测\n试abc");

            Ok(())
        }

        #[test]
        fn test_fold_mixed_ascii_utf8_characters() -> CTResult<()> {
            // 测试混合ASCII和UTF-8字符的字符计数模式
            let content = "Hello世界123测试";
            let temp_file = write_temp_file(content);

            let fold_flags = FoldFlags {
                bytes: false,
                characters: true,
                spaces: false,
                width: 8,
                files: vec![temp_file.path().to_string_lossy().to_string()],
            };

            let mut writer = Vec::new();
            fold(&mut writer, &fold_flags)?;

            let output = String::from_utf8(writer).unwrap();
            // 按字符计数：前8个字符"Hello世界1"，剩下"23测试"
            assert_eq!(output, "Hello世界1\n23测试");

            Ok(())
        }

        #[test]
        fn test_fold_characterwise_vs_bytewise_difference() -> CTResult<()> {
            // 演示字符模式和字节模式的区别
            let content = "测试中文abc";
            let temp_file1 = write_temp_file(content);
            let temp_file2 = write_temp_file(content);

            // 字符模式
            let char_flags = FoldFlags {
                bytes: false,
                characters: true,
                spaces: false,
                width: 6,
                files: vec![temp_file1.path().to_string_lossy().to_string()],
            };

            let mut char_writer = Vec::new();
            fold(&mut char_writer, &char_flags)?;
            let char_output = String::from_utf8(char_writer).unwrap();

            // 字节模式
            let byte_flags = FoldFlags {
                bytes: true,
                characters: false,
                spaces: false,
                width: 6,
                files: vec![temp_file2.path().to_string_lossy().to_string()],
            };

            let mut byte_writer = Vec::new();
            fold(&mut byte_writer, &byte_flags)?;
            let byte_output = String::from_utf8(byte_writer).unwrap();

            // 字符模式：6个字符 "测试中文ab"，剩下"c"
            assert_eq!(char_output, "测试中文ab\nc");

            // 字节模式：6字节只能容纳2个中文字符"测试"(6字节)，剩下"中文abc"
            assert_eq!(byte_output, "测试\n中文\nabc");

            Ok(())
        }

        #[test]
        fn test_fold_characters_vs_bytes_vs_columns() -> CTResult<()> {
            // 综合测试三种模式的区别：字符模式、字节模式、列模式
            let content = "中文测试abc";
            let temp_file = write_temp_file(content);

            // 字符模式
            let fold_flags_chars = FoldFlags {
                bytes: false,
                characters: true,
                spaces: false,
                width: 5,
                files: vec![temp_file.path().to_string_lossy().to_string()],
            };

            // 字节模式
            let fold_flags_bytes = FoldFlags {
                bytes: true,
                characters: false,
                spaces: false,
                width: 5,
                files: vec![temp_file.path().to_string_lossy().to_string()],
            };

            // 列模式（默认）
            let fold_flags_cols = FoldFlags {
                bytes: false,
                characters: false,
                spaces: false,
                width: 5,
                files: vec![temp_file.path().to_string_lossy().to_string()],
            };

            let mut writer_chars = Vec::new();
            let mut writer_bytes = Vec::new();
            let mut writer_cols = Vec::new();

            fold(&mut writer_chars, &fold_flags_chars)?;
            fold(&mut writer_bytes, &fold_flags_bytes)?;
            fold(&mut writer_cols, &fold_flags_cols)?;

            let output_chars = String::from_utf8(writer_chars).unwrap();
            let output_bytes = String::from_utf8(writer_bytes).unwrap();
            let output_cols = String::from_utf8(writer_cols).unwrap();

            // 验证三种输出都不为空
            assert!(!output_chars.is_empty());
            assert!(!output_bytes.is_empty());
            assert!(!output_cols.is_empty());

            // 验证字符模式：按字符计数，应该包含 "中文测试a" 在第一行
            assert!(output_chars.contains("中文测试a"));

            // 验证字节模式：由于UTF-8边界保护，每个中文字符单独成行
            assert!(output_bytes.contains("中") && output_bytes.contains("文"));
            assert!(output_bytes.contains("测") && output_bytes.contains("试"));

            // 验证列模式：与字符模式相同（对于中文字符）
            assert!(output_cols.contains("中文测试a"));

            Ok(())
        }

        #[test]
        fn test_fold_characters_exact_width() -> CTResult<()> {
            // 测试字符模式的精确宽度控制
            let content = "12345678901234567890"; // 20个ASCII字符
            let temp_file = write_temp_file(content);

            let fold_flags = FoldFlags {
                bytes: false,
                characters: true,
                spaces: false,
                width: 10,
                files: vec![temp_file.path().to_string_lossy().to_string()],
            };

            let mut writer = Vec::new();
            fold(&mut writer, &fold_flags)?;

            let output = String::from_utf8(writer).unwrap();
            let lines: Vec<&str> = output.lines().collect();

            // 应该有2行，每行10个字符
            assert_eq!(lines.len(), 2);
            assert_eq!(lines[0], "1234567890");
            assert_eq!(lines[1], "1234567890");

            Ok(())
        }

        #[test]
        fn test_fold_characters_with_emoji() -> CTResult<()> {
            // 测试字符模式对emoji的处理
            let content = "😀😃😄😁😆😅"; // 6个emoji字符
            let temp_file = write_temp_file(content);

            let fold_flags = FoldFlags {
                bytes: false,
                characters: true,
                spaces: false,
                width: 3,
                files: vec![temp_file.path().to_string_lossy().to_string()],
            };

            let mut writer = Vec::new();
            fold(&mut writer, &fold_flags)?;

            let output = String::from_utf8(writer).unwrap();
            let lines: Vec<&str> = output.lines().collect();

            // 应该有2行，每行3个emoji字符
            assert_eq!(lines.len(), 2);
            assert_eq!(lines[0].chars().count(), 3);
            assert_eq!(lines[1].chars().count(), 3);

            Ok(())
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

    #[cfg(test)]
    mod fold_file_characterwise_tests {
        use super::*;
        use std::io::{BufReader, Cursor};

        #[test]
        fn test_fold_file_characterwise_empty_input() {
            let input = "";
            let mut output = Vec::new();
            let reader = BufReader::new(Cursor::new(input));
            fold_file_characterwise(&mut output, reader, false, 10).unwrap();
            assert_eq!(String::from_utf8(output).unwrap(), "");
        }

        #[test]
        fn test_fold_file_characterwise_ascii_only() {
            let input = "This is a test line with ASCII characters only.";
            let mut output = Vec::new();
            let reader = BufReader::new(Cursor::new(input));
            fold_file_characterwise(&mut output, reader, false, 10).unwrap();
            assert_eq!(
                String::from_utf8(output).unwrap(),
                "This is a \ntest line \nwith ASCII\n character\ns only."
            );
        }

        #[test]
        fn test_fold_file_characterwise_utf8_characters() {
            let input = "这是一个测试中文字符的例子。";
            let mut output = Vec::new();
            let reader = BufReader::new(Cursor::new(input));
            fold_file_characterwise(&mut output, reader, false, 8).unwrap();
            assert_eq!(
                String::from_utf8(output).unwrap(),
                "这是一个测试中文\n字符的例子。"
            );
        }

        #[test]
        fn test_fold_file_characterwise_mixed_characters() {
            let input = "Hello世界abc测试123";
            let mut output = Vec::new();
            let reader = BufReader::new(Cursor::new(input));
            fold_file_characterwise(&mut output, reader, false, 6).unwrap();
            assert_eq!(
                String::from_utf8(output).unwrap(),
                "Hello世\n界abc测试\n123"
            );
        }

        #[test]
        fn test_fold_file_characterwise_with_spaces() {
            let input = "这是 一个 测试 中文 字符串";
            let mut output = Vec::new();
            let reader = BufReader::new(Cursor::new(input));
            fold_file_characterwise(&mut output, reader, true, 6).unwrap();
            assert_eq!(
                String::from_utf8(output).unwrap(),
                "这是 一个 \n测试 中文 \n字符串"
            );
        }

        #[test]
        fn test_fold_file_characterwise_newlines() {
            let input = "第一行\n第二行很长需要被折行\n第三行";
            let mut output = Vec::new();
            let reader = BufReader::new(Cursor::new(input));
            fold_file_characterwise(&mut output, reader, false, 6).unwrap();
            assert_eq!(
                String::from_utf8(output).unwrap(),
                "第一行\n\n第二行很长需\n要被折行\n\n第三行"
            );
        }

        #[test]
        fn test_fold_file_characterwise_emoji() {
            let input = "Hello👋世界🌍测试😊";
            let mut output = Vec::new();
            let reader = BufReader::new(Cursor::new(input));
            fold_file_characterwise(&mut output, reader, false, 8).unwrap();
            assert_eq!(String::from_utf8(output).unwrap(), "Hello👋世界\n🌍测试😊");
        }
    }

    #[cfg(test)]
    mod fold_file_bytewise_improved_tests {
        use super::*;
        use std::io::{BufReader, Cursor};

        #[test]
        fn test_fold_file_bytewise_utf8_boundary_protection() {
            // 测试UTF-8字符边界保护：确保不会在字符中间切断
            let input = "测试中文abc";
            let mut output = Vec::new();
            let reader = BufReader::new(Cursor::new(input));
            // 每个中文字符3字节，4字节应该只能放一个中文字符
            fold_file_bytewise(&mut output, reader, false, 4).unwrap();
            let result = String::from_utf8(output).unwrap();
            // 实际输出："测"(3字节) + "\n" + "试"(3字节) + "\n" + "中"(3字节) + "\n" + "文a" + "\n" + "bc"
            assert_eq!(result, "测\n试\n中\n文a\nbc");
        }

        #[test]
        fn test_fold_file_bytewise_mixed_ascii_utf8() {
            let input = "a测b试c";
            let mut output = Vec::new();
            let reader = BufReader::new(Cursor::new(input));
            // 5字节可以放"a测b"(1+3+1=5字节)
            fold_file_bytewise(&mut output, reader, false, 5).unwrap();
            let result = String::from_utf8(output).unwrap();
            assert_eq!(result, "a测b\n试c");
        }

        #[test]
        fn test_fold_file_bytewise_exact_utf8_boundary() {
            let input = "测试";
            let mut output = Vec::new();
            let reader = BufReader::new(Cursor::new(input));
            // 6字节正好是两个中文字符
            fold_file_bytewise(&mut output, reader, false, 6).unwrap();
            let result = String::from_utf8(output).unwrap();
            assert_eq!(result, "测试");
        }

        #[test]
        fn test_fold_file_bytewise_utf8_with_spaces() {
            let input = "测试 中文 字符";
            let mut output = Vec::new();
            let reader = BufReader::new(Cursor::new(input));
            // 7字节："测试 " (3+3+1=7字节)
            fold_file_bytewise(&mut output, reader, true, 7).unwrap();
            let result = String::from_utf8(output).unwrap();
            assert_eq!(result, "测试 \n中文 \n字符");
        }

        #[test]
        fn test_fold_file_bytewise_no_valid_break_point() {
            let input = "测";
            let mut output = Vec::new();
            let reader = BufReader::new(Cursor::new(input));
            // 1字节无法容纳任何中文字符，但我们的实现会强制输出整个字符以避免无限循环
            fold_file_bytewise(&mut output, reader, false, 1).unwrap();
            let result = String::from_utf8(output).unwrap();
            // 应该输出完整的字符
            assert_eq!(result, "测");
        }

        #[test]
        fn test_fold_file_bytewise_complex_utf8() {
            // 测试包含各种UTF-8字符：中文、日文、表情符号
            let input = "Hello👋こんにちは世界";
            let mut output = Vec::new();
            let reader = BufReader::new(Cursor::new(input));
            fold_file_bytewise(&mut output, reader, false, 8).unwrap();
            let result = String::from_utf8(output).unwrap();
            // 确保输出是有效的UTF-8且没有破损的字符
            assert!(result.chars().all(|c| !c.is_control() || c == '\n'));
        }
    }
}
