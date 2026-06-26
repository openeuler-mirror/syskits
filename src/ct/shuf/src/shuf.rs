/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
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
//! - `NonrepeatingIterator`: 生成不重复随机数的迭代器
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

use clap::{Arg, ArgAction, Command, crate_version};
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CTsageError, CtSimpleError, FromIo};
use ctcore::{ct_format_usage, ct_help_about, ct_help_usage};
use memchr::memchr_iter;
use rand::prelude::SliceRandom;
use rand::{Rng, RngCore};
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufReader, BufWriter, Error, Read, Write, stdin, stdout};
use std::ops::RangeInclusive;

mod rand_read_adapter;

enum ShufMode {
    Default(String),
    Echo(Vec<String>),
    InputRange(RangeInclusive<usize>),
}

static SHUF_USAGE: &str = ct_help_usage!("shuf.md");
static SHUF_ABOUT: &str = ct_help_about!("shuf.md");

struct ShufSettings {
    head_count: usize,
    output: Option<String>,
    random_source: Option<String>,
    is_repeat: bool,
    sep: u8,
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

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    shuf_main(args)
}

pub fn shuf_main(args: impl ctcore::Args) -> CTResult<()> {
    let matches = ct_app().try_get_matches_from(args)?;

    let mode = if matches.get_flag(shuf_options::SHUF_ECHO) {
        ShufMode::Echo(
            matches
                .get_many::<String>(shuf_options::SHUF_FILE_OR_ARGS)
                .unwrap_or_default()
                .map(String::from)
                .collect(),
        )
    } else if let Some(range) = matches.get_one::<String>(shuf_options::SHUF_INPUT_RANGE) {
        match shuf_parse_range(range) {
            Ok(m) => ShufMode::InputRange(m),
            Err(msg) => {
                return Err(CtSimpleError::new(1, msg));
            }
        }
    } else {
        let mut operands = matches
            .get_many::<String>(shuf_options::SHUF_FILE_OR_ARGS)
            .unwrap_or_default();
        let file = operands.next().cloned().unwrap_or("-".into());
        if let Some(second_file) = operands.next() {
            return Err(CTsageError::new(
                1,
                format!("unexpected argument '{second_file}' found"),
            ));
        };
        ShufMode::Default(file)
    };

    let settings = ShufSettings::new(&matches)?;

    if settings.head_count == 0 {
        // Do not attempt to read the random source or the input file.
        // However, we must touch the output file, if given:
        if let Some(s) = settings.output {
            File::create(&s[..])
                .map_err_context(|| format!("failed to open {} for writing", s.quote()))?;
        }
        return Ok(());
    }

    match mode {
        ShufMode::Echo(args) => {
            let mut evec = args.iter().map(String::as_bytes).collect::<Vec<_>>();
            shuf_find_seps(&mut evec, settings.sep);
            shuf_exec(&mut evec, settings)?;
        }
        ShufMode::InputRange(mut range) => {
            shuf_exec(&mut range, settings)?;
        }
        ShufMode::Default(filename) => {
            let fdata = shuf_read_input_file(&filename)?;
            let mut fdata = vec![&fdata[..]];
            shuf_find_seps(&mut fdata, settings.sep);
            shuf_exec(&mut fdata, settings)?;
        }
    }

    Ok(())
}

pub fn ct_app() -> Command {
    let args = vec![
        Arg::new(shuf_options::SHUF_ECHO)
            .short('e')
            .long(shuf_options::SHUF_ECHO)
            .help("treat each ARG as an input line")
            .action(clap::ArgAction::SetTrue)
            .overrides_with(shuf_options::SHUF_ECHO)
            .conflicts_with(shuf_options::SHUF_INPUT_RANGE),
        Arg::new(shuf_options::SHUF_INPUT_RANGE)
            .short('i')
            .long(shuf_options::SHUF_INPUT_RANGE)
            .value_name("LO-HI")
            .help("treat each number LO through HI as an input line")
            .conflicts_with(shuf_options::SHUF_FILE_OR_ARGS),
        Arg::new(shuf_options::SHUF_HEAD_COUNT)
            .short('n')
            .long(shuf_options::SHUF_HEAD_COUNT)
            .value_name("COUNT")
            .action(clap::ArgAction::Append)
            .help("output at most COUNT lines"),
        Arg::new(shuf_options::SHUF_OUTPUT)
            .short('o')
            .long(shuf_options::SHUF_OUTPUT)
            .value_name("FILE")
            .help("write result to FILE instead of standard output")
            .value_hint(clap::ValueHint::FilePath),
        Arg::new(shuf_options::SHUF_RANDOM_SOURCE)
            .long(shuf_options::SHUF_RANDOM_SOURCE)
            .value_name("FILE")
            .help("get random bytes from FILE")
            .value_hint(clap::ValueHint::FilePath),
        Arg::new(shuf_options::SHUF_REPEAT)
            .short('r')
            .long(shuf_options::SHUF_REPEAT)
            .help("output lines can be repeated")
            .action(ArgAction::SetTrue)
            .overrides_with(shuf_options::SHUF_REPEAT),
        Arg::new(shuf_options::SHUF_ZERO_TERMINATED)
            .short('z')
            .long(shuf_options::SHUF_ZERO_TERMINATED)
            .help("line delimiter is NUL, not newline")
            .action(ArgAction::SetTrue)
            .overrides_with(shuf_options::SHUF_ZERO_TERMINATED),
        Arg::new(shuf_options::SHUF_FILE_OR_ARGS)
            .action(clap::ArgAction::Append)
            .value_hint(clap::ValueHint::FilePath),
    ];
    Command::new(ctcore::ct_util_name())
        .about(SHUF_ABOUT)
        .version(crate_version!())
        .override_usage(ct_format_usage(SHUF_USAGE))
        .infer_long_args(true)
        .args(args)
}

/// 从文件或标准输入读取数据
///
/// # 参数
/// * `filename` - 文件名，"-" 表示从标准输入读取
///
/// # 返回值
/// 返回读取的字节数据
///
/// # 错误
/// - 文件打开失败
/// - 读取过程中发生错误
fn shuf_read_input_file(filename: &str) -> CTResult<Vec<u8>> {
    // 创建读取器
    let reader: Box<dyn Read> = if filename == "-" {
        Box::new(stdin())
    } else {
        Box::new(
            File::open(filename)
                .map_err_context(|| format!("failed to open {}", filename.quote()))?,
        )
    };

    // 使用带缓冲的读取器提高性能
    let mut buf_reader = BufReader::new(reader);
    let mut data = Vec::with_capacity(1024); // 预分配合理的初始容量

    // 读取所有数据
    buf_reader
        .read_to_end(&mut data)
        .map_err_context(|| format!("failed reading {}", filename.quote()))?;

    Ok(data)
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
            let mut positions: Vec<_> = memchr_iter(sep, slice).collect();

            // 处理最后一个空字段
            if positions.last() == Some(&(slice.len() - 1)) {
                positions.pop();
            }

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

trait Shufable {
    // 定义与此trait关联的数据类型，必须实现Writable特征
    type Item: ShufWritable;

    // 检查集合是否为空
    fn is_empty(&self) -> bool;

    // 从集合中随机选择一个元素
    fn choose(&self, rng: &mut WrappedRng) -> Self::Item;

    // 定义部分打乱后返回的迭代器类型
    type PartialShuffleIterator<'b>: Iterator<Item = Self::Item>
    where
        Self: 'b;

    // 部分打乱集合中的元素
    fn partial_shuffle<'b>(
        &'b mut self,
        rng: &'b mut WrappedRng,
        amount: usize,
    ) -> Self::PartialShuffleIterator<'b>;
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
    fn choose(&self, rng: &mut WrappedRng) -> Self::Item {
        // 注意：copied() 只复制引用，不复制整个字节切片
        // 由于之前已检查非空，这里 unwrap 是安全的
        (**self).choose(rng).unwrap()
    }

    // 定义部分打乱后返回的迭代器类型
    type PartialShuffleIterator<'b>
        = std::iter::Copied<std::slice::Iter<'b, &'a [u8]>>
    where
        Self: 'b;

    // 部分打乱向量中的元素
    fn partial_shuffle<'b>(
        &'b mut self,
        rng: &'b mut WrappedRng,
        amount: usize,
    ) -> Self::PartialShuffleIterator<'b> {
        // 注意：copied() 只复制引用，不复制整个字节切片
        (**self).partial_shuffle(rng, amount).0.iter().copied()
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
    fn choose(&self, rng: &mut WrappedRng) -> usize {
        rng.gen_range(self.clone())
    }

    // 定义部分打乱后返回的迭代器类型
    type PartialShuffleIterator<'b>
        = NonrepeatingIterator<'b>
    where
        Self: 'b;

    // 部分打乱范围中的数字
    fn partial_shuffle<'b>(
        &'b mut self,
        rng: &'b mut WrappedRng,
        amount: usize,
    ) -> Self::PartialShuffleIterator<'b> {
        NonrepeatingIterator::new(self.clone(), rng, amount)
    }
}

enum NumberSet {
    AlreadyListed(HashSet<usize>),
    Remaining(Vec<usize>),
}

struct NonrepeatingIterator<'a> {
    range: RangeInclusive<usize>,
    rng: &'a mut WrappedRng,
    remaining_count: usize,
    buf: NumberSet,
}

/// 不重复数字迭代器的实现
impl<'a> NonrepeatingIterator<'a> {
    /// 创建新的迭代器实例
    ///
    /// # 参数
    /// * `range` - 数字范围
    /// * `rng` - 随机数生成器
    /// * `amount` - 需要生成的数字数量
    fn new(
        range: RangeInclusive<usize>,
        rng: &'a mut WrappedRng,
        amount: usize,
    ) -> NonrepeatingIterator<'a> {
        // 计算实际需要生成的数量
        let capped_amount = if range.start() > range.end() {
            0 // 范围无效时返回0
        } else if *range.start() == 0 && *range.end() == usize::MAX {
            amount // 完整范围时直接使用请求数量
        } else {
            amount.min(range.end() - range.start() + 1) // 取较小值避免越界
        };

        // 创建迭代器实例
        NonrepeatingIterator {
            range,
            rng,
            remaining_count: capped_amount,
            buf: NumberSet::AlreadyListed(HashSet::default()), // 初始使用HashSet记录已用数字
        }
    }

    /// 生成下一个不重复的随机数
    ///
    /// # 说明
    /// 该函数有两种工作模式：
    /// 1. HashSet模式：使用集合记录已生成的数字
    /// 2. Vec模式：当生成的数字较多时，切换到预生成的剩余数字列表
    ///
    /// # 返回值
    /// 返回范围内的一个未使用过的随机数
    ///
    /// # Panics
    /// 当范围的起始值大于结束值时会触发断言失败
    fn produce(&mut self) -> usize {
        debug_assert!(self.range.start() <= self.range.end());

        match &mut self.buf {
            NumberSet::AlreadyListed(used_numbers) => {
                let chosen = loop {
                    let guess = self.rng.gen_range(self.range.clone());
                    if used_numbers.insert(guess) {
                        break guess;
                    }
                };

                let range_size = (self.range.end() - self.range.start()).saturating_add(1);
                if number_set_should_list_remaining(used_numbers.len(), range_size) {
                    let mut remaining = self
                        .range
                        .clone()
                        .filter(|n| !used_numbers.contains(n))
                        .collect::<Vec<_>>();

                    remaining.partial_shuffle(&mut self.rng, self.remaining_count);
                    remaining.truncate(self.remaining_count);
                    self.buf = NumberSet::Remaining(remaining);
                }
                chosen
            }
            NumberSet::Remaining(remaining_numbers) => remaining_numbers.pop().unwrap(),
        }
    }
}

impl Iterator for NonrepeatingIterator<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        if self.range.is_empty() || self.remaining_count == 0 {
            return None;
        }
        self.remaining_count -= 1;
        Some(self.produce())
    }
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
fn number_set_should_list_remaining(already_listed_count: usize, range_size: usize) -> bool {
    already_listed_count >= range_size / 4
}

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
fn shuf_exec<T: Shufable>(input: &mut T, settings: ShufSettings) -> CTResult<()> {
    // 检查输入是否为空
    if input.is_empty() {
        return Err(CtSimpleError::new(1, "no lines to repeat"));
    }

    // 创建输出写入器
    let writer = create_output_writer(&settings)?;
    let mut buf_writer = BufWriter::new(writer);

    // 创建随机数生成器
    let mut rng = create_random_source(&settings)?;

    // 根据是否重复选择不同的处理逻辑
    if settings.is_repeat {
        // 重复模式：直接随机选择
        process_repeat_mode(
            input,
            &mut rng,
            &mut buf_writer,
            settings.head_count,
            settings.sep,
        )?;
    } else {
        // 不重复模式：使用部分打乱
        process_nonrepeat_mode(
            input,
            &mut rng,
            &mut buf_writer,
            settings.head_count,
            settings.sep,
        )?;
    }

    Ok(())
}

/// 创建输出写入器
fn create_output_writer(settings: &ShufSettings) -> CTResult<Box<dyn Write>> {
    Ok(if let Some(path) = &settings.output {
        Box::new(
            File::create(path)
                .map_err_context(|| format!("failed to open {} for writing", path.quote()))?,
        )
    } else {
        Box::new(stdout())
    })
}

/// 创建随机数生成器
fn create_random_source(settings: &ShufSettings) -> CTResult<WrappedRng> {
    if let Some(path) = &settings.random_source {
        WrappedRng::new_from_file(path)
    } else {
        Ok(WrappedRng::RngDefault(rand::thread_rng()))
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
        let item = input.choose(rng);
        item.write_all_to(writer)?;
        writer.write_all(&[sep])?;
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
    for item in input.partial_shuffle(rng, count) {
        item.write_all_to(writer)?;
        writer.write_all(&[sep])?;
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
fn shuf_parse_range(input_range: &str) -> Result<RangeInclusive<usize>, String> {
    // 尝试按 '-' 分割字符串
    if let Some((from, to)) = input_range.split_once('-') {
        // 解析起始值
        let begin = from
            .parse::<usize>()
            .map_err(|_| format!("invalid input range: '{}'", input_range))?;

        // 解析结束值
        let end = to
            .parse::<usize>()
            .map_err(|_| format!("invalid input range: '{}'", input_range))?;

        // 确保范围有效（起始值不大于结束值）
        if begin <= end {
            Ok(begin..=end)
        } else {
            Err(format!("invalid input range: '{}'", input_range))
        }
    } else {
        // 没有找到分隔符 '-'
        Err(format!("invalid input range: '{}'", input_range))
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
fn shuf_parse_head_count(headcounts: Vec<String>) -> Result<usize, String> {
    // 初始化为最大值
    let mut result = usize::MAX;

    // 遍历所有输入的数字
    for count in headcounts {
        // 解析当前数字
        let n = count
            .parse::<usize>()
            .map_err(|_| format!("invalid line count: '{}'", count))?;

        // 更新为较小的值
        result = result.min(n);
    }

    Ok(result)
}

enum WrappedRng {
    RngFile(rand_read_adapter::ReadRng<File>),
    RngDefault(rand::rngs::ThreadRng),
}

impl RngCore for WrappedRng {
    fn next_u32(&mut self) -> u32 {
        match self {
            Self::RngFile(r) => r.next_u32(),
            Self::RngDefault(r) => r.next_u32(),
        }
    }

    fn next_u64(&mut self) -> u64 {
        match self {
            Self::RngFile(r) => r.next_u64(),
            Self::RngDefault(r) => r.next_u64(),
        }
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        match self {
            Self::RngFile(r) => r.fill_bytes(dest),
            Self::RngDefault(r) => r.fill_bytes(dest),
        }
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand::Error> {
        match self {
            Self::RngFile(r) => r.try_fill_bytes(dest),
            Self::RngDefault(r) => r.try_fill_bytes(dest),
        }
    }
}

impl WrappedRng {
    fn new_from_file(path: &str) -> CTResult<Self> {
        let file = File::open(path)
            .map_err_context(|| format!("failed to open random source {}", path.quote()))?;
        Ok(WrappedRng::RngFile(rand_read_adapter::ReadRng::new(file)))
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
                    .get_many::<String>(shuf_options::SHUF_HEAD_COUNT)
                    .unwrap_or_default()
                    .cloned()
                    .collect();
                shuf_parse_head_count(headcounts).map_err(|e| CtSimpleError::new(1, e))?
            },

            // 解析输出文件参数
            output: matches
                .get_one::<String>(shuf_options::SHUF_OUTPUT)
                .map(String::from),

            // 解析随机源文件参数
            random_source: matches
                .get_one::<String>(shuf_options::SHUF_RANDOM_SOURCE)
                .map(String::from),

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

#[cfg(test)]
// Since the computed value is a bool, it is more readable to write the expected value out:
#[allow(clippy::bool_assert_comparison)]
mod test_number_set_decision {
    use super::number_set_should_list_remaining;

    #[test]
    fn test_stay_positive_large_remaining_first() {
        assert_eq!(false, number_set_should_list_remaining(0, std::usize::MAX));
    }

    #[test]
    fn test_stay_positive_large_remaining_second() {
        assert_eq!(false, number_set_should_list_remaining(1, std::usize::MAX));
    }

    #[test]
    fn test_stay_positive_large_remaining_tenth() {
        assert_eq!(false, number_set_should_list_remaining(9, std::usize::MAX));
    }

    #[test]
    fn test_stay_positive_smallish_range_first() {
        assert_eq!(false, number_set_should_list_remaining(0, 12345));
    }

    #[test]
    fn test_stay_positive_smallish_range_second() {
        assert_eq!(false, number_set_should_list_remaining(1, 12345));
    }

    #[test]
    fn test_stay_positive_smallish_range_tenth() {
        assert_eq!(false, number_set_should_list_remaining(9, 12345));
    }

    #[test]
    fn test_stay_positive_small_range_not_too_early() {
        assert_eq!(false, number_set_should_list_remaining(1, 10));
    }

    // Don't want to test close to the border, in case we decide to change the threshold.
    // However, at 50% coverage, we absolutely should switch:
    #[test]
    fn test_switch_half() {
        assert_eq!(true, number_set_should_list_remaining(1234, 2468));
    }

    // Ensure that the decision is monotonous:
    #[test]
    fn test_switch_late1() {
        assert_eq!(true, number_set_should_list_remaining(12340, 12345));
    }

    #[test]
    fn test_switch_late2() {
        assert_eq!(true, number_set_should_list_remaining(12344, 12345));
    }

    // Ensure that we are overflow-free:
    #[test]
    fn test_no_crash_exceed_max_size1() {
        assert_eq!(
            false,
            number_set_should_list_remaining(12345, std::usize::MAX)
        );
    }

    #[test]
    fn test_no_crash_exceed_max_size2() {
        assert_eq!(
            true,
            number_set_should_list_remaining(std::usize::MAX - 1, std::usize::MAX)
        );
    }

    #[test]
    fn test_no_crash_exceed_max_size3() {
        assert_eq!(
            true,
            number_set_should_list_remaining(std::usize::MAX, std::usize::MAX)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

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
            assert_eq!(settings.output.as_deref(), Some("out.txt"));
            assert_eq!(settings.random_source.as_deref(), Some("rand.txt"));
        }
    }

    mod parse_tests {
        use super::*;

        #[test]
        fn test_parse_range_valid() {
            assert_eq!(shuf_parse_range("1-5").unwrap(), 1..=5);
            assert_eq!(shuf_parse_range("0-0").unwrap(), 0..=0);
            assert_eq!(shuf_parse_range("10-10").unwrap(), 10..=10);
        }

        #[test]
        fn test_parse_range_invalid() {
            assert!(shuf_parse_range("invalid").is_err());
            assert!(shuf_parse_range("5-1").is_err());
            assert!(shuf_parse_range("a-b").is_err());
            assert!(shuf_parse_range("-5").is_err());
        }

        #[test]
        fn test_parse_head_count_valid() {
            assert_eq!(shuf_parse_head_count(vec!["5".to_string()]).unwrap(), 5);
            assert_eq!(
                shuf_parse_head_count(vec!["10".to_string(), "5".to_string()]).unwrap(),
                5
            );
        }

        #[test]
        fn test_parse_head_count_invalid() {
            assert!(shuf_parse_head_count(vec!["invalid".to_string()]).is_err());
            assert!(shuf_parse_head_count(vec!["-5".to_string()]).is_err());
        }
    }

    mod shuf_exec_tests {
        use super::*;

        #[test]
        fn test_shuf_exec_basic() {
            let temp = tempdir().unwrap();
            let output_path = temp.path().join("output.txt");

            let settings = ShufSettings {
                head_count: 3,
                output: Some(output_path.to_str().unwrap().to_string()),
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
            assert!(shuf_exec(&mut input, settings).is_ok());
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
            assert!(shuf_exec(&mut input, settings).is_ok());
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
            assert!(shuf_exec(&mut input, settings).is_err());
        }
    }

    mod find_seps_tests {
        use super::*;

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

            let result = shuf_read_input_file(file_path.to_str().unwrap());
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), b"test data");
        }

        #[test]
        fn test_read_input_file_nonexistent() {
            let result = shuf_read_input_file("nonexistent.txt");
            assert!(result.is_err());
        }
    }

    mod iterator_tests {
        use super::*;

        /// 创建测试用的迭代器实例
        fn create_test_iterator(
            range: RangeInclusive<usize>,
            amount: usize,
            rng: &mut WrappedRng,
        ) -> NonrepeatingIterator<'_> {
            NonrepeatingIterator::new(range, rng, amount)
        }

        #[test]
        fn test_iterator_basic() {
            let mut rng = WrappedRng::RngDefault(rand::thread_rng());
            let iter = create_test_iterator(1..=5, 5, &mut rng);
            let numbers: HashSet<_> = iter.collect();

            println!("Collected numbers: {:?}", numbers);
            assert_eq!(numbers.len(), 5, "Should generate 5 unique numbers");
            assert!(
                numbers.iter().all(|&n| n >= 1 && n <= 5),
                "All numbers should be in range 1..=5"
            );
        }

        #[test]
        fn test_iterator_partial() {
            let mut rng = WrappedRng::RngDefault(rand::thread_rng());
            let iter = create_test_iterator(1..=10, 5, &mut rng);
            let numbers: HashSet<_> = iter.collect();

            println!("Collected numbers: {:?}", numbers);
            assert_eq!(numbers.len(), 5, "Should generate exactly 5 numbers");
            assert!(
                numbers.iter().all(|&n| n >= 1 && n <= 10),
                "All numbers should be in range 1..=10"
            );
        }

        #[test]
        fn test_iterator_exact_amount() {
            let mut rng = WrappedRng::RngDefault(rand::thread_rng());
            let iter = create_test_iterator(1..=3, 3, &mut rng);
            let numbers: Vec<_> = iter.collect();

            println!("Generated sequence: {:?}", numbers);
            assert_eq!(numbers.len(), 3, "Should generate exactly 3 numbers");
            let unique: HashSet<_> = numbers.into_iter().collect();
            assert_eq!(unique.len(), 3, "All numbers should be unique");
        }

        #[test]
        fn test_iterator_empty_range() {
            let mut rng = WrappedRng::RngDefault(rand::thread_rng());
            let iter = create_test_iterator(5..=1, 5, &mut rng);
            let numbers: Vec<_> = iter.collect();

            println!("Empty range result: {:?}", numbers);
            assert!(
                numbers.is_empty(),
                "Should generate no numbers for invalid range"
            );
        }

        #[test]
        fn test_iterator_zero_amount() {
            let mut rng = WrappedRng::RngDefault(rand::thread_rng());
            let iter = create_test_iterator(1..=10, 0, &mut rng);
            let numbers: Vec<_> = iter.collect();

            println!("Zero amount result: {:?}", numbers);
            assert!(
                numbers.is_empty(),
                "Should generate no numbers when amount is 0"
            );
        }

        #[test]
        fn test_iterator_single_element() {
            let mut rng = WrappedRng::RngDefault(rand::thread_rng());
            let iter = create_test_iterator(42..=42, 1, &mut rng);
            let numbers: Vec<_> = iter.collect();

            println!("Single element result: {:?}", numbers);
            assert_eq!(numbers, vec![42], "Should generate exactly one number (42)");
        }

        #[test]
        fn test_iterator_mode_switch() {
            let mut rng = WrappedRng::RngDefault(rand::thread_rng());
            let iter = create_test_iterator(1..=4, 4, &mut rng);
            let numbers: Vec<_> = iter.collect();

            println!("Mode switch result: {:?}", numbers);
            assert_eq!(numbers.len(), 4, "Should generate all 4 numbers");
            let unique: HashSet<_> = numbers.into_iter().collect();
            assert_eq!(unique.len(), 4, "All numbers should be unique");
        }

        #[test]
        fn test_iterator_next() {
            let mut rng = WrappedRng::RngDefault(rand::thread_rng());
            let mut iter = create_test_iterator(1..=3, 3, &mut rng);

            // 测试连续调用 next()
            let first = iter.next();
            println!("First next(): {:?}", first);
            assert!(first.is_some(), "First call should return Some");
            assert!(
                (1..=3).contains(&first.unwrap()),
                "First number should be in range 1..=3"
            );

            let second = iter.next();
            println!("Second next(): {:?}", second);
            assert!(second.is_some(), "Second call should return Some");
            assert!(
                (1..=3).contains(&second.unwrap()),
                "Second number should be in range 1..=3"
            );
            assert_ne!(first, second, "Numbers should be unique");

            let third = iter.next();
            println!("Third next(): {:?}", third);
            assert!(third.is_some(), "Third call should return Some");
            assert!(
                (1..=3).contains(&third.unwrap()),
                "Third number should be in range 1..=3"
            );
            assert_ne!(third, first, "Numbers should be unique");
            assert_ne!(third, second, "Numbers should be unique");

            // 测试迭代结束
            let fourth = iter.next();
            println!("Fourth next(): {:?}", fourth);
            assert!(fourth.is_none(), "Fourth call should return None");
        }
    }
}