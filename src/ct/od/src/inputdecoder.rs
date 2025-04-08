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
use half::f16;
use std::io;

use crate::byteorder_io::ByteOrder;
use crate::multifilereader::HasError;
use crate::peekreader::PeekRead;

/// 处理输入并提供以各种格式访问读取数据的功能
///
/// 目前仅适用于实现了 `PeekRead` 的输入
pub struct OdInputDecoder<'a, I>
where
    I: 'a,
{
    /// 用于读取数据的输入源
    input: &'a mut I,

    /// 内存缓冲区，大小在 `new` 中设置
    data: Vec<u8>,
    /// 缓冲区中预留给 `PeekRead` 预读数据的字节数
    reserved_peek_length: usize,

    /// 缓冲区中有效字节的数量
    used_normal_length: usize,
    /// 缓冲区中预读字节的数量
    used_peek_length: usize,

    /// 从缓冲区读取数据时使用的字节序
    byte_order: ByteOrder,
}

impl<I> OdInputDecoder<'_, I> {
    /// 创建一个新的 `InputDecoder`，分配 `normal_length` + `peek_length` 字节的缓冲区。
    /// `byte_order` 决定如何从缓冲区读取多字节格式的数据。
    pub fn new(
        input: &mut I,
        normal_length: usize,
        peek_length: usize,
        byte_order: ByteOrder,
    ) -> OdInputDecoder<I> {
        let bytes = vec![0; normal_length + peek_length];

        OdInputDecoder {
            input,
            data: bytes,
            reserved_peek_length: peek_length,
            used_normal_length: 0,
            used_peek_length: 0,
            byte_order,
        }
    }
}

impl<I> OdInputDecoder<'_, I>
where
    I: PeekRead,
{
    /// 调用内部流的 `peek_read` 来（重新）填充缓冲区。
    /// 返回一个提供访问结果的 `MemoryDecoder` 或返回 I/O 错误。
    pub fn od_peek_read(&mut self) -> io::Result<OdMemoryDecoder> {
        match self
            .input
            .peek_read(self.data.as_mut_slice(), self.reserved_peek_length)
        {
            Ok((n, p)) => {
                self.used_normal_length = n;
                self.used_peek_length = p;
                Ok(OdMemoryDecoder {
                    data: &mut self.data,
                    used_normal_length: self.used_normal_length,
                    used_peek_length: self.used_peek_length,
                    byte_order: self.byte_order,
                })
            }
            Err(e) => Err(e),
        }
    }
}

impl<I> HasError for OdInputDecoder<'_, I>
where
    I: HasError,
{
    /// 调用内部流的 `has_error` 方法
    fn has_error(&self) -> bool {
        self.input.has_error()
    }
}

/// 提供以各种格式访问内部数据的功能
pub struct OdMemoryDecoder<'a> {
    /// 父对象数据的引用
    data: &'a mut Vec<u8>,
    /// 缓冲区中有效字节的数量
    used_normal_length: usize,
    /// 缓冲区中预读字节的数量
    used_peek_length: usize,
    /// 从缓冲区读取数据时使用的字节序
    byte_order: ByteOrder,
}

impl OdMemoryDecoder<'_> {
    /// 将内部缓冲区的指定部分设置为零。
    /// 可以访问整个缓冲区，不仅限于有效数据部分。
    pub fn zero_out_buffer(&mut self, start: usize, end: usize) {
        for i in start..end {
            self.data[i] = 0;
        }
    }

    /// 返回缓冲区的当前长度（即包含的有效数据量）
    pub fn length(&self) -> usize {
        self.used_normal_length
    }

    /// 创建内部缓冲区的克隆。克隆只包含有效数据。
    pub fn clone_buffer(&self, other: &mut Vec<u8>) {
        other.clone_from(self.data);
        other.resize(self.used_normal_length, 0);
    }

    /// 返回从 `start` 开始的内部缓冲区切片
    pub fn get_buffer(&self, start: usize) -> &[u8] {
        &self.data[start..self.used_normal_length]
    }

    /// 返回从 `start` 开始的内部缓冲区切片，包括预读数据
    pub fn get_full_buffer(&self, start: usize) -> &[u8] {
        &self.data[start..self.used_normal_length + self.used_peek_length]
    }

    /// 从内部缓冲区位置 `start` 返回 u8/u16/u32/u64 值
    pub fn read_uint(&self, start: usize, byte_size: usize) -> u64 {
        match byte_size {
            1 => u64::from(self.data[start]),
            2 => u64::from(self.byte_order.read_u16(&self.data[start..start + 2])),
            4 => u64::from(self.byte_order.read_u32(&self.data[start..start + 4])),
            8 => self.byte_order.read_u64(&self.data[start..start + 8]),
            _ => panic!("无效的字节大小: {byte_size}"),
        }
    }

    /// 从内部缓冲区位置 `start` 返回 f32/f64 值
    pub fn read_float(&self, start: usize, byte_size: usize) -> f64 {
        match byte_size {
            2 => f64::from(f16::from_bits(
                self.byte_order.read_u16(&self.data[start..start + 2]),
            )),
            4 => f64::from(self.byte_order.read_f32(&self.data[start..start + 4])),
            8 => self.byte_order.read_f64(&self.data[start..start + 8]),
            _ => panic!("无效的字节大小: {byte_size}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::byteorder_io::ByteOrder;
    use crate::peekreader::PeekReader;
    use std::io::Cursor;

    #[test]
    #[allow(clippy::float_cmp)]
    #[allow(clippy::cognitive_complexity)]
    fn smoke_test() {
        let data = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0xff, 0xff];
        let mut input = PeekReader::new(Cursor::new(&data));
        let mut sut = OdInputDecoder::new(&mut input, 8, 2, ByteOrder::Little);

        // Peek normal length
        let mut mem = sut.od_peek_read().unwrap();

        assert_eq!(8, mem.length());

        assert_eq!(-2.0, mem.read_float(0, 8));
        assert_eq!(-2.0, mem.read_float(4, 4));
        assert_eq!(0xc000000000000000, mem.read_uint(0, 8));
        assert_eq!(0xc0000000, mem.read_uint(4, 4));
        assert_eq!(0xc000, mem.read_uint(6, 2));
        assert_eq!(0xc0, mem.read_uint(7, 1));
        assert_eq!(&[0, 0xc0], mem.get_buffer(6));
        assert_eq!(&[0, 0xc0, 0xff, 0xff], mem.get_full_buffer(6));

        let mut copy: Vec<u8> = Vec::new();
        mem.clone_buffer(&mut copy);
        assert_eq!(vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0], copy);

        mem.zero_out_buffer(7, 8);
        assert_eq!(&[0, 0, 0xff, 0xff], mem.get_full_buffer(6));

        // Peek tail
        let mem = sut.od_peek_read().unwrap();
        assert_eq!(2, mem.length());
        assert_eq!(0xffff, mem.read_uint(0, 2));
    }
}
