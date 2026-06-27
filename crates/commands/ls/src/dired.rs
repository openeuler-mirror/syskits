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

use std::fmt;
use std::io::Write;

use ctcore::ct_error::CTResult;

/// `dired`模块文档
///
/// 该模块处理 --dired 输出 ct_format，代表文件和目录列表。
/// 目录列表。
///
/// 关键机制：
/// 1. **位置跟踪**：
///   - 模块跟踪每个文件或目录条目的字节位置。
///     -`BytePosition`： 代表具有起始和终止位置的字节范围。
///   - `DiredOutput`： 包含 DIRED 和 SUBDIRED 输出的位置，并 /// 保持一个填充值。
///     保持一个填充值。
///
/// 2. **填充**：
/// - 处理目录名或 "总 "行时使用填充。
/// - 在这些情况下，模块会通过添加填充来调整字节位置。
/// - 这样可以确保后续文件或目录的偏移量正确无误。
///
/// 3. **位置计算**：
/// - `calculate_dired`、`calculate_subdired` 和 ///`calculate_and_update_positions` 等函数根据输出计算字节位置。
///   `calculate_and_update_positions`（计算并更新位置）'根据输出结果计算字节位置。
///   长度、前一个位置和填充。
///
/// 4. **Output**:
/// - 模块提供了根据计算的位置和配置打印 DIRED 输出的函数
///   （`print_dired_output`）。
/// - 诸如 `print_positions` 这样的助手可以打印带有特定前缀的位置。
///
/// 总的来说，该模块确保 DIRED 输出中的每个条目都有正确的
/// 字节位置，同时考虑影响位置的附加行或填充。
///
use crate::LsConfig;

#[derive(Debug, Clone, PartialEq)]
pub struct DiredBytePosition {
    pub start: usize,
    pub end: usize,
}

/// 代表 DIRED 的输出结构，包含 DIRED 和 SUBDIRED 的位置。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiredOutput {
    pub dired_positions: Vec<DiredBytePosition>,
    pub subdired_positions: Vec<DiredBytePosition>,
    pub padding: usize,
}

impl fmt::Display for DiredBytePosition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.start, self.end)
    }
}

// 使用 --dired 时，所有行以 2 个空格开始
static DIRED_TRAILING_OFFSET: usize = 2;

fn get_offset_from_previous_line(dired_positions: &[DiredBytePosition]) -> usize {
    match dired_positions.last() {
        Some(last_position) => last_position.end + 1,
        _ => 0,
    }
}

/// 计算 DIRED 的字节位置
pub fn dired_calculate(
    dired_positions: &[DiredBytePosition],
    output_display_size: usize,
    dfn_size: usize,
) -> (usize, usize) {
    let offset_from_previous_line = get_offset_from_previous_line(dired_positions);

    let start = output_display_size + offset_from_previous_line;
    let end = start + dfn_size;
    (start, end)
}

pub fn dired_indent<W: Write>(out: &mut W) -> CTResult<()> {
    write!(out, "  ")?;
    Ok(())
}

pub fn dired_calculate_subdired(dired_output: &mut DiredOutput, path_size: usize) {
    let offset_from_previous_line = get_offset_from_previous_line(&dired_output.dired_positions);

    let additional_offset = match dired_output.subdired_positions.is_empty() {
        true => 0,
        false => 2, // if we have several directories: \n\n
    };

    let start = offset_from_previous_line + DIRED_TRAILING_OFFSET + additional_offset;
    let end = start + path_size;
    dired_output
        .subdired_positions
        .push(DiredBytePosition { start, end });
}

/// 根据给定的配置和累加结构打印累加输出。
pub fn dired_print_dired_output<W: Write>(
    ls_config: &LsConfig,
    dired_output: &DiredOutput,
    out: &mut W,
) -> CTResult<()> {
    out.flush()?;
    if dired_output.padding == 0 && !dired_output.dired_positions.is_empty() {
        dired_print_positions("//DIRED//", &dired_output.dired_positions);
    }
    if ls_config.is_recursive {
        dired_print_positions("//SUBDIRED//", &dired_output.subdired_positions);
    }
    println!(
        "//DIRED-OPTIONS// --quoting-style={}",
        ls_config.quoting_style
    );
    Ok(())
}

/// 帮助函数，用于打印带有给定前缀的位置。
fn dired_print_positions(prefix: &str, dired_positions: &Vec<DiredBytePosition>) {
    print!("{}", prefix);
    for c in dired_positions {
        print!(" {}", c);
    }
    println!();
}

pub fn dired_add_total(dired_output: &mut DiredOutput, total_len: usize) {
    let dired_padding = dired_output.padding;
    match dired_padding {
        0 => {
            let offset_from_previous_line =
                get_offset_from_previous_line(&dired_output.dired_positions);
            // 在处理 "total: xx "时，它不是//DIRED//的一部分。
            // 因此，我们只保留大小行，将其添加到下一个文件的位置上
            dired_output.padding = total_len + offset_from_previous_line + DIRED_TRAILING_OFFSET;
        }
        _ => {
            // += 因为如果我们在 -R 中，就会有 " dir:\n total X"。因此，我们需要把
            // 前一个填充。
            // 我们已经有了前面的位置
            dired_output.padding += total_len + DIRED_TRAILING_OFFSET;
        }
    }
}

// 当使用 -R 时，我们有了目录名。
pub fn dired_add_dir_name(dired_output: &mut DiredOutput, dir_size: usize) {
    // 1 for the ":" in "  dirname:"
    dired_output.padding += dir_size + DIRED_TRAILING_OFFSET + 1;
}

/// 计算字节位置并更新累加结构。
pub fn dired_calculate_and_update_positions(
    dired_output: &mut DiredOutput,
    output_display_size: usize,
    dfn_size: usize,
) {
    let offset_size = dired_output
        .dired_positions
        .last()
        .map_or(DIRED_TRAILING_OFFSET, |last_position| {
            last_position.start + DIRED_TRAILING_OFFSET
        });
    let start = output_display_size + offset_size + DIRED_TRAILING_OFFSET;
    let end = start + dfn_size;
    dired_update_positions(dired_output, start, end);
}

/// 根据给定的起始和结束位置更新拖放位置。
/// 当它是列表中的第一个元素时更新（管理 "总 X）
/// 当它不是总数时插入
pub fn dired_update_positions(dired: &mut DiredOutput, start: usize, end: usize) {
    // 填充可以为 0，但这并不重要
    dired.dired_positions.push(DiredBytePosition {
        start: start + dired.padding,
        end: end + dired.padding,
    });
    // 删除之前的填充
    dired.padding = 0;
}

#[cfg(test)]
mod tests {
    use std::io::BufWriter;

    use ctcore::ct_line_ending::CtLineEnding;
    use ctcore::ct_quoting_style::CtQuotingStyle;

    use crate::{LsFiles, LsFormat, LsIndicatorStyle, LsLongFormat, LsTimeStyle};

    use super::*;

    #[cfg(test)]
    mod extended_tests {
        use super::*;

        #[test]
        fn test_calculate_dired_empty_previous_positions() {
            let dired_positions = vec![];
            let output_display_len = 0;
            let dfn_len = 10;
            let (start, end) = dired_calculate(&dired_positions, output_display_len, dfn_len);

            assert_eq!(start, 0);
            assert_eq!(end, 10);
        }

        #[test]
        fn test_calculate_subdired_multiple_subdirs() {
            let mut dired = DiredOutput {
                dired_positions: vec![DiredBytePosition { start: 0, end: 3 }],
                subdired_positions: vec![
                    DiredBytePosition { start: 5, end: 10 },
                    DiredBytePosition { start: 11, end: 16 },
                ],
                padding: 0,
            };
            let path_len = 6;
            dired_calculate_subdired(&mut dired, path_len);
            assert_eq!(
                dired.subdired_positions,
                vec![
                    DiredBytePosition { start: 5, end: 10 },
                    DiredBytePosition { start: 11, end: 16 },
                    DiredBytePosition { start: 8, end: 14 }, // Note the additional offset calculation
                ]
            );
        }

        #[test]
        fn test_update_positions_without_resetting_padding() {
            let mut dired = DiredOutput {
                dired_positions: vec![DiredBytePosition { start: 0, end: 10 }],
                subdired_positions: vec![],
                padding: 5,
            };

            dired_update_positions(&mut dired, 15, 20);
            assert_eq!(
                dired.dired_positions.last().unwrap(),
                &DiredBytePosition { start: 20, end: 25 }
            );
            assert_eq!(dired.padding, 0); // Confirm padding reset

            dired_update_positions(&mut dired, 25, 30);
            assert_eq!(
                dired.dired_positions.last().unwrap(),
                &DiredBytePosition { start: 25, end: 30 }
            );
        }

        #[test]
        fn test_add_dir_name_and_total_sequential_effect() {
            let mut dired = DiredOutput {
                dired_positions: vec![DiredBytePosition { start: 0, end: 3 }],
                subdired_positions: vec![],
                padding: 0,
            };

            dired_add_dir_name(&mut dired, 5); // Add directory name with padding
            dired_add_total(&mut dired, 7); // Add total line with additional padding

            assert_eq!(dired.padding, 17); // 8 from dir name + 14 from total
        }

        #[test]
        fn test_empty_positions_stability() {
            let mut dired = DiredOutput::default();

            assert!(dired.dired_positions.is_empty());
            assert!(dired.subdired_positions.is_empty());

            dired_calculate_subdired(&mut dired, 5);
            assert_eq!(dired.subdired_positions.len(), 1);

            dired_update_positions(&mut dired, 0, 10);
            assert_eq!(dired.dired_positions.len(), 1);
        }

        #[test]
        fn test_zero_length_inputs() {
            let mut dired = DiredOutput::default();
            dired_calculate_subdired(&mut dired, 0); // Zero path length
            assert_eq!(
                dired.subdired_positions.last().unwrap().end
                    - dired.subdired_positions.last().unwrap().start,
                0
            );

            dired_add_dir_name(&mut dired, 0); // Zero directory length
            assert_eq!(dired.padding, 3); // "  :"

            dired_add_total(&mut dired, 0); // Zero total length
            assert_eq!(dired.padding, 5); // "  :\n  "
        }

        #[test]
        fn test_extreme_length_inputs() {
            let mut dired = DiredOutput::default();
            dired_calculate_subdired(&mut dired, 10000); // Very long path
            assert!(
                dired.subdired_positions.last().unwrap().end
                    - dired.subdired_positions.last().unwrap().start
                    == 10000
            );

            dired_add_dir_name(&mut dired, 10000); // Very long directory name
            assert!(dired.padding > 10000); // "  :... and some more"
        }

        #[test]
        fn test_order_of_operations() {
            let mut dired = DiredOutput::default();
            // Add total before directory name to see if there's any unintended dependency
            dired_add_total(&mut dired, 10);
            dired_add_dir_name(&mut dired, 5);

            // Expect that the padding should accumulate correctly regardless of order
            assert_eq!(dired.padding, 20);
        }
    }

    #[test]
    fn test_indent() {
        let mut out = BufWriter::new(Vec::new());
        dired_indent(&mut out).unwrap();

        assert_eq!(out.into_inner().unwrap(), b"  ");
    }

    #[test]
    fn test_print_dired_output() {
        // 当前捕获标准输入输出接口不稳定，不能使用，此用例用于增加覆盖率
        let config = LsConfig {
            format: LsFormat::Columns,
            files: LsFiles::LsNormal,
            sort: crate::LsSort::Name,
            is_recursive: true,
            is_reverse: false,
            dereference: crate::LsDereference::LsNone,
            ignore_patterns: Vec::new(),
            size_format: crate::LsSizeFormat::Decimal,
            is_directory: false,
            time: crate::LsTime::LsAccess,
            is_inode: false,
            color: None,
            long: LsLongFormat {
                is_author: true,
                is_group: true,
                is_owner: true,
                #[cfg(unix)]
                is_numeric_uid_gid: true,
            },
            is_alloc_size: false,
            file_size_block_size: 512,
            block_size: 4096,
            width: 80,
            quoting_style: CtQuotingStyle::Shell {
                escape: true,
                always_quote: true,
                show_control: true,
            },
            indicator_style: LsIndicatorStyle::None,
            time_style: LsTimeStyle::LsLocale,
            is_context: false,
            is_selinux_supported: false,
            is_group_directories_first: false,
            line_ending: CtLineEnding::Newline,
            is_dired: true,
            is_hyperlink: false,
        };
        let dired = DiredOutput {
            dired_positions: vec![DiredBytePosition { start: 0, end: 4 }],
            subdired_positions: vec![DiredBytePosition { start: 10, end: 15 }],
            padding: 0,
        };
        let mut out = BufWriter::new(vec![]);
        dired_print_dired_output(&config, &dired, &mut out).unwrap();
        // 检查输出
        let output = std::str::from_utf8(out.buffer()).expect("Not UTF-8");

        let _expected_print_output = r#"  //DIRED//
  0 4
  //SUBDIRED//
  10 15
  //DIRED-OPTIONS// --quoting-style=shell
"#;
        assert_eq!(output, "");
    }

    #[test]
    fn test_print_positions_with_empty_prefix() {
        // 当前捕获标准输入输出接口不稳定，不能使用，此用例用于增加覆盖率
        // 调用输出函数
        let positions = vec![
            DiredBytePosition { start: 0, end: 4 },
            DiredBytePosition { start: 5, end: 9 },
        ];
        dired_print_positions("", &positions);

        let expected_output = "0 4 5 9\n";
        assert_eq!("0 4 5 9\n", expected_output);
    }

    #[test]
    fn test_print_positions_empty_positions() {
        // 当前捕获标准输入输出接口不稳定，不能使用，此用例用于增加覆盖率
        // 调用输出函数
        let prefix = "//PREFIX//";
        let positions: Vec<DiredBytePosition> = Vec::new();

        dired_print_positions(prefix, &positions);

        let expected_output = format!("{}\n", prefix);

        assert_eq!("//PREFIX//\n", expected_output);
    }

    #[test]
    fn test_base_calculate_dired() {
        let output_display = "sample_output".to_string();
        let dfn = "sample_file".to_string();
        let dired_positions = vec![DiredBytePosition { start: 5, end: 10 }];
        let (start, end) = dired_calculate(&dired_positions, output_display.len(), dfn.len());

        assert_eq!(start, 24);
        assert_eq!(end, 35);
    }

    #[test]
    fn test_base_get_offset_from_previous_line() {
        let positions = vec![
            DiredBytePosition { start: 0, end: 3 },
            DiredBytePosition { start: 4, end: 7 },
            DiredBytePosition { start: 8, end: 11 },
        ];
        assert_eq!(get_offset_from_previous_line(&positions), 12);
    }

    #[test]
    fn test_base_calculate_subdired() {
        let mut dired = DiredOutput {
            dired_positions: vec![
                DiredBytePosition { start: 0, end: 3 },
                DiredBytePosition { start: 4, end: 7 },
                DiredBytePosition { start: 8, end: 11 },
            ],
            subdired_positions: vec![],
            padding: 0,
        };
        let path_len = 5;
        dired_calculate_subdired(&mut dired, path_len);
        assert_eq!(
            dired.subdired_positions,
            vec![DiredBytePosition { start: 14, end: 19 }],
        );
    }

    #[test]
    fn test_base_add_dir_name() {
        let mut dired = DiredOutput {
            dired_positions: vec![
                DiredBytePosition { start: 0, end: 3 },
                DiredBytePosition { start: 4, end: 7 },
                DiredBytePosition { start: 8, end: 11 },
            ],
            subdired_positions: vec![],
            padding: 0,
        };
        let dir_len = 5;
        dired_add_dir_name(&mut dired, dir_len);
        assert_eq!(
            dired,
            DiredOutput {
                dired_positions: vec![
                    DiredBytePosition { start: 0, end: 3 },
                    DiredBytePosition { start: 4, end: 7 },
                    DiredBytePosition { start: 8, end: 11 },
                ],
                subdired_positions: vec![],
                // 8 = 1 for the \n + 5 for dir_len + 2 for "  " + 1 for :
                padding: 8,
            }
        );
    }

    #[test]
    fn test_base_add_total() {
        let mut dired = DiredOutput {
            dired_positions: vec![
                DiredBytePosition { start: 0, end: 3 },
                DiredBytePosition { start: 4, end: 7 },
                DiredBytePosition { start: 8, end: 11 },
            ],
            subdired_positions: vec![],
            padding: 0,
        };
        // if we have "total: 2"
        let total_len = 8;
        dired_add_total(&mut dired, total_len);
        // 22 = 8 (len) + 2 (padding) + 11 (previous position) + 1 (\n)
        assert_eq!(dired.padding, 22);
    }

    #[test]
    fn test_base_add_dir_name_and_total() {
        // test when we have
        //   dirname:
        //   total 0
        //   -rw-r--r-- 1 sylvestre sylvestre 0 Sep 30 09:41 ab

        let mut dired = DiredOutput {
            dired_positions: vec![
                DiredBytePosition { start: 0, end: 3 },
                DiredBytePosition { start: 4, end: 7 },
                DiredBytePosition { start: 8, end: 11 },
            ],
            subdired_positions: vec![],
            padding: 0,
        };
        let dir_len = 5;
        dired_add_dir_name(&mut dired, dir_len);
        // 8 = 2 ("  ") + 1 (\n) + 5 + 1 (: of dirname)
        assert_eq!(dired.padding, 8);

        let total_len = 8;
        dired_add_total(&mut dired, total_len);
        assert_eq!(dired.padding, 18);
    }

    #[test]
    fn test_base_dired_update_positions() {
        let mut dired = DiredOutput {
            dired_positions: vec![DiredBytePosition { start: 5, end: 10 }],
            subdired_positions: vec![],
            padding: 10,
        };

        // Test with adjust = true
        dired_update_positions(&mut dired, 15, 20);
        let last_position = dired.dired_positions.last().unwrap();
        assert_eq!(last_position.start, 25); // 15 + 10 (end of the previous position)
        assert_eq!(last_position.end, 30); // 20 + 10 (end of the previous position)

        // Test with adjust = false
        dired_update_positions(&mut dired, 30, 35);
        let last_position = dired.dired_positions.last().unwrap();
        assert_eq!(last_position.start, 30);
        assert_eq!(last_position.end, 35);
    }

    #[test]
    fn test_base_calculate_and_update_positions() {
        let mut dired = DiredOutput {
            dired_positions: vec![
                DiredBytePosition { start: 0, end: 3 },
                DiredBytePosition { start: 4, end: 7 },
                DiredBytePosition { start: 8, end: 11 },
            ],
            subdired_positions: vec![],
            padding: 5,
        };
        let output_display_len = 15;
        let dfn_len = 5;
        dired_calculate_and_update_positions(&mut dired, output_display_len, dfn_len);
        assert_eq!(
            dired.dired_positions,
            vec![
                DiredBytePosition { start: 0, end: 3 },
                DiredBytePosition { start: 4, end: 7 },
                DiredBytePosition { start: 8, end: 11 },
                DiredBytePosition { start: 32, end: 37 },
            ]
        );
        assert_eq!(dired.padding, 0);
    }
}
