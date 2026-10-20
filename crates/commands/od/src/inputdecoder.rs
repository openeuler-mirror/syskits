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
use std::io::{self, Read};

use crate::byteorder_io::ByteOrder;
use crate::multifilereader::HasError;

/// 处理输入并提供以各种格式访问读取数据的功能
pub struct OdInputDecoder<'a, I>
where
    I: 'a,
{
    /// 用于读取数据的输入源
    input: &'a mut I,

    /// 交替读取的两个固定大小输入块
    data: [Vec<u8>; 2],
    /// 下次读取使用的输入块
    next_index: usize,
    /// 上一次成功读取使用的输入块
    previous_index: Option<usize>,
    /// 两个输入块各自的有效字节数
    used_lengths: [usize; 2],

    /// 从缓冲区读取数据时使用的字节序
    byte_order: ByteOrder,
}

impl<I> OdInputDecoder<'_, I> {
    /// 创建一个新的 `InputDecoder`，分配两个 `normal_length` 字节的输入块。
    /// `byte_order` 决定如何从缓冲区读取多字节格式的数据。
    pub fn new(
        input: &mut I,
        normal_length: usize,
        byte_order: ByteOrder,
    ) -> io::Result<OdInputDecoder<I>> {
        let allocate_block = || -> io::Result<Vec<u8>> {
            let mut bytes = Vec::new();
            bytes
                .try_reserve_exact(normal_length)
                .map_err(|_| io::Error::other("memory exhausted"))?;
            bytes.resize(normal_length, 0);
            Ok(bytes)
        };

        Ok(OdInputDecoder {
            input,
            data: [allocate_block()?, allocate_block()?],
            next_index: 0,
            previous_index: None,
            used_lengths: [0; 2],
            byte_order,
        })
    }
}

impl<I> OdInputDecoder<'_, I>
where
    I: Read,
{
    /// 从内部流读取下一个输入块，并保留上一块供重复检测使用。
    /// 返回一个提供访问结果的 `MemoryDecoder` 或返回 I/O 错误。
    pub fn od_read_block(&mut self) -> io::Result<OdMemoryDecoder> {
        let current_index = self.next_index;
        let length = self.input.read(&mut self.data[current_index])?;
        let has_previous = self.previous_index.is_some();
        let previous_length = self
            .previous_index
            .map_or(0, |index| self.used_lengths[index]);

        self.used_lengths[current_index] = length;
        self.previous_index = Some(current_index);
        self.next_index = 1 - current_index;

        let (data, previous_data): (&mut [u8], Option<&[u8]>) = if current_index == 0 {
            let (current, previous) = self.data.split_at_mut(1);
            (
                current[0].as_mut_slice(),
                has_previous.then_some(&previous[0][..previous_length]),
            )
        } else {
            let (previous, current) = self.data.split_at_mut(1);
            (
                current[0].as_mut_slice(),
                has_previous.then_some(&previous[0][..previous_length]),
            )
        };

        Ok(OdMemoryDecoder {
            data,
            previous_data,
            used_normal_length: length,
            byte_order: self.byte_order,
        })
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

    fn has_input(&self) -> bool {
        self.input.has_input()
    }
}

/// 提供以各种格式访问内部数据的功能
pub struct OdMemoryDecoder<'a> {
    /// 父对象数据的引用
    data: &'a mut [u8],
    /// 上一个完整读取块的引用
    previous_data: Option<&'a [u8]>,
    /// 缓冲区中有效字节的数量
    used_normal_length: usize,
    /// 从缓冲区读取数据时使用的字节序
    byte_order: ByteOrder,
}

impl OdMemoryDecoder<'_> {
    /// 将内部缓冲区的指定部分设置为零。
    /// 可以访问整个缓冲区，不仅限于有效数据部分。
    pub fn zero_out_buffer(&mut self, start: usize, end: usize) {
        self.data[start..end].fill(0);
    }

    /// 返回缓冲区的当前长度（即包含的有效数据量）
    pub fn length(&self) -> usize {
        self.used_normal_length
    }

    /// 返回从 `start` 开始的内部缓冲区切片
    pub fn get_buffer(&self, start: usize) -> &[u8] {
        &self.data[start..self.used_normal_length]
    }

    /// 返回上一个读取块；首次读取时没有上一块。
    pub fn previous_buffer(&self) -> Option<&[u8]> {
        self.previous_data
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

    /// 返回按指定输入字节序转换为本机字节序后的浮点原始字节。
    pub fn read_float_bytes(&self, start: usize, byte_size: usize) -> Vec<u8> {
        let mut bytes = self.data[start..start + byte_size].to_vec();
        let reverse = match self.byte_order {
            ByteOrder::Native => false,
            ByteOrder::Little => cfg!(target_endian = "big"),
            ByteOrder::Big => cfg!(target_endian = "little"),
        };
        if reverse {
            bytes.reverse();
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::byteorder_io::ByteOrder;
    use std::io::Cursor;

    #[test]
    #[allow(clippy::float_cmp)]
    #[allow(clippy::cognitive_complexity)]
    fn smoke_test() {
        let data = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0xff, 0xff];
        let mut input = Cursor::new(&data);
        let mut sut = OdInputDecoder::new(&mut input, 8, ByteOrder::Little).unwrap();

        let mut mem = sut.od_read_block().unwrap();

        assert_eq!(8, mem.length());

        assert_eq!(-2.0, mem.read_float(0, 8));
        assert_eq!(-2.0, mem.read_float(4, 4));
        assert_eq!(0xc000000000000000, mem.read_uint(0, 8));
        assert_eq!(0xc0000000, mem.read_uint(4, 4));
        assert_eq!(0xc000, mem.read_uint(6, 2));
        assert_eq!(0xc0, mem.read_uint(7, 1));
        assert_eq!(&[0, 0xc0], mem.get_buffer(6));
        assert!(mem.previous_buffer().is_none());

        mem.zero_out_buffer(7, 8);
        assert_eq!(&[0, 0], mem.get_buffer(6));

        let mem = sut.od_read_block().unwrap();
        assert_eq!(2, mem.length());
        assert_eq!(0xffff, mem.read_uint(0, 2));
        assert_eq!(mem.previous_buffer(), Some(&[0, 0, 0, 0, 0, 0, 0, 0][..]));
    }
}
