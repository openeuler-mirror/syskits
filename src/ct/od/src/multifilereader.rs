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

use std::fs::File;
use std::io::{self, BufReader};

use ctcore::ct_display::Quotable;
use ctcore::ct_show_error;

/// 输入源的枚举类型，表示不同类型的输入来源
pub enum OdInputSource<'a> {
    /// 文件名作为输入源
    FileName(&'a str),
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
    curr_file: Option<Box<dyn io::Read>>,
    /// 是否发生过任何错误
    is_any_err: bool,
}

/// 错误状态检查接口
pub trait HasError {
    /// 检查是否发生过错误
    fn has_error(&self) -> bool;
}

impl OdMultifileReader<'_> {
    /// 创建新的多文件读取器
    ///
    /// # 参数
    /// * `fnames` - 输入源列表
    pub fn new(fnames: Vec<OdInputSource>) -> OdMultifileReader {
        let mut mf = OdMultifileReader {
            ni: fnames,
            curr_file: None, // 通常表示已完成；需要调用 next_file()
            is_any_err: false,
        };
        mf.next_file();
        mf
    }

    /// 切换到下一个输入文件
    ///
    /// # 功能
    /// - 从输入源列表中获取并处理下一个输入源
    /// - 根据输入源类型执行相应的初始化
    /// - 处理文件打开失败的情况
    ///
    /// # 实现细节
    /// - 如果输入列表为空，设置 curr_file 为 None
    /// - 对于标准输入，创建带缓冲的标准输入读取器
    /// - 对于文件名，尝试打开文件，失败时记录错误并继续处理下一个
    /// - 对于流，直接使用作为当前文件
    fn next_file(&mut self) {
        // 循环处理输入源，直到成功打开一个或处理完所有输入
        while !self.ni.is_empty() {
            // 获取并移除列表中的第一个输入源
            match self.ni.remove(0) {
                OdInputSource::Stdin => {
                    // 标准输入：创建缓冲读取器并退出循环
                    self.curr_file = Some(Box::new(BufReader::new(std::io::stdin())));
                    return;
                }
                OdInputSource::FileName(fname) => {
                    // 文件输入：尝试打开文件
                    match File::open(fname) {
                        Ok(f) => {
                            // 成功打开文件：创建缓冲读取器并退出循环
                            self.curr_file = Some(Box::new(BufReader::new(f)));
                            return;
                        }
                        Err(e) => {
                            // 文件打开失败：记录错误并继续处理下一个输入源
                            ct_show_error!("{}: {}", fname.maybe_quote(), e);
                            self.is_any_err = true;
                            // 继续循环尝试下一个输入源
                        }
                    }
                }
                OdInputSource::Stream(s) => {
                    // 流输入：直接使用并退出循环
                    self.curr_file = Some(s);
                    return;
                }
            }
        }

        // 所有输入源都处理完毕或都失败了
        self.curr_file = None;
    }
}

impl io::Read for OdMultifileReader<'_> {
    /// 从文件列表中读取字节填充缓冲区
    ///
    /// # 返回值
    /// * `Ok(读取的字节数)`
    ///
    /// 内部处理 IO 错误，因此总是返回 Ok
    /// 除非已经没有输入，否则会尝试完全填充提供的缓冲区
    /// 如果任何一次调用返回的数据少于 buf.len()，后续所有调用都将返回 Ok(0)
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut xfrd = 0;
        // 当缓冲区未满时继续读取，可能会读取多个文件
        'fillloop: while xfrd < buf.len() {
            match self.curr_file {
                None => break,
                Some(ref mut curr_file) => {
                    loop {
                        // 标准输入可能在按回车时返回，即使缓冲区未满
                        xfrd += match curr_file.read(&mut buf[xfrd..]) {
                            Ok(0) => break,
                            Ok(n) => n,
                            Err(e) => {
                                ct_show_error!("I/O: {}", e);
                                self.is_any_err = true;
                                break;
                            }
                        };
                        if xfrd == buf.len() {
                            // 已传输请求的所有数据
                            break 'fillloop;
                        }
                    }
                }
            }
            self.next_file();
        }
        Ok(xfrd)
    }
}

impl HasError for OdMultifileReader<'_> {
    /// 返回是否发生过任何错误
    fn has_error(&self) -> bool {
        self.is_any_err
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mockstream::*;
    use std::io::{Cursor, ErrorKind, Read};

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
            OdInputSource::FileName("nonexistent_file.txt"),
            OdInputSource::Stream(Box::new(Cursor::new(&b"valid"[..]))),
        ];
        let reader = OdMultifileReader::new(inputs);
        // 第一个文件应该失败，但会自动切换到第二个
        assert!(reader.curr_file.is_some());
        assert!(reader.has_error());

        // 测试混合输入源
        let inputs = vec![
            OdInputSource::Stream(Box::new(Cursor::new(&b"stream"[..]))),
            OdInputSource::FileName("nonexistent_file.txt"),
            OdInputSource::Stream(Box::new(Cursor::new(&b"another"[..]))),
        ];
        let mut reader = OdMultifileReader::new(inputs);
        assert!(reader.curr_file.is_some());
        reader.next_file(); // 尝试打开不存在的文件，会跳到下一个
        assert!(reader.curr_file.is_some());
        assert!(reader.has_error());
    }
}
