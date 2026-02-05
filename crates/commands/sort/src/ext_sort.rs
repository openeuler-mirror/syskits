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

//! 使用辅助文件存储中间块，对大文件进行分类。
//!
//! 文件被读取到内存块中，然后进行单独排序，并写入临时文件。
//! 写入临时文件。有两个线程： 一个分类器，一个读写器。
//! 单个内存块的缓冲区会循环使用。有两个缓冲区。

use std::cmp::Ordering;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use std::io::Read;
use std::sync::mpsc::{Receiver, SyncSender};
use std::thread;

use itertools::Itertools;

use ctcore::ct_error::CTResult;

use crate::SortOutput;
use crate::chunks::ChunkRecycled;
use crate::chunks::{self, Chunk};
use crate::merge::MergeClosedTmpFile;
use crate::merge::MergeWriteableCompressedTmpFile;
use crate::merge::MergeWriteablePlainTmpFile;
use crate::merge::MergeWriteableTmpFile;
use crate::tmp_dir::TmpDirWrapper;
use crate::{SortGlobalConfigs, merge, sort_by, sort_compare_by};

use crate::{SortLine, sort_print_sorted};

const EXT_SORT_START_BUFFER_SIZE: usize = 8_000;

/// 使用辅助文件存储中间块（如果需要），对文件进行排序，并输出结果。
pub fn ext_sort(
    files: &mut impl Iterator<Item = CTResult<Box<dyn Read + Send>>>,
    settings: &SortGlobalConfigs,
    output: SortOutput,
    tmp_dir: &mut TmpDirWrapper,
) -> CTResult<()> {
    let (sorted_sender, sorted_receiver) = std::sync::mpsc::sync_channel(1);
    let (recycled_sender, recycled_receiver) = std::sync::mpsc::sync_channel(1);
    thread::spawn({
        let settings = settings.clone();
        move || ext_sort_sorter(&recycled_receiver, &sorted_sender, &settings)
    });

    match settings.compress_prog {
        Some(_) => ext_sort_reader_writer::<_, MergeWriteableCompressedTmpFile>(
            files,
            settings,
            &sorted_receiver,
            recycled_sender,
            output,
            tmp_dir,
        ),
        None => ext_sort_reader_writer::<_, MergeWriteablePlainTmpFile>(
            files,
            settings,
            &sorted_receiver,
            recycled_sender,
            output,
            tmp_dir,
        ),
    }
}

fn ext_sort_reader_writer<
    F: Iterator<Item = CTResult<Box<dyn Read + Send>>>,
    Tmp: MergeWriteableTmpFile + 'static,
>(
    files: F,
    sort_settings: &SortGlobalConfigs,
    chunk_receiver: &Receiver<Chunk>,
    chunk_sender: SyncSender<Chunk>,
    sort_output: SortOutput,
    tmp_dir: &mut TmpDirWrapper,
) -> CTResult<()> {
    let separator = sort_settings.line_ending.into();

    // 启发式选择： 除以 10 似乎可以使我们的内存使用量大致
    // 设置.buffer_size 左右。
    let buffer_size = sort_settings.buffer_size / 10;
    let read_result: ExtSortReadResult<Tmp> = ext_sort_read_write_loop(
        files,
        tmp_dir,
        separator,
        buffer_size,
        sort_settings,
        chunk_receiver,
        chunk_sender,
    )?;
    match read_result {
        ExtSortReadResult::WroteChunksToFile { tmp_files } => {
            let merger = merge::merge_with_file_limit::<_, _, Tmp>(
                tmp_files.into_iter().map(|c| c.reopen()),
                sort_settings,
                tmp_dir,
            )?;
            merger.write_all(sort_settings, sort_output)?;
        }
        ExtSortReadResult::SortedSingleChunk(chunk) => match sort_settings.is_unique {
            true => {
                sort_print_sorted(
                    chunk.lines().iter().dedup_by(|a, b| {
                        sort_compare_by(a, b, sort_settings, chunk.line_data(), chunk.line_data())
                            == Ordering::Equal
                    }),
                    sort_settings,
                    sort_output,
                );
            }
            false => {
                sort_print_sorted(chunk.lines().iter(), sort_settings, sort_output);
            }
        },
        ExtSortReadResult::SortedTwoChunks([a, b]) => {
            let merged_iter = a.lines().iter().map(|line| (line, &a)).merge_by(
                b.lines().iter().map(|line| (line, &b)),
                |(line_a, a), (line_b, b)| {
                    sort_compare_by(line_a, line_b, sort_settings, a.line_data(), b.line_data())
                        != Ordering::Greater
                },
            );
            match sort_settings.is_unique {
                true => {
                    sort_print_sorted(
                        merged_iter
                            .dedup_by(|(line_a, a), (line_b, b)| {
                                sort_compare_by(
                                    line_a,
                                    line_b,
                                    sort_settings,
                                    a.line_data(),
                                    b.line_data(),
                                ) == Ordering::Equal
                            })
                            .map(|(line, _)| line),
                        sort_settings,
                        sort_output,
                    );
                }
                false => {
                    sort_print_sorted(
                        merged_iter.map(|(line, _)| line),
                        sort_settings,
                        sort_output,
                    );
                }
            }
        }
        ExtSortReadResult::EmptyInput => {
            // 不输出任何东西
        }
    }
    Ok(())
}

/// 在sort线程上执行的函数。
fn ext_sort_sorter(
    chunk_receiver: &Receiver<Chunk>,
    chunk_sender: &SyncSender<Chunk>,
    sort_settings: &SortGlobalConfigs,
) {
    while let Ok(mut payload) = chunk_receiver.recv() {
        payload.with_dependent_mut(|_, contents| {
            sort_by(&mut contents.lines, sort_settings, &contents.line_data);
        });
        if chunk_sender.send(payload).is_err() {
            // 接收者已经离开，可能是因为其他线程出错了。
            // 我们静静地停止，因为实际错误是由其他线程打印出来的。
            return;
        }
    }
}

/// 描述我们如何从输入中读取数据块。
enum ExtSortReadResult<I: MergeWriteableTmpFile> {
    /// 输入为空。没有读取任何内容。
    EmptyInput,
    /// 输入的内容被保存在内存中的一个 Chunk 中。
    SortedSingleChunk(Chunk),
    /// 输入内容分为两块，分别保存在内存中。
    SortedTwoChunks([Chunk; 2]),
    /// 输入内容被读取为多块，并写入辅助文件。
    WroteChunksToFile { tmp_files: Vec<I::Closed> },
}

/// 在读写线程上执行的函数。
fn ext_sort_read_write_loop<I: MergeWriteableTmpFile>(
    mut files: impl Iterator<Item = CTResult<Box<dyn Read + Send>>>,
    tmp_dir: &mut TmpDirWrapper,
    separator: u8,
    buffer_len: usize,
    sort_settings: &SortGlobalConfigs,
    chunk_receiver: &Receiver<Chunk>,
    chunk_sender: SyncSender<Chunk>,
) -> CTResult<ExtSortReadResult<I>> {
    let mut file = files.next().unwrap()?;

    let mut carry_over = vec![];
    // kick things off with two reads
    for _ in 0..2 {
        let should_continue = chunks::chunk_read(
            &chunk_sender,
            ChunkRecycled::new(if EXT_SORT_START_BUFFER_SIZE < buffer_len {
                EXT_SORT_START_BUFFER_SIZE
            } else {
                buffer_len
            }),
            Some(buffer_len),
            &mut carry_over,
            &mut file,
            &mut files,
            separator,
            sort_settings,
        )?;

        if !should_continue {
            drop(chunk_sender);
            // 我们已经读取了整个输入信息。由于我们正在进行前两次读取、
            // 这意味着我们可以将整个输入内容放入内存。绕过下面的写入
            // 以更直接的方式处理这种情况。
            let result = match chunk_receiver.recv() {
                Ok(first_chunk) => match chunk_receiver.recv() {
                    Ok(second_chunk) => {
                        ExtSortReadResult::SortedTwoChunks([first_chunk, second_chunk])
                    }
                    _ => ExtSortReadResult::SortedSingleChunk(first_chunk),
                },
                _ => ExtSortReadResult::EmptyInput,
            };

            return Ok(result);
        }
    }

    let mut sender_option = Some(chunk_sender);
    let mut tmp_files = vec![];
    loop {
        let chunk = match chunk_receiver.recv() {
            Ok(it) => it,
            _ => {
                return Ok(ExtSortReadResult::WroteChunksToFile { tmp_files });
            }
        };

        let tmp_file = ext_sort_write::<I>(
            &chunk,
            tmp_dir.next_file()?,
            sort_settings.compress_prog.as_deref(),
            separator,
        )?;
        tmp_files.push(tmp_file);

        let recycled_chunk = chunk.recycle();

        if let Some(sender) = &sender_option {
            let should_continue = chunks::chunk_read(
                sender,
                recycled_chunk,
                None,
                &mut carry_over,
                &mut file,
                &mut files,
                separator,
                sort_settings,
            )?;
            if !should_continue {
                sender_option = None;
            }
        }
    }
}

/// 将`chunk`中的行写入`file`，用`separator`分隔。
/// `compress_prog` 用于选择性压缩文件内容。
fn ext_sort_write<I: MergeWriteableTmpFile>(
    chunk: &Chunk,
    file: (File, PathBuf),
    compress_prog: Option<&str>,
    separator: u8,
) -> CTResult<I::Closed> {
    let mut tmp_file = I::create(file, compress_prog)?;
    ext_sort_write_lines(chunk.lines(), tmp_file.as_write(), separator);
    tmp_file.finished_writing()
}

fn ext_sort_write_lines<T: Write>(lines: &[SortLine], w: &mut T, separator: u8) {
    for s in lines {
        w.write_all(s.line.as_bytes()).unwrap();
        w.write_all(&[separator]).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(test)]
    mod sorter_test {
        use std::sync::mpsc::{self, Receiver, SyncSender};
        use std::thread;

        use ctcore::ct_line_ending::CtLineEnding;

        use crate::chunks::{ChunkContents, ChunkLineData};
        use crate::numeric_str_cmp::{NumInfo, NumInfoParseSettings};
        use crate::{SortGeneralF64ParseResult, SortMode, SortPrecomputed};

        use super::*;

        // 用样本数据创建模拟块的辅助函数
        fn create_mock_chunk() -> Chunk {
            Chunk::new(vec![0; 10], |_buffer| {
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
            })
        }

        #[test]
        fn test_sorter_settings_default() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs::default();

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_ignore_case_true() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                is_ignore_case: true,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_ignore_case_false() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                is_ignore_case: false,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_different_line_endings_newline() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                line_ending: CtLineEnding::Newline,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_different_line_endings_nul() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                line_ending: CtLineEnding::Nul,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_unique_lines_true() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                is_unique: true,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_unique_lines_false() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                is_unique: false,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_reverse_order_true() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                is_reverse: true,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 2", "Line 1"]);
        }

        #[test]
        fn test_read_function_reverse_order_false() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                is_reverse: false,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_check_sorted_true() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                is_check: true,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_check_sorted_false() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                is_check: false,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_check_silent_true() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                is_check_silent: true,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_check_silent_false() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                is_check_silent: false,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_ignore_leading_blanks_true() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                is_ignore_leading_blanks: true,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_ignore_leading_blanks_false() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                is_ignore_leading_blanks: false,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_dictionary_order_true() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                is_dictionary_order: true,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_dictionary_order_false() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                is_dictionary_order: false,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_merge_true() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                is_merge: true,
                ..SortGlobalConfigs::default()
            };
            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_merge_false() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                is_merge: false,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_debug_true() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                is_debug: true,
                ..SortGlobalConfigs::default()
            };
            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_debug_false() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                is_debug: false,
                ..SortGlobalConfigs::default()
            };
            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_stable_true() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                is_stable: true,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_stable_false() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                is_stable: false,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_sort_mode_default() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                mode: SortMode::SortDefault,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_sort_mode_version() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                mode: SortMode::SortVersion,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_sort_mode_month() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                mode: SortMode::SortMonth,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_sort_mode_random() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                mode: SortMode::SortRandom,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_sort_mode_human_numeric() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                mode: SortMode::SortHumanNumeric,
                ..SortGlobalConfigs::default()
            };
            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_sort_mode_numeric() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                mode: SortMode::SortNumeric,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_sort_mode_general_numeric() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                mode: SortMode::SortGeneralNumeric,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_salt_none() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                salt: None,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_salt_some_0() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                salt: Some([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_salt_some_digit() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                salt: Some([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]),
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_separator_none() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                separator: None,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_threads_none() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                threads: String::new(),
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_threads_qq() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                threads: String::from("qq"),
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_buffer_size_1000000() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                buffer_size: 1000000,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_buffer_size_0() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                buffer_size: 0,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_compress_prog_none() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                compress_prog: None,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_compress_prog_tar() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                compress_prog: Some("tar".to_string()),
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_compress_prog_zip() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);
            let settings = SortGlobalConfigs {
                compress_prog: Some("zip".to_string()),
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_merge_batch_size_0() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                merge_batch_size: 0,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_merge_batch_size_32() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                merge_batch_size: 32,
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_precomputed_default() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                precomputed: SortPrecomputed::default(),
                ..SortGlobalConfigs::default()
            };
            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_precomputed_needs_tokens_true() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                precomputed: SortPrecomputed {
                    is_needs_tokens: true,
                    num_infos_per_line: 0,
                    floats_per_line: 0,
                    selections_per_line: 0,
                },
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_precomputed_needs_tokens_false() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                precomputed: SortPrecomputed {
                    is_needs_tokens: false,
                    num_infos_per_line: 0,
                    floats_per_line: 0,
                    selections_per_line: 0,
                },
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_precomputed_needs_tokens_true_1_1_1() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                precomputed: SortPrecomputed {
                    is_needs_tokens: true,
                    num_infos_per_line: 1,
                    floats_per_line: 1,
                    selections_per_line: 1,
                },
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_precomputed_needs_tokens_false_1_1_1() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                precomputed: SortPrecomputed {
                    is_needs_tokens: false,
                    num_infos_per_line: 1,
                    floats_per_line: 1,
                    selections_per_line: 1,
                },
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }

        #[test]
        fn test_read_function_selectors_default() {
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);
            let (sorted_sender, sorted_receiver): (SyncSender<Chunk>, Receiver<Chunk>) =
                mpsc::sync_channel(1);

            let settings = SortGlobalConfigs {
                selectors: vec![],
                ..SortGlobalConfigs::default()
            };

            let mock_chunk = create_mock_chunk();
            sender.send(mock_chunk).unwrap(); // Send a mock chunk to be sorted

            // Run the sorter in a separate thread to simulate asynchronous operation
            let _sorter_thread = thread::spawn(move || {
                ext_sort_sorter(&receiver, &sorted_sender, &settings);
            });

            // Retrieve the sorted chunk
            let sorted_chunk = sorted_receiver.recv().unwrap();

            let sorted_lines: Vec<&str> =
                sorted_chunk.lines().iter().map(|line| line.line).collect();
            assert_eq!(sorted_lines, vec!["Line 1", "Line 2"]);
        }
    }

    #[cfg(test)]
    mod reader_writer_test {
        use std::fs::File;
        use std::io::{Cursor, Read};
        use std::sync::mpsc::{self, Receiver, SyncSender};

        use tempfile::tempdir;

        use ctcore::ct_line_ending::CtLineEnding;

        use crate::{SortMode, SortPrecomputed};

        use super::*;

        // Helper function to create a mock file reader
        fn mock_file_reader(content: &'static str) -> Box<dyn Read + Send> {
            Box::new(Cursor::new(content))
        }

        // Test for the reader_writer function
        #[test]
        fn test_reader_writer_for_writeable_plain_tmp_file() {
            let settings = SortGlobalConfigs {
                mode: SortMode::SortNumeric,
                is_debug: false,
                is_ignore_leading_blanks: true,
                is_ignore_case: false,
                is_dictionary_order: false,
                is_ignore_non_printing: false,
                is_merge: false,
                is_reverse: false,
                is_stable: false,
                is_unique: false,
                is_check: false,
                is_check_silent: false,
                salt: None,
                selectors: Vec::new(),
                separator: None,
                threads: "4".to_string(),
                line_ending: CtLineEnding::Newline,
                buffer_size: 8192,
                compress_prog: None,
                merge_batch_size: 100,
                precomputed: SortPrecomputed::default(),
            };

            let temp_dir = tempdir().unwrap();
            let mut tmp_dir_wrapper = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            // Prepare channels for chunks
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);

            // Create mock input files
            let files = vec![Ok(mock_file_reader("Hello\nWorld\n"))].into_iter();

            // Prepare output
            let output_path = temp_dir.path().join("output");
            let output_file = File::create(&output_path).unwrap();
            let output = SortOutput {
                file: Some(("output".to_string(), output_file)),
            };

            // Call the reader_writer function
            let result = ext_sort_reader_writer::<_, MergeWriteablePlainTmpFile>(
                files,
                &settings,
                &receiver,
                sender,
                output,
                &mut tmp_dir_wrapper,
            );

            assert!(result.is_ok());

            // Read the output file and verify the contents
            let mut output_contents = String::new();
            let mut file = File::open(output_path).unwrap();
            file.read_to_string(&mut output_contents).unwrap();
            assert_eq!(output_contents, "Hello\nWorld\n");
        }

        #[test]
        fn test_reader_writer_for_writeable_compressed_tmp_file() {
            let settings = SortGlobalConfigs {
                mode: SortMode::SortNumeric,
                is_debug: false,
                is_ignore_leading_blanks: true,
                is_ignore_case: false,
                is_dictionary_order: false,
                is_ignore_non_printing: false,
                is_merge: false,
                is_reverse: false,
                is_stable: false,
                is_unique: false,
                is_check: false,
                is_check_silent: false,
                salt: None,
                selectors: Vec::new(),
                separator: None,
                threads: "4".to_string(),
                line_ending: CtLineEnding::Newline,
                buffer_size: 8192,
                compress_prog: None,
                merge_batch_size: 100,
                precomputed: SortPrecomputed::default(),
            };

            let temp_dir = tempdir().unwrap();
            let mut tmp_dir_wrapper = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            // Prepare channels for chunks
            let (sender, receiver): (SyncSender<Chunk>, Receiver<Chunk>) = mpsc::sync_channel(1);

            // Create mock input files
            let files = vec![Ok(mock_file_reader("Hello\nWorld\n"))].into_iter();

            // Prepare output
            let output_path = temp_dir.path().join("output");
            let output_file = File::create(&output_path).unwrap();
            let output = SortOutput {
                file: Some(("output".to_string(), output_file)),
            };

            // Call the reader_writer function
            let result = ext_sort_reader_writer::<_, MergeWriteableCompressedTmpFile>(
                files,
                &settings,
                &receiver,
                sender,
                output,
                &mut tmp_dir_wrapper,
            );

            assert!(result.is_ok());

            // Read the output file and verify the contents
            let mut output_contents = String::new();
            let mut file = File::open(output_path).unwrap();
            file.read_to_string(&mut output_contents).unwrap();
            assert_eq!(output_contents, "Hello\nWorld\n");
        }

        #[test]
        fn test_basic_functionality_for_writeable_plain_tmp_file() {
            let files = vec![
                Ok(Box::new(Cursor::new(b"Apple\nBanana\n")) as Box<dyn Read + Send>),
                Ok(Box::new(Cursor::new(b"Cherry\nDate\n")) as Box<dyn Read + Send>),
            ]
            .into_iter();

            let settings = SortGlobalConfigs::default();
            let (sender, receiver) = mpsc::sync_channel(10);
            let tmp_dir = tempdir().unwrap();
            let mut tmp_dir_wrapper = TmpDirWrapper::new(tmp_dir.path().to_path_buf());

            let output_path = tmp_dir.path().join("output.txt");
            let output_file = File::create(&output_path).unwrap();
            let output = SortOutput {
                file: Some(("output.txt".to_string(), output_file)),
            };

            let result = ext_sort_reader_writer::<_, MergeWriteablePlainTmpFile>(
                files,
                &settings,
                &receiver,
                sender,
                output,
                &mut tmp_dir_wrapper,
            );

            assert!(result.is_ok());

            // Read the output file and check the contents
            let mut contents = String::new();
            File::open(output_path)
                .unwrap()
                .read_to_string(&mut contents)
                .unwrap();
            assert_eq!(contents, "Apple\nBanana\nCherry\nDate\n");
        }

        #[test]
        fn test_empty_input_files_for_writeable_plain_tmp_file() {
            let files = vec![
                Ok(Box::new(Cursor::new(b"")) as Box<dyn Read + Send>),
                Ok(Box::new(Cursor::new(b"")) as Box<dyn Read + Send>),
            ]
            .into_iter();
            let settings = SortGlobalConfigs::default();
            let (sender, receiver) = mpsc::sync_channel(10);
            let tmp_dir = tempdir().unwrap();
            let mut tmp_dir_wrapper = TmpDirWrapper::new(tmp_dir.path().to_path_buf());

            let output_path = tmp_dir.path().join("output.txt");
            let output_file = File::create(&output_path).unwrap();
            let output = SortOutput {
                file: Some(("output.txt".to_string(), output_file)),
            };

            let result = ext_sort_reader_writer::<_, MergeWriteablePlainTmpFile>(
                files,
                &settings,
                &receiver,
                sender,
                output,
                &mut tmp_dir_wrapper,
            );

            assert!(result.is_ok());

            // Check that the output file is empty
            let contents = std::fs::read_to_string(output_path).unwrap();
            assert!(
                contents.is_empty(),
                "Output file should be empty but was: {contents}"
            );
        }

        #[test]
        fn test_high_concurrency_and_load_for_writeable_plain_tmp_file() {
            let large_number_of_files = (0..100)
                .map(|_| Ok(Box::new(Cursor::new(b"Data\n")) as Box<dyn Read + Send>))
                .collect::<Vec<_>>()
                .into_iter();

            let settings = SortGlobalConfigs::default();
            let (sender, receiver) = mpsc::sync_channel(10);
            let tmp_dir = tempdir().unwrap();
            let mut tmp_dir_wrapper = TmpDirWrapper::new(tmp_dir.path().to_path_buf());

            let output_path = tmp_dir.path().join("output.txt");
            let output_file = File::create(&output_path).unwrap();
            let output = SortOutput {
                file: Some(("output.txt".to_string(), output_file)),
            };

            let result = ext_sort_reader_writer::<_, MergeWriteablePlainTmpFile>(
                large_number_of_files,
                &settings,
                &receiver,
                sender,
                output,
                &mut tmp_dir_wrapper,
            );

            assert!(result.is_ok());

            // Check that the output file is empty
            let contents = std::fs::read_to_string(output_path).unwrap();
            assert!(
                !contents.is_empty(),
                "Output file should be empty but was: {contents}"
            );
        }

        #[test]
        fn test_basic_functionality_for_writeable_compressed_tmp_file() {
            let files = vec![
                Ok(Box::new(Cursor::new(b"Apple\nBanana\n")) as Box<dyn Read + Send>),
                Ok(Box::new(Cursor::new(b"Cherry\nDate\n")) as Box<dyn Read + Send>),
            ]
            .into_iter();

            let settings = SortGlobalConfigs::default();
            let (sender, receiver) = mpsc::sync_channel(10);
            let tmp_dir = tempdir().unwrap();
            let mut tmp_dir_wrapper = TmpDirWrapper::new(tmp_dir.path().to_path_buf());

            let output_path = tmp_dir.path().join("output.txt");
            let output_file = File::create(&output_path).unwrap();
            let output = SortOutput {
                file: Some(("output.txt".to_string(), output_file)),
            };

            let result = ext_sort_reader_writer::<_, MergeWriteableCompressedTmpFile>(
                files,
                &settings,
                &receiver,
                sender,
                output,
                &mut tmp_dir_wrapper,
            );

            assert!(result.is_ok());

            // Read the output file and check the contents
            let mut contents = String::new();
            File::open(output_path)
                .unwrap()
                .read_to_string(&mut contents)
                .unwrap();
            assert_eq!(contents, "Apple\nBanana\nCherry\nDate\n");
        }

        #[test]
        fn test_empty_input_files_for_writeable_compressed_tmp_file() {
            let files = vec![
                Ok(Box::new(Cursor::new(b"")) as Box<dyn Read + Send>),
                Ok(Box::new(Cursor::new(b"")) as Box<dyn Read + Send>),
            ]
            .into_iter();
            let settings = SortGlobalConfigs::default();
            let (sender, receiver) = mpsc::sync_channel(10);
            let tmp_dir = tempdir().unwrap();
            let mut tmp_dir_wrapper = TmpDirWrapper::new(tmp_dir.path().to_path_buf());

            let output_path = tmp_dir.path().join("output.txt");
            let output_file = File::create(&output_path).unwrap();
            let output = SortOutput {
                file: Some(("output.txt".to_string(), output_file)),
            };

            let result = ext_sort_reader_writer::<_, MergeWriteableCompressedTmpFile>(
                files,
                &settings,
                &receiver,
                sender,
                output,
                &mut tmp_dir_wrapper,
            );

            assert!(result.is_ok());

            // Check that the output file is empty
            let contents = std::fs::read_to_string(output_path).unwrap();
            assert!(
                contents.is_empty(),
                "Output file should be empty but was: {contents}"
            );
        }

        #[test]
        fn test_high_concurrency_and_load_for_writeable_compressed_tmp_file() {
            let large_number_of_files = (0..100)
                .map(|_| Ok(Box::new(Cursor::new(b"Data\n")) as Box<dyn Read + Send>))
                .collect::<Vec<_>>()
                .into_iter();

            let settings = SortGlobalConfigs::default();
            let (sender, receiver) = mpsc::sync_channel(10);
            let tmp_dir = tempdir().unwrap();
            let mut tmp_dir_wrapper = TmpDirWrapper::new(tmp_dir.path().to_path_buf());

            let output_path = tmp_dir.path().join("output.txt");
            let output_file = File::create(&output_path).unwrap();
            let output = SortOutput {
                file: Some(("output.txt".to_string(), output_file)),
            };

            let result = ext_sort_reader_writer::<_, MergeWriteableCompressedTmpFile>(
                large_number_of_files,
                &settings,
                &receiver,
                sender,
                output,
                &mut tmp_dir_wrapper,
            );

            assert!(result.is_ok());

            // Check that the output file is empty
            let contents = std::fs::read_to_string(output_path).unwrap();
            assert!(
                !contents.is_empty(),
                "Output file should be empty but was: {contents}"
            );
        }
    }

    #[cfg(test)]
    mod ext_sort_test {
        use std::io::{Cursor, Read};

        use tempfile::TempDir;

        use ctcore::ct_line_ending::CtLineEnding;

        use crate::SortOutput;
        use crate::ext_sort::ext_sort;
        use crate::tmp_dir::TmpDirWrapper;
        use crate::{SortGlobalConfigs, SortMode, SortPrecomputed};

        #[test]
        fn test_ext_sort_default() {
            // let mut files = create_test_files();
            let data = "line1\nline2\nline3".as_bytes().to_vec();
            let reader = Cursor::new(data);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs::default();
            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_ignore_case_true() {
            let input = "line1\nLINE2\nline3";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                is_ignore_case: true,
                ..Default::default()
            };
            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_ignore_case_false() {
            let input = "line1\nLINE2\nline3";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                is_ignore_case: false,
                ..Default::default()
            };
            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_different_line_endings_newline() {
            let input = "Windows\r\nUnix\nMac\r";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                line_ending: CtLineEnding::Newline, // Assuming you have an enum or similar
                ..Default::default()
            };
            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        // #[test]
        // fn test_ext_sort_different_line_endings_nul() {
        //     let input = "Windows\r\nUnix\nMac\r";
        //     let reader = Cursor::new(input);
        //     let box_reader = Box::new(reader) as Box<dyn Read + Send>;
        //     let mut files = vec![Ok(box_reader)].into_iter();

        //     let settings = GlobalSettings {
        //         line_ending: LineEnding::Nul, // Assuming you have an enum or similar
        //         ..Default::default()
        //     };
        //     let output = Output { file: None };
        //     let temp_dir = TempDir::new().unwrap();
        //     let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

        //     let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

        //     assert!(result.is_ok());
        // }

        #[test]
        fn test_ext_sort_unique_lines_true() {
            let input = "line\nline\nline";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                is_unique: true,
                ..SortGlobalConfigs::default()
            };
            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_unique_lines_false() {
            let input = "line\nline\nline";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                is_unique: false,
                ..SortGlobalConfigs::default()
            };
            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_reverse_order_true() {
            let input = "line1\nline2\nline3";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                is_reverse: true,
                ..SortGlobalConfigs::default()
            };
            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_reverse_order_false() {
            let input = "line1\nline2\nline3";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                is_reverse: false,
                ..SortGlobalConfigs::default()
            };
            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_check_sorted_true() {
            let input = "line1\nline3\nline2"; // Intentionally unsorted
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                is_check: true,
                ..SortGlobalConfigs::default()
            };
            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_check_sorted_false() {
            let input = "line1\nline3\nline2"; // Intentionally unsorted
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                is_check: false,
                ..SortGlobalConfigs::default()
            };
            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_check_silent_true() {
            let input = "line1\nline2\nline3";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                is_check_silent: true,
                ..SortGlobalConfigs::default()
            };
            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_check_silent_false() {
            let input = "line1\nline2\nline3";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                is_check_silent: false,
                ..SortGlobalConfigs::default()
            };
            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_ignore_leading_blanks_true() {
            let input = "line1\nline2\nline3";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                is_ignore_leading_blanks: true,
                ..SortGlobalConfigs::default()
            };

            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_ignore_leading_blanks_false() {
            let input = "line1\nline2\nline3";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                is_ignore_leading_blanks: false,
                ..SortGlobalConfigs::default()
            };

            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_dictionary_order_true() {
            let input = "line1\nline2\nline3";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                is_dictionary_order: true,
                ..SortGlobalConfigs::default()
            };

            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_dictionary_order_false() {
            let input = "line1\nline2\nline3";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                is_dictionary_order: false,
                ..SortGlobalConfigs::default()
            };

            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_merge_true() {
            let input = "line1\nline2\nline3";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                is_merge: true,
                ..SortGlobalConfigs::default()
            };
            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_merge_false() {
            let input = "line1\nline2\nline3";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                is_merge: false,
                ..SortGlobalConfigs::default()
            };
            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_debug_true() {
            let input = "line1\nline2\nline3";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                is_debug: true,
                ..SortGlobalConfigs::default()
            };

            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_debug_false() {
            let input = "  line1\nline2\nline3";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                is_debug: false,
                ..SortGlobalConfigs::default()
            };

            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_stable_true() {
            let input = "line1\nline2\nline3";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                is_stable: true,
                ..SortGlobalConfigs::default()
            };
            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_stable_false() {
            let input = "line1\nline2\nline3";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                is_stable: false,
                ..SortGlobalConfigs::default()
            };
            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_sort_mode_default() {
            let input = "line1\nline2\nline3";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                mode: SortMode::SortDefault,
                ..SortGlobalConfigs::default()
            };

            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_sort_mode_version() {
            let input = "line1\nline2\nline3";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                mode: SortMode::SortVersion,
                ..SortGlobalConfigs::default()
            };

            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_sort_mode_month() {
            let input = "April\nOctober\nJuly\nAugust\nMay\nJune\nSeptember";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                mode: SortMode::SortMonth,
                ..SortGlobalConfigs::default()
            };

            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_sort_mode_random() {
            let input = "11\n11\n12\n111";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                mode: SortMode::SortRandom,
                ..SortGlobalConfigs::default()
            };

            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_sort_mode_human_numeric() {
            let input = "11\n11\n12\n111";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                mode: SortMode::SortHumanNumeric,
                ..SortGlobalConfigs::default()
            };

            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_sort_mode_numeric() {
            let input = "11\n11\n12\n111";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                mode: SortMode::SortNumeric,
                ..SortGlobalConfigs::default()
            };

            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_sort_mode_general_numeric() {
            let input = "11\n11\n12\n111";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                mode: SortMode::SortGeneralNumeric,
                ..SortGlobalConfigs::default()
            };

            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_salt_none() {
            let input = "line1\nline2\nline3\nline11";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                salt: None,
                ..SortGlobalConfigs::default()
            };

            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_salt_some_0() {
            let input = "line1\nline2\nline3\nline11";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                salt: Some([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
                ..SortGlobalConfigs::default()
            };
            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_salt_some_digit() {
            let input = "line1\nline2\nline3\nline11";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                salt: Some([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]),
                ..SortGlobalConfigs::default()
            };
            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_separator_none() {
            let input = "line1\nline2\nline3\nline11";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                separator: None,
                ..SortGlobalConfigs::default()
            };
            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_threads_none() {
            let input = "line1\nline2\nline3\nline11";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                threads: String::new(),
                ..SortGlobalConfigs::default()
            };

            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_threads_qq() {
            let input = "line1\nline2\nline3\nline11";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                threads: String::from("qq"),
                ..SortGlobalConfigs::default()
            };

            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_buffer_size_1000000() {
            let input = "line1\nline2\nline3\nline11";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                buffer_size: 1000000,
                ..SortGlobalConfigs::default()
            };

            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_buffer_size_0() {
            let input = "line1\nline2\nline3\nline11";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                buffer_size: 0,
                ..SortGlobalConfigs::default()
            };
            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_compress_prog_none() {
            let input = "line1\nline2\nline3\nline11";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                compress_prog: None,
                ..SortGlobalConfigs::default()
            };
            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_compress_prog_tar() {
            let input = "line1\nline2\nline3\nline11";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                compress_prog: Some("tar".to_string()),
                ..SortGlobalConfigs::default()
            };
            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_compress_prog_zip() {
            let input = "line1\nline2\nline3\nline11";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                compress_prog: Some("zip".to_string()),
                ..SortGlobalConfigs::default()
            };
            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_merge_batch_size_0() {
            let input = "line1\nline2\nline3\nline11";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                merge_batch_size: 0,
                ..SortGlobalConfigs::default()
            };
            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_merge_batch_size_32() {
            let input = "line1\nline2\nline3\nline11";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                merge_batch_size: 32,
                ..SortGlobalConfigs::default()
            };

            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_precomputed_default() {
            let input = "line1\nline2\nline3\nline11";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                precomputed: SortPrecomputed::default(),
                ..SortGlobalConfigs::default()
            };
            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_precomputed_needs_tokens_true() {
            let input = "line1\nline2\nline3\nline11";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                precomputed: SortPrecomputed {
                    is_needs_tokens: true,
                    num_infos_per_line: 0,
                    floats_per_line: 0,
                    selections_per_line: 0,
                },
                ..SortGlobalConfigs::default()
            };
            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_precomputed_needs_tokens_false() {
            let input = "line1\nline2\nline3\nline11";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                precomputed: SortPrecomputed {
                    is_needs_tokens: false,
                    num_infos_per_line: 0,
                    floats_per_line: 0,
                    selections_per_line: 0,
                },
                ..SortGlobalConfigs::default()
            };
            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_precomputed_needs_tokens_true_1_1_1() {
            let input = "line1\nline2\nline3\nline11";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                precomputed: SortPrecomputed {
                    is_needs_tokens: true,
                    num_infos_per_line: 1,
                    floats_per_line: 1,
                    selections_per_line: 1,
                },
                ..SortGlobalConfigs::default()
            };
            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_precomputed_needs_tokens_false_10_10_10() {
            let input = "line1\nline2\nline3\nline11";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                precomputed: SortPrecomputed {
                    is_needs_tokens: false,
                    num_infos_per_line: 10,
                    floats_per_line: 10,
                    selections_per_line: 10,
                },
                ..SortGlobalConfigs::default()
            };
            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }

        #[test]
        fn test_ext_sort_selectors_default() {
            let input = "line1\nline2\nline3\nline11";
            let reader = Cursor::new(input);
            let box_reader = Box::new(reader) as Box<dyn Read + Send>;
            let mut files = vec![Ok(box_reader)].into_iter();

            let settings = SortGlobalConfigs {
                selectors: vec![],
                ..SortGlobalConfigs::default()
            };

            let output = SortOutput { file: None };
            let temp_dir = TempDir::new().unwrap();
            let mut tmp_dir = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let result = ext_sort(&mut files, &settings, output, &mut tmp_dir);

            assert!(result.is_ok());
        }
    }
    mod write_tests {
        use std::fs;
        use std::fs::File;
        use std::io::{BufWriter, Cursor};
        use std::path::PathBuf;

        use tempfile::{NamedTempFile, tempdir};

        use crate::chunks::{ChunkContents, ChunkLineData};
        use crate::merge::MergeInput;
        use crate::numeric_str_cmp::{NumInfo, NumInfoParseSettings};
        use crate::{SortError, SortGeneralF64ParseResult};

        use super::*;

        // Mock implementation of WriteableTmpFile for testing
        struct MockWriteableTmpFile {
            writer: BufWriter<File>,
            path: PathBuf,
        }

        impl MergeWriteableTmpFile for MockWriteableTmpFile {
            type Closed = MockClosedTmpFile;
            type InnerWrite = BufWriter<File>;

            fn create(file: (File, PathBuf), _compress_prog: Option<&str>) -> CTResult<Self> {
                Ok(MockWriteableTmpFile {
                    writer: BufWriter::new(file.0),
                    path: file.1,
                })
            }

            fn finished_writing(self) -> CTResult<Self::Closed> {
                Ok(MockClosedTmpFile { path: self.path })
            }

            fn as_write(&mut self) -> &mut Self::InnerWrite {
                &mut self.writer
            }
        }

        struct MockClosedTmpFile {
            path: PathBuf,
        }

        struct MockMergeInput {
            data: Cursor<Vec<u8>>,
        }

        impl MergeInput for MockMergeInput {
            type InnerRead = Cursor<Vec<u8>>;

            fn finished_reading(self) -> CTResult<()> {
                Ok(())
            }

            fn as_read(&mut self) -> &mut Self::InnerRead {
                &mut self.data
            }
        }

        impl MergeClosedTmpFile for MockClosedTmpFile {
            type Reopened = MockMergeInput;

            fn reopen(self) -> CTResult<Self::Reopened> {
                // Reopen the file and prepare the MockMergeInput
                let data = fs::read(&self.path)
                    .map_err(|e| SortError::SortOpenTmpFileFailed { error: e })?;
                Ok(MockMergeInput {
                    data: Cursor::new(data),
                })
            }
        }

        // Helper function to create a mock chunk
        fn create_mock_chunk() -> Chunk {
            Chunk::new(vec![0; 10], |_buffer| {
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
            })
        }

        fn create_mock_chunk_with_known_data() -> Chunk {
            Chunk::new(vec![0; 10], |_buffer| {
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
            })
        }

        fn create_empty_chunk() -> Chunk {
            Chunk::new(vec![0; 10], |_buffer| {
                let lines = Vec::new(); // No lines
                let line_data = ChunkLineData {
                    selections: Vec::new(),
                    num_infos: Vec::new(),
                    parsed_floats: Vec::new(),
                };
                ChunkContents { lines, line_data }
            })
        }

        #[test]
        fn test_write_function() {
            let temp_dir = tempdir().unwrap();
            let file_path = temp_dir.path().join("test_output.txt");
            let file = File::create(&file_path).unwrap();

            let chunk = create_mock_chunk();
            let result = ext_sort_write::<MockWriteableTmpFile>(
                &chunk,
                (file, file_path.clone()),
                None,
                b'\n',
            );

            assert!(result.is_ok());

            // Read back the file to check contents
            let mut contents = String::new();
            let mut file = File::open(file_path).unwrap();
            file.read_to_string(&mut contents).unwrap();
            assert_eq!(contents, "Line 1\nLine 2\n");
        }

        #[test]
        fn test_write_typical() {
            let temp_file = NamedTempFile::new().unwrap();
            let file_path = temp_file.path().to_path_buf();
            let file = temp_file.reopen().unwrap();

            let chunk = create_mock_chunk(); // Assume this creates a valid Chunk with some lines
            let separator = b'\n';

            let result =
                ext_sort_write::<MockWriteableTmpFile>(&chunk, (file, file_path), None, separator);
            assert!(result.is_ok());
        }

        #[test]
        fn test_write_with_compression() {
            let temp_file = NamedTempFile::new().unwrap();
            let file_path = temp_file.path().to_path_buf();
            let file = temp_file.reopen().unwrap();

            let chunk = create_mock_chunk(); // Same as above
            let separator = b'\n';

            // Assuming "gzip" is a placeholder for actual compression behavior
            let result = ext_sort_write::<MergeWriteableCompressedTmpFile>(
                &chunk,
                (file, file_path),
                Some("gzip"),
                separator,
            );
            assert!(result.is_ok());
        }

        #[test]
        fn test_write_error_handling() {
            let temp_dir = tempdir().unwrap();
            let file_path = temp_dir.path().join("non_writable_file");
            let file = File::create(&file_path).unwrap();

            let chunk = create_mock_chunk();

            // Set file permissions to read-only to simulate a write error
            let metadata = fs::metadata(&file_path).unwrap();
            let mut permissions = metadata.permissions();
            permissions.set_readonly(true);
            fs::set_permissions(&file_path, permissions).unwrap();

            let result =
                ext_sort_write::<MockWriteableTmpFile>(&chunk, (file, file_path), None, b'\n');
            assert!(result.is_ok());
        }

        #[test]
        fn test_write_empty_chunk() {
            let temp_file = NamedTempFile::new().unwrap();
            let file_path = temp_file.path().to_path_buf();
            let file = temp_file.reopen().unwrap();

            let chunk = create_empty_chunk(); // Assume this creates an empty Chunk
            let separator = b'\n';

            let result =
                ext_sort_write::<MockWriteableTmpFile>(&chunk, (file, file_path), None, separator);
            assert!(result.is_ok());
        }

        #[test]
        fn test_data_integrity() {
            let temp_file = NamedTempFile::new().unwrap();
            let file_path = temp_file.path().to_path_buf();
            let file = temp_file.reopen().unwrap();

            let chunk = create_mock_chunk_with_known_data(); // Assume this creates a chunk with predictable data
            let separator = b'\n';

            let _ = ext_sort_write::<MockWriteableTmpFile>(
                &chunk,
                (file, file_path.clone()),
                None,
                separator,
            );

            let mut contents = String::new();
            let mut file = File::open(file_path).unwrap();
            file.read_to_string(&mut contents).unwrap();

            assert_eq!(contents, "Line 1\nLine 2\n"); // Assuming "KnownData" was the expected output
        }
    }

    mod read_write_loop_test {
        use std::io::Cursor;
        use std::sync::mpsc;

        use tempfile::tempdir;

        use super::*;

        // #[test]
        // fn test_single_small_file() {
        //     let input_data = b"Line 1\nLine 2\n"; // Simple data that fits into the initial read buffer
        //     let cursor = Cursor::new(input_data.clone());
        //     let files = vec![Ok(Box::new(cursor) as Box<dyn Read + Send>)].into_iter();
        //
        //     let temp_dir = tempdir().unwrap();
        //     let mut tmp_dir_wrapper = TmpDirWrapper::new(temp_dir.path().to_path_buf());
        //
        //     let (sender, receiver) = mpsc::sync_channel(1);
        //     let (response_sender, response_receiver) = mpsc::sync_channel(1);
        //     let sender_clone = sender.clone();
        //     let settings = GlobalSettings::default();
        //
        //     // Kick off the loop in a separate thread
        //     let handle = std::thread::spawn(move || {
        //         let result = read_write_loop::<WriteablePlainTmpFile>(
        //             files,
        //             &mut tmp_dir_wrapper,
        //             b'\n',
        //             input_data.len(),
        //             &settings,
        //             &receiver,
        //             sender_clone,
        //         );
        //         response_sender.send(result).unwrap();
        //     });
        //
        //     // Sending fake chunks to simulate processing
        //     sender.send(Chunk::new(vec![], |_buf| ChunkContents {
        //         lines: vec![],
        //         line_data: LineData {
        //             selections: vec![],
        //             num_infos: vec![],
        //             parsed_floats: vec![],
        //         },
        //     })).unwrap();
        //
        //     // Retrieve result and verify
        //     let result = response_receiver.recv().unwrap();
        //     // assert!(matches!(result, Ok(ReadResult::SortedSingleChunk(_))));
        //
        //     // handle.join().unwrap();
        // }

        #[test]
        fn test_empty_input() {
            let files = vec![Ok(Box::new(Cursor::new(vec![])) as Box<dyn Read + Send>)].into_iter();
            let temp_dir = tempdir().unwrap();
            let mut tmp_dir_wrapper = TmpDirWrapper::new(temp_dir.path().to_path_buf());

            let (sender, receiver) = mpsc::sync_channel(1);

            let result = ext_sort_read_write_loop::<MergeWriteablePlainTmpFile>(
                files,
                &mut tmp_dir_wrapper,
                b'\n',
                1024,
                &SortGlobalConfigs::default(),
                &receiver,
                sender,
            );

            assert!(matches!(result, Ok(ExtSortReadResult::EmptyInput)));
        }
    }

    mod write_lines_tests {
        use std::io::Cursor;

        use crate::SortLine;
        use crate::ext_sort::ext_sort_write_lines;

        #[test]
        fn test_write_lines() {
            let lines = vec![
                SortLine {
                    line: "Hello",
                    index: 0,
                },
                SortLine {
                    line: "World",
                    index: 0,
                },
                SortLine {
                    line: "Rust",
                    index: 0,
                },
            ];

            let mut writer = Cursor::new(Vec::new());
            ext_sort_write_lines(&lines, &mut writer, b'\n');
            let result = String::from_utf8(writer.into_inner()).unwrap();

            assert_eq!(result, "Hello\nWorld\nRust\n");
        }

        #[test]
        fn test_write_lines_empty_lines() {
            let lines: &[SortLine] = &[];

            let mut writer = Cursor::new(Vec::new());
            ext_sort_write_lines(lines, &mut writer, b'\n');
            let result = String::from_utf8(writer.into_inner()).unwrap();

            assert_eq!(result, "");
        }

        #[test]
        fn test_write_lines_single_line() {
            let lines = [SortLine {
                line: "Hello",
                index: 0,
            }];

            let mut writer = Cursor::new(Vec::new());
            ext_sort_write_lines(&lines, &mut writer, b'\n');
            let result = String::from_utf8(writer.into_inner()).unwrap();

            assert_eq!(result, "Hello\n");
        }

        #[test]
        fn test_write_lines_multiple_lines() {
            let lines = vec![
                SortLine {
                    line: "Hello",
                    index: 0,
                },
                SortLine {
                    line: "World",
                    index: 1,
                },
                SortLine {
                    line: "Rust",
                    index: 2,
                },
            ];

            let mut writer = Cursor::new(Vec::new());
            ext_sort_write_lines(&lines, &mut writer, b'\n');
            let result = String::from_utf8(writer.into_inner()).unwrap();

            assert_eq!(result, "Hello\nWorld\nRust\n");
        }

        #[test]
        fn test_write_lines_with_different_separator() {
            let lines = vec![
                SortLine {
                    line: "Hello",
                    index: 0,
                },
                SortLine {
                    line: "World",
                    index: 1,
                },
                SortLine {
                    line: "Rust",
                    index: 2,
                },
            ];

            let mut writer = Cursor::new(Vec::new());
            ext_sort_write_lines(&lines, &mut writer, b';');
            let result = String::from_utf8(writer.into_inner()).unwrap();

            assert_eq!(result, "Hello;World;Rust;");
        }

        #[test]
        fn test_write_lines_single_line_index2() {
            let lines = vec![SortLine {
                line: "Hello",
                index: 2,
            }];
            let mut writer = Cursor::new(Vec::new());
            ext_sort_write_lines(&lines, &mut writer, b'\n');
            let result = String::from_utf8(writer.into_inner()).unwrap();
            assert_eq!(result, "Hello\n");
        }

        #[test]
        fn test_write_lines_empty_lines_index2() {
            let lines = vec![
                SortLine { line: "", index: 2 },
                SortLine { line: "", index: 2 },
                SortLine { line: "", index: 2 },
            ];
            let mut writer = Cursor::new(Vec::new());
            ext_sort_write_lines(&lines, &mut writer, b'\n');
            let result = String::from_utf8(writer.into_inner()).unwrap();
            assert_eq!(result, "\n\n\n");
        }

        #[test]
        fn test_write_lines_separator_space() {
            let lines = vec![
                SortLine {
                    line: "Hello",
                    index: 0,
                },
                SortLine {
                    line: "World",
                    index: 0,
                },
                SortLine {
                    line: "Rust",
                    index: 0,
                },
            ];
            let mut writer = Cursor::new(Vec::new());
            ext_sort_write_lines(&lines, &mut writer, b' ');
            let result = String::from_utf8(writer.into_inner()).unwrap();
            assert_eq!(result, "Hello World Rust ");
        }

        #[test]
        fn test_write_lines_no_lines() {
            let lines: Vec<SortLine> = Vec::new();
            let mut writer = Cursor::new(Vec::new());
            ext_sort_write_lines(&lines, &mut writer, b'\n');
            let result = String::from_utf8(writer.into_inner()).unwrap();
            assert_eq!(result, "");
        }
    }
}
