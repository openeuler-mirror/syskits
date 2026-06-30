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
//!
//! The finite values are stored as [`BigDecimal`] instances. Because
//! the `bigdecimal` library does not represent infinity, NaN, etc., we
//! need to represent them explicitly ourselves. The
//! [`ExtendedBigDecimal`] enumeration does that.
//!
//! # Examples
//!
//! Addition works for [`ExtendedBigDecimal`] as it does for floats. For
//! example, adding infinity to any finite value results in infinity:
//!
//! ```rust,ignore
//! let summand1 = ExtendedBigDecimal::BigDecimal(BigDecimal::zero());
//! let summand2 = ExtendedBigDecimal::Infinity;
//! assert_eq!(summand1 + summand2, ExtendedBigDecimal::Infinity);
//! ```
use std::cmp::Ordering;
use std::fmt::Display;
use std::ops::Add;

use bigdecimal::BigDecimal;
use num_traits::Zero;

/// 扩展的大数类型，支持特殊浮点值
#[derive(Debug, Clone, PartialEq)]
pub enum ExtendedBigDecimal {
    /// 任意精度浮点数
    BigDecimal(BigDecimal),
    /// 正无穷大
    Infinity,
    /// 负无穷大
    MinusInfinity,
    /// 负零
    MinusZero,
    /// 非数值
    Nan,
}

impl ExtendedBigDecimal {
    /// 创建值为0的实例
    #[cfg(test)]
    pub fn zero() -> Self {
        Self::BigDecimal(0.into())
    }

    /// 创建值为1的实例
    pub fn one() -> Self {
        Self::BigDecimal(1.into())
    }
}

impl Display for ExtendedBigDecimal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BigDecimal(x) => {
                let (n, p) = x.as_bigint_and_exponent();
                let mut s = if p > 0 {
                    let s_n = n.to_string();
                    let scale = p as usize;
                    let (sign, digits) = if let Some(stripped) = s_n.strip_prefix('-') {
                        ("-", stripped)
                    } else {
                        ("", s_n.as_str())
                    };
                    let len_digits = digits.len();
                    if len_digits > scale {
                        let split = len_digits - scale;
                        format!("{}{}.{}", sign, &digits[..split], &digits[split..])
                    } else {
                        let zeros = "0".repeat(scale - len_digits);
                        format!("{sign}0.{zeros}{digits}")
                    }
                } else {
                    let mut s = n.to_string();
                    let zeros = (-p) as usize;
                    for _ in 0..zeros {
                        s.push('0');
                    }
                    s
                };

                if let Some(req_prec) = f.precision() {
                    let parts: Vec<&str> = s.split('.').collect();
                    let int_part = parts[0];
                    let frac_part = if parts.len() > 1 { parts[1] } else { "" };

                    if req_prec == 0 {
                        s = int_part.to_string();
                    } else {
                        let mut frac = frac_part.to_string();
                        if frac.len() < req_prec {
                            frac.push_str(&"0".repeat(req_prec - frac.len()));
                        } else {
                            frac.truncate(req_prec);
                        }
                        s = format!("{int_part}.{frac}");
                    }
                }

                let width = f.width().unwrap_or(0);
                let s_len = s.chars().count();
                if width > s_len {
                    let diff = width - s_len;
                    if f.sign_aware_zero_pad() {
                        if let Some(stripped) = s.strip_prefix('-') {
                            write!(f, "-")?;
                            for _ in 0..diff {
                                write!(f, "0")?;
                            }
                            write!(f, "{stripped}")
                        } else {
                            for _ in 0..diff {
                                write!(f, "0")?;
                            }
                            write!(f, "{s}")
                        }
                    } else {
                        write!(f, "{s:>width$}")
                    }
                } else {
                    f.write_str(&s)
                }
            }
            Self::Infinity => {
                let width = f.width().unwrap_or(0);
                if width > 0 {
                    write!(f, "{:>width$}", "inf", width = width)
                } else {
                    write!(f, "inf")
                }
            }
            Self::MinusInfinity => write!(f, "-inf"),
            Self::MinusZero => write!(f, "-0"),
            Self::Nan => write!(f, "nan"),
        }
    }
}

impl Zero for ExtendedBigDecimal {
    fn zero() -> Self {
        Self::BigDecimal(BigDecimal::zero())
    }

    fn is_zero(&self) -> bool {
        match self {
            Self::BigDecimal(n) => n.is_zero(),
            Self::MinusZero => true,
            _ => false,
        }
    }
}

impl Add for ExtendedBigDecimal {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        match (&self, &other) {
            // 处理普通数值
            (Self::BigDecimal(_), Self::BigDecimal(_)) => Self::BigDecimal(match self {
                Self::BigDecimal(m) => {
                    m + match other {
                        Self::BigDecimal(n) => n,
                        _ => unreachable!(),
                    }
                }
                _ => unreachable!(),
            }),
            (Self::BigDecimal(_), Self::MinusZero) | (Self::MinusZero, Self::BigDecimal(_)) => {
                if matches!(self, Self::BigDecimal(_)) {
                    self
                } else {
                    other
                }
            }

            // 处理无穷大
            (Self::Infinity, Self::MinusInfinity) | (Self::MinusInfinity, Self::Infinity) => {
                Self::Nan
            }
            (Self::Infinity, _) | (_, Self::Infinity) => Self::Infinity,
            (Self::MinusInfinity, _) | (_, Self::MinusInfinity) => Self::MinusInfinity,

            // 处理NaN
            (Self::Nan, _) | (_, Self::Nan) => Self::Nan,

            // 处理负零
            (Self::MinusZero, _) => other,
        }
    }
}

impl PartialOrd for ExtendedBigDecimal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        // 首先检查是否涉及 NaN
        if matches!(self, Self::Nan) || matches!(other, Self::Nan) {
            return None;
        }

        match (self, other) {
            // 处理普通数值
            (Self::BigDecimal(m), Self::BigDecimal(n)) => m.partial_cmp(n),
            (Self::BigDecimal(m), Self::MinusZero) => m.partial_cmp(&BigDecimal::zero()),
            (Self::MinusZero, Self::BigDecimal(n)) => BigDecimal::zero().partial_cmp(n),

            // 处理无穷大
            (Self::Infinity, Self::Infinity) => Some(Ordering::Equal),
            (Self::Infinity, _) => Some(Ordering::Greater),
            (_, Self::Infinity) => Some(Ordering::Less),

            // 处理负无穷大
            (Self::MinusInfinity, Self::MinusInfinity) => Some(Ordering::Equal),
            (Self::MinusInfinity, _) => Some(Ordering::Less),
            (_, Self::MinusInfinity) => Some(Ordering::Greater),

            // 处理负零
            (Self::MinusZero, Self::MinusZero) => Some(Ordering::Equal),

            // NaN 的情况已在前面处理
            _ => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_addition() {
        // 测试普通数值
        let a = ExtendedBigDecimal::zero();
        let b = ExtendedBigDecimal::one();
        assert_eq!(a + b, ExtendedBigDecimal::one());

        // 测试无穷大
        assert_eq!(
            ExtendedBigDecimal::zero() + ExtendedBigDecimal::Infinity,
            ExtendedBigDecimal::Infinity
        );
        assert_eq!(
            ExtendedBigDecimal::zero() + ExtendedBigDecimal::MinusInfinity,
            ExtendedBigDecimal::MinusInfinity
        );
        assert!(matches!(
            ExtendedBigDecimal::Infinity + ExtendedBigDecimal::MinusInfinity,
            ExtendedBigDecimal::Nan
        ));
    }

    #[test]
    fn test_display() {
        // 测试零值显示，允许 "0" 或 "0.0" 格式
        let zero_str = ExtendedBigDecimal::zero().to_string();
        assert!(
            zero_str == "0" || zero_str == "0.0",
            "zero should be displayed as '0' or '0.0', got '{zero_str}'"
        );

        assert_eq!(ExtendedBigDecimal::Infinity.to_string(), "inf");
        assert_eq!(ExtendedBigDecimal::MinusInfinity.to_string(), "-inf");
        assert_eq!(ExtendedBigDecimal::MinusZero.to_string(), "-0");
        assert_eq!(ExtendedBigDecimal::Nan.to_string(), "nan");
    }

    #[test]
    fn test_display_with_width() {
        assert_eq!(format!("{:8}", ExtendedBigDecimal::Infinity), "     inf");
        assert_eq!(format!("{}", ExtendedBigDecimal::Infinity), "inf");
    }

    #[test]
    fn test_comparison() {
        let zero = ExtendedBigDecimal::zero();
        let one = ExtendedBigDecimal::one();
        let inf = ExtendedBigDecimal::Infinity;
        let neg_inf = ExtendedBigDecimal::MinusInfinity;
        let nan = ExtendedBigDecimal::Nan;

        // Test regular numbers
        let cmp = zero.partial_cmp(&one);
        assert!(matches!(cmp, Some(Ordering::Less)));

        // Test with infinity
        let cmp = neg_inf.partial_cmp(&zero);
        assert!(matches!(cmp, Some(Ordering::Less)));

        let cmp = zero.partial_cmp(&inf);
        assert!(matches!(cmp, Some(Ordering::Less)));

        let cmp = neg_inf.partial_cmp(&inf);
        assert!(matches!(cmp, Some(Ordering::Less)));

        // Test with NaN
        let cmp = inf.partial_cmp(&nan);
        assert_eq!(cmp, None);

        // Test equality cases
        let cmp = inf.partial_cmp(&inf);
        assert!(matches!(cmp, Some(Ordering::Equal)));
    }
}
