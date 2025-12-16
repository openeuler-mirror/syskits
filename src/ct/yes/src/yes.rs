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

use std::error::Error;
use std::ffi::OsString;
use std::io::{self, Write};

use clap::{builder::ValueParser, crate_version, Arg, ArgAction, Command};

use ctcore::ct_error::{CTResult, CtSimpleError};
#[cfg(unix)]
use ctcore::ct_signals::enable_pipe_errors;
use ctcore::{ct_format_usage, ct_help_about, ct_help_usage};

#[cfg(target_os = "linux")]
mod splice;

const YES_ABOUT: &str = ct_help_about!("yes.md");
const YES_USAGE: &str = ct_help_usage!("yes.md");

// 在某些系统上，使用更小或更大的缓冲区可能会提供更好的性能，当前设置满足需求
const YES_BUF_SIZE: usize = 16 * 1024;

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    yes_main(args)
}

pub fn yes_main(args: impl ctcore::Args) -> CTResult<()> {
    let matches = ct_app().try_get_matches_from(args)?;

    let mut buff = Vec::with_capacity(YES_BUF_SIZE);
    yes_args_into_buff(&mut buff, matches.get_many::<OsString>("STRING")).unwrap();
    yes_prepare_buff(&mut buff);

    if let Err(err) = yes_exec(&buff) {
        if matches!(err.kind(), io::ErrorKind::BrokenPipe) {
            Ok(())
        } else {
            Err(CtSimpleError::new(1, format!("standard output: {err}")))
        }
    } else {
        Ok(())
    }
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = YES_ABOUT;
    let usage_description = ct_format_usage(YES_USAGE);
    let arg = Arg::new("STRING")
        .value_parser(ValueParser::os_string())
        .action(ArgAction::Append);

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .arg(arg)
}

// 将`i`中的单词复制到`buf`中，中间用空格隔开。
fn yes_args_into_buff<'a>(
    buffer: &mut Vec<u8>,
    iter_option: Option<impl Iterator<Item = &'a OsString>>,
) -> Result<(), Box<dyn Error>> {
    // 如果没有提供参数，则直接在缓冲区中追加 "y\n" 并返回成功。
    let Some(iter) = iter_option else {
        buffer.extend_from_slice(b"y\n");
        return Ok(());
    };

    // Unix 系统（包括 WASI）处理逻辑：直接将 OsString 转为字节序列并以空格分隔。
    #[cfg(unix)]
    {
        #[cfg(unix)]
        use std::os::unix::ffi::OsStrExt;
        #[cfg(target_os = "wasi")]
        use std::os::wasi::ffi::OsStrExt;

        for part in itertools::intersperse(iter.map(|a| a.as_bytes()), b" ") {
            buffer.extend_from_slice(part);
        }
    }

    // Windows 系统处理逻辑：必须将 OsString 转换为 String，以处理可能的 UTF-8 编码问题。
    #[cfg(not(unix))]
    {
        for part_option in itertools::intersperse(iter.map(|os_str| os_str.to_str()), Some(" ")) {
            let b = match part_option {
                Some(p) => p.as_bytes(),
                None => return Err("arguments contain invalid UTF-8".into()),
            };
            buffer.extend_from_slice(b);
        }
    }

    // 在参数序列末尾追加换行符。
    buffer.push(b'\n');

    Ok(())
}

// 假定 buf 保存了从命令行参数中伪造的单个输出行，然后反复复制，直到缓冲区在 BUF_SIZE 范围内尽可能多地保存副本为止。
fn yes_prepare_buff(buffer: &mut Vec<u8>) {
    if buffer.len() * 2 > YES_BUF_SIZE {
        return;
    }

    assert!(!buffer.is_empty());

    let line_len = buffer.len();
    let target_size = line_len * (YES_BUF_SIZE / line_len);

    while buffer.len() < target_size {
        let to_copy = std::cmp::min(target_size - buffer.len(), buffer.len());
        debug_assert_eq!(to_copy % line_len, 0);
        buffer.extend_from_within(..to_copy);
    }
}

pub fn yes_exec(bytes_data: &[u8]) -> io::Result<()> {
    let io_ouput = io::stdout();
    let mut std_output: io::StdoutLock<'_> = io_ouput.lock();
    #[cfg(unix)]
    enable_pipe_errors()?;

    #[cfg(target_os = "linux")]
    {
        if splice::splice_data(bytes_data, &std_output).is_ok() {
            return Ok(());
        } else if let Err(splice::SpliceError::Io(err)) =
            splice::splice_data(bytes_data, &std_output)
        {
            return Err(err);
        } else if let Err(splice::SpliceError::Unsupported) =
            splice::splice_data(bytes_data, &std_output)
        {
            // 处理不支持的错误(do nothing)
        }
    }

    loop {
        std_output.write_all(bytes_data)?;
    }
}

