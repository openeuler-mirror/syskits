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

use super::matcher::Matcher;

// 基于特定匹配器的通用搜索器
// 此结构体表示一个搜索上下文，它使用提供的匹配器在给定的字节切片（`haystack`）中查找序列。
// 它跟踪当前的搜索位置。
pub struct Searcher<'a, 'b, M: Matcher> {
    matcher: &'a M,     // 引用用于查找序列的匹配器
    haystack: &'b [u8], // 正在被搜索的字节切片
    position: usize,    // 当前搜索位置
}

// 创建一个新的Searcher实例。
// 此构造函数初始化一个新的Searcher，指定匹配器和要搜索的字节切片。
impl<'a, 'b, M: Matcher> Searcher<'a, 'b, M> {
    pub fn new(matcher: &'a M, haystack: &'b [u8]) -> Self {
        Self {
            matcher,
            haystack,
            position: 0,
        }
    }
}

// 为Searcher实现迭代器特质。
// 此实现使得Searcher可以用作迭代器，遍历`haystack`中分隔符匹配的位置。
// 每次迭代返回匹配序列的第一个和最后一个字节的位置。
impl<'a, 'b, M: Matcher> Iterator for Searcher<'a, 'b, M> {
    type Item = (usize, usize); // 迭代器返回元素的类型

    fn next(&mut self) -> Option<Self::Item> {
        // 尝试从当前位置开始找到下一个匹配项。
        // 如找到匹配项，则更新位置到匹配项最后一个字节之后，
        // 并返回匹配序列首尾字节的位置，然后返回这些位置。
        match self.matcher.next_match(&self.haystack[self.position..]) {
            Some((first, last)) => {
                let result = (first + self.position, last + self.position);
                self.position += last;
                Some(result)
            }
            None => None,
        }
    }
}

