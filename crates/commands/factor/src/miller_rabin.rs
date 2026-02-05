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

#[cfg(test)]
mod tests {
    use super::*;
    use quickcheck::quickcheck;

    #[test]
    fn test_is_prime_small_numbers() {
        // 测试小数字的素性
        assert!(!is_prime(0));
        assert!(!is_prime(1));
        assert!(is_prime(2));
        assert!(is_prime(3));
        assert!(!is_prime(4));
        assert!(is_prime(5));
        assert!(!is_prime(6));
        assert!(is_prime(7));
        assert!(!is_prime(8));
        assert!(!is_prime(9));
        assert!(!is_prime(10));
        assert!(is_prime(11));
    }

    #[test]
    fn test_is_prime_known_primes() {
        // 测试已知的素数
        let known_primes = [
            2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37, 41, 43, 47, 53, 59, 61, 67, 71, 73, 79, 83,
            89, 97, 101, 103, 107, 109, 113, 127, 131, 137, 139, 149, 151, 157, 163, 167, 173, 179,
            181, 191, 193, 197, 199, 211, 223, 227, 229, 233, 239, 241, 251, 257, 263, 269, 271,
            277, 281, 283, 293,
        ];
        for &p in &known_primes {
            assert!(is_prime(p), "{p} should be prime");
        }
    }

    #[test]
    fn test_is_prime_known_composites() {
        // 测试已知的合数
        let known_composites = [
            4, 6, 8, 9, 10, 12, 14, 15, 16, 18, 20, 21, 22, 24, 25, 26, 27, 28, 30, 32, 33, 34, 35,
            36, 38, 39, 40, 42, 44, 45, 46, 48, 49, 50, 51, 52, 54, 55, 56, 57, 58, 60, 62, 63, 64,
            65, 66, 68, 69, 70, 72, 74, 75, 76, 77, 78, 80, 81, 82, 84, 85, 86, 87, 88, 90, 91, 92,
            93, 94, 95, 96, 98, 99, 100,
        ];
        for &c in &known_composites {
            assert!(!is_prime(c), "{c} should be composite");
        }
    }

    #[test]
    fn test_is_prime_large_primes() {
        // 测试一些大素数
        let large_primes = [
            997, 1009, 1013, 1019, 10007, 10009, 10037, 100003, 100019, 100043, 1000003, 1000033,
            1000037, 10000019, 10000079, 10000103, 100000007, 100000037, 100000039, 1000000007,
            1000000009, 1000000021,
        ];
        for &p in &large_primes {
            assert!(is_prime(p), "{p} should be prime");
        }
    }

    #[test]
    fn test_is_prime_large_composites() {
        // 测试一些大合数
        let large_composites = [
            1000 * 1001,
            10000 * 10001,
            100000 * 100001,
            997 * 991,
            1009 * 1013,
            10007 * 10009,
            100003 * 100019,
            1000003 * 1000033,
        ];
        for &c in &large_composites {
            assert!(!is_prime(c), "{c} should be composite");
        }
    }

    #[test]
    fn test_is_prime_carmichael_numbers() {
        // 测试 Carmichael 数（强伪素数）
        let carmichael_numbers = [
            561, 1105, 1729, 2465, 2821, 6601, 8911, 10585, 15841, 29341, 41041, 46657, 52633,
            62745, 63973, 75361, 101101, 115921, 126217, 162401, 172081, 188461, 252601, 278545,
            294409, 314821, 334153, 340561, 399001, 410041, 449065, 488881, 512461,
        ];
        for &c in &carmichael_numbers {
            assert!(!is_prime(c), "{c} should be composite (Carmichael number)");
        }
    }

    #[test]
    fn test_primality_small_numbers() {
        // 测试小数字的素性类型
        assert_eq!(primality(0), Primality::No);
        assert_eq!(primality(1), Primality::No);
        assert_eq!(primality(2), Primality::Yes);
        assert_eq!(primality(3), Primality::Yes);
        assert_eq!(primality(4), Primality::No);
        assert_eq!(primality(5), Primality::Yes);
        assert_eq!(primality(6), Primality::No);
        assert_eq!(primality(7), Primality::Yes);
        assert_eq!(primality(8), Primality::No);
        assert_eq!(primality(9), Primality::No);
        assert_eq!(primality(10), Primality::No);
        assert_eq!(primality(11), Primality::Yes);
    }

    #[test]
    fn test_primality_known_primes() {
        // 测试已知素数的素性类型
        let known_primes = [
            2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37, 41, 43, 47, 53, 59, 61, 67, 71,
        ];
        for &p in &known_primes {
            assert_eq!(primality(p), Primality::Yes, "{p} should be Prime");
        }
    }

    #[test]
    fn test_primality_known_composites() {
        // 测试已知合数的素性类型
        let known_composites = [
            4, 6, 8, 9, 10, 12, 14, 15, 16, 18, 20, 21, 22, 24, 25, 26, 27, 28, 30,
        ];
        for &c in &known_composites {
            assert_eq!(primality(c), Primality::No, "{c} should be Composite");
        }
    }

    #[test]
    fn test_primality_pseudoprimes() {
        // 测试伪素数的素性类型
        let pseudoprimes = [561, 1105, 1729, 2465, 2821, 6601, 8911, 10585, 15841, 29341];
        for &p in &pseudoprimes {
            // 注意：Miller-Rabin 测试可能会将某些伪素数识别为合数
            // 这里我们只是确保它不会被错误地识别为素数
            let result = primality(p);
            assert_eq!(
                result,
                Primality::No,
                "{p} should be identified as composite, got {result:?}"
            );
        }
    }

    #[test]
    fn test_miller_rabin_deterministic_range() {
        // 测试 Miller-Rabin 在确定性范围内的正确性
        // 对于 u32，Miller-Rabin 测试在 2^32 范围内是确定性的
        for i in 1..1000 {
            let n = i * i + i + 41; // 生成一些可能是素数的数
            let n = n as u64; // 转换为 u64 类型
            let is_prime_result = is_prime(n);
            let primality_result = primality(n);

            if is_prime_result {
                assert_eq!(
                    primality_result,
                    Primality::Yes,
                    "{n} is_prime() returned true but primality() returned {primality_result:?}"
                );
            } else {
                assert_eq!(
                    primality_result,
                    Primality::No,
                    "{n} is_prime() returned false but primality() returned {primality_result:?}"
                );
            }
        }
    }

    quickcheck! {
        // 使用 quickcheck 进行随机测试
        fn quickcheck_primality_consistency(n: u32) -> bool {
            // 确保 is_prime 和 primality 函数的结果一致
            let n = n as u64; // 转换为 u64 类型
            let is_prime_result = is_prime(n);
            let primality_result = primality(n);

            if is_prime_result {
                primality_result == Primality::Yes
            } else {
                primality_result == Primality::No
            }
        }
    }
}
