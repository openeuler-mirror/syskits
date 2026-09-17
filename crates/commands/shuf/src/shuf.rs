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

//! shuf 命令的核心实现
//!
//! # 功能概述
//! 该模块实现了类似 GNU shuf 的功能，用于随机打乱输入行或数字范围。
//!
//! # 主要组件
//! - `ShufSettings`: 配置选项（输出数量、重复模式、分隔符等）
//! - `Shufable`: 可打乱数据的特征
//! - `shuf_partial_permutation_indices`: 使用稀疏交换生成不重复排列
//!
//! # 核心功能
//! - 从文件或标准输入读取数据
//! - 支持数字范围输入
//! - 支持重复/不重复模式
//! - 支持自定义分隔符
//! - 支持指定随机源
//!
//! # 实现说明
//! - 使用 HashSet 和 Vec 两种模式处理不重复随机数
//! - 根据数据量自动切换处理模式以优化性能
//! - 支持大范围数字的高效处理
//! - 提供详细的错误处理和报告

// spell-checker:ignore (ToDO) cmdline evec nonrepeating seps shufable rvec fdata

extern crate rust_i18n;
use clap::{Arg, ArgAction, Command, builder::OsStringValueParser, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::Tool;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTError, CTResult, CTsageError, CtSimpleError, FromIo};
use ctcore::ct_posix::GnuGetoptCommandExt;
use ctcore::ct_quoting_style::escape_shell_bytes_with_classifier;

use memchr::memchr_iter;
use rand::RngCore;
use std::collections::HashMap;
#[cfg(test)]
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Error, ErrorKind, Read, Write, stdout};
use std::ops::RangeInclusive;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use sys_locale::get_locale;

mod rand_read_adapter;

const RESERVOIR_MIN_INPUT: u64 = 8192 * 1024;

#[cfg(unix)]
unsafe extern "C" {
    fn mbrtowc(
        wide: *mut ctcore::libc::wchar_t,
        bytes: *const ctcore::libc::c_char,
        length: usize,
        state: *mut ctcore::libc::mbstate_t,
    ) -> usize;
    fn iswprint(wide: ctcore::libc::c_uint) -> ctcore::libc::c_int;
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ShufMode {
    Default(OsString),
    Echo(Vec<OsString>),
    InputRange(RangeInclusive<usize>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ShufSettings {
    head_count: usize,
    output: Option<OsString>,
    random_source: Option<OsString>,
    is_repeat: bool,
    sep: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShufRow {
    pub row_index: usize,
    pub item_kind: String,
    pub output_text: String,
    pub line: Option<String>,
    pub number: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShufSemantic {
    pub input_kind: String,
    pub head_count: Option<usize>,
    pub repeat: bool,
    pub zero_terminated: bool,
    pub separator_text: String,
    pub output_file: Option<String>,
    pub random_source: Option<String>,
    pub input_file: Option<String>,
    pub range_start: Option<usize>,
    pub range_end: Option<usize>,
    pub rows: Vec<ShufRow>,
    pub classic_text: String,
    pub stderr_text: String,
    pub exit_code: i32,
}

mod shuf_options {
    pub static SHUF_ECHO: &str = "echo";
    pub static SHUF_INPUT_RANGE: &str = "input-range";
    pub static SHUF_HEAD_COUNT: &str = "head-count";
    pub static SHUF_OUTPUT: &str = "output";
    pub static SHUF_RANDOM_SOURCE: &str = "random-source";
    pub static SHUF_REPEAT: &str = "repeat";
    pub static SHUF_ZERO_TERMINATED: &str = "zero-terminated";
    pub static SHUF_FILE_OR_ARGS: &str = "file-or-args";
}

#[derive(Default)]
pub struct Shuf;
impl Tool for Shuf {
    fn name(&self) -> &'static str {
        "shuf"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        shuf_main(args.iter().cloned())
    }
}

pub fn shuf_main(args: impl ctcore::Args) -> CTResult<()> {
    unsafe {
        ctcore::libc::setlocale(ctcore::libc::LC_ALL, c"".as_ptr());
    }
    configure_sigpipe();

    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let (mode, settings) = shuf_parse_invocation(args)?;

    if settings.head_count == 0 {
        // GNU skips input here, but repeat mode still initializes its random source.
        if settings.is_repeat {
            let _ = create_random_source(&settings)?;
        }
        if let Some(s) = &settings.output {
            File::create(s).map_err_context(|| shuf_quotef(s.as_os_str()))?;
        }
        return Ok(());
    }

    match mode {
        ShufMode::Echo(args) => {
            let echo_input = shuf_echo_input(&args, settings.sep);
            let mut evec = vec![echo_input.as_slice()];
            shuf_find_seps(&mut evec, settings.sep);
            shuf_exec(&mut evec, &settings)?;
        }
        ShufMode::InputRange(mut range) => {
            shuf_exec(&mut range, &settings)?;
        }
        ShufMode::Default(filename) => {
            shuf_exec_default_input(&filename, &settings)?;
        }
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn configure_sigpipe() {
    if !parent_ignores_sigpipe() {
        let _ = ctcore::ct_signals::enable_pipe_errors();
    }
}

#[cfg(not(target_os = "linux"))]
fn configure_sigpipe() {}

#[cfg(target_os = "linux")]
fn parent_ignores_sigpipe() -> bool {
    let parent = unsafe { ctcore::libc::getppid() };
    let Ok(status) = std::fs::read_to_string(format!("/proc/{parent}/status")) else {
        return false;
    };
    sigpipe_is_ignored_in_status(&status)
}

#[cfg(target_os = "linux")]
fn sigpipe_is_ignored_in_status(status: &str) -> bool {
    let Some(mask) = status
        .lines()
        .find_map(|line| line.strip_prefix("SigIgn:\t"))
        .and_then(|mask| u64::from_str_radix(mask, 16).ok())
    else {
        return false;
    };
    mask & (1_u64 << (ctcore::libc::SIGPIPE - 1)) != 0
}

fn shuf_parse_invocation(args: impl ctcore::Args) -> CTResult<(ShufMode, ShufSettings)> {
    let matches = ct_app().try_get_matches_from(args)?;
    let echo = matches.get_flag(shuf_options::SHUF_ECHO);
    let input_ranges = matches
        .get_many::<OsString>(shuf_options::SHUF_INPUT_RANGE)
        .unwrap_or_default()
        .cloned()
        .collect::<Vec<_>>();
    let operands = matches
        .get_many::<OsString>(shuf_options::SHUF_FILE_OR_ARGS)
        .unwrap_or_default()
        .cloned()
        .collect::<Vec<_>>();

    if input_ranges.len() > 1 {
        return Err(CtSimpleError::new(1, "multiple -i options specified"));
    }
    if echo && !input_ranges.is_empty() {
        return Err(CTsageError::new(1, "cannot combine -e and -i options"));
    }

    let mode = if echo {
        ShufMode::Echo(operands)
    } else if let Some(range) = input_ranges.first() {
        if let Some(extra_operand) = operands.first() {
            return Err(CTsageError::new(
                1,
                format!("extra operand {}", extra_operand.quote()),
            ));
        }
        match shuf_parse_range(range.as_os_str()) {
            Ok(m) => ShufMode::InputRange(m),
            Err(msg) => {
                return Err(CtSimpleError::new(1, msg));
            }
        }
    } else {
        let file = operands
            .first()
            .cloned()
            .unwrap_or_else(|| OsString::from("-"));
        if let Some(second_file) = operands.get(1) {
            return Err(CTsageError::new(
                1,
                format!("extra operand {}", second_file.quote()),
            ));
        };
        ShufMode::Default(file)
    };

    let settings = ShufSettings::new(&matches)?;
    Ok((mode, settings))
}

pub fn ct_app() -> Command {
    let args = vec![
        Arg::new(shuf_options::SHUF_ECHO)
            .short('e')
            .long(shuf_options::SHUF_ECHO)
            .help(t!("shuf.clap.shuf_echo"))
            .action(clap::ArgAction::SetTrue)
            .overrides_with(shuf_options::SHUF_ECHO),
        Arg::new(shuf_options::SHUF_INPUT_RANGE)
            .short('i')
            .long(shuf_options::SHUF_INPUT_RANGE)
            .value_name("LO-HI")
            .allow_hyphen_values(true)
            .help(t!("shuf.clap.shuf_input_range"))
            .action(clap::ArgAction::Append)
            .value_parser(OsStringValueParser::new()),
        Arg::new(shuf_options::SHUF_HEAD_COUNT)
            .short('n')
            .long(shuf_options::SHUF_HEAD_COUNT)
            .value_name("COUNT")
            .allow_hyphen_values(true)
            .action(clap::ArgAction::Append)
            .help(t!("shuf.clap.shuf_head_count"))
            .value_parser(OsStringValueParser::new()),
        Arg::new(shuf_options::SHUF_OUTPUT)
            .short('o')
            .long(shuf_options::SHUF_OUTPUT)
            .value_name("FILE")
            .help(t!("shuf.clap.shuf_output"))
            .action(clap::ArgAction::Append)
            .value_parser(OsStringValueParser::new())
            .value_hint(clap::ValueHint::FilePath),
        Arg::new(shuf_options::SHUF_RANDOM_SOURCE)
            .long(shuf_options::SHUF_RANDOM_SOURCE)
            .value_name("FILE")
            .help(t!("shuf.clap.shuf_random_source"))
            .action(clap::ArgAction::Append)
            .value_parser(OsStringValueParser::new())
            .value_hint(clap::ValueHint::FilePath),
        Arg::new(shuf_options::SHUF_REPEAT)
            .short('r')
            .long(shuf_options::SHUF_REPEAT)
            .help(t!("shuf.clap.shuf_repeat"))
            .action(ArgAction::SetTrue)
            .overrides_with(shuf_options::SHUF_REPEAT),
        Arg::new(shuf_options::SHUF_ZERO_TERMINATED)
            .short('z')
            .long(shuf_options::SHUF_ZERO_TERMINATED)
            .help(t!("shuf.clap.shuf_zero_terminated"))
            .action(ArgAction::SetTrue)
            .overrides_with(shuf_options::SHUF_ZERO_TERMINATED),
        Arg::new(shuf_options::SHUF_FILE_OR_ARGS)
            .action(clap::ArgAction::Append)
            .value_parser(OsStringValueParser::new())
            .value_hint(clap::ValueHint::FilePath),
    ];
    Command::new(ctcore::ct_util_name())
        .about(t!("shuf.about"))
        .version(crate_version!())
        .override_usage(t!("shuf.usage"))
        .infer_long_args(true)
        .args(args)
        .gnu_getopt()
}

fn shuf_exec_default_input(filename: &OsStr, settings: &ShufSettings) -> CTResult<()> {
    let (reader, input_size) = shuf_open_input(filename)?;
    if shuf_should_use_reservoir(settings, input_size) {
        return shuf_exec_reservoir(reader, settings);
    }

    let fdata = shuf_read_input(reader)?;
    let mut fdata = vec![&fdata[..]];
    shuf_find_seps(&mut fdata, settings.sep);
    shuf_exec(&mut fdata, settings)
}

fn shuf_should_use_reservoir(settings: &ShufSettings, input_size: Option<u64>) -> bool {
    !settings.is_repeat
        && settings.head_count != usize::MAX
        && input_size.is_none_or(|size| size > RESERVOIR_MIN_INPUT)
}

fn shuf_open_input(filename: &OsStr) -> CTResult<(Box<dyn Read>, Option<u64>)> {
    if filename.as_encoded_bytes() == b"-" {
        return Ok((ctcore::ct_io::stdin_reader_box(), shuf_stdin_input_size()));
    }

    let file = File::open(filename).map_err_context(|| shuf_quotef(filename))?;
    let input_size = file
        .metadata()
        .ok()
        .filter(|metadata| metadata.file_type().is_file())
        .map(|metadata| metadata.len());
    Ok((Box::new(file), input_size))
}

#[cfg(target_os = "linux")]
fn shuf_stdin_input_size() -> Option<u64> {
    std::fs::metadata("/proc/self/fd/0")
        .ok()
        .filter(|metadata| metadata.file_type().is_file())
        .map(|metadata| metadata.len())
}

#[cfg(not(target_os = "linux"))]
fn shuf_stdin_input_size() -> Option<u64> {
    None
}

/// 从文件或标准输入读取数据。
fn shuf_read_input_file(filename: &OsStr) -> CTResult<Vec<u8>> {
    let (reader, _) = shuf_open_input(filename)?;
    shuf_read_input(reader)
}

fn shuf_read_input(reader: impl Read) -> CTResult<Vec<u8>> {
    let mut buf_reader = BufReader::new(reader);
    let mut data = Vec::with_capacity(1024);
    buf_reader
        .read_to_end(&mut data)
        .map_err_context(|| String::from("read error"))?;
    Ok(data)
}

fn shuf_exec_reservoir(reader: Box<dyn Read>, settings: &ShufSettings) -> CTResult<()> {
    let mut rng = create_random_source(settings)?;
    let reservoir = shuf_reservoir_sample(reader, settings.head_count, settings.sep, &mut rng)?;
    let mut input = reservoir.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let writer = create_output_writer(settings)?;
    let mut buf_writer = BufWriter::new(writer);
    shuf_exec_with_rng(&mut input, settings, &mut rng, &mut buf_writer)?;
    buf_writer
        .flush()
        .map_err_context(|| String::from("write error"))?;
    Ok(())
}

fn shuf_reservoir_sample<R: Read>(
    reader: R,
    count: usize,
    sep: u8,
    rng: &mut WrappedRng,
) -> CTResult<Vec<Vec<u8>>> {
    let mut reader = BufReader::new(reader);
    let mut reservoir = Vec::with_capacity(count.min(1024));
    let mut record = Vec::new();
    let mut total = 0_usize;

    while total < count && shuf_read_record(&mut reader, sep, &mut record)? {
        reservoir.push(std::mem::take(&mut record));
        total += 1;
    }

    if total == count {
        loop {
            let choices = total
                .checked_add(1)
                .ok_or_else(|| CtSimpleError::new(1, "too many input lines"))?;
            let selected = rng.choose_index(choices)?;
            if !shuf_read_record(&mut reader, sep, &mut record)? {
                break;
            }
            total += 1;
            if selected < reservoir.len() {
                std::mem::swap(&mut reservoir[selected], &mut record);
            }
        }
    }

    Ok(reservoir)
}

fn shuf_read_record(reader: &mut impl BufRead, sep: u8, record: &mut Vec<u8>) -> CTResult<bool> {
    record.clear();
    if reader
        .read_until(sep, record)
        .map_err_context(|| String::from("read error"))?
        == 0
    {
        return Ok(false);
    }
    if record.last() == Some(&sep) {
        record.pop();
    }
    Ok(true)
}

/// 在数据中查找分隔符并分割数据
///
/// # 参数
/// * `data` - 要处理的数据切片向量
/// * `sep` - 分隔符
///
/// # 说明
/// - 如果输入为空或只包含一个空元素，则清空数据
/// - 否则按分隔符分割所有数据
fn shuf_find_seps(data: &mut Vec<&[u8]>, sep: u8) {
    // 特殊情况：空输入
    if data.len() == 1 && data[0].is_empty() {
        data.clear();
        return;
    }

    // 从后向前处理，避免频繁移动数据
    for i in (0..data.len()).rev() {
        let current = data[i];

        // 如果当前切片包含分隔符
        if current.contains(&sep) {
            // 移除当前元素并获取所有权
            let slice = data.swap_remove(i);

            // 收集所有分隔符的位置
            let positions: Vec<_> = memchr_iter(sep, slice).collect();

            // 根据分隔符位置分割数据
            let mut start = 0;
            for &pos in &positions {
                data.push(&slice[start..pos]);
                start = pos + 1;
            }

            // 添加最后一个字段
            if start < slice.len() {
                data.push(&slice[start..]);
            }
        }
    }
}

fn shuf_echo_input(args: &[OsString], sep: u8) -> Vec<u8> {
    let mut input = Vec::with_capacity(
        args.iter()
            .map(|arg| arg.as_encoded_bytes().len() + 1)
            .sum(),
    );
    for arg in args {
        input.extend_from_slice(arg.as_encoded_bytes());
        input.push(sep);
    }
    input
}

trait Shufable {
    type Item: ShufWritable;

    fn is_empty(&self) -> bool;

    fn choose(&self, rng: &mut WrappedRng) -> CTResult<Self::Item>;

    fn partial_shuffle(&mut self, rng: &mut WrappedRng, amount: usize)
    -> CTResult<Vec<Self::Item>>;
}

/// 为字节切片向量实现 Shufable trait
impl<'a> Shufable for Vec<&'a [u8]> {
    // 定义关联类型为字节切片引用
    type Item = &'a [u8];

    // 检查向量是否为空
    fn is_empty(&self) -> bool {
        (**self).is_empty()
    }

    // 从向量中随机选择一个元素
    fn choose(&self, rng: &mut WrappedRng) -> CTResult<Self::Item> {
        let items = &**self;
        Ok(items[rng.choose_index(items.len())?])
    }

    // 部分打乱向量中的元素
    fn partial_shuffle(
        &mut self,
        rng: &mut WrappedRng,
        amount: usize,
    ) -> CTResult<Vec<Self::Item>> {
        let items = &**self;
        Ok(shuf_partial_permutation_indices(rng, items.len(), amount)?
            .into_iter()
            .map(|index| items[index])
            .collect())
    }
}

/// 为数字范围实现 Shufable trait
impl Shufable for RangeInclusive<usize> {
    // 定义关联类型为 usize
    type Item = usize;

    // 检查范围是否为空
    fn is_empty(&self) -> bool {
        self.is_empty()
    }

    // 从范围中随机选择一个数字
    fn choose(&self, rng: &mut WrappedRng) -> CTResult<usize> {
        let item_count = self.end() - self.start() + 1;
        Ok(*self.start() + rng.choose_index(item_count)?)
    }

    // 部分打乱范围中的数字
    fn partial_shuffle(
        &mut self,
        rng: &mut WrappedRng,
        amount: usize,
    ) -> CTResult<Vec<Self::Item>> {
        let start = *self.start();
        let item_count = self.end() - self.start() + 1;
        Ok(shuf_partial_permutation_indices(rng, item_count, amount)?
            .into_iter()
            .map(|index| start + index)
            .collect())
    }
}

fn shuf_partial_permutation_indices(
    rng: &mut WrappedRng,
    item_count: usize,
    amount: usize,
) -> CTResult<Vec<usize>> {
    let selected_count = amount.min(item_count);
    let mut swaps = HashMap::with_capacity(selected_count.saturating_mul(2));
    let mut permutation = Vec::with_capacity(selected_count);

    for index in 0..selected_count {
        let selected_index = index + rng.choose_index(item_count - index)?;
        let index_value = *swaps.get(&index).unwrap_or(&index);
        let selected_value = *swaps.get(&selected_index).unwrap_or(&selected_index);
        swaps.insert(index, selected_value);
        swaps.insert(selected_index, index_value);
        permutation.push(selected_value);
    }

    Ok(permutation)
}

/// 判断是否应该切换到列表模式
///
/// # 参数
/// * `already_listed_count` - 已经生成的数字数量
/// * `range_size` - 总的数字范围大小
///
/// # 返回值
/// 如果应该切换到列表模式则返回 true，否则返回 false
///
/// # 说明
/// 当已生成的数字数量达到一定比例时，继续使用 HashSet 查找未使用的数字会变得低效。
/// 此时应该切换到预生成剩余数字列表的模式。
///
/// 切换条件：
/// 1. 如果范围很小（<= 10），则不切换
/// 2. 如果范围很大（接近 usize::MAX），则不切换
/// 3. 如果已生成数字占总范围的比例较大，则切换
///
/// 这个策略可以在时间和空间效率之间取得平衡。
trait ShufWritable {
    fn write_all_to(&self, output: &mut impl Write) -> Result<(), Error>;
}

impl ShufWritable for &[u8] {
    fn write_all_to(&self, output: &mut impl Write) -> Result<(), Error> {
        output.write_all(self)
    }
}

impl ShufWritable for usize {
    fn write_all_to(&self, output: &mut impl Write) -> Result<(), Error> {
        output.write_all(format!("{self}").as_bytes())
    }
}

/// 执行随机打乱操作
///
/// # 参数
/// * `input` - 要打乱的输入数据
/// * `settings` - 打乱设置
///
/// # 返回值
/// 成功返回 Ok(())，失败返回错误
///
/// # 错误
/// - 输入为空时返回错误
/// - 打开输出文件失败时返回错误
/// - 写入数据失败时返回错误
fn shuf_exec_to_writer<T: Shufable, W: Write>(
    input: &mut T,
    settings: &ShufSettings,
    writer: &mut W,
) -> CTResult<()> {
    // 检查输入是否为空
    if input.is_empty() {
        if settings.is_repeat {
            return Err(CtSimpleError::new(1, "no lines to repeat"));
        }
        return Ok(());
    }

    let mut rng = create_random_source(settings)?;
    shuf_exec_with_rng(input, settings, &mut rng, writer)
}

fn shuf_exec_with_rng<T: Shufable, W: Write>(
    input: &mut T,
    settings: &ShufSettings,
    rng: &mut WrappedRng,
    writer: &mut W,
) -> CTResult<()> {
    if settings.is_repeat {
        process_repeat_mode(input, rng, writer, settings.head_count, settings.sep)?;
    } else {
        process_nonrepeat_mode(input, rng, writer, settings.head_count, settings.sep)?;
    }

    Ok(())
}

fn shuf_exec<T: Shufable>(input: &mut T, settings: &ShufSettings) -> CTResult<()> {
    if input.is_empty() {
        if settings.is_repeat {
            let _ = create_random_source(settings)?;
        }
        let writer = create_output_writer(settings)?;
        let mut buf_writer = BufWriter::new(writer);
        if settings.is_repeat {
            return Err(CtSimpleError::new(1, "no lines to repeat"));
        }
        buf_writer
            .flush()
            .map_err_context(|| String::from("write error"))?;
        return Ok(());
    }

    let mut rng = create_random_source(settings)?;
    let writer = create_output_writer(settings)?;
    let mut buf_writer = BufWriter::new(writer);
    shuf_exec_with_rng(input, settings, &mut rng, &mut buf_writer)?;
    buf_writer
        .flush()
        .map_err_context(|| String::from("write error"))?;
    Ok(())
}

/// 创建输出写入器
fn create_output_writer(settings: &ShufSettings) -> CTResult<Box<dyn Write>> {
    Ok(if let Some(path) = &settings.output {
        Box::new(File::create(path).map_err_context(|| shuf_quotef(path.as_os_str()))?)
    } else {
        Box::new(stdout())
    })
}

fn shuf_quotef(path: &OsStr) -> String {
    #[cfg(unix)]
    {
        let bytes = path.as_bytes();
        let quoted = escape_shell_bytes_with_classifier(bytes, shuf_classify_locale_sequence);

        String::from_utf8(quoted).expect("shell-escaped file names are valid UTF-8")
    }
    #[cfg(not(unix))]
    {
        path.to_string_lossy().into_owned()
    }
}

/// 创建随机数生成器
fn create_random_source(settings: &ShufSettings) -> CTResult<WrappedRng> {
    if let Some(path) = &settings.random_source {
        WrappedRng::new_from_file(path)
    } else {
        Ok(WrappedRng::RngDefault {
            reader: rand::thread_rng(),
            randnum: 0,
            randmax: 0,
        })
    }
}

/// 处理重复模式
fn process_repeat_mode<T: Shufable>(
    input: &mut T,
    rng: &mut WrappedRng,
    writer: &mut impl Write,
    count: usize,
    sep: u8,
) -> CTResult<()> {
    for _ in 0..count {
        let item = input.choose(rng)?;
        item.write_all_to(writer)
            .map_err_context(|| String::from("write error"))?;
        writer
            .write_all(&[sep])
            .map_err_context(|| String::from("write error"))?;
    }
    Ok(())
}

/// 处理不重复模式
fn process_nonrepeat_mode<T: Shufable>(
    input: &mut T,
    rng: &mut WrappedRng,
    writer: &mut impl Write,
    count: usize,
    sep: u8,
) -> CTResult<()> {
    let shuffled = input.partial_shuffle(rng, count)?;

    for item in shuffled {
        item.write_all_to(writer)
            .map_err_context(|| String::from("write error"))?;
        writer
            .write_all(&[sep])
            .map_err_context(|| String::from("write error"))?;
    }
    Ok(())
}

/// 解析范围字符串为数字范围
///
/// # 参数
/// * `input_range` - 格式为 "LO-HI" 的范围字符串
///
/// # 返回值
/// * `Ok(RangeInclusive<usize>)` - 解析成功返回包含范围
/// * `Err(String)` - 解析失败返回错误信息
fn shuf_parse_range(input_range: &OsStr) -> Result<RangeInclusive<usize>, String> {
    let invalid = || {
        format!(
            "invalid input range: {}",
            shuf_quote_numeric_argument(input_range)
        )
    };
    let bytes = input_range.as_encoded_bytes();

    // 尝试按 '-' 分割字符串
    if let Some(separator) = bytes.iter().position(|byte| *byte == b'-') {
        let ShufUnsigned::Value(begin) =
            shuf_parse_unsigned(&bytes[..separator]).map_err(|_| invalid())?
        else {
            return Err(invalid());
        };
        let ShufUnsigned::Value(end) =
            shuf_parse_unsigned(&bytes[separator + 1..]).map_err(|_| invalid())?
        else {
            return Err(invalid());
        };

        // 确保范围有效（起始值不大于结束值）
        if begin <= end
            && end
                .checked_sub(begin)
                .and_then(|width| width.checked_add(1))
                .is_some()
        {
            Ok(begin..=end)
        } else {
            Err(invalid())
        }
    } else {
        // 没有找到分隔符 '-'
        Err(invalid())
    }
}

enum ShufUnsigned {
    Value(usize),
    Overflow,
}

fn shuf_parse_unsigned(bytes: &[u8]) -> Result<ShufUnsigned, ()> {
    let mut index = 0;
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    if bytes.get(index) == Some(&b'+') {
        index += 1;
    }
    if bytes.get(index) == Some(&b'-') {
        return Err(());
    }

    let mut value = 0_usize;
    let mut saw_digit = false;
    let mut overflow = false;
    while let Some(byte) = bytes.get(index) {
        if !byte.is_ascii_digit() {
            return Err(());
        }
        saw_digit = true;
        if !overflow {
            match value
                .checked_mul(10)
                .and_then(|value| value.checked_add(usize::from(byte - b'0')))
            {
                Some(next) => value = next,
                None => overflow = true,
            }
        }
        index += 1;
    }

    if !saw_digit {
        return Err(());
    }
    if overflow {
        Ok(ShufUnsigned::Overflow)
    } else {
        Ok(ShufUnsigned::Value(value))
    }
}

/// 解析并获取最小的 head count 值
///
/// # 参数
/// * `headcounts` - 包含数字字符串的向量
///
/// # 返回值
/// * `Ok(usize)` - 解析成功返回最小的有效数字
/// * `Err(String)` - 解析失败返回错误信息
fn shuf_parse_head_count(headcounts: Vec<OsString>) -> Result<usize, String> {
    // 初始化为最大值
    let mut result = usize::MAX;

    // 遍历所有输入的数字
    for count in headcounts {
        match shuf_parse_unsigned(count.as_encoded_bytes()) {
            Ok(ShufUnsigned::Value(value)) => result = result.min(value),
            Ok(ShufUnsigned::Overflow) => {}
            Err(()) => {
                return Err(format!(
                    "invalid line count: {}",
                    shuf_quote_numeric_argument(count.as_os_str())
                ));
            }
        }
    }

    Ok(result)
}

fn shuf_quote_numeric_argument(input: &OsStr) -> String {
    let bytes = input.as_encoded_bytes();
    if let Ok(text) = std::str::from_utf8(bytes) {
        return ctcore::ct_display::locale_quote(text);
    }

    let (left_quote, right_quote) = shuf_diagnostic_quote_marks();
    let right_quote_bytes = right_quote.as_bytes();
    let mut escaped = String::with_capacity(bytes.len() + left_quote.len() + right_quote.len());
    escaped.push_str(left_quote);

    let mut index = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(right_quote_bytes) {
            escaped.push('\\');
            escaped.push_str(right_quote);
            index += right_quote_bytes.len();
            continue;
        }

        let byte = bytes[index];
        if byte.is_ascii() {
            match byte {
                b'\x07' => escaped.push_str("\\a"),
                b'\x08' => escaped.push_str("\\b"),
                b'\t' => escaped.push_str("\\t"),
                b'\n' => escaped.push_str("\\n"),
                b'\x0b' => escaped.push_str("\\v"),
                b'\x0c' => escaped.push_str("\\f"),
                b'\r' => escaped.push_str("\\r"),
                b'\\' => escaped.push_str("\\\\"),
                0x00..=0x1f | 0x7f => shuf_push_octal_byte(&mut escaped, byte),
                _ => escaped.push(char::from(byte)),
            }
            index += 1;
            continue;
        }

        #[cfg(unix)]
        let (length, printable) = shuf_classify_locale_sequence(&bytes[index..]);
        #[cfg(not(unix))]
        let (length, printable) = (1, false);

        if printable {
            escaped.push_str(
                std::str::from_utf8(&bytes[index..index + length])
                    .expect("printable locale sequence must be valid UTF-8"),
            );
        } else {
            for byte in &bytes[index..index + length] {
                shuf_push_octal_byte(&mut escaped, *byte);
            }
        }
        index += length;
    }
    escaped.push_str(right_quote);
    escaped
}

fn shuf_push_octal_byte(output: &mut String, byte: u8) {
    output.push('\\');
    output.push(char::from(b'0' + (byte >> 6)));
    output.push(char::from(b'0' + ((byte >> 3) & 0o7)));
    output.push(char::from(b'0' + (byte & 0o7)));
}

#[cfg(unix)]
fn shuf_classify_locale_sequence(remaining: &[u8]) -> (usize, bool) {
    unsafe {
        let mut state: ctcore::libc::mbstate_t = std::mem::zeroed();
        let mut wide = 0 as ctcore::libc::wchar_t;
        let length = mbrtowc(
            &mut wide,
            remaining.as_ptr().cast(),
            remaining.len(),
            &mut state,
        );
        if length == usize::MAX {
            return (1, false);
        }
        if length == usize::MAX - 1 {
            return (remaining.len(), false);
        }

        let length = if length == 0 { 1 } else { length };
        let is_utf8 = std::str::from_utf8(&remaining[..length]).is_ok();
        (
            length,
            is_utf8 && iswprint(wide as ctcore::libc::c_uint) != 0,
        )
    }
}

fn shuf_diagnostic_quote_marks() -> (&'static str, &'static str) {
    let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
        .into_iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
        .unwrap_or_else(|| String::from("C"))
        .to_ascii_uppercase();

    if locale.contains("UTF-8") || locale.contains("UTF8") {
        ("‘", "’")
    } else if locale.contains("GB18030") {
        ("\u{a1ae}", "\u{a1af}")
    } else {
        ("'", "'")
    }
}

enum WrappedRng {
    RngFile {
        reader: rand_read_adapter::ReadRng<File>,
        path: OsString,
        randnum: u64,
        randmax: u64,
    },
    RngDefault {
        reader: rand::rngs::ThreadRng,
        randnum: u64,
        randmax: u64,
    },
}

impl WrappedRng {
    fn new_from_file(path: &OsStr) -> CTResult<Self> {
        let file = File::open(path).map_err_context(|| shuf_quotef(path))?;
        Ok(WrappedRng::RngFile {
            reader: rand_read_adapter::ReadRng::new(file),
            path: path.to_os_string(),
            randnum: 0,
            randmax: 0,
        })
    }

    fn random_source_error(
        path: &OsStr,
        failure: rand_read_adapter::ReadFailure,
    ) -> Box<dyn CTError> {
        let message = if failure.kind == ErrorKind::UnexpectedEof {
            format!("{}: end of file", shuf_quotef(path))
        } else if let Some(errno) = failure.raw_os_error {
            format!("{}: {}", shuf_quotef(path), Error::from_raw_os_error(errno))
        } else {
            format!("{}: {}", shuf_quotef(path), Error::from(failure.kind))
        };
        CtSimpleError::new(1, message)
    }

    fn read_random_bytes(&mut self, dest: &mut [u8]) -> CTResult<()> {
        match self {
            Self::RngFile { reader, path, .. } => {
                if reader.try_fill_bytes(dest).is_err() {
                    let failure =
                        reader
                            .take_last_error()
                            .unwrap_or(rand_read_adapter::ReadFailure {
                                kind: ErrorKind::Other,
                                raw_os_error: None,
                            });
                    return Err(Self::random_source_error(path.as_os_str(), failure));
                }
                Ok(())
            }
            Self::RngDefault { reader, .. } => {
                reader.fill_bytes(dest);
                Ok(())
            }
        }
    }

    fn random_state(&self) -> (u64, u64) {
        match self {
            Self::RngFile {
                randnum, randmax, ..
            }
            | Self::RngDefault {
                randnum, randmax, ..
            } => (*randnum, *randmax),
        }
    }

    fn set_random_state(&mut self, randnum: u64, randmax: u64) {
        match self {
            Self::RngFile {
                randnum: state_randnum,
                randmax: state_randmax,
                ..
            }
            | Self::RngDefault {
                randnum: state_randnum,
                randmax: state_randmax,
                ..
            } => {
                *state_randnum = randnum;
                *state_randmax = randmax;
            }
        }
    }

    fn choose_index(&mut self, choices: usize) -> CTResult<usize> {
        debug_assert!(choices > 0);
        Ok(self.randint_genmax((choices - 1) as u64)? as usize)
    }

    fn randint_genmax(&mut self, genmax: u64) -> CTResult<u64> {
        let (mut randnum, mut randmax) = self.random_state();
        let choices = genmax + 1;

        loop {
            if randmax < genmax {
                let mut byte_count = 0;
                let mut new_randmax = randmax;
                while new_randmax < genmax {
                    new_randmax = (new_randmax << 8) | u64::from(u8::MAX);
                    byte_count += 1;
                }

                let mut bytes = [0_u8; std::mem::size_of::<u64>()];
                self.read_random_bytes(&mut bytes[..byte_count])?;
                for byte in &bytes[..byte_count] {
                    randnum = (randnum << 8) | u64::from(*byte);
                    randmax = (randmax << 8) | u64::from(u8::MAX);
                }
            }

            if randmax == genmax {
                self.set_random_state(0, 0);
                return Ok(randnum);
            }

            let excess_choices = randmax - genmax;
            let unusable_choices = excess_choices % choices;
            let last_usable_choice = randmax - unusable_choices;
            let reduced_randnum = randnum % choices;

            if randnum <= last_usable_choice {
                self.set_random_state(randnum / choices, excess_choices / choices);
                return Ok(reduced_randnum);
            }

            randnum = reduced_randnum;
            randmax = unusable_choices - 1;
        }
    }
}

impl ShufSettings {
    /// 从命令行参数创建设置实例
    ///
    /// # 参数
    /// * `matches` - 命令行参数匹配结果
    ///
    /// # 返回值
    /// 返回解析后的设置
    pub fn new(matches: &clap::ArgMatches) -> CTResult<Self> {
        Ok(Self {
            // 解析 head_count 参数
            head_count: {
                let headcounts = matches
                    .get_many::<OsString>(shuf_options::SHUF_HEAD_COUNT)
                    .unwrap_or_default()
                    .cloned()
                    .collect();
                shuf_parse_head_count(headcounts).map_err(|e| CtSimpleError::new(1, e))?
            },

            // 解析输出文件参数
            output: shuf_repeated_path(
                matches,
                shuf_options::SHUF_OUTPUT,
                "multiple output files specified",
            )
            .map_err(|message| CtSimpleError::new(1, message))?,

            // 解析随机源文件参数
            random_source: shuf_repeated_path(
                matches,
                shuf_options::SHUF_RANDOM_SOURCE,
                "multiple random sources specified",
            )
            .map_err(|message| CtSimpleError::new(1, message))?,

            // 解析重复选项
            is_repeat: matches.get_flag(shuf_options::SHUF_REPEAT),

            // 解析分隔符选项
            sep: if matches.get_flag(shuf_options::SHUF_ZERO_TERMINATED) {
                0x00_u8
            } else {
                0x0a_u8
            },
        })
    }
}

fn shuf_repeated_path(
    matches: &clap::ArgMatches,
    option: &str,
    multiple_error: &str,
) -> Result<Option<OsString>, String> {
    let values = matches
        .get_many::<OsString>(option)
        .unwrap_or_default()
        .collect::<Vec<_>>();
    let Some(first) = values.first() else {
        return Ok(None);
    };
    if values
        .iter()
        .any(|value| value.as_os_str() != first.as_os_str())
    {
        return Err(multiple_error.to_string());
    }
    Ok(Some((*first).clone()))
}

fn shuf_input_kind(mode: &ShufMode) -> &'static str {
    match mode {
        ShufMode::Default(_) => "file",
        ShufMode::Echo(_) => "echo",
        ShufMode::InputRange(_) => "input_range",
    }
}

fn shuf_head_count_value(settings: &ShufSettings) -> Option<usize> {
    if settings.head_count == usize::MAX {
        None
    } else {
        Some(settings.head_count)
    }
}

fn shuf_range_bounds(mode: &ShufMode) -> (Option<usize>, Option<usize>) {
    match mode {
        ShufMode::InputRange(range) => (Some(*range.start()), Some(*range.end())),
        _ => (None, None),
    }
}

fn shuf_rows_from_output(output: &[u8], sep: u8, mode: &ShufMode) -> Vec<ShufRow> {
    if output.is_empty() {
        return Vec::new();
    }

    let mut items = output.split(|byte| *byte == sep).collect::<Vec<_>>();
    if output.last().copied() == Some(sep) {
        let _ = items.pop();
    }

    items
        .into_iter()
        .enumerate()
        .map(|(index, item)| {
            let output_text = String::from_utf8_lossy(item).into_owned();
            let (item_kind, line, number) = match mode {
                ShufMode::InputRange(_) => {
                    ("number".into(), None, output_text.parse::<usize>().ok())
                }
                _ => ("line".into(), Some(output_text.clone()), None),
            };

            ShufRow {
                row_index: index + 1,
                item_kind,
                output_text,
                line,
                number,
            }
        })
        .collect()
}

pub fn shuf_native_semantic(args: impl ctcore::Args) -> CTResult<ShufSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let (mode, settings) = shuf_parse_invocation(args)?;

    let output_file = settings
        .output
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned());
    let random_source = settings
        .random_source
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned());
    let mut buffered_output = Vec::new();

    if settings.head_count == 0 {
        if let Some(path) = &settings.output {
            File::create(path).map_err_context(|| shuf_quotef(path.as_os_str()))?;
        }
    } else {
        match &mode {
            ShufMode::Echo(args) => {
                let echo_input = shuf_echo_input(args, settings.sep);
                let mut evec = vec![echo_input.as_slice()];
                shuf_find_seps(&mut evec, settings.sep);
                if output_file.is_none() {
                    shuf_exec_to_writer(&mut evec, &settings, &mut buffered_output)?;
                } else {
                    shuf_exec(&mut evec, &settings)?;
                }
            }
            ShufMode::InputRange(range) => {
                let mut range = range.clone();
                if output_file.is_none() {
                    shuf_exec_to_writer(&mut range, &settings, &mut buffered_output)?;
                } else {
                    shuf_exec(&mut range, &settings)?;
                }
            }
            ShufMode::Default(filename) => {
                let fdata = shuf_read_input_file(filename)?;
                let mut fdata = vec![&fdata[..]];
                shuf_find_seps(&mut fdata, settings.sep);
                if output_file.is_none() {
                    shuf_exec_to_writer(&mut fdata, &settings, &mut buffered_output)?;
                } else {
                    shuf_exec(&mut fdata, &settings)?;
                }
            }
        }
    }

    let classic_text = String::from_utf8_lossy(&buffered_output).into_owned();
    let (range_start, range_end) = shuf_range_bounds(&mode);

    Ok(ShufSemantic {
        input_kind: shuf_input_kind(&mode).into(),
        head_count: shuf_head_count_value(&settings),
        repeat: settings.is_repeat,
        zero_terminated: settings.sep == 0,
        separator_text: String::from_utf8_lossy(&[settings.sep]).into_owned(),
        output_file,
        random_source,
        input_file: match &mode {
            ShufMode::Default(filename) => Some(filename.to_string_lossy().into_owned()),
            _ => None,
        },
        range_start,
        range_end,
        rows: shuf_rows_from_output(&buffered_output, settings.sep, &mode),
        classic_text,
        stderr_text: String::new(),
        exit_code: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::tempdir;

    #[test]
    fn test_tool_implementation() {
        let tool = Shuf;

        // 测试 name 方法
        assert_eq!(tool.name(), "shuf");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("shuf"));

        // 测试 execute 方法 - 帮助命令应该返回错误，但不会崩溃
        let args = vec![OsString::from("shuf"), OsString::from("--help")];
        assert!(tool.execute(&args).is_err());
    }

    mod settings_tests {
        use super::*;

        #[test]
        fn test_new_settings_default() {
            let matches = ct_app().try_get_matches_from(vec!["shuf"]).unwrap();
            let settings = ShufSettings::new(&matches).unwrap();
            assert_eq!(settings.head_count, usize::MAX);
            assert_eq!(settings.sep, 0x0a_u8);
            assert!(!settings.is_repeat);
            assert!(settings.output.is_none());
            assert!(settings.random_source.is_none());
        }

        #[test]
        fn test_new_settings_with_options() {
            let matches = ct_app()
                .try_get_matches_from(vec![
                    "shuf",
                    "-n",
                    "5",
                    "-z",
                    "-r",
                    "-o",
                    "out.txt",
                    "--random-source",
                    "rand.txt",
                ])
                .unwrap();

            let settings = ShufSettings::new(&matches).unwrap();
            assert_eq!(settings.head_count, 5);
            assert_eq!(settings.sep, 0x00_u8);
            assert!(settings.is_repeat);
            assert_eq!(settings.output, Some(OsString::from("out.txt")));
            assert_eq!(settings.random_source, Some(OsString::from("rand.txt")));
        }
    }

    #[cfg(target_os = "linux")]
    mod sigpipe_tests {
        use super::*;

        #[test]
        fn sigpipe_status_mask_reports_only_an_ignored_sigpipe() {
            assert!(sigpipe_is_ignored_in_status(
                "Name:\tbash\nSigIgn:\t0000000000001000\n"
            ));
            assert!(!sigpipe_is_ignored_in_status(
                "Name:\tbash\nSigIgn:\t0000000000000000\n"
            ));
            assert!(!sigpipe_is_ignored_in_status(
                "Name:\tbash\nSigIgn:\tinvalid\n"
            ));
            assert!(!sigpipe_is_ignored_in_status("Name:\tbash\n"));
        }
    }

    mod parse_tests {
        use super::*;

        static POSIXLY_CORRECT_LOCK: Mutex<()> = Mutex::new(());

        fn parse_args(args: &[&str]) -> std::vec::IntoIter<OsString> {
            args.iter()
                .map(|arg| OsString::from(*arg))
                .collect::<Vec<_>>()
                .into_iter()
        }

        #[test]
        fn test_repeated_input_range_uses_gnu_diagnostic() {
            let error =
                shuf_parse_invocation(parse_args(&["shuf", "-i", "0-1", "-i", "2-3"])).unwrap_err();

            assert_eq!(error.to_string(), "multiple -i options specified");
        }

        #[test]
        fn test_default_mode_extra_operand_uses_gnu_diagnostic() {
            let error = shuf_parse_invocation(parse_args(&["shuf", "file1", "file2"]))
                .expect_err("a second file operand must fail");

            assert_eq!(error.to_string(), "extra operand 'file2'");
            assert!(error.usage());
        }

        #[test]
        fn test_echo_and_input_range_conflict_requests_usage() {
            let error =
                shuf_parse_invocation(parse_args(&["shuf", "-e", "-i", "0-1"])).unwrap_err();

            assert_eq!(error.to_string(), "cannot combine -e and -i options");
            assert!(error.usage());
        }

        #[test]
        fn test_posixly_correct_stops_option_parsing_at_file_operand() {
            let _guard = POSIXLY_CORRECT_LOCK.lock().unwrap();
            let previous = std::env::var_os("POSIXLY_CORRECT");
            unsafe { std::env::set_var("POSIXLY_CORRECT", "1") };

            let result = shuf_parse_invocation(parse_args(&["shuf", "/dev/null", "-n", "0"]));

            match previous {
                Some(value) => unsafe { std::env::set_var("POSIXLY_CORRECT", value) },
                None => unsafe { std::env::remove_var("POSIXLY_CORRECT") },
            }

            assert!(result.is_err());
        }

        #[cfg(unix)]
        #[test]
        fn test_echo_argument_accepts_non_utf8_bytes() {
            use std::os::unix::ffi::OsStringExt;

            let raw = OsString::from_vec(vec![0xff]);
            let (mode, settings) = shuf_parse_invocation(
                vec![OsString::from("shuf"), OsString::from("-e"), raw].into_iter(),
            )
            .unwrap();
            let ShufMode::Echo(args) = mode else {
                panic!("expected echo mode");
            };

            assert_eq!(args[0].as_encoded_bytes(), [0xff]);
            assert_eq!(shuf_echo_input(&args, settings.sep), vec![0xff, b'\n']);
        }

        #[cfg(unix)]
        #[test]
        fn test_numeric_options_report_non_utf8_bytes_with_gnu_diagnostics() {
            use std::os::unix::ffi::OsStringExt;

            let head_count_error = shuf_parse_invocation(
                vec![
                    OsString::from("shuf"),
                    OsString::from("-n"),
                    OsString::from_vec(vec![0xff]),
                    OsString::from("-i"),
                    OsString::from("1-1"),
                ]
                .into_iter(),
            )
            .expect_err("non-UTF-8 -n value must reach GNU numeric validation");
            assert!(
                head_count_error.to_string().contains("\\377"),
                "diagnostic must retain the original non-UTF-8 byte as octal"
            );

            let range_error = shuf_parse_invocation(
                vec![
                    OsString::from("shuf"),
                    OsString::from("-i"),
                    OsString::from_vec(vec![0xff, b'-', b'1']),
                ]
                .into_iter(),
            )
            .expect_err("non-UTF-8 -i value must reach GNU range validation");
            assert!(
                range_error.to_string().contains("\\377-1"),
                "diagnostic must retain the original non-UTF-8 range byte as octal"
            );
        }

        #[cfg(unix)]
        #[test]
        fn test_quote_numeric_argument_uses_gnu_escape_sequences() {
            use std::os::unix::ffi::OsStringExt;

            let quoted = shuf_quote_numeric_argument(
                OsString::from_vec(vec![0xff, b'\'', b'\\', 0x07]).as_os_str(),
            );

            assert!(
                matches!(
                    quoted.as_str(),
                    "'\\377\\'\\\\\\a'" | "\u{2018}\\377'\\\\\\a\u{2019}"
                ),
                "unexpected GNU-style numeric quote: {quoted:?}"
            );
        }

        #[test]
        fn test_repeated_output_accepts_identical_path() {
            let (_, settings) = shuf_parse_invocation(parse_args(&[
                "shuf",
                "-i",
                "0-1",
                "-o",
                "/dev/null",
                "-o",
                "/dev/null",
            ]))
            .unwrap();

            assert_eq!(settings.output, Some(OsString::from("/dev/null")));
        }

        #[test]
        fn test_repeated_output_rejects_different_paths() {
            let error = shuf_parse_invocation(parse_args(&[
                "shuf",
                "-i",
                "0-1",
                "-o",
                "/dev/null",
                "-o",
                "/dev/full",
            ]))
            .unwrap_err();

            assert_eq!(error.to_string(), "multiple output files specified");
        }

        #[test]
        fn test_repeated_random_source_accepts_identical_path() {
            let (_, settings) = shuf_parse_invocation(parse_args(&[
                "shuf",
                "-i",
                "0-1",
                "--random-source=/dev/zero",
                "--random-source=/dev/zero",
            ]))
            .unwrap();

            assert_eq!(settings.random_source, Some(OsString::from("/dev/zero")));
        }

        #[test]
        fn test_repeated_random_source_rejects_different_paths() {
            let error = shuf_parse_invocation(parse_args(&[
                "shuf",
                "-i",
                "0-1",
                "--random-source=/dev/zero",
                "--random-source=/dev/null",
            ]))
            .unwrap_err();

            assert_eq!(error.to_string(), "multiple random sources specified");
        }

        #[test]
        fn test_parse_range_valid() {
            assert_eq!(shuf_parse_range(OsStr::new("1-5")).unwrap(), 1..=5);
            assert_eq!(shuf_parse_range(OsStr::new("0-0")).unwrap(), 0..=0);
            assert_eq!(shuf_parse_range(OsStr::new("10-10")).unwrap(), 10..=10);
        }

        #[test]
        fn test_parse_range_accepts_gnu_leading_whitespace() {
            assert_eq!(shuf_parse_range(OsStr::new(" 0- 1")).unwrap(), 0..=1);
        }

        #[test]
        fn test_parse_range_invalid() {
            assert!(shuf_parse_range(OsStr::new("invalid")).is_err());
            assert!(shuf_parse_range(OsStr::new("5-1")).is_err());
            assert!(shuf_parse_range(OsStr::new("a-b")).is_err());
            assert!(shuf_parse_range(OsStr::new("-5")).is_err());
        }

        #[test]
        fn test_parse_range_rejects_full_usize_interval() {
            assert_eq!(
                shuf_parse_range(OsStr::new("0-18446744073709551615")).unwrap_err(),
                format!(
                    "invalid input range: {}",
                    ctcore::ct_display::locale_quote("0-18446744073709551615")
                )
            );
        }

        #[test]
        fn test_parse_head_count_valid() {
            assert_eq!(shuf_parse_head_count(vec![OsString::from("5")]).unwrap(), 5);
            assert_eq!(
                shuf_parse_head_count(vec![OsString::from("10"), OsString::from("5")]).unwrap(),
                5
            );
        }

        #[test]
        fn test_parse_head_count_uses_gnu_overflow_and_whitespace_rules() {
            assert_eq!(
                shuf_parse_head_count(vec![OsString::from(" 1")]).unwrap(),
                1
            );
            assert_eq!(
                shuf_parse_head_count(vec![OsString::from("18446744073709551616")]).unwrap(),
                usize::MAX
            );
        }

        #[test]
        fn test_hyphen_prefixed_numeric_values_reach_gnu_validation() {
            let head_error = shuf_parse_invocation(parse_args(&["shuf", "-n", "-0"])).unwrap_err();
            assert_eq!(
                head_error.to_string(),
                format!(
                    "invalid line count: {}",
                    ctcore::ct_display::locale_quote("-0")
                )
            );

            let range_error =
                shuf_parse_invocation(parse_args(&["shuf", "-i", "-0-1"])).unwrap_err();
            assert_eq!(
                range_error.to_string(),
                format!(
                    "invalid input range: {}",
                    ctcore::ct_display::locale_quote("-0-1")
                )
            );
        }

        #[test]
        fn test_parse_head_count_invalid() {
            assert!(shuf_parse_head_count(vec![OsString::from("invalid")]).is_err());
            assert!(shuf_parse_head_count(vec![OsString::from("-5")]).is_err());
        }
    }

    mod shuf_exec_tests {
        use super::*;

        struct FailingWriter;

        impl Write for FailingWriter {
            fn write(&mut self, _: &[u8]) -> Result<usize, Error> {
                Err(Error::from_raw_os_error(ctcore::libc::ENOSPC))
            }

            fn flush(&mut self) -> Result<(), Error> {
                Ok(())
            }
        }

        #[test]
        fn test_output_write_error_has_gnu_context() {
            let settings = ShufSettings {
                head_count: 1,
                output: None,
                random_source: None,
                is_repeat: false,
                sep: b'\n',
            };
            let mut input = 1..=1;

            let error = shuf_exec_to_writer(&mut input, &settings, &mut FailingWriter).unwrap_err();

            assert_eq!(error.to_string(), "write error: No space left on device");
        }

        #[test]
        fn test_reservoir_sample_consumes_random_value_after_filling_reservoir() {
            let temp = tempdir().unwrap();
            let random_source = temp.path().join("empty-random-source");
            std::fs::write(&random_source, []).unwrap();
            let mut rng = WrappedRng::new_from_file(random_source.as_os_str()).unwrap();

            let error = shuf_reservoir_sample(&b"first\nsecond\n"[..], 2, b'\n', &mut rng)
                .expect_err("GNU probes one further record after the reservoir fills");

            assert_eq!(
                error.to_string(),
                format!("{}: end of file", shuf_quotef(random_source.as_os_str()))
            );
        }

        #[test]
        fn test_repeat_zero_head_count_validates_random_source() {
            let temp = tempdir().unwrap();
            let missing_random_source = temp.path().join("missing-random-source");
            let error = shuf_main(
                vec![
                    OsString::from("shuf"),
                    OsString::from("-r"),
                    OsString::from("-n"),
                    OsString::from("0"),
                    OsString::from("--random-source"),
                    missing_random_source.clone().into_os_string(),
                ]
                .into_iter(),
            )
            .expect_err("repeat mode must open its random source even with -n 0");

            assert_eq!(error.code(), 1);
            assert_eq!(
                error.to_string(),
                format!(
                    "{}: No such file or directory",
                    shuf_quotef(missing_random_source.as_os_str())
                )
            );
        }

        #[test]
        fn test_random_source_open_error_uses_source_path() {
            let error = match WrappedRng::new_from_file(OsStr::new("")) {
                Ok(_) => panic!("opening an empty random-source path must fail"),
                Err(error) => error,
            };

            assert_eq!(error.to_string(), "'': No such file or directory");
        }

        #[cfg(unix)]
        #[test]
        fn test_random_source_open_error_uses_gnu_quotef_for_raw_bytes() {
            use std::os::unix::ffi::OsStringExt;

            let path = OsString::from_vec(vec![0xff]);
            let error = match WrappedRng::new_from_file(path.as_os_str()) {
                Ok(_) => panic!("the raw-byte random-source path must not exist"),
                Err(error) => error,
            };

            assert_eq!(
                error.to_string(),
                format!(
                    "{}: No such file or directory",
                    shuf_quotef(path.as_os_str())
                )
            );
        }

        #[test]
        fn test_empty_repeat_input_checks_random_source_before_no_lines_error() {
            let temp = tempdir().unwrap();
            let missing_random_source = temp.path().join("missing-random-source");
            let settings = ShufSettings {
                head_count: usize::MAX,
                output: None,
                random_source: Some(missing_random_source.clone().into_os_string()),
                is_repeat: true,
                sep: b'\n',
            };
            let mut input = Vec::<&[u8]>::new();

            let error = shuf_exec(&mut input, &settings).expect_err(
                "empty repeat input must initialize its configured random source first",
            );

            assert_eq!(
                error.to_string(),
                format!(
                    "{}: No such file or directory",
                    shuf_quotef(missing_random_source.as_os_str())
                )
            );
        }

        #[test]
        fn test_empty_random_source_returns_gnu_eof_error() {
            let temp = tempdir().unwrap();
            let random_source = temp.path().join("empty-random-source");
            std::fs::write(&random_source, []).unwrap();
            let settings = ShufSettings {
                head_count: 2,
                output: None,
                random_source: Some(random_source.clone().into_os_string()),
                is_repeat: false,
                sep: b'\n',
            };
            let mut input = 1..=2;
            let mut output = Vec::new();

            let error = shuf_exec_to_writer(&mut input, &settings, &mut output).unwrap_err();

            assert_eq!(
                error.to_string(),
                format!("{}: end of file", shuf_quotef(random_source.as_os_str()))
            );
            assert!(output.is_empty());
        }

        #[test]
        fn test_single_input_does_not_consume_empty_random_source() {
            let temp = tempdir().unwrap();
            let random_source = temp.path().join("empty-random-source");
            std::fs::write(&random_source, []).unwrap();
            let settings = ShufSettings {
                head_count: 1,
                output: None,
                random_source: Some(random_source.clone().into_os_string()),
                is_repeat: false,
                sep: b'\n',
            };
            let mut input = 1..=1;
            let mut output = Vec::new();

            shuf_exec_to_writer(&mut input, &settings, &mut output).unwrap();

            assert_eq!(output, b"1\n");
        }

        #[test]
        fn test_multiple_input_lines_consume_empty_random_source_for_one_output() {
            let temp = tempdir().unwrap();
            let random_source = temp.path().join("empty-random-source");
            std::fs::write(&random_source, []).unwrap();
            let settings = ShufSettings {
                head_count: 1,
                output: None,
                random_source: Some(random_source.clone().into_os_string()),
                is_repeat: false,
                sep: b'\n',
            };
            let mut input = vec![b"first".as_slice(), b"second".as_slice()];
            let mut output = Vec::new();

            let error = shuf_exec_to_writer(&mut input, &settings, &mut output).unwrap_err();

            assert_eq!(
                error.to_string(),
                format!("{}: end of file", shuf_quotef(random_source.as_os_str()))
            );
            assert!(output.is_empty());
        }

        #[test]
        fn test_random_source_uses_gnu_minimum_byte_count() {
            let temp = tempdir().unwrap();
            let random_source = temp.path().join("one-random-byte");
            std::fs::write(&random_source, [0_u8]).unwrap();
            let settings = ShufSettings {
                head_count: 2,
                output: None,
                random_source: Some(random_source.into_os_string()),
                is_repeat: false,
                sep: b'\n',
            };
            let mut input = 1..=2;
            let mut output = Vec::new();

            shuf_exec_to_writer(&mut input, &settings, &mut output).unwrap();

            assert_eq!(output, b"1\n2\n");
        }

        #[test]
        fn test_shuf_exec_basic() {
            let temp = tempdir().unwrap();
            let output_path = temp.path().join("output.txt");

            let settings = ShufSettings {
                head_count: 3,
                output: Some(output_path.into_os_string()),
                random_source: None,
                is_repeat: false,
                sep: b'\n',
            };

            let mut input = vec![
                b"1".as_ref(),
                b"2".as_ref(),
                b"3".as_ref(),
                b"4".as_ref(),
                b"5".as_ref(),
            ];
            assert!(shuf_exec(&mut input, &settings).is_ok());
        }

        #[test]
        fn test_shuf_exec_with_repeat() {
            let settings = ShufSettings {
                head_count: 5,
                output: None,
                random_source: None,
                is_repeat: true,
                sep: b'\n',
            };

            let mut input = vec![b"1".as_ref(), b"2".as_ref(), b"3".as_ref()];
            assert!(shuf_exec(&mut input, &settings).is_ok());
        }

        #[test]
        fn test_shuf_exec_empty_input() {
            let settings = ShufSettings {
                head_count: 5,
                output: None,
                random_source: None,
                is_repeat: true,
                sep: b'\n',
            };

            let mut input: Vec<&[u8]> = vec![];
            assert!(shuf_exec(&mut input, &settings).is_err());
        }

        #[test]
        fn test_output_directory_uses_gnu_open_diagnostic() {
            let temp = tempdir().unwrap();
            let output = temp.path().join("output-directory");
            std::fs::create_dir(&output).unwrap();
            let settings = ShufSettings {
                head_count: 1,
                output: Some(output.clone().into_os_string()),
                random_source: None,
                is_repeat: false,
                sep: b'\n',
            };

            let error = match create_output_writer(&settings) {
                Ok(_) => panic!("opening a directory as shuf output must fail"),
                Err(error) => error,
            };

            assert_eq!(
                error.to_string(),
                format!("{}: Is a directory", output.display())
            );
        }
    }

    mod semantic_tests {
        use super::*;

        fn write_semantic_fixture() -> (tempfile::TempDir, String, String) {
            let temp = tempdir().expect("tempdir");
            let input = temp.path().join("input.txt");
            let random = temp.path().join("random.bin");
            std::fs::write(&input, "alpha\nbeta\ngamma\n").expect("write shuf semantic input");
            std::fs::write(&random, vec![0_u8; 4096]).expect("write shuf semantic random");
            (
                temp,
                input.display().to_string(),
                random.display().to_string(),
            )
        }

        #[test]
        fn test_shuf_native_semantic_default_rows() {
            let (_temp, input, random) = write_semantic_fixture();

            let semantic = shuf_native_semantic(
                vec![
                    OsString::from("shuf"),
                    OsString::from("-n"),
                    OsString::from("3"),
                    OsString::from("--random-source"),
                    OsString::from(&random),
                    OsString::from(&input),
                ]
                .into_iter(),
            )
            .expect("semantic");

            assert_eq!(semantic.input_kind, "file");
            assert_eq!(semantic.head_count, Some(3));
            assert!(!semantic.repeat);
            assert!(!semantic.zero_terminated);
            assert_eq!(semantic.separator_text, "\n");
            assert_eq!(semantic.input_file.as_deref(), Some(input.as_str()));
            assert_eq!(semantic.random_source.as_deref(), Some(random.as_str()));
            assert_eq!(semantic.stderr_text, "");
            assert_eq!(semantic.exit_code, 0);
            assert_eq!(semantic.rows.len(), 3);

            let lines = semantic
                .rows
                .iter()
                .map(|row| row.line.clone().expect("line row"))
                .collect::<HashSet<_>>();

            assert_eq!(
                lines,
                HashSet::from([
                    String::from("alpha"),
                    String::from("beta"),
                    String::from("gamma"),
                ])
            );
            assert_eq!(semantic.classic_text.lines().count(), 3);
        }

        #[test]
        fn test_shuf_native_semantic_missing_file_error() {
            let temp = tempdir().expect("tempdir");
            let random = temp.path().join("random.bin");
            let missing = temp.path().join("missing.txt");
            std::fs::write(&random, vec![0_u8; 4096]).expect("write shuf random");

            let err = shuf_native_semantic(
                vec![
                    OsString::from("shuf"),
                    OsString::from("--random-source"),
                    OsString::from(random.display().to_string()),
                    OsString::from(missing.display().to_string()),
                ]
                .into_iter(),
            )
            .expect_err("missing file should error");

            assert_eq!(err.code(), 1);
            assert_eq!(
                err.to_string(),
                format!("{}: No such file or directory", missing.display())
            );
        }
    }

    mod find_seps_tests {
        use super::*;

        #[test]
        fn test_echo_input_appends_separator_after_each_argument() {
            let input = shuf_echo_input(&[OsString::from("a\n")], b'\n');
            let mut lines = vec![input.as_slice()];
            shuf_find_seps(&mut lines, b'\n');

            assert_eq!(lines, vec![b"a".as_slice(), b"".as_slice()]);
        }

        #[test]
        fn test_find_seps_basic() {
            let mut data = vec![&b"1\n2\n3"[..]];
            shuf_find_seps(&mut data, b'\n');
            assert_eq!(data.len(), 3);
            assert_eq!(data, vec![b"1", b"2", b"3"]);
        }

        #[test]
        fn test_find_seps_empty() {
            let mut data = vec![b"".as_ref()];
            shuf_find_seps(&mut data, b'\n');
            assert!(data.is_empty());
        }

        #[test]
        fn test_find_seps_no_separator() {
            let mut data = vec![b"123".as_ref()];
            shuf_find_seps(&mut data, b'\n');
            assert_eq!(data.len(), 1);
            assert_eq!(data, vec![b"123"]);
        }
    }

    mod read_input_file_tests {
        use super::*;

        #[test]
        fn test_read_input_file_valid() {
            let temp = tempdir().unwrap();
            let file_path = temp.path().join("test.txt");
            std::fs::write(&file_path, "test data").unwrap();

            let result = shuf_read_input_file(file_path.as_os_str());
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), b"test data");
        }

        #[test]
        fn test_read_input_file_nonexistent() {
            let temp = tempdir().unwrap();
            let missing = temp.path().join("missing.txt");
            let error = shuf_read_input_file(missing.as_os_str()).unwrap_err();

            assert_eq!(
                error.to_string(),
                format!("{}: No such file or directory", missing.display())
            );
        }

        #[test]
        fn test_read_input_directory_uses_gnu_read_error() {
            let temp = tempdir().unwrap();
            let directory = temp.path().join("input-directory");
            std::fs::create_dir(&directory).unwrap();

            let error = shuf_read_input_file(directory.as_os_str()).unwrap_err();

            assert_eq!(error.to_string(), "read error: Is a directory");
        }
    }
}
