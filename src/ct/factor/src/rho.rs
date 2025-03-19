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
    const BATCH_SIZE: u64 = 100;

    let mut r = 1u64;
    let mut ys = x; // 初始化 ys 变量

    while g == 1 {
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
        r *= 2;

        // 如果 g 是 n 本身，我们需要回溯找到确切的因子
        if g == n {
            // 回溯找到确切的因子
            g = 1;
            let mut y = ys;
            x = m.to_mod(x0);

            while g == 1 {
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
