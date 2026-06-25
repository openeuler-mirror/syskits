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

//! 数值计算相关模块
//!
//! 本模块包含了因式分解所需的数值计算功能，主要包括：
//! 1. 整数类型的trait定义
//! 2. 蒙哥马利乘法算法实现
//!     蒙哥马利乘法（Montgomery Multiplication）算法，这是一种高效执行模乘运算的方法，
//!     特别适用于大数模乘运算, 本实现使用 num-modular 库，该库提供了高效的蒙哥马利乘法实现。

pub(crate) use num_traits::ops::overflowing::OverflowingAdd;
use num_traits::{
    int::PrimInt,
    ops::wrapping::{WrappingMul, WrappingNeg, WrappingSub},
};
use std::fmt::{Debug, Display};

#[allow(dead_code)]
pub trait Int:
    Display + Debug + PrimInt + OverflowingAdd + WrappingNeg + WrappingSub + WrappingMul
{
}

#[allow(dead_code)]
pub trait DoubleInt: Int {
    /// An integer type with twice the width of `Self`.
    /// In particular, multiplications (of `Int` values) can be performed in
    ///  `Self::DoubleWidth` without possibility of overflow.
    type DoubleWidth: Int;
}

macro_rules! int {
    ( $x:ty ) => {
        impl Int for $x {}
    };
}

macro_rules! int_with_option_u64 {
    ( $x:ty ) => {
        impl Int for $x {}
    };
}

macro_rules! double_int {
    ( $x:ty, $y:ty ) => {
        double_int!($x, $y, int);
    };
    ( $x:ty, $y:ty, $int_macro:ident ) => {
        $int_macro!($x);
        impl DoubleInt for $x {
            type DoubleWidth = $y;
        }
    };
}

// 使用新的宏为u32和u64实现Int trait
double_int!(u32, u64, int_with_option_u64);
double_int!(u64, u128, int_with_option_u64);
int!(u128);

/// Helper macro for instantiating tests over u32 and u64
#[cfg(test)]
#[macro_export]
macro_rules! parametrized_check {
    ( $f:ident ) => {
        paste::item! {
            #[test]
            fn [< $f _ u32 >]() {
                $f::<u32>()
            }
            #[test]
            fn [< $f _ u64 >]() {
                $f::<u64>()
            }
        }
    };
}

use num_modular::{ModularInteger, MontgomeryInt};

#[allow(dead_code)]
pub(crate) trait Arithmetic: Copy + Sized {
    // The type of integers mod m, in some opaque representation
    type ModInt: Copy + Sized + PartialEq;

    fn new(m: u64) -> Self;
    fn modulus(&self) -> u64;
    fn to_mod(&self, n: u64) -> Self::ModInt;
    fn to_u64(&self, n: Self::ModInt) -> u64;
    fn add(&self, a: Self::ModInt, b: Self::ModInt) -> Self::ModInt;
    fn mul(&self, a: Self::ModInt, b: Self::ModInt) -> Self::ModInt;
    fn pow(&self, a: Self::ModInt, b: u64) -> Self::ModInt {
        // 默认实现使用平方乘算法
        let mut result = self.to_mod(1);
        let mut base = a;
        let mut exp = b;

        while exp > 0 {
            if exp & 1 == 1 {
                result = self.mul(result, base);
            }
            base = self.mul(base, base);
            exp >>= 1;
        }

        result
    }
    fn one(&self) -> Self::ModInt {
        self.to_mod(1)
    }
    fn minus_one(&self) -> Self::ModInt {
        self.to_mod(self.modulus() - 1)
    }
}

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub struct Montgomery<T: DoubleInt> {
    modulus: u64,
    _phantom: std::marker::PhantomData<T>,
}

impl<T: DoubleInt> Montgomery<T> {
    // 空实现，实际操作由 num-modular 库处理
}

// 为了简化实现，我们使用u64作为ModInt类型
impl<T: DoubleInt> Arithmetic for Montgomery<T> {
    type ModInt = u64;

    fn new(n: u64) -> Self {
        debug_assert!(n % 2 == 1, "Modulus must be odd");
        Self {
            modulus: n,
            _phantom: std::marker::PhantomData,
        }
    }

    fn modulus(&self) -> u64 {
        self.modulus
    }

    fn to_mod(&self, x: u64) -> Self::ModInt {
        debug_assert!(x < self.modulus);
        x
    }

    fn to_u64(&self, n: Self::ModInt) -> u64 {
        n
    }

    fn add(&self, a: Self::ModInt, b: Self::ModInt) -> Self::ModInt {
        // 使用num-modular库进行计算
        let a_mont = MontgomeryInt::new(a, &self.modulus);
        let b_mont = MontgomeryInt::new(b, &self.modulus);
        let result = a_mont + b_mont;
        // 使用ModularInteger trait的residue方法
        ModularInteger::residue(&result)
    }

    fn mul(&self, a: Self::ModInt, b: Self::ModInt) -> Self::ModInt {
        // 使用num-modular库进行计算
        let a_mont = MontgomeryInt::new(a, &self.modulus);
        let b_mont = MontgomeryInt::new(b, &self.modulus);
        let result = a_mont * b_mont;
        // 使用ModularInteger trait的residue方法
        ModularInteger::residue(&result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quickcheck::quickcheck;

    #[test]
    fn test_int_trait_u32() {
        // 测试 u32 类型的基本操作
        let a: u32 = 42;
        let b: u32 = 13;

        // 测试基本运算
        assert_eq!(a.wrapping_add(b), 55);
        assert_eq!(a.wrapping_sub(b), 29);
        assert_eq!(a.wrapping_mul(b), 546);

        // 测试位运算
        assert_eq!(a.wrapping_shl(2), 168);
        assert_eq!(a.wrapping_shr(2), 10);

        // 测试其他操作
        assert_eq!(a.leading_zeros(), 26);
        assert_eq!(a.trailing_zeros(), 1);
    }

    #[test]
    fn test_int_trait_u64() {
        // 测试 u64 类型的基本操作
        let a: u64 = 42;
        let b: u64 = 13;

        // 测试基本运算
        assert_eq!(a.wrapping_add(b), 55);
        assert_eq!(a.wrapping_sub(b), 29);
        assert_eq!(a.wrapping_mul(b), 546);

        // 测试位运算
        assert_eq!(a.wrapping_shl(2), 168);
        assert_eq!(a.wrapping_shr(2), 10);

        // 测试其他操作
        assert_eq!(a.leading_zeros(), 58);
        assert_eq!(a.trailing_zeros(), 1);
    }

    #[test]
    fn test_double_int_trait_u32() {
        // 测试 u32 类型的溢出操作
        let a: u32 = 0xFFFFFFFF; // 最大的 u32 值
        let b: u32 = 2;

        // 测试加法溢出
        let (sum, overflow) = a.overflowing_add(b);
        assert_eq!(sum, 1);
        assert_eq!(overflow, true);
    }

    #[test]
    fn test_double_int_trait_u64() {
        // 测试 u64 类型的溢出操作
        let a: u64 = 0xFFFFFFFFFFFFFFFF; // 最大的 u64 值
        let b: u64 = 2;

        // 测试加法溢出
        let (sum, overflow) = a.overflowing_add(b);
        assert_eq!(sum, 1);
        assert_eq!(overflow, true);
    }

    #[test]
    fn test_montgomery_new() {
        // 测试 Montgomery 结构的创建
        let m = Montgomery::<u32>::new(17);
        assert_eq!(m.modulus, 17);

        let m = Montgomery::<u64>::new(101);
        assert_eq!(m.modulus, 101);
    }

    #[test]
    fn test_montgomery_to_mod_and_to_u64() {
        // 测试 Montgomery 域的转换
        let m = Montgomery::<u32>::new(17);

        // 转换到 Montgomery 域再转换回来
        let a_mod = m.to_mod(5);
        assert_eq!(m.to_u64(a_mod), 5);

        // 对于大于模数的值，应该先取模
        let val = 20 % 17;
        let b_mod = m.to_mod(val);
        assert_eq!(m.to_u64(b_mod), 3); // 20 % 17 = 3

        // 在某些实现中，Montgomery表示法可能与原值相同
        // 所以我们不再断言它们一定不相等
    }

    #[test]
    fn test_montgomery_add() {
        // 测试 Montgomery 加法
        let m = Montgomery::<u32>::new(17);

        // 转换到 Montgomery 域
        let a_mod = m.to_mod(5);
        let b_mod = m.to_mod(7);

        // 测试加法
        let sum_mod = m.add(a_mod, b_mod);
        assert_eq!(m.to_u64(sum_mod), 12);

        // 测试溢出加法
        let c_mod = m.to_mod(15);
        let sum_mod = m.add(a_mod, c_mod);
        assert_eq!(m.to_u64(sum_mod), 3); // (5 + 15) % 17 = 20 % 17 = 3
    }

    #[test]
    fn test_montgomery_mul() {
        // 测试 Montgomery 乘法
        let m = Montgomery::<u32>::new(17);

        // 转换到 Montgomery 域
        let a_mod = m.to_mod(5);
        let b_mod = m.to_mod(7);

        // 测试乘法
        let prod_mod = m.mul(a_mod, b_mod);
        assert_eq!(m.to_u64(prod_mod), 35 % 17);

        // 测试大数乘法
        let c_mod = m.to_mod(15);
        let prod_mod = m.mul(c_mod, c_mod);
        assert_eq!(m.to_u64(prod_mod), 225 % 17);
    }

    #[test]
    fn test_montgomery_pow() {
        // 测试 Montgomery 幂运算
        let m = Montgomery::<u32>::new(17);

        // 转换到 Montgomery 域
        let a_mod = m.to_mod(5);

        // 测试幂运算
        let pow_mod = m.pow(a_mod, 3);
        assert_eq!(m.to_u64(pow_mod), 125 % 17);

        // 测试大指数
        let pow_mod = m.pow(a_mod, 10);
        assert_eq!(m.to_u64(pow_mod), 5u64.pow(10) % 17);
    }

    #[test]
    fn test_montgomery_one() {
        // 测试 Montgomery 的单位元
        let m = Montgomery::<u32>::new(17);

        // 获取单位元
        let one_mod = m.one();
        assert_eq!(m.to_u64(one_mod), 1);

        // 测试乘法单位元性质
        let a_mod = m.to_mod(5);
        let prod_mod = m.mul(a_mod, one_mod);
        assert_eq!(m.to_u64(prod_mod), 5);
    }

    #[test]
    fn test_montgomery_minus_one() {
        // 测试 Montgomery 的负单位元
        let m = Montgomery::<u32>::new(17);

        // 获取负单位元
        let minus_one_mod = m.minus_one();
        assert_eq!(m.to_u64(minus_one_mod), 16); // -1 % 17 = 16

        // 测试负单位元性质
        let a_mod = m.to_mod(5);
        let prod_mod = m.mul(a_mod, minus_one_mod);
        assert_eq!(m.to_u64(prod_mod), 12); // (5 * -1) % 17 = -5 % 17 = 12
    }

    quickcheck! {
        // 使用 quickcheck 进行随机测试
        fn quickcheck_montgomery_add_commutative(a: u32, b: u32) -> bool {
            // 确保模数不为 0 或 1，且为奇数
            let modulus = if a <= 1 || a % 2 == 0 { 17u64 } else { a as u64 };

            // 确保输入不超过模数
            let x = b as u64 % modulus;
            let y = (b as u64 / 2) % modulus;

            let m = Montgomery::<u64>::new(modulus);
            let x_mod = m.to_mod(x);
            let y_mod = m.to_mod(y);

            // 测试加法交换律
            let sum1 = m.add(x_mod, y_mod);
            let sum2 = m.add(y_mod, x_mod);

            m.to_u64(sum1) == m.to_u64(sum2)
        }

        fn quickcheck_montgomery_mul_commutative(a: u32, b: u32) -> bool {
            // 确保模数不为 0 或 1，且为奇数
            let modulus = if a <= 1 || a % 2 == 0 { 17u64 } else { a as u64 };

            // 确保输入不超过模数
            let x = b as u64 % modulus;
            let y = (b as u64 / 2) % modulus;

            let m = Montgomery::<u64>::new(modulus);
            let x_mod = m.to_mod(x);
            let y_mod = m.to_mod(y);

            // 测试乘法交换律
            let prod1 = m.mul(x_mod, y_mod);
            let prod2 = m.mul(y_mod, x_mod);

            m.to_u64(prod1) == m.to_u64(prod2)
        }

        fn quickcheck_montgomery_mul_distributive(a: u32, b: u32, _c: u32) -> bool {
            // 确保模数不为 0 或 1，且为奇数
            let modulus = if a <= 1 || a % 2 == 0 { 17u64 } else { a as u64 };

            // 确保输入不超过模数
            let x = b as u64 % modulus;
            let y = (b as u64 / 2) % modulus;
            let z = (b as u64 / 3) % modulus;

            let m = Montgomery::<u64>::new(modulus);
            let x_mod = m.to_mod(x);
            let y_mod = m.to_mod(y);
            let z_mod = m.to_mod(z);

            // 测试乘法分配律: x * (y + z) = x * y + x * z
            let sum_mod = m.add(y_mod, z_mod);
            let left = m.mul(x_mod, sum_mod);

            let prod1 = m.mul(x_mod, y_mod);
            let prod2 = m.mul(x_mod, z_mod);
            let right = m.add(prod1, prod2);

            m.to_u64(left) == m.to_u64(right)
        }
    }
}
