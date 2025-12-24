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
    numeric_str_cmp::NumInfo, SortError, SortGeneralF64ParseResult, SortGlobalConfigs, SortLine,
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
/// （即`buffer.len()`应等于`buffer.capacity()`)
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
/// 然后该函数继续读取。
/// * `buffer`： 装满字节的缓冲区。其内容大部分会被覆盖（参见 `start_offset`.
/// 以及 `start_offset`）。如果有必要，它将增长到 `max_buffer_size`，但始终会增长到至少读取两行。
/// * `max_buffer_size`： 最多将缓冲区增长到这个长度。如果为 "无"，缓冲区将不会增长，除非需要读取至少两行。
/// * `start_offset`： 缓冲区起始处的字节数，这些字节是上一次读取时遗留下来的。
/// 上一次读取时携带的、不应被覆盖的字节数。
/// * `separator`： 分隔行的字节。
///
/// # 返回
///
/// * `buffer`中现在可以解释为行的字节数。
/// 剩下的字节必须复制到缓冲区的起点，以便下次调用、
/// 如果需要再次调用，则由其他返回值决定。
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

}