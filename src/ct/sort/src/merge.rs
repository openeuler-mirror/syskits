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
//!
//! 我们通过在两个线程之间拆分排序和写入任务以及读取和解析任务来提高性能。
//! 线程通过通道进行通信。在阅读器->分拣器的方向上，每个文件有一个通道，但从sort返回阅读器只有
//! 从sort返回阅读器的通道只有一个。到sort的通道用于发送读取的数据块。
//! 当读取的数据行数耗尽后，sort需要下一个数据块时，就会从通道中读取下一个数据块。
//！从上一次读取的文件中读取下一个数据块。从sort返回到阅读器的通道有两个目的： 允许阅读器重用内存分配，并告诉阅读器下一步读取哪个文件。

use std::cmp::Ordering;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::iter;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::rc::Rc;
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender, SyncSender};
use std::thread::{self, JoinHandle};

use compare::Compare;
use itertools::Itertools;

use ctcore::ct_error::CTResult;

use crate::chunks::{self, Chunk, ChunkRecycled};
use crate::tmp_dir::TmpDirWrapper;
use crate::{sort_compare_by, sort_open};
use crate::{SortError, SortGlobalConfigs, SortOutput};

/// 如果输出文件也出现在输入文件中，则复制输出文件的内容
/// 并用该副本替换输入文件中出现的输出文件。
fn merge_replace_output_file_in_input_files(
    files: &mut [OsString],
    output: Option<&str>,
    tmp_dir: &mut TmpDirWrapper,
) -> CTResult<()> {
    let mut copy: Option<PathBuf> = None;
    if let Some(Ok(output_path)) = output.map(|path| Path::new(path).canonicalize()) {
        for file in files {
            if let Ok(file_path) = Path::new(file).canonicalize() {
                if file_path == output_path {
                    match &copy {
                        Some(copy) => {
                            *file = copy.clone().into_os_string();
                        }
                        _ => {
                            let (_file, copy_path) = tmp_dir.next_file()?;
                            std::fs::copy(file_path, &copy_path)
                                .map_err(|error| SortError::SortOpenTmpFileFailed { error })?;
                            *file = copy_path.clone().into_os_string();
                            copy = Some(copy_path);
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// 合并预先排序的 `Box<dyn Read>`s。
///
/// 如果 `settings.merge_batch_size` 大于 `files` 的长度，将使用中间文件。
/// 如果 `settings.compress_prog` 为 `Some`，中间文件将被压缩。
pub fn merge<'a>(
    files: &mut [OsString],
    settings: &'a SortGlobalConfigs,
    output: Option<&str>,
    tmp_dir: &mut TmpDirWrapper,
) -> CTResult<MergeFileMerger<'a>> {
    merge_replace_output_file_in_input_files(files, output, tmp_dir)?;
    match settings.compress_prog {
        Some(_) => merge_with_file_limit::<_, _, MergeWriteableCompressedTmpFile>(
            files
                .iter()
                .map(|file| sort_open(file).map(|file| PlainMergeInput { inner: file })),
            settings,
            tmp_dir,
        ),
        None => merge_with_file_limit::<_, _, MergeWriteablePlainTmpFile>(
            files
                .iter()
                .map(|file| sort_open(file).map(|file| PlainMergeInput { inner: file })),
            settings,
            tmp_dir,
        ),
    }
}

// 合并已排序的`MergeInput`s。
pub fn merge_with_file_limit<
    'a,
    M: MergeInput + 'static,
    F: ExactSizeIterator<Item = CTResult<M>>,
    Tmp: MergeWriteableTmpFile + 'static,
>(
    files: F,
    settings: &'a SortGlobalConfigs,
    tmp_dir: &mut TmpDirWrapper,
) -> CTResult<MergeFileMerger<'a>> {
    if files.len() > settings.merge_batch_size {
        let mut remaining_files_len = files.len();
        let batches = files.chunks(settings.merge_batch_size);
        let mut batches = batches.into_iter();
        let mut temporary_files_vec = vec![];
        while remaining_files_len != 0 {
            // Work around the fact that `Chunks` is not an `ExactSizeIterator`.
            remaining_files_len = remaining_files_len.saturating_sub(settings.merge_batch_size);
            let merger = merge_without_limit(batches.next().unwrap(), settings)?;
            let mut tmp_file =
                Tmp::create(tmp_dir.next_file()?, settings.compress_prog.as_deref())?;
            merger.write_all_to(settings, tmp_file.as_write())?;
            temporary_files_vec.push(tmp_file.finished_writing()?);
        }
        assert!(batches.next().is_none());
        merge_with_file_limit::<_, _, Tmp>(
            temporary_files_vec
                .into_iter()
                .map(Box::new(|c: Tmp::Closed| c.reopen())
                    as Box<
                        dyn FnMut(
                            Tmp::Closed,
                        )
                            -> CTResult<<Tmp::Closed as MergeClosedTmpFile>::Reopened>,
                    >),
            settings,
            tmp_dir,
        )
    } else {
        merge_without_limit(files, settings)
    }
}

/// 合并文件时不限制同时打开的文件数量。
///
/// 调用者有责任确保 `files` 只产生我们允许同时打开的尽可能多的文件。
/// 允许同时打开的文件数量。
fn merge_without_limit<M: MergeInput + 'static, F: Iterator<Item = CTResult<M>>>(
    files: F,
    sort_settings: &SortGlobalConfigs,
) -> CTResult<MergeFileMerger> {
    let (request_sender, request_receiver) = channel();
    let mut reader_files_vec = Vec::with_capacity(files.size_hint().0);
    let mut loaded_receivers_vec = Vec::with_capacity(files.size_hint().0);
    for (file_number, file) in files.enumerate() {
        let (sender, receiver) = sync_channel(2);
        loaded_receivers_vec.push(receiver);
        reader_files_vec.push(Some(MergeReaderFile {
            file: file?,
            sender,
            carry_over: vec![],
        }));
        // 发送初始块以触发每个文件的读取
        request_sender
            .send((file_number, ChunkRecycled::new(8 * 1024)))
            .unwrap();
    }

    // 为每个文件发送第二个数据块
    for file_number in 0..reader_files_vec.len() {
        request_sender
            .send((file_number, ChunkRecycled::new(8 * 1024)))
            .unwrap();
    }

    let reader_join_handle = thread::spawn({
        let settings = sort_settings.clone();
        move || {
            reader(
                &request_receiver,
                &mut reader_files_vec,
                &settings,
                settings.line_ending.into(),
            )
        }
    });

    let mut mergeable_files_vec = vec![];

    for (file_number, receiver) in loaded_receivers_vec.into_iter().enumerate() {
        if let Ok(chunk) = receiver.recv() {
            mergeable_files_vec.push(MergeableFile {
                current_chunk: Rc::new(chunk),
                file_number,
                line_idx: 0,
                receiver,
            });
        }
    }

    Ok(MergeFileMerger {
        heap: binary_heap_plus::BinaryHeap::from_vec_cmp(
            mergeable_files_vec,
            MergeFileComparator {
                settings: sort_settings,
            },
        ),
        request_sender,
        prev: None,
        reader_join_handle,
    })
}

/// 阅读器线程上代表输入文件的结构体
struct MergeReaderFile<M: MergeInput> {
    file: M,
    sender: SyncSender<Chunk>,
    carry_over: Vec<u8>,
}

/// 在阅读器线程上运行的函数。
fn reader(
    chunk_recycled_receiver: &Receiver<(usize, ChunkRecycled)>,
    files: &mut [Option<MergeReaderFile<impl MergeInput>>],
    sort_settings: &SortGlobalConfigs,
    separator: u8,
) -> CTResult<()> {
    for (file_idx, recycled_chunk) in chunk_recycled_receiver {
        if let Some(MergeReaderFile {
            file,
            sender,
            carry_over,
        }) = &mut files[file_idx]
        {
            let should_continue = chunks::chunk_read(
                sender,
                recycled_chunk,
                None,
                carry_over,
                file.as_read(),
                &mut iter::empty(),
                separator,
                sort_settings,
            )?;
            if !should_continue {
                // 用 `None` 替换文件，将其从列表中删除。
                let MergeReaderFile { file, .. } = files[file_idx].take().unwrap();
                // 根据 `MergeInput` 的类型，这可能会删除文件：
                file.finished_reading()?;
            }
        }
    }
    Ok(())
}

/// 主线程上代表输入文件的结构体
pub struct MergeableFile {
    current_chunk: Rc<Chunk>,
    line_idx: usize,
    receiver: Receiver<Chunk>,
    file_number: usize,
}

/// 一个用于跟踪我们遇到的前一行的结构。
///
/// 这是重复数据删除所必需的。
struct MergePreviousLine {
    chunk: Rc<Chunk>,
    line_idx: usize,
    file_number: usize,
}

/// 合并文件。这不是一个迭代器，因为存在寿命问题。
pub struct MergeFileMerger<'a> {
    heap: binary_heap_plus::BinaryHeap<MergeableFile, MergeFileComparator<'a>>,
    request_sender: Sender<(usize, ChunkRecycled)>,
    prev: Option<MergePreviousLine>,
    reader_join_handle: JoinHandle<CTResult<()>>,
}

impl<'a> MergeFileMerger<'a> {
    /// 将合并后的内容写入输出文件。
    pub fn write_all(self, settings: &SortGlobalConfigs, output: SortOutput) -> CTResult<()> {
        let mut out = output.into_write();
        self.write_all_to(settings, &mut out)
    }

    pub fn write_all_to(
        mut self,
        settings: &SortGlobalConfigs,
        out: &mut impl Write,
    ) -> CTResult<()> {
        while self.write_next(settings, out) {}
        drop(self.request_sender);
        self.reader_join_handle.join().unwrap()
    }

    fn write_next(&mut self, settings: &SortGlobalConfigs, out: &mut impl Write) -> bool {
        if let Some(file) = self.heap.peek() {
            let prev = self.prev.replace(MergePreviousLine {
                chunk: file.current_chunk.clone(),
                line_idx: file.line_idx,
                file_number: file.file_number,
            });

            file.current_chunk.with_dependent(|_, contents| {
                let current_line = &contents.lines[file.line_idx];
                if settings.is_unique {
                    if let Some(prev) = &prev {
                        let cmp = sort_compare_by(
                            &prev.chunk.lines()[prev.line_idx],
                            current_line,
                            settings,
                            prev.chunk.line_data(),
                            file.current_chunk.line_data(),
                        );
                        if cmp == Ordering::Equal {
                            return;
                        }
                    }
                }
                current_line.print(out, settings);
            });

            let was_last_line_for_file = file.current_chunk.lines().len() == file.line_idx + 1;

            if was_last_line_for_file {
                match file.receiver.recv() {
                    Ok(next_chunk) => {
                        let mut file = self.heap.peek_mut().unwrap();
                        file.current_chunk = Rc::new(next_chunk);
                        file.line_idx = 0;
                    }
                    _ => {
                        self.heap.pop();
                    }
                }
            } else {
                // 这将导致比较使用不同的行，堆将重新调整。
                self.heap.peek_mut().unwrap().line_idx += 1;
            }

            if let Some(prev) = prev {
                if let Ok(prev_chunk) = Rc::try_unwrap(prev.chunk) {
                    // 如果没有任何内容再引用前一个分块，这意味着前一行
                    // 是该语块的最后一行。我们就可以回收该块。
                    self.request_sender
                        .send((prev.file_number, prev_chunk.recycle()))
                        .ok();
                }
            }
        }
        !self.heap.is_empty()
    }
}

/// 按当前行比较文件。
struct MergeFileComparator<'a> {
    settings: &'a SortGlobalConfigs,
}

impl<'a> Compare<MergeableFile> for MergeFileComparator<'a> {
    fn compare(&self, a: &MergeableFile, b: &MergeableFile) -> Ordering {
        let mut cmp = sort_compare_by(
            &a.current_chunk.lines()[a.line_idx],
            &b.current_chunk.lines()[b.line_idx],
            self.settings,
            a.current_chunk.line_data(),
            b.current_chunk.line_data(),
        );
        if cmp == Ordering::Equal {
            // 为了保证排序的稳定性，我们还需要考虑文件编号、
            // 因为编号较低的文件中的行会被认为是 "较早 "的。
            cmp = a.file_number.cmp(&b.file_number);
        }
        // BinaryHeap 是一个最大堆。我们将其用作最小堆，因此需要颠倒排序。
        cmp.reverse()
    }
}

// 等待子代退出并检查其退出代码。
fn merge_check_child_success(mut child: Child, program: &str) -> CTResult<()> {
    match child.wait().map(|e| e.code()) {
        Ok(Some(0)) | Ok(None) | Err(_) => Ok(()),
        _ => Err(SortError::SortCompressProgTerminatedAbnormally {
            prog: program.to_owned(),
        }
        .into()),
    }
}

/// 可以写入的临时文件。
pub trait MergeWriteableTmpFile: Sized {
    type Closed: MergeClosedTmpFile;
    type InnerWrite: Write;
    fn create(file: (File, PathBuf), compress_prog: Option<&str>) -> CTResult<Self>;
    /// 关闭临时文件。
    fn finished_writing(self) -> CTResult<Self::Closed>;
    fn as_write(&mut self) -> &mut Self::InnerWrite;
}

/// 一个（暂时）关闭但可以重新打开的临时文件。
pub trait MergeClosedTmpFile {
    type Reopened: MergeInput;
    /// 重新打开临时文件。
    fn reopen(self) -> CTResult<Self::Reopened>;
}

/// A pre-sorted input for merging.
pub trait MergeInput: Send {
    type InnerRead: Read;
    /// 清理这个 `MergeInput` 。
    /// 实现可以删除后备文件。
    fn finished_reading(self) -> CTResult<()>;
    fn as_read(&mut self) -> &mut Self::InnerRead;
}

pub struct MergeWriteablePlainTmpFile {
    path: PathBuf,
    file: BufWriter<File>,
}

pub struct MergeClosedPlainTmpFile {
    path: PathBuf,
}

pub struct MergePlainTmpMergeInput {
    path: PathBuf,
    file: File,
}

impl MergeWriteableTmpFile for MergeWriteablePlainTmpFile {
    type Closed = MergeClosedPlainTmpFile;
    type InnerWrite = BufWriter<File>;

    fn create((file, path): (File, PathBuf), _: Option<&str>) -> CTResult<Self> {
        Ok(Self {
            file: BufWriter::new(file),
            path,
        })
    }

    fn finished_writing(self) -> CTResult<Self::Closed> {
        Ok(MergeClosedPlainTmpFile { path: self.path })
    }

    fn as_write(&mut self) -> &mut Self::InnerWrite {
        &mut self.file
    }
}

impl MergeClosedTmpFile for MergeClosedPlainTmpFile {
    type Reopened = MergePlainTmpMergeInput;
    fn reopen(self) -> CTResult<Self::Reopened> {
        Ok(MergePlainTmpMergeInput {
            file: File::open(&self.path)
                .map_err(|error| SortError::SortOpenTmpFileFailed { error })?,
            path: self.path,
        })
    }
}

impl MergeInput for MergePlainTmpMergeInput {
    type InnerRead = File;

    fn finished_reading(self) -> CTResult<()> {
        // 我们忽略删除临时文件的失败、
        // 因为在执行结束时会出现竞赛，整个
        // 临时目录可能已经删除。
        let _ = fs::remove_file(self.path);
        Ok(())
    }

    fn as_read(&mut self) -> &mut Self::InnerRead {
        &mut self.file
    }
}

pub struct MergeWriteableCompressedTmpFile {
    path: PathBuf,
    compress_prog: String,
    child: Child,
    child_stdin: BufWriter<ChildStdin>,
}

pub struct MergeClosedCompressedTmpFile {
    path: PathBuf,
    compress_prog: String,
}

pub struct MergeCompressedTmpMergeInput {
    path: PathBuf,
    compress_prog: String,
    child: Child,
    child_stdout: ChildStdout,
}

impl MergeWriteableTmpFile for MergeWriteableCompressedTmpFile {
    type Closed = MergeClosedCompressedTmpFile;
    type InnerWrite = BufWriter<ChildStdin>;

    fn create((file, path): (File, PathBuf), compress_prog: Option<&str>) -> CTResult<Self> {
        let compress_prog = compress_prog.unwrap();
        let mut command = Command::new(compress_prog);
        command.stdin(Stdio::piped()).stdout(file);
        let mut child =
            command
                .spawn()
                .map_err(|err| SortError::SortCompressProgExecutionFailed {
                    code: err.raw_os_error().unwrap(),
                })?;
        let child_stdin = child.stdin.take().unwrap();
        Ok(Self {
            path,
            compress_prog: compress_prog.to_owned(),
            child,
            child_stdin: BufWriter::new(child_stdin),
        })
    }

    fn finished_writing(self) -> CTResult<Self::Closed> {
        drop(self.child_stdin);
        merge_check_child_success(self.child, &self.compress_prog)?;
        Ok(MergeClosedCompressedTmpFile {
            path: self.path,
            compress_prog: self.compress_prog,
        })
    }

    fn as_write(&mut self) -> &mut Self::InnerWrite {
        &mut self.child_stdin
    }
}

impl MergeClosedTmpFile for MergeClosedCompressedTmpFile {
    type Reopened = MergeCompressedTmpMergeInput;

    fn reopen(self) -> CTResult<Self::Reopened> {
        let mut cmd = Command::new(&self.compress_prog);
        let file = File::open(&self.path).unwrap();
        cmd.stdin(file).stdout(Stdio::piped()).arg("-d");
        let mut child = cmd
            .spawn()
            .map_err(|err| SortError::SortCompressProgExecutionFailed {
                code: err.raw_os_error().unwrap(),
            })?;
        let child_stdout = child.stdout.take().unwrap();
        Ok(MergeCompressedTmpMergeInput {
            path: self.path,
            compress_prog: self.compress_prog,
            child,
            child_stdout,
        })
    }
}

impl MergeInput for MergeCompressedTmpMergeInput {
    type InnerRead = ChildStdout;

    fn finished_reading(self) -> CTResult<()> {
        drop(self.child_stdout);
        merge_check_child_success(self.child, &self.compress_prog)?;
        let _ = fs::remove_file(self.path);
        Ok(())
    }

    fn as_read(&mut self) -> &mut Self::InnerRead {
        &mut self.child_stdout
    }
}

pub struct PlainMergeInput<R: Read + Send> {
    inner: R,
}

impl<R: Read + Send> MergeInput for PlainMergeInput<R> {
    type InnerRead = R;
    fn finished_reading(self) -> CTResult<()> {
        Ok(())
    }
    fn as_read(&mut self) -> &mut Self::InnerRead {
        &mut self.inner
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::*;

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

    struct MockWriteableTmpFile {
        data: Cursor<Vec<u8>>,
    }

    impl MergeWriteableTmpFile for MockWriteableTmpFile {
        type Closed = Self;
        type InnerWrite = Cursor<Vec<u8>>;

        fn create(_: (File, PathBuf), _: Option<&str>) -> CTResult<Self> {
            Ok(Self {
                data: Cursor::new(Vec::new()),
            })
        }

        fn finished_writing(self) -> CTResult<Self::Closed> {
            Ok(self)
        }

        fn as_write(&mut self) -> &mut Self::InnerWrite {
            &mut self.data
        }
    }

    impl MergeClosedTmpFile for MockWriteableTmpFile {
        type Reopened = MockMergeInput;

        fn reopen(self) -> CTResult<Self::Reopened> {
            Ok(MockMergeInput {
                data: Cursor::new(self.data.into_inner()),
            })
        }
    }

    // fn create_reader_file(data: Vec<u8>) -> ReaderFile<MockMergeInput> {
    //     let (sender, _) = mpsc::sync_channel(1); // We won't receive in this test
    //     ReaderFile {
    //         file: MockMergeInput { data: Cursor::new(data) },
    //         sender,
    //         carry_over: vec![],
    //     }
    // }
    #[cfg(test)]
    mod compare_test {
        // use crate::chunks::{ChunkContents, LineData};
        // use crate::{GeneralF64ParseResult, Line};
        // use crate::numeric_str_cmp::{NumInfo, NumInfoParseSettings};
        // use super::*;
        //  fn setup_default_chunk() -> Chunk () {
        //      Chunk::new(vec![0; 10], |_buffer| {
        //          let lines = vec![
        //              Line {
        //                  line: "Line 1",
        //                  index: 0,
        //              },
        //              Line {
        //                  line: "Line 2",
        //                  index: 1,
        //              },
        //          ];
        //          let settings = NumInfoParseSettings::default();
        //          let a_info = NumInfo::parse("123e5", &settings).0;
        //          let b_info = NumInfo::parse("12300000", &settings).0;
        //          let line_data = LineData {
        //              selections: vec!["Selection 1", "Selection 2"],
        //              num_infos: vec![a_info, b_info],
        //              parsed_floats: vec![GeneralF64ParseResult::NaN, GeneralF64ParseResult::NaN],
        //          };
        //          ChunkContents { lines, line_data }
        //      })
        //  }
        //
        //
        //
        //
    }

    #[cfg(test)]
    mod merge_without_limit_test {
        use std::io::Cursor;

        use super::*;

        #[test]
        fn test_basic_merge() {
            let settings = SortGlobalConfigs {
                merge_batch_size: 100, // Not used here but must be set
                ..Default::default()
            };

            let files = vec![
                Ok(MockMergeInput {
                    data: Cursor::new(b"Hello".to_vec()),
                }),
                Ok(MockMergeInput {
                    data: Cursor::new(b"World".to_vec()),
                }),
            ];

            let result = merge_without_limit(files.into_iter(), &settings);
            assert!(result.is_ok());

            // Further checks to ensure merged output is correct
            let mut merger = result.unwrap();
            let mut output = Vec::new();
            while merger.write_next(&settings, &mut output) {}
            assert_eq!(String::from_utf8(output).unwrap(), "Hello\nWorld\n");
        }

        #[test]
        fn test_no_files() {
            let settings = SortGlobalConfigs {
                merge_batch_size: 100, // Not used here but must be set
                ..Default::default()
            };

            let files: Vec<CTResult<MockMergeInput>> = vec![];

            let result = merge_without_limit(files.into_iter(), &settings);
            assert!(result.is_ok());
            // Ensure the merger does not proceed with any operations
            let merger = result.unwrap();
            assert!(merger.heap.is_empty());
        }

        #[test]
        fn test_single_file() {
            let settings = SortGlobalConfigs {
                merge_batch_size: 100, // Not used here but must be set
                ..Default::default()
            };

            let files = vec![Ok(MockMergeInput {
                data: Cursor::new(b"Only one file\n".to_vec()),
            })];

            let result = merge_without_limit(files.into_iter(), &settings);
            assert!(result.is_ok());
            // Check if single file contents are processed correctly
            let mut merger = result.unwrap();
            let mut output = Vec::new();
            while merger.write_next(&settings, &mut output) {}
            assert_eq!(String::from_utf8(output).unwrap(), "Only one file\n");
        }

        #[test]
        fn test_handling_different_line_endings() {
            let settings = SortGlobalConfigs {
                merge_batch_size: 100, // Not used here
                ..Default::default()
            };

            let files = vec![
                Ok(MockMergeInput {
                    data: Cursor::new(b"Hello".to_vec()),
                }),
                Ok(MockMergeInput {
                    data: Cursor::new(b"World".to_vec()),
                }),
            ];

            let result = merge_without_limit(files.into_iter(), &settings);
            assert!(result.is_ok());

            let mut merger = result.unwrap();
            let mut output = Vec::new();
            while merger.write_next(&settings, &mut output) {}
            assert_eq!(String::from_utf8(output).unwrap(), "Hello\nWorld\n");
        }

        #[test]
        fn test_complex_merge_patterns() {
            let settings = SortGlobalConfigs {
                merge_batch_size: 100, // Not used here
                ..Default::default()
            };

            let files = vec![
                Ok(MockMergeInput {
                    data: Cursor::new(b"Apple\nBanana\nApple\n".to_vec()),
                }),
                Ok(MockMergeInput {
                    data: Cursor::new(b"Banana\nApple\nBanana\n".to_vec()),
                }),
            ];

            let result = merge_without_limit(files.into_iter(), &settings);
            assert!(result.is_ok());

            let mut merger = result.unwrap();
            let mut output = Vec::new();
            while merger.write_next(&settings, &mut output) {}
            // The expected output should be checked against a valid sorted order if applicable
        }

        #[test]
        fn test_large_data_volume() {
            let settings = SortGlobalConfigs {
                merge_batch_size: 100,
                ..Default::default()
            };

            let large_input = "Line\n".repeat(10000); // Simulate large data volume
            let large_input_vec = large_input.into_bytes();
            let files = vec![
                Ok(MockMergeInput {
                    data: Cursor::new(large_input_vec.clone()),
                }),
                Ok(MockMergeInput {
                    data: Cursor::new(large_input_vec),
                }),
            ];

            let result = merge_without_limit(files.into_iter(), &settings);
            assert!(result.is_ok());

            let mut merger = result.unwrap();
            let mut output = Vec::new();
            while merger.write_next(&settings, &mut output) {}
            assert!(output.len() > 0, "Output should be large but is empty");
        }
    }

    #[cfg(test)]
    mod merge_with_file_limit_test {
        use super::*;

        #[test]
        fn test_basic_merge_without_limits() {
            let files = vec![
                Ok(MockMergeInput {
                    data: Cursor::new(b"Hello".to_vec()),
                }),
                Ok(MockMergeInput {
                    data: Cursor::new(b"World".to_vec()),
                }),
            ];

            let tmp_dir = TempDir::new().unwrap();
            let mut tmp_dir_wrapper = TmpDirWrapper::new(tmp_dir.path().to_path_buf());
            let settings = SortGlobalConfigs {
                merge_batch_size: 10, // Set higher than the number of files
                compress_prog: None,  // Other settings are set as needed
                ..Default::default()
            };

            let result = merge_with_file_limit::<_, _, MockWriteableTmpFile>(
                files.into_iter(),
                &settings,
                &mut tmp_dir_wrapper,
            );

            assert!(result.is_ok());
        }

        #[test]
        fn test_batch_processing() {
            let num_files = 25; // More than typical batch size to force batching
            let batch_size = 10;
            let mut files = vec![];
            for _ in 0..num_files {
                files.push(Ok(MockMergeInput {
                    data: Cursor::new(b"Data".to_vec()),
                }));
            }

            let tmp_dir = TempDir::new().unwrap();
            let mut tmp_dir_wrapper = TmpDirWrapper::new(tmp_dir.path().to_path_buf());
            let settings = SortGlobalConfigs {
                merge_batch_size: batch_size,
                compress_prog: None,
                ..Default::default()
            };

            let result = merge_with_file_limit::<_, _, MockWriteableTmpFile>(
                files.into_iter(),
                &settings,
                &mut tmp_dir_wrapper,
            );

            assert!(result.is_ok());
        }

        #[test]
        fn test_no_files_provided() {
            let tmp_dir = TempDir::new().unwrap();
            let mut tmp_dir_wrapper = TmpDirWrapper::new(tmp_dir.path().to_path_buf());
            let settings = SortGlobalConfigs {
                merge_batch_size: 10,
                compress_prog: None,
                ..Default::default()
            };

            let files: Vec<CTResult<MockMergeInput>> = vec![];

            let result = merge_with_file_limit::<_, _, MockWriteableTmpFile>(
                files.into_iter(),
                &settings,
                &mut tmp_dir_wrapper,
            );

            assert!(matches!(result, Ok(_))); // Expecting no error even if no files are provided
        }

        #[test]
        fn test_single_file_provided() {
            let tmp_dir = TempDir::new().unwrap();
            let mut tmp_dir_wrapper = TmpDirWrapper::new(tmp_dir.path().to_path_buf());
            let settings = SortGlobalConfigs {
                merge_batch_size: 10,
                compress_prog: None,
                ..Default::default()
            };

            let files = vec![Ok(MockMergeInput {
                data: Cursor::new(b"Single file".to_vec()),
            })];

            let result = merge_with_file_limit::<_, _, MockWriteableTmpFile>(
                files.into_iter(),
                &settings,
                &mut tmp_dir_wrapper,
            );

            assert!(result.is_ok()); // Should handle merging of a single file gracefully
        }
    }

    #[cfg(test)]
    mod merge_tests {
        use std::ffi::OsString;
        use std::path::PathBuf;

        use tempfile::tempdir;

        use ctcore::ct_line_ending::CtLineEnding;

        use crate::{SortMode, SortPrecomputed};

        use super::*;

        #[test]
        fn test_merge_without_compression() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz\nworld 2200 ccccc\nCtyunOs 2000 aaaaa\nCtyunOs 1900 ababa"
            )
            .expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(
                tmp_file2,
                "Hello1 1001 zzzzz1\nworld1 2201 ccccc1\nCtyunOs1 2001 aaaaa1\nCtyunOs1 1901 ababa1"
            )
            .expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs::default();

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_without_compression_nofile_err() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let file_path2 = dir.path().join("sort_test_file2");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs::default();

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            // println!("{:?}", result);
            assert!(result.is_err());
        }

        #[test]
        fn test_merge_output_file_in_list() {
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(
                tmp_file,
                "Hello 1000 zzzzz\nworld 2200 ccccc\nCtyunOs 2000 aaaaa\nCtyunOs 1900 ababa"
            )
            .expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(
                tmp_file2,
                "Hello1 1001 zzzzz1\nworld1 2201 ccccc1\nCtyunOs1 2001 aaaaa1\nCtyunOs1 1901 ababa1"
            )
            .expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];
            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/some/path"));
            let settings = SortGlobalConfigs::default();

            let result = merge(
                &mut files,
                &settings,
                Some("/path/to/output.txt"),
                &mut tmp_dir,
            );
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_output_file_in_list_nofile_err() {
            let mut files = vec![
                OsString::from("/path/to/output.txt"),
                OsString::from("/path/to/file2.txt"),
            ];
            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/some/path"));
            let settings = SortGlobalConfigs::default();

            let result = merge(
                &mut files,
                &settings,
                Some("/path/to/output.txt"),
                &mut tmp_dir,
            );
            assert!(result.is_err());
            assert_eq!(files[0], OsString::from("/path/to/output.txt")); // Output should be replaced
        }

        #[test]
        fn test_merge_failure_to_open_file() {
            // This test assumes there will be an error in opening the files
            let mut files = vec![OsString::from("/nonexistent/path/file1.txt")];
            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/some/path"));
            let settings = SortGlobalConfigs::default();

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_err());
        }

        #[test]
        fn test_merge_tmp_dir_creation_failure() {
            // This test assumes that temp directory creation will fail
            let mut files = vec![OsString::from("/path/to/file1.txt")];
            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/some/path"));
            // Not setting temp_dir to simulate creation failure
            let settings = SortGlobalConfigs::default();

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_err());
        }

        #[test]
        fn test_merge_ignore_case_true() {
            let input = "line1\nLINE4\nline3";
            let input2 = "line2\nLINE7\nline3";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                is_ignore_case: true,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_ignore_case_false() {
            let input = "line1\nLINE2\nline3";
            let input2 = "line2\nLINE7\nline3";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                is_ignore_case: false,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_different_line_endings_newline() {
            let input = "Windows\r\nUnix\nMac\r";
            let input2 = "HPUnix\nMac\r";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                line_ending: CtLineEnding::Newline,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_different_line_endings_nul() {
            let input = "Windows\r\nUnix\nMac\r";
            let input2 = "HPUnix\nMac\r";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                line_ending: CtLineEnding::Nul,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }
    
        #[test]
        fn test_merge_unique_lines_true() {
            let input = "line\nline\nline";
            let input2 = "line2\nLINE7\nline3";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                is_unique: true,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_unique_lines_false() {
            let input = "line\nline\nline";
            let input2 = "line2\nLINE7\nline3";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                is_unique: false,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_reverse_order_true() {
            let input = "line1\nLINE4\nline3";
            let input2 = "line2\nLINE7\nline3";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                is_reverse: true,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_reverse_order_false() {
            let input = "line1\nLINE4\nline3";
            let input2 = "line2\nLINE7\nline3";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                is_reverse: false,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_check_sorted_true() {
            let input = "line1\nline3\nline2"; // Intentionally unsorted
            let input2 = "line2\nLINE7\nline3";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                is_check: true,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_check_sorted_false() {
            let input = "line1\nline3\nline2"; // Intentionally unsorted
            let input2 = "line2\nLINE7\nline3";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                is_check: false,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_check_silent_true() {
            let input = "line1\nLINE4\nline3";
            let input2 = "line2\nLINE7\nline3";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                is_check_silent: true,
                ..Default::default()
            };
            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_check_silent_false() {
            let input = "line1\nLINE4\nline3";
            let input2 = "line2\nLINE7\nline3";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                is_check_silent: false,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_ignore_leading_blanks_true() {
            let input = "line1\nLINE4\nline3";
            let input2 = "line2\nLINE7\nline3";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                is_ignore_leading_blanks: true,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_ignore_leading_blanks_false() {
            let input = "line1\nLINE4\nline3";
            let input2 = "line2\nLINE7\nline3";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                is_ignore_leading_blanks: false,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_dictionary_order_true() {
            let input = "line1\nLINE4\nline3";
            let input2 = "line2\nLINE7\nline3";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                is_dictionary_order: true,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_dictionary_order_false() {
            let input = "line1\nLINE4\nline3";
            let input2 = "line2\nLINE7\nline3";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                is_dictionary_order: false,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_merge_true() {
            let input = "line1\nLINE4\nline3";
            let input2 = "line2\nLINE7\nline3";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                is_merge: true,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_merge_false() {
            let input = "line1\nLINE4\nline3";
            let input2 = "line2\nLINE7\nline3";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                is_merge: false,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_debug_true() {
            let input = "line1\nLINE4\nline3";
            let input2 = "line2\nLINE7\nline3";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                is_debug: true,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_debug_false() {
            let input = "line1\nLINE4\nline3";
            let input2 = "line2\nLINE7\nline3";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                is_debug: false,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_stable_true() {
            let input = "line1\nLINE4\nline3";
            let input2 = "line2\nLINE7\nline3";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                is_stable: true,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_stable_false() {
            let input = "line1\nLINE4\nline3";
            let input2 = "line2\nLINE7\nline3";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                is_stable: false,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_sort_mode_default() {
            let input = "line1\nLINE4\nline3";
            let input2 = "line2\nLINE7\nline3";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                mode: SortMode::SortDefault,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_sort_mode_version() {
            let input = "line1\nLINE4\nline3";
            let input2 = "line2\nLINE7\nline3";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                mode: SortMode::SortVersion,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_sort_mode_month() {
            let input = "April\nOctober\nJuly\nJune\nSeptember";
            let input2 = "July\nAugust\nMay\nJune";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                mode: SortMode::SortMonth,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_sort_mode_random() {
            let input = "11\n11\n12\n111";
            let input2 = "22\n33\n44\n222";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                mode: SortMode::SortRandom,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_sort_mode_human_numeric() {
            let input = "11\n11\n12\n111";
            let input2 = "22\n33\n44\n222";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                mode: SortMode::SortHumanNumeric,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_sort_mode_numeric() {
            let input = "11\n11\n12\n111";
            let input2 = "22\n33\n44\n222";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                mode: SortMode::SortNumeric,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_sort_mode_general_numeric() {
            let input = "11\n11\n12\n111";
            let input2 = "22\n33\n44\n222";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                mode: SortMode::SortGeneralNumeric,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_salt_none() {
            let input = "11\n11\n12\n111";
            let input2 = "22\n33\n44\n222";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));

            let settings = SortGlobalConfigs {
                salt: None,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_salt_some_0() {
            let input = "line1\nline2\nline23\nline31";
            let input2 = "line11\nline12\nline33\nline11";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                salt: Some([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_salt_some_digit() {
            let input = "line1\nline2\nline23\nline31";
            let input2 = "line11\nline12\nline33\nline11";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                salt: Some([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]),
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_separator_none() {
            let input = "line1\nline2\nline23\nline31";
            let input2 = "line11\nline12\nline33\nline11";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                separator: None,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_threads_none() {
            let input = "line1\nline2\nline23\nline31";
            let input2 = "line11\nline12\nline33\nline11";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                threads: String::new(),
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }
    
        #[test]
        fn test_merge_threads_qq() {
            let input = "line1\nline2\nline23\nline31";
            let input2 = "line11\nline12\nline33\nline11";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                threads: String::from("qq"),
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_buffer_size_1000000() {
            let input = "line1\nline2\nline23\nline31";
            let input2 = "line11\nline12\nline33\nline11";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                buffer_size: 1000000,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_buffer_size_0() {
            let input = "line1\nline2\nline23\nline31";
            let input2 = "line11\nline12\nline33\nline11";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                buffer_size: 0,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_compress_prog_none() {
            let input = "line1\nline2\nline23\nline31";
            let input2 = "line11\nline12\nline33\nline11";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                compress_prog: None,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_compress_prog_tar() {
            let input = "line1\nline2\nline23\nline31";
            let input2 = "line11\nline12\nline33\nline11";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                compress_prog: Some("tar".to_string()),
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_compress_prog_zip() {
            let input = "line1\nline2\nline23\nline31";
            let input2 = "line11\nline12\nline33\nline11";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                compress_prog: Some("zip".to_string()),
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_merge_batch_size_1() {
            let input = "line1\nline2\nline23\nline31";
            let input2 = "line11\nline12\nline33\nline11";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                merge_batch_size: 1,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_err());
        }

        #[test]
        fn test_merge_merge_batch_size_32() {
            let input = "line1\nline2\nline23\nline31";
            let input2 = "line11\nline12\nline33\nline11";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                merge_batch_size: 32,
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_precomputed_default() {
            let input = "line1\nline2\nline23\nline31";
            let input2 = "line11\nline12\nline33\nline11";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                precomputed: SortPrecomputed::default(),
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_precomputed_needs_tokens_true() {
            let input = "line1\nline2\nline23\nline31";
            let input2 = "line11\nline12\nline33\nline11";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                precomputed: SortPrecomputed {
                    is_needs_tokens: true,
                    num_infos_per_line: 0,
                    floats_per_line: 0,
                    selections_per_line: 0,
                },
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_precomputed_needs_tokens_false() {
            let input = "line1\nline2\nline23\nline31";
            let input2 = "line11\nline12\nline33\nline11";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                precomputed: SortPrecomputed {
                    is_needs_tokens: false,
                    num_infos_per_line: 0,
                    floats_per_line: 0,
                    selections_per_line: 0,
                },
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_precomputed_needs_tokens_true_1_1_1() {
            let input = "line1\nline2\nline23\nline31";
            let input2 = "line11\nline12\nline33\nline11";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                precomputed: SortPrecomputed {
                    is_needs_tokens: true,
                    num_infos_per_line: 1,
                    floats_per_line: 1,
                    selections_per_line: 1,
                },
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_precomputed_needs_tokens_false_1_1_1() {
            let input = "line1\nline2\nline23\nline31";
            let input2 = "line11\nline12\nline33\nline11";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                precomputed: SortPrecomputed {
                    is_needs_tokens: false,
                    num_infos_per_line: 1,
                    floats_per_line: 1,
                    selections_per_line: 1,
                },
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }

        #[test]
        fn test_merge_selectors_default() {
            let input = "line1\nline2\nline23\nline31";
            let input2 = "line11\nline12\nline33\nline11";
            let dir = tempdir().unwrap();
            let file_path = dir.path().join("sort_test_file");
            let mut tmp_file = File::create(&file_path).unwrap();
            writeln!(tmp_file, "{}", input).expect("TODO: panic message");

            let file_path2 = dir.path().join("sort_test_file2");
            let mut tmp_file2 = File::create(&file_path2).unwrap();
            writeln!(tmp_file2, "{}", input2).expect("TODO: panic message");

            let mut files = vec![file_path.into_os_string(), file_path2.into_os_string()];

            let mut tmp_dir = TmpDirWrapper::new(PathBuf::from("/tmp/path"));
            let settings = SortGlobalConfigs {
                selectors: vec![],
                ..Default::default()
            };

            let result = merge(&mut files, &settings, None, &mut tmp_dir);
            assert!(result.is_ok());
        }
    }
}