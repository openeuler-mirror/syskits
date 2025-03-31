/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

use std::io::{Cursor, Error, ErrorKind, Read, Result};

#[derive(Clone)]
pub struct OdFailingMockStream {
    /// 错误类型
    kind: ErrorKind,
    /// 错误消息
    message: &'static str,
    /// 重复失败的次数
    repeat_count: i32,
}

impl OdFailingMockStream {
    /// 创建一个新的 FailingMockStream
    ///
    /// 当调用 `read` 或 `write` 时，会连续返回 `repeat_count` 次错误。
    /// 可以通过 `kind` 和 `message` 指定具体的错误类型和消息。
    pub fn new(kind: ErrorKind, message: &'static str, repeat_count: i32) -> Self {
        Self {
            kind,
            message,
            repeat_count,
        }
    }

    /// 生成错误或返回成功
    ///
    /// 如果 repeat_count 为 0，返回 Ok(0)
    /// 否则减少 repeat_count 并返回指定的错误
    fn error(&mut self) -> Result<usize> {
        if self.repeat_count == 0 {
            Ok(0)
        } else {
            if self.repeat_count > 0 {
                self.repeat_count -= 1;
            }
            Err(Error::new(self.kind, self.message))
        }
    }
}

/// 实现 Read trait，使其可以作为读取器使用
impl Read for OdFailingMockStream {
    /// 读取操作，直接返回 error() 的结果
    fn read(&mut self, _: &mut [u8]) -> Result<usize> {
        self.error()
    }
}

#[test]
fn test_failing_mock_stream_read() {
    // 创建一个会失败一次的模拟流
    let mut s =
        OdFailingMockStream::new(ErrorKind::BrokenPipe, "The dog ate the ethernet cable", 1);
    let mut v = [0; 4];

    // 第一次读取应该失败
    let error = s.read(v.as_mut()).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::BrokenPipe);
    assert_eq!(error.to_string(), "The dog ate the ethernet cable");

    // 之后的读取应该返回 Ok(0)
    assert_eq!(s.read(v.as_mut()).unwrap(), 0);
}

#[test]
fn test_failing_mock_stream_chain_interrupted() {
    // 创建一个三部分组成的链式读取器
    let mut c = Cursor::new(&b"abcd"[..])
        .chain(OdFailingMockStream::new(
            ErrorKind::Interrupted,
            "Interrupted",
            5,
        ))
        .chain(Cursor::new(&b"ABCD"[..]));

    // 读取8个字节并验证内容
    let mut v = [0; 8];
    c.read_exact(v.as_mut()).unwrap();
    assert_eq!(v, [0x61, 0x62, 0x63, 0x64, 0x41, 0x42, 0x43, 0x44]);

    // 后续读取应该返回 EOF (Ok(0))
    assert_eq!(c.read(v.as_mut()).unwrap(), 0);
}
