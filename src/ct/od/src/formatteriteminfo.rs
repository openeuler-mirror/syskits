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

use std::fmt;

#[allow(clippy::enum_variant_names)]
#[derive(Copy)]
pub enum OdFormatWriter {
    IntWriter(fn(u64) -> String),
    FloatWriter(fn(f64) -> String),
    MultibyteWriter(fn(&[u8]) -> String),
}

impl Clone for OdFormatWriter {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl PartialEq for OdFormatWriter {
    fn eq(&self, other: &Self) -> bool {
        // 导入枚举变体，避免重复使用完整路径
        use crate::formatteriteminfo::OdFormatWriter::*;

        // 使用模式匹配比较两个格式化器
        match (self, other) {
            // 如果都是整数格式化器，比较它们的函数指针
            (IntWriter(a), IntWriter(b)) => std::ptr::fn_addr_eq(*a, *b),
            // 如果都是浮点数格式化器，比较它们的函数指针
            (FloatWriter(a), FloatWriter(b)) => std::ptr::fn_addr_eq(*a, *b),
            // 如果都是多字节格式化器，将函数指针转换为地址后比较
            (MultibyteWriter(a), MultibyteWriter(b)) => *a as usize == *b as usize,
            // 不同类型的格式化器永远不相等
            _ => false,
        }
    }
}

impl Eq for OdFormatWriter {}

impl fmt::Debug for OdFormatWriter {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // 根据格式化器类型生成不同的调试输出
        match *self {
            // 整数格式化器：显示类型名和函数指针地址
            Self::IntWriter(ref p) => {
                f.write_str("IntWriter:")?; // 写入类型标识
                fmt::Pointer::fmt(p, f) // 写入函数指针的十六进制地址
            }
            // 浮点数格式化器：显示类型名和函数指针地址
            Self::FloatWriter(ref p) => {
                f.write_str("FloatWriter:")?; // 写入类型标识
                fmt::Pointer::fmt(p, f) // 写入函数指针的十六进制地址
            }
            // 多字节格式化器：显示类型名和函数指针地址
            Self::MultibyteWriter(ref p) => {
                f.write_str("MultibyteWriter:")?; // 写入类型标识
                fmt::Pointer::fmt(&(*p as *const ()), f) // 将函数指针转换为原始指针后写入地址
            }
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct FormatterItemInfo {
    pub byte_size: usize,
    pub print_width: usize, // including a space in front of the text
    pub formatter: OdFormatWriter,
}

#[cfg(test)]
mod tests {
    use super::*;

    // 用于测试的格式化函数
    fn int_fmt1(n: u64) -> String {
        n.to_string()
    }
    fn int_fmt2(n: u64) -> String {
        format!("0x{:x}", n)
    }
    fn float_fmt1(n: f64) -> String {
        n.to_string()
    }
    fn float_fmt2(n: f64) -> String {
        format!("{:.2}", n)
    }
    fn mb_fmt1(b: &[u8]) -> String {
        String::from_utf8_lossy(b).to_string()
    }
    fn mb_fmt2(b: &[u8]) -> String {
        format!("{:02x}", b[0])
    } // 使用标准库替代 hex

    #[test]
    fn test_format_writer_eq() {
        // 测试相同函数的格式化器相等
        assert_eq!(
            OdFormatWriter::IntWriter(int_fmt1),
            OdFormatWriter::IntWriter(int_fmt1)
        );
        assert_eq!(
            OdFormatWriter::FloatWriter(float_fmt1),
            OdFormatWriter::FloatWriter(float_fmt1)
        );
        assert_eq!(
            OdFormatWriter::MultibyteWriter(mb_fmt1),
            OdFormatWriter::MultibyteWriter(mb_fmt1)
        );

        // 测试不同函数的格式化器不相等
        assert_ne!(
            OdFormatWriter::IntWriter(int_fmt1),
            OdFormatWriter::IntWriter(int_fmt2)
        );
        assert_ne!(
            OdFormatWriter::FloatWriter(float_fmt1),
            OdFormatWriter::FloatWriter(float_fmt2)
        );
        assert_ne!(
            OdFormatWriter::MultibyteWriter(mb_fmt1),
            OdFormatWriter::MultibyteWriter(mb_fmt2)
        );

        // 测试不同类型的格式化器不相等
        assert_ne!(
            OdFormatWriter::IntWriter(int_fmt1),
            OdFormatWriter::FloatWriter(float_fmt1)
        );
        assert_ne!(
            OdFormatWriter::FloatWriter(float_fmt1),
            OdFormatWriter::MultibyteWriter(mb_fmt1)
        );
    }

    #[test]
    fn test_format_writer_debug() {
        // 测试Debug输出格式
        let int_writer = OdFormatWriter::IntWriter(int_fmt1);
        assert!(format!("{:?}", int_writer).starts_with("IntWriter:0x"));

        let float_writer = OdFormatWriter::FloatWriter(float_fmt1);
        assert!(format!("{:?}", float_writer).starts_with("FloatWriter:0x"));

        let mb_writer = OdFormatWriter::MultibyteWriter(mb_fmt1);
        assert!(format!("{:?}", mb_writer).starts_with("MultibyteWriter:0x"));
    }
}
