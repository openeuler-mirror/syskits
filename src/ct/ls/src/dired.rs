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
/// - 模块跟踪每个文件或目录条目的字节位置。
/// -`BytePosition`： 代表具有起始和终止位置的字节范围。
/// - `DiredOutput`： 包含 DIRED 和 SUBDIRED 输出的位置，并 /// 保持一个填充值。
/// 保持一个填充值。
///
/// 2. **填充**：
/// - 处理目录名或 "总 "行时使用填充。
/// - 在这些情况下，模块会通过添加填充来调整字节位置。
/// - 这样可以确保后续文件或目录的偏移量正确无误。
///
/// 3. **位置计算**：
/// - `calculate_dired`、`calculate_subdired` 和 ///`calculate_and_update_positions` 等函数根据输出计算字节位置。
/// `calculate_and_update_positions`（计算并更新位置）'根据输出结果计算字节位置。
/// 长度、前一个位置和填充。
///
/// 4. **Output**:
/// - 模块提供了根据计算的位置和配置打印 DIRED 输出的函数
/// （`print_dired_output`）。
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

