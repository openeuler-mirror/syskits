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

//! 因式分解实现
//!
//! 结合了高效的 Miller-Rabin 素性测试和 Pollard's Rho 算法
//! 使用 num-prime 和 num-modular 库提高性能

use smallvec::SmallVec;
use std::cell::RefCell;
use std::fmt;

use crate::miller_rabin::{self, is_prime};
use crate::rho::find_divisor;
use crate::table;

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
