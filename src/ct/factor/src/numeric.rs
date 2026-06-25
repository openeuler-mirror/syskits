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
