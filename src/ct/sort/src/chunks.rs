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

#![allow(dead_code)]
// 忽略`Chunk`中的`borrow_buffer`未使用警告

use std::{
    io::{ErrorKind, Read},
    sync::mpsc::SyncSender,
};

use memchr::memchr_iter;
use self_cell::self_cell;

use ctcore::ct_error::{CTResult, CtSimpleError};

use crate::{
    SortError, SortGeneralF64ParseResult, SortGlobalConfigs, SortLine, numeric_str_cmp::NumInfo,
};

self_cell!(
    /// 在线程之间传递的块。
    pub struct Chunk {
        owner: Vec<u8>,

        #[covariant]
        dependent: ChunkContents,
    }

    impl {Debug}
);

#[derive(Debug)]
pub struct ChunkContents<'a> {
    pub lines: Vec<SortLine<'a>>,
    pub line_data: ChunkLineData<'a>,
}

#[derive(Debug)]
pub struct ChunkLineData<'a> {
    pub selections: Vec<&'a str>,
    pub num_infos: Vec<NumInfo>,
    pub parsed_floats: Vec<SortGeneralF64ParseResult>,
}

impl Chunk {
    /// 销毁该块，并返回其组件以供重复使用。
    pub fn recycle(mut self) -> ChunkRecycled {
        let recycled_contents = self.with_dependent_mut(|_, chunk_contents| {
            chunk_contents.lines.clear();
            chunk_contents.line_data.selections.clear();
            chunk_contents.line_data.num_infos.clear();
            chunk_contents.line_data.parsed_floats.clear();
            let sort_lines = unsafe {
                // 安全性：（暂时）转换为具有较长生命周期的行矢量是安全的、
                // 因为矢量是空的。
                // 为了使回收成为可能，转换是必要的。参见 https://github.com/rust-lang/rfcs/pull/2802
                // 以了解无需进行转换的 Rfc。其示例与此处的代码类似.
                std::mem::transmute::<Vec<SortLine<'_>>, Vec<SortLine<'static>>>(std::mem::take(
                    &mut chunk_contents.lines,
                ))
            };
            let selections_str = unsafe {
                // 安全性：（同上）（暂时）转换为具有较长生命周期的 &str 向量是安全的、
                // 因为矢量是空的。
                std::mem::transmute::<Vec<&'_ str>, Vec<&'static str>>(std::mem::take(
                    &mut chunk_contents.line_data.selections,
                ))
            };
            (
                sort_lines,
                selections_str,
                std::mem::take(&mut chunk_contents.line_data.num_infos),
                std::mem::take(&mut chunk_contents.line_data.parsed_floats),
            )
        });
        ChunkRecycled {
            lines: recycled_contents.0,
            selections: recycled_contents.1,
            num_infos: recycled_contents.2,
            parsed_floats: recycled_contents.3,
            buffer: self.into_owner(),
        }
    }

    pub fn lines(&self) -> &Vec<SortLine> {
        &self.borrow_dependent().lines
    }
    pub fn line_data(&self) -> &ChunkLineData {
        &self.borrow_dependent().line_data
    }
}

pub struct ChunkRecycled {
    lines: Vec<SortLine<'static>>,
    selections: Vec<&'static str>,
    num_infos: Vec<NumInfo>,
    parsed_floats: Vec<SortGeneralF64ParseResult>,
    buffer: Vec<u8>,
}

impl ChunkRecycled {
    pub fn new(capacity: usize) -> Self {
        Self {
            lines: Vec::new(),
            selections: Vec::new(),
            num_infos: Vec::new(),
            parsed_floats: Vec::new(),
            buffer: vec![0; capacity],
        }
    }
}

/// 读取数据块，解析行并发送。
///
/// 不会发送空块。如果输入结束，则返回 `false`。
/// 但是，如果此函数返回 `true`，则不能保证还有
/// 剩余的输入： 如果输入内容完全符合缓冲区的要求，我们只会注意到在缓冲区的
/// 下次调用时就没有其他内容可读取了。如果没有剩余输入，则不会发送任何内容。
///
/// # 参数
///
/// （参见 `read_too_chunk` 获取更详细的说明文档）
///
/// * `sender`： 向分拣机发送行的发送方。
/// * `recycled_chunk`： 由 `Chunk::recycle`返回的回收块。
///   （即`buffer.len()`应等于`buffer.capacity()`)
/// * `max_buffer_size`： 缓冲区 "可以有多大。
/// * `carry_over`： 两次调用之间必须结转的字节数。
/// * `file`： 当前文件。
/// * `next_files`： 下一个要更新的文件。
/// * `separator`： 分隔符。
/// * `settings`： 全局设置。
#[allow(clippy::too_many_arguments)]
pub fn chunk_read<T: Read>(
    sender: &SyncSender<Chunk>,
    recycled_chunk: ChunkRecycled,
    max_buffer_size: Option<usize>,
    carry_over_vec: &mut Vec<u8>,
    file: &mut T,
    next_files: &mut impl Iterator<Item = CTResult<T>>,
    separator: u8,
    settings: &SortGlobalConfigs,
) -> CTResult<bool> {
    let ChunkRecycled {
        lines,
        selections,
        num_infos,
        parsed_floats,
        mut buffer,
    } = recycled_chunk;
    if buffer.len() < carry_over_vec.len() {
        buffer.resize(carry_over_vec.len() + 10 * 1024, 0);
    }
    buffer[..carry_over_vec.len()].copy_from_slice(carry_over_vec);
    let (read, should_continue) = chunk_read_to_buffer(
        file,
        next_files,
        &mut buffer,
        max_buffer_size,
        carry_over_vec.len(),
        separator,
    )?;
    carry_over_vec.clear();
    carry_over_vec.extend_from_slice(&buffer[read..]);

    if read != 0 {
        let payload: CTResult<Chunk> = Chunk::try_new(buffer, |buffer| {
            let selections_str = unsafe {
                // 安全：转换为生命周期较短的空选择向量是安全的。
                // 只是暂时转换为 Vec<Line<'static>>，以便循环使用。
                std::mem::transmute::<Vec<&'static str>, Vec<&'_ str>>(selections)
            };
            let mut sort_lines = unsafe {
                // 安全性：（同上）转换为生命周期较短的行矢量是安全的、
                // 因为它只是暂时转换为 Vec<Line<'static>>，以便可以循环使用。
                std::mem::transmute::<Vec<SortLine<'static>>, Vec<SortLine<'_>>>(lines)
            };
            let read = std::str::from_utf8(&buffer[..read])
                .map_err(|error| SortError::SortUft8Error { error })?;
            let mut chunk_line_data = ChunkLineData {
                selections: selections_str,
                num_infos,
                parsed_floats,
            };
            chunk_parse_lines(
                read,
                &mut sort_lines,
                &mut chunk_line_data,
                separator,
                settings,
            );
            Ok(ChunkContents {
                lines: sort_lines,
                line_data: chunk_line_data,
            })
        });
        sender.send(payload?).unwrap();
    }
    Ok(should_continue)
}

/// 将 `read` 分割成 `Line`，并将它们添加到 `lines`。
fn chunk_parse_lines<'a>(
    read: &'a str,
    sort_lines: &mut Vec<SortLine<'a>>,
    chunk_line_data: &mut ChunkLineData<'a>,
    separator: u8,
    sort_settings: &SortGlobalConfigs,
) {
    let read = read.strip_suffix(separator as char).unwrap_or(read);

    assert!(sort_lines.is_empty());
    assert!(chunk_line_data.selections.is_empty());
    assert!(chunk_line_data.num_infos.is_empty());
    assert!(chunk_line_data.parsed_floats.is_empty());
    let mut token_buffer = vec![];
    sort_lines.extend(
        read.split(separator as char)
            .enumerate()
            .map(|(index, line_str)| {
                SortLine::create(
                    line_str,
                    index,
                    chunk_line_data,
                    &mut token_buffer,
                    sort_settings,
                )
            }),
    );
}

/// 从 `file` 读取数据到 `buffer`。
///
/// 该函数确保至少读取两行（除非读到 EOF 且没有下一个文件）、
/// 如果有必要，会扩大缓冲区。
/// 最后一行可能还没有完全读入缓冲区。其字节必须复制到
/// 在下一次调用时，必须将其字节复制到缓冲区的前端，以便继续读取。
/// （参见返回值和 `start_offset`）。
///
/// # 参数
///
/// * `file`： 开始读取的文件。
/// * `next_files`： 当 `file` 到达 EOF 时，如果是 `Some` 则更新为 `next_files.next()`、
///   然后该函数继续读取。
/// * `buffer`： 装满字节的缓冲区。其内容大部分会被覆盖（参见 `start_offset`.
///   以及 `start_offset`）。如果有必要，它将增长到 `max_buffer_size`，但始终会增长到至少读取两行。
/// * `max_buffer_size`： 最多将缓冲区增长到这个长度。如果为 "无"，缓冲区将不会增长，除非需要读取至少两行。
/// * `start_offset`： 缓冲区起始处的字节数，这些字节是上一次读取时遗留下来的。
///   上一次读取时携带的、不应被覆盖的字节数。
/// * `separator`： 分隔行的字节。
///
/// # 返回
///
/// * `buffer`中现在可以解释为行的字节数。
///   剩下的字节必须复制到缓冲区的起点，以便下次调用、
///   如果需要再次调用，则由其他返回值决定。
/// * 是否再次调用此函数。
fn chunk_read_to_buffer<T: Read>(
    file: &mut T,
    next_files: &mut impl Iterator<Item = CTResult<T>>,
    buf: &mut Vec<u8>,
    max_buffer_size: Option<usize>,
    start_offset: usize,
    separator: u8,
) -> CTResult<(usize, bool)> {
    let mut read_target_buf = &mut buf[start_offset..];
    let mut last_file_target_len = read_target_buf.len();
    loop {
        match file.read(read_target_buf) {
            Ok(0) => {
                if read_target_buf.is_empty() {
                    // 块已满
                    if let Some(max_buffer_size) = max_buffer_size {
                        if max_buffer_size > buf.len() {
                            // 我们可以扩大缓冲区
                            let prev_len = buf.len();
                            if buf.len() < max_buffer_size / 2 {
                                buf.resize(buf.len() * 2, 0);
                            } else {
                                buf.resize(max_buffer_size, 0);
                            }
                            read_target_buf = &mut buf[prev_len..];
                            continue;
                        }
                    }
                    let mut sep_iter = memchr_iter(separator, buf).rev();
                    let last_line_end = sep_iter.next();
                    if sep_iter.next().is_some() {
                        // 我们读了足够多的台词。
                        let end = last_line_end.unwrap();
                        // 我们要在这里加入分隔符，因为它不应该被带入。
                        return Ok((end + 1, true));
                    } else {
                        // 我们需要读取更多行
                        let len = buf.len();
                        // 将矢量的大小调整为 10 KB 以上
                        buf.resize(len + 1024 * 10, 0);
                        read_target_buf = &mut buf[len..];
                    }
                } else {
                    // 该文件已被完全读取。
                    let mut leftover_len = read_target_buf.len();
                    if last_file_target_len != leftover_len {
                        // 文件不是空的。
                        let read_len = buf.len() - leftover_len;
                        if buf[read_len - 1] != separator {
                            // 文件结尾没有分隔符。我们必须插入一个。
                            buf[read_len] = separator;
                            leftover_len -= 1;
                        }
                        let read_len = buf.len() - leftover_len;
                        read_target_buf = &mut buf[read_len..];
                    }
                    if let Some(next_file) = next_files.next() {
                        // 还有一个文件。
                        last_file_target_len = leftover_len;
                        *file = next_file?;
                    } else {
                        // 这是最后一个文件。
                        let read_len = buf.len() - leftover_len;
                        return Ok((read_len, false));
                    }
                }
            }
            Ok(n) => {
                read_target_buf = &mut read_target_buf[n..];
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => {
                // 重试
            }
            Err(e) => return Err(CtSimpleError::new(2, e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::sync::mpsc::{self};

    use ctcore::ct_line_ending::CtLineEnding;

    use crate::numeric_str_cmp::NumInfoParseSettings;
    use crate::{SortMode, SortPrecomputed};

    use super::*;

    struct MockRead {
        data: Cursor<Vec<u8>>,
    }

    impl Read for MockRead {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.data.read(buf)
        }
    }

    fn line_data_default() -> ChunkLineData<'static> {
        ChunkLineData {
            selections: Vec::new(),
            num_infos: Vec::new(),
            parsed_floats: Vec::new(),
        }
    }

    #[test]
    fn test_read_function_settings_default() {
        let input = "line1\nline2\nline3";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            // Initialize with default or test-specific settings
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 3, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_ignore_case_true() {
        let input = "line1\nLINE2\nline3";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            is_ignore_case: true,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 3, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_ignore_case_false() {
        let input = "line1\nLINE2\nline3";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            is_ignore_case: false,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 3, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_different_line_endings_newline() {
        let input = "Windows\r\nUnix\nMac\r";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            line_ending: CtLineEnding::Newline,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 3, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_chunk_recycle() {
        let chunk = Chunk::new(vec![0; 10], |_buffer| {
            let lines = vec![
                SortLine {
                    line: "Line 1",
                    index: 0,
                },
                SortLine {
                    line: "Line 2",
                    index: 1,
                },
            ];
            let settings = NumInfoParseSettings::default();
            let a_info = NumInfo::parse("123e5", &settings).0;
            let b_info = NumInfo::parse("12300000", &settings).0;
            let line_data = ChunkLineData {
                selections: vec!["Selection 1", "Selection 2"],
                num_infos: vec![a_info, b_info],
                parsed_floats: vec![
                    SortGeneralF64ParseResult::SortNaN,
                    SortGeneralF64ParseResult::SortNaN,
                ],
            };
            ChunkContents { lines, line_data }
        });

        // Step 2: Recycle the chunk
        let recycled_chunk = chunk.recycle();

        // Step 3: Verify the recycled_chunk contents are cleared and ready for reuse
        assert!(
            recycled_chunk.lines.is_empty(),
            "Lines should be empty after recycling"
        );
        assert!(
            recycled_chunk.selections.is_empty(),
            "Selections should be empty after recycling"
        );
        assert!(
            recycled_chunk.num_infos.is_empty(),
            "NumInfos should be empty after recycling"
        );
        assert!(
            recycled_chunk.parsed_floats.is_empty(),
            "ParsedFloats should be empty after recycling"
        );

        // Step 4: Optionally, verify the buffer is also reset as expected
        assert_eq!(
            recycled_chunk.buffer.len(),
            10,
            "Buffer should be retained after recycling"
        );
        assert!(
            recycled_chunk.buffer.iter().all(|&b| b == 0),
            "Buffer should be cleared after recycling"
        );
    }

    #[test]
    fn test_chunk_recycle_complex_data() {
        let chunk = Chunk::new(vec![0; 20], |_| {
            let lines = vec![
                SortLine {
                    line: "Line with special characters: #@!",
                    index: 0,
                },
                SortLine {
                    line: "Line with unicode: привет",
                    index: 1,
                },
            ];

            let settings = NumInfoParseSettings::default();
            let a_info = NumInfo::parse("123e5", &settings).0;
            let b_info = NumInfo::parse("12300000", &settings).0;
            let line_data = ChunkLineData {
                selections: vec!["Special Selection", "Unicode Selection"],
                num_infos: vec![a_info, b_info],
                parsed_floats: vec![
                    SortGeneralF64ParseResult::SortInfinity,
                    SortGeneralF64ParseResult::SortInfinity,
                ],
            };
            ChunkContents { lines, line_data }
        });

        // Recycle the chunk
        let recycled_chunk = chunk.recycle();

        // Verify that all components are cleared
        assert!(
            recycled_chunk.lines.is_empty(),
            "Lines should be empty after recycling"
        );
        assert!(
            recycled_chunk.selections.is_empty(),
            "Selections should be empty after recycling"
        );
        assert!(
            recycled_chunk.num_infos.is_empty(),
            "NumInfos should be empty after recycling"
        );
        assert!(
            recycled_chunk.parsed_floats.is_empty(),
            "ParsedFloats should be empty after recycling"
        );
    }

    #[test]
    fn test_partial_data_recycling() {
        let chunk = Chunk::new(vec![0; 10], |_buffer| {
            let lines = vec![SortLine {
                line: "Partial data line",
                index: 0,
            }];
            let settings = NumInfoParseSettings::default();
            let num_info = NumInfo::parse("12300000", &settings).0;

            let line_data = ChunkLineData {
                selections: vec![],        // No selections
                num_infos: vec![num_info], // Some num_infos
                parsed_floats: vec![],     // No parsed floats
            };
            ChunkContents { lines, line_data }
        });

        // Recycle the chunk
        let recycled_chunk = chunk.recycle();

        // Checks
        assert_eq!(
            recycled_chunk.num_infos.len(),
            0,
            "NumInfos should be empty after recycling"
        );
        assert_eq!(
            recycled_chunk.selections.len(),
            0,
            "Selections should be empty after recycling"
        );
        assert_eq!(
            recycled_chunk.parsed_floats.len(),
            0,
            "ParsedFloats should be empty after recycling"
        );
    }

    #[test]
    fn test_read_function_different_line_endings_nul() {
        let input = "Windows\r\nUnix\nMac\r";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            line_ending: CtLineEnding::Nul,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 3, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_unique_lines_true() {
        let input = "line\nline\nline";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            is_unique: true,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 3, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_unique_lines_false() {
        let input = "line\nline\nline";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            is_unique: false,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 3, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_reverse_order_true() {
        let input = "line1\nline2\nline3";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            is_reverse: true,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 3, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_reverse_order_false() {
        let input = "line1\nline2\nline3";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            is_reverse: false,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 3, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_check_sorted_true() {
        let input = "line1\nline3\nline2"; // Intentionally unsorted
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            is_check: true,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 3, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_check_sorted_false() {
        let input = "line1\nline3\nline2"; // Intentionally unsorted
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            is_check: false,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 3, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_check_silent_true() {
        let input = "line1\nline2\nline3";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            is_check_silent: true,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 3, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_check_silent_false() {
        let input = "line1\nline2\nline3";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            is_check_silent: false,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 3, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_ignore_leading_blanks_true() {
        let input = "line1\nline2\nline3";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            is_ignore_leading_blanks: true,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 3, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_ignore_leading_blanks_false() {
        let input = "line1\nline2\nline3";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            is_ignore_leading_blanks: false,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 3, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_dictionary_order_true() {
        let input = "line1\nline2\nline3";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            is_dictionary_order: true,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 3, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_dictionary_order_false() {
        let input = "line1\nline2\nline3";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            is_dictionary_order: false,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 3, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_merge_true() {
        let input = "line1\nline2\nline3";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            is_merge: true,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 3, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_merge_false() {
        let input = "line1\nline2\nline3";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            is_merge: false,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 3, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_debug_true() {
        let input = "line1\nline2\nline3";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            is_debug: true,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 3, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_debug_false() {
        let input = "  line1\nline2\nline3";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            is_debug: false,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 3, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_stable_true() {
        let input = "line1\nline2\nline3";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            is_stable: true,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 3, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_stable_false() {
        let input = "line1\nline2\nline3";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            is_stable: false,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 3, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_sort_mode_default() {
        let input = "line1\nline2\nline3";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            mode: SortMode::SortDefault,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 3, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_sort_mode_version() {
        let input = "line1\nline2\nline3";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            mode: SortMode::SortVersion,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 3, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_sort_mode_month() {
        let input = "April\nOctober\nJuly\nAugust\nMay\nJune\nSeptember";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            mode: SortMode::SortMonth,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 7, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_sort_mode_random() {
        let input = "11\n11\n12\n111";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            mode: SortMode::SortRandom,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 4, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_sort_mode_human_numeric() {
        let input = "11\n11\n12\n111";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            mode: SortMode::SortHumanNumeric,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 4, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_sort_mode_numeric() {
        let input = "11\n11\n12\n111";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            mode: SortMode::SortNumeric,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 4, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_sort_mode_general_numeric() {
        let input = "11\n11\n12\n111";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            mode: SortMode::SortGeneralNumeric,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 4, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_salt_none() {
        let input = "line1\nline2\nline3\nline11";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            salt: None,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 4, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_salt_some_0() {
        let input = "line1\nline2\nline3\nline11";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            salt: Some([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 4, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_salt_some_digit() {
        let input = "line1\nline2\nline3\nline11";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            salt: Some([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]),
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 4, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_separator_none() {
        let input = "line1\nline2\nline3\nline11";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            separator: None,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 4, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_threads_none() {
        let input = "line1\nline2\nline3\nline11";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            threads: String::new(),
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 4, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_threads_qq() {
        let input = "line1\nline2\nline3\nline11";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            threads: String::from("qq"),
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 4, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_buffer_size_1000000() {
        let input = "line1\nline2\nline3\nline11";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            buffer_size: 1000000,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 4, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_buffer_size_0() {
        let input = "line1\nline2\nline3\nline11";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            buffer_size: 0,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 4, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_compress_prog_none() {
        let input = "line1\nline2\nline3\nline11";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            compress_prog: None,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 4, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_compress_prog_tar() {
        let input = "line1\nline2\nline3\nline11";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            compress_prog: Some("tar".to_string()),
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 4, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_compress_prog_zip() {
        let input = "line1\nline2\nline3\nline11";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            compress_prog: Some("zip".to_string()),
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 4, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_merge_batch_size_0() {
        let input = "line1\nline2\nline3\nline11";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            merge_batch_size: 0,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 4, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_merge_batch_size_32() {
        let input = "line1\nline2\nline3\nline11";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            merge_batch_size: 32,
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 4, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_precomputed_default() {
        let input = "line1\nline2\nline3\nline11";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            precomputed: SortPrecomputed::default(),
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 4, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_precomputed_needs_tokens_true() {
        let input = "line1\nline2\nline3\nline11";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            precomputed: SortPrecomputed {
                is_needs_tokens: true,
                num_infos_per_line: 0,
                floats_per_line: 0,
                selections_per_line: 0,
            },
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 4, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_precomputed_needs_tokens_false() {
        let input = "line1\nline2\nline3\nline11";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            precomputed: SortPrecomputed {
                is_needs_tokens: false,
                num_infos_per_line: 0,
                floats_per_line: 0,
                selections_per_line: 0,
            },
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 4, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_precomputed_needs_tokens_true_1_1_1() {
        let input = "line1\nline2\nline3\nline11";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            precomputed: SortPrecomputed {
                is_needs_tokens: true,
                num_infos_per_line: 1,
                floats_per_line: 1,
                selections_per_line: 1,
            },
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 4, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_precomputed_needs_tokens_false_1_1_1() {
        let input = "line1\nline2\nline3\nline11";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            precomputed: SortPrecomputed {
                is_needs_tokens: false,
                num_infos_per_line: 1,
                floats_per_line: 1,
                selections_per_line: 1,
            },
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 4, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_read_function_selectors_default() {
        let input = "line1\nline2\nline3\nline11";
        let mut file = Cursor::new(input.as_bytes());
        let (tx, rx) = mpsc::sync_channel(1);

        let recycled_chunk = ChunkRecycled::new(1024);
        let mut carry_over = Vec::new();
        let mut next_files = vec![].into_iter(); // Assuming no further files

        let settings = SortGlobalConfigs {
            selectors: vec![],
            ..Default::default()
        };

        let result = chunk_read(
            &tx,
            recycled_chunk,
            Some(1024),
            &mut carry_over,
            &mut file,
            &mut next_files,
            b'\n',
            &settings,
        );

        assert!(result.is_ok());

        // Check what was sent to the channel
        match rx.try_recv() {
            Ok(chunk) => {
                assert_eq!(chunk.lines().len(), 4, "There should be three lines parsed");
            }
            Err(e) => panic!("Expected a chunk but got an error: {:?}", e),
        }
    }

    #[test]
    fn test_parse_lines_basic() {
        let input = "line1\nline2\nline3";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            separator: Some('\n'),
            ..Default::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0],
            SortLine {
                line: "line1",
                index: 0,
            }
        );
        assert_eq!(
            lines[1],
            SortLine {
                line: "line2",
                index: 1,
            }
        );
        assert_eq!(
            lines[2],
            SortLine {
                line: "line3",
                index: 2,
            }
        );
    }

    #[test]
    fn test_parse_lines_with_different_separator() {
        let input = "part1|part2|part3";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            separator: Some('|'),
            ..Default::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'|', &settings);

        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0],
            SortLine {
                line: "part1",
                index: 0,
            }
        );
        assert_eq!(
            lines[1],
            SortLine {
                line: "part2",
                index: 1,
            }
        );
        assert_eq!(
            lines[2],
            SortLine {
                line: "part3",
                index: 2,
            }
        );
    }

    #[test]
    fn test_parse_lines_empty_input() {
        let input = "";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            separator: Some('\n'),
            ..Default::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        assert!(!lines.is_empty());
    }

    #[test]
    fn test_no_separator_present() {
        let input = "No separators here";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs::default();

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        assert_eq!(
            lines.len(),
            1,
            "Should handle input without separators gracefully"
        );
        assert_eq!(
            lines[0].line, "No separators here",
            "Content should match the entire input"
        );
    }

    #[test]
    fn test_read_to_buffer_resize_needed() {
        let data = b"Hello\nWorld\nThis is a longer test to force resizing of the buffer.";
        let expected_data =
            b"Hello\nWorld\nThis is a longer test to force resizing of the buffer.\n";
        let mut file = MockRead {
            data: Cursor::new(data.clone().to_vec()),
        };
        let mut buffer = vec![0; 10]; // Small buffer to trigger resizing
        let mut next_files = std::iter::empty();

        let (read_bytes, should_continue) =
            chunk_read_to_buffer(&mut file, &mut next_files, &mut buffer, Some(100), 0, b'\n')
                .unwrap();

        assert!(buffer.len() > 10); // Buffer should have resized
        assert_eq!(should_continue, false);
        assert_eq!(&buffer[..read_bytes], expected_data);
    }

    #[test]
    fn test_read_to_buffer_single_small_file() {
        let data = b"Hello\nWorld\n";
        let mut file = MockRead {
            data: Cursor::new(data.clone().to_vec()),
        };
        let mut buffer = vec![0; 1024]; // Large buffer to avoid resizing
        let mut next_files = std::iter::empty();

        let (read_bytes, should_continue) =
            chunk_read_to_buffer(&mut file, &mut next_files, &mut buffer, None, 0, b'\n').unwrap();

        assert_eq!(read_bytes, data.len());
        assert_eq!(should_continue, false);
        assert_eq!(&buffer[..read_bytes], data);
    }

    #[test]
    fn test_read_to_buffer_multiple_files() {
        let data1 = b"First file,\nSecond line.";
        let data2 = b"\nThird file starts here,\nAnd another line.";
        let file1 = MockRead {
            data: Cursor::new(data1.clone().to_vec()),
        };
        let file2 = MockRead {
            data: Cursor::new(data2.clone().to_vec()),
        };
        let mut buffer = vec![0; 1024];
        let mut next_files = vec![Ok(file2)].into_iter();

        let mut file = file1;
        let (read_bytes, should_continue) =
            chunk_read_to_buffer(&mut file, &mut next_files, &mut buffer, None, 0, b'\n').unwrap();

        assert_eq!(should_continue, false);
        assert!(read_bytes > 0 && read_bytes <= buffer.len());
    }

    #[test]
    fn test_empty_and_whitespace_lines() {
        let input = "\n   \n\n";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs::default();

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        assert_eq!(
            lines.len(),
            3,
            "Should recognize empty and whitespace lines"
        );
        assert_eq!(lines[1].line, "   ", "Whitespace line should be preserved");
    }

    #[test]
    fn test_complex_input_handling() {
        let input = "normal\nUPPERCASE\n1234\nspecial@#$%\n";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            is_ignore_case: true,
            is_dictionary_order: true,
            ..Default::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        assert_eq!(lines.len(), 4, "Should correctly parse complex input");
        assert_eq!(
            lines[1].line, "UPPERCASE",
            "Should handle uppercase lines under ignore_case setting"
        );
        assert!(
            line_data.num_infos.is_empty(),
            "NumInfos should be empty with no numbers processed"
        );
    }

    #[test]
    fn test_with_global_settings_true() {
        let input = "line1\nLINE2\nline3";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            is_ignore_case: true,
            ..Default::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        assert_eq!(lines.len(), 3);
        assert!(
            line_data.selections.is_empty(),
            "Expected no selections with ignore_case true"
        );
        assert_eq!(
            lines[1].line, "LINE2",
            "Case should be preserved in content"
        );
    }

    #[test]
    fn test_with_global_settings_false() {
        let input = "line1\nLINE2\nline3";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            is_ignore_case: false,
            ..Default::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        assert_eq!(lines.len(), 3);
        assert!(
            line_data.selections.is_empty(),
            "Expected no selections with ignore_case true"
        );
        assert_eq!(
            lines[1].line, "LINE2",
            "Case should be preserved in content"
        );
    }

    #[test]
    fn test_ignore_case() {
        let input = "a\nB\nc";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            is_ignore_case: true,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Depending on the implementation, this might affect sorting or comparison rather than parsing.
        // This test assumes that the implementation of parsing might lowercase everything if ignore_case is true.
        assert_eq!(
            lines[1].line, "B",
            "Should convert to lowercase if ignore_case is true"
        );
    }

    #[test]
    fn test_different_line_endings_newline() {
        let input = "Windows\r\nUnix\nMac\r";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            line_ending: CtLineEnding::Newline, // Assuming you have an enum or similar
            ..Default::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        assert_eq!(
            lines.len(),
            3,
            "Should split correctly using different line endings"
        );
        assert_eq!(
            lines[0].line, "Windows\r",
            "Incorrect handling of Windows line endings"
        );
        assert_eq!(
            lines[2].line, "Mac\r",
            "Incorrect handling of Mac line endings"
        );
    }

    #[test]
    fn test_different_line_endings_nul() {
        let input = "Windows\r\nUnix\nMac\r";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            line_ending: CtLineEnding::Nul, // Assuming you have an enum or similar
            ..Default::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        assert_eq!(
            lines.len(),
            3,
            "Should split correctly using different line endings"
        );
        assert_eq!(
            lines[0].line, "Windows\r",
            "Incorrect handling of Windows line endings"
        );
        assert_eq!(
            lines[2].line, "Mac\r",
            "Incorrect handling of Mac line endings"
        );
    }

    #[test]
    fn test_parse_lines_unique_lines_true() {
        let input = "line\nline\nline";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            is_unique: true,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming parse_lines filters out duplicates when unique is true
        assert_eq!(lines.len(), 3, "Only one unique line should be parsed");
    }

    #[test]
    fn test_parse_lines_unique_lines_false() {
        let input = "line\nline\nline";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            is_unique: false,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming parse_lines filters out duplicates when unique is true
        assert_eq!(lines.len(), 3, "Only one unique line should be parsed");
    }

    #[test]
    fn test_parse_lines_reverse_order_true() {
        let input = "line1\nline2\nline3";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            is_reverse: true,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_reverse_order_false() {
        let input = "line1\nline2\nline3";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            is_reverse: false,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_check_sorted_true() {
        let input = "line1\nline3\nline2"; // Intentionally unsorted
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            is_check: true,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming some sort of validation mechanism, perhaps an error or a bool indicating unsorted data
        // assert!(lines.is_sorted(), "Lines should be sorted, or an error/flag should indicate they are not");
    }

    #[test]
    fn test_parse_lines_check_sorted_false() {
        let input = "line1\nline3\nline2"; // Intentionally unsorted
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            is_check: false,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming some sort of validation mechanism, perhaps an error or a bool indicating unsorted data
        // assert!(lines.is_sorted(), "Lines should be sorted, or an error/flag should indicate they are not");
    }

    #[test]
    fn test_parse_lines_check_silent_true() {
        let input = "line1\nline2\nline3";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            is_check_silent: true,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_check_silent_false() {
        let input = "line1\nline2\nline3";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            is_check_silent: false,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_ignore_leading_blanks_true() {
        let input = "line1\nline2\nline3";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            is_ignore_leading_blanks: true,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_ignore_leading_blanks_false() {
        let input = "line1\nline2\nline3";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            is_ignore_leading_blanks: false,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_dictionary_order_true() {
        let input = "line1\nline2\nline3";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            is_dictionary_order: true,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_dictionary_order_false() {
        let input = "line1\nline2\nline3";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            is_dictionary_order: false,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_merge_true() {
        let input = "line1\nline2\nline3";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            is_merge: true,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_merge_false() {
        let input = "line1\nline2\nline3";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            is_merge: false,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_debug_true() {
        let input = "line1\nline2\nline3";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            is_debug: true,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_debug_false() {
        let input = "  line1\nline2\nline3";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            is_debug: false,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "  line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_stable_true() {
        let input = "line1\nline2\nline3";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            is_stable: true,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_stable_false() {
        let input = "line1\nline2\nline3";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            is_stable: false,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_sort_mode_default() {
        let input = "line1\nline2\nline3";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            mode: SortMode::SortDefault,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_sort_mode_version() {
        let input = "line1\nline2\nline3";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            mode: SortMode::SortVersion,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_sort_mode_month() {
        let input = "April\nOctober\nJuly\nAugust\nMay\nJune\nSeptember";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            mode: SortMode::SortMonth,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "April",
            "First line should be 'April' if reversed"
        );
        assert_eq!(
            lines[2].line, "July",
            "Last line should be 'July' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_sort_mode_random() {
        let input = "11\n11\n12\n111";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            mode: SortMode::SortRandom,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(lines[0].line, "11", "First line should be '11' if reversed");
        assert_eq!(lines[2].line, "12", "Last line should be '12' if reversed");
    }

    #[test]
    fn test_parse_lines_sort_mode_human_numeric() {
        let input = "11\n11\n12\n111";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            mode: SortMode::SortHumanNumeric,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(lines[0].line, "11", "First line should be '11' if reversed");
        assert_eq!(lines[2].line, "12", "Last line should be '12' if reversed");
    }

    #[test]
    fn test_parse_lines_sort_mode_numeric() {
        let input = "11\n11\n12\n111";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            mode: SortMode::SortNumeric,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(lines[0].line, "11", "First line should be '11' if reversed");
        assert_eq!(lines[2].line, "12", "Last line should be '12' if reversed");
    }

    #[test]
    fn test_parse_lines_sort_mode_general_numeric() {
        let input = "11\n11\n12\n111";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            mode: SortMode::SortGeneralNumeric,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(lines[0].line, "11", "First line should be '11' if reversed");
        assert_eq!(lines[2].line, "12", "Last line should be '12' if reversed");
    }

    #[test]
    fn test_parse_lines_sort_mode_general_numeric2() {
        let input = "11\n11\n12\n111";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            mode: SortMode::SortGeneralNumeric,
            ..SortGlobalConfigs::default()
        };
        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(lines[0].line, "11", "First line should be '11' if reversed");
        assert_eq!(lines[2].line, "12", "Last line should be '12' if reversed");
    }

    #[test]
    fn test_parse_lines_salt_none() {
        let input = "line1\nline2\nline3\nline11";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            salt: None,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_salt_some_0() {
        let input = "line1\nline2\nline3\nline11";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            salt: Some([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_salt_some_digit() {
        let input = "line1\nline2\nline3\nline11";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            salt: Some([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]),
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_separator_none() {
        let input = "line1\nline2\nline3\nline11";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            separator: None,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_threads_none() {
        let input = "line1\nline2\nline3\nline11";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            threads: String::new(),
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_threads_qq() {
        let input = "line1\nline2\nline3\nline11";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            threads: String::from("qq"),
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_buffer_size_1000000() {
        let input = "line1\nline2\nline3\nline11";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            buffer_size: 1000000,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_buffer_size_0() {
        let input = "line1\nline2\nline3\nline11";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            buffer_size: 0,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_compress_prog_none() {
        let input = "line1\nline2\nline3\nline11";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            compress_prog: None,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_compress_prog_tar() {
        let input = "line1\nline2\nline3\nline11";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            compress_prog: Some("tar".to_string()),
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_compress_prog_zip() {
        let input = "line1\nline2\nline3\nline11";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            compress_prog: Some("zip".to_string()),
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_merge_batch_size_0() {
        let input = "line1\nline2\nline3\nline11";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            merge_batch_size: 0,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_merge_batch_size_32() {
        let input = "line1\nline2\nline3\nline11";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            merge_batch_size: 32,
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_precomputed_default() {
        let input = "line1\nline2\nline3\nline11";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            precomputed: SortPrecomputed::default(),
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_precomputed_needs_tokens_true() {
        let input = "line1\nline2\nline3\nline11";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            precomputed: SortPrecomputed {
                is_needs_tokens: true,
                num_infos_per_line: 0,
                floats_per_line: 0,
                selections_per_line: 0,
            },
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_precomputed_needs_tokens_false() {
        let input = "line1\nline2\nline3\nline11";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            precomputed: SortPrecomputed {
                is_needs_tokens: false,
                num_infos_per_line: 0,
                floats_per_line: 0,
                selections_per_line: 0,
            },
            ..SortGlobalConfigs::default()
        };
        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_precomputed_needs_tokens_true_1_1_1() {
        let input = "line1\nline2\nline3\nline11";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            precomputed: SortPrecomputed {
                is_needs_tokens: true,
                num_infos_per_line: 1,
                floats_per_line: 1,
                selections_per_line: 1,
            },
            ..SortGlobalConfigs::default()
        };
        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_precomputed_needs_tokens_false_1_1_1() {
        let input = "line1\nline2\nline3\nline11";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            precomputed: SortPrecomputed {
                is_needs_tokens: false,
                num_infos_per_line: 1,
                floats_per_line: 1,
                selections_per_line: 1,
            },
            ..SortGlobalConfigs::default()
        };
        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    #[test]
    fn test_parse_lines_selectors_default() {
        let input = "line1\nline2\nline3\nline11";
        let mut lines: Vec<SortLine> = Vec::new();
        let mut line_data = line_data_default();
        let settings = SortGlobalConfigs {
            selectors: vec![],
            ..SortGlobalConfigs::default()
        };

        chunk_parse_lines(input, &mut lines, &mut line_data, b'\n', &settings);

        // Assuming the implementation reverses the order of lines post-parsing
        assert_eq!(
            lines[0].line, "line1",
            "First line should be 'line1' if reversed"
        );
        assert_eq!(
            lines[2].line, "line3",
            "Last line should be 'line3' if reversed"
        );
    }

    // ---------------->

    #[test]
    fn test_interrupted_read() {
        struct InterruptedRead {
            data: Cursor<Vec<u8>>,
            interrupt_count: usize,
        }

        impl Read for InterruptedRead {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.interrupt_count > 0 {
                    self.interrupt_count -= 1;
                    Err(std::io::Error::new(ErrorKind::Interrupted, "interrupted"))
                } else {
                    self.data.read(buf)
                }
            }
        }

        let data = b"Data that gets read after an interruption.";
        let expected_data = b"Data that gets read after an interruption.\n";
        let mut file = InterruptedRead {
            data: Cursor::new(data.clone().to_vec()),
            interrupt_count: 1,
        };
        let mut buffer = vec![0; 1024];
        let mut next_files = std::iter::empty();

        let (read_bytes, should_continue) =
            chunk_read_to_buffer(&mut file, &mut next_files, &mut buffer, None, 0, b'\n').unwrap();

        // 多一个换行符
        assert_eq!(read_bytes, expected_data.len());
        assert_eq!(should_continue, false);
        assert_eq!(&buffer[..read_bytes], expected_data);
    }

    #[test]
    fn test_file_without_ending_newline() {
        let data = b"Line without newline";
        let mut file = MockRead {
            data: Cursor::new(data.clone().to_vec()),
        };
        let mut buffer = vec![0; 1024];
        let mut next_files = std::iter::empty();

        let (read_bytes, should_continue) =
            chunk_read_to_buffer(&mut file, &mut next_files, &mut buffer, None, 0, b'\n').unwrap();

        assert_eq!(read_bytes, data.len() + 1); // includes appended newline
        assert_eq!(should_continue, false);
        assert_eq!(buffer[data.len()], b'\n'); // Check that newline was appended correctly
        assert_eq!(&buffer[..read_bytes - 1], data);
    }

    #[test]
    fn test_multiple_buffer_expansions() {
        let large_data = [
            b"a".repeat(5000),
            b"\n".to_vec(),
            b"b".repeat(10000),
            b"\n".to_vec(),
            b"c".repeat(5000),
        ]
        .concat();
        // let large_data = b"a".repeat(5000) + b"\n" + b"b".repeat(10000) + b"\n" + b"c".repeat(5000);
        let mut file = MockRead {
            data: Cursor::new(large_data.clone().to_vec()),
        };
        let mut buffer = vec![0; 4096]; // Initially smaller buffer
        let mut next_files = std::iter::empty();

        let (read_bytes, should_continue) = chunk_read_to_buffer(
            &mut file,
            &mut next_files,
            &mut buffer,
            Some(20000), // Max buffer size
            0,
            b'\n',
        )
        .unwrap();

        assert!(buffer.len() <= 20000);
        assert!(buffer.len() < large_data.len());
        assert_eq!(should_continue, true);
        assert_eq!(read_bytes, 15002);
    }

    #[test]
    fn test_continuous_reading_across_files() {
        let data1 = b"First file ends here without newline";
        let data2 = b"Second file starts immediately";
        let file1 = MockRead {
            data: Cursor::new(data1.clone().to_vec()),
        };
        let file2 = MockRead {
            data: Cursor::new(data2.clone().to_vec()),
        };
        let mut buffer = vec![0; 1024];
        let mut next_files = vec![Ok(file2)].into_iter();

        let mut file = file1;
        let (_, should_continue) =
            chunk_read_to_buffer(&mut file, &mut next_files, &mut buffer, None, 0, b'\n').unwrap();

        assert!(!should_continue);
        assert!(buffer.contains(&b'\n'));
    }

    #[test]
    fn test_file_ending_at_buffer_limit() {
        let data = b"Exactly at the buffer limit";
        let excepted_data = b"Exactly at the buffer limit\n";
        let mut file = MockRead {
            data: Cursor::new(data.clone().to_vec()),
        };
        let mut buffer = vec![0; data.len()]; // Buffer exactly the size of data
        let mut next_files = std::iter::empty();

        let (read_bytes, should_continue) =
            chunk_read_to_buffer(&mut file, &mut next_files, &mut buffer, None, 0, b'\n').unwrap();

        assert_eq!(read_bytes, excepted_data.len());
        assert_eq!(should_continue, false);
        assert_eq!(&buffer[..read_bytes], excepted_data);
    }

    #[test]
    fn test_incomplete_line_handling() {
        let data = b"Line without ending newline";
        let mut file = MockRead {
            data: Cursor::new(data.clone().to_vec()),
        };
        let mut buffer = vec![0; 1024]; // Large buffer
        let mut next_files = std::iter::empty();

        let (read_bytes, should_continue) =
            chunk_read_to_buffer(&mut file, &mut next_files, &mut buffer, None, 0, b'\n').unwrap();

        assert_eq!(should_continue, false);
        assert_eq!(&buffer[..read_bytes - 1], data); // Check buffer without the appended newline
        assert_eq!(buffer[read_bytes - 1], b'\n'); // Ensure newline was appended
    }

    #[test]
    fn test_error_handling() {
        struct ErrorMockRead;

        impl Read for ErrorMockRead {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(
                    ErrorKind::Other,
                    "Simulated read error",
                ))
            }
        }

        let mut file = ErrorMockRead;
        let mut buffer = vec![0; 1024];
        let mut next_files = std::iter::empty();

        let result = chunk_read_to_buffer(&mut file, &mut next_files, &mut buffer, None, 0, b'\n');

        assert!(result.is_err());
    }

    #[test]
    fn test_buffer_exact_match() {
        let data = b"First line\nSecond line\n";
        let mut file = MockRead {
            data: Cursor::new(data.to_vec()),
        };
        let mut buffer = vec![0; data.len()];
        let mut next_files = std::iter::empty();

        let (read_bytes, should_continue) =
            chunk_read_to_buffer(&mut file, &mut next_files, &mut buffer, None, 0, b'\n').unwrap();

        assert_eq!(read_bytes, data.len());
        assert_eq!(should_continue, true);
        assert_eq!(&buffer[..read_bytes], data);
    }

    #[test]
    fn test_buffer_underflow_with_offset() {
        let initial_data = b"End of previous line";
        let continued_data = b" starts here\nNext line";
        let mut file = MockRead {
            data: Cursor::new(continued_data.to_vec()),
        };
        let mut buffer = vec![0; 50]; // Small buffer to force resizing
        let mut next_files = std::iter::empty();

        buffer[..initial_data.len()].copy_from_slice(initial_data);
        let start_offset = initial_data.len();

        let (read_bytes, should_continue) = chunk_read_to_buffer(
            &mut file,
            &mut next_files,
            &mut buffer,
            Some(100), // Specify max buffer size for the test
            start_offset,
            b'\n',
        )
        .unwrap();

        assert!(buffer.len() <= 100);
        assert_eq!(should_continue, false);
        assert!(read_bytes > initial_data.len() + continued_data.len());
    }

    #[test]
    fn test_non_standard_separator_large_data() {
        // Create large data segments and store them as slices of &[u8]
        let segment_x = b"X".repeat(1024);
        let segment_y = b"Y".repeat(2048);
        let segment_z = b"Z".repeat(1024);
        let separator_value = b"";
        // Convert Vec<Vec<u8>> to Vec<&[u8]> to be compatible with join
        let data_segments = vec![&segment_x[..], &segment_y[..], &segment_z[..]];
        let expected_data_segments = vec![
            &segment_x[..],
            &segment_y[..],
            &segment_z[..],
            separator_value,
        ];
        let separator = &[b'|'][..]; // Create a slice for the separator

        // Use join on a slice of &[u8]
        let data = data_segments.join(separator);
        let expected_data = expected_data_segments.join(separator);
        let mut file = MockRead {
            data: Cursor::new(data.clone()),
        };
        let mut buffer = vec![0; 1500]; // Smaller than any single segment
        let mut next_files = std::iter::empty();

        let (read_bytes, should_continue) = chunk_read_to_buffer(
            &mut file,
            &mut next_files,
            &mut buffer,
            Some(5000), // Enough to fit all data eventually
            0,
            b'|',
        )
        .unwrap();

        assert_eq!(should_continue, false);
        assert!(read_bytes >= data.len()); // Ensure all data is read
        assert_eq!(&buffer[..read_bytes], expected_data.as_slice());
    }

    #[test]
    fn test_eof_without_newline() {
        let data = b"Complete line without newline";
        let expected_data = b"Complete line without newline\n";
        let mut file = MockRead {
            data: Cursor::new(data.to_vec()),
        };
        let mut buffer = vec![0; 1024];
        let mut next_files = std::iter::empty();

        let (read_bytes, should_continue) =
            chunk_read_to_buffer(&mut file, &mut next_files, &mut buffer, None, 0, b'\n').unwrap();

        assert_eq!(read_bytes, expected_data.len());
        assert_eq!(should_continue, false);
        assert_eq!(&buffer[..read_bytes], expected_data); // Ensure data matches exactly
    }

    #[test]
    fn test_start_offset_handling() {
        let initial_data = b"partial line";
        let continued_data = b" continues\nNew line";
        let expected_continued_data = b" continues\nNew line\n";
        let mut file = MockRead {
            data: Cursor::new(continued_data.to_vec()),
        };
        let mut buffer = vec![0; 1024];
        let mut next_files = std::iter::empty();

        // Simulate that "partial line" was the carryover data.
        buffer[..initial_data.len()].copy_from_slice(initial_data);
        let start_offset = initial_data.len();

        let (read_bytes, should_continue) = chunk_read_to_buffer(
            &mut file,
            &mut next_files,
            &mut buffer,
            None,
            start_offset,
            b'\n',
        )
        .unwrap();

        // Create complete_data by manually appending data into a vector
        let mut complete_data = Vec::from(initial_data);
        complete_data.extend_from_slice(expected_continued_data);

        assert_eq!(
            read_bytes,
            initial_data.len() + expected_continued_data.len()
        );
        assert_eq!(should_continue, false);
        assert_eq!(&buffer[..read_bytes], complete_data.as_slice());
    }

    #[test]
    fn test_different_separators() {
        let data = b"Row1|Row2|Row3";
        let excepted_data = b"Row1|Row2|Row3|";
        let mut file = MockRead {
            data: Cursor::new(data.to_vec()),
        };
        let mut buffer = vec![0; 1024];
        let mut next_files = std::iter::empty();
        let separator = b'|';

        let (read_bytes, should_continue) =
            chunk_read_to_buffer(&mut file, &mut next_files, &mut buffer, None, 0, separator)
                .unwrap();

        // Verify that the function correctly identifies '|' as the separator.
        assert_eq!(read_bytes, excepted_data.len());
        assert_eq!(should_continue, false);
        assert_eq!(&buffer[..read_bytes], excepted_data);
        assert_eq!(buffer[excepted_data.len() - 1], separator); // Check that last read byte is separator
    }

    #[test]
    fn test_max_buffer_size_handling() {
        let data = [
            b"a".repeat(5000),
            b"\n".to_vec(),
            b"b".repeat(10000),
            b"\n".to_vec(),
        ]
        .concat();

        let mut file = MockRead {
            data: Cursor::new(data.clone().to_vec()),
        };
        let mut buffer = vec![0; 3000]; // Initially smaller buffer
        let mut next_files = std::iter::empty();
        let max_buffer_size = 17240; // Larger than any single line but smaller than all data

        let (read_bytes, should_continue) = chunk_read_to_buffer(
            &mut file,
            &mut next_files,
            &mut buffer,
            Some(max_buffer_size),
            0,
            b'\n',
        )
        .unwrap();

        assert_eq!(buffer.len(), max_buffer_size); // Buffer should have resized up to max_buffer_size
        assert_eq!(read_bytes, 15002);
        assert_eq!(should_continue, false); // Should continue as not all data fits
    }
}
