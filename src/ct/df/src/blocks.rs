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
use crate::DF_OPT_BLOCKSIZE;
use crate::DF_OPT_PORTABILITY;
use clap::ArgMatches;
use std::env;
use std::fmt;

use ctcore::{
    ct_display::Quotable,
    ct_parse_size::{parse_size_u64, ParseSizeError},
};

/// The first ten powers of 1024.
const BLOCKS_IEC_BASES: [u128; 10] = [
    1,
    1_024,
    1_048_576,
    1_073_741_824,
    1_099_511_627_776,
    1_125_899_906_842_624,
    1_152_921_504_606_846_976,
    1_180_591_620_717_411_303_424,
    1_208_925_819_614_629_174_706_176,
    1_237_940_039_285_380_274_899_124_224,
];

/// The first ten powers of 1000.
const BLOCKS_SI_BASES: [u128; 10] = [
    1,
    1_000,
    1_000_000,
    1_000_000_000,
    1_000_000_000_000,
    1_000_000_000_000_000,
    1_000_000_000_000_000_000,
    1_000_000_000_000_000_000_000,
    1_000_000_000_000_000_000_000_000,
    1_000_000_000_000_000_000_000_000_000,
];

/// A SuffixType determines whether the suffixes are 1000 or 1024 based, and whether they are
/// intended for HumanReadable mode or not.
#[derive(Clone, Copy)]
pub(crate) enum BlocksSuffixType {
    Iec,
    Si,
    HumanReadable(BlocksHumanReadable),
}

impl BlocksSuffixType {
    /// 获取基于1024或1000的十进制幂。
    ///
    /// 根据BlocksSuffixType的实例化，返回一个包含前十个1024或1000的幂的数组。
    ///
    /// 返回值:
    /// - [u128; 10]：包含10个幂的数组，具体取决于实例化的类型。
    fn blocks_bases(&self) -> [u128; 10] {
        match self {
            Self::Iec | Self::HumanReadable(BlocksHumanReadable::Binary) => BLOCKS_IEC_BASES,
            Self::Si | Self::HumanReadable(BlocksHumanReadable::Decimal) => BLOCKS_SI_BASES,
        }
    }

    /// 获取单位后缀。
    ///
    /// 根据BlocksSuffixType的实例化，返回一个包含前九个单位后缀的数组。
    /// 对于IEC和SI标准，这个数组用于表示数据量的大小。对于HumanReadable选项，这取决于是二进制还是十进制标准。
    ///
    /// 返回值:
    /// - [&'static str; 9]：包含9个单位后缀的静态字符串数组。
    fn blocks_suffixes(&self) -> [&'static str; 9] {
        match self {
            // 我们使用 "kB" 而非 "KB"，与 GNU df 保持一致
            Self::Si => ["B", "kB", "MB", "GB", "TB", "PB", "EB", "ZB", "YB"],
            Self::Iec => ["B", "K", "M", "G", "T", "P", "E", "Z", "Y"],
            Self::HumanReadable(BlocksHumanReadable::Binary) => {
                ["", "K", "M", "G", "T", "P", "E", "Z", "Y"]
            }
            Self::HumanReadable(BlocksHumanReadable::Decimal) => {
                ["", "k", "M", "G", "T", "P", "E", "Z", "Y"]
            }
        }
    }
}

/// Convert a number into a magnitude and a multi-byte unit suffix.
///
/// The returned string has a maximum length of 5 chars, for example: "1.1kB", "999kB", "1MB".
/**
 * 将给定的数值转换成带有单位后缀的字符串表示。
 *
 * 此函数根据指定的单位后缀类型（如KB、MB、GB等），将给定的数值（n）转换成
 * 一个带有适当单位后缀的字符串。如果数值可以整除基础单位量（bases），则仅返回
 * 数值和单位后缀的组合；如果不能整除，将返回带有一位小数的字符串表示。
 *
 * @param n 要转换的数值，类型为u128。
 * @param suffix_type 单位后缀的类型，定义了基数和后缀。
 * @return 转换后的字符串，包含数值和单位后缀。
 */
pub(crate) fn blocks_to_magnitude_and_suffix(
    n: u128,
    blocks_suffix_type: BlocksSuffixType,
) -> String {
    // 获取单位后缀的基础量和后缀列表
    let blocks_bases = blocks_suffix_type.blocks_bases();
    let blocks_suffixes = blocks_suffix_type.blocks_suffixes();
    let mut size = 0;

    // 找到最合适的单位基数索引
    while blocks_bases[size + 1] - blocks_bases[size] < n && size < blocks_suffixes.len() {
        size += 1;
    }

    // 计算商和余数，以确定最终的数值和单位
    let blocks_quot = n / blocks_bases[size];
    let rem_size = n % blocks_bases[size];
    let blocks_suffix = blocks_suffixes[size];

    // 如果余数为0，直接返回商和单位后缀的组合
    if rem_size == 0 {
        format!("{blocks_quot}{blocks_suffix}")
    } else {
        // 计算十分位数值
        let tenths_place_size = rem_size / (blocks_bases[size] / 10);

        // 如果余数可以整除十分之一的基础单位量，返回带有一位小数的字符串
        if rem_size % (blocks_bases[size] / 10) == 0 {
            format!("{blocks_quot}.{tenths_place_size}{blocks_suffix}")
        } else if tenths_place_size + 1 == 10 || blocks_quot >= 10 {
            // 特殊情况处理，如进位或位数过多
            format!("{}{}", blocks_quot + 1, blocks_suffix)
        } else {
            // 一般情况，返回带有两位小数的字符串
            format!("{}.{}{}", blocks_quot, tenths_place_size + 1, blocks_suffix)
        }
    }
}

/// A mode to use in condensing the human readable display of a large number
/// of bytes.
///
/// The [`BlocksHumanReadable::Decimal`] and[`BlocksHumanReadable::Binary`] variants
/// represent dynamic block sizes: as the number of bytes increases, the
/// divisor increases as well (for example, from 1 to 1,000 to 1,000,000
/// and so on in the case of [`BlocksHumanReadable::Decimal`]).
#[derive(Clone, Copy)]
pub(crate) enum BlocksHumanReadable {
    /// Use the largest divisor corresponding to a unit, like B, K, M, G, etc.
    ///
    /// This variant represents powers of 1,000. Contrast with
    /// [`BlocksHumanReadable::Binary`], which represents powers of
    /// 1,024.
    Decimal,

    /// Use the largest divisor corresponding to a unit, like B, K, M, G, etc.
    ///
    /// This variant represents powers of 1,024. Contrast with
    /// [`BlocksHumanReadable::Decimal`], which represents powers
    /// of 1,000.
    Binary,
}

/// A block size to use in condensing the display of a large number of bytes.
///
/// The [`BlockSize::Bytes`] variant represents a static block
/// size.
///
/// The default variant is `Bytes(1024)`.
// BlockSize 枚举定义了块的大小，支持固定字节数。
#[derive(Debug, PartialEq)]
pub(crate) enum BlockSize {
    /// Bytes 替代表示一个固定数量的字节。
    ///
    /// 数量必须为正数。
    Bytes(u64),
}

// BlockSize 实现了 as_u64 方法，用于获取关联的 u64 值。
impl BlockSize {
    /// 返回枚举项关联的 u64 值。
    ///
    /// - 参数：无
    /// - 返回值：关联的 u64 值。
    pub(crate) fn as_u64(&self) -> u64 {
        match *self {
            Self::Bytes(n) => n,
        }
    }
}

// BlockSize 实现了 Default 特性，用于提供一个默认的块大小。
impl Default for BlockSize {
    fn default() -> Self {
        // 根据环境变量 POSIXLY_CORRECT 的设置选择默认的块大小。
        // 如果该变量存在且不为空，则默认为 512 字节；否则，默认为 1024 字节。
        if env::var("POSIXLY_CORRECT").is_ok() {
            Self::Bytes(512)
        } else {
            Self::Bytes(1024)
        }
    }
}

/**
 * 根据命令行参数或环境变量读取并解析块大小。
 *
 * 这个函数首先尝试从命令行参数中读取块大小（如果指定了`OPT_BLOCKSIZE`）。
 * 如果找到了有效的块大小值，则尝试将其解析为字节。如果解析成功且值大于0，
 * 则返回该值作为块大小。如果值不大于0或解析失败，则返回一个错误。
 *
 * 如果没有指定块大小，但指定了`OPT_PORTABILITY`标志，则返回默认的块大小。
 *
 * 如果以上条件都不满足，则尝试从环境变量中读取块大小。如果成功读取到值，
 * 则返回该值作为块大小。如果环境变量中没有设置块大小，则返回默认的块大小。
 *
 * @param args_match 命令行参数匹配结果，用于读取块大小或可移植性标志。
 * @return Result<BlockSize, ParseSizeError> 返回解析后的块大小，或者在解析过程中遇到的错误。
 */
pub(crate) fn block_size_read(args_match: &ArgMatches) -> Result<BlockSize, ParseSizeError> {
    // 尝试从命令行参数读取块大小
    if args_match.contains_id(DF_OPT_BLOCKSIZE) {
        let str = args_match.get_one::<String>(DF_OPT_BLOCKSIZE).unwrap();
        let bytes = parse_size_u64(str)?;

        // 如果解析成功且值大于0，则返回解析后的字节大小
        if bytes > 0 {
            Ok(BlockSize::Bytes(bytes))
        } else {
            // 如果解析失败或值不大于0，则返回错误
            Err(ParseSizeError::ParseFailure(format!("{}", str.quote())))
        }
        // 检查是否指定了可移植性标志
    } else if args_match.get_flag(DF_OPT_PORTABILITY) {
        Ok(BlockSize::default())
        // 尝试从环境变量读取块大小
    } else if let Some(bytes) = block_size_from_env() {
        Ok(BlockSize::Bytes(bytes))
        // 如果以上条件都不满足，则返回默认块大小
    } else {
        Ok(BlockSize::default())
    }
}

/**
 * 从环境变量中尝试获取块大小。
 *
 * 尝试读取多个环境变量来获取块大小的设置，它们的优先级从高到低分别是：
 * "DF_BLOCK_SIZE", "BLOCK_SIZE", "BLOCKSIZE"。
 * 如果找到了一个有效的值（能够被解析为u64的正数），则返回该值的Option形式；
 * 如果没有找到或找到了无法解析的值，则返回None。
 *
 * @return Option<u64> - 表示块大小的u64类型值的Option，如果没有找到有效的值则为None。
 */
fn block_size_from_env() -> Option<u64> {
    // 遍历一系列环境变量名称尝试获取块大小的设置
    for blocks_env_var in ["DF_BLOCK_SIZE", "BLOCK_SIZE", "BLOCKSIZE"] {
        // 尝试获取当前环境变量的值
        if let Ok(blocks_env_size) = env::var(blocks_env_var) {
            // 尝试将环境变量的值解析为u64类型的大小
            if let Ok(size) = parse_size_u64(&blocks_env_size) {
                // 如果解析成功，返回解析得到的大小
                return Some(size);
            } else {
                // 如果解析失败，返回None
                return None;
            }
        }
    }

    // 如果所有环境变量都没有设置或设置无效，最终返回None
    None
}

/// 实现 `fmt::Display` trait 以便于 `BlockSize` 结构体可以被格式化输出。
///
/// # 参数
/// - `self`: `BlockSize` 的一个引用，表示要进行格式化的对象。
/// - `f`: 一个 `fmt::Formatter` 的引用，用于控制输出格式。
///
/// # 返回值
/// 返回一个 `fmt::Result`，表示格式化操作的结果。如果操作成功，返回 `Ok(())`；如果失败，则返回相应的错误信息。
impl fmt::Display for BlockSize {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // 根据 `BlockSize` 的具体类型，选择使用 IEC 或 SI 后缀进行大小显示
        match self {
            Self::Bytes(n) => {
                // 根据 `n` 的值决定使用 IEC 还是 SI 单位系统
                let s = if n % 1024 == 0 && n % 1000 != 0 {
                    blocks_to_magnitude_and_suffix(*n as u128, BlocksSuffixType::Iec)
                } else {
                    blocks_to_magnitude_and_suffix(*n as u128, BlocksSuffixType::Si)
                };

                // 使用选择好的单位系统进行输出
                write!(f, "{s}")
            }
        }
    }
}

#[cfg(test)]
mod tests {

    use std::env;

    use crate::blocks::blocks_to_magnitude_and_suffix;
    use crate::blocks::BlockSize;
    use crate::blocks::BlocksHumanReadable;
    use crate::blocks::BlocksSuffixType;
    use crate::blocks::BLOCKS_IEC_BASES;
    use crate::blocks::BLOCKS_SI_BASES;

    #[test]
    fn test_to_magnitude_and_suffix_1k() {
        assert_eq!(
            blocks_to_magnitude_and_suffix(1024, BlocksSuffixType::Iec),
            "1K"
        );
    }

    #[test]
    fn test_to_magnitude_and_suffix_2k() {
        assert_eq!(
            blocks_to_magnitude_and_suffix(2048, BlocksSuffixType::Iec),
            "2K"
        );
    }

    #[test]
    fn test_to_magnitude_and_suffix_4k() {
        assert_eq!(
            blocks_to_magnitude_and_suffix(4096, BlocksSuffixType::Iec),
            "4K"
        );
    }

    #[test]
    fn test_to_magnitude_and_suffix_1m() {
        assert_eq!(
            blocks_to_magnitude_and_suffix(1024 * 1024, BlocksSuffixType::Iec),
            "1M"
        );
    }

    #[test]
    fn test_to_magnitude_and_suffix_2m() {
        assert_eq!(
            blocks_to_magnitude_and_suffix(2 * 1024 * 1024, BlocksSuffixType::Iec),
            "2M"
        );
    }

    #[test]
    fn test_to_magnitude_and_suffix_1g() {
        assert_eq!(
            blocks_to_magnitude_and_suffix(1024 * 1024 * 1024, BlocksSuffixType::Iec),
            "1G"
        );
    }

    #[test]
    fn test_to_magnitude_and_suffix_34g() {
        assert_eq!(
            blocks_to_magnitude_and_suffix(34 * 1024 * 1024 * 1024, BlocksSuffixType::Iec),
            "34G"
        );
    }

    #[allow(clippy::cognitive_complexity)]
    #[test]
    fn test_to_magnitude_and_suffix_single_byte_si() {
        assert_eq!(
            blocks_to_magnitude_and_suffix(1, BlocksSuffixType::Si),
            "1B"
        );
    }

    #[test]
    fn test_to_magnitude_and_suffix_below_kilobyte_si() {
        assert_eq!(
            blocks_to_magnitude_and_suffix(999, BlocksSuffixType::Si),
            "999B"
        );
    }

    #[test]
    fn test_to_magnitude_and_suffix_exactly_kilobyte_si() {
        assert_eq!(
            blocks_to_magnitude_and_suffix(1000, BlocksSuffixType::Si),
            "1kB"
        );
    }
}