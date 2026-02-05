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

//! 输出或变更终端特性。
//! <设置> 字符串可以添加 "-" 前缀，表示禁用该项设置。下文中的 * 表示这项设置不属于 POSIX 标准。各项设置是否可用取决于底层的系统。

extern crate rust_i18n;
mod device;
mod settings;
mod termios;

use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::Tool;
use ctcore::ct_error::{CTResult, CtSimpleError};

use device::Device;
use nix::libc::{O_NONBLOCK, TIOCGWINSZ, TIOCSWINSZ, c_ushort};
use nix::sys::termios::{
    SpecialCharacterIndices, Termios, cfgetospeed, cfsetospeed, tcgetattr, tcsetattr,
};
use nix::{ioctl_read_bad, ioctl_write_ptr_bad};
use settings::{BAUD_RATES, Settings};
use settings::{CONTROL_CHARS, CONTROL_SETTINGS, INPUT_SETTINGS, LOCAL_SETTINGS, OUTPUT_SETTINGS};
use std::ffi::OsString;
use std::io::stdout;
use std::ops::ControlFlow;
use std::os::fd::AsFd;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use sys_locale::get_locale;
use termios::TermiosFlag;

mod stty_flags {
    pub const STTY_ALL: &str = "all";
    pub const STTY_SAVE: &str = "save";
    pub const STTY_FILE: &str = "file";
    pub const STTY_SETTINGS: &str = "settings";
}

struct SttyFlags {
    is_all: bool,
    is_save: bool,
    file: Device,
    settings: Option<Vec<String>>,
}

// 解析配置参数，将参数与可能的值组合成一个字符串列表
//
// # 参数
// - `args`: &[&str] - 一个字符串切片，包含配置参数
//
// # 返回值
// - Vec<String> - 一个字符串列表，每个字符串包含参数及其可能的值
fn parse_settings(args: &[&str]) -> Vec<String> {
    let mut cfg_settings = Vec::new();
    let mut iter = args.iter().peekable();

    while let Some(arg) = iter.next() {
        let mut setting = arg.to_string();

        // 检查当前参数是否为特殊设置，并获取其 requires_value 标志
        if let Some(entry) = settings::SPECIAL_SETTINGS
            .iter()
            .find(|entry| entry.name == *arg)
        {
            if entry.requires_value {
                // 如果下一个参数存在，则将其作为值附加到当前设置后面
                if let Some(next_arg) = iter.peek() {
                    setting.push(' ');
                    setting.push_str(next_arg);
                    iter.next();
                }
            }
        }

        cfg_settings.push(setting);
    }

    cfg_settings
}

impl SttyFlags {
    fn new(matches: &ArgMatches) -> CTResult<Self> {
        // 处理特别配置中的键值对逻辑，这里从命令行参数中获取设置项，然后解析这些设置项
        let settings = matches
            .get_many::<String>(stty_flags::STTY_SETTINGS)
            .map(|v| v.map(|s| s.as_str()).collect::<Vec<&str>>())
            .map(|args| parse_settings(&args));

        // 根据命令行参数确定设备类型
        let file = match matches.get_one::<String>(stty_flags::STTY_FILE) {
            // 当指定文件时，以非阻塞模式打开文件
            Some(f) => {
                let fd = std::fs::OpenOptions::new()
                    .read(true)
                    .custom_flags(O_NONBLOCK)
                    .open(f)
                    .map_err(|e| CtSimpleError::new(1, format!("Failed to open device: {e}")))?;
                Device::File(fd)
            }
            None => Device::Stdout(stdout()),
        };

        Ok(Self {
            // 是否显示所有设置的标志
            is_all: matches.get_flag(stty_flags::STTY_ALL),
            // 是否保存当前设置的标志
            is_save: matches.get_flag(stty_flags::STTY_SAVE),
            file,
            settings,
        })
    }

    /// 检查配置选项是否有效
    ///
    /// 此函数旨在确保不允许可选的详细输出风格和stty可读输出风格同时启用，
    /// 并且在指定输出风格时，不允许设置模式。通过执行这些检查，该函数确保了
    /// 配置选项的正确性和一致性。
    ///
    /// # 返回值
    ///
    /// * `Ok(())` - 如果配置选项没有冲突且有效
    /// * `Err(CtSimpleError)` - 如果检测到配置选项冲突，包含错误代码和消息
    fn check(&self) -> CTResult<()> {
        // 检查是否同时启用了详细输出风格和stty可读输出风格，如果同时启用，则返回错误
        if self.is_save && self.is_all {
            let err_message =
                "the options for verbose and stty-readable output styles are mutually exclusive";
            return Err(CtSimpleError::new(1, err_message));
        }

        // 检查是否在指定输出风格的同时设置了模式，如果设置了模式，则返回错误
        if self.settings.is_some() && (self.is_save || self.is_all) {
            let err_message = "when specifying an output style, modes may not be set";
            return Err(CtSimpleError::new(1, err_message));
        }

        Ok(())
    }
}

// Needs to be repr(C) because we pass it to the ioctl calls.
#[repr(C)]
#[derive(Default, Debug)]
pub struct TermSize {
    rows: c_ushort,
    columns: c_ushort,
    x: c_ushort,
    y: c_ushort,
}

ioctl_read_bad!(
    /// Get terminal window size
    tiocgwinsz,
    TIOCGWINSZ,
    TermSize
);

ioctl_write_ptr_bad!(
    /// Set terminal window size
    tiocswinsz,
    TIOCSWINSZ,
    TermSize
);

pub fn stty_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let matches = ct_app().try_get_matches_from(args)?;
    let stty_opts = SttyFlags::new(&matches)?;
    stty(&stty_opts)
}

/// 设置或打印终端的配置
///
/// 此函数根据提供的`SttyFlags`参数检查、获取或设置终端属性
/// 它使用`tcgetattr`获取当前终端配置，然后根据`opts.settings`
/// 是否存在来决定是应用新设置还是打印当前设置
///
/// # 参数
///
/// * `opts`: 一个`SttyFlags`的引用，包含了函数运行所需的参数和标志
///
/// # 返回值
///
/// 返回一个`CTResult`，表示操作成功完成或包含错误信息
fn stty(opts: &SttyFlags) -> CTResult<()> {
    // Check 参数冲突
    opts.check()?;

    // 通过 tcgetattr 获取终端配置
    let mut termios = tcgetattr(opts.file.as_fd()).expect("Could not get terminal attributes");

    // 通过 stty_apply_setting 应用设置
    if let Some(settings) = &opts.settings {
        for setting in settings {
            if let ControlFlow::Break(false) = stty_apply_setting(&mut termios, setting, &opts.file)
            {
                let err_message = format!("invalid argument '{setting}'");
                return Err(CtSimpleError::new(1, err_message));
            }
        }

        tcsetattr(
            opts.file.as_fd(),
            nix::sys::termios::SetArg::TCSANOW,
            &termios,
        )
        .expect("Could not write terminal attributes");
    } else {
        // 如果没有设置需要应用，则打印当前设置
        stty_print_settings(&termios, opts).expect("make proper error here from nix error");
    }
    Ok(())
}

/// 根据提供的 termios 结构和选项打印终端大小和行设置。
///
/// 该函数首先通过 `cfgetospeed` 获取终端的输出速度，
/// 然后遍历已知的波特率列表以找到匹配项并打印波特率信息。
/// 如果 `opts` 中的 `is_all` 标志被设置，则还会打印终端的行和列信息。
/// 最后，从 `termios` 结构中获取并打印终端的行规约（line discipline）。
///
/// # 参数
/// - `termios`: 指向 `Termios` 结构的引用，包含终端设置。
/// - `opts`: 指向 `SttyFlags` 结构的引用，包含要打印的信息选项。
///
/// # 返回值
/// - `CTResult<()>`: 自定义的结果类型，表示成功执行或发生错误。
fn stty_print_terminal_size(termios: &Termios, opts: &SttyFlags) -> CTResult<()> {
    let speed = cfgetospeed(termios);

    for (text, _, baud_rate) in BAUD_RATES {
        if *baud_rate == speed {
            print!("speed {text} baud; ");
            break;
        }
    }

    // 如果 `is_all` 选项被设置，则打印终端的行和列信息。
    if opts.is_all {
        stty_print_terminal_rows_columns(opts)?;
    }

    // 由于 nix 的 Termios 结构不直接暴露行规约字段，
    // 因此我们通过转换为底层的 libc::termios 结构来获取行规约信息。
    let libc_termios: nix::libc::termios = termios.clone().into();
    let line = libc_termios.c_line;
    println!("line = {line};");

    Ok(())
}

/// 打印终端的行数和列数
///
/// 此函数通过系统调用获取当前终端的尺寸信息，并打印出终端的行数和列数
/// 它主要用于配置终端，以便适应不同的终端大小和使用场景
///
/// # 参数
///
/// * `opts`: 一个 `SttyFlags` 结构体的引用，包含了执行此操作所需的文件描述符等信息
///
/// # 返回值
///
/// 返回一个 `CTResult`，表示操作是否成功如果成功，返回 `Ok(())`
fn stty_print_terminal_rows_columns(opts: &SttyFlags) -> CTResult<()> {
    let mut size = TermSize::default();

    // 使用系统调用 `tiocgwinsz` 获取终端尺寸信息
    // 这里使用了 `unsafe`，因为 `tiocgwinsz` 是一个低级的系统调用，需要显式地标记为不安全
    // `opts.file.as_raw_fd()` 获取文件描述符，`&mut size as *mut _` 将 `TermSize` 的引用转换为指针，以便系统调用使用
    unsafe { tiocgwinsz(opts.file.as_raw_fd(), &mut size as *mut _)? };
    print!("rows {}; columns {}; ", size.rows, size.columns);
    Ok(())
}

/// 将 `stty` 使用的控制字符转换为字符串表示。
///
/// 该函数接收一个来自 `nix::libc` 库的控制字符 (`cc_t` 类型) 作为输入，
/// 并根据 `stty` 工具的输出约定将其转换为人类可读的字符串格式。
/// 它根据特定规则处理特殊字符、控制字符和带有元前缀的字符。
///
/// # 参数
/// - `cc`: 需要转换的控制字符，类型为 `nix::libc::cc_t`。
///
/// # 返回值
/// - `CTResult<String>`: 包含控制字符字符串表示的结果类型，
///   如果转换失败则返回错误。
fn stty_control_char_to_string(cc: nix::libc::cc_t) -> CTResult<String> {
    // 处理未定义字符的情况
    if cc == 0 {
        return Ok("<undef>".to_string());
    }

    // 确定元前缀和调整后的字符代码
    let (meta_prefix, code) = if cc >= 0x80 {
        ("M-", cc - 0x80)
    } else {
        ("", cc)
    };

    // 根据代码确定 '^' 前缀（如果适用）和字符
    let (ctrl_prefix, character) = match code {
        // ASCII 范围内的控制字符
        0..=0x1f => Ok(("^", (b'@' + code) as char)),
        // 可打印的 ASCII 字符
        0x20..=0x7e => Ok(("", code as char)),
        // DEL 字符
        0x7f => Ok(("^", '?')),
        // 超出 8 位范围的字符
        _ => Err(nix::errno::Errno::ERANGE),
    }?;

    Ok(format!("{meta_prefix}{ctrl_prefix}{character}"))
}

/// 打印终端控制字符设置
///
/// 该函数根据提供的`Termios`结构体和`SttyFlags`选项，打印出终端的控制字符设置
/// 如果`opts.is_all`为`false`，则函数什么也不做直接返回
/// 如果`opts.is_all`为`true`，则函数会遍历一个控制字符索引列表，并打印出每个控制字符的当前设置
/// 最后，函数还会打印出`VMIN`和`VTIME`特殊字符的设置
///
/// # 参数
///
/// - `termios`: 一个指向`Termios`结构体的引用，包含了终端的I/O控制设置
/// - `opts`: 一个指向`SttyFlags`结构体的引用，包含了`stty`命令行选项的信息
///
/// # 返回值
///
/// 返回一个`CTResult<()>`，表示操作是否成功
fn stty_print_control_chars(termios: &Termios, opts: &SttyFlags) -> CTResult<()> {
    // 如果`opts.is_all`为`false`，则不打印任何信息直接返回
    // 未来的工作是实现一个逻辑来比较并打印与默认值不同的设置
    if !opts.is_all {
        // TODO: this branch should print values that differ from defaults
        return Ok(());
    }
    // 遍历控制字符索引列表，打印每个控制字符的当前设置
    for (text, cc_index) in CONTROL_CHARS {
        print!(
            "{text} = {}; ",
            stty_control_char_to_string(termios.control_chars[*cc_index as usize])?
        );
    }
    // 打印`VMIN`和`VTIME`特殊字符的设置
    println!(
        "min = {}; time = {};",
        termios.control_chars[SpecialCharacterIndices::VMIN as usize],
        termios.control_chars[SpecialCharacterIndices::VTIME as usize]
    );
    Ok(())
}

/// 将termios结构体的配置信息以特定格式打印出来
///
/// # 参数
/// * `termios` - 一个指向Termios结构体的引用，包含了终端的配置信息
///
/// 该函数按照特定的格式打印终端的输入标志、输出标志、控制标志和本地标志，
/// 以及控制字符。这种格式方便用户查看和理解当前终端的配置
fn stty_print_in_save_format(termios: &Termios) {
    print!(
        "{:x}:{:x}:{:x}:{:x}",
        termios.input_flags.bits(),
        termios.output_flags.bits(),
        termios.control_flags.bits(),
        termios.local_flags.bits()
    );
    // 遍历并打印控制字符，每个字符以十六进制格式
    for cc in termios.control_chars {
        print!(":{cc:x}");
    }
    println!();
}

/// 打印终端设置
///
/// 该函数根据提供的`Termios`结构体和`SttyFlags`选项来打印终端的当前设置
/// 如果`opts.is_save`为true，则以保存格式打印设置，否则打印详细的终端设置信息
///
/// # 参数
///
/// - `termios`: 一个指向`Termios`结构体的引用，包含终端的设置信息
/// - `opts`: 一个指向`SttyFlags`结构体的引用，包含打印设置的选项
///
/// # 返回值
///
/// 返回一个`CTResult<()>`，表示操作的结果，如果打印成功则返回`Ok(())`
fn stty_print_settings(termios: &Termios, opts: &SttyFlags) -> CTResult<()> {
    // 根据opts.is_save决定打印格式
    if opts.is_save {
        // 如果是保存格式，则调用相应函数打印
        stty_print_in_save_format(termios);
    } else {
        // 否则，详细打印终端的尺寸、控制字符和各种设置
        stty_print_terminal_size(termios, opts)?;
        stty_print_control_chars(termios, opts)?;
        stty_print_flags(termios, opts, CONTROL_SETTINGS);
        stty_print_flags(termios, opts, INPUT_SETTINGS);
        stty_print_flags(termios, opts, OUTPUT_SETTINGS);
        stty_print_flags(termios, opts, LOCAL_SETTINGS);
    }
    Ok(())
}

/// 打印终端的当前设置标志
///
/// 该函数根据提供的`SttyFlags`选项和`Settings`数组，打印出终端的当前设置标志
/// 它主要用于在命令行界面中显示终端的配置信息
///
/// # 参数
///
/// - `termios`: 一个`Termios`引用，包含终端的当前设置
/// - `opts`: 一个`SttyFlags`引用，包含命令行选项
/// - `flags`: 一个`Settings<T>`切片，描述要检查和打印的标志
fn stty_print_flags<T: TermiosFlag>(termios: &Termios, opts: &SttyFlags, flags: &[Settings<T>]) {
    // 初始化一个标志变量，用于跟踪是否已经打印了设置
    let mut printed = false;
    // 遍历每个设置标志
    for &Settings {
        name,
        flag,
        is_show,
        is_sane,
        group,
    } in flags
    {
        // 如果设置标志不应显示，则跳过
        if !is_show {
            continue;
        }
        // 检查设置标志是否在`termios`中设置
        let is_val = flag.is_in(termios, group);
        // 处理属于某个组的设置标志
        if group.is_some() {
            // 如果设置标志已设置且不被视为正常，或如果选择了显示所有设置，则打印
            if is_val && (!is_sane || opts.is_all) {
                print!("{name} ");
                printed = true;
            }
        } else if opts.is_all || is_val != is_sane {
            if !is_val {
                print!("-");
            }
            print!("{name} ");
            printed = true;
        }
    }

    // 如果打印了任何设置，则换行
    if printed {
        println!();
    }
}

/// 根据提供的字符串和设备信息应用终端设置。
///
/// 该函数通过解析字符串 `s` 并将相应的设置应用到 `termios` 结构中，处理各种终端设置。它首先尝试应用波特率设置，
/// 然后是特殊设置，最后是常规标志设置。如果字符串 `s` 以 '-' 开头，则表示应移除该设置。函数返回一个 `ControlFlow`，
/// 表示是否应用了特殊设置。
///
/// # 参数
/// - `termios`: 可变引用到 `Termios` 结构，表示终端的设置。
/// - `s`: 包含要应用设置的字符串切片。
/// - `device`: 引用到 `Device` 结构，提供设备特定的信息。
///
/// # 返回值
/// - `ControlFlow::Continue(false)`: 如果没有应用特殊设置。
/// - `ControlFlow::Break(true)`: 如果成功应用了特殊设置。
/// - `ControlFlow::Break(false)`: 如果成功应用了常规标志设置。
fn stty_apply_setting(termios: &mut Termios, s: &str, device: &Device) -> ControlFlow<bool> {
    // 首先处理波特率设置
    stty_apply_baud_rate_flag(termios, s)?;

    // 处理特殊设置
    if let ControlFlow::Break(applied) = stty_apply_special_setting(termios, s, device) {
        return ControlFlow::Break(applied);
    }

    // 处理常规标志设置
    let (remove, name) = match s.strip_prefix('-') {
        Some(s) => (true, s),
        None => (false, s),
    };
    // 应用控制、输入、输出和本地设置中的标志
    stty_apply_flag(termios, CONTROL_SETTINGS, name, remove)?;
    stty_apply_flag(termios, INPUT_SETTINGS, name, remove)?;
    stty_apply_flag(termios, OUTPUT_SETTINGS, name, remove)?;
    stty_apply_flag(termios, LOCAL_SETTINGS, name, remove)?;
    ControlFlow::Break(false)
}

/// 为终端应用特殊设置。例如：speed, rows, columns etc.
///
/// 此函数解析包含设置信息的字符串，并根据预定义的特殊设置列表匹配和应用相应的配置。
///
/// # 参数
/// - `termios`: 可变引用到 `Termios` 结构体，表示当前终端设置。
/// - `s`: 包含设置信息的字符串切片，例如 "eol ^M"。
/// - `device`: 引用到 `Device` 结构体，表示当前设备信息。
///
/// # 返回值
/// - 返回一个 `ControlFlow<bool>` 值，表示执行状态：
///   - `ControlFlow::Continue(())`: 如果未找到匹配的特殊设置，或者设置不需要值但未提供值。
///   - `ControlFlow::Break(false)`: 如果需要值但未提供。
///   - `ControlFlow::Break(true)`: 如果设置成功应用。
fn stty_apply_special_setting(
    termios: &mut Termios,
    s: &str,
    device: &Device,
) -> ControlFlow<bool> {
    // 将输入拆分为部分（例如 "eol ^M" -> ["eol", "^M"]）
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.is_empty() {
        return ControlFlow::Continue(());
    }

    // 查找特殊设置
    for entry in settings::SPECIAL_SETTINGS {
        if parts[0] == entry.name {
            // 获取设置名称对应的值
            let value = parts.get(1).copied();

            // 检查是否需要值但未提供
            if entry.requires_value && value.is_none() {
                return ControlFlow::Break(false);
            }

            // 应用设置
            let applied = entry.setting.apply(termios, value, device).unwrap();
            return ControlFlow::Break(applied);
        }
    }

    ControlFlow::Continue(())
}

/// 应用或移除指定的 termios 配置标志。
///
/// 该函数用于根据输入的标志名称，在给定的 termios 配置中应用或移除一个标志。
/// 它支持处理多个设置标志，并对带有组属性的标志进行特殊处理。
///
/// # 泛型参数
/// * `T`: 实现 `TermiosFlag` 特性的类型，表示可以应用于 termios 结构的标志。
///
/// # 参数
/// * `termios`: 指向 `Termios` 结构的可变引用，表示终端配置。
/// * `flags`: 包含要应用或移除的标志的 `Settings<T>` 类型切片。
/// * `input`: 表示要应用或移除的标志名称的字符串切片。
/// * `is_remove`: 布尔值，指示操作是移除标志。`true` 表示移除，`false` 表示应用。
///
/// # 返回值
/// * 返回 `ControlFlow<bool>` 类型，表示操作的执行状态。
///   * `ControlFlow::Break(true)` 表示标志成功应用或移除。
///   * `ControlFlow::Break(false)` 表示尝试移除带有组属性的标志，这是不允许的。
///   * `ControlFlow::Continue(())` 表示未找到匹配的标志，操作继续。
fn stty_apply_flag<T: TermiosFlag>(
    termios: &mut Termios,
    flags: &[Settings<T>],
    input: &str,
    is_remove: bool,
) -> ControlFlow<bool> {
    for Settings {
        name, flag, group, ..
    } in flags
    {
        if input == *name {
            // 带有组属性的标志不能被移除。
            // 由于名称匹配，可以短路并停止检查其他标志。
            if is_remove && group.is_some() {
                return ControlFlow::Break(false);
            }
            // 如果存在组属性，在应用标志之前应清除该组的位。
            if let Some(group) = group {
                group.apply(termios, false);
            }
            // 根据 `is_remove` 参数应用或移除标志。
            flag.apply(termios, !is_remove);
            // 一旦标志被处理，跳出循环并返回 true。
            return ControlFlow::Break(true);
        }
    }
    ControlFlow::Continue(())
}

/// 根据输入字符串应用波特率设置到 termios 结构体
///
/// # 参数
/// - `termios`: 可变引用到一个 `Termios` 结构体，包含终端 I/O 设置
/// - `input`: 包含用户期望的波特率设置的字符串切片
///
/// # 返回值
/// - 返回一个 `ControlFlow<bool>`，指示是否匹配到波特率设置
///   - `ControlFlow::Break(true)` 如果匹配到并成功设置了波特率
///   - `ControlFlow::Continue(())` 如果未匹配到任何波特率设置
///
/// # 描述
/// 该函数遍历预定义的波特率列表，尝试将输入字符串与其中一个波特率匹配。如果找到匹配项，
/// 它会使用 `cfsetospeed` 函数将终端的输入和输出速度设置为对应的波特率。如果设置波特率失败，
/// 将会触发错误并 panic。如果没有找到匹配项，则返回 `ControlFlow::Continue(())`，表示输入未匹配到任何波特率设置。
fn stty_apply_baud_rate_flag(termios: &mut Termios, input: &str) -> ControlFlow<bool> {
    // 设置特殊设置中的波特率：将输入和输出速度设置为 N 波特
    for (text, _, baud_rate) in BAUD_RATES {
        if *text == input {
            cfsetospeed(termios, *baud_rate).expect("Failed to set baud rate");
            return ControlFlow::Break(true);
        }
    }
    ControlFlow::Continue(())
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("stty.usage");
    let usage_description = t!("stty.about");
    let args = vec![
        Arg::new(stty_flags::STTY_ALL)
            .short('a')
            .long(stty_flags::STTY_ALL)
            .help(t!("stty.clap.stty_all"))
            .action(ArgAction::SetTrue),
        Arg::new(stty_flags::STTY_SAVE)
            .short('g')
            .long(stty_flags::STTY_SAVE)
            .help(t!("stty.clap.stty_save"))
            .action(ArgAction::SetTrue),
        Arg::new(stty_flags::STTY_FILE)
            .short('F')
            .long(stty_flags::STTY_FILE)
            .value_hint(clap::ValueHint::FilePath)
            .value_name("DEVICE")
            .help(t!("stty.clap.stty_file")),
        Arg::new(stty_flags::STTY_SETTINGS)
            .action(ArgAction::Append)
            .allow_hyphen_values(true)
            .help(t!("stty.clap.stty_settings")),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(&args)
}

#[derive(Default)]
pub struct Stty;
impl Tool for Stty {
    fn name(&self) -> &'static str {
        "stty"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        stty_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Stty;

        // Test name method
        assert_eq!(tool.name(), "stty");

        // Test command method
        let command = tool.command();
        assert!(command.get_name().contains("stty"));

        // Test execute method with help flag (should work)
        let args = vec![OsString::from("stty"), OsString::from("--help")];
        let result = tool.execute(&args);
        assert!(result.is_err());
    }

    /// 检测是否在容器环境中运行
    fn is_container() -> bool {
        // 检查常见的容器环境标识
        if std::env::var("KUBERNETES_SERVICE_HOST").is_ok()
            || std::env::var("DOCKER_CONTAINER").is_ok()
            || std::path::Path::new("/.dockerenv").exists()
            || std::path::Path::new("/run/.containerenv").exists()
        {
            return true;
        }

        // 检查 cgroup
        if let Ok(contents) = std::fs::read_to_string("/proc/1/cgroup") {
            if contents.contains("/docker/") || contents.contains("/kubepods/") {
                return true;
            }
        }

        false
    }

    #[cfg(test)]
    mod ct_app_tests {
        use super::*;
        use clap::error::ErrorKind;

        #[test]
        fn test_ct_app_execution_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_help() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--help"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_invalid_argument() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
        }

        #[test]
        fn test_ct_app_all_flag() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-a"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
            assert!(executable.unwrap().get_flag(stty_flags::STTY_ALL));
        }

        #[test]
        fn test_ct_app_save_flag() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-g"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
            assert!(executable.unwrap().get_flag(stty_flags::STTY_SAVE));
        }

        #[test]
        fn test_ct_app_file_option() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-F", "/dev/tty"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
            assert_eq!(
                executable
                    .unwrap()
                    .get_one::<String>(stty_flags::STTY_FILE)
                    .unwrap(),
                "/dev/tty"
            );
        }

        #[test]
        fn test_ct_app_multiple_settings() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "raw", "-echo", "9600", "size"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
            let settings: Vec<String> = executable
                .unwrap()
                .get_many::<String>(stty_flags::STTY_SETTINGS)
                .unwrap()
                .cloned()
                .collect();
            assert_eq!(settings, vec!["raw", "-echo", "9600", "size"]);
        }
    }

    #[cfg(test)]
    mod stty_flags_tests {
        use super::*;

        #[test]
        fn test_stty_flags_new_with_all_flag() {
            let command = ct_app();
            let matches = command
                .try_get_matches_from(vec![ctcore::ct_util_name(), "-a"])
                .unwrap();
            let flags = SttyFlags::new(&matches).unwrap();
            assert!(flags.is_all);
            assert!(!flags.is_save);
            assert!(matches!(flags.file, Device::Stdout(_)));
        }

        #[test]
        fn test_stty_flags_new_with_save_flag() {
            let command = ct_app();
            let matches = command
                .try_get_matches_from(vec![ctcore::ct_util_name(), "-g"])
                .unwrap();
            let flags = SttyFlags::new(&matches).unwrap();
            assert!(!flags.is_all);
            assert!(flags.is_save);
        }

        #[test]
        fn test_stty_flags_new_with_file_option() {
            if is_container() {
                println!("Skipping test_stty_flags_new_with_file_option in container environment");
                return;
            }
            let command = ct_app();
            let matches = command
                .try_get_matches_from(vec![ctcore::ct_util_name(), "-F", "/dev/tty"])
                .unwrap();
            let flags = SttyFlags::new(&matches).unwrap();
            assert!(matches!(flags.file, Device::File(_)));
        }

        #[test]
        fn test_stty_flags_new_with_settings() {
            let command = ct_app();
            let matches = command
                .try_get_matches_from(vec![ctcore::ct_util_name(), "raw", "-echo"])
                .unwrap();
            let flags = SttyFlags::new(&matches).unwrap();
            assert!(flags.settings.is_some());
            assert_eq!(
                flags.settings.unwrap(),
                vec!["raw".to_string(), "-echo".to_string()]
            );
        }

        #[test]
        fn test_stty_flags_check_valid_combinations() {
            let command = ct_app();
            let matches = command
                .try_get_matches_from(vec![ctcore::ct_util_name(), "-a"])
                .unwrap();
            let flags = SttyFlags::new(&matches).unwrap();
            assert!(flags.check().is_ok());
        }

        #[test]
        fn test_stty_flags_check_invalid_combinations() {
            let flags = SttyFlags {
                is_all: true,
                is_save: true,
                file: Device::Stdout(stdout()),
                settings: None,
            };
            assert!(flags.check().is_err());
        }
    }

    #[cfg(test)]
    mod parse_settings_tests {
        use super::*;

        #[test]
        fn test_parse_settings_basic() {
            let args = &["raw", "9600"];
            let settings = parse_settings(args);
            assert_eq!(settings, vec!["raw", "9600"]);
        }

        #[test]
        fn test_parse_settings_with_special_settings() {
            let args = &["min", "1", "time", "10"];
            let settings = parse_settings(args);
            assert_eq!(settings, vec!["min 1", "time 10"]);
        }

        #[test]
        fn test_parse_settings_mixed() {
            let args = &["raw", "min", "1", "9600"];
            let settings = parse_settings(args);
            assert_eq!(settings, vec!["raw", "min 1", "9600"]);
        }

        #[test]
        fn test_parse_settings_empty() {
            let args: &[&str] = &[];
            let settings = parse_settings(args);
            assert!(settings.is_empty());
        }
    }

    #[cfg(test)]
    mod stty_print_tests {
        use super::*;

        #[test]
        fn test_stty_print_terminal_size() {
            if is_container() {
                println!("Skipping test_stty_print_terminal_size in container environment");
                return;
            }

            let termios = unsafe { std::mem::zeroed::<Termios>() };
            let device = Device::Stdout(stdout());
            let opts = SttyFlags {
                is_all: true,
                is_save: false,
                file: device,
                settings: None,
            };
            assert!(stty_print_terminal_size(&termios, &opts).is_ok());
        }

        #[test]
        fn test_stty_print_control_chars() {
            if is_container() {
                println!("Skipping test_stty_print_control_chars in container environment");
                return;
            }

            let termios = unsafe { std::mem::zeroed::<Termios>() };
            let opts = SttyFlags {
                is_all: true,
                is_save: false,
                file: Device::Stdout(stdout()),
                settings: None,
            };
            assert!(stty_print_control_chars(&termios, &opts).is_ok());
        }

        #[test]
        fn test_stty_print_in_save_format() {
            if is_container() {
                println!("Skipping test_stty_print_in_save_format in container environment");
                return;
            }

            let mut termios = unsafe { std::mem::zeroed::<Termios>() };
            // 设置一些非零值以确保输出不为空
            termios.input_flags = nix::sys::termios::InputFlags::BRKINT;
            termios.output_flags = nix::sys::termios::OutputFlags::OPOST;
            termios.control_flags = nix::sys::termios::ControlFlags::CREAD;
            termios.local_flags = nix::sys::termios::LocalFlags::ECHO;

            // 捕获输出并打印
            stty_print_in_save_format(&termios);
        }
    }

    #[cfg(test)]
    mod stty_apply_tests {
        use super::*;

        #[test]
        fn test_stty_apply_baud_rate_valid() {
            let mut termios = unsafe { std::mem::zeroed::<Termios>() };
            for (text, _, _) in BAUD_RATES {
                assert!(matches!(
                    stty_apply_baud_rate_flag(&mut termios, text),
                    ControlFlow::Break(true)
                ));
            }
        }

        #[test]
        fn test_stty_apply_baud_rate_invalid() {
            let mut termios = unsafe { std::mem::zeroed::<Termios>() };
            assert!(matches!(
                stty_apply_baud_rate_flag(&mut termios, "invalid"),
                ControlFlow::Continue(())
            ));
        }

        #[test]
        fn test_stty_apply_special_setting_size() {
            if is_container() {
                println!("Skipping test_stty_apply_special_setting_size in container environment");
                return;
            }

            let mut termios = unsafe { std::mem::zeroed::<Termios>() };
            let device = Device::Stdout(stdout());
            assert!(matches!(
                stty_apply_special_setting(&mut termios, "size", &device),
                ControlFlow::Break(true)
            ));
        }

        #[test]
        fn test_stty_apply_special_setting_min() {
            let mut termios = unsafe { std::mem::zeroed::<Termios>() };
            let device = Device::Stdout(stdout());
            assert!(matches!(
                stty_apply_special_setting(&mut termios, "min 1", &device),
                ControlFlow::Break(true)
            ));
        }

        #[test]
        fn test_stty_apply_special_setting_invalid() {
            let mut termios = unsafe { std::mem::zeroed::<Termios>() };
            let device = Device::Stdout(stdout());
            assert!(matches!(
                stty_apply_special_setting(&mut termios, "invalid", &device),
                ControlFlow::Continue(())
            ));
        }
    }

    #[cfg(test)]
    mod stty_main_tests {
        use super::*;

        #[test]
        fn test_stty_main_help() {
            let args = [ctcore::ct_util_name(), "--help"];
            let result = stty_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_stty_main_version() {
            let args = [ctcore::ct_util_name(), "--version"];
            let result = stty_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }

        #[test]
        fn test_stty_main_all_flag() {
            if is_container() {
                println!("Skipping test_stty_main_all_flag in container environment");
                return;
            }

            let args = [ctcore::ct_util_name(), "-a"];
            let result = stty_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_stty_main_save_flag() {
            if is_container() {
                println!("Skipping test_stty_main_save_flag in container environment");
                return;
            }

            let args = [ctcore::ct_util_name(), "-g"];
            let result = stty_main(args.iter().map(OsString::from));
            assert!(result.is_ok());
        }

        #[test]
        fn test_stty_main_invalid_device() {
            let args = [ctcore::ct_util_name(), "-F", "/nonexistent/device"];
            let result = stty_main(args.iter().map(OsString::from));
            match result {
                Err(e) => {
                    assert!(e.to_string().contains("No such file or directory"));
                }
                Ok(_) => {
                    panic!("Expected error when opening non-existent device");
                }
            }
        }

        #[test]
        fn test_stty_main_multiple_settings() {
            if is_container() {
                println!("Skipping test_stty_main_multiple_settings in container environment");
                return;
            }

            let args = [ctcore::ct_util_name(), "9600", "-echo", "raw"];
            let result = stty_main(args.iter().map(OsString::from));
            assert!(result.is_err());
        }
    }
}
