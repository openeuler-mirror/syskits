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

//! Miller-Rabin 素性测试算法实现
//! 结合了 num-prime 库的高效素性测试和原有实现的因子提取功能
//! 能够识别出强伪素数（通过常规素性测试但实际上是合数的数）

use crate::numeric::*;
use gcd::Gcd;
use num_prime::nt_funcs::is_prime as np_is_prime;
use num_prime::{Primality, PrimalityTestConfig};

#[derive(Eq, PartialEq)]
#[must_use = "Ignoring the output of a primality test."]
pub(crate) enum Result {
    Prime,
    Pseudoprime,
    Composite(u64),
}

/// Miller-Rabin 测试，结合 num-prime 库和因子提取功能
pub(crate) fn test(n: u64) -> Result {
    use self::Result::*;

    if n <= 1 {
        return Composite(1);
    }

    if n == 2 {
        return Prime;
    }

    if n % 2 == 0 {
        return Composite(2);
    }

    // 使用 num-prime 库的确定性 Miller-Rabin 测试
    // 对于 u64 类型，num-prime 使用确定性的测试基底
    let config = PrimalityTestConfig::default();

    if np_is_prime(&n, Some(config)) == Primality::Yes {
        return Prime;
    }

    // 如果不是素数，尝试提取因子
    // n-1 = r 2ⁱ
    let i = (n - 1).trailing_zeros();
    let r = (n - 1) >> i;

    // 使用与原实现相同的测试基底
    let bases: &[u64] = if n < (1 << 32) {
        &[2, 7, 61]
    } else {
        &[2, 325, 9375, 28178, 450_775, 9_780_504, 1_795_265_022]
    };

    // 尝试提取因子
    for &a in bases {
        let a = a % n;
        if a == 0 {
            continue;
        }

        // 使用 Montgomery 乘法计算 a^r mod n
        let m = Montgomery::<u64>::new(n);
        let a_mod = m.to_mod(a);
        let mut x = m.pow(a_mod, r);

        if x == m.one() || x == m.minus_one() {
            continue;
        }

        for _ in 1..i {
            let y = m.mul(x, x);
            if y == m.one() {
                // 找到因子: gcd(x-1, n)
                return Composite(m.to_u64(x).wrapping_sub(1).gcd(n));
            } else if y == m.minus_one() {
                // 这个基底元素不是 n 为合数的见证
                break;
            }
            x = y;
        }

        // 如果我们到达这里，这个基底元素是 n 为合数的见证
        // 但我们没有找到因子，所以这可能是一个强伪素数
        return Pseudoprime;
    }

    // 如果所有基底都没有证明 n 是合数，那么 n 可能是素数
    // 但由于我们已经用 num-prime 确定它不是素数，所以这是一个伪素数
    Pseudoprime
}

/// 简化的素性测试接口
pub fn is_prime(n: u64) -> bool {
    if n < 2 {
        false
    } else if n % 2 == 0 {
        n == 2
    } else {
        // 直接使用 num-prime 库的确定性测试
        let config = PrimalityTestConfig::default();
        np_is_prime(&n, Some(config)) == Primality::Yes
    }
}

/// 返回数字的素性类型
pub fn primality(n: u64) -> Primality {
    if n < 2 {
        Primality::No
    } else if n % 2 == 0 {
        if n == 2 {
            Primality::Yes
        } else {
            Primality::No
        }
    } else {
        // 使用 num-prime 库的确定性测试
        let config = PrimalityTestConfig::default();
        np_is_prime(&n, Some(config))
    }
}
