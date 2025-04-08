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

use std::cmp;
use std::slice::Iter;

use crate::formatteriteminfo::FormatterItemInfo;
use crate::parse_formats::ParsedFormatterItemInfo;

/// 单个数据类型的最大字节数。例如，对于128位数字设置为16。
const MAX_BYTES_PER_UNIT: usize = 8;

/// 包含用于以人类可读形式输出单行数据的信息
pub struct SpacedFormatterItemInfo {
    /// 包含输出数据的函数指针和输出格式的信息
    pub formatter_item_info: FormatterItemInfo,
    /// 包含需要添加的空格数，用于与其他输出格式对齐
    ///
    /// 如果对应的数据是单字节，数组中的每个条目包含输出每个字节时要插入的空格数。
    /// 如果对应的数据是多字节，则只使用第一个字节位置。
    /// 例如，对于32位数据类型，可以使用位置0、4、8、12等。
    /// 由于每个块的格式相同，因此只设置单个块的间距。
    pub spacing: [usize; MAX_BYTES_PER_UNIT],
    /// 如果设置为true，则在行尾添加ASCII转储
    pub add_ascii_dump: bool,
}

/// 包含所有输出行的信息
pub struct OutputInfo {
    /// 一行的字节数
    pub byte_size_line: usize,
    /// 一行在人类可读格式下的宽度
    pub print_width_line: usize,

    /// 一个块的字节数（这是 `spaced_formatters` 中最大数据类型的大小）
    pub byte_size_block: usize,
    /// 一个块在人类可读格式下的宽度（最大格式的大小）
    #[allow(dead_code)]
    pub print_width_block: usize,
    /// 所有格式
    spaced_formatters: Vec<SpacedFormatterItemInfo>,
    /// 决定是否打印重复的输出行，或者
    /// 使用"*"跳过并显示跳过了一行或多行
    pub output_duplicates: bool,
}

impl OutputInfo {
    /// 返回 `SpacedFormatterItemInfo` 向量的迭代器
    pub fn spaced_formatters_iter(&self) -> Iter<SpacedFormatterItemInfo> {
        self.spaced_formatters.iter()
    }

    /// 基于参数创建新的 `OutputInfo`
    pub fn new(
        line_bytes: usize,
        formats: &[ParsedFormatterItemInfo],
        output_duplicates: bool,
    ) -> Self {
        // 计算块的字节大小（使用最大的数据类型大小）
        let byte_size_block = formats.iter().fold(1, |max, next| {
            cmp::max(max, next.formatter_item_info.byte_size)
        });
        // 计算块的打印宽度
        let print_width_block = formats.iter().fold(1, |max, next| {
            cmp::max(
                max,
                next.formatter_item_info.print_width
                    * (byte_size_block / next.formatter_item_info.byte_size),
            )
        });
        // 计算行的打印宽度
        let print_width_line = print_width_block * (line_bytes / byte_size_block);

        // 创建格式化器信息
        let spaced_formatters =
            Self::create_spaced_formatter_info(formats, byte_size_block, print_width_block);

        Self {
            byte_size_line: line_bytes,
            print_width_line,
            byte_size_block,
            print_width_block,
            spaced_formatters,
            output_duplicates,
        }
    }

    /// 创建带间距的格式化器信息
    fn create_spaced_formatter_info(
        formats: &[ParsedFormatterItemInfo],
        byte_size_block: usize,
        print_width_block: usize,
    ) -> Vec<SpacedFormatterItemInfo> {
        formats
            .iter()
            .map(|f| SpacedFormatterItemInfo {
                formatter_item_info: f.formatter_item_info,
                add_ascii_dump: f.add_ascii_dump,
                spacing: Self::calculate_alignment(f, byte_size_block, print_width_block),
            })
            .collect()
    }

    /// 计算单行输出的对齐方式
    ///
    /// # 参数
    /// * `sf` - 实现了 TypeSizeInfo trait 的类型，提供字节大小和打印宽度信息
    /// * `byte_size_block` - 块的字节大小（最大类型的大小）
    /// * `print_width_block` - 块的打印宽度（最大格式所需的空间）
    ///
    /// # 返回值
    /// 返回一个固定大小的数组，包含每个位置需要的空格数
    ///
    /// # Panics
    /// 当 byte_size_block 超过 MAX_BYTES_PER_UNIT 时会 panic
    fn calculate_alignment(
        sf: &dyn TypeSizeInfo,
        byte_size_block: usize,
        print_width_block: usize,
    ) -> [usize; MAX_BYTES_PER_UNIT] {
        // 验证块大小不超过最大限制
        assert!(
            byte_size_block <= MAX_BYTES_PER_UNIT,
            "{}-bits types are unsupported. Current max={}-bits.",
            8 * byte_size_block,
            8 * MAX_BYTES_PER_UNIT
        );

        // 初始化空格数组
        let mut spacing = [0; MAX_BYTES_PER_UNIT];

        // 获取当前类型的基本信息
        let mut byte_size = sf.byte_size();
        let mut items_in_block = byte_size_block / byte_size;

        // 计算当前块的总宽度和需要填充的空格数
        let thisblock_width = sf.print_width() * items_in_block;
        let mut remaining_spaces = print_width_block - thisblock_width;

        // 逐步分配空格
        while items_in_block > 0 {
            // 计算每个位置应该分配的空格数
            let spaces_per_item = remaining_spaces / items_in_block;

            // 为每个位置分配空格
            for i in 0..items_in_block {
                spacing[i * byte_size] += spaces_per_item;
                remaining_spaces -= spaces_per_item;
            }

            // 准备下一轮分配
            items_in_block /= 2;
            byte_size *= 2;
        }

        spacing
    }
}

/// 类型大小信息接口
trait TypeSizeInfo {
    /// 返回类型的字节大小
    fn byte_size(&self) -> usize;
    /// 返回类型的打印宽度
    fn print_width(&self) -> usize;
}

impl TypeSizeInfo for ParsedFormatterItemInfo {
    fn byte_size(&self) -> usize {
        self.formatter_item_info.byte_size
    }
    fn print_width(&self) -> usize {
        self.formatter_item_info.print_width
    }
}

#[cfg(test)]
struct TypeInfo {
    byte_size: usize,
    print_width: usize,
}

#[cfg(test)]
impl TypeSizeInfo for TypeInfo {
    fn byte_size(&self) -> usize {
        self.byte_size
    }
    fn print_width(&self) -> usize {
        self.print_width
    }
}

#[test]
#[allow(clippy::cognitive_complexity)]
fn test_calculate_alignment() {
    // For this example `byte_size_block` is 8 and 'print_width_block' is 23:
    // 1777777777777777777777 1777777777777777777777
    //  4294967295 4294967295  4294967295 4294967295
    //   ffff ffff  ffff ffff   ffff ffff  ffff ffff

    // the first line has no additional spacing:
    assert_eq!(
        [0, 0, 0, 0, 0, 0, 0, 0],
        OutputInfo::calculate_alignment(
            &TypeInfo {
                byte_size: 8,
                print_width: 23,
            },
            8,
            23
        )
    );
    // the second line a single space at the start of the block:
    assert_eq!(
        [1, 0, 0, 0, 0, 0, 0, 0],
        OutputInfo::calculate_alignment(
            &TypeInfo {
                byte_size: 4,
                print_width: 11,
            },
            8,
            23
        )
    );
    // the third line two spaces at pos 0, and 1 space at pos 4:
    assert_eq!(
        [2, 0, 0, 0, 1, 0, 0, 0],
        OutputInfo::calculate_alignment(
            &TypeInfo {
                byte_size: 2,
                print_width: 5,
            },
            8,
            23
        )
    );

    // For this example `byte_size_block` is 8 and 'print_width_block' is 28:
    //        18446744073709551615        18446744073709551615
    //      ffffffff      ffffffff      ffffffff      ffffffff
    // 177777 177777 177777 177777 177777 177777 177777 177777
    //  ff ff  ff ff  ff ff  ff ff  ff ff  ff ff  ff ff  ff ff

    assert_eq!(
        [7, 0, 0, 0, 0, 0, 0, 0],
        OutputInfo::calculate_alignment(
            &TypeInfo {
                byte_size: 8,
                print_width: 21,
            },
            8,
            28
        )
    );
    assert_eq!(
        [5, 0, 0, 0, 5, 0, 0, 0],
        OutputInfo::calculate_alignment(
            &TypeInfo {
                byte_size: 4,
                print_width: 9,
            },
            8,
            28
        )
    );
    assert_eq!(
        [0, 0, 0, 0, 0, 0, 0, 0],
        OutputInfo::calculate_alignment(
            &TypeInfo {
                byte_size: 2,
                print_width: 7,
            },
            8,
            28
        )
    );
    assert_eq!(
        [1, 0, 1, 0, 1, 0, 1, 0],
        OutputInfo::calculate_alignment(
            &TypeInfo {
                byte_size: 1,
                print_width: 3,
            },
            8,
            28
        )
    );

    // 9 tests where 8 .. 16 spaces are spread across 8 positions
    assert_eq!(
        [1, 1, 1, 1, 1, 1, 1, 1],
        OutputInfo::calculate_alignment(
            &TypeInfo {
                byte_size: 1,
                print_width: 2,
            },
            8,
            16 + 8
        )
    );
    assert_eq!(
        [2, 1, 1, 1, 1, 1, 1, 1],
        OutputInfo::calculate_alignment(
            &TypeInfo {
                byte_size: 1,
                print_width: 2,
            },
            8,
            16 + 9
        )
    );
    assert_eq!(
        [2, 1, 1, 1, 2, 1, 1, 1],
        OutputInfo::calculate_alignment(
            &TypeInfo {
                byte_size: 1,
                print_width: 2,
            },
            8,
            16 + 10
        )
    );
    assert_eq!(
        [3, 1, 1, 1, 2, 1, 1, 1],
        OutputInfo::calculate_alignment(
            &TypeInfo {
                byte_size: 1,
                print_width: 2,
            },
            8,
            16 + 11
        )
    );
    assert_eq!(
        [2, 1, 2, 1, 2, 1, 2, 1],
        OutputInfo::calculate_alignment(
            &TypeInfo {
                byte_size: 1,
                print_width: 2,
            },
            8,
            16 + 12
        )
    );
    assert_eq!(
        [3, 1, 2, 1, 2, 1, 2, 1],
        OutputInfo::calculate_alignment(
            &TypeInfo {
                byte_size: 1,
                print_width: 2,
            },
            8,
            16 + 13
        )
    );
    assert_eq!(
        [3, 1, 2, 1, 3, 1, 2, 1],
        OutputInfo::calculate_alignment(
            &TypeInfo {
                byte_size: 1,
                print_width: 2,
            },
            8,
            16 + 14
        )
    );
    assert_eq!(
        [4, 1, 2, 1, 3, 1, 2, 1],
        OutputInfo::calculate_alignment(
            &TypeInfo {
                byte_size: 1,
                print_width: 2,
            },
            8,
            16 + 15
        )
    );
    assert_eq!(
        [2, 2, 2, 2, 2, 2, 2, 2],
        OutputInfo::calculate_alignment(
            &TypeInfo {
                byte_size: 1,
                print_width: 2,
            },
            8,
            16 + 16
        )
    );

    // 4 tests where 15 spaces are spread across 8, 4, 2 or 1 position(s)
    assert_eq!(
        [4, 1, 2, 1, 3, 1, 2, 1],
        OutputInfo::calculate_alignment(
            &TypeInfo {
                byte_size: 1,
                print_width: 2,
            },
            8,
            16 + 15
        )
    );
    assert_eq!(
        [5, 0, 3, 0, 4, 0, 3, 0],
        OutputInfo::calculate_alignment(
            &TypeInfo {
                byte_size: 2,
                print_width: 4,
            },
            8,
            16 + 15
        )
    );
    assert_eq!(
        [8, 0, 0, 0, 7, 0, 0, 0],
        OutputInfo::calculate_alignment(
            &TypeInfo {
                byte_size: 4,
                print_width: 8,
            },
            8,
            16 + 15
        )
    );
    assert_eq!(
        [15, 0, 0, 0, 0, 0, 0, 0],
        OutputInfo::calculate_alignment(
            &TypeInfo {
                byte_size: 8,
                print_width: 16,
            },
            8,
            16 + 15
        )
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_alignment_basic() {
        // 测试基本对齐情况
        let type_info = TypeInfo {
            byte_size: 1,
            print_width: 2,
        };
        let result = OutputInfo::calculate_alignment(&type_info, 4, 12);
        assert_eq!(result[0..4], [1, 1, 1, 1]);
    }

    #[test]
    fn test_calculate_alignment_single_item() {
        // 测试单个大项的对齐
        let type_info = TypeInfo {
            byte_size: 8,
            print_width: 16,
        };
        let result = OutputInfo::calculate_alignment(&type_info, 8, 20);
        assert_eq!(result[0], 4); // 所有空格都应该在开头
        assert_eq!(&result[1..], &[0; 7]); // 其余位置应该是0
    }

    #[test]
    fn test_calculate_alignment_no_spacing() {
        // 测试不需要额外空格的情况
        let type_info = TypeInfo {
            byte_size: 2,
            print_width: 4,
        };
        let result = OutputInfo::calculate_alignment(&type_info, 4, 8);
        assert_eq!(result[0..4], [0, 0, 0, 0]);
    }

    #[test]
    fn test_calculate_alignment_uneven_distribution() {
        // 测试不均匀分布的空格
        let type_info = TypeInfo {
            byte_size: 2,
            print_width: 5,
        };
        let result = OutputInfo::calculate_alignment(&type_info, 8, 24);
        // 根据算法实际的空格分布进行断言
        assert_eq!(result[0], 1); // 第一个位置的空格数
        assert_eq!(result[4], 1); // 中间位置的空格数
        // 验证其他位置都是0
        assert_eq!(result[1..4], [0, 1, 0]);
        assert_eq!(result[5..8], [0, 1, 0]);
    }

    #[test]
    #[should_panic(expected = "bits types are unsupported")]
    fn test_calculate_alignment_too_large() {
        // 测试超出最大字节数的情况
        let type_info = TypeInfo {
            byte_size: MAX_BYTES_PER_UNIT + 1,
            print_width: 20,
        };
        OutputInfo::calculate_alignment(&type_info, MAX_BYTES_PER_UNIT + 1, 40);
    }
}
