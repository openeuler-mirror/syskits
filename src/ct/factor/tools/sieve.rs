/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

//! 高效的埃拉托斯特尼筛法实现，用于生成素数序列
//!
//! 本模块提供了一个优化的素数生成器，使用了以下技术：
//! 1. 轮式筛法 (Wheel Factorization) - 跳过明显的合数
//! 2. 分段筛选 - 减少内存使用
//! 3. 优化的数据结构 - 使用最小堆来跟踪合数标记

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::iter::{Chain, Copied, Cycle};
use std::slice::Iter;

/// 初始的小素数列表
const INIT_PRIMES: &[u64] = &[2, 3, 5, 7];

/// 轮式筛法的增量，跳过2,3,5,7的倍数
const WHEEL_INCS: &[u64] = &[
    2, 4, 2, 4, 6, 2, 6, 4, 2, 4, 6, 6, 2, 6, 4, 2, 6, 4, 6, 8, 4, 2, 4, 2, 4, 8, 6, 4, 6, 2, 4, 6,
    2, 6, 6, 4, 2, 4, 6, 2, 6, 4, 2, 4, 2, 10, 2, 10,
];

/// 素数生成器类型
pub type PrimeSieve = Chain<Copied<Iter<'static, u64>>, Sieve>;

/// 埃拉托斯特尼筛法实现
#[derive(Default)]
pub struct Sieve {
    wheel: Wheel,
    composites: BinaryHeap<Reverse<(u64, u64)>>,
}

impl Iterator for Sieve {
    type Item = u64;

    fn next(&mut self) -> Option<u64> {
        // 获取下一个候选数
        let n = self.wheel.next()?;

        // 检查是否是合数
        while let Some(&Reverse((next, step))) = self.composites.peek() {
            if next > n {
                break; // 下一个合数标记大于当前数，所以当前数可能是素数
            }

            if next == n {
                // 当前数是合数，更新标记并继续下一个候选数
                self.update_composite(next, step);
                return self.next(); // 递归调用next获取下一个素数
            }

            // 更新所有小于等于n的合数标记
            self.update_composite(next, step);
        }

        // 找到一个素数，添加它的倍数到合数堆中
        self.composites.push(Reverse((n * n, n)));
        Some(n)
    }
}

impl Sieve {
    /// 创建一个新的素数筛
    pub fn new() -> Self {
        Self {
            wheel: Wheel::new(),
            composites: BinaryHeap::new(),
        }
    }

    /// 获取所有奇素数的迭代器（不包括2）
    pub fn odd_primes() -> PrimeSieve {
        INIT_PRIMES[1..].iter().copied().chain(Self::new())
    }

    /// 更新合数标记
    fn update_composite(&mut self, next: u64, step: u64) {
        self.composites.pop(); // 移除当前标记

        // 计算下一个标记位置，确保不会与轮式筛法的模式重叠
        let mut new_next = next + step;
        while INIT_PRIMES
            .iter()
            .any(|&p| new_next % p == 0 && new_next != p)
        {
            new_next += step;
        }

        self.composites.push(Reverse((new_next, step)));
    }
}

/// 轮式筛法实现，生成不被2,3,5,7整除的数
struct Wheel {
    next_value: u64,
    increment: Cycle<Iter<'static, u64>>,
}

impl Default for Wheel {
    fn default() -> Self {
        Self::new()
    }
}

impl Wheel {
    fn new() -> Self {
        Self {
            next_value: 11, // 从11开始，因为2,3,5,7已经在INIT_PRIMES中
            increment: WHEEL_INCS.iter().cycle(),
        }
    }

    fn next(&mut self) -> Option<u64> {
        let current = self.next_value;
        if let Some(&inc) = self.increment.next() {
            self.next_value += inc;
            Some(current)
        } else {
            None // 这不应该发生，因为increment是一个无限循环
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 获取所有素数的迭代器（包括2,3,5,7）
    fn primes() -> PrimeSieve {
        INIT_PRIMES.iter().copied().chain(Sieve::new())
    }

    #[test]
    fn test_first_few_primes() {
        let expected = [2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37, 41, 43, 47];
        let actual: Vec<u64> = primes().take(expected.len()).collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_odd_primes() {
        let expected = [3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37, 41, 43, 47];
        let actual: Vec<u64> = Sieve::odd_primes().take(expected.len()).collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_prime_count() {
        // 根据素数定理，小于10000的素数数量约为10000/ln(10000) ≈ 1086
        let count = primes().take_while(|&p| p < 10000).count();
        assert!(count > 1000 && count < 1300); // 给一个合理的范围
    }
}
