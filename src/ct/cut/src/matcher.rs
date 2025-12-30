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

use memchr::memchr;
use memchr::memchr2;

// 定义一个匹配器接口，用于在字节切片中找到下一个匹配的字节序列位置。
// 返回值 (first, last) 表示haystack中的[first..last]范围对应于匹配的模式。
pub trait Matcher {
    fn next_match(&self, haystack: &[u8]) -> Option<(usize, usize)>;
}

// 对精确的字节序列模式进行匹配。
pub struct ExactMatcher<'a> {
    needle: &'a [u8], // 搜索的模式
}

// 构造一个精确匹配器实例。
impl<'a> ExactMatcher<'a> {
    pub fn new(needle: &'a [u8]) -> Self {
        // 确保模式不为空
        assert!(!needle.is_empty());
        Self { needle }
    }
}

// 实现Matcher接口，用于精确匹配。
impl<'a> Matcher for ExactMatcher<'a> {
    fn next_match(&self, haystack: &[u8]) -> Option<(usize, usize)> {
        let mut pos = 0usize;
        loop {
            // 查找haystack中与needle的第一个字节相匹配的位置。
            match memchr(self.needle[0], &haystack[pos..]) {
                Some(match_idx) => {
                    let match_idx = match_idx + pos; // 考虑到搜索是从pos开始的
                                                     // 如果needle长度为1，或者haystack的后续部分以needle的剩余部分开始，则找到匹配。
                    if self.needle.len() == 1
                        || haystack[match_idx + 1..].starts_with(&self.needle[1..])
                    {
                        return Some((match_idx, match_idx + self.needle.len()));
                    } else {
                        // 如果找到的不是完整的needle，则继续在后续位置搜索。
                        pos = match_idx + 1;
                    }
                }
                None => {
                    // 如果无法找到匹配，则返回None。
                    return None;
                }
            }
        }
    }
}

// 匹配任意数量的SPACE或TAB。
pub struct WhitespaceMatcher {}

// 实现Matcher接口，用于匹配任意数量的空格或制表符。
impl Matcher for WhitespaceMatcher {
    fn next_match(&self, haystack: &[u8]) -> Option<(usize, usize)> {
        // 使用memchr2查找haystack中的第一个空格或制表符的位置。
        match memchr2(b' ', b'\t', haystack) {
            Some(match_idx) => {
                // 继续扫描haystack，找到所有连续的空格或制表符。
                let mut skip = match_idx + 1;
                while skip < haystack.len() {
                    match haystack[skip] {
                        b' ' | b'\t' => skip += 1,
                        _ => break,
                    }
                }
                // 返回匹配的起始位置和扫描跳过的结束位置。
                Some((match_idx, skip))
            }
            None => None,
        }
    }
}

