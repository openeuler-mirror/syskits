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

use std::ffi::OsStr;
use std::fs::{File, Metadata};
use std::io::{self, BufReader, Read, Seek, SeekFrom};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

use ctcore::ct_display::Quotable;
use ctcore::ct_error::strip_errno;
use ctcore::ct_show_error;
use ctcore::libc;

/// 输入源的枚举类型，表示不同类型的输入来源
pub enum OdInputSource<'a> {
    /// 文件名作为输入源
    FileName(&'a OsStr),
    /// 标准输入作为输入源
    Stdin,
    /// 任意实现了 Read trait 的流作为输入源
    #[allow(dead_code)]
    Stream(Box<dyn io::Read>),
}

/// 多文件读取器 - 将所有输入（文件或标准输入）连接在一起
pub struct OdMultifileReader<'a> {
    /// 待处理的输入源列表
    ni: Vec<OdInputSource<'a>>,
    /// 当前正在读取的文件
    curr_file: Option<OdCurrentInput>,
    /// 是否至少成功打开或选择过一个输入
    has_opened_input: bool,
    /// 当前输入的名称，用于读取错误诊断
    curr_name: Option<&'a OsStr>,
    /// 是否发生过任何错误
    is_any_err: bool,
}

enum OdCurrentInput {
    RegularFile(BufReader<File>),
    Stream(Box<dyn Read>),
}

impl Read for OdCurrentInput {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::RegularFile(file) => file.read(buf),
            Self::Stream(stream) => stream.read(buf),
        }
    }
}

fn has_usable_regular_file_size(metadata: &Metadata) -> bool {
    if !metadata.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        metadata.len() > metadata.blksize()
    }

    #[cfg(not(unix))]
    {
        metadata.len() > 8192
    }
}

/// 错误状态检查接口
pub trait HasError {
    /// 检查是否发生过错误
    fn has_error(&self) -> bool;

    /// 检查是否至少成功打开或选择过一个输入
    fn has_input(&self) -> bool;
}

#[cfg(unix)]
struct UnbufferedStdin;

#[cfg(unix)]
impl io::Read for UnbufferedStdin {
    /// 使用底层的 libc::read 实现绝对的无缓冲读取。
    /// 这样当上层 (PartialReader) 限制只读 N 个字节时，
    /// 管道中剩余的数据不会被多余的缓冲区吸走，留给后续进程继续使用。
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let res = unsafe { libc::read(0, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if res < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(res as usize)
        }
    }
}

impl OdMultifileReader<'_> {
    /// 创建新的多文件读取器
    pub fn new(fnames: Vec<OdInputSource>) -> OdMultifileReader {
        let mut mf = OdMultifileReader {
            ni: fnames,
            curr_file: None,
            has_opened_input: false,
            curr_name: None,
            is_any_err: false,
        };
        mf.next_file();
        mf
    }

    fn next_file(&mut self) {
        while !self.ni.is_empty() {
            match self.ni.remove(0) {
                OdInputSource::Stdin => {
                    self.has_opened_input = true;
                    self.curr_name = Some(OsStr::new("standard input"));
                    self.curr_file = Some(OdCurrentInput::Stream(
                        match ctcore::ct_io::injected_stdin_bytes() {
                            Some(_) => ctcore::ct_io::stdin_reader_box(),
                            None => {
                                #[cfg(unix)]
                                {
                                    // 使用无缓冲的标准输入，避免 over-read
                                    Box::new(UnbufferedStdin) as Box<dyn io::Read>
                                }
                                #[cfg(not(unix))]
                                {
                                    // Windows 等非 Unix 系统暂退回标准 stdin
                                    Box::new(std::io::stdin()) as Box<dyn io::Read>
                                }
                            }
                        },
                    ));
                    return;
                }
                OdInputSource::FileName(fname) => match File::open(fname) {
                    Ok(f) => {
                        self.has_opened_input = true;
                        self.curr_name = Some(fname);
                        self.curr_file = Some(OdCurrentInput::RegularFile(BufReader::new(f)));
                        return;
                    }
                    Err(e) => {
                        ct_show_error!("{}: {}", fname.maybe_quote(), strip_errno(&e));
                        self.is_any_err = true;
                    }
                },
                OdInputSource::Stream(s) => {
                    self.has_opened_input = true;
                    self.curr_name = None;
                    self.curr_file = Some(OdCurrentInput::Stream(s));
                    return;
                }
            }
        }
        self.curr_name = None;
        self.curr_file = None;
    }

    pub fn skip_regular_files(&mut self, mut bytes: u64) -> u64 {
        while bytes > 0 {
            let Some(OdCurrentInput::RegularFile(file)) = self.curr_file.as_mut() else {
                break;
            };
            let Ok(metadata) = file.get_ref().metadata() else {
                break;
            };
            if !has_usable_regular_file_size(&metadata) {
                break;
            }
            let Ok(position) = file.stream_position() else {
                break;
            };
            let available = metadata.len().saturating_sub(position);

            if available < bytes {
                bytes -= available;
                self.next_file();
                continue;
            }

            if file.seek(SeekFrom::Start(position + bytes)).is_ok() {
                bytes = 0;
            }
            break;
        }

        bytes
    }
}

impl io::Read for OdMultifileReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut xfrd = 0;
        'fillloop: while xfrd < buf.len() {
            let curr_name = self.curr_name;
            match self.curr_file {
                None => break,
                Some(ref mut curr_file) => loop {
                    xfrd += match curr_file.read(&mut buf[xfrd..]) {
                        Ok(0) => break,
                        Ok(n) => n,
                        Err(e) => {
                            if let Some(name) = curr_name {
                                ct_show_error!("{}: {}", name.maybe_quote(), strip_errno(&e));
                            } else {
                                ct_show_error!("I/O: {}", strip_errno(&e));
                            }
                            self.is_any_err = true;
                            break;
                        }
                    };
                    if xfrd == buf.len() {
                        break 'fillloop;
                    }
                },
            }
            self.next_file();
        }
        Ok(xfrd)
    }
}

impl HasError for OdMultifileReader<'_> {
    fn has_error(&self) -> bool {
        self.is_any_err
    }

    fn has_input(&self) -> bool {
        self.has_opened_input
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mockstream::*;
    use std::io::{Cursor, ErrorKind, Read};

    #[test]
    fn test_skip_large_regular_file() {
        let path =
            std::env::temp_dir().join(format!("syskits-od-seek-skip-{}", std::process::id()));
        let file = File::create(&path).unwrap();
        file.set_len(1024 * 1024).unwrap();
        drop(file);

        let mut reader = OdMultifileReader::new(vec![OdInputSource::FileName(path.as_os_str())]);
        assert_eq!(reader.skip_regular_files(1024 * 1024 - 1), 0);
        let mut byte = [1];
        assert_eq!(reader.read(&mut byte).unwrap(), 1);
        assert_eq!(byte, [0]);

        drop(reader);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn test_multi_file_reader_one_read() {
        let inputs = vec![
            OdInputSource::Stream(Box::new(Cursor::new(&b"abcd"[..]))),
            OdInputSource::Stream(Box::new(Cursor::new(&b"ABCD"[..]))),
        ];
        let mut v = [0; 10];

        let mut sut = OdMultifileReader::new(inputs);

        assert_eq!(sut.read(v.as_mut()).unwrap(), 8);
        assert_eq!(v, [0x61, 0x62, 0x63, 0x64, 0x41, 0x42, 0x43, 0x44, 0, 0]);
        assert_eq!(sut.read(v.as_mut()).unwrap(), 0);
    }

    #[test]
    fn test_multi_file_reader_two_reads() {
        let inputs = vec![
            OdInputSource::Stream(Box::new(Cursor::new(&b"abcd"[..]))),
            OdInputSource::Stream(Box::new(Cursor::new(&b"ABCD"[..]))),
        ];
        let mut v = [0; 5];

        let mut sut = OdMultifileReader::new(inputs);

        assert_eq!(sut.read(v.as_mut()).unwrap(), 5);
        assert_eq!(v, [0x61, 0x62, 0x63, 0x64, 0x41]);
        assert_eq!(sut.read(v.as_mut()).unwrap(), 3);
        assert_eq!(v, [0x42, 0x43, 0x44, 0x64, 0x41]); // last two bytes are not overwritten
    }

    #[test]
    fn test_multi_file_reader_read_error() {
        let c = Cursor::new(&b"1234"[..])
            .chain(OdFailingMockStream::new(ErrorKind::Other, "Failing", 1))
            .chain(Cursor::new(&b"5678"[..]));
        let inputs = vec![
            OdInputSource::Stream(Box::new(c)),
            OdInputSource::Stream(Box::new(Cursor::new(&b"ABCD"[..]))),
        ];
        let mut v = [0; 5];

        let mut sut = OdMultifileReader::new(inputs);

        assert_eq!(sut.read(v.as_mut()).unwrap(), 5);
        assert_eq!(v, [49, 50, 51, 52, 65]);
        assert_eq!(sut.read(v.as_mut()).unwrap(), 3);
        assert_eq!(v, [66, 67, 68, 52, 65]); // last two bytes are not overwritten

        // note: no retry on i/o error, so 5678 is missing
    }

    #[test]
    fn test_multi_file_reader_read_error_at_start() {
        let inputs = vec![
            OdInputSource::Stream(Box::new(OdFailingMockStream::new(
                ErrorKind::Other,
                "Failing",
                1,
            ))),
            OdInputSource::Stream(Box::new(Cursor::new(&b"abcd"[..]))),
            OdInputSource::Stream(Box::new(OdFailingMockStream::new(
                ErrorKind::Other,
                "Failing",
                1,
            ))),
            OdInputSource::Stream(Box::new(Cursor::new(&b"ABCD"[..]))),
            OdInputSource::Stream(Box::new(OdFailingMockStream::new(
                ErrorKind::Other,
                "Failing",
                1,
            ))),
        ];
        let mut v = [0; 5];

        let mut sut = OdMultifileReader::new(inputs);

        assert_eq!(sut.read(v.as_mut()).unwrap(), 5);
        assert_eq!(v, [0x61, 0x62, 0x63, 0x64, 0x41]);
        assert_eq!(sut.read(v.as_mut()).unwrap(), 3);
        assert_eq!(v, [0x42, 0x43, 0x44, 0x64, 0x41]); // last two bytes are not overwritten
    }

    #[test]
    fn test_next_file() {
        // 测试空输入列表
        let reader = OdMultifileReader::new(vec![]);
        assert!(reader.curr_file.is_none());
        assert!(!reader.has_error());

        // 测试单个流
        let inputs = vec![OdInputSource::Stream(Box::new(Cursor::new(&b"test"[..])))];
        let reader = OdMultifileReader::new(inputs);
        assert!(reader.curr_file.is_some());
        assert!(!reader.has_error());

        // 测试多个流的切换
        let inputs = vec![
            OdInputSource::Stream(Box::new(Cursor::new(&b"first"[..]))),
            OdInputSource::Stream(Box::new(Cursor::new(&b"second"[..]))),
        ];
        let mut reader = OdMultifileReader::new(inputs);
        assert!(reader.curr_file.is_some());
        reader.next_file();
        assert!(reader.curr_file.is_some());
        reader.next_file();
        assert!(reader.curr_file.is_none());
        assert!(!reader.has_error());

        // 测试无效文件名处理
        let inputs = vec![
            OdInputSource::FileName(OsStr::new("nonexistent_file.txt")),
            OdInputSource::Stream(Box::new(Cursor::new(&b"valid"[..]))),
        ];
        let reader = OdMultifileReader::new(inputs);
        // 第一个文件应该失败，但会自动切换到第二个
        assert!(reader.curr_file.is_some());
        assert!(reader.has_error());

        // 测试混合输入源
        let inputs = vec![
            OdInputSource::Stream(Box::new(Cursor::new(&b"stream"[..]))),
            OdInputSource::FileName(OsStr::new("nonexistent_file.txt")),
            OdInputSource::Stream(Box::new(Cursor::new(&b"another"[..]))),
        ];
        let mut reader = OdMultifileReader::new(inputs);
        assert!(reader.curr_file.is_some());
        reader.next_file(); // 尝试打开不存在的文件，会跳到下一个
        assert!(reader.curr_file.is_some());
        assert!(reader.has_error());
    }
}
