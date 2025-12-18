/*
 *    Copyright(c) 2022-2024 China Telecom Cloud Technologies co., Ltd. All rights reserved
 *     syskits is licensed under Mulan PSL v2.
 *    You can use this software according to the terms and conditions of the Mulan PSL V2
 *    You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 *    THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 *    KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 *    NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 *    See the Mulan PSL v2 for more details.
 *
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
