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

use crate::device::Device;
use crate::{TermSize, tiocgwinsz, tiocswinsz};
use ctcore::ct_error::{CTResult, CtSimpleError};
use nix::libc::c_ushort;
use nix::sys::termios::{BaudRate, SpecialCharacterIndices, Termios, cfgetospeed};
use nix::sys::termios::{
    ControlFlags as C, InputFlags as I, LocalFlags as L, OutputFlags as O,
    SpecialCharacterIndices as S,
};
use std::os::fd::AsRawFd;

#[derive(Clone, Copy, Debug)]
pub struct Settings<T> {
    pub name: &'static str,
    pub flag: T,
    pub is_show: bool,
    pub is_sane: bool,
    pub group: Option<T>,
}

#[derive(Clone, Copy, Debug)]
pub struct SpecialSettingEntry {
    pub name: &'static str,
    pub setting: SpecialSetting,
    pub requires_value: bool,
}

impl<T> Settings<T> {
    pub const fn new(name: &'static str, flag: T) -> Self {
        Self {
            name,
            flag,
            is_show: true,
            is_sane: false,
            group: None,
        }
    }

    pub const fn new_grouped(name: &'static str, flag: T, group: T) -> Self {
        Self {
            name,
            flag,
            is_show: true,
            is_sane: false,
            group: Some(group),
        }
    }

    pub const fn hidden(mut self) -> Self {
        self.is_show = false;
        self
    }

    pub const fn sane(mut self) -> Self {
        self.is_sane = true;
        self
    }
}

pub const CONTROL_SETTINGS: &[Settings<C>] = &[
    Settings::new("parenb", C::PARENB),
    Settings::new("parodd", C::PARODD),
    Settings::new("cmspar", C::CMSPAR),
    Settings::new_grouped("cs5", C::CS5, C::CSIZE),
    Settings::new_grouped("cs6", C::CS6, C::CSIZE),
    Settings::new_grouped("cs7", C::CS7, C::CSIZE),
    Settings::new_grouped("cs8", C::CS8, C::CSIZE).sane(),
    Settings::new("hupcl", C::HUPCL),
    Settings::new("cstopb", C::CSTOPB),
    Settings::new("cread", C::CREAD).sane(),
    Settings::new("clocal", C::CLOCAL),
    Settings::new("crtscts", C::CRTSCTS),
];

pub const INPUT_SETTINGS: &[Settings<I>] = &[
    Settings::new("ignbrk", I::IGNBRK),
    Settings::new("brkint", I::BRKINT).sane(),
    Settings::new("ignpar", I::IGNPAR),
    Settings::new("parmrk", I::PARMRK),
    Settings::new("inpck", I::INPCK),
    Settings::new("istrip", I::ISTRIP),
    Settings::new("inlcr", I::INLCR),
    Settings::new("igncr", I::IGNCR),
    Settings::new("icrnl", I::ICRNL).sane(),
    Settings::new("ixoff", I::IXOFF),
    Settings::new("tandem", I::IXOFF),
    Settings::new("ixon", I::IXON),
    // not supported by nix
    // Settings::new("iuclc", I::IUCLC),
    Settings::new("ixany", I::IXANY),
    Settings::new("imaxbel", I::IMAXBEL).sane(),
    Settings::new("iutf8", I::IUTF8),
];

pub const OUTPUT_SETTINGS: &[Settings<O>] = &[
    Settings::new("opost", O::OPOST).sane(),
    Settings::new("olcuc", O::OLCUC),
    Settings::new("ocrnl", O::OCRNL),
    Settings::new("onlcr", O::ONLCR).sane(),
    Settings::new("onocr", O::ONOCR),
    Settings::new("onlret", O::ONLRET),
    Settings::new("ofdel", O::OFDEL),
    Settings::new_grouped("nl0", O::NL0, O::NLDLY).sane(),
    Settings::new_grouped("nl1", O::NL1, O::NLDLY),
    Settings::new_grouped("cr0", O::CR0, O::CRDLY).sane(),
    Settings::new_grouped("cr1", O::CR1, O::CRDLY),
    Settings::new_grouped("cr2", O::CR2, O::CRDLY),
    Settings::new_grouped("cr3", O::CR3, O::CRDLY),
    Settings::new_grouped("tab0", O::TAB0, O::TABDLY).sane(),
    Settings::new_grouped("tab1", O::TAB1, O::TABDLY),
    Settings::new_grouped("tab2", O::TAB2, O::TABDLY),
    Settings::new_grouped("tab3", O::TAB3, O::TABDLY),
    Settings::new_grouped("bs0", O::BS0, O::BSDLY).sane(),
    Settings::new_grouped("bs1", O::BS1, O::BSDLY),
    Settings::new_grouped("vt0", O::VT0, O::VTDLY).sane(),
    Settings::new_grouped("vt1", O::VT1, O::VTDLY),
    Settings::new_grouped("ff0", O::FF0, O::FFDLY).sane(),
    Settings::new_grouped("ff1", O::FF1, O::FFDLY),
];

pub const LOCAL_SETTINGS: &[Settings<L>] = &[
    Settings::new("isig", L::ISIG).sane(),
    Settings::new("icanon", L::ICANON).sane(),
    Settings::new("iexten", L::IEXTEN).sane(),
    Settings::new("echo", L::ECHO).sane(),
    Settings::new("echoe", L::ECHOE).sane(),
    Settings::new("crterase", L::ECHOE).hidden().sane(),
    Settings::new("echok", L::ECHOK).sane(),
    Settings::new("echonl", L::ECHONL),
    Settings::new("noflsh", L::NOFLSH),
    // Not supported by nix
    // Flag::new("xcase", L::XCASE),
    Settings::new("tostop", L::TOSTOP),
    Settings::new("echoprt", L::ECHOPRT),
    Settings::new("prterase", L::ECHOPRT).hidden(),
    Settings::new("echoctl", L::ECHOCTL).sane(),
    Settings::new("ctlecho", L::ECHOCTL).sane().hidden(),
    Settings::new("echoke", L::ECHOKE).sane(),
    Settings::new("crtkill", L::ECHOKE).sane().hidden(),
    Settings::new("flusho", L::FLUSHO),
    Settings::new("extproc", L::EXTPROC),
];

// BSD's use u32 as baud rate, to using the enum is unnecessary.
pub const BAUD_RATES: &[(&str, u32, BaudRate)] = &[
    ("0", 0, BaudRate::B0),
    ("50", 50, BaudRate::B50),
    ("75", 75, BaudRate::B75),
    ("110", 110, BaudRate::B110),
    ("134", 134, BaudRate::B134),
    ("150", 150, BaudRate::B150),
    ("200", 200, BaudRate::B200),
    ("300", 300, BaudRate::B300),
    ("600", 600, BaudRate::B600),
    ("1200", 1200, BaudRate::B1200),
    ("1800", 1800, BaudRate::B1800),
    ("2400", 2400, BaudRate::B2400),
    ("9600", 9600, BaudRate::B9600),
    ("19200", 19200, BaudRate::B19200),
    ("38400", 38400, BaudRate::B38400),
    ("57600", 57600, BaudRate::B57600),
    ("115200", 115200, BaudRate::B115200),
    ("230400", 230400, BaudRate::B230400),
    ("500000", 500000, BaudRate::B500000),
    ("576000", 576000, BaudRate::B576000),
    ("921600", 921600, BaudRate::B921600),
    ("1000000", 1000000, BaudRate::B1000000),
    ("1152000", 1152000, BaudRate::B1152000),
    ("1500000", 1500000, BaudRate::B1500000),
    ("2000000", 2000000, BaudRate::B2000000),
    ("2500000", 2500000, BaudRate::B2500000),
    ("3000000", 3000000, BaudRate::B3000000),
    ("3500000", 3500000, BaudRate::B3500000),
    ("4000000", 4000000, BaudRate::B4000000),
];
/// Control characters for the stty command.
///
/// This constant provides a mapping between the names of control characters
/// and their corresponding values in the `S` enum.
pub const CONTROL_CHARS: &[(&str, S)] = &[
    // Sends an interrupt signal (SIGINT).
    ("intr", S::VINTR),
    // Sends a quit signal (SIGQUIT).
    ("quit", S::VQUIT),
    // Deletes the last typed character.
    ("erase", S::VERASE),
    // Deletes the current line.
    ("kill", S::VKILL),
    // Signals the end of input.
    ("eof", S::VEOF),
    // Signals the end of line.
    ("eol", S::VEOL),
    // Alternate end-of-line character.
    ("eol2", S::VEOL2),
    // Switch character (only on Linux).
    #[cfg(target_os = "linux")]
    ("swtch", S::VSWTC),
    // Starts output after it has been stopped.
    ("start", S::VSTART),
    // Stops output.
    ("stop", S::VSTOP),
    // Sends a suspend signal (SIGTSTP).
    ("susp", S::VSUSP),
    // Reprints the current line.
    ("rprnt", S::VREPRINT),
    // Deletes the last word typed.
    ("werase", S::VWERASE),
    // Enters literal mode (next character is taken literally).
    ("lnext", S::VLNEXT),
    // Discards the current line.
    ("discard", S::VDISCARD),
];

/// Special settings that can be applied to a terminal
#[derive(Clone, Copy, Debug)]
pub enum SpecialSetting {
    // Set input speed
    InputSpeed,
    // Set output speed
    OutputSpeed,
    // Set minimum characters for read
    MinChars,
    // Set read timeout
    TimeoutTenths,
    // Print/set terminal size
    Size,
    // Print current speed
    ShowSpeed,
    // Set number of rows
    Rows,
    // Set number of columns
    Columns,
    // Set line discipline
    Line,
    // Wait for output drain before applying settings
    Drain(bool),
    // Discard character
    Discard,
    // EOF character
    Eof,
    // EOL character
    Eol,
    // EOL2 character
    Eol2,
    // Erase character
    Erase,
    // Intr character
    Intr,
    // Kill character
    Kill,
    // Lnext character
    Lnext,
    // Quit character
    Quit,
    // Rprnt character
    Rprnt,
    // Start character
    Start,
    // Stop character
    Stop,
    // Susp character
    Susp,
    // Swtch character
    Swtch,
    // Werase character
    Werase,
}

pub const SPECIAL_SETTINGS: &[SpecialSettingEntry] = &[
    // Speed settings
    SpecialSettingEntry {
        name: "ispeed",
        setting: SpecialSetting::InputSpeed,
        requires_value: true,
    },
    SpecialSettingEntry {
        name: "ospeed",
        setting: SpecialSetting::OutputSpeed,
        requires_value: true,
    },
    SpecialSettingEntry {
        name: "speed",
        setting: SpecialSetting::ShowSpeed,
        requires_value: false,
    },
    // Size related settings
    SpecialSettingEntry {
        name: "size",
        setting: SpecialSetting::Size,
        requires_value: false,
    },
    SpecialSettingEntry {
        name: "rows",
        setting: SpecialSetting::Rows,
        requires_value: true,
    },
    SpecialSettingEntry {
        name: "cols",
        setting: SpecialSetting::Columns,
        requires_value: true,
    },
    SpecialSettingEntry {
        name: "columns",
        setting: SpecialSetting::Columns,
        requires_value: true,
    },
    // Other settings
    SpecialSettingEntry {
        name: "min",
        setting: SpecialSetting::MinChars,
        requires_value: true,
    },
    SpecialSettingEntry {
        name: "time",
        setting: SpecialSetting::TimeoutTenths,
        requires_value: true,
    },
    SpecialSettingEntry {
        name: "line",
        setting: SpecialSetting::Line,
        requires_value: true,
    },
    SpecialSettingEntry {
        name: "drain",
        setting: SpecialSetting::Drain(true),
        requires_value: false,
    },
    SpecialSettingEntry {
        name: "-drain",
        setting: SpecialSetting::Drain(false),
        requires_value: false,
    },
    // Special characters
    SpecialSettingEntry {
        name: "discard",
        setting: SpecialSetting::Discard,
        requires_value: true,
    },
    SpecialSettingEntry {
        name: "eof",
        setting: SpecialSetting::Eof,
        requires_value: true,
    },
    SpecialSettingEntry {
        name: "eol",
        setting: SpecialSetting::Eol,
        requires_value: true,
    },
    SpecialSettingEntry {
        name: "eol2",
        setting: SpecialSetting::Eol2,
        requires_value: true,
    },
    SpecialSettingEntry {
        name: "erase",
        setting: SpecialSetting::Erase,
        requires_value: true,
    },
    SpecialSettingEntry {
        name: "intr",
        setting: SpecialSetting::Intr,
        requires_value: true,
    },
    SpecialSettingEntry {
        name: "kill",
        setting: SpecialSetting::Kill,
        requires_value: true,
    },
    SpecialSettingEntry {
        name: "lnext",
        setting: SpecialSetting::Lnext,
        requires_value: true,
    },
    SpecialSettingEntry {
        name: "quit",
        setting: SpecialSetting::Quit,
        requires_value: true,
    },
    SpecialSettingEntry {
        name: "rprnt",
        setting: SpecialSetting::Rprnt,
        requires_value: true,
    },
    SpecialSettingEntry {
        name: "start",
        setting: SpecialSetting::Start,
        requires_value: true,
    },
    SpecialSettingEntry {
        name: "stop",
        setting: SpecialSetting::Stop,
        requires_value: true,
    },
    SpecialSettingEntry {
        name: "susp",
        setting: SpecialSetting::Susp,
        requires_value: true,
    },
    SpecialSettingEntry {
        name: "swtch",
        setting: SpecialSetting::Swtch,
        requires_value: true,
    },
    SpecialSettingEntry {
        name: "werase",
        setting: SpecialSetting::Werase,
        requires_value: true,
    },
];
impl SpecialSetting {
    /// 应用特殊设置到终端
    ///
    /// 此函数根据提供的`SpecialSetting`类型、`Termios`结构体、一个可选的字符串值以及一个设备引用，
    /// 执行相应的操作。每个分支处理一种特定的终端配置，如输入速度、输出速度或终端尺寸等。
    /// 对于需要应用更改的设置，函数会修改`Termios`结构体或查询设备信息。
    ///
    /// # 参数
    /// - `termios`: 一个可变引用，指向`Termios`结构体，用于配置终端行为。
    /// - `value`: 一个可选的字符串引用，为某些设置提供具体数值。
    /// - `device`: 一个设备引用，用于查询或应用设备特定设置。
    ///
    /// # 返回
    /// 返回一个`CTResult`，包含一个布尔值，指示操作是否成功。
    pub fn apply(
        self,
        termios: &mut Termios,
        value: Option<&str>,
        device: &Device,
    ) -> CTResult<bool> {
        match self {
            // 应用输入速度设置
            SpecialSetting::InputSpeed => self.apply_input_speed(termios, value),
            // 应用输出速度设置
            SpecialSetting::OutputSpeed => self.apply_output_speed(termios, value),
            // 应用最小字符数设置
            SpecialSetting::MinChars => self.apply_min_chars(termios, value),
            // 应用超时时间设置，单位为十分之一秒
            SpecialSetting::TimeoutTenths => self.apply_timeout(termios, value),
            // 查询设备尺寸信息
            SpecialSetting::Size => self.query_size(device),
            // 查询当前速度设置
            SpecialSetting::ShowSpeed => self.query_speed(termios),
            // 应用行数或列数设置
            SpecialSetting::Rows | SpecialSetting::Columns => self.apply_size(device, value),
            // 应用行模式设置
            SpecialSetting::Line => self.apply_line(termios, value),
            // 应用排水设置，根据enable参数开启或关闭
            SpecialSetting::Drain(enable) => self.apply_drain(enable),
            // 应用其他特殊字符设置
            _ => self.apply_special_characters(termios, value),
        }
    }

    /// 设置输入速度
    ///
    /// 此函数用于设置终端的输入速度，通过解析给定的速度字符串并应用到termios结构体中
    /// 如果提供了速度值，则解析该值并尝试设置输入速度；如果未提供，则不进行任何操作
    ///
    /// # 参数
    /// - `termios`: 一个可变引用，指向Termios结构体，用于配置终端的I/O操作
    /// - `value`: 一个可选的字符串引用，表示要设置的输入速度
    ///
    /// # 返回
    /// - `CTResult<bool>`: 结果类型包装，表示操作是否成功如果速度值无效或设置速度失败，将返回错误
    fn apply_input_speed(&self, termios: &mut Termios, value: Option<&str>) -> CTResult<bool> {
        if let Some(speed) = value {
            let speed = parse_baud_rate(speed)?;
            // 设置输入速度，并处理可能的错误
            nix::sys::termios::cfsetispeed(termios, speed)
                .map_err(|e| CtSimpleError::new(1, format!("Failed to set input speed: {}", e)))?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// 应用输出速度设置到终端
    ///
    /// 此函数旨在修改终端的输出速度（波特率）。它接受一个`Termios`结构体的可变引用，
    /// 以及一个可能包含速度值的`Option`类型参数。如果提供了速度值，函数将尝试解析
    /// 并应用这个速度。如果速度值未提供或设置过程中遇到错误，函数将按原样返回错误。
    ///
    /// # 参数
    ///
    /// - `termios`: 一个`Termios`结构体的可变引用，用于修改终端的I/O属性。
    /// - `value`: 一个`Option<&str>`类型参数，可能包含要设置的输出速度字符串表示。
    ///
    /// # 返回值
    ///
    /// - `CTResult<bool>`: 一个结果类型，包含一个布尔值。如果成功设置输出速度，返回`Ok(true)`；
    ///   如果未提供速度值或设置失败，返回`Ok(false)`。如果设置过程中遇到错误，返回一个错误类型。
    fn apply_output_speed(&self, termios: &mut Termios, value: Option<&str>) -> CTResult<bool> {
        if let Some(speed) = value {
            // 解析提供的速度字符串为`Speed`枚举类型
            let speed = parse_baud_rate(speed)?;
            nix::sys::termios::cfsetospeed(termios, speed)
                .map_err(|e| CtSimpleError::new(1, format!("Failed to set output speed: {}", e)))?;

            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// 将最小字符数设置到termios配置中
    ///
    /// # Parameters
    ///
    /// * `termios`: 一个指向Termios结构的可变引用，用于配置终端
    /// * `value`: 一个可选的字符串切片，表示要设置的最小字符数
    ///
    /// # Returns
    ///
    /// * `CTResult<bool>`: 一个结果类型，包含操作是否成功的布尔值如果提供了一个值并且成功设置，则返回Ok(true)；如果未提供值，则返回Ok(false)
    /// * 如果解析值失败，则返回一个错误
    fn apply_min_chars(&self, termios: &mut Termios, value: Option<&str>) -> CTResult<bool> {
        if let Some(val) = value {
            let parsed_val = val
                .parse::<u8>()
                .map_err(|_| CtSimpleError::new(1, "Invalid value"))?;
            // 将解析后的值设置为termios配置中的最小字符数
            termios.control_chars[SpecialCharacterIndices::VMIN as usize] = parsed_val;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// 在当前对象上应用超时设置
    ///
    /// 此函数用于根据提供的字符串值解析并设置终端的超时属性
    /// 如果提供了值，它会尝试解析为u8类型，并在成功后更新到Termios结构体中
    /// 如果解析失败或值未提供，则不会应用任何更改，并返回Ok(false)
    ///
    /// # 参数
    /// - `termios`: 一个可变引用，指向Termios结构体，用于配置终端的I/O属性
    /// - `value`: 一个可选的字符串引用，表示要设置的超时值如果为None，则不进行设置
    ///
    /// # 返回
    /// - `CTResult<bool>`: 一个结果类型，包含一个布尔值，表示是否成功应用了超时设置
    ///   如果成功应用或没有提供值，则返回Ok(true)或Ok(false)，错误情况下返回Err
    fn apply_timeout(&self, termios: &mut Termios, value: Option<&str>) -> CTResult<bool> {
        if let Some(val) = value {
            let parsed_val = val
                .parse::<u8>()
                .map_err(|_| CtSimpleError::new(1, "Invalid value"))?;
            // 将解析后的值设置为Termios结构体中的VTIME特殊字符
            termios.control_chars[SpecialCharacterIndices::VTIME as usize] = parsed_val;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// 查询设备的终端尺寸
    ///
    /// 该函数通过系统调用获取终端的行数和列数，并打印出来
    /// 它主要用于调试和信息显示目的
    ///
    /// # 参数
    ///
    /// * `device` - 一个引用，指向设备对象，用于获取终端尺寸信息
    ///
    /// # 返回值
    ///
    /// 返回一个`CTResult`，包含一个布尔值，表示操作是否成功
    /// 主要用于确认函数执行无误
    fn query_size(&self, device: &Device) -> CTResult<bool> {
        // 初始化一个默认的终端尺寸结构体
        let mut size = TermSize::default();

        // 使用系统调用安全地获取终端尺寸信息
        // 这里使用`unsafe`是因为`tiocgwinsz`是一个直接与操作系统交互的低级操作
        // 需要确保调用是正确的，避免不安全的操作
        unsafe { tiocgwinsz(device.as_raw_fd(), &mut size as *mut _)? };
        println!("rows {}; columns {}", size.rows, size.columns);
        Ok(true)
    }

    /// 查询并打印当前终端的波特率
    ///
    /// # Parameters
    ///
    /// * `termios`: 一个指向Termios结构的引用，包含了终端的配置信息
    ///
    /// # Returns
    ///
    /// 返回一个结果为布尔值的CTResult，表示操作是否成功
    fn query_speed(&self, termios: &Termios) -> CTResult<bool> {
        // 获取当前终端的输出速度（波特率）
        let speed = cfgetospeed(termios);

        // 遍历预定义的波特率列表，查找与当前速度相匹配的波特率
        for (text, _, baud_rate) in BAUD_RATES {
            // 如果找到匹配的波特率，则打印当前的波特率并终止遍历
            if *baud_rate == speed {
                println!("speed {} baud", text);
                break;
            }
        }
        Ok(true)
    }

    /// 在设备上应用指定的行或列大小。
    ///
    /// 此函数旨在为终端设备设置特定的行数或列数（即屏幕的行或列大小）。它首先检查提供的值是否有效，
    /// 然后根据设备的文件描述符获取当前的终端尺寸，接着根据要求设置行数或列数，并将更改应用回设备。
    ///
    /// # 参数
    /// - `device`: 设备引用，表示要应用大小设置的终端设备。
    /// - `value`: 一个可选的字符串引用，表示要设置的行数或列数。如果为None，则不进行任何更改。
    ///
    /// # 返回
    /// - `Ok(true)`: 如果大小设置成功应用，则返回Ok(true)。
    /// - `Ok(false)`: 如果未提供值（即value为None），则返回Ok(false)。
    /// - `Err(_)`: 如果解析失败或设备不支持，则返回一个错误。
    fn apply_size(&self, device: &Device, value: Option<&str>) -> CTResult<bool> {
        // 检查是否有值提供，如果没有则直接返回Ok(false)
        if let Some(val) = value {
            // 将提供的字符串值解析为无符号短整型，如果解析失败则返回错误
            let parsed_val = val
                .parse::<c_ushort>()
                .map_err(|_| CtSimpleError::new(1, "Invalid value"))?;

            // 获取当前终端的尺寸
            let mut size = TermSize::default();
            unsafe { tiocgwinsz(device.as_raw_fd(), &mut size as *mut _)? };

            // 根据self的类型设置行数或列数
            match self {
                SpecialSetting::Rows => size.rows = parsed_val,
                SpecialSetting::Columns => size.columns = parsed_val,
                _ => unreachable!(),
            }
            unsafe { tiocswinsz(device.as_raw_fd(), &size as *const _)? };
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// 应用新的行纪律值到Termios配置中
    ///
    /// # 参数
    /// * `termios`: 一个可变的Termios结构体引用，用于存储终端的配置
    /// * `value`: 一个可选的字符串引用，表示要设置的行纪律值
    ///
    /// # 返回
    /// * `CTResult<bool>`: 一个结果类型，包含一个布尔值，表示是否成功设置了新的行纪律值
    ///
    /// # 功能描述
    /// 该函数尝试将提供的行纪律值解析为u8类型，并将其应用到提供的Termios配置中
    /// 如果提供了值并且解析成功，则更新Termios配置并返回Ok(true)
    /// 如果解析失败或未提供值，则返回Ok(false)，并不改变Termios配置
    fn apply_line(&self, termios: &mut Termios, value: Option<&str>) -> CTResult<bool> {
        if let Some(line) = value {
            let line = line
                .parse::<u8>()
                .map_err(|_| CtSimpleError::new(1, "Invalid line discipline value"))?;
            // 将当前的Termios配置转换为nix::libc::termios类型，直接修改c_line字段
            let mut libc_termios: nix::libc::termios = termios.clone().into();
            libc_termios.c_line = line;
            // 更新原始的Termios配置
            *termios = Termios::from(libc_termios);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// 控制是否启用drain模式
    ///
    /// 本函数通过线程局部存储（TLS）来控制drain模式的启用与禁用。drain模式通常用于
    /// 在某个时间点后防止某些操作继续进行，例如在关闭服务前停止接受新的请求。
    ///
    /// # 参数
    /// - `enable`: 一个布尔值，指示是否启用drain模式。`true`表示启用，`false`表示禁用。
    ///
    /// # 返回值
    /// 返回一个`CTResult<bool>`类型，始终返回`Ok(true)`，表示操作成功。
    ///
    /// # 注意
    /// 本函数使用了`thread_local!`宏来定义线程局部静态变量`DRAIN_ENABLED`，这是因为
    /// drain模式的启用状态是针对每个线程独立维护的。每个线程都可以独立地启用或禁用drain模式，
    /// 而不会相互影响。
    fn apply_drain(&self, enable: bool) -> CTResult<bool> {
        // 定义线程局部静态变量DRAIN_ENABLED，初始值为true
        thread_local! {
            static DRAIN_ENABLED: std::cell::RefCell<bool> = const { std::cell::RefCell::new(true) };
        }
        // 修改DRAIN_ENABLED的值为传入的enable参数值
        DRAIN_ENABLED.with(|enabled| *enabled.borrow_mut() = enable);
        Ok(true)
    }

    /// 在当前终端设置中应用特殊字符。
    ///
    /// 此函数旨在为终端的特定控制字符赋予新的值。它通过接收一个可能的字符串切片，
    /// 解析该字符串以获取控制字符，并根据`SpecialSetting`变体将该控制字符分配到
    /// `Termios`结构体的相应位置。
    ///
    /// # 参数
    /// - `termios`: 一个可变引用，指向`Termios`结构体，该结构体包含了终端的设置。
    /// - `value`: 一个`Option`类型，可能包含一个字符串切片，指示新的特殊字符值。
    ///
    /// # 返回值
    /// - `CTResult<bool>`: 一个结果类型，包含一个布尔值，表示是否成功设置了特殊字符。
    ///   如果`value`为`None`或解析失败，则返回`Ok(false)`。
    fn apply_special_characters(
        &self,
        termios: &mut Termios,
        value: Option<&str>,
    ) -> CTResult<bool> {
        if let Some(char) = value {
            let control_char = parse_control_char(char)?;
            // 根据`SpecialSetting`的类型，选择相应的特殊字符索引
            let index = match self {
                SpecialSetting::Discard => SpecialCharacterIndices::VDISCARD,
                SpecialSetting::Eof => SpecialCharacterIndices::VEOF,
                SpecialSetting::Eol => SpecialCharacterIndices::VEOL,
                SpecialSetting::Eol2 => SpecialCharacterIndices::VEOL2,
                SpecialSetting::Erase => SpecialCharacterIndices::VERASE,
                SpecialSetting::Intr => SpecialCharacterIndices::VINTR,
                SpecialSetting::Kill => SpecialCharacterIndices::VKILL,
                SpecialSetting::Lnext => SpecialCharacterIndices::VLNEXT,
                SpecialSetting::Quit => SpecialCharacterIndices::VQUIT,
                SpecialSetting::Rprnt => SpecialCharacterIndices::VREPRINT,
                SpecialSetting::Start => SpecialCharacterIndices::VSTART,
                SpecialSetting::Stop => SpecialCharacterIndices::VSTOP,
                SpecialSetting::Susp => SpecialCharacterIndices::VSUSP,
                SpecialSetting::Swtch => SpecialCharacterIndices::VSWTC,
                SpecialSetting::Werase => SpecialCharacterIndices::VWERASE,
                // 如果`SpecialSetting`类型不匹配任何已知的特殊字符索引，则返回`Ok(false)`
                _ => return Ok(false),
            };
            // 将解析得到的控制字符赋值给`termios`中相应的特殊字符位置
            termios.control_chars[index as usize] = control_char;

            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// 解析串行端口的波特率
///
/// 此函数尝试将给定的字符串解析为一个有效的波特率值如果解析成功并且波特率是支持的，
/// 则返回相应的波特率值否则，如果给定的字符串不能解析为有效的波特率值，或者解析的波特率值
/// 不被系统支持，则返回错误
///
/// 参数:
/// - `s`: 一个字符串切片，表示可能的波特率值
///
/// 返回值:
/// - `Ok(BaudRate)`: 如果解析成功且波特率被支持，则返回相应的波特率值
/// - `Err(CtSimpleError)`: 如果解析失败或波特率不被支持，则返回错误
fn parse_baud_rate(s: &str) -> CTResult<BaudRate> {
    // 尝试将字符串解析为u32类型，如果解析失败，则返回一个自定义的错误
    let rate = s
        .parse::<u32>()
        .map_err(|_| CtSimpleError::new(1, "Invalid baud rate"))?;

    // 遍历预定义的波特率数组，查找匹配的波特率值
    for (_, baud_rate_val, baud_rate) in BAUD_RATES {
        // 如果找到匹配的波特率值，则返回相应的波特率
        if *baud_rate_val == rate {
            return Ok(*baud_rate);
        }
    }

    Err(CtSimpleError::new(1, "Unsupported baud rate"))
}

/// Parse a control character from a string
/// 解析控制字符
///
/// 控制字符可以是单个字符或以 '^' 开头的两个字符序列，其中第二个字符是字母
/// 单个字符直接返回其 ASCII 值，'^' 开头的序列转换为相应的控制字符
///
/// # 参数
/// - `s`: 待解析的控制字符字符串
///
/// # 返回值
/// - `Ok(nix::libc::cc_t)`: 成功解析后的控制字符
/// - `Err(CtSimpleError)`: 解析失败时返回错误
fn parse_control_char(s: &str) -> CTResult<nix::libc::cc_t> {
    // 检查字符串长度，以确定解析方式
    if s.len() == 1 {
        // 如果是单个字符，直接返回其 ASCII 值
        return Ok(s.as_bytes()[0]);
    } else if s.len() == 2 && s.starts_with('^') {
        // 如果是以 ^ 开头的两个字符，解析为控制字符
        let char = s.chars().nth(1).unwrap();
        if char.is_ascii_alphabetic() {
            return Ok((char as u8) & 0x1F); // 转换为控制字符
        }
    }

    Err(CtSimpleError::new(
        1,
        "Control character must be a single character or a valid control sequence like '^A'",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::sys::termios::{ControlFlags, Termios};
    use std::io::stdout;

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

    #[test]
    fn test_settings_struct() {
        let settings = Settings {
            name: "test",
            flag: ControlFlags::CREAD,
            is_show: true,
            is_sane: false,
            group: None,
        };

        assert_eq!(settings.name, "test");
        assert!(settings.is_show);
        assert!(!settings.is_sane);
        assert!(settings.group.is_none());
    }

    #[test]
    fn test_baud_rates() {
        for &(text, _, _baud_rate) in BAUD_RATES {
            assert!(!text.is_empty());
        }
    }

    #[test]
    fn test_control_settings() {
        for setting in CONTROL_SETTINGS {
            assert!(!setting.name.is_empty());
        }
    }

    #[test]
    fn test_input_settings() {
        for setting in INPUT_SETTINGS {
            assert!(!setting.name.is_empty());
        }
    }

    #[test]
    fn test_output_settings() {
        for setting in OUTPUT_SETTINGS {
            assert!(!setting.name.is_empty());
        }
    }

    #[test]
    fn test_local_settings() {
        for setting in LOCAL_SETTINGS {
            assert!(!setting.name.is_empty());
        }
    }

    #[test]
    fn test_control_chars() {
        for &(text, _) in CONTROL_CHARS {
            assert!(!text.is_empty());
        }
    }

    #[test]
    fn test_special_settings() {
        for entry in SPECIAL_SETTINGS {
            assert!(!entry.name.is_empty());
        }
    }

    #[test]
    fn test_special_setting_size() {
        if is_container() {
            println!("Skipping test_special_setting_size in container environment");
            return;
        }

        let mut termios = unsafe { std::mem::zeroed::<Termios>() };
        let device = Device::Stdout(stdout());
        let size_setting = SPECIAL_SETTINGS
            .iter()
            .find(|s| s.name == "size")
            .expect("size setting should exist");

        assert!(!size_setting.requires_value);
        let result = size_setting.setting.apply(&mut termios, None, &device);
        assert!(result.is_ok());
    }

    #[test]
    fn test_special_setting_min() {
        let mut termios = unsafe { std::mem::zeroed::<Termios>() };
        let device = Device::Stdout(stdout());
        let min_setting = SPECIAL_SETTINGS
            .iter()
            .find(|s| s.name == "min")
            .expect("min setting should exist");

        assert!(min_setting.requires_value);
        let result = min_setting.setting.apply(&mut termios, Some("1"), &device);
        assert!(result.is_ok());
    }

    #[test]
    fn test_special_setting_time() {
        let mut termios = unsafe { std::mem::zeroed::<Termios>() };
        let device = Device::Stdout(stdout());
        let time_setting = SPECIAL_SETTINGS
            .iter()
            .find(|s| s.name == "time")
            .expect("time setting should exist");

        assert!(time_setting.requires_value);
        let result = time_setting.setting.apply(&mut termios, Some("1"), &device);
        assert!(result.is_ok());
    }

    #[test]
    fn test_special_setting_invalid_value() {
        let mut termios = unsafe { std::mem::zeroed::<Termios>() };
        let device = Device::Stdout(stdout());
        let min_setting = SPECIAL_SETTINGS
            .iter()
            .find(|s| s.name == "min")
            .expect("min setting should exist");

        let result = min_setting
            .setting
            .apply(&mut termios, Some("invalid"), &device);
        assert!(result.is_err());
    }
}
