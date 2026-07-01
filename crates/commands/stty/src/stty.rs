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
use nix::libc::{O_NONBLOCK, TIOCGWINSZ, TIOCSWINSZ, c_ushort, tcflag_t};
use nix::sys::termios::{
    ControlFlags as C, InputFlags as I, LocalFlags as L, OutputFlags as O, SpecialCharacterIndices,
    Termios, cfgetispeed, cfgetospeed, cfsetospeed, tcgetattr, tcsetattr,
};
use nix::{ioctl_read_bad, ioctl_write_ptr_bad};
use settings::{BAUD_RATES, Settings};
use settings::{CONTROL_CHARS, CONTROL_SETTINGS, INPUT_SETTINGS, LOCAL_SETTINGS, OUTPUT_SETTINGS};
use std::ffi::OsString;
use std::io::stdin;
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
    file_name: Option<String>,
    file: Device,
    settings: Option<Vec<String>>,
}

#[cfg(any(target_os = "linux", target_os = "android"))]
const IUCLC_RAW_BIT: tcflag_t = 0o0001000;
#[cfg(not(any(target_os = "linux", target_os = "android")))]
const IUCLC_RAW_BIT: tcflag_t = 0;

#[cfg(any(target_os = "linux", target_os = "android"))]
const XCASE_RAW_BIT: tcflag_t = 0o0000004;
#[cfg(not(any(target_os = "linux", target_os = "android")))]
const XCASE_RAW_BIT: tcflag_t = 0;

#[cfg(any(target_os = "linux", target_os = "android"))]
const OFILL_RAW_BIT: tcflag_t = 0o0000100;
#[cfg(not(any(target_os = "linux", target_os = "android")))]
const OFILL_RAW_BIT: tcflag_t = 0;

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
            None => Device::Stdin(stdin()),
        };
        let file_name = matches.get_one::<String>(stty_flags::STTY_FILE).cloned();

        Ok(Self {
            // 是否显示所有设置的标志
            is_all: matches.get_flag(stty_flags::STTY_ALL),
            // 是否保存当前设置的标志
            is_save: matches.get_flag(stty_flags::STTY_SAVE),
            file_name,
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

fn find_special_setting(name: &str) -> Option<settings::SpecialSettingEntry> {
    settings::SPECIAL_SETTINGS
        .iter()
        .copied()
        .find(|entry| entry.name == name)
}

fn find_regular_setting(name: &str) -> Option<bool> {
    for setting in CONTROL_SETTINGS {
        if setting.name == name {
            return Some(setting.group.is_some());
        }
    }
    for setting in INPUT_SETTINGS {
        if setting.name == name {
            return Some(setting.group.is_some());
        }
    }
    for setting in OUTPUT_SETTINGS {
        if setting.name == name {
            return Some(setting.group.is_some());
        }
    }
    for setting in LOCAL_SETTINGS {
        if setting.name == name {
            return Some(setting.group.is_some());
        }
    }
    None
}

fn is_custom_regular_setting(name: &str) -> bool {
    matches!(name, "ofill" | "iuclc" | "xcase")
}

fn find_combination_setting(name: &str) -> Option<bool> {
    let reversible = match name {
        "evenp" | "parity" | "oddp" | "nl" | "cooked" | "raw" | "pass8" | "litout" | "cbreak"
        | "decctlq" | "tabs" | "lcase" | "LCASE" => true,
        "ek" | "sane" | "crt" | "dec" => false,
        _ => return None,
    };
    Some(reversible)
}

fn validate_setting_syntax(setting: &str) -> CTResult<()> {
    if BAUD_RATES.iter().any(|(text, _, _)| *text == setting) {
        return Ok(());
    }

    if setting.contains(':') {
        return Ok(());
    }

    let mut parts = setting.split_whitespace();
    let Some(name) = parts.next() else {
        return Err(CtSimpleError::new(1, "invalid argument ''"));
    };
    let value = parts.next();

    if let Some(entry) = find_special_setting(name) {
        if entry.requires_value && value.is_none() {
            return Err(CtSimpleError::new(
                1,
                format!("invalid argument '{setting}'"),
            ));
        }
        return Ok(());
    }

    let (is_remove, base_name) = match name.strip_prefix('-') {
        Some(s) => (true, s),
        None => (false, name),
    };

    if is_custom_regular_setting(base_name) {
        if value.is_some() {
            return Err(CtSimpleError::new(
                1,
                format!("invalid argument '{setting}'"),
            ));
        }
        return Ok(());
    }

    if let Some(is_reversible) = find_combination_setting(base_name) {
        if value.is_some() || (is_remove && !is_reversible) {
            return Err(CtSimpleError::new(
                1,
                format!("invalid argument '{setting}'"),
            ));
        }
        return Ok(());
    }

    let Some(is_grouped) = find_regular_setting(base_name) else {
        return Err(CtSimpleError::new(
            1,
            format!("invalid argument '{setting}'"),
        ));
    };

    if value.is_some() || (is_remove && is_grouped) {
        return Err(CtSimpleError::new(
            1,
            format!("invalid argument '{setting}'"),
        ));
    }

    Ok(())
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

    if let Some(settings) = &opts.settings {
        for setting in settings {
            validate_setting_syntax(setting)?;
        }
    }

    // 通过 tcgetattr 获取终端配置
    let mut termios =
        tcgetattr(opts.file.as_fd()).map_err(|e| CtSimpleError::new(1, e.to_string()))?;

    // 通过 stty_apply_setting 应用设置
    if let Some(settings) = &opts.settings {
        let drain_only = settings
            .iter()
            .all(|setting| matches!(setting.as_str(), "drain" | "-drain"));

        for setting in settings {
            let applied = stty_apply_setting(&mut termios, setting, &opts.file)?;
            if matches!(
                applied,
                ControlFlow::Break(false) | ControlFlow::Continue(())
            ) {
                let err_message = format!("invalid argument '{setting}'");
                return Err(CtSimpleError::new(1, err_message));
            }
        }

        if drain_only {
            stty_print_settings(&termios, opts)?;
        } else {
            tcsetattr(
                opts.file.as_fd(),
                nix::sys::termios::SetArg::TCSANOW,
                &termios,
            )
            .map_err(|e| CtSimpleError::new(1, e.to_string()))?;
            let current =
                tcgetattr(opts.file.as_fd()).map_err(|e| CtSimpleError::new(1, e.to_string()))?;
            if !termios_equal(&termios, &current) {
                let msg = match &opts.file_name {
                    Some(file) => format!("{file}: unable to perform all requested operations"),
                    None => "unable to perform all requested operations".to_string(),
                };
                return Err(CtSimpleError::new(1, msg));
            }
        }
    } else {
        // 如果没有设置需要应用，则打印当前设置
        stty_print_settings(&termios, opts)?;
    }
    Ok(())
}

fn termios_equal(lhs: &Termios, rhs: &Termios) -> bool {
    let lhs_libc: nix::libc::termios = lhs.clone().into();
    let rhs_libc: nix::libc::termios = rhs.clone().into();
    let lhs_input = lhs.input_flags.bits() & !IUCLC_RAW_BIT;
    let rhs_input = rhs.input_flags.bits() & !IUCLC_RAW_BIT;
    let lhs_output = lhs.output_flags.bits() & !OFILL_RAW_BIT;
    let rhs_output = rhs.output_flags.bits() & !OFILL_RAW_BIT;
    let lhs_local = lhs.local_flags.bits() & !XCASE_RAW_BIT;
    let rhs_local = rhs.local_flags.bits() & !XCASE_RAW_BIT;

    lhs_input == rhs_input
        && lhs_output == rhs_output
        && lhs.control_flags.bits() == rhs.control_flags.bits()
        && lhs_local == rhs_local
        && lhs_libc.c_line == rhs_libc.c_line
        && lhs.control_chars == rhs.control_chars
        && cfgetispeed(lhs) == cfgetispeed(rhs)
        && cfgetospeed(lhs) == cfgetospeed(rhs)
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
    let mut printed_flags = Vec::new();
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
                printed_flags.push(name.to_string());
            }
        } else if opts.is_all || is_val != is_sane {
            let text = if is_val {
                name.to_string()
            } else {
                format!("-{name}")
            };
            printed_flags.push(text);
        }
    }

    if !printed_flags.is_empty() {
        println!("{}", printed_flags.join(" "));
    }
}

fn set_input_raw_bit(termios: &mut Termios, bit: tcflag_t, enabled: bool) {
    if bit == 0 {
        return;
    }
    let new_bits = if enabled {
        termios.input_flags.bits() | bit
    } else {
        termios.input_flags.bits() & !bit
    };
    termios.input_flags = I::from_bits_retain(new_bits);
}

fn set_output_raw_bit(termios: &mut Termios, bit: tcflag_t, enabled: bool) {
    if bit == 0 {
        return;
    }
    let new_bits = if enabled {
        termios.output_flags.bits() | bit
    } else {
        termios.output_flags.bits() & !bit
    };
    termios.output_flags = O::from_bits_retain(new_bits);
}

fn set_local_raw_bit(termios: &mut Termios, bit: tcflag_t, enabled: bool) {
    if bit == 0 {
        return;
    }
    let new_bits = if enabled {
        termios.local_flags.bits() | bit
    } else {
        termios.local_flags.bits() & !bit
    };
    termios.local_flags = L::from_bits_retain(new_bits);
}

fn apply_sane_flags<T: TermiosFlag>(termios: &mut Termios, flags: &[Settings<T>]) {
    for Settings {
        flag,
        is_sane,
        group,
        ..
    } in flags
    {
        if let Some(group) = group {
            group.apply(termios, false);
        }
        flag.apply(termios, *is_sane);
    }
}

fn sane_control_char_value(name: &str) -> Option<u8> {
    match name {
        "intr" => Some(3),
        "quit" => Some(28),
        "erase" => Some(127),
        "kill" => Some(21),
        "eof" => Some(4),
        "eol" => Some(0),
        "eol2" => Some(0),
        "swtch" => Some(0),
        "start" => Some(17),
        "stop" => Some(19),
        "susp" => Some(26),
        "rprnt" => Some(18),
        "werase" => Some(23),
        "lnext" => Some(22),
        "discard" => Some(15),
        _ => None,
    }
}

fn apply_sane_mode(termios: &mut Termios) {
    apply_sane_flags(termios, CONTROL_SETTINGS);
    apply_sane_flags(termios, INPUT_SETTINGS);
    apply_sane_flags(termios, OUTPUT_SETTINGS);
    apply_sane_flags(termios, LOCAL_SETTINGS);

    set_input_raw_bit(termios, IUCLC_RAW_BIT, false);
    set_output_raw_bit(termios, OFILL_RAW_BIT, false);
    set_local_raw_bit(termios, XCASE_RAW_BIT, false);

    for (name, cc_index) in CONTROL_CHARS {
        if let Some(value) = sane_control_char_value(name) {
            termios.control_chars[*cc_index as usize] = value;
        }
    }
    termios.control_chars[SpecialCharacterIndices::VMIN as usize] = 1;
    termios.control_chars[SpecialCharacterIndices::VTIME as usize] = 0;
}

fn apply_cooked_mode(termios: &mut Termios) {
    I::BRKINT.apply(termios, true);
    I::IGNPAR.apply(termios, true);
    I::ISTRIP.apply(termios, true);
    I::ICRNL.apply(termios, true);
    I::IXON.apply(termios, true);
    O::OPOST.apply(termios, true);
    L::ISIG.apply(termios, true);
    L::ICANON.apply(termios, true);
}

fn apply_raw_mode(termios: &mut Termios) {
    termios.input_flags = I::from_bits_retain(0);
    O::OPOST.apply(termios, false);
    L::ISIG.apply(termios, false);
    L::ICANON.apply(termios, false);
    set_local_raw_bit(termios, XCASE_RAW_BIT, false);
    termios.control_chars[SpecialCharacterIndices::VMIN as usize] = 1;
    termios.control_chars[SpecialCharacterIndices::VTIME as usize] = 0;
}

fn stty_apply_custom_flag(termios: &mut Termios, name: &str, is_remove: bool) -> ControlFlow<bool> {
    match name {
        "ofill" => {
            set_output_raw_bit(termios, OFILL_RAW_BIT, !is_remove);
            ControlFlow::Break(true)
        }
        "iuclc" => {
            set_input_raw_bit(termios, IUCLC_RAW_BIT, !is_remove);
            ControlFlow::Break(true)
        }
        "xcase" => {
            set_local_raw_bit(termios, XCASE_RAW_BIT, !is_remove);
            ControlFlow::Break(true)
        }
        _ => ControlFlow::Continue(()),
    }
}

fn stty_apply_combination_setting(
    termios: &mut Termios,
    name: &str,
    reversed: bool,
) -> ControlFlow<bool> {
    match name {
        "evenp" | "parity" => {
            if reversed {
                C::PARENB.apply(termios, false);
                C::CSIZE.apply(termios, false);
                C::CS8.apply(termios, true);
            } else {
                C::PARODD.apply(termios, false);
                C::CSIZE.apply(termios, false);
                C::PARENB.apply(termios, true);
                C::CS7.apply(termios, true);
            }
            ControlFlow::Break(true)
        }
        "oddp" => {
            if reversed {
                C::PARENB.apply(termios, false);
                C::CSIZE.apply(termios, false);
                C::CS8.apply(termios, true);
            } else {
                C::CSIZE.apply(termios, false);
                C::CS7.apply(termios, true);
                C::PARODD.apply(termios, true);
                C::PARENB.apply(termios, true);
            }
            ControlFlow::Break(true)
        }
        "nl" => {
            if reversed {
                I::ICRNL.apply(termios, true);
                I::INLCR.apply(termios, false);
                I::IGNCR.apply(termios, false);
                O::ONLCR.apply(termios, true);
                O::OCRNL.apply(termios, false);
                O::ONLRET.apply(termios, false);
            } else {
                I::ICRNL.apply(termios, false);
                O::ONLCR.apply(termios, false);
            }
            ControlFlow::Break(true)
        }
        "ek" => {
            if reversed {
                ControlFlow::Break(false)
            } else {
                termios.control_chars[SpecialCharacterIndices::VERASE as usize] = 127;
                termios.control_chars[SpecialCharacterIndices::VKILL as usize] = 21;
                ControlFlow::Break(true)
            }
        }
        "sane" => {
            if reversed {
                ControlFlow::Break(false)
            } else {
                apply_sane_mode(termios);
                ControlFlow::Break(true)
            }
        }
        "cbreak" => {
            L::ICANON.apply(termios, reversed);
            ControlFlow::Break(true)
        }
        "pass8" => {
            if reversed {
                C::CSIZE.apply(termios, false);
                C::CS7.apply(termios, true);
                C::PARENB.apply(termios, true);
                I::ISTRIP.apply(termios, true);
            } else {
                C::PARENB.apply(termios, false);
                C::CSIZE.apply(termios, false);
                C::CS8.apply(termios, true);
                I::ISTRIP.apply(termios, false);
            }
            ControlFlow::Break(true)
        }
        "litout" => {
            if reversed {
                C::CSIZE.apply(termios, false);
                C::CS7.apply(termios, true);
                C::PARENB.apply(termios, true);
                I::ISTRIP.apply(termios, true);
                O::OPOST.apply(termios, true);
            } else {
                C::PARENB.apply(termios, false);
                C::CSIZE.apply(termios, false);
                C::CS8.apply(termios, true);
                I::ISTRIP.apply(termios, false);
                O::OPOST.apply(termios, false);
            }
            ControlFlow::Break(true)
        }
        "raw" | "cooked" => {
            let cooked_mode = (name == "raw" && reversed) || (name == "cooked" && !reversed);
            if cooked_mode {
                apply_cooked_mode(termios);
            } else {
                apply_raw_mode(termios);
            }
            ControlFlow::Break(true)
        }
        "decctlq" => {
            I::IXANY.apply(termios, reversed);
            ControlFlow::Break(true)
        }
        "tabs" => {
            O::TABDLY.apply(termios, false);
            if reversed {
                O::TAB3.apply(termios, true);
            } else {
                O::TAB0.apply(termios, true);
            }
            ControlFlow::Break(true)
        }
        "lcase" | "LCASE" => {
            set_local_raw_bit(termios, XCASE_RAW_BIT, !reversed);
            set_input_raw_bit(termios, IUCLC_RAW_BIT, !reversed);
            O::OLCUC.apply(termios, !reversed);
            ControlFlow::Break(true)
        }
        "crt" => {
            if reversed {
                ControlFlow::Break(false)
            } else {
                L::ECHOE.apply(termios, true);
                L::ECHOCTL.apply(termios, true);
                L::ECHOKE.apply(termios, true);
                ControlFlow::Break(true)
            }
        }
        "dec" => {
            if reversed {
                ControlFlow::Break(false)
            } else {
                termios.control_chars[SpecialCharacterIndices::VINTR as usize] = 3;
                termios.control_chars[SpecialCharacterIndices::VERASE as usize] = 127;
                termios.control_chars[SpecialCharacterIndices::VKILL as usize] = 21;
                L::ECHOE.apply(termios, true);
                L::ECHOCTL.apply(termios, true);
                L::ECHOKE.apply(termios, true);
                I::IXANY.apply(termios, false);
                ControlFlow::Break(true)
            }
        }
        _ => ControlFlow::Continue(()),
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
fn stty_apply_setting(
    termios: &mut Termios,
    s: &str,
    device: &Device,
) -> CTResult<ControlFlow<bool>> {
    if let ControlFlow::Break(applied) = stty_apply_recover_mode(termios, s) {
        return Ok(ControlFlow::Break(applied));
    }

    // 首先处理波特率设置
    if let ControlFlow::Break(applied) = stty_apply_baud_rate_flag(termios, s)? {
        return Ok(ControlFlow::Break(applied));
    }

    // 处理特殊设置
    if let ControlFlow::Break(applied) = stty_apply_special_setting(termios, s, device)? {
        return Ok(ControlFlow::Break(applied));
    }

    // 处理常规标志设置
    let (remove, name) = match s.strip_prefix('-') {
        Some(s) => (true, s),
        None => (false, s),
    };

    if let ControlFlow::Break(applied) = stty_apply_custom_flag(termios, name, remove) {
        return Ok(ControlFlow::Break(applied));
    }
    if let ControlFlow::Break(applied) = stty_apply_combination_setting(termios, name, remove) {
        return Ok(ControlFlow::Break(applied));
    }

    if let ControlFlow::Break(applied) = stty_apply_flag(termios, CONTROL_SETTINGS, name, remove) {
        return Ok(ControlFlow::Break(applied));
    }
    if let ControlFlow::Break(applied) = stty_apply_flag(termios, INPUT_SETTINGS, name, remove) {
        return Ok(ControlFlow::Break(applied));
    }
    if let ControlFlow::Break(applied) = stty_apply_flag(termios, OUTPUT_SETTINGS, name, remove) {
        return Ok(ControlFlow::Break(applied));
    }
    if let ControlFlow::Break(applied) = stty_apply_flag(termios, LOCAL_SETTINGS, name, remove) {
        return Ok(ControlFlow::Break(applied));
    }

    Ok(ControlFlow::Continue(()))
}

fn stty_apply_recover_mode(termios: &mut Termios, input: &str) -> ControlFlow<bool> {
    if !input.contains(':') {
        return ControlFlow::Continue(());
    }

    let parts: Vec<&str> = input.split(':').collect();
    if parts.len() != 4 + termios.control_chars.len() {
        return ControlFlow::Break(false);
    }

    let mut parsed = [0u64; 4];
    for (idx, raw) in parts.iter().take(4).enumerate() {
        let Ok(value) = u64::from_str_radix(raw, 16) else {
            return ControlFlow::Break(false);
        };
        parsed[idx] = value;
    }

    termios.input_flags = nix::sys::termios::InputFlags::from_bits_truncate(parsed[0] as _);
    termios.output_flags = nix::sys::termios::OutputFlags::from_bits_truncate(parsed[1] as _);
    termios.control_flags = nix::sys::termios::ControlFlags::from_bits_truncate(parsed[2] as _);
    termios.local_flags = nix::sys::termios::LocalFlags::from_bits_truncate(parsed[3] as _);

    for (idx, raw) in parts.iter().skip(4).enumerate() {
        let Ok(value) = u8::from_str_radix(raw, 16) else {
            return ControlFlow::Break(false);
        };
        termios.control_chars[idx] = value;
    }

    ControlFlow::Break(true)
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
) -> CTResult<ControlFlow<bool>> {
    // 将输入拆分为部分（例如 "eol ^M" -> ["eol", "^M"]）
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.is_empty() {
        return Ok(ControlFlow::Continue(()));
    }

    // 查找特殊设置
    for entry in settings::SPECIAL_SETTINGS {
        if parts[0] == entry.name {
            // 获取设置名称对应的值
            let value = parts.get(1).copied();

            // 检查是否需要值但未提供
            if entry.requires_value && value.is_none() {
                return Ok(ControlFlow::Break(false));
            }

            // 应用设置
            return match entry.setting.apply(termios, value, device) {
                Ok(applied) => Ok(ControlFlow::Break(applied)),
                Err(err) if err.to_string().starts_with("invalid argument") => {
                    Ok(ControlFlow::Break(false))
                }
                Err(err) => Err(err),
            };
        }
    }

    Ok(ControlFlow::Continue(()))
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
fn stty_apply_baud_rate_flag(termios: &mut Termios, input: &str) -> CTResult<ControlFlow<bool>> {
    // 设置特殊设置中的波特率：将输入和输出速度设置为 N 波特
    for (text, _, baud_rate) in BAUD_RATES {
        if *text == input {
            cfsetospeed(termios, *baud_rate)
                .map_err(|e| CtSimpleError::new(1, format!("Failed to set output speed: {e}")))?;
            nix::sys::termios::cfsetispeed(termios, *baud_rate)
                .map_err(|e| CtSimpleError::new(1, format!("Failed to set input speed: {e}")))?;
            return Ok(ControlFlow::Break(true));
        }
    }
    Ok(ControlFlow::Continue(()))
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("stty.about");
    let usage_description = t!("stty.usage");
    let after_help = t!("stty.after_help");
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
        .after_help(after_help)
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
    use std::os::fd::AsRawFd;

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
        // 无 tty 的非交互环境下，依赖终端 ioctl 的测试无法稳定执行。
        if unsafe { nix::libc::isatty(stdin().as_raw_fd()) } != 1 {
            return true;
        }
        if std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")
            .is_err()
        {
            return true;
        }

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
            assert!(matches!(flags.file, Device::Stdin(_)));
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
                file_name: None,
                file: Device::Stdin(stdin()),
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

        #[test]
        fn test_validate_setting_syntax_combination_and_custom() {
            assert!(validate_setting_syntax("evenp").is_ok());
            assert!(validate_setting_syntax("-evenp").is_ok());
            assert!(validate_setting_syntax("ek").is_ok());
            assert!(validate_setting_syntax("-ek").is_err());

            assert!(validate_setting_syntax("ofill").is_ok());
            assert!(validate_setting_syntax("-ofill").is_ok());
            assert!(validate_setting_syntax("ofill extra").is_err());
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
            let device = Device::Stdin(stdin());
            let opts = SttyFlags {
                is_all: true,
                is_save: false,
                file_name: None,
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
                file_name: None,
                file: Device::Stdin(stdin()),
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
                    Ok(ControlFlow::Break(true))
                ));
            }
        }

        #[test]
        fn test_stty_apply_baud_rate_invalid() {
            let mut termios = unsafe { std::mem::zeroed::<Termios>() };
            assert!(matches!(
                stty_apply_baud_rate_flag(&mut termios, "invalid"),
                Ok(ControlFlow::Continue(()))
            ));
        }

        #[test]
        fn test_stty_apply_special_setting_size() {
            if is_container() {
                println!("Skipping test_stty_apply_special_setting_size in container environment");
                return;
            }

            let mut termios = unsafe { std::mem::zeroed::<Termios>() };
            let device = Device::Stdin(stdin());
            assert!(matches!(
                stty_apply_special_setting(&mut termios, "size", &device),
                Ok(ControlFlow::Break(true))
            ));
        }

        #[test]
        fn test_stty_apply_special_setting_min() {
            let mut termios = unsafe { std::mem::zeroed::<Termios>() };
            let device = Device::Stdin(stdin());
            assert!(matches!(
                stty_apply_special_setting(&mut termios, "min 1", &device),
                Ok(ControlFlow::Break(true))
            ));
        }

        #[test]
        fn test_stty_apply_special_setting_invalid() {
            let mut termios = unsafe { std::mem::zeroed::<Termios>() };
            let device = Device::Stdin(stdin());
            assert!(matches!(
                stty_apply_special_setting(&mut termios, "invalid", &device),
                Ok(ControlFlow::Continue(()))
            ));
        }

        #[test]
        fn test_stty_apply_combination_flags() {
            let mut termios = unsafe { std::mem::zeroed::<Termios>() };
            let device = Device::Stdin(stdin());

            assert!(matches!(
                stty_apply_setting(&mut termios, "evenp", &device),
                Ok(ControlFlow::Break(true))
            ));
            assert!(termios.control_flags.contains(C::PARENB));
            assert!(!termios.control_flags.contains(C::PARODD));

            assert!(matches!(
                stty_apply_setting(&mut termios, "-evenp", &device),
                Ok(ControlFlow::Break(true))
            ));
            assert!(!termios.control_flags.contains(C::PARENB));
            assert!(termios.control_flags.contains(C::CS8));

            assert!(matches!(
                stty_apply_setting(&mut termios, "raw", &device),
                Ok(ControlFlow::Break(true))
            ));
            assert!(termios.input_flags.bits() == 0);
            assert_eq!(
                termios.control_chars[SpecialCharacterIndices::VMIN as usize],
                1
            );
            assert_eq!(
                termios.control_chars[SpecialCharacterIndices::VTIME as usize],
                0
            );

            assert!(matches!(
                stty_apply_setting(&mut termios, "-raw", &device),
                Ok(ControlFlow::Break(true))
            ));
            assert!(termios.input_flags.contains(I::BRKINT));
            assert!(termios.output_flags.contains(O::OPOST));
            assert!(termios.local_flags.contains(L::ICANON));

            assert!(matches!(
                stty_apply_setting(&mut termios, "tabs", &device),
                Ok(ControlFlow::Break(true))
            ));
            assert!(O::TAB0.is_in(&termios, Some(O::TABDLY)));

            assert!(matches!(
                stty_apply_setting(&mut termios, "-tabs", &device),
                Ok(ControlFlow::Break(true))
            ));
            assert!(O::TAB3.is_in(&termios, Some(O::TABDLY)));
        }

        #[test]
        fn test_stty_apply_non_reversible_combination_flags() {
            let mut termios = unsafe { std::mem::zeroed::<Termios>() };
            let device = Device::Stdin(stdin());

            assert!(matches!(
                stty_apply_setting(&mut termios, "-ek", &device),
                Ok(ControlFlow::Break(false))
            ));
            assert!(matches!(
                stty_apply_setting(&mut termios, "-sane", &device),
                Ok(ControlFlow::Break(false))
            ));
            assert!(matches!(
                stty_apply_setting(&mut termios, "-crt", &device),
                Ok(ControlFlow::Break(false))
            ));
            assert!(matches!(
                stty_apply_setting(&mut termios, "-dec", &device),
                Ok(ControlFlow::Break(false))
            ));
        }

        #[test]
        fn test_stty_apply_custom_flags() {
            let mut termios = unsafe { std::mem::zeroed::<Termios>() };
            let device = Device::Stdin(stdin());

            assert!(matches!(
                stty_apply_setting(&mut termios, "ofill", &device),
                Ok(ControlFlow::Break(true))
            ));
            if OFILL_RAW_BIT != 0 {
                assert_ne!(termios.output_flags.bits() & OFILL_RAW_BIT, 0);
            }
            assert!(matches!(
                stty_apply_setting(&mut termios, "-ofill", &device),
                Ok(ControlFlow::Break(true))
            ));
            if OFILL_RAW_BIT != 0 {
                assert_eq!(termios.output_flags.bits() & OFILL_RAW_BIT, 0);
            }

            assert!(matches!(
                stty_apply_setting(&mut termios, "iuclc", &device),
                Ok(ControlFlow::Break(true))
            ));
            if IUCLC_RAW_BIT != 0 {
                assert_ne!(termios.input_flags.bits() & IUCLC_RAW_BIT, 0);
            }

            assert!(matches!(
                stty_apply_setting(&mut termios, "xcase", &device),
                Ok(ControlFlow::Break(true))
            ));
            if XCASE_RAW_BIT != 0 {
                assert_ne!(termios.local_flags.bits() & XCASE_RAW_BIT, 0);
            }
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
