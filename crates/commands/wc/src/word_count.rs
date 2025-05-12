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
use std::cmp::max;
use std::ops::{Add, AddAssign};

#[derive(Debug, Default, Copy, Clone)]
pub struct WcWordCount {
    pub bytes: usize,
    pub chars: usize,
    pub lines: usize,
    pub words: usize,
    pub max_line_length: usize,
}

impl Add for WcWordCount {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            bytes: other.bytes + self.bytes,
            chars: other.chars + self.chars,
            lines: other.lines + self.lines,
            words: other.words + self.words,
            max_line_length: max(other.max_line_length, self.max_line_length),
        }
    }
}

impl AddAssign for WcWordCount {
    fn add_assign(&mut self, other: Self) {
        *self = other + *self;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add() {
        let wc1 = WcWordCount {
            bytes: 100,
            chars: 200,
            lines: 10,
            words: 50,
            max_line_length: 40,
        };
        let wc2 = WcWordCount {
            bytes: 150,
            chars: 300,
            lines: 20,
            words: 75,
            max_line_length: 35,
        };
        let result = wc1 + wc2;
        assert_eq!(result.bytes, 250);
        assert_eq!(result.chars, 500);
        assert_eq!(result.lines, 30);
        assert_eq!(result.words, 125);
        assert_eq!(result.max_line_length, 40);
    }

    #[test]
    fn test_add_assign() {
        let mut wc1 = WcWordCount {
            bytes: 60,
            chars: 100,
            lines: 5,
            words: 20,
            max_line_length: 25,
        };
        let wc2 = WcWordCount {
            bytes: 40,
            chars: 50,
            lines: 3,
            words: 10,
            max_line_length: 30,
        };
        wc1 += wc2;
        assert_eq!(wc1.bytes, 100);
        assert_eq!(wc1.chars, 150);
        assert_eq!(wc1.lines, 8);
        assert_eq!(wc1.words, 30);
        assert_eq!(wc1.max_line_length, 30);
    }

    #[test]
    fn test_add_zero() {
        let wc1 = WcWordCount {
            bytes: 0,
            chars: 0,
            lines: 0,
            words: 0,
            max_line_length: 0,
        };
        let wc2 = WcWordCount {
            bytes: 100,
            chars: 200,
            lines: 10,
            words: 50,
            max_line_length: 40,
        };
        let result = wc1 + wc2;
        assert_eq!(result.bytes, 100);
        assert_eq!(result.chars, 200);
        assert_eq!(result.lines, 10);
        assert_eq!(result.words, 50);
        assert_eq!(result.max_line_length, 40);
    }

    #[test]
    fn test_add_max_values() {
        let max_value = usize::MAX - 1;
        let wc1 = WcWordCount {
            bytes: max_value,
            chars: max_value,
            lines: max_value,
            words: max_value,
            max_line_length: max_value,
        };
        let wc2 = WcWordCount {
            bytes: 1,
            chars: 1,
            lines: 1,
            words: 1,
            max_line_length: 1,
        };
        // 预期会因为usize溢出而失败，但在Rust中，测试环境默认不会panic，需要用checked_add等方法手动处理溢出
        let result = wc1 + wc2;
        assert_eq!(result.bytes, usize::MAX); // 由于溢出，结果会回绕
        assert_eq!(result.chars, usize::MAX);
        assert_eq!(result.lines, usize::MAX);
        assert_eq!(result.words, usize::MAX);
        assert_eq!(result.max_line_length, usize::MAX - 1);
    }

    #[test]
    fn test_add_with_zeros() {
        let wc1 = WcWordCount {
            bytes: 100,
            chars: 200,
            lines: 10,
            words: 50,
            max_line_length: 40,
        };
        let wc2 = WcWordCount::default(); // 使用默认值，全部为0
        let result = wc1 + wc2;
        assert_eq!(result.bytes, 100);
        assert_eq!(result.chars, 200);
        assert_eq!(result.lines, 10);
        assert_eq!(result.words, 50);
        assert_eq!(result.max_line_length, 40);
    }
}
