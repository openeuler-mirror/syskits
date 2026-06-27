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

// workaround until https://github.com/BurntSushi/byteorder/issues/41 has been fixed
// based on: https://github.com/netvl/immeta/blob/4460ee/src/utils.rs#L76

use byteorder::ByteOrder as ByteOrderTrait;
use byteorder::{BigEndian, LittleEndian, NativeEndian};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ByteOrder {
    Little,
    Big,
    Native,
}

/// 生成字节序相关的读写操作实现
///
/// 这个宏为 ByteOrder 枚举生成一系列读写方法，支持不同的数据类型。
/// 每个生成的方法都会根据字节序（大端、小端或本机字节序）调用相应的实现。
///
/// # 参数说明
/// 宏接受一系列三元组：($read_name, $write_name -> $tpe)
/// * $read_name: 读取方法的名称
/// * $write_name: 写入方法的名称
/// * $tpe: 数据类型
///
/// # 生成的方法
/// 对每个数据类型会生成两个方法：
/// * read_xxx(&\[u8]) -> type：从字节切片读取指定类型的值
/// * write_xxx(&mut \[u8], type)：将指定类型的值写入字节切片
macro_rules! gen_byte_order_ops {
    ($($read_name:ident, $write_name:ident -> $tpe:ty),+) => {
        impl ByteOrder {
            $(
            #[allow(dead_code)]
            #[inline]
            pub fn $read_name(self, source: &[u8]) -> $tpe {
                match self {
                    ByteOrder::Little => LittleEndian::$read_name(source),
                    ByteOrder::Big => BigEndian::$read_name(source),
                    ByteOrder::Native => NativeEndian::$read_name(source),
                }
            }

            #[allow(dead_code)]
            pub fn $write_name(self, target: &mut [u8], n: $tpe) {
                match self {
                    ByteOrder::Little => LittleEndian::$write_name(target, n),
                    ByteOrder::Big => BigEndian::$write_name(target, n),
                    ByteOrder::Native => NativeEndian::$write_name(target, n),
                }
            }
            )+
        }
    }
}

gen_byte_order_ops! {
    read_u16, write_u16 -> u16,
    read_u32, write_u32 -> u32,
    read_u64, write_u64 -> u64,
    read_i16, write_i16 -> i16,
    read_i32, write_i32 -> i32,
    read_i64, write_i64 -> i64,
    read_f32, write_f32 -> f32,
    read_f64, write_f64 -> f64
}
