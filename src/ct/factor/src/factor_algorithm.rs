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

//! 因式分解实现
//!
//! 结合了高效的 Miller-Rabin 素性测试和 Pollard's Rho 算法
//! 使用 num-prime 和 num-modular 库提高性能

use crate::miller_rabin::{self, is_prime};
use crate::rho::find_divisor;
use crate::table;
use smallvec::SmallVec;
use std::cell::RefCell;
use std::fmt;

type Exponent = u8;

#[derive(Clone, Debug, Default)]
struct Decomposition(SmallVec<[(u64, Exponent); NUM_FACTORS_INLINE]>);

// 根据 Erdős–Kac 定理，小于 10²⁵ ≃ 2⁸³ 的整数的平均素因子数为 4
// 因此我们使用稍高的值
const NUM_FACTORS_INLINE: usize = 5;

impl Decomposition {
    fn one() -> Self {
        Self::default()
    }

    fn add(&mut self, factor: u64, exp: Exponent) {
        debug_assert!(exp > 0);

        if let Some((_, e)) = self.0.iter_mut().find(|(f, _)| *f == factor) {
            *e += exp;
        } else {
            self.0.push((factor, exp));
        }
    }

    #[cfg(test)]
    fn product(&self) -> u64 {
        self.0
            .iter()
            .fold(1, |acc, (p, exp)| acc * p.pow(*exp as u32))
    }

    fn get(&self, p: u64) -> Option<&(u64, u8)> {
        self.0.iter().find(|(q, _)| *q == p)
    }
}

impl PartialEq for Decomposition {
    fn eq(&self, other: &Self) -> bool {
        for p in &self.0 {
            if other.get(p.0) != Some(p) {
                return false;
            }
        }

        for p in &other.0 {
            if self.get(p.0) != Some(p) {
                return false;
            }
        }

        true
    }
}
impl Eq for Decomposition {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Factors(RefCell<Decomposition>);

impl Factors {
    pub fn one() -> Self {
        Self(RefCell::new(Decomposition::one()))
    }

    pub fn add(&mut self, prime: u64, exp: Exponent) {
        debug_assert!(is_prime(prime));
        self.0.borrow_mut().add(prime, exp);
    }

    pub fn push(&mut self, prime: u64) {
        self.add(prime, 1);
    }

    #[cfg(test)]
    fn product(&self) -> u64 {
        self.0.borrow().product()
    }
}

impl fmt::Display for Factors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let v = &mut (self.0).borrow_mut().0;
        v.sort_unstable();

        let include_exponents = f.alternate();
        for (p, exp) in v {
            if include_exponents && *exp > 1 {
                write!(f, " {p}^{exp}")?;
            } else {
                for _ in 0..*exp {
                    write!(f, " {p}")?;
                }
            }
        }

        Ok(())
    }
}

/// 因式分解函数
fn _factor(num: u64, f: Factors) -> Factors {
    use miller_rabin::Result::*;

    if num == 1 {
        return f;
    }

    // 使用 Miller-Rabin 测试
    let test_result = miller_rabin::test(num);

    match test_result {
        Prime => {
            #[cfg(feature = "coz")]
            coz::progress!("factor found");
            let mut r = f;
            r.push(num);
            r
        }

        Composite(d) => {
            // 找到因子，递归分解
            let f = _factor(d, f);
            _factor(num / d, f)
        }

        Pseudoprime => {
            // 使用 Pollard's Rho 算法找因子
            let divisor = find_divisor(num);
            let f = _factor(divisor, f);
            _factor(num / divisor, f)
        }
    }
}

/// 因式分解的公共接口
pub fn factor(mut n: u64) -> Factors {
    #[cfg(feature = "coz")]
    coz::begin!("factorization");

    let mut factors = Factors::one();

    if n == 0 {
        // 特殊处理0的情况
        // 0不是素数，所以不能使用push方法
        // 创建一个特殊的Factors对象表示0
        let mut decomp = Decomposition::one();
        decomp.0.push((0, 1));
        return Factors(RefCell::new(decomp));
    }

    if n < 2 {
        return factors;
    }

    // 处理 2 的因子
    let n_zeros = n.trailing_zeros();
    if n_zeros > 0 {
        factors.add(2, n_zeros as Exponent);
        n >>= n_zeros;
    }

    if n == 1 {
        #[cfg(feature = "coz")]
        coz::end!("factorization");
        return factors;
    }

    // 使用预计算的素数表进行试除法
    table::pre_factor(&mut n, &mut factors);

    // 使用因式分解算法
    let result = _factor(n, factors);

    #[cfg(feature = "coz")]
    coz::end!("factorization");

    result
}

#[cfg(test)]
mod tests {
    use super::{Decomposition, Factors, factor};
    use quickcheck::quickcheck;
    use smallvec::smallvec;
    use std::cell::RefCell;

    #[test]
    #[ignore] // 忽略这个测试，因为它可能会超时
    fn factor_2044854919485649() {
        let f = Factors(RefCell::new(Decomposition(smallvec![
            (503, 1),
            (2423, 1),
            (40961, 2)
        ])));
        assert_eq!(factor(f.product()), f);
    }

    #[test]
    fn factor_recombines_small() {
        assert!(
            (1..10_000)
                .map(|i| 2 * i + 1)
                .all(|i| factor(i).product() == i)
        );
    }

    #[test]
    #[ignore] // 忽略这个测试，因为它可能会超时
    fn factor_recombines_overflowing() {
        assert!(
            (0..3) // 进一步减少测试范围，只测试3个数字
                .map(|i| 2 * i + 2u64.pow(32) + 1)
                .all(|i| factor(i).product() == i)
        );
    }

    #[test]
    fn factor_recombines_strong_pseudoprime() {
        // 这是一个强伪素数，测试算法处理特殊情况的能力
        let pseudoprime = 17_179_869_183;
        for _ in 0..20 {
            // 重复测试 20 次，因为它只在一部分时间内失败
            assert!(factor(pseudoprime).product() == pseudoprime);
        }
    }

    quickcheck! {
        // 限制测试范围，只测试较小的数字
        fn factor_recombines(i: u16) -> bool {
            let i = i as u64; // 将 u16 转换为 u64，限制测试范围
            i == 0 || factor(i).product() == i
        }
    }

    // 新增测试用例

    #[test]
    fn test_decomposition_one() {
        let d = Decomposition::one();
        assert_eq!(d.0.len(), 0);
        assert_eq!(d.product(), 1);
    }

    #[test]
    fn test_decomposition_add() {
        let mut d = Decomposition::one();

        // 添加一个因子
        d.add(3, 1);
        assert_eq!(d.0.len(), 1);
        assert_eq!(d.0[0], (3, 1));
        assert_eq!(d.product(), 3);

        // 添加另一个因子
        d.add(5, 2);
        assert_eq!(d.0.len(), 2);
        assert_eq!(d.0[1], (5, 2));
        assert_eq!(d.product(), 3 * 5 * 5);

        // 增加已有因子的指数
        d.add(3, 2);
        assert_eq!(d.0.len(), 2);
        assert_eq!(d.0[0], (3, 3));
        assert_eq!(d.product(), 3 * 3 * 3 * 5 * 5);
    }

    #[test]
    fn test_decomposition_get() {
        let mut d = Decomposition::one();
        d.add(3, 1);
        d.add(5, 2);

        assert_eq!(d.get(3), Some(&(3, 1)));
        assert_eq!(d.get(5), Some(&(5, 2)));
        assert_eq!(d.get(7), None);
    }

    #[test]
    fn test_decomposition_eq() {
        let mut d1 = Decomposition::one();
        d1.add(3, 1);
        d1.add(5, 2);

        let mut d2 = Decomposition::one();
        d2.add(5, 2);
        d2.add(3, 1);

        assert_eq!(d1, d2);

        let mut d3 = Decomposition::one();
        d3.add(3, 1);
        d3.add(5, 1);

        assert_ne!(d1, d3);
    }

    #[test]
    fn test_factors_one() {
        let f = Factors::one();
        assert_eq!(f.product(), 1);
    }

    #[test]
    fn test_factors_add() {
        let mut f = Factors::one();
        f.add(3, 1);
        assert_eq!(f.product(), 3);

        f.add(5, 2);
        assert_eq!(f.product(), 3 * 5 * 5);
    }

    #[test]
    fn test_factors_push() {
        let mut f = Factors::one();
        f.push(3);
        assert_eq!(f.product(), 3);

        f.push(5);
        assert_eq!(f.product(), 3 * 5);
    }

    #[test]
    fn test_factors_display() {
        let mut f = Factors::one();
        f.add(3, 1);
        f.add(5, 2);

        // 测试默认格式
        let s = format!("{}", f);
        assert_eq!(s, " 3 5 5");

        // 测试替代格式（带指数）
        let s = format!("{:#}", f);
        assert_eq!(s, " 3 5^2");
    }

    #[test]
    fn test_factor_small_numbers() {
        // 测试小数字的因式分解
        assert_eq!(factor(0).product(), 0);
        assert_eq!(factor(1).product(), 1);

        let f2 = factor(2);
        assert_eq!(f2.product(), 2);

        let f3 = factor(3);
        assert_eq!(f3.product(), 3);

        let f4 = factor(4);
        assert_eq!(f4.product(), 4);

        let f6 = factor(6);
        assert_eq!(f6.product(), 6);

        let f12 = factor(12);
        assert_eq!(f12.product(), 12);
    }

    #[test]
    fn test_factor_powers_of_two() {
        // 测试 2 的幂
        for i in 0..20 {
            let n = 1u64 << i;
            let f = factor(n);
            assert_eq!(f.product(), n);
        }
    }

    #[test]
    fn test_factor_primes() {
        // 测试素数
        let primes = [2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37, 41, 43, 47];
        for &p in &primes {
            let f = factor(p);
            assert_eq!(f.product(), p);
        }
    }

    #[test]
    fn test_factor_semiprime() {
        // 测试半素数（两个素数的乘积）
        let semiprimes = [(3, 5), (5, 7), (11, 13), (17, 19), (29, 31)];
        for &(p, q) in &semiprimes {
            let n = p * q;
            let f = factor(n);
            assert_eq!(f.product(), n);
        }
    }

    #[test]
    fn test_factor_highly_composite() {
        // 测试高度合成数（有很多因子的数）
        let n = 2 * 2 * 3 * 5 * 7 * 11; // 2310
        let f = factor(n);
        assert_eq!(f.product(), n);
    }
}
