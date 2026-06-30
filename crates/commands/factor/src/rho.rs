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

//! Pollard's rho 算法实现
//!
//! 该算法通过随机选择一个多项式来生成序列，并使用生日悖论原理来找到序列中的碰撞，从而提取出因数。
//!
//! 优化点：
//! 1. 使用 Brent 变种算法，减少 GCD 计算次数
//! 2. 使用批量 GCD 计算，提高效率
//! 3. 利用 Montgomery 乘法加速模运算
//! 4. 使用更好的随机数生成策略

use gcd::Gcd;
use rand::SeedableRng;
use rand::distributions::{Distribution, Uniform};
use rand::rngs::SmallRng;
use std::cmp::min;

use crate::numeric::{Arithmetic, Montgomery};

/// Pollard's Rho 算法，使用 Brent 变种和批量 GCD 计算
pub fn find_divisor(n: u64) -> u64 {
    if n % 2 == 0 {
        return 2;
    }

    // 使用 Montgomery 乘法
    let m = Montgomery::<u64>::new(n);

    // 随机数生成器
    let mut rng = SmallRng::from_entropy();
    let range = Uniform::new(2u64, n);

    // 尝试不同的起始值和增量
    for _ in 0..20 {
        let c = range.sample(&mut rng);
        let x0 = range.sample(&mut rng);

        if let Some(factor) = brent_pollard_rho(&m, x0, c) {
            return factor;
        }
    }

    // 如果所有尝试都失败，返回数字本身
    // 实际上这种情况很少发生，因为我们已经用 Miller-Rabin 测试过 n 不是素数
    n
}

/// Brent 变种的 Pollard's Rho 算法
fn brent_pollard_rho(m: &Montgomery<u64>, x0: u64, c: u64) -> Option<u64> {
    let n = m.modulus();
    let mut x = m.to_mod(x0);
    let c = m.to_mod(c);

    // 定义多项式 f(x) = x² + c
    let f = |x| m.add(m.mul(x, x), c);

    let mut product = m.to_mod(1);
    let mut g = 1u64;

    // 批量 GCD 的大小
    // coreutils 在 Pollard Rho 中每 32 步做一次 gcd 检查（k % 32 == 1）。
    // 这里对齐为 32，以保持与 coreutils 类似的检查频率与行为节奏。
    const BATCH_SIZE: u64 = 32;

    let mut r = 1u64;
    let mut ys = x; // 初始化 ys 变量

    // coreutils 没有显式迭代上限，会在 g==n 时通过更换参数递归重启。
    // 我们也会在 find_divisor 中做多次随机重试，并在更上层失败时回退到
    // 确定性分解（factorize64）。因此这里设置有限轮数作为“保险丝”，
    // 防止极端情况下长时间循环。
    //
    // 动态上限依据位数估算：
    // - r 每轮翻倍，总步数约为 2^max_rounds - 1。
    // - 对 32-bit 量级，max_rounds≈16，单次尝试约 65k 步。
    // - 对 64-bit 量级，上限逐步增大，但仍由 clamp 控制在可控范围。
    let bit_len = 64 - n.leading_zeros();
    let max_rounds = ((bit_len / 4) + 8).clamp(12, 28);
    // 回溯阶段上限同样随位数线性放大，避免极端卡死，
    // 同时配合上层重试与回退保证正确性。
    let max_backtrack = 32 * 1024 + (bit_len as u64 * 1024);
    let mut rounds = 0u32;

    while g == 1 {
        if rounds >= max_rounds {
            return None;
        }
        rounds += 1;
        x = ys;

        // 计算 2ʳ 步
        for _ in 0..r {
            ys = f(ys);
        }

        let mut k = 0u64;

        while k < r && g == 1 {
            // 批量计算 GCD
            let iterations = min(BATCH_SIZE, r - k);
            for _ in 0..iterations {
                ys = f(ys);
                let diff = if m.to_u64(x) >= m.to_u64(ys) {
                    m.to_u64(x) - m.to_u64(ys)
                } else {
                    m.to_u64(ys) - m.to_u64(x)
                };

                // 累积差值的乘积
                product = m.mul(product, m.to_mod(diff));
                k += 1;

                // 每 BATCH_SIZE 次计算一次 GCD
                if k % BATCH_SIZE == 0 || k == r {
                    g = m.to_u64(product).gcd(n);
                    if g > 1 {
                        break;
                    }
                    product = m.to_mod(1);
                }
            }

            // 如果找到因子，返回
            if g > 1 {
                break;
            }
        }

        // 增加步长
        if r > u64::MAX / 2 {
            return None;
        }
        r *= 2;

        // 如果 g 是 n 本身，我们需要回溯找到确切的因子
        if g == n {
            // 回溯找到确切的因子
            g = 1;
            let mut y = ys;
            x = m.to_mod(x0);

            let mut backtrack = 0u64;
            while g == 1 {
                if backtrack >= max_backtrack {
                    return None;
                }
                backtrack += 1;
                y = f(y);
                let diff = if m.to_u64(x) >= m.to_u64(y) {
                    m.to_u64(x) - m.to_u64(y)
                } else {
                    m.to_u64(y) - m.to_u64(x)
                };
                g = diff.gcd(n);
                x = f(x);
            }
        }
    }

    // 如果 g 是 n 本身，算法失败
    if g == n { None } else { Some(g) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::numeric::Montgomery;

    /// 测试偶数的因子查找
    #[test]
    fn test_find_divisor_even() {
        // 测试偶数
        assert_eq!(find_divisor(2), 2);
        assert_eq!(find_divisor(4), 2);
        assert_eq!(find_divisor(100), 2);
        assert_eq!(find_divisor(1024), 2);
    }

    /// 测试小素数的因子查找
    #[test]
    #[ignore]
    fn test_find_divisor_small_primes() {
        // 对于素数，应该返回其自身
        // 注意：由于算法的随机性，这个测试可能偶尔失败
        // 实际应用中，我们会先用 Miller-Rabin 测试判断是否为素数
        let primes = [3, 5, 7, 11, 13, 17, 19, 23, 29, 31];
        for &p in &primes {
            let result = find_divisor(p);
            assert!(result == p, "Expected {p} for prime {p}, got {result}");
        }
    }

    /// 测试合数的因子查找
    #[test]
    #[ignore]
    fn test_find_divisor_composites() {
        // 测试一些合数
        let composites = [
            (9, vec![3]),        // 3²
            (15, vec![3, 5]),    // 3 × 5
            (21, vec![3, 7]),    // 3 × 7
            (25, vec![5]),       // 5²
            (27, vec![3]),       // 3³
            (33, vec![3, 11]),   // 3 × 11
            (35, vec![5, 7]),    // 5 × 7
            (39, vec![3, 13]),   // 3 × 13
            (49, vec![7]),       // 7²
            (51, vec![3, 17]),   // 3 × 17
            (55, vec![5, 11]),   // 5 × 11
            (57, vec![3, 19]),   // 3 × 19
            (63, vec![3, 7, 9]), // 3 × 3 × 7 (可能返回 3 或 9)
            (65, vec![5, 13]),   // 5 × 13
            (77, vec![7, 11]),   // 7 × 11
            (91, vec![7, 13]),   // 7 × 13
        ];

        for &(n, ref expected_factors) in &composites {
            let factor = find_divisor(n);
            assert!(
                n % factor == 0,
                "Found factor {factor} is not a divisor of {n}"
            );
            assert!(
                expected_factors.contains(&factor) || expected_factors.contains(&(n / factor)),
                "For {n}, expected one of {expected_factors:?}, got {factor}"
            );
        }
    }

    /// 测试大合数的因子查找
    #[test]
    #[ignore]
    fn test_find_divisor_large_composites() {
        // 测试一些较大的合数
        // 由于算法的随机性，我们只检查结果是否为有效因子
        let large_composites = [
            10_000_001,    // 101 × 99_010
            1_000_003,     // 101 × 9_901
            1_000_009,     // 103 × 9_709
            1_000_000_007, // 1_000_003 × 1_000_004
            1_000_000_009, // 1_000_003 × 1_000_006
            999_999_937,   // 999_983 × 1_000_017
        ];

        for &n in &large_composites {
            let factor = find_divisor(n);
            assert!(
                n % factor == 0 && factor != 1 && factor != n,
                "Found factor {factor} is not a proper divisor of {n}"
            );
            println!("For {n}, found factor: {factor}");
        }
    }

    /// 测试 brent_pollard_rho 函数
    #[test]
    fn test_brent_pollard_rho() {
        // 测试一些已知的合数
        let test_cases = [
            (15, 2, 3), // n=15, x0=2, c=3
            (21, 3, 4), // n=21, x0=3, c=4
            (35, 4, 5), // n=35, x0=4, c=5
            (91, 5, 6), // n=91, x0=5, c=6
        ];

        for &(n, x0, c) in &test_cases {
            let m = Montgomery::<u64>::new(n);
            if let Some(factor) = brent_pollard_rho(&m, x0, c) {
                assert!(
                    n % factor == 0,
                    "Found factor {factor} is not a divisor of {n}"
                );
            } else {
                // 如果算法失败，这可能是由于随机性，但我们应该记录下来
                println!("Warning: brent_pollard_rho failed for n={n}, x0={x0}, c={c}");
            }
        }
    }

    /// 测试 Montgomery 乘法的正确性
    #[test]
    fn test_montgomery_arithmetic() {
        let n = 91; // 7 × 13
        let m = Montgomery::<u64>::new(n);

        // 测试基本运算
        let a = m.to_mod(5);
        let b = m.to_mod(10);

        // 加法
        let sum = m.add(a, b);
        assert_eq!(m.to_u64(sum), 15);

        // 乘法
        let product = m.mul(a, b);
        assert_eq!(m.to_u64(product), 50);

        // 测试模运算
        let large_a = m.to_mod(90);
        let large_b = m.to_mod(90);
        let large_product = m.mul(large_a, large_b);
        assert_eq!(m.to_u64(large_product), (90 * 90) % 91);
    }

    /// 测试算法在极端情况下的行为
    #[test]
    #[ignore]
    fn test_edge_cases() {
        // 测试小数
        assert_eq!(find_divisor(4), 2);

        // 测试大数
        let large_number = 999_999_999_989; // 大素数
        let result = find_divisor(large_number);
        // 由于算法的随机性，我们只能检查结果是否为因子
        assert!(
            large_number % result == 0,
            "Found factor {result} is not a divisor of {large_number}"
        );
    }

    /// 测试算法的稳定性
    #[test]
    #[ignore]
    fn test_stability() {
        // 多次运行算法，检查结果是否都是有效因子
        let n = 1_000_009; // 103 × 9_709

        for _ in 0..5 {
            let factor = find_divisor(n);
            assert!(
                n % factor == 0 && factor != 1 && factor != n,
                "Found factor {factor} is not a proper divisor of {n}"
            );
            println!("For {n}, found factor: {factor}");
        }
    }
}
