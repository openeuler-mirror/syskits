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
use std::str::from_utf8;

use crate::formatteriteminfo::*;
use half::f16;
use std::f32;
use std::f64;
use std::num::FpCategory;

pub static FORMAT_ITEM_A: FormatterItemInfo = FormatterItemInfo {
    byte_size: 1,
    print_width: 4,
    formatter: OdFormatWriter::IntWriter(format_item_a),
};

pub static FORMAT_ITEM_C: FormatterItemInfo = FormatterItemInfo {
    byte_size: 1,
    print_width: 4,
    formatter: OdFormatWriter::MultibyteWriter(format_item_c),
};

static A_CHARS: [&str; 128] = [
    "nul", "soh", "stx", "etx", "eot", "enq", "ack", "bel", "bs", "ht", "nl", "vt", "ff", "cr",
    "so", "si", "dle", "dc1", "dc2", "dc3", "dc4", "nak", "syn", "etb", "can", "em", "sub", "esc",
    "fs", "gs", "rs", "us", "sp", "!", "\"", "#", "$", "%", "&", "'", "(", ")", "*", "+", ",", "-",
    ".", "/", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", ":", ";", "<", "=", ">", "?", "@",
    "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S",
    "T", "U", "V", "W", "X", "Y", "Z", "[", "\\", "]", "^", "_", "`", "a", "b", "c", "d", "e", "f",
    "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s", "t", "u", "v", "w", "x", "y",
    "z", "{", "|", "}", "~", "del",
];

fn format_item_a(p: u64) -> String {
    // item-bytes == 1
    let b = (p & 0x7f) as u8;
    format!("{:>4}", A_CHARS.get(b as usize).unwrap_or(&"??"))
}

static C_CHARS: [&str; 128] = [
    "\\0", "001", "002", "003", "004", "005", "006", "\\a", "\\b", "\\t", "\\n", "\\v", "\\f",
    "\\r", "016", "017", "020", "021", "022", "023", "024", "025", "026", "027", "030", "031",
    "032", "033", "034", "035", "036", "037", " ", "!", "\"", "#", "$", "%", "&", "'", "(", ")",
    "*", "+", ",", "-", ".", "/", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", ":", ";", "<",
    "=", ">", "?", "@", "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O",
    "P", "Q", "R", "S", "T", "U", "V", "W", "X", "Y", "Z", "[", "\\", "]", "^", "_", "`", "a", "b",
    "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s", "t", "u",
    "v", "w", "x", "y", "z", "{", "|", "}", "~", "177",
];

fn format_item_c(bytes: &[u8]) -> String {
    // item-bytes == 1
    let b = bytes[0];

    if b & 0x80 == 0x00 {
        match C_CHARS.get(b as usize) {
            Some(s) => format!("{s:>4}"),
            None => format!("{b:>4}"),
        }
    } else if (b & 0xc0) == 0x80 {
        // second or subsequent octet of an utf-8 sequence
        String::from("  **")
    } else if ((b & 0xe0) == 0xc0) && (bytes.len() >= 2) {
        // start of a 2 octet utf-8 sequence
        match from_utf8(&bytes[0..2]) {
            Ok(s) => format!("{s:>4}"),
            Err(_) => format!(" {b:03o}"),
        }
    } else if ((b & 0xf0) == 0xe0) && (bytes.len() >= 3) {
        // start of a 3 octet utf-8 sequence
        match from_utf8(&bytes[0..3]) {
            Ok(s) => format!("{s:>4}"),
            Err(_) => format!(" {b:03o}"),
        }
    } else if ((b & 0xf8) == 0xf0) && (bytes.len() >= 4) {
        // start of a 4 octet utf-8 sequence
        match from_utf8(&bytes[0..4]) {
            Ok(s) => format!("{s:>4}"),
            Err(_) => format!(" {b:03o}"),
        }
    } else {
        // invalid utf-8
        format!(" {b:03o}")
    }
}

pub fn format_ascii_dump(bytes: &[u8]) -> String {
    let mut result = String::new();

    result.push('>');
    for c in bytes {
        if *c >= 0x20 && *c <= 0x7e {
            result.push_str(C_CHARS[*c as usize]);
        } else {
            result.push('.');
        }
    }
    result.push('<');

    result
}

pub static FORMAT_ITEM_F16: FormatterItemInfo = FormatterItemInfo {
    byte_size: 2,
    print_width: 10,
    formatter: OdFormatWriter::FloatWriter(format_item_flo16),
};

pub static FORMAT_ITEM_F32: FormatterItemInfo = FormatterItemInfo {
    byte_size: 4,
    print_width: 15,
    formatter: OdFormatWriter::FloatWriter(format_item_flo32),
};

pub static FORMAT_ITEM_F64: FormatterItemInfo = FormatterItemInfo {
    byte_size: 8,
    print_width: 25,
    formatter: OdFormatWriter::FloatWriter(format_item_flo64),
};

pub fn format_item_flo16(f: f64) -> String {
    format!(" {}", format_flo16(f16::from_f64(f)))
}

pub fn format_item_flo32(f: f64) -> String {
    format!(" {}", format_flo32(f as f32))
}

pub fn format_item_flo64(f: f64) -> String {
    format!(" {}", format_flo64(f))
}

fn format_flo16(f: f16) -> String {
    format_float(f64::from(f), 9, 4)
}

// formats float with 8 significant digits, eg 12345678 or -1.2345678e+12
// always returns a string of 14 characters
fn format_flo32(f: f32) -> String {
    let width: usize = 14;
    let precision: usize = 8;

    if f.classify() == FpCategory::Subnormal {
        // subnormal numbers will be normal as f64, so will print with a wrong precision
        format!("{f:width$e}") // subnormal numbers
    } else {
        format_float(f64::from(f), width, precision)
    }
}

fn format_flo64(f: f64) -> String {
    format_float(f, 24, 17)
}

fn format_float(f: f64, width: usize, precision: usize) -> String {
    if !f.is_normal() {
        if f == -0.0 && f.is_sign_negative() {
            return format!("{:>width$}", "-0");
        }
        if f == 0.0 || !f.is_finite() {
            return format!("{f:width$}");
        }
        return format!("{f:width$e}"); // subnormal numbers
    }

    let mut l = f.abs().log10().floor() as i32;

    let r = 10f64.powi(l);
    if (f > 0.0 && r > f) || (f < 0.0 && -r < f) {
        // fix precision error
        l -= 1;
    }

    if l >= 0 && l <= (precision as i32 - 1) {
        format!("{:width$.dec$}", f, dec = (precision - 1) - l as usize)
    } else if l == -1 {
        format!("{f:width$.precision$}")
    } else {
        format!("{:width$.dec$e}", f, dec = precision - 1)
    }
}

// ===== 格式化宏定义 =====

/// 八进制格式化字符串，用于格式化无符号整数
macro_rules! OCT {
    () => {
        " {:0width$o}" // 使用前导零填充，宽度由width参数指定
    };
}

/// 十六进制格式化字符串，用于格式化无符号整数
macro_rules! HEX {
    () => {
        " {:0width$x}" // 使用前导零填充，小写十六进制
    };
}

/// 十进制格式化字符串，用于格式化有符号和无符号整数
macro_rules! DEC {
    () => {
        " {:width$}" // 使用空格填充到指定宽度
    };
}

/// 定义无符号整数的格式化器
///
/// # 参数
/// * `$NAME` - 格式化器的静态变量名
/// * `$byte_size` - 整数类型的字节大小
/// * `$print_width` - 输出的最大宽度
/// * `$function` - 格式化函数名
/// * `$format_str` - 使用的格式化字符串（OCT、HEX 或 DEC）
macro_rules! int_writer_unsigned {
    ($NAME:ident, $byte_size:expr, $print_width:expr, $function:ident, $format_str:expr) => {
        // 定义格式化函数
        fn $function(p: u64) -> String {
            format!($format_str, p, width = $print_width - 1)
        }

        // 创建静态格式化器实例
        pub static $NAME: FormatterItemInfo = FormatterItemInfo {
            byte_size: $byte_size,
            print_width: $print_width,
            formatter: OdFormatWriter::IntWriter($function),
        };
    };
}

/// 定义有符号整数的格式化器
///
/// # 参数
/// * `$NAME` - 格式化器的静态变量名
/// * `$byte_size` - 整数类型的字节大小
/// * `$print_width` - 输出的最大宽度
/// * `$function` - 格式化函数名
/// * `$format_str` - 使用的格式化字符串（通常是 DEC）
macro_rules! int_writer_signed {
    ($NAME:ident, $byte_size:expr, $print_width:expr, $function:ident, $format_str:expr) => {
        // 定义格式化函数
        fn $function(p: u64) -> String {
            let s = sign_extend(p, $byte_size);
            format!($format_str, s, width = $print_width - 1)
        }

        // 创建静态格式化器实例
        pub static $NAME: FormatterItemInfo = FormatterItemInfo {
            byte_size: $byte_size,
            print_width: $print_width,
            formatter: OdFormatWriter::IntWriter($function),
        };
    };
}

// ===== 整数格式化实现 =====

/// 将无符号数值扩展为有符号数
///
/// # 参数
/// * `item` - 要扩展的无符号数值
/// * `item_bytes` - 原始数值的字节大小
///
/// # 返回值
/// 扩展后的64位有符号整数
fn sign_extend(item: u64, item_bytes: usize) -> i64 {
    let shift = 64 - item_bytes * 8;
    (item << shift) as i64 >> shift
}

// 八进制格式化器定义
int_writer_unsigned!(FORMAT_ITEM_OCT8, 1, 4, format_item_oct8, OCT!()); // 最大值: 377
int_writer_unsigned!(FORMAT_ITEM_OCT16, 2, 7, format_item_oct16, OCT!()); // 最大值: 177777
int_writer_unsigned!(FORMAT_ITEM_OCT32, 4, 12, format_item_oct32, OCT!()); // 最大值: 37777777777
int_writer_unsigned!(FORMAT_ITEM_OCT64, 8, 23, format_item_oct64, OCT!()); // 最大值: 1777777777777777777777

// 十六进制格式化器定义
int_writer_unsigned!(FORMAT_ITEM_HEX8, 1, 3, format_item_hex8, HEX!()); // 最大值: ff
int_writer_unsigned!(FORMAT_ITEM_HEX16, 2, 5, format_item_hex16, HEX!()); // 最大值: ffff
int_writer_unsigned!(FORMAT_ITEM_HEX32, 4, 9, format_item_hex32, HEX!()); // 最大值: ffffffff
int_writer_unsigned!(FORMAT_ITEM_HEX64, 8, 17, format_item_hex64, HEX!()); // 最大值: ffffffffffffffff

// 无符号十进制格式化器定义
int_writer_unsigned!(FORMAT_ITEM_DEC8U, 1, 4, format_item_dec_u8, DEC!()); // 最大值: 255
int_writer_unsigned!(FORMAT_ITEM_DEC16U, 2, 6, format_item_dec_u16, DEC!()); // 最大值: 65535
int_writer_unsigned!(FORMAT_ITEM_DEC32U, 4, 11, format_item_dec_u32, DEC!()); // 最大值: 4294967295
int_writer_unsigned!(FORMAT_ITEM_DEC64U, 8, 21, format_item_dec_u64, DEC!()); // 最大值: 18446744073709551615

// 有符号十进制格式化器定义
int_writer_signed!(FORMAT_ITEM_DEC8S, 1, 5, format_item_dec_s8, DEC!()); // 最小值: -128
int_writer_signed!(FORMAT_ITEM_DEC16S, 2, 7, format_item_dec_s16, DEC!()); // 最小值: -32768
int_writer_signed!(FORMAT_ITEM_DEC32S, 4, 12, format_item_dec_s32, DEC!()); // 最小值: -2147483648
int_writer_signed!(FORMAT_ITEM_DEC64S, 8, 21, format_item_dec_s64, DEC!()); // 最小值: -9223372036854775808

#[test]
#[allow(clippy::cognitive_complexity)]
fn test_format_item_a() {
    assert_eq!(" nul", format_item_a(0x00));
    assert_eq!(" soh", format_item_a(0x01));
    assert_eq!("  sp", format_item_a(0x20));
    assert_eq!("   A", format_item_a(0x41));
    assert_eq!("   ~", format_item_a(0x7e));
    assert_eq!(" del", format_item_a(0x7f));

    assert_eq!(" nul", format_item_a(0x80));
    assert_eq!("   A", format_item_a(0xc1));
    assert_eq!("   ~", format_item_a(0xfe));
    assert_eq!(" del", format_item_a(0xff));
}

#[test]
#[allow(clippy::cognitive_complexity)]
fn test_format_item_c() {
    assert_eq!("  \\0", format_item_c(&[0x00]));
    assert_eq!(" 001", format_item_c(&[0x01]));
    assert_eq!("    ", format_item_c(&[0x20]));
    assert_eq!("   A", format_item_c(&[0x41]));
    assert_eq!("   ~", format_item_c(&[0x7e]));
    assert_eq!(" 177", format_item_c(&[0x7f]));
    assert_eq!("   A", format_item_c(&[0x41, 0x21]));

    assert_eq!("  **", format_item_c(&[0x80]));
    assert_eq!("  **", format_item_c(&[0x9f]));

    assert_eq!("   ß", format_item_c(&[0xc3, 0x9f]));
    assert_eq!("   ß", format_item_c(&[0xc3, 0x9f, 0x21]));

    assert_eq!("   \u{1000}", format_item_c(&[0xe1, 0x80, 0x80]));
    assert_eq!("   \u{1000}", format_item_c(&[0xe1, 0x80, 0x80, 0x21]));

    assert_eq!("   \u{1f496}", format_item_c(&[0xf0, 0x9f, 0x92, 0x96]));
    assert_eq!(
        "   \u{1f496}",
        format_item_c(&[0xf0, 0x9f, 0x92, 0x96, 0x21])
    );

    assert_eq!(" 300", format_item_c(&[0xc0, 0x80])); // invalid utf-8 (UTF-8 null)
    assert_eq!(" 301", format_item_c(&[0xc1, 0xa1])); // invalid utf-8
    assert_eq!(" 303", format_item_c(&[0xc3, 0xc3])); // invalid utf-8
    assert_eq!(" 360", format_item_c(&[0xf0, 0x82, 0x82, 0xac])); // invalid utf-8 (overlong)
    assert_eq!(" 360", format_item_c(&[0xf0, 0x9f, 0x92])); // invalid utf-8 (missing octet)
    assert_eq!("   \u{10FFFD}", format_item_c(&[0xf4, 0x8f, 0xbf, 0xbd])); // largest valid utf-8   // spell-checker:ignore 10FFFD FFFD
    assert_eq!(" 364", format_item_c(&[0xf4, 0x90, 0x00, 0x00])); // invalid utf-8
    assert_eq!(" 365", format_item_c(&[0xf5, 0x80, 0x80, 0x80])); // invalid utf-8
    assert_eq!(" 377", format_item_c(&[0xff])); // invalid utf-8
}

#[test]
fn test_format_ascii_dump() {
    assert_eq!(">.<", format_ascii_dump(&[0x00]));
    assert_eq!(
        ">. A~.<",
        format_ascii_dump(&[0x1f, 0x20, 0x41, 0x7e, 0x7f])
    );
}

#[test]
#[allow(clippy::excessive_precision)]
#[allow(clippy::cognitive_complexity)]
fn test_format_flo32() {
    assert_eq!(format_flo32(1.0), "     1.0000000");
    assert_eq!(format_flo32(9.9999990), "     9.9999990");
    assert_eq!(format_flo32(10.0), "     10.000000");
    assert_eq!(format_flo32(99.999977), "     99.999977");
    assert_eq!(format_flo32(99.999992), "     99.999992");
    assert_eq!(format_flo32(100.0), "     100.00000");
    assert_eq!(format_flo32(999.99994), "     999.99994");
    assert_eq!(format_flo32(1000.0), "     1000.0000");
    assert_eq!(format_flo32(9999.9990), "     9999.9990");
    assert_eq!(format_flo32(10000.0), "     10000.000");
    assert_eq!(format_flo32(99999.992), "     99999.992");
    assert_eq!(format_flo32(100000.0), "     100000.00");
    assert_eq!(format_flo32(999999.94), "     999999.94");
    assert_eq!(format_flo32(1000000.0), "     1000000.0");
    assert_eq!(format_flo32(9999999.0), "     9999999.0");
    assert_eq!(format_flo32(10000000.0), "      10000000");
    assert_eq!(format_flo32(99999992.0), "      99999992");
    assert_eq!(format_flo32(100000000.0), "   1.0000000e8");
    assert_eq!(format_flo32(9.9999994e8), "   9.9999994e8");
    assert_eq!(format_flo32(1.0e9), "   1.0000000e9");
    assert_eq!(format_flo32(9.9999990e9), "   9.9999990e9");
    assert_eq!(format_flo32(1.0e10), "  1.0000000e10");

    assert_eq!(format_flo32(0.1), "    0.10000000");
    assert_eq!(format_flo32(0.99999994), "    0.99999994");
    assert_eq!(format_flo32(0.010000001), "  1.0000001e-2");
    assert_eq!(format_flo32(0.099999994), "  9.9999994e-2");
    assert_eq!(format_flo32(0.001), "  1.0000000e-3");
    assert_eq!(format_flo32(0.0099999998), "  9.9999998e-3");

    assert_eq!(format_flo32(-1.0), "    -1.0000000");
    assert_eq!(format_flo32(-9.9999990), "    -9.9999990");
    assert_eq!(format_flo32(-10.0), "    -10.000000");
    assert_eq!(format_flo32(-99.999977), "    -99.999977");
    assert_eq!(format_flo32(-99.999992), "    -99.999992");
    assert_eq!(format_flo32(-100.0), "    -100.00000");
    assert_eq!(format_flo32(-999.99994), "    -999.99994");
    assert_eq!(format_flo32(-1000.0), "    -1000.0000");
    assert_eq!(format_flo32(-9999.9990), "    -9999.9990");
    assert_eq!(format_flo32(-10000.0), "    -10000.000");
    assert_eq!(format_flo32(-99999.992), "    -99999.992");
    assert_eq!(format_flo32(-100000.0), "    -100000.00");
    assert_eq!(format_flo32(-999999.94), "    -999999.94");
    assert_eq!(format_flo32(-1000000.0), "    -1000000.0");
    assert_eq!(format_flo32(-9999999.0), "    -9999999.0");
    assert_eq!(format_flo32(-10000000.0), "     -10000000");
    assert_eq!(format_flo32(-99999992.0), "     -99999992");
    assert_eq!(format_flo32(-100000000.0), "  -1.0000000e8");
    assert_eq!(format_flo32(-9.9999994e8), "  -9.9999994e8");
    assert_eq!(format_flo32(-1.0e9), "  -1.0000000e9");
    assert_eq!(format_flo32(-9.9999990e9), "  -9.9999990e9");
    assert_eq!(format_flo32(-1.0e10), " -1.0000000e10");

    assert_eq!(format_flo32(-0.1), "   -0.10000000");
    assert_eq!(format_flo32(-0.99999994), "   -0.99999994");
    assert_eq!(format_flo32(-0.010000001), " -1.0000001e-2");
    assert_eq!(format_flo32(-0.099999994), " -9.9999994e-2");
    assert_eq!(format_flo32(-0.001), " -1.0000000e-3");
    assert_eq!(format_flo32(-0.0099999998), " -9.9999998e-3");

    assert_eq!(format_flo32(3.4028233e38), "  3.4028233e38");
    assert_eq!(format_flo32(-3.4028233e38), " -3.4028233e38");
    assert_eq!(format_flo32(-1.1663108e-38), "-1.1663108e-38");
    assert_eq!(format_flo32(-4.7019771e-38), "-4.7019771e-38");
    assert_eq!(format_flo32(1e-45), "         1e-45");

    assert_eq!(format_flo32(-3.402823466e+38), " -3.4028235e38");
    assert_eq!(format_flo32(f32::NAN), "           NaN");
    assert_eq!(format_flo32(f32::INFINITY), "           inf");
    assert_eq!(format_flo32(f32::NEG_INFINITY), "          -inf");
    assert_eq!(format_flo32(-0.0), "            -0");
    assert_eq!(format_flo32(0.0), "             0");
}

#[test]
#[allow(clippy::cognitive_complexity)]
fn test_format_flo64() {
    assert_eq!(format_flo64(1.0), "      1.0000000000000000");
    assert_eq!(format_flo64(10.0), "      10.000000000000000");
    assert_eq!(format_flo64(1000000000000000.0), "      1000000000000000.0");
    assert_eq!(
        format_flo64(10000000000000000.0),
        "       10000000000000000"
    );
    assert_eq!(
        format_flo64(100000000000000000.0),
        "   1.0000000000000000e17"
    );

    assert_eq!(format_flo64(-0.1), "    -0.10000000000000001");
    assert_eq!(format_flo64(-0.01), "  -1.0000000000000000e-2");

    assert_eq!(
        format_flo64(-2.2250738585072014e-308),
        "-2.2250738585072014e-308"
    );
    assert_eq!(format_flo64(4e-320), "                  4e-320");
    assert_eq!(format_flo64(f64::NAN), "                     NaN");
    assert_eq!(format_flo64(f64::INFINITY), "                     inf");
    assert_eq!(format_flo64(f64::NEG_INFINITY), "                    -inf");
    assert_eq!(format_flo64(-0.0), "                      -0");
    assert_eq!(format_flo64(0.0), "                       0");
}

#[test]
#[allow(clippy::cognitive_complexity)]
fn test_format_flo16() {
    assert_eq!(format_flo16(f16::from_bits(0x8400u16)), "-6.104e-5");
    assert_eq!(format_flo16(f16::from_bits(0x8401u16)), "-6.109e-5");
    assert_eq!(format_flo16(f16::from_bits(0x8402u16)), "-6.115e-5");
    assert_eq!(format_flo16(f16::from_bits(0x8403u16)), "-6.121e-5");

    assert_eq!(format_flo16(f16::from_f32(1.0)), "    1.000");
    assert_eq!(format_flo16(f16::from_f32(10.0)), "    10.00");
    assert_eq!(format_flo16(f16::from_f32(100.0)), "    100.0");
    assert_eq!(format_flo16(f16::from_f32(1000.0)), "     1000");
    assert_eq!(format_flo16(f16::from_f32(10000.0)), "  1.000e4");

    assert_eq!(format_flo16(f16::from_f32(-0.2)), "  -0.2000");
    assert_eq!(format_flo16(f16::from_f32(-0.02)), "-2.000e-2");

    assert_eq!(format_flo16(f16::MIN_POSITIVE_SUBNORMAL), " 5.960e-8");
    assert_eq!(format_flo16(f16::MIN), " -6.550e4");
    assert_eq!(format_flo16(f16::NAN), "      NaN");
    assert_eq!(format_flo16(f16::INFINITY), "      inf");
    assert_eq!(format_flo16(f16::NEG_INFINITY), "     -inf");
    assert_eq!(format_flo16(f16::NEG_ZERO), "       -0");
    assert_eq!(format_flo16(f16::ZERO), "        0");
}

#[test]
#[allow(clippy::cognitive_complexity)]
fn test_sign_extend() {
    assert_eq!(
        0xffff_ffff_ffff_ff80u64 as i64,
        sign_extend(0x0000_0000_0000_0080, 1)
    );
    assert_eq!(
        0xffff_ffff_ffff_8000u64 as i64,
        sign_extend(0x0000_0000_0000_8000, 2)
    );
    assert_eq!(
        0xffff_ffff_ff80_0000u64 as i64,
        sign_extend(0x0000_0000_0080_0000, 3)
    );
    assert_eq!(
        0xffff_ffff_8000_0000u64 as i64,
        sign_extend(0x0000_0000_8000_0000, 4)
    );
    assert_eq!(
        0xffff_ff80_0000_0000u64 as i64,
        sign_extend(0x0000_0080_0000_0000, 5)
    );
    assert_eq!(
        0xffff_8000_0000_0000u64 as i64,
        sign_extend(0x0000_8000_0000_0000, 6)
    );
    assert_eq!(
        0xff80_0000_0000_0000u64 as i64,
        sign_extend(0x0080_0000_0000_0000, 7)
    );
    assert_eq!(
        0x8000_0000_0000_0000u64 as i64,
        sign_extend(0x8000_0000_0000_0000, 8)
    );

    assert_eq!(0x0000_0000_0000_007f, sign_extend(0x0000_0000_0000_007f, 1));
    assert_eq!(0x0000_0000_0000_7fff, sign_extend(0x0000_0000_0000_7fff, 2));
    assert_eq!(0x0000_0000_007f_ffff, sign_extend(0x0000_0000_007f_ffff, 3));
    assert_eq!(0x0000_0000_7fff_ffff, sign_extend(0x0000_0000_7fff_ffff, 4));
    assert_eq!(0x0000_007f_ffff_ffff, sign_extend(0x0000_007f_ffff_ffff, 5));
    assert_eq!(0x0000_7fff_ffff_ffff, sign_extend(0x0000_7fff_ffff_ffff, 6));
    assert_eq!(0x007f_ffff_ffff_ffff, sign_extend(0x007f_ffff_ffff_ffff, 7));
    assert_eq!(0x7fff_ffff_ffff_ffff, sign_extend(0x7fff_ffff_ffff_ffff, 8));
}

#[test]
fn test_format_item_oct() {
    assert_eq!(" 000", format_item_oct8(0));
    assert_eq!(" 377", format_item_oct8(0xff));
    assert_eq!(" 000000", format_item_oct16(0));
    assert_eq!(" 177777", format_item_oct16(0xffff));
    assert_eq!(" 00000000000", format_item_oct32(0));
    assert_eq!(" 37777777777", format_item_oct32(0xffff_ffff));
    assert_eq!(" 0000000000000000000000", format_item_oct64(0));
    assert_eq!(
        " 1777777777777777777777",
        format_item_oct64(0xffff_ffff_ffff_ffff)
    );
}

#[test]
fn test_format_item_hex() {
    assert_eq!(" 00", format_item_hex8(0));
    assert_eq!(" ff", format_item_hex8(0xff));
    assert_eq!(" 0000", format_item_hex16(0));
    assert_eq!(" ffff", format_item_hex16(0xffff));
    assert_eq!(" 00000000", format_item_hex32(0));
    assert_eq!(" ffffffff", format_item_hex32(0xffff_ffff));
    assert_eq!(" 0000000000000000", format_item_hex64(0));
    assert_eq!(
        " ffffffffffffffff",
        format_item_hex64(0xffff_ffff_ffff_ffff)
    );
}

#[test]
fn test_format_item_dec_u() {
    assert_eq!("   0", format_item_dec_u8(0));
    assert_eq!(" 255", format_item_dec_u8(0xff));
    assert_eq!("     0", format_item_dec_u16(0));
    assert_eq!(" 65535", format_item_dec_u16(0xffff));
    assert_eq!("          0", format_item_dec_u32(0));
    assert_eq!(" 4294967295", format_item_dec_u32(0xffff_ffff));
    assert_eq!("                    0", format_item_dec_u64(0));
    assert_eq!(
        " 18446744073709551615",
        format_item_dec_u64(0xffff_ffff_ffff_ffff)
    );
}

#[test]
#[allow(clippy::cognitive_complexity)]
fn test_format_item_dec_s() {
    assert_eq!("    0", format_item_dec_s8(0));
    assert_eq!("  127", format_item_dec_s8(0x7f));
    assert_eq!(" -128", format_item_dec_s8(0x80));
    assert_eq!("      0", format_item_dec_s16(0));
    assert_eq!("  32767", format_item_dec_s16(0x7fff));
    assert_eq!(" -32768", format_item_dec_s16(0x8000));
    assert_eq!("           0", format_item_dec_s32(0));
    assert_eq!("  2147483647", format_item_dec_s32(0x7fff_ffff));
    assert_eq!(" -2147483648", format_item_dec_s32(0x8000_0000));
    assert_eq!("                    0", format_item_dec_s64(0));
    assert_eq!(
        "  9223372036854775807",
        format_item_dec_s64(0x7fff_ffff_ffff_ffff)
    );
    assert_eq!(
        " -9223372036854775808",
        format_item_dec_s64(0x8000_0000_0000_0000)
    );
}
