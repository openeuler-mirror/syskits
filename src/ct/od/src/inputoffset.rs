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
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum OdRadix {
    Decimal,     // 十进制
    Hexadecimal, // 十六进制
    Octal,       // 八进制
    NoPrefix,    // 无前缀
}

/// 提供在左边距打印的字节偏移量
pub struct OdInputOffset {
    /// 用于打印字节偏移量的进制。NoPrefix 不会打印字节偏移量。
    radix: OdRadix,
    /// 当前位置。在 `new` 中初始化，使用 `increase_position` 增加。
    byte_pos: u64,
    /// 可选的标签，以括号形式打印，通常与 `byte_pos` 不同，
    /// 但在 `byte_pos` 增加时会增加相同的值。
    label: Option<u64>,
}

impl OdInputOffset {
    /// 使用提供的值创建新的 `InputOffset`
    pub fn new(radix: OdRadix, byte_pos: u64, label: Option<u64>) -> Self {
        Self {
            radix,
            byte_pos,
            label,
        }
    }

    /// 增加 `byte_pos`，如果使用了标签，也增加标签值
    pub fn increase_position(&mut self, n: u64) {
        self.byte_pos += n;
        if let Some(l) = self.label {
            self.label = Some(l + n);
        }
    }

    #[cfg(test)]
    fn set_radix(&mut self, radix: OdRadix) {
        self.radix = radix;
    }

    /// 返回当前字节偏移量的字符串表示
    pub fn format_byte_offset(&self) -> String {
        match (self.radix, self.label) {
            (OdRadix::Decimal, None) => format!("{:07}", self.byte_pos),
            (OdRadix::Decimal, Some(l)) => format!("{:07} ({:07})", self.byte_pos, l),
            (OdRadix::Hexadecimal, None) => format!("{:06X}", self.byte_pos),
            (OdRadix::Hexadecimal, Some(l)) => format!("{:06X} ({:06X})", self.byte_pos, l),
            (OdRadix::Octal, None) => format!("{:07o}", self.byte_pos),
            (OdRadix::Octal, Some(l)) => format!("{:07o} ({:07o})", self.byte_pos, l),
            (OdRadix::NoPrefix, None) => String::new(),
            (OdRadix::NoPrefix, Some(l)) => format!("({l:07o})"),
        }
    }

    /// 打印字节偏移量后跟换行符，如果设置了 `Radix::NoPrefix` 且没有使用标签（--traditional），
    /// 则不打印任何内容。
    pub fn print_final_offset(&self) {
        if self.radix != OdRadix::NoPrefix || self.label.is_some() {
            println!("{}", self.format_byte_offset());
        }
    }
}

#[test]
fn test_input_offset() {
    let mut sut = OdInputOffset::new(OdRadix::Hexadecimal, 10, None);
    assert_eq!("00000A", &sut.format_byte_offset());
    sut.increase_position(10);
    assert_eq!("000014", &sut.format_byte_offset());

    // note normally the radix will not change after initialization
    sut.set_radix(OdRadix::Decimal);
    assert_eq!("0000020", &sut.format_byte_offset());

    sut.set_radix(OdRadix::Hexadecimal);
    assert_eq!("000014", &sut.format_byte_offset());

    sut.set_radix(OdRadix::Octal);
    assert_eq!("0000024", &sut.format_byte_offset());

    sut.set_radix(OdRadix::NoPrefix);
    assert_eq!("", &sut.format_byte_offset());

    sut.increase_position(10);
    sut.set_radix(OdRadix::Octal);
    assert_eq!("0000036", &sut.format_byte_offset());
}

#[test]
fn test_input_offset_with_label() {
    let mut sut = OdInputOffset::new(OdRadix::Hexadecimal, 10, Some(20));
    assert_eq!("00000A (000014)", &sut.format_byte_offset());
    sut.increase_position(10);
    assert_eq!("000014 (00001E)", &sut.format_byte_offset());

    // note normally the radix will not change after initialization
    sut.set_radix(OdRadix::Decimal);
    assert_eq!("0000020 (0000030)", &sut.format_byte_offset());

    sut.set_radix(OdRadix::Hexadecimal);
    assert_eq!("000014 (00001E)", &sut.format_byte_offset());

    sut.set_radix(OdRadix::Octal);
    assert_eq!("0000024 (0000036)", &sut.format_byte_offset());

    sut.set_radix(OdRadix::NoPrefix);
    assert_eq!("(0000036)", &sut.format_byte_offset());

    sut.increase_position(10);
    sut.set_radix(OdRadix::Octal);
    assert_eq!("0000036 (0000050)", &sut.format_byte_offset());
}
